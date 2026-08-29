//! Versioned, strict recording contracts for public market-data replay.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    domain::{
        decision::OrderEventKind,
        ids::{ClientOrderId, InstrumentId, IntentId, VenueId, VenueOrderId},
        market::{BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook},
        numeric::{BaseQty, DurationMillis, Price, UnixNanos},
    },
    market::{
        book_store::FeedKey,
        normalizer::{MarketNormalizer, NormalizationError, RawBookLevel, RawBookSnapshot},
    },
};

/// Current on-disk schema. Readers reject every other version.
pub const RECORDING_SCHEMA_VERSION: u16 = 1;
/// Stable file marker which prevents unrelated JSONL from being treated as a recording.
pub const RECORDING_FORMAT: &str = "riftbot-recording";

/// Header at line one of every complete recording.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordingHeader {
    pub format: String,
    pub schema_version: u16,
}

impl RecordingHeader {
    #[must_use]
    pub fn v1() -> Self {
        Self {
            format: RECORDING_FORMAT.to_owned(),
            schema_version: RECORDING_SCHEMA_VERSION,
        }
    }
}

/// One persisted event plus its deterministic FIFO sequence and payload checksum.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventEnvelope {
    pub sequence: u64,
    pub event: RecordedEvent,
    pub checksum_sha256: String,
}

/// The exact bytes covered by each event checksum.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SequencedEvent {
    pub sequence: u64,
    pub event: RecordedEvent,
}

/// Mandatory last line. Its checksum covers the header and every complete event line.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordingTrailer {
    pub schema_version: u16,
    pub event_count: u64,
    pub content_sha256: String,
}

/// A normalized fixed-decimal level as persisted at the P1 domain boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedBookLevel {
    #[serde(with = "rust_decimal::serde::str")]
    pub price: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub quantity: Decimal,
}

/// Normalized book event. Age is deliberately absent because replay derives it from `receive_ts`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedMarketBook {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub bids: Vec<RecordedBookLevel>,
    pub asks: Vec<RecordedBookLevel>,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
    pub version: BookVersion,
}

impl RecordedMarketBook {
    /// Captures an already-normalized book while excluding its derived age.
    pub fn from_domain(book: &VenueBook) -> Result<Self, SchemaError> {
        let recorded = Self::from_domain_unchecked(book);
        recorded.to_domain()?;
        Ok(recorded)
    }

    /// Revalidates and canonicalizes the persisted event through the P1 normalizer.
    pub fn to_domain(&self) -> Result<VenueBook, SchemaError> {
        if self.version.0 == 0 {
            return Err(SchemaError::ZeroBookVersion);
        }
        let normalized = MarketNormalizer::normalize(RawBookSnapshot {
            venue_id: self.venue_id.clone(),
            instrument_id: self.instrument_id.clone(),
            bids: self
                .bids
                .iter()
                .map(|level| RawBookLevel {
                    price: level.price,
                    quantity: level.quantity,
                })
                .collect(),
            asks: self
                .asks
                .iter()
                .map(|level| RawBookLevel {
                    price: level.price,
                    quantity: level.quantity,
                })
                .collect(),
            exchange_ts: self.exchange_ts,
            receive_ts: self.receive_ts,
            version: self.version,
        })?;
        if Self::from_domain_unchecked(&normalized) != *self {
            return Err(SchemaError::NonCanonicalMarketBook);
        }
        Ok(normalized)
    }

    fn from_domain_unchecked(book: &VenueBook) -> Self {
        Self {
            venue_id: book.venue_id.clone(),
            instrument_id: book.instrument_id.clone(),
            bids: book
                .bids
                .iter()
                .map(|level| RecordedBookLevel {
                    price: level.price.value(),
                    quantity: level.quantity.value(),
                })
                .collect(),
            asks: book
                .asks
                .iter()
                .map(|level| RecordedBookLevel {
                    price: level.price.value(),
                    quantity: level.quantity.value(),
                })
                .collect(),
            exchange_ts: book.exchange_ts,
            receive_ts: book.receive_ts,
            version: book.version,
        }
    }
}

/// Explicit transport lifecycle input required to reproduce P1 feed health.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedFeedConnection {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub state: FeedConnectionState,
    pub transition_ts: UnixNanos,
}

/// A deterministic health assertion evaluated at a caller-supplied recorded time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedFeedHealth {
    pub observed_at: UnixNanos,
    pub expected: RecordedFeedHealthState,
}

/// Strict persisted form of the derived P1 health snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedFeedHealthState {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub connection: FeedConnectionState,
    pub freshness: FeedFreshness,
    pub last_transition_ts: UnixNanos,
    pub last_exchange_ts: Option<UnixNanos>,
    pub last_receive_ts: Option<UnixNanos>,
    pub age_ms: Option<DurationMillis>,
}

