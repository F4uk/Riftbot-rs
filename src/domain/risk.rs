//! Risk inputs, outcomes, regime, and kill-state audit contracts.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    ids::{DecisionId, PairId, Symbol, VenueId},
    numeric::{Delta, DurationMillis, Money, Notional, TargetFraction, UnixNanos},
};

/// Market/system classification input to risk. This is not authorization or persistent state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    Normal,
    Degraded,
    ReduceOnly,
    Halted,
}

/// Authorization disposition for exactly one P5 decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskDecision {
    Approve,
    Deny,
    ReduceOnly,
    FlattenRequired,
    HaltRequired,
}

/// Persistent/global highest-authority operational state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KillState {
    Ready,
    PauseNew,
    ReduceOnly,
    Flatten,
    Halt,
}

/// P4 action preserved without importing strategy policy into the domain module.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskInputAction {
    NoChange,
    IncreaseRisk,
    ReduceRisk,
    FlattenForReversal,
    IncreaseBlocked,
    AmbiguousOpposingIncrease,
    AmbiguousEffectiveInventory,
}

impl RiskInputAction {
    #[must_use]
    pub const fn is_reduction(self) -> bool {
        matches!(self, Self::ReduceRisk | Self::FlattenForReversal)
    }
}

/// Explicit health state. There is deliberately no healthy default.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Stale,
    Unhealthy,
    Unknown,
}

/// Required health facts for one venue involved in a proposed increase.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VenueRiskHealth {
    pub venue_id: VenueId,
    pub market_feed: HealthStatus,
    pub connectivity: HealthStatus,
    pub account_private_stream: HealthStatus,
}

/// Fail-closed operational facts. Outstanding exposure must be accounted in effective inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskHealthSnapshot {
    pub venues: Vec<VenueRiskHealth>,
    pub reconciliation: HealthStatus,
    pub state_freshness: HealthStatus,
    pub latency: HealthStatus,
    pub outstanding_operations: u32,
    pub unknown_operations: u32,
    pub outstanding_exposure_included: bool,
}

/// Actual, reserved, and pending absolute notional in one unit context.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExposureComponents {
    pub actual: Notional,
    pub reserved: Notional,
    pub pending: Notional,
}

/// Absolute effective exposure at one venue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VenueExposure {
    pub venue_id: VenueId,
    pub exposure: ExposureComponents,
}

/// Signed actual, reserved, and pending global USD delta.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalDeltaComponents {
    pub actual: Delta,
    pub reserved: Delta,
    pub pending: Delta,
}

/// Risk-owned exposure view. Pair values are matched notional per leg; venue values are absolute.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskExposureSnapshot {
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub pair_per_leg: ExposureComponents,
    pub venues: Vec<VenueExposure>,
    pub global_delta: GlobalDeltaComponents,
}

/// Hard-limit and P5 policy values frozen into every authorization record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskLimitsSnapshot {
    pub max_venue_notional: Notional,
    pub max_pair_notional_per_leg: Notional,
    pub max_global_delta: Delta,
    pub max_session_loss: Notional,
    pub max_measurement_age_ms: DurationMillis,
    pub degraded_authorization_fraction: TargetFraction,
    pub session_loss_required_state: KillState,
}

/// Current and projected exposure facts used by one assessment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskExposureAudit {
    pub pair_current_notional_per_leg: Notional,
    pub pair_candidate_projected_notional_per_leg: Notional,
    pub pair_authorized_projected_notional_per_leg: Notional,
    pub long_venue: VenueId,
    pub long_current_notional: Notional,
    pub long_candidate_projected_notional: Notional,
    pub long_authorized_projected_notional: Notional,
    pub short_venue: VenueId,
    pub short_current_notional: Notional,
    pub short_candidate_projected_notional: Notional,
    pub short_authorized_projected_notional: Notional,
    pub global_delta_current: Delta,
    pub global_delta_candidate_projected: Delta,
    pub global_delta_authorized_projected: Delta,
    pub session_pnl: Option<Money>,
    pub session_loss: Option<Notional>,
}

