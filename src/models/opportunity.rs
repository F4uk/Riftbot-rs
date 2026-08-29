//! Cost-adjusted measurement packaging. This module cannot emit inventory or submit orders.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::{MeasurementConfigFingerprint, RegimeConfig},
    domain::{
        ids::{IdentifierError, ModelVersion},
        numeric::Bps,
        opportunity::Opportunity,
        risk::Regime,
        spread::{
            ExecutableRouteMeasurement, ExplicitRiskCost, FundingState, MeasurementValidity,
            SpreadSnapshot,
        },
    },
    models::{
        P3_MODEL_VERSION,
        fair_value::{FairValueSnapshot, ReferenceRejection},
        regime::{RegimeFilter, RegimeInput},
    },
};

/// Both auditable spread facts and their measurement-only opportunity evaluation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpportunityEvaluation {
    pub spread: SpreadSnapshot,
    pub opportunity: Opportunity,
}

/// Inputs which remain explicit instead of being hidden in model state.
#[derive(Clone, Debug)]
pub struct OpportunityInput {
    pub executable: ExecutableRouteMeasurement,
    pub fair_value: FairValueSnapshot,
    pub other_explicit_risk_costs_bps: Vec<ExplicitRiskCost>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum OpportunityError {
    #[error("executable and fair-value route identities differ")]
    RouteMismatch,
    #[error("explicit risk cost name must be non-empty")]
    EmptyRiskCostName,
    #[error("explicit risk cost {name} must be non-negative")]
    NegativeRiskCost { name: String },
    #[error("fixed-decimal opportunity arithmetic failed")]
    Arithmetic,
    #[error("invalid P3 model version: {0}")]
    ModelVersion(#[from] IdentifierError),
}

/// Stateless economics evaluator plus deterministic regime classifier.
#[derive(Clone, Debug)]
pub struct OpportunityModel {
    regime_filter: RegimeFilter,
    model_version: ModelVersion,
    config_fingerprint: String,
}

impl OpportunityModel {
    pub fn new(
        regime_config: &RegimeConfig,
        config_fingerprint: &MeasurementConfigFingerprint,
    ) -> Result<Self, OpportunityError> {
        Ok(Self {
            regime_filter: RegimeFilter::new(regime_config),
            model_version: ModelVersion::try_from(P3_MODEL_VERSION)?,
            config_fingerprint: config_fingerprint.to_string(),
        })
    }

    pub fn evaluate(
        &self,
        input: OpportunityInput,
    ) -> Result<OpportunityEvaluation, OpportunityError> {
        let executable = input.executable;
        let fair_value = input.fair_value;
        if executable.pair_id != fair_value.route.pair_id
            || executable.long_venue != fair_value.route.long_venue
            || executable.short_venue != fair_value.route.short_venue
            || executable.observed_at != fair_value.tick_ts
        {
            return Err(OpportunityError::RouteMismatch);
        }
        validate_explicit_costs(&input.other_explicit_risk_costs_bps)?;

        let market_validity = fair_value_validity(&fair_value);
        let midline_bps = fair_value.midline_bps;
        let deviation_bps = if market_validity == MeasurementValidity::Valid {
            Some(Bps::new(
                executable
                    .raw_executable_premium_bps
                    .value()
                    .checked_sub(midline_bps.ok_or(OpportunityError::Arithmetic)?.value())
                    .ok_or(OpportunityError::Arithmetic)?,
            ))
        } else {
            None
        };
        let regime = self.regime_filter.classify(RegimeInput {
            market_validity,
            deviation_bps,
            dispersion_bps: fair_value.dispersion_bps,
        });
        let validity = final_validity(market_validity, &executable);
        let tradable_edge_bps = if validity == MeasurementValidity::Valid {
            Some(Bps::new(calculate_tradable_edge(
                deviation_bps.ok_or(OpportunityError::Arithmetic)?,
                executable.fee_bps.ok_or(OpportunityError::Arithmetic)?,
                executable.execution_buffer_bps,
                executable
                    .funding_adjustment_bps
                    .ok_or(OpportunityError::Arithmetic)?,
                &input.other_explicit_risk_costs_bps,
            )?))
        } else {
            None
        };
        let increase_risk_economically_allowed = validity.permits_increase()
            && matches!(regime, Regime::Normal | Regime::Degraded)
            && tradable_edge_bps.is_some_and(|edge| edge.value() > Decimal::ZERO);
        let rejection_reason = rejection_reason(validity, fair_value.rejection);

        let spread = SpreadSnapshot {
            pair_id: executable.pair_id.clone(),
            symbol: executable.symbol.clone(),
            long_venue: executable.long_venue.clone(),
            short_venue: executable.short_venue.clone(),
            long_instrument_id: executable.long_instrument_id.clone(),
            short_instrument_id: executable.short_instrument_id.clone(),
            requested_base_quantity: executable.requested_base_quantity,
            maximum_executable_base_quantity: executable.maximum_executable_base_quantity,
            executable_long_price: executable.executable_long_price,
            executable_short_price: executable.executable_short_price,
            executable_notional: executable.executable_notional,
            raw_executable_premium_bps: executable.raw_executable_premium_bps,
            midline_bps,
            deviation_bps,
            fee_bps: executable.fee_bps,
            depth_impact_bps: executable.depth_impact_bps,
            execution_buffer_bps: executable.execution_buffer_bps,
            funding_state: executable.funding_state,
            funding_adjustment_bps: executable.funding_adjustment_bps,
            other_explicit_risk_costs_bps: input.other_explicit_risk_costs_bps.clone(),
            tradable_edge_bps,
            fair_value_sample_count: fair_value.sample_count,
            fair_value_dispersion_bps: fair_value.dispersion_bps,
            long_age_ms: executable.long_age_ms,
            short_age_ms: executable.short_age_ms,
            receive_skew_ms: executable.receive_skew_ms,
            timestamp: executable.observed_at,
            validity,
            rejection_reason: rejection_reason.clone(),
            model_version: self.model_version.clone(),
            config_fingerprint: self.config_fingerprint.clone(),
        };
        let opportunity = Opportunity {
            pair_id: executable.pair_id,
            symbol: executable.symbol,
            long_venue: executable.long_venue,
            short_venue: executable.short_venue,
            executable_long_price: executable.executable_long_price,
            executable_short_price: executable.executable_short_price,
            executable_notional: executable.executable_notional,
            raw_executable_premium_bps: executable.raw_executable_premium_bps,
            midline_bps,
            deviation_bps,
            fee_bps: executable.fee_bps,
            depth_impact_bps: executable.depth_impact_bps,
            execution_buffer_bps: executable.execution_buffer_bps,
            funding_state: executable.funding_state,
            funding_adjustment_bps: executable.funding_adjustment_bps,
            other_explicit_risk_costs_bps: input.other_explicit_risk_costs_bps,
            tradable_edge_bps,
            regime,
            validity,
            rejection_reason,
            increase_risk_economically_allowed,
            timestamp: executable.observed_at,
            model_version: self.model_version.clone(),
            config_fingerprint: self.config_fingerprint.clone(),
        };
        Ok(OpportunityEvaluation {
            spread,
            opportunity,
        })
    }
}

fn validate_explicit_costs(costs: &[ExplicitRiskCost]) -> Result<(), OpportunityError> {
    for cost in costs {
        if cost.name.trim().is_empty() {
            return Err(OpportunityError::EmptyRiskCostName);
        }
        if cost.bps.value() < Decimal::ZERO {
            return Err(OpportunityError::NegativeRiskCost {
                name: cost.name.clone(),
            });
        }
    }
    Ok(())
}

fn fair_value_validity(fair_value: &FairValueSnapshot) -> MeasurementValidity {
    if let Some(rejection) = fair_value.rejection {
        return match rejection {
            ReferenceRejection::VenueUnhealthy => MeasurementValidity::VenueUnhealthy,
            ReferenceRejection::StaleBook | ReferenceRejection::FutureReceiveTimestamp => {
                MeasurementValidity::StaleBook
            }
            ReferenceRejection::ReceiveTimeSkew => MeasurementValidity::ReceiveTimeSkew,
            ReferenceRejection::EmptyBookSide => MeasurementValidity::EmptyBookSide,
            ReferenceRejection::CrossedOrCorruptBook => MeasurementValidity::CrossedOrCorruptBook,
        };
    }
    if !fair_value.warmed_up {
        return MeasurementValidity::WarmingUp;
    }
    if fair_value.midline_bps.is_none() || fair_value.dispersion_bps.is_none() {
        return MeasurementValidity::InvalidFairValue;
    }
    MeasurementValidity::Valid
}

fn final_validity(
    market_validity: MeasurementValidity,
    executable: &ExecutableRouteMeasurement,
) -> MeasurementValidity {
    if market_validity != MeasurementValidity::Valid {
        return market_validity;
    }
    if executable.fee_bps.is_none() {
        return MeasurementValidity::FeeUnavailable;
    }
    if executable.funding_state == FundingState::Unavailable
        || executable.funding_adjustment_bps.is_none()
    {
        return MeasurementValidity::FundingUnavailable;
    }
    MeasurementValidity::Valid
}

fn calculate_tradable_edge(
    deviation: Bps,
    fees: Bps,
    execution_buffer: Bps,
    funding: Bps,
    other_costs: &[ExplicitRiskCost],
) -> Result<Decimal, OpportunityError> {
    let other_total = other_costs.iter().try_fold(Decimal::ZERO, |total, cost| {
        total
            .checked_add(cost.bps.value())
            .ok_or(OpportunityError::Arithmetic)
    })?;
    deviation
        .value()
        .checked_sub(fees.value())
        .and_then(|edge| edge.checked_sub(execution_buffer.value()))
        .and_then(|edge| edge.checked_add(funding.value()))
        .and_then(|edge| edge.checked_sub(other_total))
        .ok_or(OpportunityError::Arithmetic)
}

fn rejection_reason(
    validity: MeasurementValidity,
    reference_rejection: Option<ReferenceRejection>,
) -> Option<String> {
    if validity == MeasurementValidity::Valid {
        return None;
    }
    Some(
        match reference_rejection {
            Some(ReferenceRejection::VenueUnhealthy) => "fair_value_venue_unhealthy",
            Some(ReferenceRejection::StaleBook) => "fair_value_stale_book",
            Some(ReferenceRejection::ReceiveTimeSkew) => "fair_value_receive_time_skew",
            Some(ReferenceRejection::EmptyBookSide) => "fair_value_empty_book_side",
            Some(ReferenceRejection::CrossedOrCorruptBook) => "fair_value_crossed_or_corrupt_book",
            Some(ReferenceRejection::FutureReceiveTimestamp) => {
                "fair_value_future_receive_timestamp"
            }
            None => match validity {
                MeasurementValidity::Valid => unreachable!("validity handled above"),
                MeasurementValidity::WarmingUp => "warming_up",
                MeasurementValidity::StaleBook => "stale_book",
                MeasurementValidity::ReceiveTimeSkew => "receive_time_skew",
                MeasurementValidity::EmptyBookSide => "empty_book_side",
                MeasurementValidity::InsufficientDepth => "insufficient_depth",
                MeasurementValidity::CrossedOrCorruptBook => "crossed_or_corrupt_book",
                MeasurementValidity::VenueUnhealthy => "venue_unhealthy",
                MeasurementValidity::FeeUnavailable => "fee_unavailable",
                MeasurementValidity::FundingUnavailable => "funding_unavailable",
                MeasurementValidity::InvalidFairValue => "invalid_fair_value",
            },
        }
        .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{OpportunityInput, OpportunityModel};
    use crate::{
        config::{RegimeConfig, measurement_config_fingerprint, parse_toml},
        domain::{
            ids::{InstrumentId, PairId, Symbol, VenueId},
            numeric::{BaseQty, Bps, DurationMillis, Notional, Price, UnixNanos},
            risk::Regime,
            spread::{
                ExecutableRouteMeasurement, ExplicitRiskCost, FundingState, MeasurementValidity,
            },
        },
        models::fair_value::{FairValueSnapshot, OrientedRouteKey},
    };

    const EXAMPLE: &str = include_str!("../../config/example.toml");

    fn model() -> Result<OpportunityModel, Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        Ok(OpportunityModel::new(
            &RegimeConfig {
                degraded_dispersion_bps: Bps::new(Decimal::from(8)),
                reduce_only_deviation_bps: Bps::new(Decimal::from(50)),
                halted_deviation_bps: Bps::new(Decimal::from(100)),
            },
            &measurement_config_fingerprint(&config)?,
        )?)
    }

    fn executable(raw_bps: i64) -> Result<ExecutableRouteMeasurement, Box<dyn Error>> {
        Ok(ExecutableRouteMeasurement {
            pair_id: PairId::try_from("sndk_pair")?,
            symbol: Symbol::try_from("SNDK")?,
            long_venue: VenueId::try_from("entropy")?,
            short_venue: VenueId::try_from("lighter")?,
            long_instrument_id: InstrumentId::try_from("SNDK-PERP.entropy")?,
            short_instrument_id: InstrumentId::try_from("SNDK-PERP.lighter")?,
            requested_base_quantity: BaseQty::new(Decimal::ONE)?,
            maximum_executable_base_quantity: BaseQty::new(Decimal::from(10))?,
            executable_long_price: Price::new(Decimal::from(100))?,
            executable_short_price: Price::new(
                Decimal::from(100)
                    * (Decimal::ONE + Decimal::from(raw_bps) / Decimal::from(10_000)),
            )?,
            executable_notional: Notional::new(Decimal::from(100))?,
            raw_executable_premium_bps: Bps::new(Decimal::from(raw_bps)),
            fee_bps: Some(Bps::new(Decimal::from(2))),
            depth_impact_bps: Bps::new(Decimal::from(7)),
            execution_buffer_bps: Bps::new(Decimal::from(2)),
            funding_state: FundingState::Disabled,
            funding_adjustment_bps: Some(Bps::new(Decimal::ZERO)),
            long_exchange_ts: UnixNanos(999),
            short_exchange_ts: UnixNanos(999),
            long_receive_ts: UnixNanos(1_000),
            short_receive_ts: UnixNanos(1_000),
            long_age_ms: DurationMillis(0),
            short_age_ms: DurationMillis(0),
            receive_skew_ms: DurationMillis(0),
            observed_at: UnixNanos(1_000),
        })
    }

    fn fair_value(midline: i64) -> Result<FairValueSnapshot, Box<dyn Error>> {
        Ok(FairValueSnapshot {
            route: OrientedRouteKey::new(
                PairId::try_from("sndk_pair")?,
                VenueId::try_from("entropy")?,
                VenueId::try_from("lighter")?,
            ),
            tick_ts: UnixNanos(1_000),
            reference_basis_bps: Some(Bps::new(Decimal::from(midline))),
            midline_bps: Some(Bps::new(Decimal::from(midline))),
            dispersion_bps: Some(Bps::new(Decimal::ONE)),
            sample_count: 300,
            minimum_samples: 300,
            warmed_up: true,
            rejection: None,
        })
    }

    #[test]
    fn gate_example_a_must_not_increase_risk() -> Result<(), Box<dyn Error>> {
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: executable(20)?,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            evaluation.opportunity.deviation_bps.map(Bps::value),
            Some(Decimal::from(2))
        );
        assert_eq!(
            evaluation.opportunity.tradable_edge_bps.map(Bps::value),
            Some(Decimal::from(-2))
        );
        assert!(!evaluation.opportunity.increase_risk_economically_allowed);
        Ok(())
    }

    #[test]
    fn gate_example_b_may_increase_risk() -> Result<(), Box<dyn Error>> {
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: executable(43)?,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            evaluation.opportunity.deviation_bps.map(Bps::value),
            Some(Decimal::from(25))
        );
        assert_eq!(
            evaluation.opportunity.tradable_edge_bps.map(Bps::value),
            Some(Decimal::from(21))
        );
        assert!(evaluation.opportunity.increase_risk_economically_allowed);
        Ok(())
    }

