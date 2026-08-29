//! Sole P4 `Deviation -> TargetInventory` strategy model.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::{GridConfig, GridLevelConfig, StrategyConfig},
    domain::{
        ids::{DecisionId, IdentifierError, ModelVersion, PairId, Symbol, VenueId},
        inventory::{TargetDirection, TargetInventory},
        numeric::{Bps, Notional, NumericError, TargetFraction},
    },
    models::P4_MODEL_VERSION,
};

/// Frozen V1 behavior between configured deviation boundaries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BetweenGridRule {
    /// Select the highest configured boundary which does not exceed the positive deviation.
    FloorStep,
}

/// One independently oriented input. `PairId` itself carries no direction.
#[derive(Clone, Copy, Debug)]
pub struct GridRouteInput<'a> {
    pub pair_id: &'a PairId,
    pub symbol: &'a Symbol,
    pub long_venue: &'a VenueId,
    pub short_venue: &'a VenueId,
    pub deviation_bps: Bps,
    pub decision_id: &'a DecisionId,
}

/// Auditable grid output before pair-level arbitration or inventory comparison.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GridRouteTarget {
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub deviation_bps: Bps,
    pub applied_boundary_bps: Option<Bps>,
    pub between_grid_rule: BetweenGridRule,
    pub target_fraction: TargetFraction,
    pub target_notional_per_leg: Notional,
    pub reason: String,
    pub model_version: ModelVersion,
    pub decision_id: DecisionId,
}

impl GridRouteTarget {
    #[must_use]
    pub fn target_fraction(&self) -> TargetFraction {
        self.target_fraction
    }

    #[must_use]
    pub fn target_notional_per_leg(&self) -> Notional {
        self.target_notional_per_leg
    }

    #[must_use]
    pub fn requests_risk(&self) -> bool {
        self.target_fraction.value() > Decimal::ZERO
    }

    #[must_use]
    pub fn target_direction(&self) -> TargetDirection {
        if self.requests_risk() {
            TargetDirection::LongShort {
                long_venue: self.long_venue.clone(),
                short_venue: self.short_venue.clone(),
            }
        } else {
            TargetDirection::Flat
        }
    }