/// Deterministic, typed explanations. Ordering in an assessment is evaluation ordering.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskReasonCode {
    Approved,
    NoRiskChange,
    P4ProposalInvalid,
    MeasurementBasisMissing,
    MeasurementTimestampFuture,
    MeasurementStale,
    MeasurementSizeCapInvalid,
    RegimeDegradedClip,
    RegimeReduceOnly,
    RegimeHalted,
    KillPauseNew,
    KillReduceOnly,
    KillFlatten,
    KillHalt,
    HealthMissing,
    VenueHealthMissing,
    MarketFeedNotHealthy,
    VenueConnectivityNotHealthy,
    AccountStreamNotHealthy,
    ReconciliationNotHealthy,
    StateNotFresh,
    LatencyNotHealthy,
    UnknownOperations,
    OutstandingExposureUnaccounted,
    ExposureMissing,
    ExposureIdentityMismatch,
    ExposureDuplicateVenue,
    PairLimitExceeded,
    VenueLimitExceeded,
    GlobalDeltaLimitExceeded,
    SessionPnlMissing,
    SessionLossLimitReached,
    ArithmeticFailure,
}

/// Fields accepted by the validated P5 authorization and serde boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskAssessmentParams {
    pub decision_id: DecisionId,
    pub evaluated_at: UnixNanos,
    pub input_action: RiskInputAction,
    pub requested_change_notional_per_leg: Money,
    pub proposed_change_notional_per_leg: Notional,
    pub authorized_change_notional_per_leg: Notional,
    pub decision: RiskDecision,
    pub regime: Regime,
    pub kill_state: KillState,
    pub reason_codes: Vec<RiskReasonCode>,
    pub explanation: String,
    pub exposure: Option<RiskExposureAudit>,
    pub measurement_age_ms: Option<DurationMillis>,
    pub measurement_safe_matched_notional_cap: Option<Notional>,
    pub limits: RiskLimitsSnapshot,
    pub config_fingerprint: String,
}

/// Validated, auditable output of one deterministic P5 evaluation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "RiskAssessmentParams")]
pub struct RiskAssessment {
    decision_id: DecisionId,
    evaluated_at: UnixNanos,
    input_action: RiskInputAction,
    requested_change_notional_per_leg: Money,
    proposed_change_notional_per_leg: Notional,
    authorized_change_notional_per_leg: Notional,
    decision: RiskDecision,
    regime: Regime,
    kill_state: KillState,
    reason_codes: Vec<RiskReasonCode>,
    explanation: String,
    exposure: Option<RiskExposureAudit>,
    measurement_age_ms: Option<DurationMillis>,
    measurement_safe_matched_notional_cap: Option<Notional>,
    limits: RiskLimitsSnapshot,
    config_fingerprint: String,
}

impl RiskAssessment {
    pub fn new(params: RiskAssessmentParams) -> Result<Self, RiskDomainError> {
        Self::try_from(params)
    }

    #[must_use]
    pub fn decision_id(&self) -> &DecisionId {
        &self.decision_id
    }

    #[must_use]
    pub const fn evaluated_at(&self) -> UnixNanos {
        self.evaluated_at
    }

    #[must_use]
    pub const fn input_action(&self) -> RiskInputAction {
        self.input_action
    }

    #[must_use]
    pub const fn requested_change_notional_per_leg(&self) -> Money {
        self.requested_change_notional_per_leg
    }

    #[must_use]
    pub const fn proposed_change_notional_per_leg(&self) -> Notional {
        self.proposed_change_notional_per_leg
    }

    #[must_use]
    pub const fn authorized_change_notional_per_leg(&self) -> Notional {
        self.authorized_change_notional_per_leg
    }

    #[must_use]
    pub const fn decision(&self) -> RiskDecision {
        self.decision
    }

    #[must_use]
    pub const fn regime(&self) -> Regime {
        self.regime
    }

    #[must_use]
    pub const fn kill_state(&self) -> KillState {
        self.kill_state
    }

    #[must_use]
    pub fn reason_codes(&self) -> &[RiskReasonCode] {
        &self.reason_codes
    }

    #[must_use]
    pub fn explanation(&self) -> &str {
        &self.explanation
    }

