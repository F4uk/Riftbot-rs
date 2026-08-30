//! Deterministic P5 hard-risk authorization and persistent kill-state transitions.

use std::collections::HashSet;

use rust_decimal::Decimal;
use thiserror::Error;

use crate::{
    config::{RiskConfigFingerprint, RiskLimitsConfig, SessionLossAction},
    domain::{
        ids::VenueId,
        inventory::TargetDirection,
        numeric::{Delta, DurationMillis, Money, Notional, NumericError, UnixNanos},
        risk::{
            HealthStatus, KillState, KillTransition, KillTransitionParams, Regime, RiskAssessment,
            RiskAssessmentParams, RiskDecision, RiskDomainError, RiskExposureAudit,
            RiskExposureSnapshot, RiskHealthSnapshot, RiskInputAction, RiskLimitsSnapshot,
            RiskReasonCode,
        },
    },
    strategy::inventory_manager::{InventoryAction, InventoryDecision},
};

use super::limits::{
    RiskArithmeticError, absolute, add_notional, exposure_total, global_delta_total, session_loss,
    subtract_notional,
};

const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// All facts needed for one authorization. Logical time is always supplied by the caller.
#[derive(Clone, Copy, Debug)]
pub struct RiskEvaluationInput<'a> {
    pub inventory: &'a InventoryDecision,
    pub regime: Regime,
    pub kill_state: KillState,
    pub evaluated_at: UnixNanos,
    pub exposure: Option<&'a RiskExposureSnapshot>,
    pub health: Option<&'a RiskHealthSnapshot>,
    pub session_pnl: Option<Money>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RiskManagerError {
    #[error("P5 risk configuration is invalid")]
    InvalidConfiguration,
    #[error(transparent)]
    Domain(#[from] RiskDomainError),
    #[error(transparent)]
    Numeric(#[from] NumericError),
}

/// Stateless hard-risk policy. Persistent kill state is supplied explicitly on every decision.
#[derive(Clone, Debug)]
pub struct RiskManager {
    limits: RiskLimitsConfig,
    limits_snapshot: RiskLimitsSnapshot,
    config_fingerprint: String,
}

impl RiskManager {
    pub fn new(
        limits: &RiskLimitsConfig,
        config_fingerprint: &RiskConfigFingerprint,
    ) -> Result<Self, RiskManagerError> {
        if limits.max_venue_notional.value() <= Decimal::ZERO
            || limits.max_pair_notional.value() <= Decimal::ZERO
            || limits.max_global_delta.value() <= Decimal::ZERO
            || limits.max_session_loss.value() <= Decimal::ZERO
            || limits.max_measurement_age_ms.0 == 0
            || limits.degraded_authorization_fraction.value() <= Decimal::ZERO
            || limits.degraded_authorization_fraction.value() >= Decimal::ONE
        {
            return Err(RiskManagerError::InvalidConfiguration);
        }
        let session_loss_required_state = match limits.session_loss_action {
            SessionLossAction::Flatten => KillState::Flatten,
            SessionLossAction::Halt => KillState::Halt,
        };
        Ok(Self {
            limits: limits.clone(),
            limits_snapshot: RiskLimitsSnapshot {
                max_venue_notional: limits.max_venue_notional,
                max_pair_notional_per_leg: limits.max_pair_notional,
                max_global_delta: limits.max_global_delta,
                max_session_loss: limits.max_session_loss,
                max_measurement_age_ms: limits.max_measurement_age_ms,
                degraded_authorization_fraction: limits.degraded_authorization_fraction,
                session_loss_required_state,
            },
            config_fingerprint: config_fingerprint.as_str().to_owned(),
        })
    }

    pub fn assess(
        &self,
        input: RiskEvaluationInput<'_>,
    ) -> Result<RiskAssessment, RiskManagerError> {
        let action = RiskInputAction::from(input.inventory.action);
        let proposed = input.inventory.proposed_change_notional_per_leg;
        let zero = Notional::new(Decimal::ZERO)?;
        let mut reasons = Vec::new();
        let p4_valid = validate_p4_proposal(input.inventory, action, &mut reasons);
        let mut effective_decision = if p4_valid {
            RiskDecision::Approve
        } else {
            RiskDecision::Deny
        };
        let mut candidate_change =
            if action == RiskInputAction::IncreaseRisk || action.is_reduction() {
                proposed
            } else {
                zero
            };
        let route = proposal_route(input.inventory, action);
        let mut measurement_age = None;

        if action == RiskInputAction::IncreaseRisk {
            check_measurement_recency(
                input.inventory,
                input.evaluated_at,
                self.limits.max_measurement_age_ms,
                &mut measurement_age,
                &mut reasons,
                &mut effective_decision,
            );
            apply_regime_to_increase(
                input.regime,
                self.limits.degraded_authorization_fraction.value(),
                &mut candidate_change,
                &mut reasons,
                &mut effective_decision,
            );
            apply_kill_state_to_increase(input.kill_state, &mut reasons, &mut effective_decision);
            check_health(input.health, route, &mut reasons, &mut effective_decision);
        } else if action.is_reduction() {
            apply_regime_to_reduction(input.regime, &mut reasons, &mut effective_decision);
            apply_kill_state_to_reduction(input.kill_state, &mut reasons, &mut effective_decision);
        } else {
            apply_non_action_authority(
                input.regime,
                input.kill_state,
                &mut reasons,
                &mut effective_decision,
            );
        }

        let (session_loss_value, session_arithmetic_ok) = evaluate_session_loss(
            input.session_pnl,
            action,
            &mut reasons,
            &mut effective_decision,
        );
        if let Some(loss) = session_loss_value
            && loss.value() >= self.limits.max_session_loss.value()
        {
            push_once(&mut reasons, RiskReasonCode::SessionLossLimitReached);
            apply_decision(
                &mut effective_decision,
                match self.limits.session_loss_action {
                    SessionLossAction::Flatten => RiskDecision::FlattenRequired,
                    SessionLossAction::Halt => RiskDecision::HaltRequired,
                },
            );
        }

        let mut exposure_base = evaluate_exposure(
            input.inventory,
            route,
            input.exposure,
            &mut reasons,
            &mut effective_decision,
        );
        let current_exposure_valid = if let Some(base) = &exposure_base {
            check_current_limits(base, &self.limits, &mut reasons, &mut effective_decision)
        } else {
            false
        };
        if action == RiskInputAction::IncreaseRisk
            && let Some(base) = &exposure_base
        {
            check_increase_limits(
                base,
                candidate_change,
                &self.limits,
                &mut reasons,
                &mut effective_decision,
            );
        }

        let can_authorize_reduction = action.is_reduction()
            && p4_valid
            && exposure_base.is_some()
            && current_exposure_valid
            && session_arithmetic_ok
            && !matches!(
                effective_decision,
                RiskDecision::Deny | RiskDecision::HaltRequired
            );
        let can_authorize_increase = action == RiskInputAction::IncreaseRisk
            && p4_valid
            && exposure_base.is_some()
            && current_exposure_valid
            && session_arithmetic_ok
            && effective_decision == RiskDecision::Approve
            && candidate_change.value() > Decimal::ZERO;
        let authorized = if can_authorize_increase || can_authorize_reduction {
            candidate_change
        } else {
            zero
        };

        let exposure_audit = exposure_base.take().and_then(|base| {
            base.audit(
                action,
                candidate_change,
                authorized,
                input.session_pnl,
                session_loss_value,
            )
            .ok()
        });
        if (action == RiskInputAction::IncreaseRisk || action.is_reduction())
            && exposure_audit.is_none()
        {
            push_once(&mut reasons, RiskReasonCode::ArithmeticFailure);
            apply_decision(&mut effective_decision, RiskDecision::Deny);
        }

        let authorized = if exposure_audit.is_none()
            || (action == RiskInputAction::IncreaseRisk
                && effective_decision != RiskDecision::Approve)
            || (action.is_reduction()
                && matches!(
                    effective_decision,
                    RiskDecision::Deny | RiskDecision::HaltRequired
                )) {
            zero
        } else {
            authorized
        };

        if reasons.is_empty() {
            push_once(
                &mut reasons,
                if action == RiskInputAction::NoChange {
                    RiskReasonCode::NoRiskChange
                } else {
                    RiskReasonCode::Approved
                },
            );
        } else if effective_decision == RiskDecision::Approve
            && (authorized.value() > Decimal::ZERO || action == RiskInputAction::NoChange)
        {
            push_once(&mut reasons, RiskReasonCode::Approved);
        }

        RiskAssessment::new(RiskAssessmentParams {
            decision_id: input.inventory.decision_id.clone(),
            evaluated_at: input.evaluated_at,
            input_action: action,
            requested_change_notional_per_leg: input.inventory.required_change_notional_per_leg,
            proposed_change_notional_per_leg: proposed,
            authorized_change_notional_per_leg: authorized,
            decision: effective_decision,
            regime: input.regime,
            kill_state: input.kill_state,
            explanation: explanation(&reasons),
            reason_codes: reasons,
            exposure: exposure_audit,
            measurement_age_ms: measurement_age,
            measurement_safe_matched_notional_cap: if action == RiskInputAction::IncreaseRisk {
                input
                    .inventory
                    .increase_size_basis
                    .as_ref()
                    .map(|basis| basis.measured_matched_notional_cap)
            } else {
                None
            },
            limits: self.limits_snapshot.clone(),
            config_fingerprint: self.config_fingerprint.clone(),
        })
        .map_err(RiskManagerError::from)
    }
}

impl From<InventoryAction> for RiskInputAction {
    fn from(action: InventoryAction) -> Self {
        match action {
            InventoryAction::NoChange => Self::NoChange,
            InventoryAction::IncreaseRisk => Self::IncreaseRisk,
            InventoryAction::ReduceRisk => Self::ReduceRisk,
            InventoryAction::FlattenForReversal => Self::FlattenForReversal,
            InventoryAction::IncreaseBlocked => Self::IncreaseBlocked,
            InventoryAction::AmbiguousOpposingIncrease => Self::AmbiguousOpposingIncrease,
            InventoryAction::AmbiguousEffectiveInventory => Self::AmbiguousEffectiveInventory,
        }
    }
}

fn validate_p4_proposal(
    inventory: &InventoryDecision,
    action: RiskInputAction,
    reasons: &mut Vec<RiskReasonCode>,
) -> bool {
    let proposed = inventory.proposed_change_notional_per_leg.value();
    let required = inventory.required_change_notional_per_leg.value();
    let identity_valid = inventory.selected_target.as_ref().is_none_or(|target| {
        target.decision_id() == &inventory.decision_id
            && target.pair_id() == &inventory.pair_id
            && target.symbol() == &inventory.symbol
    });
    let valid = match action {
        RiskInputAction::IncreaseRisk => {
            let target = inventory.selected_target.as_ref();
            let actual = inventory.effective_actual.as_ref();
            let basis = inventory.increase_size_basis.as_ref();
            let route_matches = target.zip(actual).is_some_and(|(target, actual)| {
                matches!(
                    (target.direction(), &actual.direction),
                    (
                        TargetDirection::LongShort {
                            long_venue: target_long,
                            short_venue: target_short,
                        },
                        TargetDirection::LongShort {
                            long_venue: actual_long,
                            short_venue: actual_short,
                        }
                    ) if target_long == actual_long && target_short == actual_short
                )
            });
            let size_cap_valid = basis.is_some_and(|basis| {
                let safe_cap = basis.measured_matched_notional_cap.value();
                safe_cap > Decimal::ZERO
                    && safe_cap
                        == basis
                            .long_measured_notional
                            .value()
                            .min(basis.short_measured_notional.value())
                    && proposed <= safe_cap
                    && !basis.measurement_config_fingerprint.is_empty()
            });
            if basis.is_some() && !size_cap_valid {
                push_once(reasons, RiskReasonCode::MeasurementSizeCapInvalid);
            }
            proposed > Decimal::ZERO
                && required >= proposed
                && inventory.block_reason.is_none()
                && identity_valid
                && target.is_some_and(|target| {
                    matches!(target.direction(), TargetDirection::LongShort { .. })
                })
                && actual.is_some()
                && route_matches
                && size_cap_valid
        }
        RiskInputAction::ReduceRisk | RiskInputAction::FlattenForReversal => {
            let reduction_magnitude = Decimal::ZERO.checked_sub(required);
            proposed > Decimal::ZERO
                && required < Decimal::ZERO
                && reduction_magnitude == Some(proposed)
                && inventory.selected_target.is_some()
                && inventory
                    .effective_actual
                    .as_ref()
                    .is_some_and(|actual| proposed <= actual.total_notional_per_leg.value())
                && identity_valid
        }
        RiskInputAction::NoChange => proposed == Decimal::ZERO && required == Decimal::ZERO,
        RiskInputAction::IncreaseBlocked
        | RiskInputAction::AmbiguousOpposingIncrease
        | RiskInputAction::AmbiguousEffectiveInventory => proposed == Decimal::ZERO,
    };
    if !valid {
        push_once(reasons, RiskReasonCode::P4ProposalInvalid);
    }
    valid
}

fn proposal_route(
    inventory: &InventoryDecision,
    action: RiskInputAction,
) -> Option<(&VenueId, &VenueId)> {
    if action == RiskInputAction::IncreaseRisk {
        return inventory.selected_target.as_ref().and_then(|target| {
            if let TargetDirection::LongShort {
                long_venue,
                short_venue,
            } = target.direction()
            {
                Some((long_venue, short_venue))
            } else {
                None
            }
        });
    }
    inventory.effective_actual.as_ref().and_then(|actual| {
        if let TargetDirection::LongShort {
            long_venue,
            short_venue,
        } = &actual.direction
        {
            Some((long_venue, short_venue))
        } else {
            None
        }
    })
}

fn check_measurement_recency(
    inventory: &InventoryDecision,
    evaluated_at: UnixNanos,
    max_age: DurationMillis,
    age: &mut Option<DurationMillis>,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    let Some(basis) = &inventory.increase_size_basis else {
        push_once(reasons, RiskReasonCode::MeasurementBasisMissing);
        apply_decision(decision, RiskDecision::Deny);
        return;
    };
    if basis.observed_at > evaluated_at {
        push_once(reasons, RiskReasonCode::MeasurementTimestampFuture);
        apply_decision(decision, RiskDecision::Deny);
        return;
    }
    let Some(allowed_nanos) = max_age.0.checked_mul(NANOS_PER_MILLISECOND) else {
        push_once(reasons, RiskReasonCode::ArithmeticFailure);
        apply_decision(decision, RiskDecision::Deny);
        return;
    };
    let elapsed_nanos = evaluated_at.0 - basis.observed_at.0;
    *age = Some(DurationMillis(elapsed_nanos / NANOS_PER_MILLISECOND));
    if elapsed_nanos > allowed_nanos {
        push_once(reasons, RiskReasonCode::MeasurementStale);
        apply_decision(decision, RiskDecision::Deny);
    }
}

fn apply_regime_to_increase(
    regime: Regime,
    degraded_fraction: Decimal,
    candidate: &mut Notional,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    match regime {
        Regime::Normal => {}
        Regime::Degraded => {
            let Some(clipped) = candidate.value().checked_mul(degraded_fraction) else {
                push_once(reasons, RiskReasonCode::ArithmeticFailure);
                apply_decision(decision, RiskDecision::Deny);
                return;
            };
            match Notional::new(clipped) {
                Ok(value) if value.value() > Decimal::ZERO => {
                    *candidate = value;
                    push_once(reasons, RiskReasonCode::RegimeDegradedClip);
                }
                _ => {
                    push_once(reasons, RiskReasonCode::ArithmeticFailure);
                    apply_decision(decision, RiskDecision::Deny);
                }
            }
        }
        Regime::ReduceOnly => {
            push_once(reasons, RiskReasonCode::RegimeReduceOnly);
            apply_decision(decision, RiskDecision::ReduceOnly);
        }
        Regime::Halted => {
            push_once(reasons, RiskReasonCode::RegimeHalted);
            apply_decision(decision, RiskDecision::FlattenRequired);
        }
    }
}

fn apply_regime_to_reduction(
    regime: Regime,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    match regime {
        Regime::Normal | Regime::Degraded => {}
        Regime::ReduceOnly => {
            push_once(reasons, RiskReasonCode::RegimeReduceOnly);
            apply_decision(decision, RiskDecision::ReduceOnly);
        }
        Regime::Halted => {
            push_once(reasons, RiskReasonCode::RegimeHalted);
            apply_decision(decision, RiskDecision::FlattenRequired);
        }
    }
}

fn apply_kill_state_to_increase(
    state: KillState,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    match state {
        KillState::Ready => {}
        KillState::PauseNew => {
            push_once(reasons, RiskReasonCode::KillPauseNew);
            apply_decision(decision, RiskDecision::Deny);
        }
        KillState::ReduceOnly => {
            push_once(reasons, RiskReasonCode::KillReduceOnly);
            apply_decision(decision, RiskDecision::ReduceOnly);
        }
        KillState::Flatten => {
            push_once(reasons, RiskReasonCode::KillFlatten);
            apply_decision(decision, RiskDecision::FlattenRequired);
        }
        KillState::Halt => {
            push_once(reasons, RiskReasonCode::KillHalt);
            apply_decision(decision, RiskDecision::HaltRequired);
        }
    }
}

fn apply_kill_state_to_reduction(
    state: KillState,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    match state {
        KillState::Ready | KillState::PauseNew => {}
        KillState::ReduceOnly => {
            push_once(reasons, RiskReasonCode::KillReduceOnly);
            apply_decision(decision, RiskDecision::ReduceOnly);
        }
        KillState::Flatten => {
            push_once(reasons, RiskReasonCode::KillFlatten);
            apply_decision(decision, RiskDecision::FlattenRequired);
        }
        KillState::Halt => {
            push_once(reasons, RiskReasonCode::KillHalt);
            apply_decision(decision, RiskDecision::HaltRequired);
        }
    }
}

fn apply_non_action_authority(
    regime: Regime,
    kill_state: KillState,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    if regime == Regime::Halted {
        push_once(reasons, RiskReasonCode::RegimeHalted);
        apply_decision(decision, RiskDecision::FlattenRequired);
    } else if regime == Regime::ReduceOnly {
        push_once(reasons, RiskReasonCode::RegimeReduceOnly);
        apply_decision(decision, RiskDecision::ReduceOnly);
    }
    apply_kill_state_to_increase(kill_state, reasons, decision);
}

fn check_health(
    health: Option<&RiskHealthSnapshot>,
    route: Option<(&VenueId, &VenueId)>,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    let Some(health) = health else {
        push_once(reasons, RiskReasonCode::HealthMissing);
        apply_decision(decision, RiskDecision::Deny);
        return;
    };
    let Some((long_venue, short_venue)) = route else {
        push_once(reasons, RiskReasonCode::P4ProposalInvalid);
        apply_decision(decision, RiskDecision::Deny);
        return;
    };
    let long = unique_health(&health.venues, long_venue);
    let short = unique_health(&health.venues, short_venue);
    if long.is_none() || short.is_none() {
        push_once(reasons, RiskReasonCode::VenueHealthMissing);
        apply_decision(decision, RiskDecision::Deny);
    }
    for venue in [long, short].into_iter().flatten() {
        if venue.market_feed != HealthStatus::Healthy {
            push_once(reasons, RiskReasonCode::MarketFeedNotHealthy);
        }
        if venue.connectivity != HealthStatus::Healthy {
            push_once(reasons, RiskReasonCode::VenueConnectivityNotHealthy);
        }
        if venue.account_private_stream != HealthStatus::Healthy {
            push_once(reasons, RiskReasonCode::AccountStreamNotHealthy);
        }
    }
    if health.reconciliation != HealthStatus::Healthy {
        push_once(reasons, RiskReasonCode::ReconciliationNotHealthy);
    }
    if health.state_freshness != HealthStatus::Healthy {
        push_once(reasons, RiskReasonCode::StateNotFresh);
    }
    if health.latency != HealthStatus::Healthy {
        push_once(reasons, RiskReasonCode::LatencyNotHealthy);
    }
    if health.unknown_operations > 0 {
        push_once(reasons, RiskReasonCode::UnknownOperations);
    }
    if health.outstanding_operations > 0 && !health.outstanding_exposure_included {
        push_once(reasons, RiskReasonCode::OutstandingExposureUnaccounted);
    }
    if reasons.iter().any(|reason| {
        matches!(
            reason,
            RiskReasonCode::VenueHealthMissing
                | RiskReasonCode::MarketFeedNotHealthy
                | RiskReasonCode::VenueConnectivityNotHealthy
                | RiskReasonCode::AccountStreamNotHealthy
                | RiskReasonCode::ReconciliationNotHealthy
                | RiskReasonCode::StateNotFresh
                | RiskReasonCode::LatencyNotHealthy
                | RiskReasonCode::UnknownOperations
                | RiskReasonCode::OutstandingExposureUnaccounted
        )
    }) {
        apply_decision(decision, RiskDecision::Deny);
    }
}

fn unique_health<'a>(
    venues: &'a [crate::domain::risk::VenueRiskHealth],
    venue_id: &VenueId,
) -> Option<&'a crate::domain::risk::VenueRiskHealth> {
    let mut matches = venues.iter().filter(|venue| &venue.venue_id == venue_id);
    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first)
    }
}

