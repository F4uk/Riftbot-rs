//! Sole P4 `Deviation -> TargetInventory` strategy model.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::{GridConfig, GridLevelConfig, StrategyConfig},
    domain::{
        ids::{DecisionId, IdentifierError, ModelVersion, PairId, Symbol, VenueId},
        inventory::{
            InventoryDomainError, TargetDirection, TargetInventory, TargetInventoryParams,
        },
        numeric::{BaseQty, Bps, Notional, NumericError, Price, TargetFraction, UnixNanos},
        spread::MeasurementValidity,
    },
    models::{P4_MODEL_VERSION, opportunity::OpportunityEvaluation},
};

/// Frozen V1 behavior between configured deviation boundaries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BetweenGridRule {
    /// Select the highest configured boundary which does not exceed the positive deviation.
    FloorStep,
}

/// Immutable P4 view of one P3 `OpportunityEvaluation` snapshot.
///
/// Production callers can only construct this through `TryFrom<&OpportunityEvaluation>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GridMeasurementInput {
    pair_id: PairId,
    symbol: Symbol,
    long_venue: VenueId,
    short_venue: VenueId,
    source_deviation_bps: Bps,
    source_observed_at: UnixNanos,
    source_measurement_model_version: ModelVersion,
    source_measurement_config_fingerprint: String,
    requested_base_quantity: BaseQty,
    executable_long_price: Price,
    executable_short_price: Price,
    long_measured_notional: Notional,
    short_measured_notional: Notional,
    measured_matched_notional_cap: Notional,
    validity: MeasurementValidity,
    tradable_edge_bps: Option<Bps>,
    increase_risk_economically_allowed: bool,
}

impl GridMeasurementInput {
    #[must_use]
    pub fn pair_id(&self) -> &PairId {
        &self.pair_id
    }

    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    #[must_use]
    pub fn long_venue(&self) -> &VenueId {
        &self.long_venue
    }

    #[must_use]
    pub fn short_venue(&self) -> &VenueId {
        &self.short_venue
    }

    #[must_use]
    pub const fn source_deviation_bps(&self) -> Bps {
        self.source_deviation_bps
    }

    #[must_use]
    pub const fn source_observed_at(&self) -> UnixNanos {
        self.source_observed_at
    }

    #[must_use]
    pub fn source_measurement_model_version(&self) -> &ModelVersion {
        &self.source_measurement_model_version
    }

    #[must_use]
    pub fn source_measurement_config_fingerprint(&self) -> &str {
        &self.source_measurement_config_fingerprint
    }

    #[must_use]
    pub const fn requested_base_quantity(&self) -> BaseQty {
        self.requested_base_quantity
    }

    #[must_use]
    pub const fn executable_long_price(&self) -> Price {
        self.executable_long_price
    }

    #[must_use]
    pub const fn executable_short_price(&self) -> Price {
        self.executable_short_price
    }

    #[must_use]
    pub const fn long_measured_notional(&self) -> Notional {
        self.long_measured_notional
    }

    #[must_use]
    pub const fn short_measured_notional(&self) -> Notional {
        self.short_measured_notional
    }

    #[must_use]
    pub const fn measured_matched_notional_cap(&self) -> Notional {
        self.measured_matched_notional_cap
    }

    #[must_use]
    pub const fn validity(&self) -> MeasurementValidity {
        self.validity
    }

    #[must_use]
    pub const fn tradable_edge_bps(&self) -> Option<Bps> {
        self.tradable_edge_bps
    }

    #[must_use]
    pub const fn increase_risk_economically_allowed(&self) -> bool {
        self.increase_risk_economically_allowed
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn for_test(
        pair_id: PairId,
        symbol: Symbol,
        long_venue: VenueId,
        short_venue: VenueId,
        deviation_bps: Bps,
        observed_at: UnixNanos,
        measurement_model_version: ModelVersion,
        config_fingerprint: String,
        requested_base_quantity: BaseQty,
        executable_long_price: Price,
        executable_short_price: Price,
        validity: MeasurementValidity,
        tradable_edge_bps: Option<Bps>,
        increase_risk_economically_allowed: bool,
    ) -> Result<Self, GridInventoryError> {
        build_measurement_input(
            pair_id,
            symbol,
            long_venue,
            short_venue,
            deviation_bps,
            observed_at,
            measurement_model_version,
            config_fingerprint,
            requested_base_quantity,
            executable_long_price,
            executable_short_price,
            validity,
            tradable_edge_bps,
            increase_risk_economically_allowed,
        )
    }
}

impl TryFrom<&OpportunityEvaluation> for GridMeasurementInput {
    type Error = GridInventoryError;