    #[must_use]
    pub const fn exposure(&self) -> Option<&RiskExposureAudit> {
        self.exposure.as_ref()
    }

    #[must_use]
    pub const fn measurement_age_ms(&self) -> Option<DurationMillis> {
        self.measurement_age_ms
    }

    #[must_use]
    pub const fn measurement_safe_matched_notional_cap(&self) -> Option<Notional> {
        self.measurement_safe_matched_notional_cap
    }

    #[must_use]
    pub const fn limits(&self) -> &RiskLimitsSnapshot {
        &self.limits
    }

    #[must_use]
    pub fn config_fingerprint(&self) -> &str {
        &self.config_fingerprint
    }
}

impl TryFrom<RiskAssessmentParams> for RiskAssessment {
    type Error = RiskDomainError;

    fn try_from(params: RiskAssessmentParams) -> Result<Self, Self::Error> {
        validate_assessment(&params)?;
        Ok(Self {
            decision_id: params.decision_id,
            evaluated_at: params.evaluated_at,
            input_action: params.input_action,
            requested_change_notional_per_leg: params.requested_change_notional_per_leg,
            proposed_change_notional_per_leg: params.proposed_change_notional_per_leg,
            authorized_change_notional_per_leg: params.authorized_change_notional_per_leg,
            decision: params.decision,
            regime: params.regime,
            kill_state: params.kill_state,
            reason_codes: params.reason_codes,
            explanation: params.explanation,
            exposure: params.exposure,
            measurement_age_ms: params.measurement_age_ms,
            measurement_safe_matched_notional_cap: params.measurement_safe_matched_notional_cap,
            limits: params.limits,
            config_fingerprint: params.config_fingerprint,
        })
    }
}

