//! Normalized order-book contracts.

use serde::{Deserialize, Serialize};

use super::{
    ids::{InstrumentId, VenueId},
    numeric::{BaseQty, DurationMillis, Price, UnixNanos},
};

/// A normalized price level.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BookLevel {
    pub price: Price,
    pub quantity: BaseQty,
}

/// Monotonic adapter-provided book sequence or local version.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BookVersion(pub u64);

/// Venue order book with both venue and local receipt time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VenueBook {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
    pub age_ms: DurationMillis,
    pub version: BookVersion,
}