    /// Materializes the sole external target only after pair-level arbitration.
    pub(crate) fn to_target_inventory(&self) -> TargetInventory {
        TargetInventory {
            symbol: self.symbol.clone(),
            pair_id: self.pair_id.clone(),
            target_fraction: self.target_fraction,
            target_notional: self.target_notional_per_leg,
            direction: self.target_direction(),
            reason: self.reason.clone(),
            model_version: self.model_version.clone(),
            decision_id: self.decision_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GridInventoryError {
    #[error("grid configuration must be positive and strictly monotonic")]
    InvalidConfiguration,
    #[error("an oriented route must use distinct venues")]
    SameVenue,
    #[error("fixed-decimal grid arithmetic failed")]
    Arithmetic,
    #[error(transparent)]
    Numeric(#[from] NumericError),
    #[error("invalid P4 model version: {0}")]
    ModelVersion(#[from] IdentifierError),
}

/// Deterministic, stateless segmented-grid model. It has no inventory, risk, or order dependency.
#[derive(Clone, Debug)]
pub struct GridInventoryModel {
    levels: Vec<GridLevelConfig>,
    max_target_notional: Notional,
    model_version: ModelVersion,
}

impl GridInventoryModel {
    pub fn new(grid: &GridConfig, strategy: &StrategyConfig) -> Result<Self, GridInventoryError> {
        if grid.levels.is_empty() || strategy.max_target_notional.value() <= Decimal::ZERO {
            return Err(GridInventoryError::InvalidConfiguration);
        }
        let mut previous_deviation = Decimal::ZERO;
        let mut previous_target = Decimal::ZERO;
        for level in &grid.levels {
            let deviation = level.deviation_bps.value();
            let target = level.target_fraction.value();
            if deviation <= previous_deviation || target <= previous_target {
                return Err(GridInventoryError::InvalidConfiguration);
            }
            previous_deviation = deviation;
            previous_target = target;
        }
        Ok(Self {
            levels: grid.levels.clone(),
            max_target_notional: strategy.max_target_notional,
            model_version: ModelVersion::try_from(P4_MODEL_VERSION)?,
        })
    }

    /// Maps one explicit route's deviation to a non-negative floor-step target.
    pub fn evaluate(
        &self,
        input: GridRouteInput<'_>,
    ) -> Result<GridRouteTarget, GridInventoryError> {
        if input.long_venue == input.short_venue {
            return Err(GridInventoryError::SameVenue);
        }
        let applied = self
            .levels
            .iter()
            .take_while(|level| input.deviation_bps.value() >= level.deviation_bps.value())
            .last();
        let target_fraction = applied.map_or_else(
            || TargetFraction::new(Decimal::ZERO),
            |level| Ok(level.target_fraction),
        )?;
        let target_notional_value = self
            .max_target_notional
            .value()
            .checked_mul(target_fraction.value())
            .ok_or(GridInventoryError::Arithmetic)?;
        let target_notional = Notional::new(target_notional_value)?;
        let reason = applied.map_or_else(
            || "below_first_grid_boundary".to_owned(),
            |level| format!("floor_grid_boundary_{}_bps", level.deviation_bps.value()),
        );
        Ok(GridRouteTarget {
            pair_id: input.pair_id.clone(),
            symbol: input.symbol.clone(),
            long_venue: input.long_venue.clone(),
            short_venue: input.short_venue.clone(),
            deviation_bps: input.deviation_bps,
            applied_boundary_bps: applied.map(|level| level.deviation_bps),
            between_grid_rule: BetweenGridRule::FloorStep,
            target_fraction,
            target_notional_per_leg: target_notional,
            reason,
            model_version: self.model_version.clone(),
            decision_id: input.decision_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{BetweenGridRule, GridInventoryModel, GridRouteInput};
    use crate::{
        config::parse_toml,
        domain::{
            ids::{DecisionId, PairId, Symbol, VenueId},
            inventory::TargetDirection,
            numeric::Bps,
        },
    };

    const EXAMPLE: &str = include_str!("../../config/example.toml");

    struct Fixture {
        pair_id: PairId,
        symbol: Symbol,
        entropy: VenueId,
        lighter: VenueId,
        decision_id: DecisionId,
    }

    fn fixture() -> Result<Fixture, Box<dyn Error>> {
        Ok(Fixture {
            pair_id: PairId::try_from("SNDK-ENTROPY-LIGHTER")?,
            symbol: Symbol::try_from("SNDK")?,
            entropy: VenueId::try_from("entropy")?,
            lighter: VenueId::try_from("lighter")?,
            decision_id: DecisionId::try_from("grid-test")?,
        })
    }

    fn target(
        deviation: Decimal,
        reverse: bool,
    ) -> Result<(crate::config::AppConfig, super::GridRouteTarget), Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let model = GridInventoryModel::new(&config.grid, &config.strategy)?;
        let fixture = fixture()?;
        let (long_venue, short_venue) = if reverse {
            (&fixture.lighter, &fixture.entropy)
        } else {
            (&fixture.entropy, &fixture.lighter)
        };
        let output = model.evaluate(GridRouteInput {
            pair_id: &fixture.pair_id,
            symbol: &fixture.symbol,
            long_venue,
            short_venue,
            deviation_bps: Bps::new(deviation),
            decision_id: &fixture.decision_id,
        })?;
        Ok((config, output))
    }

    #[test]
    fn zero_and_below_first_boundary_are_flat() -> Result<(), Box<dyn Error>> {
        for deviation in [Decimal::ZERO, Decimal::new(499, 2), Decimal::NEGATIVE_ONE] {
            let (_, output) = target(deviation, false)?;
            assert_eq!(output.target_fraction().value(), Decimal::ZERO);
            assert_eq!(output.target_notional_per_leg().value(), Decimal::ZERO);
            assert_eq!(output.target_direction(), TargetDirection::Flat);
        }
        Ok(())
    }

    #[test]
    fn every_grid_boundary_maps_exactly() -> Result<(), Box<dyn Error>> {
        for (deviation, fraction) in [(5, 20), (10, 40), (15, 60), (20, 80), (25, 100)] {
            let (_, output) = target(Decimal::new(deviation, 0), false)?;
            assert_eq!(output.target_fraction().value(), Decimal::new(fraction, 2));
            assert_eq!(
                output.applied_boundary_bps.expect("grid boundary").value(),
                Decimal::new(deviation, 0)
            );
        }
        Ok(())
    }

    #[test]
    fn between_boundaries_uses_conservative_floor_step() -> Result<(), Box<dyn Error>> {
        let (_, output) = target(Decimal::new(1499, 2), false)?;
        assert_eq!(output.target_fraction().value(), Decimal::new(40, 2));
        assert_eq!(output.between_grid_rule, BetweenGridRule::FloorStep);
        Ok(())
    }

    #[test]
    fn expansion_is_monotonic_and_convergence_lowers_target() -> Result<(), Box<dyn Error>> {
        let deviations = [0_i64, 5, 9, 10, 17, 20, 30];
        let fractions: Vec<_> = deviations
            .into_iter()
            .map(|deviation| {
                target(Decimal::new(deviation, 0), false)
                    .map(|(_, output)| output.target_fraction().value())
            })
            .collect::<Result<_, _>>()?;
        assert!(fractions.windows(2).all(|window| window[0] <= window[1]));
        assert!(
            fractions
                .iter()
                .rev()
                .copied()
                .collect::<Vec<_>>()
                .windows(2)
                .all(|window| window[0] >= window[1])
        );
        Ok(())
    }

    #[test]
    fn route_direction_is_explicit_and_reverse_is_independent() -> Result<(), Box<dyn Error>> {
        let (_, forward) = target(Decimal::new(10, 0), false)?;
        let (_, reverse) = target(Decimal::new(20, 0), true)?;
        assert_eq!(forward.target_fraction().value(), Decimal::new(40, 2));
        assert_eq!(reverse.target_fraction().value(), Decimal::new(80, 2));
        assert_eq!(
            forward.target_direction(),
            TargetDirection::LongShort {
                long_venue: VenueId::try_from("entropy")?,
                short_venue: VenueId::try_from("lighter")?,
            }
        );
        assert_eq!(
            reverse.target_direction(),
            TargetDirection::LongShort {
                long_venue: VenueId::try_from("lighter")?,
                short_venue: VenueId::try_from("entropy")?,
            }
        );
        Ok(())
    }

    #[test]
    fn full_grid_target_uses_strategy_cap_per_leg_not_risk_limit() -> Result<(), Box<dyn Error>> {
        let (config, output) = target(Decimal::new(25, 0), false)?;
        assert_eq!(output.target_fraction().value(), Decimal::ONE);
        assert_eq!(
            output.target_notional_per_leg(),
            config.strategy.max_target_notional
        );
        assert_ne!(
            output.target_notional_per_leg(),
            config.risk.max_pair_notional
        );
        assert!(
            output.target_notional_per_leg().value() * Decimal::from(2_u8)
                > output.target_notional_per_leg().value()
        );
        Ok(())
    }
}
