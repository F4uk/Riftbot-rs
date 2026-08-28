//! Versioned in-memory order books and deterministic feed health.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::domain::{
    ids::{InstrumentId, VenueId},
    market::{BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook},
    numeric::{DurationMillis, UnixNanos},
};

const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Stable map key for a venue/instrument public feed.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FeedKey {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
}

impl FeedKey {
    #[must_use]
    pub fn new(venue_id: VenueId, instrument_id: InstrumentId) -> Self {
        Self {
            venue_id,
            instrument_id,
        }
    }
}

#[derive(Clone, Debug)]
struct FeedLifecycle {
    connection: FeedConnectionState,
    last_transition_ts: UnixNanos,
    awaiting_recovery_data: bool,
}

/// Invalid or regressive update rejected by the store.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum BookStoreError {
    #[error("stale_after_ms must be non-zero")]
    ZeroStaleThreshold,
    #[error("connection transition timestamp must be non-zero")]
    ZeroTransitionTimestamp,
    #[error("connection transition regressed from {current:?} to {incoming:?}")]
    RegressiveTransition {
        current: UnixNanos,
        incoming: UnixNanos,
    },
    #[error("book version must increase beyond {current:?}, received {incoming:?}")]
    NonMonotonicVersion {
        current: BookVersion,
        incoming: BookVersion,
    },
    #[error("book receive timestamp regressed from {current:?} to {incoming:?}")]
    RegressiveReceiveTimestamp {
        current: UnixNanos,
        incoming: UnixNanos,
    },
}

/// Latest accepted full book plus connection and freshness state.
#[derive(Clone, Debug)]
pub struct BookStore {
    stale_after_ms: DurationMillis,
    books: BTreeMap<FeedKey, VenueBook>,
    feeds: BTreeMap<FeedKey, FeedLifecycle>,
}

impl BookStore {
    /// Creates a store with a deterministic freshness threshold.
    pub fn new(stale_after_ms: DurationMillis) -> Result<Self, BookStoreError> {
        if stale_after_ms.0 == 0 {
            return Err(BookStoreError::ZeroStaleThreshold);
        }
        Ok(Self {
            stale_after_ms,
            books: BTreeMap::new(),
            feeds: BTreeMap::new(),
        })
    }

    /// Records a socket lifecycle transition. Disconnect and reconnect require a new book before
    /// the feed can be healthy again.
    pub fn set_connection_state(
        &mut self,
        key: FeedKey,
        state: FeedConnectionState,
        transition_ts: UnixNanos,
    ) -> Result<(), BookStoreError> {
        if transition_ts.0 == 0 {
            return Err(BookStoreError::ZeroTransitionTimestamp);
        }
        if let Some(current) = self.feeds.get(&key)
            && transition_ts < current.last_transition_ts
        {
            return Err(BookStoreError::RegressiveTransition {
                current: current.last_transition_ts,
                incoming: transition_ts,
            });
        }

        let awaiting_recovery_data = match state {
            FeedConnectionState::Connected => self
                .feeds
                .get(&key)
                .is_some_and(|current| current.awaiting_recovery_data),
            FeedConnectionState::Connecting
            | FeedConnectionState::Reconnecting
            | FeedConnectionState::Disconnected => true,
        };
        self.feeds.insert(
            key,
            FeedLifecycle {
                connection: state,
                last_transition_ts: transition_ts,
                awaiting_recovery_data,
            },
        );
        Ok(())
    }

    /// Inserts a newer normalized snapshot and marks the feed recovered.
    pub fn update(&mut self, mut book: VenueBook) -> Result<(), BookStoreError> {
        let key = FeedKey::new(book.venue_id.clone(), book.instrument_id.clone());
        if let Some(current) = self.books.get(&key) {
            if book.version <= current.version {
                return Err(BookStoreError::NonMonotonicVersion {
                    current: current.version,
                    incoming: book.version,
                });
            }
            if book.receive_ts < current.receive_ts {
                return Err(BookStoreError::RegressiveReceiveTimestamp {
                    current: current.receive_ts,
                    incoming: book.receive_ts,
                });
            }
        }

        book.age_ms = DurationMillis(0);
        let transition_ts = self
            .feeds
            .get(&key)
            .map_or(book.receive_ts, |feed| feed.last_transition_ts);
        self.books.insert(key.clone(), book);
        self.feeds.insert(
            key,
            FeedLifecycle {
                connection: FeedConnectionState::Connected,
                last_transition_ts: transition_ts,
                awaiting_recovery_data: false,
            },
        );
        Ok(())
    }

    /// Returns the latest book with age recalculated at the supplied time.
    #[must_use]
    pub fn book(&self, key: &FeedKey, now: UnixNanos) -> Option<VenueBook> {
        self.books.get(key).cloned().map(|mut book| {
            book.age_ms = age_at(book.receive_ts, now);
            book
        })
    }

    /// Returns an auditable combined health snapshot at the supplied time.
    #[must_use]
    pub fn health(&self, key: &FeedKey, now: UnixNanos) -> Option<FeedHealth> {
        let lifecycle = self.feeds.get(key)?;
        let book = self.books.get(key);
        let age_ms = book.map(|book| age_at(book.receive_ts, now));
        let freshness = if lifecycle.awaiting_recovery_data {
            FeedFreshness::AwaitingRecovery
        } else if let Some(age) = age_ms {
            if age > self.stale_after_ms {
                FeedFreshness::Stale
            } else {
                FeedFreshness::Fresh
            }
        } else {
            FeedFreshness::Missing
        };

        Some(FeedHealth {
            venue_id: key.venue_id.clone(),
            instrument_id: key.instrument_id.clone(),
            connection: lifecycle.connection,
            freshness,
            last_transition_ts: lifecycle.last_transition_ts,
            last_exchange_ts: book.map(|book| book.exchange_ts),
            last_receive_ts: book.map(|book| book.receive_ts),
            age_ms,
        })
    }

