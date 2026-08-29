//! P4 target-versus-effective-actual comparison. This module never creates execution intents.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    domain::{
        ids::{DecisionId, ModelVersion, PairId, Symbol},
        inventory::{EffectiveActual, EffectiveInventory, InventoryDomainError, TargetInventory},
        numeric::{BaseQty, Money, Notional, NumericError, UnixNanos},
        spread::MeasurementValidity,
    },
    models::grid_inventory::{GridMeasurementInput, GridRouteTarget},
};

/// Exact measurement-size ceiling attached to an accepted increase proposal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IncreaseSizeBasis {
    pub requested_base_quantity: BaseQty,
    pub long_measured_notional: Notional,
    pub short_measured_notional: Notional,
    pub measured_matched_notional_cap: Notional,
    pub observed_at: UnixNanos,
    pub measurement_model_version: ModelVersion,
    pub measurement_config_fingerprint: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryAction {
    NoChange,
    IncreaseRisk,
    ReduceRisk,
    FlattenForReversal,
    IncreaseBlocked,
    AmbiguousOpposingIncrease,
    AmbiguousEffectiveInventory,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncreaseBlockReason {
    MeasurementMissing,
    MeasurementRouteMismatch,
    MeasurementObservedAtMismatch,
    MeasurementConfigFingerprintMismatch,
    MeasurementModelVersionMismatch,
    MeasurementDeviationMismatch,
    MeasurementInvalid,
    NonPositiveTradableEdge,
    EconomicPermissionDenied,
    MeasuredSizeUnavailable,
    OpposingRouteIncrease,
    OpposingEffectiveExposure,
}

/// P4-only proposal. It is neither a RiskDecision nor an ExecutionIntent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryDecision {
    pub decision_id: DecisionId,
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub action: InventoryAction,
    pub selected_target: Option<TargetInventory>,
    pub effective_actual: Option<EffectiveActual>,
    pub required_change_notional_per_leg: Money,
    pub proposed_change_notional_per_leg: Notional,
    pub increase_size_basis: Option<IncreaseSizeBasis>,
    pub block_reason: Option<IncreaseBlockReason>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum InventoryManagerError {
    #[error("forward/reverse grid target identity or orientation is inconsistent")]
    TargetIdentityMismatch,
    #[error("effective inventory identity does not match the target pair")]
    EffectiveInventoryIdentityMismatch,
    #[error("effective inventory contains a route outside the configured pair")]
    UnexpectedEffectiveRoute,
    #[error("grid target direction or zero shape is invalid")]
    InvalidTargetShape,
    #[error("fixed-decimal target-versus-actual arithmetic failed")]
    Arithmetic,
    #[error(transparent)]
    Inventory(#[from] InventoryDomainError),
    #[error(transparent)]
    Numeric(#[from] NumericError),
}

/// Stateless P4 comparison and route-arbitration boundary.
#[derive(Clone, Copy, Debug, Default)]
pub struct InventoryManager;

impl InventoryManager {
    #[allow(clippy::too_many_arguments)]
    pub fn decide(
        &self,
        forward: &GridRouteTarget,
        reverse: &GridRouteTarget,
        effective: &EffectiveInventory,
        forward_measurement: Option<&GridMeasurementInput>,
        reverse_measurement: Option<&GridMeasurementInput>,
    ) -> Result<InventoryDecision, InventoryManagerError> {
        validate_inputs(forward, reverse, effective)?;
        let forward_actual = effective.project(forward.long_venue(), forward.short_venue())?;
        let reverse_actual = effective.project(reverse.long_venue(), reverse.short_venue())?;
        let forward_current = forward_actual.total_notional_per_leg.value();
        let reverse_current = reverse_actual.total_notional_per_leg.value();

        if forward_current > Decimal::ZERO && reverse_current > Decimal::ZERO {
            return neutral_decision(
                forward,
                InventoryAction::AmbiguousEffectiveInventory,
                IncreaseBlockReason::OpposingEffectiveExposure,
            );
        }

        let forward_increase = forward.target_notional_per_leg().value() > forward_current;
        let reverse_increase = reverse.target_notional_per_leg().value() > reverse_current;
        if forward_increase && reverse_increase {
            return neutral_decision(
                forward,
                InventoryAction::AmbiguousOpposingIncrease,
                IncreaseBlockReason::OpposingRouteIncrease,
            );
        }

        if forward_current > Decimal::ZERO {
            return self.decide_with_current(
                forward,
                reverse,
                &forward_actual,
                reverse_increase,
                reverse_measurement,
                forward_measurement,
            );
        }
        if reverse_current > Decimal::ZERO {
            return self.decide_with_current(
                reverse,
                forward,
                &reverse_actual,
                forward_increase,
                forward_measurement,
                reverse_measurement,
            );
        }
        if forward_increase {
            return increase_decision(forward, &forward_actual, forward_measurement);
        }
        if reverse_increase {
            return increase_decision(reverse, &reverse_actual, reverse_measurement);
        }
        no_change_decision(forward, forward_actual)
    }

    fn decide_with_current(
        &self,
        current_route: &GridRouteTarget,
        opposite_route: &GridRouteTarget,
        current: &EffectiveActual,
        opposite_increase: bool,
        opposite_measurement: Option<&GridMeasurementInput>,
        current_measurement: Option<&GridMeasurementInput>,
    ) -> Result<InventoryDecision, InventoryManagerError> {
        let current_value = current.total_notional_per_leg.value();
        let target_value = current_route.target_notional_per_leg().value();
        if opposite_increase && measurement_basis(opposite_route, opposite_measurement).is_ok() {
            return Ok(InventoryDecision {
                decision_id: current_route.decision_id().clone(),
                pair_id: current_route.pair_id().clone(),
                symbol: current_route.symbol().clone(),
                action: InventoryAction::FlattenForReversal,
                selected_target: Some(opposite_route.to_target_inventory()?),
                effective_actual: Some(current.clone()),
                required_change_notional_per_leg: Money::new(-current_value),
                proposed_change_notional_per_leg: current.total_notional_per_leg,
                increase_size_basis: None,
                block_reason: None,
            });
        }
        if target_value < current_value {
            return reduction_decision(current_route, current);
        }
        if target_value == current_value {
            return no_change_decision(current_route, current.clone());
        }
        increase_decision(current_route, current, current_measurement)
    }
}

fn validate_inputs(
    forward: &GridRouteTarget,
    reverse: &GridRouteTarget,
    effective: &EffectiveInventory,
) -> Result<(), InventoryManagerError> {
    if forward.pair_id() != reverse.pair_id()
        || forward.symbol() != reverse.symbol()
        || forward.long_venue() != reverse.short_venue()
        || forward.short_venue() != reverse.long_venue()
        || forward.decision_id() != reverse.decision_id()
    {
        return Err(InventoryManagerError::TargetIdentityMismatch);
    }
    validate_target_shape(forward)?;
    validate_target_shape(reverse)?;
    effective.validate()?;
    if &effective.pair_id != forward.pair_id() || &effective.symbol != forward.symbol() {
        return Err(InventoryManagerError::EffectiveInventoryIdentityMismatch);
    }
    if effective.exposures.iter().any(|exposure| {
        !((&exposure.long_venue == forward.long_venue()
            && &exposure.short_venue == forward.short_venue())
            || (&exposure.long_venue == reverse.long_venue()
                && &exposure.short_venue == reverse.short_venue()))
    }) {
        return Err(InventoryManagerError::UnexpectedEffectiveRoute);
    }
    Ok(())
}

fn validate_target_shape(target: &GridRouteTarget) -> Result<(), InventoryManagerError> {
    let fraction_is_zero = target.target_fraction().value() == Decimal::ZERO;
    let notional_is_zero = target.target_notional_per_leg().value() == Decimal::ZERO;
    if target.long_venue() == target.short_venue() || fraction_is_zero != notional_is_zero {
        return Err(InventoryManagerError::InvalidTargetShape);
    }
    let direction = target.target_direction();
    if (fraction_is_zero && !direction.is_flat())
        || (!fraction_is_zero
            && !direction.matches_route(target.long_venue(), target.short_venue()))
    {
        return Err(InventoryManagerError::InvalidTargetShape);
    }
    Ok(())
}

fn measurement_basis(
    target: &GridRouteTarget,
    measurement: Option<&GridMeasurementInput>,
) -> Result<IncreaseSizeBasis, IncreaseBlockReason> {
    let measurement = measurement.ok_or(IncreaseBlockReason::MeasurementMissing)?;
    if measurement.pair_id() != target.pair_id()
        || measurement.symbol() != target.symbol()
        || measurement.long_venue() != target.long_venue()
        || measurement.short_venue() != target.short_venue()
    {
        return Err(IncreaseBlockReason::MeasurementRouteMismatch);
    }
    if measurement.source_observed_at() != target.source_observed_at() {
        return Err(IncreaseBlockReason::MeasurementObservedAtMismatch);
    }
    if measurement.source_measurement_config_fingerprint()
        != target.source_measurement_config_fingerprint()
    {
        return Err(IncreaseBlockReason::MeasurementConfigFingerprintMismatch);
    }
    if measurement.source_measurement_model_version() != target.source_measurement_model_version() {
        return Err(IncreaseBlockReason::MeasurementModelVersionMismatch);
    }
    if measurement.source_deviation_bps() != target.source_deviation_bps() {
        return Err(IncreaseBlockReason::MeasurementDeviationMismatch);
    }
    if measurement.validity() != MeasurementValidity::Valid {
        return Err(IncreaseBlockReason::MeasurementInvalid);
    }
    if !measurement
        .tradable_edge_bps()
        .is_some_and(|edge| edge.value() > Decimal::ZERO)
    {
        return Err(IncreaseBlockReason::NonPositiveTradableEdge);
    }
    if !measurement.increase_risk_economically_allowed() {
        return Err(IncreaseBlockReason::EconomicPermissionDenied);
    }
    if measurement.measured_matched_notional_cap().value() <= Decimal::ZERO {
        return Err(IncreaseBlockReason::MeasuredSizeUnavailable);
    }
    Ok(IncreaseSizeBasis {
        requested_base_quantity: measurement.requested_base_quantity(),
        long_measured_notional: measurement.long_measured_notional(),
        short_measured_notional: measurement.short_measured_notional(),
        measured_matched_notional_cap: measurement.measured_matched_notional_cap(),
        observed_at: measurement.source_observed_at(),
        measurement_model_version: measurement.source_measurement_model_version().clone(),
        measurement_config_fingerprint: measurement
            .source_measurement_config_fingerprint()
            .to_owned(),
    })
}

fn increase_decision(
    target: &GridRouteTarget,
    current: &EffectiveActual,
    measurement: Option<&GridMeasurementInput>,
) -> Result<InventoryDecision, InventoryManagerError> {
    let required = target
        .target_notional_per_leg()
        .value()
        .checked_sub(current.total_notional_per_leg.value())
        .ok_or(InventoryManagerError::Arithmetic)?;
    match measurement_basis(target, measurement) {
        Ok(basis) => {
            let proposed = required.min(basis.measured_matched_notional_cap.value());
            Ok(InventoryDecision {
                decision_id: target.decision_id().clone(),
                pair_id: target.pair_id().clone(),
                symbol: target.symbol().clone(),
                action: InventoryAction::IncreaseRisk,
                selected_target: Some(target.to_target_inventory()?),
                effective_actual: Some(current.clone()),
                required_change_notional_per_leg: Money::new(required),
                proposed_change_notional_per_leg: Notional::new(proposed)?,
                increase_size_basis: Some(basis),
                block_reason: None,
            })
        }
        Err(reason) => Ok(InventoryDecision {
            decision_id: target.decision_id().clone(),
            pair_id: target.pair_id().clone(),
            symbol: target.symbol().clone(),
            action: InventoryAction::IncreaseBlocked,
            selected_target: Some(target.to_target_inventory()?),
            effective_actual: Some(current.clone()),
            required_change_notional_per_leg: Money::new(required),
            proposed_change_notional_per_leg: Notional::new(Decimal::ZERO)?,
            increase_size_basis: None,
            block_reason: Some(reason),
        }),
    }
}

fn reduction_decision(
    target: &GridRouteTarget,
    current: &EffectiveActual,
) -> Result<InventoryDecision, InventoryManagerError> {
    let required = target
        .target_notional_per_leg()
        .value()
        .checked_sub(current.total_notional_per_leg.value())
        .ok_or(InventoryManagerError::Arithmetic)?;
    Ok(InventoryDecision {
        decision_id: target.decision_id().clone(),
        pair_id: target.pair_id().clone(),
        symbol: target.symbol().clone(),
        action: InventoryAction::ReduceRisk,
        selected_target: Some(target.to_target_inventory()?),
        effective_actual: Some(current.clone()),
        required_change_notional_per_leg: Money::new(required),
        proposed_change_notional_per_leg: Notional::new(-required)?,
        increase_size_basis: None,
        block_reason: None,
    })
}

fn no_change_decision(
    target: &GridRouteTarget,
    current: EffectiveActual,
) -> Result<InventoryDecision, InventoryManagerError> {
    Ok(InventoryDecision {
        decision_id: target.decision_id().clone(),
        pair_id: target.pair_id().clone(),
        symbol: target.symbol().clone(),
        action: InventoryAction::NoChange,
        selected_target: Some(target.to_target_inventory()?),
        effective_actual: Some(current),
        required_change_notional_per_leg: Money::new(Decimal::ZERO),
        proposed_change_notional_per_leg: Notional::new(Decimal::ZERO)?,
        increase_size_basis: None,
        block_reason: None,
    })
}

fn neutral_decision(
    target: &GridRouteTarget,
    action: InventoryAction,
    block_reason: IncreaseBlockReason,
) -> Result<InventoryDecision, InventoryManagerError> {
    Ok(InventoryDecision {
        decision_id: target.decision_id().clone(),
        pair_id: target.pair_id().clone(),
        symbol: target.symbol().clone(),
        action,
        selected_target: None,
        effective_actual: None,
        required_change_notional_per_leg: Money::new(Decimal::ZERO),
        proposed_change_notional_per_leg: Notional::new(Decimal::ZERO)?,
        increase_size_basis: None,
        block_reason: Some(block_reason),
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{IncreaseBlockReason, InventoryAction, InventoryManager};
    use crate::{
        config::parse_toml,
        domain::{
            ids::{DecisionId, ModelVersion, PairId, Symbol, VenueId},
            inventory::{EffectiveInventory, OrientedExposure, TargetDirection},
            numeric::{BaseQty, Bps, Notional, Price, UnixNanos},
            spread::MeasurementValidity,
        },
        models::grid_inventory::{GridInventoryModel, GridMeasurementInput, GridRouteTarget},
    };

    const EXAMPLE: &str = include_str!("../../config/example.toml");
    const OBSERVED_AT: UnixNanos = UnixNanos(123_000_000_000);
    const MODEL_VERSION: &str = "p3-measurement-v1";
    const CONFIG_FINGERPRINT: &str = "measurement-config-sha256";

    struct TargetPair {
        forward: GridRouteTarget,
        reverse: GridRouteTarget,
        forward_measurement: GridMeasurementInput,
        reverse_measurement: GridMeasurementInput,
    }

    fn targets(
        forward_deviation: Decimal,
        reverse_deviation: Decimal,
    ) -> Result<TargetPair, Box<dyn Error>> {
        targets_with_forward_measurement(
            forward_deviation,
            reverse_deviation,
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(10, 0)),
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn targets_with_forward_measurement(
        forward_deviation: Decimal,
        reverse_deviation: Decimal,
        forward_quantity: Decimal,
        forward_long_price: Decimal,
        forward_short_price: Decimal,
        forward_edge: Option<Decimal>,
        forward_allowed: bool,
    ) -> Result<TargetPair, Box<dyn Error>> {
        let config = parse_toml(EXAMPLE)?;
        let model = GridInventoryModel::new(&config.grid, &config.strategy)?;
        let pair_id = PairId::try_from("SNDK-ENTROPY-LIGHTER")?;
        let symbol = Symbol::try_from("SNDK")?;
        let entropy = VenueId::try_from("entropy")?;
        let lighter = VenueId::try_from("lighter")?;
        let decision_id = DecisionId::try_from("inventory-manager-test")?;
        let forward_measurement = measurement(
            &pair_id,
            &symbol,
            &entropy,
            &lighter,
            forward_deviation,
            OBSERVED_AT,
            MODEL_VERSION,
            CONFIG_FINGERPRINT,
            forward_quantity,
            forward_long_price,
            forward_short_price,
            forward_edge,
            forward_allowed,
        )?;
        let reverse_measurement = measurement(
            &pair_id,
            &symbol,
            &lighter,
            &entropy,
            reverse_deviation,
            OBSERVED_AT,
            MODEL_VERSION,
            CONFIG_FINGERPRINT,
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(10, 0)),
            true,
        )?;
        Ok(TargetPair {
            forward: model.evaluate(&forward_measurement, &decision_id)?,
            reverse: model.evaluate(&reverse_measurement, &decision_id)?,
            forward_measurement,
            reverse_measurement,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn measurement(
        pair_id: &PairId,
        symbol: &Symbol,
        long_venue: &VenueId,
        short_venue: &VenueId,
        deviation: Decimal,
        observed_at: UnixNanos,
        model_version: &str,
        config_fingerprint: &str,
        requested_quantity: Decimal,
        long_price: Decimal,
        short_price: Decimal,
        edge: Option<Decimal>,
        allowed: bool,
    ) -> Result<GridMeasurementInput, Box<dyn Error>> {
        Ok(GridMeasurementInput::for_test(
            pair_id.clone(),
            symbol.clone(),
            long_venue.clone(),
            short_venue.clone(),
            Bps::new(deviation),
            observed_at,
            ModelVersion::try_from(model_version)?,
            config_fingerprint.to_owned(),
            BaseQty::new(requested_quantity)?,
            Price::new(long_price)?,
            Price::new(short_price)?,
            MeasurementValidity::Valid,
            edge.map(Bps::new),
            allowed,
        )?)
    }

    #[allow(clippy::too_many_arguments)]
    fn measurement_for_target(
        target: &GridRouteTarget,
        observed_at: UnixNanos,
        model_version: &str,
        config_fingerprint: &str,
        deviation: Decimal,
        requested_quantity: Decimal,
        long_price: Decimal,
        short_price: Decimal,
        edge: Option<Decimal>,
        allowed: bool,
    ) -> Result<GridMeasurementInput, Box<dyn Error>> {
        measurement(
            target.pair_id(),
            target.symbol(),
            target.long_venue(),
            target.short_venue(),
            deviation,
            observed_at,
            model_version,
            config_fingerprint,
            requested_quantity,
            long_price,
            short_price,
            edge,
            allowed,
        )
    }

    fn exposure(
        route: &GridRouteTarget,
        actual: Decimal,
        reserved: Decimal,
        pending: Decimal,
    ) -> Result<OrientedExposure, Box<dyn Error>> {
        Ok(OrientedExposure::new(
            route.long_venue().clone(),
            route.short_venue().clone(),
            Notional::new(actual)?,
            Notional::new(reserved)?,
            Notional::new(pending)?,
        )?)
    }

    fn effective(
        targets: &TargetPair,
        exposures: Vec<OrientedExposure>,
    ) -> Result<EffectiveInventory, Box<dyn Error>> {
        Ok(EffectiveInventory::new(
            targets.forward.symbol().clone(),
            targets.forward.pair_id().clone(),
            exposures,
        )?)
    }

    #[test]
    fn target_above_effective_actual_proposes_only_measured_size() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(20, 0), Decimal::ZERO)?;
        let effective = effective(
            &targets,
            vec![exposure(
                &targets.forward,
                Decimal::new(200, 0),
                Decimal::new(50, 0),
                Decimal::new(25, 0),
            )?],
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective,
            Some(&targets.forward_measurement),
            None,
        )?;
        assert_eq!(decision.action, InventoryAction::IncreaseRisk);
        assert_eq!(
            decision.required_change_notional_per_leg.value(),
            Decimal::new(125, 0)
        );
        assert_eq!(
            decision.proposed_change_notional_per_leg.value(),
            Decimal::new(50, 0)
        );
        let basis = decision.increase_size_basis.expect("increase size basis");
        assert_eq!(
            basis.measured_matched_notional_cap.value(),
            Decimal::new(50, 0)
        );
        assert_eq!(basis.long_measured_notional.value(), Decimal::new(50, 0));
        assert_eq!(basis.short_measured_notional.value(), Decimal::new(505, 1));
        assert_eq!(basis.requested_base_quantity.value(), Decimal::new(5, 1));
        Ok(())
    }

    #[test]
    fn target_below_actual_reduces_without_entry_measurement() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let effective = effective(
            &targets,
            vec![exposure(
                &targets.forward,
                Decimal::new(300, 0),
                Decimal::ZERO,
                Decimal::ZERO,
            )?],
        )?;
        let decision =
            InventoryManager.decide(&targets.forward, &targets.reverse, &effective, None, None)?;
        assert_eq!(decision.action, InventoryAction::ReduceRisk);
        assert_eq!(
            decision.required_change_notional_per_leg.value(),
            Decimal::new(-100, 0)
        );
        assert_eq!(
            decision.proposed_change_notional_per_leg.value(),
            Decimal::new(100, 0)
        );
        assert!(decision.increase_size_basis.is_none());
        Ok(())
    }

    #[test]
    fn non_positive_tradable_edge_blocks_only_added_risk() -> Result<(), Box<dyn Error>> {
        let targets = targets_with_forward_measurement(
            Decimal::new(10, 0),
            Decimal::ZERO,
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::ZERO),
            false,
        )?;
        let effective = effective(&targets, Vec::new())?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective,
            Some(&targets.forward_measurement),
            None,
        )?;
        assert_eq!(decision.action, InventoryAction::IncreaseBlocked);
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::NonPositiveTradableEdge)
        );
        assert_eq!(
            decision.proposed_change_notional_per_leg.value(),
            Decimal::ZERO
        );
        Ok(())
    }

    #[test]
    fn bad_edge_does_not_block_necessary_reduction() -> Result<(), Box<dyn Error>> {
        let targets = targets_with_forward_measurement(
            Decimal::new(5, 0),
            Decimal::ZERO,
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(-5, 0)),
            false,
        )?;
        let effective = effective(
            &targets,
            vec![exposure(
                &targets.forward,
                Decimal::new(200, 0),
                Decimal::ZERO,
                Decimal::ZERO,
            )?],
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective,
            Some(&targets.forward_measurement),
            None,
        )?;
        assert_eq!(decision.action, InventoryAction::ReduceRisk);
        assert_eq!(
            decision.proposed_change_notional_per_leg.value(),
            Decimal::new(100, 0)
        );
        Ok(())
    }

    #[test]
    fn opposing_increase_requests_fail_closed_explicitly() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(10, 0), Decimal::new(15, 0))?;
        let effective = effective(&targets, Vec::new())?;
        let decision =
            InventoryManager.decide(&targets.forward, &targets.reverse, &effective, None, None)?;
        assert_eq!(decision.action, InventoryAction::AmbiguousOpposingIncrease);
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::OpposingRouteIncrease)
        );
        assert!(decision.selected_target.is_none());
        Ok(())
    }

    #[test]
    fn reversal_flattens_old_route_before_opposite_increase() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::ZERO, Decimal::new(10, 0))?;
        let current = effective(
            &targets,
            vec![exposure(
                &targets.forward,
                Decimal::new(300, 0),
                Decimal::ZERO,
                Decimal::ZERO,
            )?],
        )?;
        let flatten = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &current,
            None,
            Some(&targets.reverse_measurement),
        )?;
        assert_eq!(flatten.action, InventoryAction::FlattenForReversal);
        assert_eq!(
            flatten.required_change_notional_per_leg.value(),
            Decimal::new(-300, 0)
        );
        assert_eq!(
            flatten.proposed_change_notional_per_leg.value(),
            Decimal::new(300, 0)
        );
        assert!(flatten.increase_size_basis.is_none());

        let flat = effective(&targets, Vec::new())?;
        let increase = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &flat,
            None,
            Some(&targets.reverse_measurement),
        )?;
        assert_eq!(increase.action, InventoryAction::IncreaseRisk);
        assert_eq!(
            increase
                .selected_target
                .expect("reverse target")
                .direction(),
            &TargetDirection::LongShort {
                long_venue: targets.reverse.long_venue().clone(),
                short_venue: targets.reverse.short_venue().clone(),
            }
        );
        Ok(())
    }

    #[test]
    fn stale_positive_measurement_cannot_authorize_newer_grid_target() -> Result<(), Box<dyn Error>>
    {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let stale = measurement_for_target(
            &targets.forward,
            UnixNanos(OBSERVED_AT.0 - 1),
            MODEL_VERSION,
            CONFIG_FINGERPRINT,
            targets.forward.source_deviation_bps().value(),
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(10, 0)),
            true,
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective(&targets, Vec::new())?,
            Some(&stale),
            None,
        )?;
        assert_eq!(decision.action, InventoryAction::IncreaseBlocked);
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::MeasurementObservedAtMismatch)
        );
        Ok(())
    }

    #[test]
    fn same_route_but_different_observed_at_is_rejected() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let different = measurement_for_target(
            &targets.forward,
            UnixNanos(OBSERVED_AT.0 + 1),
            MODEL_VERSION,
            CONFIG_FINGERPRINT,
            targets.forward.source_deviation_bps().value(),
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(10, 0)),
            true,
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective(&targets, Vec::new())?,
            Some(&different),
            None,
        )?;
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::MeasurementObservedAtMismatch)
        );
        Ok(())
    }

    #[test]
    fn same_route_and_timestamp_but_different_fingerprint_is_rejected() -> Result<(), Box<dyn Error>>
    {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let different = measurement_for_target(
            &targets.forward,
            OBSERVED_AT,
            MODEL_VERSION,
            "different-config-sha256",
            targets.forward.source_deviation_bps().value(),
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(10, 0)),
            true,
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective(&targets, Vec::new())?,
            Some(&different),
            None,
        )?;
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::MeasurementConfigFingerprintMismatch)
        );
        Ok(())
    }

    #[test]
    fn different_measurement_model_version_is_rejected() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let different = measurement_for_target(
            &targets.forward,
            OBSERVED_AT,
            "p3-measurement-v2",
            CONFIG_FINGERPRINT,
            targets.forward.source_deviation_bps().value(),
            Decimal::new(5, 1),
            Decimal::new(100, 0),
            Decimal::new(101, 0),
            Some(Decimal::new(10, 0)),
            true,
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective(&targets, Vec::new())?,
            Some(&different),
            None,
        )?;
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::MeasurementModelVersionMismatch)
        );
        Ok(())
    }

    #[test]
    fn matching_measurement_snapshot_remains_accepted() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective(&targets, Vec::new())?,
            Some(&targets.forward_measurement),
            None,
        )?;
        assert_eq!(decision.action, InventoryAction::IncreaseRisk);
        assert_eq!(decision.block_reason, None);
        Ok(())
    }

    #[test]
    fn sell_price_above_buy_price_uses_smaller_long_leg_cap() -> Result<(), Box<dyn Error>> {
        assert_two_leg_cap(
            Decimal::new(100, 0),
            Decimal::new(110, 0),
            Decimal::new(10, 0),
            Decimal::new(11, 0),
            Decimal::new(10, 0),
        )
    }

    #[test]
    fn sell_price_below_buy_price_uses_smaller_short_leg_cap() -> Result<(), Box<dyn Error>> {
        assert_two_leg_cap(
            Decimal::new(110, 0),
            Decimal::new(100, 0),
            Decimal::new(11, 0),
            Decimal::new(10, 0),
            Decimal::new(10, 0),
        )
    }

    fn assert_two_leg_cap(
        long_price: Decimal,
        short_price: Decimal,
        expected_long_notional: Decimal,
        expected_short_notional: Decimal,
        expected_cap: Decimal,
    ) -> Result<(), Box<dyn Error>> {
        let targets = targets_with_forward_measurement(
            Decimal::new(20, 0),
            Decimal::ZERO,
            Decimal::new(1, 1),
            long_price,
            short_price,
            Some(Decimal::new(10, 0)),
            true,
        )?;
        let decision = InventoryManager.decide(
            &targets.forward,
            &targets.reverse,
            &effective(&targets, Vec::new())?,
            Some(&targets.forward_measurement),
            None,
        )?;
        assert_eq!(decision.action, InventoryAction::IncreaseRisk);
        let proposed = decision.proposed_change_notional_per_leg.value();
        let basis = decision.increase_size_basis.expect("increase size basis");
        assert_eq!(basis.long_measured_notional.value(), expected_long_notional);
        assert_eq!(
            basis.short_measured_notional.value(),
            expected_short_notional
        );
        assert_eq!(basis.measured_matched_notional_cap.value(), expected_cap);
        assert_eq!(proposed, expected_cap);
        let requested_quantity = basis.requested_base_quantity.value();
        let implied_long_quantity = proposed
            .checked_div(long_price)
            .ok_or("long implied quantity division failed")?;
        let implied_short_quantity = proposed
            .checked_div(short_price)
            .ok_or("short implied quantity division failed")?;
        assert!(implied_long_quantity <= requested_quantity);
        assert!(implied_short_quantity <= requested_quantity);
        Ok(())
    }

    #[test]
    fn reserved_and_pending_exposure_prevent_duplicate_increase() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(10, 0), Decimal::ZERO)?;
        let effective = effective(
            &targets,
            vec![exposure(
                &targets.forward,
                Decimal::new(100, 0),
                Decimal::new(50, 0),
                Decimal::new(50, 0),
            )?],
        )?;
        let decision =
            InventoryManager.decide(&targets.forward, &targets.reverse, &effective, None, None)?;
        assert_eq!(decision.action, InventoryAction::NoChange);
        let actual = decision.effective_actual.expect("effective actual");
        assert_eq!(actual.actual_notional_per_leg.value(), Decimal::new(100, 0));
        assert_eq!(
            actual.reserved_notional_per_leg.value(),
            Decimal::new(50, 0)
        );
        assert_eq!(actual.pending_notional_per_leg.value(), Decimal::new(50, 0));
        assert_eq!(actual.total_notional_per_leg.value(), Decimal::new(200, 0));
        Ok(())
    }

    #[test]
    fn opposing_effective_exposure_is_visible_and_cannot_add_risk() -> Result<(), Box<dyn Error>> {
        let targets = targets(Decimal::new(20, 0), Decimal::new(20, 0))?;
        let effective = effective(
            &targets,
            vec![
                exposure(
                    &targets.forward,
                    Decimal::new(10, 0),
                    Decimal::ZERO,
                    Decimal::ZERO,
                )?,
                exposure(
                    &targets.reverse,
                    Decimal::ZERO,
                    Decimal::new(10, 0),
                    Decimal::ZERO,
                )?,
            ],
        )?;
        let decision =
            InventoryManager.decide(&targets.forward, &targets.reverse, &effective, None, None)?;
        assert_eq!(
            decision.action,
            InventoryAction::AmbiguousEffectiveInventory
        );
        assert_eq!(
            decision.block_reason,
            Some(IncreaseBlockReason::OpposingEffectiveExposure)
        );
        Ok(())
    }
}