fn validate_assessment(params: &RiskAssessmentParams) -> Result<(), RiskDomainError> {
    let authorized = params.authorized_change_notional_per_leg.value();
    let proposed = params.proposed_change_notional_per_leg.value();
    if params.reason_codes.is_empty()
        || params.explanation.trim().is_empty()
        || !is_sha256(&params.config_fingerprint)
        || authorized > proposed
        || params.limits.max_venue_notional.value() <= Decimal::ZERO
        || params.limits.max_pair_notional_per_leg.value() <= Decimal::ZERO
        || params.limits.max_global_delta.value() <= Decimal::ZERO
        || params.limits.max_session_loss.value() <= Decimal::ZERO
        || params.limits.max_measurement_age_ms.0 == 0
        || params.limits.degraded_authorization_fraction.value() <= Decimal::ZERO
        || params.limits.degraded_authorization_fraction.value() >= Decimal::ONE
        || !matches!(
            params.limits.session_loss_required_state,
            KillState::Flatten | KillState::Halt
        )
    {
        return Err(RiskDomainError::InvalidAssessment);
    }

    match params.input_action {
        RiskInputAction::IncreaseRisk => {
            let approved = params.decision == RiskDecision::Approve;
            let has_pnl_audit = params
                .exposure
                .as_ref()
                .is_some_and(|audit| audit.session_pnl.is_some() && audit.session_loss.is_some());
            let within_safe_cap = params
                .measurement_safe_matched_notional_cap
                .is_some_and(|cap| authorized <= cap.value());
            if (approved
                && (params.requested_change_notional_per_leg.value() <= Decimal::ZERO
                    || proposed <= Decimal::ZERO
                    || authorized <= Decimal::ZERO
                    || params.measurement_age_ms.is_none()
                    || !within_safe_cap
                    || !has_pnl_audit))
                || (!approved && authorized != Decimal::ZERO)
            {
                return Err(RiskDomainError::InvalidAssessment);
            }
        }
        RiskInputAction::ReduceRisk | RiskInputAction::FlattenForReversal => {
            let requested_reduction =
                Decimal::ZERO.checked_sub(params.requested_change_notional_per_leg.value());
            if matches!(
                params.decision,
                RiskDecision::Deny | RiskDecision::HaltRequired
            ) && authorized != Decimal::ZERO
                || (authorized > Decimal::ZERO
                    && (requested_reduction.is_none_or(|value| value < authorized)
                        || proposed <= Decimal::ZERO))
            {
                return Err(RiskDomainError::InvalidAssessment);
            }
        }
        RiskInputAction::NoChange
        | RiskInputAction::IncreaseBlocked
        | RiskInputAction::AmbiguousOpposingIncrease
        | RiskInputAction::AmbiguousEffectiveInventory => {
            if authorized != Decimal::ZERO {
                return Err(RiskDomainError::InvalidAssessment);
            }
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Fields accepted by the validated kill-state transition and serde boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KillTransitionParams {
    pub from: KillState,
    pub to: KillState,
    pub reason: String,
    pub timestamp: UnixNanos,
    pub trigger: String,
}

/// Auditable persistent-state transition. Impossible graph edges cannot deserialize.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "KillTransitionParams")]
pub struct KillTransition {
    from: KillState,
    to: KillState,
    reason: String,
    timestamp: UnixNanos,
    trigger: String,
}

impl KillTransition {
    pub fn new(params: KillTransitionParams) -> Result<Self, RiskDomainError> {
        Self::try_from(params)
    }

    #[must_use]
    pub const fn from(&self) -> KillState {
        self.from
    }

    #[must_use]
    pub const fn to(&self) -> KillState {
        self.to
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    #[must_use]
    pub const fn timestamp(&self) -> UnixNanos {
        self.timestamp
    }

    #[must_use]
    pub fn trigger(&self) -> &str {
        &self.trigger
    }
}

impl TryFrom<KillTransitionParams> for KillTransition {
    type Error = RiskDomainError;

    fn try_from(params: KillTransitionParams) -> Result<Self, Self::Error> {
        if params.reason.trim().is_empty()
            || params.trigger.trim().is_empty()
            || !valid_kill_transition(params.from, params.to)
        {
            return Err(RiskDomainError::InvalidKillTransition);
        }
        Ok(Self {
            from: params.from,
            to: params.to,
            reason: params.reason,
            timestamp: params.timestamp,
            trigger: params.trigger,
        })
    }
}

/// Frozen V1 kill-state graph. Recovery from severe states must pass through a restrictive state.
#[must_use]
pub const fn valid_kill_transition(from: KillState, to: KillState) -> bool {
    matches!(
        (from, to),
        (
            KillState::Ready,
            KillState::PauseNew | KillState::ReduceOnly | KillState::Flatten | KillState::Halt
        ) | (
            KillState::PauseNew,
            KillState::Ready | KillState::ReduceOnly | KillState::Flatten | KillState::Halt
        ) | (
            KillState::ReduceOnly,
            KillState::Ready | KillState::Flatten | KillState::Halt
        ) | (KillState::Flatten, KillState::ReduceOnly | KillState::Halt)
            | (KillState::Halt, KillState::Flatten)
    )
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RiskDomainError {
    #[error("risk authorization fields form an invalid or unsafe shape")]
    InvalidAssessment,
    #[error("kill-state transition is not allowed by the frozen graph")]
    InvalidKillTransition,
}

/// Frozen risk context attached to a future execution intent contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskContext {
    pub regime: Regime,
    pub kill_state: KillState,
    pub assessment: RiskAssessment,
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;
    use serde_json::json;

    use super::{
        KillState, KillTransition, KillTransitionParams, Regime, RiskAssessment,
        RiskAssessmentParams, RiskDecision, RiskInputAction, RiskLimitsSnapshot, RiskReasonCode,
    };
    use crate::domain::{
        ids::{DecisionId, VenueId},
        numeric::{Delta, DurationMillis, Money, Notional, TargetFraction, UnixNanos},
    };

    fn limits() -> Result<RiskLimitsSnapshot, Box<dyn Error>> {
        Ok(RiskLimitsSnapshot {
            max_venue_notional: Notional::new(Decimal::from(1_000))?,
            max_pair_notional_per_leg: Notional::new(Decimal::from(1_500))?,
            max_global_delta: Delta::new(Decimal::from(25)),
            max_session_loss: Notional::new(Decimal::from(100))?,
            max_measurement_age_ms: DurationMillis(1_500),
            degraded_authorization_fraction: TargetFraction::new(Decimal::new(5, 1))?,
            session_loss_required_state: KillState::Flatten,
        })
    }

    fn approved_params() -> Result<RiskAssessmentParams, Box<dyn Error>> {
        Ok(RiskAssessmentParams {
            decision_id: DecisionId::try_from("risk-domain-test")?,
            evaluated_at: UnixNanos(2_000_000_000),
            input_action: RiskInputAction::IncreaseRisk,
            requested_change_notional_per_leg: Money::new(Decimal::from(100)),
            proposed_change_notional_per_leg: Notional::new(Decimal::from(50))?,
            authorized_change_notional_per_leg: Notional::new(Decimal::from(50))?,
            decision: RiskDecision::Approve,
            regime: Regime::Normal,
            kill_state: KillState::Ready,
            reason_codes: vec![RiskReasonCode::Approved],
            explanation: "all P5 limits satisfied".to_owned(),
            exposure: Some(super::RiskExposureAudit {
                pair_current_notional_per_leg: Notional::new(Decimal::ZERO)?,
                pair_candidate_projected_notional_per_leg: Notional::new(Decimal::from(50))?,
                pair_authorized_projected_notional_per_leg: Notional::new(Decimal::from(50))?,
                long_venue: VenueId::try_from("entropy")?,
                long_current_notional: Notional::new(Decimal::ZERO)?,
                long_candidate_projected_notional: Notional::new(Decimal::from(50))?,
                long_authorized_projected_notional: Notional::new(Decimal::from(50))?,
                short_venue: VenueId::try_from("lighter")?,
                short_current_notional: Notional::new(Decimal::ZERO)?,
                short_candidate_projected_notional: Notional::new(Decimal::from(50))?,
                short_authorized_projected_notional: Notional::new(Decimal::from(50))?,
                global_delta_current: Delta::new(Decimal::ZERO),
                global_delta_candidate_projected: Delta::new(Decimal::ZERO),
                global_delta_authorized_projected: Delta::new(Decimal::ZERO),
                session_pnl: Some(Money::new(Decimal::ZERO)),
                session_loss: Some(Notional::new(Decimal::ZERO)?),
            }),
            measurement_age_ms: Some(DurationMillis(100)),
            measurement_safe_matched_notional_cap: Some(Notional::new(Decimal::from(50))?),
            limits: limits()?,
            config_fingerprint: "a".repeat(64),
        })
    }

    #[test]
    fn serde_cannot_construct_invalid_risk_authorization() -> Result<(), Box<dyn Error>> {
        let valid = approved_params()?;
        let mut enlarged = serde_json::to_value(&valid)?;
        enlarged["authorized_change_notional_per_leg"] = json!("51");
        assert!(serde_json::from_value::<RiskAssessment>(enlarged).is_err());

        let mut missing_age = serde_json::to_value(&valid)?;
        missing_age["measurement_age_ms"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<RiskAssessment>(missing_age).is_err());

        let mut above_safe_cap = serde_json::to_value(&valid)?;
        above_safe_cap["measurement_safe_matched_notional_cap"] = json!("49");
        assert!(serde_json::from_value::<RiskAssessment>(above_safe_cap).is_err());

        let mut deny_with_size = serde_json::to_value(&valid)?;
        deny_with_size["decision"] = json!("deny");
        assert!(serde_json::from_value::<RiskAssessment>(deny_with_size).is_err());
        Ok(())
    }

    #[test]
    fn kill_transition_graph_and_serde_fail_closed() -> Result<(), Box<dyn Error>> {
        let valid = KillTransition::new(KillTransitionParams {
            from: KillState::Ready,
            to: KillState::Flatten,
            reason: "session loss".to_owned(),
            timestamp: UnixNanos(10),
            trigger: "risk_manager".to_owned(),
        })?;
        assert_eq!(valid.to(), KillState::Flatten);

        let invalid = json!({
            "from": "halt",
            "to": "ready",
            "reason": "unsafe direct recovery",
            "timestamp": 11,
            "trigger": "operator"
        });
        assert!(serde_json::from_value::<KillTransition>(invalid).is_err());
        Ok(())
    }
}
