//! Executable spread measurement contracts; these types do not choose inventory.

use serde::{Deserialize, Serialize};

use super::{
    ids::{Symbol, VenueId},
    numeric::{Bps, Notional, Price, UnixNanos},
};

/// Why a spread measurement may or may not be usable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementValidity {
    Valid,
    WarmingUp,
    StaleBook,
    EmptyBookSide,
    InsufficientDepth,
    CrossedOrCorruptBook,
    VenueUnhealthy,
}

/// Frozen executable spread facts for one long/short venue direction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpreadSnapshot {
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub executable_long_price: Price,
    pub executable_short_price: Price,
    pub executable_notional: Notional,
    pub gross_spread_bps: Bps,
    pub fee_bps: Bps,
    pub estimated_slippage_bps: Bps,
    pub funding_adjustment_bps: Bps,
    pub risk_buffer_bps: Bps,
    pub net_edge_bps: Bps,
    pub midline_bps: Option<Bps>,
    pub deviation_bps: Option<Bps>,
    pub timestamp: UnixNanos,
    pub validity: MeasurementValidity,
}
