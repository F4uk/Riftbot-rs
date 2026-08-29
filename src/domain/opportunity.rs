//! Measurement-only opportunity facts. This module has no inventory or execution dependency.

use serde::{Deserialize, Serialize};

use super::{
    ids::{ModelVersion, PairId, Symbol, VenueId},
    numeric::{Bps, Notional, Price, UnixNanos},
    risk::Regime,
    spread::{ExplicitRiskCost, FundingState, MeasurementValidity},
};

/// P3 packaging/evaluation output for one explicit oriented route.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Opportunity {
    pub pair_id: PairId,
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
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
    pub regime: Regime,
    pub validity: MeasurementValidity,
    pub rejection_reason: Option<String>,
    pub increase_risk_economically_allowed: bool,
    pub timestamp: UnixNanos,
    pub model_version: ModelVersion,
    pub config_fingerprint: String,
}