fn evaluate_session_loss(
    pnl: Option<Money>,
    action: RiskInputAction,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) -> (Option<Notional>, bool) {
    let Some(pnl) = pnl else {
        if action == RiskInputAction::IncreaseRisk {
            push_once(reasons, RiskReasonCode::SessionPnlMissing);
            apply_decision(decision, RiskDecision::Deny);
        }
        return (None, true);
    };
    match session_loss(pnl) {
        Ok(loss) => (Some(loss), true),
        Err(RiskArithmeticError::Overflow) => {
            push_once(reasons, RiskReasonCode::ArithmeticFailure);
            apply_decision(decision, RiskDecision::Deny);
            (None, false)
        }
    }
}

#[derive(Clone, Debug)]
struct ExposureBase {
    pair_current: Notional,
    long_venue: VenueId,
    long_current: Notional,
    short_venue: VenueId,
    short_current: Notional,
    global_delta_current: Delta,
}

impl ExposureBase {
    fn audit(
        &self,
        action: RiskInputAction,
        candidate: Notional,
        authorized: Notional,
        pnl: Option<Money>,
        loss: Option<Notional>,
    ) -> Result<RiskExposureAudit, RiskArithmeticError> {
        let candidate_values = self.project(action, candidate)?;
        let authorized_values = self.project(action, authorized)?;
        Ok(RiskExposureAudit {
            pair_current_notional_per_leg: self.pair_current,
            pair_candidate_projected_notional_per_leg: candidate_values.0,
            pair_authorized_projected_notional_per_leg: authorized_values.0,
            long_venue: self.long_venue.clone(),
            long_current_notional: self.long_current,
            long_candidate_projected_notional: candidate_values.1,
            long_authorized_projected_notional: authorized_values.1,
            short_venue: self.short_venue.clone(),
            short_current_notional: self.short_current,
            short_candidate_projected_notional: candidate_values.2,
            short_authorized_projected_notional: authorized_values.2,
            global_delta_current: self.global_delta_current,
            global_delta_candidate_projected: self.global_delta_current,
            global_delta_authorized_projected: self.global_delta_current,
            session_pnl: pnl,
            session_loss: loss,
        })
    }

