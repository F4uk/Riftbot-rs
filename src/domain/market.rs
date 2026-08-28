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

/// Public market-data connection lifecycle for one venue feed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedConnectionState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

/// Freshness of the last accepted book independently of connection state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedFreshness {
    Missing,
    AwaitingRecovery,
    Fresh,
    Stale,
}

/// Auditable health snapshot for a single venue and instrument feed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FeedHealth {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub connection: FeedConnectionState,
    pub freshness: FeedFreshness,
    pub last_transition_ts: UnixNanos,
    pub last_exchange_ts: Option<UnixNanos>,
    pub last_receive_ts: Option<UnixNanos>,
    pub age_ms: Option<DurationMillis>,
}

impl FeedHealth {
    /// Returns true only after a connected feed has supplied a fresh recovery book.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.connection == FeedConnectionState::Connected && self.freshness == FeedFreshness::Fresh
    }
}
