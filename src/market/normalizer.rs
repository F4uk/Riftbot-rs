//! Deterministic normalization for full public L2 snapshots.

use std::{cmp::Reverse, collections::HashSet};

use rust_decimal::Decimal;
use thiserror::Error;

use crate::domain::{
    ids::{InstrumentId, VenueId},
    market::{BookLevel, BookVersion, VenueBook},
    numeric::{BaseQty, DurationMillis, NumericError, Price, UnixNanos},
};

/// Unvalidated price level received at the Nautilus edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawBookLevel {
    pub price: Decimal,
    pub quantity: Decimal,
}

/// Full book snapshot ready for domain normalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBookSnapshot {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub bids: Vec<RawBookLevel>,
    pub asks: Vec<RawBookLevel>,
    pub exchange_ts: UnixNanos,
    pub receive_ts: UnixNanos,
    pub version: BookVersion,
}

/// Side associated with a malformed input level.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BookSide {
    Bid,
    Ask,
}

/// Snapshot rejected before it can enter the store.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum NormalizationError {
    #[error("{side:?} side is empty")]
    EmptySide { side: BookSide },
    #[error("{side:?} level {index} has invalid price: {source}")]
    InvalidPrice {
        side: BookSide,
        index: usize,
        source: NumericError,
    },
    #[error("{side:?} level {index} has invalid quantity: {source}")]
    InvalidQuantity {
        side: BookSide,
        index: usize,
        source: NumericError,
    },
    #[error("{side:?} side contains duplicate price {price}")]
    DuplicatePrice { side: BookSide, price: Decimal },
    #[error("crossed or locked book: best bid {best_bid} >= best ask {best_ask}")]
    CrossedBook {
        best_bid: Decimal,
        best_ask: Decimal,
    },
    #[error("{field} timestamp must be non-zero")]
    MissingTimestamp { field: &'static str },
}

/// Stateless canonicalizer for adapter-provided full snapshots.
#[derive(Clone, Copy, Debug, Default)]
pub struct MarketNormalizer;

impl MarketNormalizer {
    /// Validates a snapshot and sorts bids descending and asks ascending.
    pub fn normalize(snapshot: RawBookSnapshot) -> Result<VenueBook, NormalizationError> {
        if snapshot.exchange_ts.0 == 0 {
            return Err(NormalizationError::MissingTimestamp {
                field: "exchange_ts",
            });
        }
        if snapshot.receive_ts.0 == 0 {
            return Err(NormalizationError::MissingTimestamp {
                field: "receive_ts",
            });
        }

        let mut bids = normalize_side(snapshot.bids, BookSide::Bid)?;
        let mut asks = normalize_side(snapshot.asks, BookSide::Ask)?;
        bids.sort_unstable_by_key(|level| Reverse(level.price.value()));
        asks.sort_unstable_by_key(|level| level.price.value());

        let best_bid = bids[0].price.value();
        let best_ask = asks[0].price.value();
        if best_bid >= best_ask {
            return Err(NormalizationError::CrossedBook { best_bid, best_ask });
        }

        Ok(VenueBook {
            venue_id: snapshot.venue_id,
            instrument_id: snapshot.instrument_id,
            bids,
            asks,
            exchange_ts: snapshot.exchange_ts,
            receive_ts: snapshot.receive_ts,
            age_ms: DurationMillis(0),
            version: snapshot.version,
        })
    }
}

fn normalize_side(
    levels: Vec<RawBookLevel>,
    side: BookSide,
) -> Result<Vec<BookLevel>, NormalizationError> {
    if levels.is_empty() {
        return Err(NormalizationError::EmptySide { side });
    }

    let mut prices = HashSet::with_capacity(levels.len());
    let mut normalized = Vec::with_capacity(levels.len());
    for (index, level) in levels.into_iter().enumerate() {
        let price = Price::new(level.price).map_err(|source| NormalizationError::InvalidPrice {
            side,
            index,
            source,
        })?;
        let quantity =
            BaseQty::new(level.quantity).map_err(|source| NormalizationError::InvalidQuantity {
                side,
                index,
                source,
            })?;
        if !prices.insert(level.price) {
            return Err(NormalizationError::DuplicatePrice {
                side,
                price: level.price,
            });
        }
        normalized.push(BookLevel { price, quantity });
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{BookSide, MarketNormalizer, NormalizationError, RawBookLevel, RawBookSnapshot};
    use crate::domain::{
        ids::{InstrumentId, VenueId},
        market::BookVersion,
        numeric::UnixNanos,
    };

    fn level(mantissa: i64, scale: u32, quantity: i64) -> RawBookLevel {
        RawBookLevel {
            price: Decimal::new(mantissa, scale),
            quantity: Decimal::new(quantity, 0),
        }
    }

    fn snapshot() -> Result<RawBookSnapshot, Box<dyn Error>> {
        Ok(RawBookSnapshot {
            venue_id: VenueId::try_from("entropy")?,
            instrument_id: InstrumentId::try_from("BTC-USD-PERP.HYPERLIQUID")?,
            bids: vec![level(99, 0, 2), level(100, 0, 1)],
            asks: vec![level(102, 0, 2), level(101, 0, 1)],
            exchange_ts: UnixNanos(1_000_000_000),
            receive_ts: UnixNanos(1_001_000_000),
            version: BookVersion(7),
        })
    }

    #[test]
    fn canonicalizes_side_ordering() -> Result<(), Box<dyn Error>> {
        let book = MarketNormalizer::normalize(snapshot()?)?;
        assert_eq!(book.bids[0].price.value(), Decimal::new(100, 0));
        assert_eq!(book.asks[0].price.value(), Decimal::new(101, 0));
        assert_eq!(book.age_ms.0, 0);
        Ok(())
    }

    #[test]
    fn rejects_empty_side_and_zero_level() -> Result<(), Box<dyn Error>> {
        let mut empty = snapshot()?;
        empty.bids.clear();
        assert_eq!(
            MarketNormalizer::normalize(empty),
            Err(NormalizationError::EmptySide {
                side: BookSide::Bid
            })
        );

        let mut zero = snapshot()?;
        zero.asks[0].quantity = Decimal::ZERO;
        assert!(matches!(
            MarketNormalizer::normalize(zero),
            Err(NormalizationError::InvalidQuantity {
                side: BookSide::Ask,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn rejects_duplicate_and_crossed_books() -> Result<(), Box<dyn Error>> {
        let mut duplicate = snapshot()?;
        duplicate.bids.push(level(100, 0, 3));
        assert_eq!(
            MarketNormalizer::normalize(duplicate),
            Err(NormalizationError::DuplicatePrice {
                side: BookSide::Bid,
                price: Decimal::new(100, 0),
            })
        );

        let mut crossed = snapshot()?;
        crossed.bids[0].price = Decimal::new(102, 0);
        assert!(matches!(
            MarketNormalizer::normalize(crossed),
            Err(NormalizationError::CrossedBook { .. })
        ));
        Ok(())
    }

    #[test]
    fn requires_both_timestamps() -> Result<(), Box<dyn Error>> {
        let mut missing = snapshot()?;
        missing.exchange_ts = UnixNanos(0);
        assert_eq!(
            MarketNormalizer::normalize(missing),
            Err(NormalizationError::MissingTimestamp {
                field: "exchange_ts"
            })
        );
        Ok(())
    }
}