    fn project(
        &self,
        action: RiskInputAction,
        change: Notional,
    ) -> Result<(Notional, Notional, Notional), RiskArithmeticError> {
        if action == RiskInputAction::IncreaseRisk {
            Ok((
                add_notional(self.pair_current, change)?,
                add_notional(self.long_current, change)?,
                add_notional(self.short_current, change)?,
            ))
        } else if action.is_reduction() {
            Ok((
                subtract_notional(self.pair_current, change)?,
                subtract_notional(self.long_current, change)?,
                subtract_notional(self.short_current, change)?,
            ))
        } else {
            Ok((self.pair_current, self.long_current, self.short_current))
        }
    }
}

fn evaluate_exposure(
    inventory: &InventoryDecision,
    route: Option<(&VenueId, &VenueId)>,
    exposure: Option<&RiskExposureSnapshot>,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) -> Option<ExposureBase> {
    let Some(exposure) = exposure else {
        push_once(reasons, RiskReasonCode::ExposureMissing);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    };
    let Some((long_venue, short_venue)) = route else {
        push_once(reasons, RiskReasonCode::ExposureIdentityMismatch);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    };
    if exposure.pair_id != inventory.pair_id || exposure.symbol != inventory.symbol {
        push_once(reasons, RiskReasonCode::ExposureIdentityMismatch);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    }
    let unique_count = exposure
        .venues
        .iter()
        .map(|venue| &venue.venue_id)
        .collect::<HashSet<_>>()
        .len();
    if unique_count != exposure.venues.len() {
        push_once(reasons, RiskReasonCode::ExposureDuplicateVenue);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    }
    let long = exposure
        .venues
        .iter()
        .find(|venue| &venue.venue_id == long_venue);
    let short = exposure
        .venues
        .iter()
        .find(|venue| &venue.venue_id == short_venue);
    let (Some(long), Some(short)) = (long, short) else {
        push_once(reasons, RiskReasonCode::ExposureMissing);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    };
    let totals = (
        exposure_total(exposure.pair_per_leg),
        exposure_total(long.exposure),
        exposure_total(short.exposure),
        global_delta_total(exposure.global_delta),
    );
    let (Ok(pair_current), Ok(long_current), Ok(short_current), Ok(global_delta_current)) = totals
    else {
        push_once(reasons, RiskReasonCode::ArithmeticFailure);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    };
    let actual_matches = inventory.effective_actual.as_ref().is_some_and(|actual| {
        actual.total_notional_per_leg == pair_current
            && long_current.value() >= pair_current.value()
            && short_current.value() >= pair_current.value()
    });
    if !actual_matches {
        push_once(reasons, RiskReasonCode::ExposureIdentityMismatch);
        apply_decision(decision, RiskDecision::Deny);
        return None;
    }
    Some(ExposureBase {
        pair_current,
        long_venue: long_venue.clone(),
        long_current,
        short_venue: short_venue.clone(),
        short_current,
        global_delta_current,
    })
}