    fn try_from(evaluation: &OpportunityEvaluation) -> Result<Self, Self::Error> {
        let spread = &evaluation.spread;
        let opportunity = &evaluation.opportunity;
        if spread.pair_id != opportunity.pair_id
            || spread.symbol != opportunity.symbol
            || spread.long_venue != opportunity.long_venue
            || spread.short_venue != opportunity.short_venue
            || spread.executable_long_price != opportunity.executable_long_price
            || spread.executable_short_price != opportunity.executable_short_price
            || spread.executable_notional != opportunity.executable_notional
            || spread.deviation_bps != opportunity.deviation_bps
            || spread.tradable_edge_bps != opportunity.tradable_edge_bps
            || spread.validity != opportunity.validity
            || spread.timestamp != opportunity.timestamp
            || spread.model_version != opportunity.model_version
            || spread.config_fingerprint != opportunity.config_fingerprint
        {
            return Err(GridInventoryError::InconsistentMeasurementSnapshot);
        }
        let deviation_bps = opportunity
            .deviation_bps
            .ok_or(GridInventoryError::MissingDeviation)?;
        let input = build_measurement_input(
            opportunity.pair_id.clone(),
            opportunity.symbol.clone(),
            opportunity.long_venue.clone(),
            opportunity.short_venue.clone(),
            deviation_bps,
            opportunity.timestamp,
            opportunity.model_version.clone(),
            opportunity.config_fingerprint.clone(),
            spread.requested_base_quantity,
            opportunity.executable_long_price,
            opportunity.executable_short_price,
            opportunity.validity,
            opportunity.tradable_edge_bps,
            opportunity.increase_risk_economically_allowed,
        )?;
        if input.long_measured_notional != opportunity.executable_notional {
            return Err(GridInventoryError::InconsistentMeasurementSnapshot);
        }
        Ok(input)
    }
}

#[allow(clippy::too_many_arguments)]
fn build_measurement_input(
    pair_id: PairId,
    symbol: Symbol,
    long_venue: VenueId,
    short_venue: VenueId,
    source_deviation_bps: Bps,
    source_observed_at: UnixNanos,
    source_measurement_model_version: ModelVersion,
    source_measurement_config_fingerprint: String,
    requested_base_quantity: BaseQty,
    executable_long_price: Price,
    executable_short_price: Price,
    validity: MeasurementValidity,
    tradable_edge_bps: Option<Bps>,
    increase_risk_economically_allowed: bool,
) -> Result<GridMeasurementInput, GridInventoryError> {
    if long_venue == short_venue || source_measurement_config_fingerprint.is_empty() {
        return Err(GridInventoryError::InconsistentMeasurementSnapshot);
    }
    let long_value = executable_long_price
        .value()
        .checked_mul(requested_base_quantity.value())
        .ok_or(GridInventoryError::Arithmetic)?;
    let short_value = executable_short_price
        .value()
        .checked_mul(requested_base_quantity.value())
        .ok_or(GridInventoryError::Arithmetic)?;
    let long_measured_notional = Notional::new(long_value)?;
    let short_measured_notional = Notional::new(short_value)?;
    let measured_matched_notional_cap = Notional::new(long_value.min(short_value))?;
    Ok(GridMeasurementInput {
        pair_id,
        symbol,
        long_venue,
        short_venue,
        source_deviation_bps,
        source_observed_at,
        source_measurement_model_version,
        source_measurement_config_fingerprint,
        requested_base_quantity,
        executable_long_price,
        executable_short_price,
        long_measured_notional,
        short_measured_notional,
        measured_matched_notional_cap,
        validity,
        tradable_edge_bps,
        increase_risk_economically_allowed,
    })
}

/// Auditable grid output before pair-level arbitration or inventory comparison.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GridRouteTarget {
    pair_id: PairId,
    symbol: Symbol,
    long_venue: VenueId,
    short_venue: VenueId,
    source_observed_at: UnixNanos,
    source_measurement_model_version: ModelVersion,
    source_measurement_config_fingerprint: String,
    source_deviation_bps: Bps,
    applied_boundary_bps: Option<Bps>,
    between_grid_rule: BetweenGridRule,
    target_fraction: TargetFraction,
    target_notional_per_leg: Notional,
    reason: String,
    model_version: ModelVersion,
    decision_id: DecisionId,
}

impl GridRouteTarget {
    #[must_use]
    pub fn pair_id(&self) -> &PairId {
        &self.pair_id
    }

    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    #[must_use]
    pub fn long_venue(&self) -> &VenueId {
        &self.long_venue
    }

