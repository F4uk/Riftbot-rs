//! Risk outcomes, regime, and kill-state audit contracts.

use serde::{Deserialize, Serialize};

use super::numeric::UnixNanos;

/// Market/operational regime visible to strategy and risk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    Normal,
    Degraded,
    ReduceOnly,
    Halted,
}

/// Authoritative risk-manager disposition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskDecision {
    Approve,
    Deny,
    ReduceOnly,
    FlattenRequired,
    HaltRequired,
}

/// Recorded output of one risk evaluation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskAssessment {
    pub decision: RiskDecision,
    pub reason: String,
    pub evaluated_at: UnixNanos,
}

/// Global operational kill state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KillState {
    Ready,
    PauseNew,
    ReduceOnly,
    Flatten,
    Halt,
}

/// Auditable kill-state transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KillTransition {
    pub from: KillState,
    pub to: KillState,
    pub reason: String,
    pub timestamp: UnixNanos,
    pub trigger: String,
}

/// Frozen risk context attached to an execution intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskContext {
    pub regime: Regime,
    pub kill_state: KillState,
    pub assessment: RiskAssessment,
}