fn check_current_limits(
    base: &ExposureBase,
    limits: &RiskLimitsConfig,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) -> bool {
    if base.pair_current.value() > limits.max_pair_notional.value() {
        push_once(reasons, RiskReasonCode::PairLimitExceeded);
        apply_decision(decision, RiskDecision::ReduceOnly);
    }
    if base.long_current.value() > limits.max_venue_notional.value()
        || base.short_current.value() > limits.max_venue_notional.value()
    {
        push_once(reasons, RiskReasonCode::VenueLimitExceeded);
        apply_decision(decision, RiskDecision::ReduceOnly);
    }
    match absolute(base.global_delta_current.value()) {
        Ok(delta) if delta > limits.max_global_delta.value() => {
            push_once(reasons, RiskReasonCode::GlobalDeltaLimitExceeded);
            apply_decision(decision, RiskDecision::FlattenRequired);
        }
        Err(RiskArithmeticError::Overflow) => {
            push_once(reasons, RiskReasonCode::ArithmeticFailure);
            apply_decision(decision, RiskDecision::Deny);
            return false;
        }
        _ => {}
    }
    true
}

fn check_increase_limits(
    base: &ExposureBase,
    candidate: Notional,
    limits: &RiskLimitsConfig,
    reasons: &mut Vec<RiskReasonCode>,
    decision: &mut RiskDecision,
) {
    let projected = base.project(RiskInputAction::IncreaseRisk, candidate);
    let Ok((pair, long, short)) = projected else {
        push_once(reasons, RiskReasonCode::ArithmeticFailure);
        apply_decision(decision, RiskDecision::Deny);
        return;
    };
    if pair.value() > limits.max_pair_notional.value() {
        push_once(reasons, RiskReasonCode::PairLimitExceeded);
    }
    if long.value() > limits.max_venue_notional.value()
        || short.value() > limits.max_venue_notional.value()
    {
        push_once(reasons, RiskReasonCode::VenueLimitExceeded);
    }
    if reasons.iter().any(|reason| {
        matches!(
            reason,
            RiskReasonCode::PairLimitExceeded
                | RiskReasonCode::VenueLimitExceeded
                | RiskReasonCode::GlobalDeltaLimitExceeded
                | RiskReasonCode::ArithmeticFailure
        )
    }) {
        apply_decision(decision, RiskDecision::Deny);
    }
}