impl RecordedFeedHealthState {
    #[must_use]
    pub fn from_domain(health: FeedHealth) -> Self {
        Self {
            venue_id: health.venue_id,
            instrument_id: health.instrument_id,
            connection: health.connection,
            freshness: health.freshness,
            last_transition_ts: health.last_transition_ts,
            last_exchange_ts: health.last_exchange_ts,
            last_receive_ts: health.last_receive_ts,
            age_ms: health.age_ms,
        }
    }

    #[must_use]
    pub fn to_domain(&self) -> FeedHealth {
        FeedHealth {
            venue_id: self.venue_id.clone(),
            instrument_id: self.instrument_id.clone(),
            connection: self.connection,
            freshness: self.freshness,
            last_transition_ts: self.last_transition_ts,
            last_exchange_ts: self.last_exchange_ts,
            last_receive_ts: self.last_receive_ts,
            age_ms: self.age_ms,
        }
    }
}

/// Events implemented by P2. Future private-side schemas are intentionally not variants here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordedEvent {
    MarketBook { book: RecordedMarketBook },
    FeedConnection { connection: RecordedFeedConnection },
    FeedHealth { health: RecordedFeedHealth },
}

impl RecordedEvent {
    /// Constructs and validates a normalized public book event.
    pub fn market_book(book: &VenueBook) -> Result<Self, SchemaError> {
        Ok(Self::MarketBook {
            book: RecordedMarketBook::from_domain(book)?,
        })
    }

    /// Constructs an explicit feed transport transition.
    pub fn feed_connection(
        key: &FeedKey,
        state: FeedConnectionState,
        transition_ts: UnixNanos,
    ) -> Result<Self, SchemaError> {
        let event = Self::FeedConnection {
            connection: RecordedFeedConnection {
                venue_id: key.venue_id.clone(),
                instrument_id: key.instrument_id.clone(),
                state,
                transition_ts,
            },
        };
        event.validate()?;
        Ok(event)
    }

    /// Captures a P1 health state at an explicit replay timestamp.
    pub fn feed_health(observed_at: UnixNanos, expected: FeedHealth) -> Result<Self, SchemaError> {
        let event = Self::FeedHealth {
            health: RecordedFeedHealth {
                observed_at,
                expected: RecordedFeedHealthState::from_domain(expected),
            },
        };
        event.validate()?;
        Ok(event)
    }

    /// Validates all invariants which do not depend on earlier events.
    pub fn validate(&self) -> Result<(), SchemaError> {
        match self {
            Self::MarketBook { book } => {
                book.to_domain()?;
            }
            Self::FeedConnection { connection } => {
                if connection.transition_ts.0 == 0 {
                    return Err(SchemaError::ZeroEventTimestamp {
                        field: "transition_ts",
                    });
                }
            }
            Self::FeedHealth { health } => {
                if health.observed_at.0 == 0 {
                    return Err(SchemaError::ZeroEventTimestamp {
                        field: "observed_at",
                    });
                }
                if health.expected.last_transition_ts.0 == 0 {
                    return Err(SchemaError::ZeroEventTimestamp {
                        field: "last_transition_ts",
                    });
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub(crate) fn event_time(&self) -> UnixNanos {
        match self {
            Self::MarketBook { book } => book.receive_ts,
            Self::FeedConnection { connection } => connection.transition_ts,
            Self::FeedHealth { health } => health.observed_at,
        }
    }

    #[must_use]
    pub(crate) fn feed_key(&self) -> FeedKey {
        match self {
            Self::MarketBook { book } => {
                FeedKey::new(book.venue_id.clone(), book.instrument_id.clone())
            }
            Self::FeedConnection { connection } => FeedKey::new(
                connection.venue_id.clone(),
                connection.instrument_id.clone(),
            ),
            Self::FeedHealth { health } => FeedKey::new(
                health.expected.venue_id.clone(),
                health.expected.instrument_id.clone(),
            ),
        }
    }
}

/// Future account-event shape. It deliberately excludes credentials, account IDs, and raw payloads.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FutureAccountEventSchema {
    pub venue_id: VenueId,
    pub kind: FutureAccountEventKind,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FutureAccountEventKind {
    SnapshotReceived,
    BalanceChanged,
    PositionChanged,
}

/// Future order-event shape for audit correlation; P2 has no producer or execution consumer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FutureOrderEventSchema {
    pub venue_id: VenueId,
    pub intent_id: IntentId,
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub instrument_id: InstrumentId,
    pub event: OrderEventKind,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
}

/// Future fill-event shape for audit correlation; P2 has no producer or execution consumer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FutureFillEventSchema {
    pub venue_id: VenueId,
    pub intent_id: IntentId,
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub instrument_id: InstrumentId,
    pub quantity: BaseQty,
    pub price: Price,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SchemaError {
    #[error("book version must be non-zero")]
    ZeroBookVersion,
    #[error("recorded market book is not in canonical normalized order")]
    NonCanonicalMarketBook,
    #[error("{field} must be non-zero")]
    ZeroEventTimestamp { field: &'static str },
    #[error(transparent)]
    Normalization(#[from] NormalizationError),
}

#[must_use]
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
