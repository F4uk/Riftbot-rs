//! Venue-agnostic opportunity facts used to avoid pair-specific strategy branches.

use serde::{Deserialize, Serialize};

use super::{
    ids::{Symbol, VenueId},
    numeric::{Bps, Notional},
    spread::MeasurementValidity,
};

/// A ranked candidate shape; ranking behavior belongs to a later stage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Opportunity {
    pub symbol: Symbol,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
    pub net_edge_bps: Bps,
    pub midline_bps: Option<Bps>,
    pub deviation_bps: Option<Bps>,
    pub executable_notional: Notional,
    pub validity: MeasurementValidity,
}