    #[must_use]
    pub fn short_venue(&self) -> &VenueId {
        &self.short_venue
    }

    #[must_use]
    pub const fn source_observed_at(&self) -> UnixNanos {
        self.source_observed_at
    }

    #[must_use]
    pub fn source_measurement_model_version(&self) -> &ModelVersion {
        &self.source_measurement_model_version
    }

    #[must_use]
    pub fn source_measurement_config_fingerprint(&self) -> &str {
        &self.source_measurement_config_fingerprint
    }

    #[must_use]
    pub const fn source_deviation_bps(&self) -> Bps {
        self.source_deviation_bps
    }

    #[must_use]
    pub const fn applied_boundary_bps(&self) -> Option<Bps> {
        self.applied_boundary_bps
    }

    #[must_use]
    pub const fn between_grid_rule(&self) -> BetweenGridRule {
        self.between_grid_rule
    }

    #[must_use]
    pub fn decision_id(&self) -> &DecisionId {
        &self.decision_id
    }

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
    pub(crate) fn to_target_inventory(&self) -> Result<TargetInventory, InventoryDomainError> {
        TargetInventory::new(TargetInventoryParams {
            symbol: self.symbol.clone(),
            pair_id: self.pair_id.clone(),
            target_fraction: self.target_fraction,
            target_notional: self.target_notional_per_leg,
            direction: self.target_direction(),
            reason: self.reason.clone(),
            model_version: self.model_version.clone(),
            decision_id: self.decision_id.clone(),
        })
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GridInventoryError {
    #[error("grid configuration must be positive and strictly monotonic")]
    InvalidConfiguration,
    #[error("P3 opportunity evaluation contains inconsistent snapshot fields")]
    InconsistentMeasurementSnapshot,
    #[error("P3 opportunity evaluation does not contain a deviation")]
    MissingDeviation,
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

    /// Maps one immutable P3 measurement snapshot to a non-negative floor-step target.
    pub fn evaluate(
        &self,
        input: &GridMeasurementInput,
        decision_id: &DecisionId,
    ) -> Result<GridRouteTarget, GridInventoryError> {
        if input.long_venue == input.short_venue {
            return Err(GridInventoryError::SameVenue);
        }
        let applied = self
            .levels
            .iter()
            .take_while(|level| input.source_deviation_bps.value() >= level.deviation_bps.value())
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
            source_observed_at: input.source_observed_at,
            source_measurement_model_version: input.source_measurement_model_version.clone(),
            source_measurement_config_fingerprint: input
                .source_measurement_config_fingerprint
                .clone(),
            source_deviation_bps: input.source_deviation_bps,
            applied_boundary_bps: applied.map(|level| level.deviation_bps),
            between_grid_rule: BetweenGridRule::FloorStep,
            target_fraction,
            target_notional_per_leg: target_notional,
            reason,
            model_version: self.model_version.clone(),
            decision_id: decision_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{BetweenGridRule, GridInventoryModel, GridMeasurementInput};
    use crate::{
        config::parse_toml,
        domain::{
            ids::{DecisionId, ModelVersion, PairId, Symbol, VenueId},
            inventory::TargetDirection,
            numeric::{BaseQty, Bps, Price, UnixNanos},
            spread::MeasurementValidity,
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
        let measurement = GridMeasurementInput::for_test(
            fixture.pair_id.clone(),
            fixture.symbol.clone(),
            long_venue.clone(),
            short_venue.clone(),
            Bps::new(deviation),
            UnixNanos(100_000_000_000),
            ModelVersion::try_from("p3-measurement-v1")?,
            "measurement-config-sha256".to_owned(),
            BaseQty::new(Decimal::new(10, 2))?,
            Price::new(Decimal::new(100, 0))?,
            Price::new(Decimal::new(101, 0))?,
            MeasurementValidity::Valid,
            Some(Bps::new(Decimal::new(10, 0))),
            true,
        )?;
        let output = model.evaluate(&measurement, &fixture.decision_id)?;
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
                output
                    .applied_boundary_bps()
                    .expect("grid boundary")
                    .value(),
                Decimal::new(deviation, 0)
            );
        }
        Ok(())
    }

    #[test]
    fn between_boundaries_uses_conservative_floor_step() -> Result<(), Box<dyn Error>> {
        let (_, output) = target(Decimal::new(1499, 2), false)?;
        assert_eq!(output.target_fraction().value(), Decimal::new(40, 2));
        assert_eq!(output.between_grid_rule(), BetweenGridRule::FloorStep);
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