fn apply_decision(current: &mut RiskDecision, candidate: RiskDecision) {
    if decision_severity(candidate) > decision_severity(*current) {
        *current = candidate;
    }
}

const fn decision_severity(decision: RiskDecision) -> u8 {
    match decision {
        RiskDecision::Approve => 0,
        RiskDecision::Deny => 1,
        RiskDecision::ReduceOnly => 2,
        RiskDecision::FlattenRequired => 3,
        RiskDecision::HaltRequired => 4,
    }
}

fn push_once(reasons: &mut Vec<RiskReasonCode>, reason: RiskReasonCode) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn explanation(reasons: &[RiskReasonCode]) -> String {
    reasons
        .iter()
        .map(|reason| match reason {
            RiskReasonCode::Approved => "hard-risk authorization approved",
            RiskReasonCode::NoRiskChange => "no risk-changing action requested",
            RiskReasonCode::P4ProposalInvalid => "P4 proposal shape is invalid",
            RiskReasonCode::MeasurementBasisMissing => "increase-size measurement basis is missing",
            RiskReasonCode::MeasurementTimestampFuture => "measurement timestamp is in the future",
            RiskReasonCode::MeasurementStale => "measurement exceeds configured logical-time age",
            RiskReasonCode::MeasurementSizeCapInvalid => "P3 safe measured size cap is invalid",
            RiskReasonCode::RegimeDegradedClip => "degraded regime clipped the proposed increase",
            RiskReasonCode::RegimeReduceOnly => "regime permits reduction only",
            RiskReasonCode::RegimeHalted => "halted regime requires flattening policy",
            RiskReasonCode::KillPauseNew => "kill state pauses new risk",
            RiskReasonCode::KillReduceOnly => "kill state permits reduction only",
            RiskReasonCode::KillFlatten => "kill state requires flattening",
            RiskReasonCode::KillHalt => "kill state halts routine P4 actions",
            RiskReasonCode::HealthMissing => "required health snapshot is missing",
            RiskReasonCode::VenueHealthMissing => "required venue health is missing or duplicated",
            RiskReasonCode::MarketFeedNotHealthy => "market feed is not healthy",
            RiskReasonCode::VenueConnectivityNotHealthy => "venue connectivity is not healthy",
            RiskReasonCode::AccountStreamNotHealthy => "account private stream is not healthy",
            RiskReasonCode::ReconciliationNotHealthy => "reconciliation is not healthy",
            RiskReasonCode::StateNotFresh => "risk/account state is not fresh",
            RiskReasonCode::LatencyNotHealthy => "operational latency is not healthy",
            RiskReasonCode::UnknownOperations => "unknown operations are outstanding",
            RiskReasonCode::OutstandingExposureUnaccounted => {
                "outstanding operation exposure is not included"
            }
            RiskReasonCode::ExposureMissing => "required exposure facts are missing",
            RiskReasonCode::ExposureIdentityMismatch => {
                "risk exposure does not match P4 effective inventory"
            }
            RiskReasonCode::ExposureDuplicateVenue => "risk exposure duplicates a venue",
            RiskReasonCode::PairLimitExceeded => "projected matched per-leg pair limit is exceeded",
            RiskReasonCode::VenueLimitExceeded => "projected venue notional limit is exceeded",
            RiskReasonCode::GlobalDeltaLimitExceeded => "effective global delta limit is exceeded",
            RiskReasonCode::SessionPnlMissing => "signed session PnL is missing",
            RiskReasonCode::SessionLossLimitReached => "session loss limit is reached or exceeded",
            RiskReasonCode::ArithmeticFailure => "checked fixed-decimal arithmetic failed",
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Persistent kill-state holder with caller-timestamped, audited transition validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KillStateMachine {
    state: KillState,
    last_transition_at: UnixNanos,
}

impl KillStateMachine {
    #[must_use]
    pub const fn new(initial: KillState, initialized_at: UnixNanos) -> Self {
        Self {
            state: initial,
            last_transition_at: initialized_at,
        }
    }

    #[must_use]
    pub const fn state(&self) -> KillState {
        self.state
    }

    #[must_use]
    pub const fn last_transition_at(&self) -> UnixNanos {
        self.last_transition_at
    }

    pub fn transition(
        &mut self,
        to: KillState,
        reason: impl Into<String>,
        timestamp: UnixNanos,
        trigger: impl Into<String>,
    ) -> Result<KillTransition, KillStateTransitionError> {
        if timestamp < self.last_transition_at {
            return Err(KillStateTransitionError::TimestampRegression);
        }
        let transition = KillTransition::new(KillTransitionParams {
            from: self.state,
            to,
            reason: reason.into(),
            timestamp,
            trigger: trigger.into(),
        })?;
        self.state = to;
        self.last_transition_at = timestamp;
        Ok(transition)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum KillStateTransitionError {
    #[error("kill-state transition timestamp regressed")]
    TimestampRegression,
    #[error(transparent)]
    Domain(#[from] RiskDomainError),
}
