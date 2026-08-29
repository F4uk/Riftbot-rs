//! P3 executable-route and spread measurement contracts; these types never choose inventory.

use serde::{Deserialize, Serialize};

use super::{
    ids::{InstrumentId, ModelVersion, PairId, Symbol, VenueId},
    numeric::{BaseQty, Bps, DurationMillis, Notional, Price, UnixNanos},
};

/// Why a final P3 measurement may not authorize increasing risk.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementValidity {
    Valid,
    WarmingUp,
    StaleBook,
    ReceiveTimeSkew,
    EmptyBookSide,
    InsufficientDepth,
    CrossedOrCorruptBook,
    VenueUnhealthy,
    FeeUnavailable,
    FundingUnavailable,
    InvalidFairValue,
}

impl MeasurementValidity {
    #[must_use]
    pub const fn permits_increase(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Explicit availability semantics for a signed funding adjustment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingState {
    Unavailable,
    Disabled,
    Available,
}

/// One separately auditable cost which is neither fees nor the execution uncertainty buffer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExplicitRiskCost {
    pub name: String,
    pub bps: Bps,
}

/// Valid real-L2 VWAP facts for one explicit oriented route before fair-value enrichment.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableRouteMeasurement {
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub long_instrument_id: InstrumentId,
    pub short_instrument_id: InstrumentId,
    pub requested_base_quantity: BaseQty,
    pub maximum_executable_base_quantity: BaseQty,
    pub executable_long_price: Price,
    pub executable_short_price: Price,
    pub executable_notional: Notional,
    pub raw_executable_premium_bps: Bps,
    pub fee_bps: Option<Bps>,
    pub depth_impact_bps: Bps,
    pub execution_buffer_bps: Bps,
    pub funding_state: FundingState,
    pub funding_adjustment_bps: Option<Bps>,
    pub long_exchange_ts: UnixNanos,
    pub short_exchange_ts: UnixNanos,
    pub long_receive_ts: UnixNanos,
    pub short_receive_ts: UnixNanos,
    pub long_age_ms: DurationMillis,
    pub short_age_ms: DurationMillis,
    pub receive_skew_ms: DurationMillis,
    pub observed_at: UnixNanos,
}

/// Complete immutable P3 measurement facts for one explicit oriented route.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpreadSnapshot {
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub long_instrument_id: InstrumentId,
    pub short_instrument_id: InstrumentId,
    pub requested_base_quantity: BaseQty,
    pub maximum_executable_base_quantity: BaseQty,
    pub executable_long_price: Price,
    pub executable_short_price: Price,
    pub executable_notional: Notional,
    pub raw_executable_premium_bps: Bps,
    pub midline_bps: Option<Bps>,
    pub deviation_bps: Option<Bps>,
    pub fee_bps: Option<Bps>,
    pub depth_impact_bps: Bps,
    pub execution_buffer_bps: Bps,
    pub funding_state: FundingState,
    pub funding_adjustment_bps: Option<Bps>,
    pub other_explicit_risk_costs_bps: Vec<ExplicitRiskCost>,
    pub tradable_edge_bps: Option<Bps>,
    pub fair_value_sample_count: usize,
    pub fair_value_dispersion_bps: Option<Bps>,
    pub long_age_ms: DurationMillis,
    pub short_age_ms: DurationMillis,
    pub receive_skew_ms: DurationMillis,
    pub timestamp: UnixNanos,
    pub validity: MeasurementValidity,
    pub rejection_reason: Option<String>,
    pub model_version: ModelVersion,
    pub config_fingerprint: String,
}