    /// Returns a book only when connection and freshness checks both pass.
    #[must_use]
    pub fn healthy_book(&self, key: &FeedKey, now: UnixNanos) -> Option<VenueBook> {
        self.health(key, now)
            .filter(FeedHealth::is_healthy)
            .and_then(|_| self.book(key, now))
    }
}

fn age_at(receive_ts: UnixNanos, now: UnixNanos) -> DurationMillis {
    DurationMillis(now.0.saturating_sub(receive_ts.0) / NANOS_PER_MILLISECOND)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{BookStore, BookStoreError, FeedKey};
    use crate::domain::{
        ids::{InstrumentId, VenueId},
        market::{BookLevel, BookVersion, FeedConnectionState, FeedFreshness, VenueBook},
        numeric::{BaseQty, DurationMillis, Price, UnixNanos},
    };

    fn key() -> Result<FeedKey, Box<dyn Error>> {
        Ok(FeedKey::new(
            VenueId::try_from("lighter")?,
            InstrumentId::try_from("BTC-PERP.LIGHTER")?,
        ))
    }

    fn book(version: u64, receive_ts: u64) -> Result<VenueBook, Box<dyn Error>> {
        let key = key()?;
        Ok(VenueBook {
            venue_id: key.venue_id,
            instrument_id: key.instrument_id,
            bids: vec![BookLevel {
                price: Price::new(Decimal::new(100, 0))?,
                quantity: BaseQty::new(Decimal::ONE)?,
            }],
            asks: vec![BookLevel {
                price: Price::new(Decimal::new(101, 0))?,
                quantity: BaseQty::new(Decimal::ONE)?,
            }],
            exchange_ts: UnixNanos(receive_ts - 1_000_000),
            receive_ts: UnixNanos(receive_ts),
            age_ms: DurationMillis(999),
            version: BookVersion(version),
        })
    }

    #[test]
    fn tracks_freshness_without_wall_clock() -> Result<(), Box<dyn Error>> {
        let mut store = BookStore::new(DurationMillis(1_500))?;
        let key = key()?;
        store.set_connection_state(
            key.clone(),
            FeedConnectionState::Connected,
            UnixNanos(1_000_000_000),
        )?;
        store.update(book(1, 1_100_000_000)?)?;

        let fresh = store
            .health(&key, UnixNanos(2_600_000_000))
            .expect("feed exists");
        assert_eq!(fresh.age_ms, Some(DurationMillis(1_500)));
        assert_eq!(fresh.freshness, FeedFreshness::Fresh);
        assert!(fresh.is_healthy());

        let stale = store
            .health(&key, UnixNanos(2_601_000_000))
            .expect("feed exists");
        assert_eq!(stale.freshness, FeedFreshness::Stale);
        assert!(!stale.is_healthy());
        assert!(store.healthy_book(&key, UnixNanos(2_601_000_000)).is_none());
        Ok(())
    }

    #[test]
    fn reconnect_requires_newer_recovery_book() -> Result<(), Box<dyn Error>> {
        let mut store = BookStore::new(DurationMillis(1_500))?;
        let key = key()?;
        store.update(book(4, 2_000_000_000)?)?;
        store.set_connection_state(
            key.clone(),
            FeedConnectionState::Disconnected,
            UnixNanos(2_100_000_000),
        )?;
        store.set_connection_state(
            key.clone(),
            FeedConnectionState::Reconnecting,
            UnixNanos(2_200_000_000),
        )?;
        store.set_connection_state(
            key.clone(),
            FeedConnectionState::Connected,
            UnixNanos(2_300_000_000),
        )?;

        let waiting = store
            .health(&key, UnixNanos(2_300_000_000))
            .expect("feed exists");
        assert_eq!(waiting.freshness, FeedFreshness::AwaitingRecovery);
        assert!(!waiting.is_healthy());

        assert!(matches!(
            store.update(book(4, 2_400_000_000)?),
            Err(BookStoreError::NonMonotonicVersion { .. })
        ));
        store.update(book(5, 2_400_000_000)?)?;
        assert!(
            store
                .health(&key, UnixNanos(2_400_000_000))
                .expect("feed exists")
                .is_healthy()
        );
        Ok(())
    }

    #[test]
    fn rejects_regressive_book_and_transition_updates() -> Result<(), Box<dyn Error>> {
        let mut store = BookStore::new(DurationMillis(500))?;
        let key = key()?;
        store.update(book(2, 2_000_000_000)?)?;
        assert!(matches!(
            store.update(book(3, 1_900_000_000)?),
            Err(BookStoreError::RegressiveReceiveTimestamp { .. })
        ));
        store.set_connection_state(
            key.clone(),
            FeedConnectionState::Disconnected,
            UnixNanos(3_000_000_000),
        )?;
        assert!(matches!(
            store.set_connection_state(
                key,
                FeedConnectionState::Reconnecting,
                UnixNanos(2_999_999_999)
            ),
            Err(BookStoreError::RegressiveTransition { .. })
        ));
        Ok(())
    }
}
