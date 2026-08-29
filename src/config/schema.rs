//! Serializable configuration schema. It intentionally contains no credential fields.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{
    ids::{InstrumentId, PairId, Symbol, VenueId},
    numeric::{BaseQty, Bps, Delta, DurationMillis, Notional, TargetFraction},
};

/// Root versioned configuration schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u16,
    pub runtime: RuntimeConfig,
    pub venues: Vec<VenueConfig>,
    pub pair: Option<PairConfig>,
    pub market_data: MarketDataConfig,
    pub fair_value: FairValueConfig,
    pub funding: FundingConfig,
    pub regime: RegimeConfig,
    pub strategy: StrategyConfig,
    pub grid: GridConfig,
    pub risk: RiskLimitsConfig,
    pub execution: ExecutionConfig,
    pub recording: RecordingConfig,
}

/// Runtime behavior available during P0.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub mode: RuntimeMode,
}

/// P0 deliberately has no live-order mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    SignalOnly,
}

/// Target official adapter family; venue-specific connection details belong to P1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueKind {
    HyperliquidHip3,
    Lighter,
}

/// Public venue configuration. Authentication must come from a secret provider later.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VenueConfig {
    pub id: VenueId,
    pub kind: VenueKind,
    pub enabled: bool,
    pub taker_fee_bps: Option<Bps>,
}

/// Optional two-venue pair selected only after P1 discovery succeeds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PairConfig {
    pub id: PairId,
    pub symbol: Symbol,
    pub venues: [VenueId; 2],
    pub instruments: [InstrumentId; 2],
}

/// Market-data safety assumptions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarketDataConfig {
    pub stale_after_ms: DurationMillis,
    pub max_receive_skew_ms: DurationMillis,
    pub minimum_depth_notional: Notional,
    pub requested_base_quantity: BaseQty,
    pub execution_buffer_bps: Bps,
}

/// Deterministic logical-time fair-value sampling contract. P3 implements its behavior.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FairValueConfig {
    pub sample_interval_ms: DurationMillis,
    pub window_duration_ms: DurationMillis,
    pub minimum_samples: usize,
    pub max_sample_age_ms: DurationMillis,
}

/// Availability of one route's signed funding adjustment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingStateConfig {
    Unavailable,
    Disabled,
    Available,
}

/// Signed funding economics for one explicit oriented route.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteFundingConfig {
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub state: FundingStateConfig,
    pub adjustment_bps: Option<Bps>,
}

/// Both independent V1 route funding contracts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FundingConfig {
    pub routes: Vec<RouteFundingConfig>,
}

/// Explainable measurement-only regime thresholds. P3 owns classification behavior.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegimeConfig {
    pub degraded_dispersion_bps: Bps,
    pub reduce_only_deviation_bps: Bps,
    pub halted_deviation_bps: Bps,
}

/// Strategy-owned target ceiling. P4 implements target-inventory behavior.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyConfig {
    pub max_target_notional: Notional,
}

/// Monotonic grid definition. P4 implements the sole target-inventory model.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GridConfig {
    pub levels: Vec<GridLevelConfig>,
}

/// Positive deviation magnitude mapped to a non-negative inventory fraction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GridLevelConfig {
    pub deviation_bps: Bps,
    pub target_fraction: TargetFraction,
}

/// Hard monetary limits. P5 owns enforcement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskLimitsConfig {
    pub max_venue_notional: Notional,
    pub max_pair_notional: Notional,
    pub max_global_delta: Delta,
    pub max_session_loss: Notional,
    pub max_measurement_age_ms: DurationMillis,
    pub degraded_authorization_fraction: TargetFraction,
    pub session_loss_action: SessionLossAction,
}

/// Persistent-state escalation required when the session-loss limit is reached.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLossAction {
    Flatten,
    Halt,
}

/// V1 basket limits. P6 owns execution behavior.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    pub max_residual_delta: Delta,
    pub max_slippage_bps: Bps,
    pub intent_expiry_ms: DurationMillis,
}

/// Recorder capacity and non-secret output path. P2 owns persistence behavior.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingConfig {
    pub directory: PathBuf,
    pub channel_capacity: usize,
}