    #[test]
    fn depth_impact_is_not_deducted_again() -> Result<(), Box<dyn Error>> {
        let mut measured = executable(43)?;
        measured.depth_impact_bps = Bps::new(Decimal::from(999));
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: measured,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            evaluation.opportunity.tradable_edge_bps.map(Bps::value),
            Some(Decimal::from(21))
        );
        Ok(())
    }

    #[test]
    fn unavailable_funding_fails_closed() -> Result<(), Box<dyn Error>> {
        let mut measured = executable(43)?;
        measured.funding_state = FundingState::Unavailable;
        measured.funding_adjustment_bps = None;
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: measured,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            evaluation.opportunity.validity,
            MeasurementValidity::FundingUnavailable
        );
        assert_eq!(evaluation.opportunity.tradable_edge_bps, None);
        assert!(!evaluation.opportunity.increase_risk_economically_allowed);
        assert_eq!(evaluation.opportunity.regime, Regime::Normal);
        Ok(())
    }

    #[test]
    fn named_risk_cost_is_deducted_once() -> Result<(), Box<dyn Error>> {
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: executable(43)?,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: vec![ExplicitRiskCost {
                name: "basis_uncertainty".to_owned(),
                bps: Bps::new(Decimal::from(3)),
            }],
        })?;
        assert_eq!(
            evaluation.opportunity.tradable_edge_bps.map(Bps::value),
            Some(Decimal::from(18))
        );
        Ok(())
    }

    #[test]
    fn warmup_never_invents_a_zero_midline() -> Result<(), Box<dyn Error>> {
        let mut warming = fair_value(18)?;
        warming.warmed_up = false;
        warming.midline_bps = None;
        warming.dispersion_bps = None;
        warming.sample_count = 10;
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: executable(43)?,
            fair_value: warming,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            evaluation.opportunity.validity,
            MeasurementValidity::WarmingUp
        );
        assert_eq!(evaluation.opportunity.midline_bps, None);
        assert_eq!(evaluation.opportunity.deviation_bps, None);
        assert_eq!(evaluation.opportunity.tradable_edge_bps, None);
        Ok(())
    }

    #[test]
    fn signed_funding_adjustment_changes_edge_with_its_sign() -> Result<(), Box<dyn Error>> {
        let mut favorable = executable(43)?;
        favorable.funding_state = FundingState::Available;
        favorable.funding_adjustment_bps = Some(Bps::new(Decimal::from(3)));
        let positive = model()?.evaluate(OpportunityInput {
            executable: favorable,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            positive.opportunity.tradable_edge_bps.map(Bps::value),
            Some(Decimal::from(24))
        );

        let mut adverse = executable(43)?;
        adverse.funding_state = FundingState::Available;
        adverse.funding_adjustment_bps = Some(Bps::new(Decimal::from(-3)));
        let negative = model()?.evaluate(OpportunityInput {
            executable: adverse,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(
            negative.opportunity.tradable_edge_bps.map(Bps::value),
            Some(Decimal::from(18))
        );
        Ok(())
    }

    #[test]
    fn extreme_deviation_halts_economic_increase() -> Result<(), Box<dyn Error>> {
        let evaluation = model()?.evaluate(OpportunityInput {
            executable: executable(118)?,
            fair_value: fair_value(18)?,
            other_explicit_risk_costs_bps: Vec::new(),
        })?;
        assert_eq!(evaluation.opportunity.regime, Regime::Halted);
        assert!(!evaluation.opportunity.increase_risk_economically_allowed);
        Ok(())
    }
}
