//! Deterministic real-L2 VWAP measurement for one explicit oriented route.

use rust_decimal::Decimal;
use thiserror::Error;

use crate::domain::{
    ids::{PairId, Symbol, VenueId},
    market::{BookLevel, FeedHealth, VenueBook},
    numeric::{BaseQty, Bps, DurationMillis, Notional, NumericError, Price, UnixNanos},
    spread::{ExecutableRouteMeasurement, FundingState, MeasurementValidity},
};

const BPS_SCALE: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);
const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Explicit cost inputs for one route. Venue fees are per fill; the engine expands them to a
/// four-fill expected round trip (entry two-leg plus expected exit two-leg).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteCostInput {
    pub long_taker_fee_bps: Option<Bps>,
    pub short_taker_fee_bps: Option<Bps>,
    pub execution_buffer_bps: Bps,
    pub funding_state: FundingState,
    pub funding_adjustment_bps: Option<Bps>,
}

/// All caller-supplied facts needed to measure one oriented route at logical time.
#[derive(Clone, Copy, Debug)]
pub struct RouteMeasurementInput<'a> {
    pub pair_id: &'a PairId,
    pub symbol: &'a Symbol,
    pub long_book: &'a VenueBook,
    pub short_book: &'a VenueBook,
    pub long_health: &'a FeedHealth,
    pub short_health: &'a FeedHealth,
    pub requested_base_quantity: BaseQty,
    pub max_book_age_ms: DurationMillis,
    pub max_receive_skew_ms: DurationMillis,
    pub observed_at: UnixNanos,
    pub costs: RouteCostInput,
}

/// Fail-closed reason an executable route could not be measured.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SpreadError {
    #[error("long and short venue must differ")]
    SameVenue,
    #[error("{leg} book and health identity do not match")]
    HealthIdentityMismatch { leg: &'static str },
    #[error("{leg} feed is not connected and fresh: {venue}")]
    VenueUnhealthy { leg: &'static str, venue: VenueId },
    #[error("{leg} receive timestamp is later than logical observation time")]
    FutureReceiveTimestamp { leg: &'static str },
    #[error("{leg} book age {age_ms}ms exceeds {maximum_ms}ms")]
    StaleBook {
        leg: &'static str,
        age_ms: u64,
        maximum_ms: u64,
    },
    #[error("book receive-time skew {actual_ms}ms exceeds {maximum_ms}ms")]
    ReceiveTimeSkew { actual_ms: u64, maximum_ms: u64 },
    #[error("{venue} {side} side is empty")]
    EmptyBookSide { venue: VenueId, side: &'static str },
    #[error("{venue} book is non-canonical, locked, or crossed")]
    CrossedOrCorruptBook { venue: VenueId },
    #[error("insufficient {side} depth: requested {requested}, available {available}")]
    InsufficientDepth {
        side: &'static str,
        requested: Decimal,
        available: Decimal,
    },
    #[error("{field} must be non-negative")]
    NegativeCost { field: &'static str },
    #[error("funding state and adjustment value are inconsistent")]
    InvalidFundingState,
    #[error("fixed-decimal arithmetic overflow or division failure")]
    Arithmetic,
    #[error("derived numeric value is invalid: {0}")]
    Numeric(#[from] NumericError),
}

impl SpreadError {
    #[must_use]
    pub const fn validity(&self) -> MeasurementValidity {
        match self {
            Self::VenueUnhealthy { .. } => MeasurementValidity::VenueUnhealthy,
            Self::StaleBook { .. } | Self::FutureReceiveTimestamp { .. } => {
                MeasurementValidity::StaleBook
            }
            Self::ReceiveTimeSkew { .. } => MeasurementValidity::ReceiveTimeSkew,
            Self::EmptyBookSide { .. } => MeasurementValidity::EmptyBookSide,
            Self::InsufficientDepth { .. } => MeasurementValidity::InsufficientDepth,
            Self::SameVenue
            | Self::HealthIdentityMismatch { .. }
            | Self::CrossedOrCorruptBook { .. }
            | Self::NegativeCost { .. }
            | Self::InvalidFundingState
            | Self::Arithmetic
            | Self::Numeric(_) => MeasurementValidity::CrossedOrCorruptBook,
        }
    }
}

/// Stateless executable-route calculator. It has no clock and no venue client.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpreadEngine;

impl SpreadEngine {
    pub fn measure(
        input: RouteMeasurementInput<'_>,
    ) -> Result<ExecutableRouteMeasurement, SpreadError> {
        if input.long_book.venue_id == input.short_book.venue_id {
            return Err(SpreadError::SameVenue);
        }
        validate_health("long", input.long_book, input.long_health)?;
        validate_health("short", input.short_book, input.short_health)?;
        validate_book(input.long_book)?;
        validate_book(input.short_book)?;

        let long_age_ms = age_at("long", input.long_book.receive_ts, input.observed_at)?;
        let short_age_ms = age_at("short", input.short_book.receive_ts, input.observed_at)?;
        ensure_fresh("long", long_age_ms, input.max_book_age_ms)?;
        ensure_fresh("short", short_age_ms, input.max_book_age_ms)?;
        let receive_skew_ms = DurationMillis(
            input
                .long_book
                .receive_ts
                .0
                .abs_diff(input.short_book.receive_ts.0)
                / NANOS_PER_MILLISECOND,
        );
        if receive_skew_ms > input.max_receive_skew_ms {
            return Err(SpreadError::ReceiveTimeSkew {
                actual_ms: receive_skew_ms.0,
                maximum_ms: input.max_receive_skew_ms.0,
            });
        }
        validate_costs(input.costs)?;

        let requested = input.requested_base_quantity.value();
        let long_available = total_quantity(&input.long_book.asks)?;
        let short_available = total_quantity(&input.short_book.bids)?;
        let maximum_executable = long_available.min(short_available);
        if long_available < requested {
            return Err(SpreadError::InsufficientDepth {
                side: "long asks",
                requested,
                available: long_available,
            });
        }
        if short_available < requested {
            return Err(SpreadError::InsufficientDepth {
                side: "short bids",
                requested,
                available: short_available,
            });
        }

        let buy_vwap = walk_vwap(&input.long_book.asks, requested, "long asks")?;
        let sell_vwap = walk_vwap(&input.short_book.bids, requested, "short bids")?;
        let best_ask = input.long_book.asks[0].price.value();
        let best_bid = input.short_book.bids[0].price.value();
        let raw_premium = ratio_bps(sell_vwap, buy_vwap)?;
        let buy_depth_impact = ratio_bps(buy_vwap, best_ask)?;
        let sell_depth_impact = ratio_bps(best_bid, sell_vwap)?;
        let depth_impact = buy_depth_impact
            .checked_add(sell_depth_impact)
            .ok_or(SpreadError::Arithmetic)?;
        let executable_notional = buy_vwap
            .checked_mul(requested)
            .ok_or(SpreadError::Arithmetic)?;

        Ok(ExecutableRouteMeasurement {
            pair_id: input.pair_id.clone(),
            symbol: input.symbol.clone(),
            long_venue: input.long_book.venue_id.clone(),
            short_venue: input.short_book.venue_id.clone(),
            long_instrument_id: input.long_book.instrument_id.clone(),
            short_instrument_id: input.short_book.instrument_id.clone(),
            requested_base_quantity: input.requested_base_quantity,
            maximum_executable_base_quantity: BaseQty::new(maximum_executable)?,
            executable_long_price: Price::new(buy_vwap)?,
            executable_short_price: Price::new(sell_vwap)?,
            executable_notional: Notional::new(executable_notional)?,
            raw_executable_premium_bps: Bps::new(raw_premium),
            fee_bps: round_trip_fee_bps(input.costs)?,
            depth_impact_bps: Bps::new(depth_impact),
            execution_buffer_bps: input.costs.execution_buffer_bps,
            funding_state: input.costs.funding_state,
            funding_adjustment_bps: normalized_funding_adjustment(input.costs),
            long_exchange_ts: input.long_book.exchange_ts,
            short_exchange_ts: input.short_book.exchange_ts,
            long_receive_ts: input.long_book.receive_ts,
            short_receive_ts: input.short_book.receive_ts,
            long_age_ms,
            short_age_ms,
            receive_skew_ms,
            observed_at: input.observed_at,
        })
    }
}

fn validate_health(
    leg: &'static str,
    book: &VenueBook,
    health: &FeedHealth,
) -> Result<(), SpreadError> {
    if book.venue_id != health.venue_id || book.instrument_id != health.instrument_id {
        return Err(SpreadError::HealthIdentityMismatch { leg });
    }
    if !health.is_healthy() {
        return Err(SpreadError::VenueUnhealthy {
            leg,
            venue: book.venue_id.clone(),
        });
    }
    Ok(())
}

fn validate_book(book: &VenueBook) -> Result<(), SpreadError> {
    if book.bids.is_empty() {
        return Err(SpreadError::EmptyBookSide {
            venue: book.venue_id.clone(),
            side: "bids",
        });
    }
    if book.asks.is_empty() {
        return Err(SpreadError::EmptyBookSide {
            venue: book.venue_id.clone(),
            side: "asks",
        });
    }
    let bids_canonical = book
        .bids
        .windows(2)
        .all(|window| window[0].price.value() > window[1].price.value());
    let asks_canonical = book
        .asks
        .windows(2)
        .all(|window| window[0].price.value() < window[1].price.value());
    if !bids_canonical
        || !asks_canonical
        || book.bids[0].price.value() >= book.asks[0].price.value()
    {
        return Err(SpreadError::CrossedOrCorruptBook {
            venue: book.venue_id.clone(),
        });
    }
    Ok(())
}

fn validate_costs(costs: RouteCostInput) -> Result<(), SpreadError> {
    for (field, value) in [
        ("long_taker_fee_bps", costs.long_taker_fee_bps),
        ("short_taker_fee_bps", costs.short_taker_fee_bps),
        ("execution_buffer_bps", Some(costs.execution_buffer_bps)),
    ] {
        if value.is_some_and(|bps| bps.value() < Decimal::ZERO) {
            return Err(SpreadError::NegativeCost { field });
        }
    }
    let funding_valid = match costs.funding_state {
        FundingState::Unavailable | FundingState::Disabled => {
            costs.funding_adjustment_bps.is_none()
        }
        FundingState::Available => costs.funding_adjustment_bps.is_some(),
    };
    if !funding_valid {
        return Err(SpreadError::InvalidFundingState);
    }
    Ok(())
}

fn normalized_funding_adjustment(costs: RouteCostInput) -> Option<Bps> {
    match costs.funding_state {
        FundingState::Unavailable => None,
        FundingState::Disabled => Some(Bps::new(Decimal::ZERO)),
        FundingState::Available => costs.funding_adjustment_bps,
    }
}

fn round_trip_fee_bps(costs: RouteCostInput) -> Result<Option<Bps>, SpreadError> {
    let (Some(long_fee), Some(short_fee)) = (costs.long_taker_fee_bps, costs.short_taker_fee_bps)
    else {
        return Ok(None);
    };
    let entry_two_leg = long_fee
        .value()
        .checked_add(short_fee.value())
        .ok_or(SpreadError::Arithmetic)?;
    let expected_round_trip = entry_two_leg
        .checked_mul(Decimal::from(2_u8))
        .ok_or(SpreadError::Arithmetic)?;
    Ok(Some(Bps::new(expected_round_trip)))
}

fn age_at(
    leg: &'static str,
    receive_ts: UnixNanos,
    observed_at: UnixNanos,
) -> Result<DurationMillis, SpreadError> {
    let nanos = observed_at
        .0
        .checked_sub(receive_ts.0)
        .ok_or(SpreadError::FutureReceiveTimestamp { leg })?;
    Ok(DurationMillis(nanos / NANOS_PER_MILLISECOND))
}

fn ensure_fresh(
    leg: &'static str,
    age: DurationMillis,
    maximum: DurationMillis,
) -> Result<(), SpreadError> {
    if age > maximum {
        return Err(SpreadError::StaleBook {
            leg,
            age_ms: age.0,
            maximum_ms: maximum.0,
        });
    }
    Ok(())
}

fn total_quantity(levels: &[BookLevel]) -> Result<Decimal, SpreadError> {
    levels.iter().try_fold(Decimal::ZERO, |total, level| {
        total
            .checked_add(level.quantity.value())
            .ok_or(SpreadError::Arithmetic)
    })
}

fn walk_vwap(
    levels: &[BookLevel],
    requested: Decimal,
    side: &'static str,
) -> Result<Decimal, SpreadError> {
    let mut remaining = requested;
    let mut quote = Decimal::ZERO;
    let mut available = Decimal::ZERO;
    for level in levels {
        available = available
            .checked_add(level.quantity.value())
            .ok_or(SpreadError::Arithmetic)?;
        let take = remaining.min(level.quantity.value());
        quote = quote
            .checked_add(
                level
                    .price
                    .value()
                    .checked_mul(take)
                    .ok_or(SpreadError::Arithmetic)?,
            )
            .ok_or(SpreadError::Arithmetic)?;
        remaining = remaining.checked_sub(take).ok_or(SpreadError::Arithmetic)?;
        if remaining == Decimal::ZERO {
            return quote.checked_div(requested).ok_or(SpreadError::Arithmetic);
        }
    }
    Err(SpreadError::InsufficientDepth {
        side,
        requested,
        available,
    })
}

fn ratio_bps(numerator: Decimal, denominator: Decimal) -> Result<Decimal, SpreadError> {
    numerator
        .checked_div(denominator)
        .and_then(|ratio| ratio.checked_sub(Decimal::ONE))
        .and_then(|ratio| ratio.checked_mul(BPS_SCALE))
        .ok_or(SpreadError::Arithmetic)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{RouteCostInput, RouteMeasurementInput, SpreadEngine, SpreadError};
    use crate::domain::{
        ids::{InstrumentId, PairId, Symbol, VenueId},
        market::{
            BookLevel, BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook,
        },
        numeric::{BaseQty, Bps, DurationMillis, Price, UnixNanos},
        spread::FundingState,
    };

    fn level(price: i64, quantity: i64) -> Result<BookLevel, Box<dyn Error>> {
        Ok(BookLevel {
            price: Price::new(Decimal::from(price))?,
            quantity: BaseQty::new(Decimal::from(quantity))?,
        })
    }

    fn book(
        venue: &str,
        bids: &[(i64, i64)],
        asks: &[(i64, i64)],
        receive_ms: u64,
    ) -> Result<VenueBook, Box<dyn Error>> {
        Ok(VenueBook {
            venue_id: VenueId::try_from(venue)?,
            instrument_id: InstrumentId::try_from(format!("SNDK-PERP.{venue}"))?,
            bids: bids
                .iter()
                .map(|(price, quantity)| level(*price, *quantity))
                .collect::<Result<_, _>>()?,
            asks: asks
                .iter()
                .map(|(price, quantity)| level(*price, *quantity))
                .collect::<Result<_, _>>()?,
            exchange_ts: UnixNanos(receive_ms * 1_000_000 - 1),
            receive_ts: UnixNanos(receive_ms * 1_000_000),
            age_ms: DurationMillis(0),
            version: BookVersion(1),
        })
    }

    fn health(book: &VenueBook) -> FeedHealth {
        FeedHealth {
            venue_id: book.venue_id.clone(),
            instrument_id: book.instrument_id.clone(),
            connection: FeedConnectionState::Connected,
            freshness: FeedFreshness::Fresh,
            last_transition_ts: UnixNanos(1),
            last_exchange_ts: Some(book.exchange_ts),
            last_receive_ts: Some(book.receive_ts),
            age_ms: Some(DurationMillis(0)),
        }
    }

    fn costs() -> RouteCostInput {
        RouteCostInput {
            long_taker_fee_bps: Some(Bps::new(Decimal::ONE)),
            short_taker_fee_bps: Some(Bps::new(Decimal::new(15, 1))),
            execution_buffer_bps: Bps::new(Decimal::new(2, 0)),
            funding_state: FundingState::Disabled,
            funding_adjustment_bps: None,
        }
    }

    fn measure(
        long: &VenueBook,
        short: &VenueBook,
        quantity: Decimal,
        observed_ms: u64,
        costs: RouteCostInput,
    ) -> Result<crate::domain::spread::ExecutableRouteMeasurement, SpreadError> {
        let pair_id = PairId::try_from("sndk_entropy_lighter").expect("valid pair");
        let symbol = Symbol::try_from("SNDK").expect("valid symbol");
        SpreadEngine::measure(RouteMeasurementInput {
            pair_id: &pair_id,
            symbol: &symbol,
            long_book: long,
            short_book: short,
            long_health: &health(long),
            short_health: &health(short),
            requested_base_quantity: BaseQty::new(quantity).expect("positive quantity"),
            max_book_age_ms: DurationMillis(1_500),
            max_receive_skew_ms: DurationMillis(500),
            observed_at: UnixNanos(observed_ms * 1_000_000),
            costs,
        })
    }

    #[test]
    fn top_level_vwap_and_round_trip_fees_are_exact() -> Result<(), Box<dyn Error>> {
        let long = book("entropy", &[(99, 5)], &[(100, 5)], 1_000)?;
        let short = book("lighter", &[(102, 5)], &[(103, 5)], 1_000)?;
        let result = measure(&long, &short, Decimal::from(2), 1_000, costs())?;
        assert_eq!(result.executable_long_price.value(), Decimal::from(100));
        assert_eq!(result.executable_short_price.value(), Decimal::from(102));
        assert_eq!(
            result.raw_executable_premium_bps.value(),
            Decimal::from(200)
        );
        assert_eq!(result.fee_bps.map(Bps::value), Some(Decimal::from(5)));
        assert_eq!(result.depth_impact_bps.value(), Decimal::ZERO);
        assert_eq!(
            result.funding_adjustment_bps.map(Bps::value),
            Some(Decimal::ZERO)
        );
        Ok(())
    }

    #[test]
    fn multi_level_vwap_embeds_depth_impact() -> Result<(), Box<dyn Error>> {
        let long = book("entropy", &[(99, 3)], &[(100, 1), (102, 2)], 1_000)?;
        let short = book("lighter", &[(105, 1), (103, 2)], &[(106, 3)], 1_000)?;
        let result = measure(&long, &short, Decimal::from(3), 1_000, costs())?;
        assert_eq!(
            result.executable_long_price.value(),
            Decimal::new(304, 0) / Decimal::from(3)
        );
        assert_eq!(
            result.executable_short_price.value(),
            Decimal::new(311, 0) / Decimal::from(3)
        );
        assert!(result.depth_impact_bps.value() > Decimal::ZERO);
        assert_eq!(
            result.maximum_executable_base_quantity.value(),
            Decimal::from(3)
        );
        Ok(())
    }

    #[test]
    fn exact_depth_boundary_is_accepted_and_shortfall_rejected() -> Result<(), Box<dyn Error>> {
        let long = book("entropy", &[(99, 2)], &[(100, 2)], 1_000)?;
        let short = book("lighter", &[(102, 2)], &[(103, 2)], 1_000)?;
        assert!(measure(&long, &short, Decimal::from(2), 1_000, costs()).is_ok());
        assert!(matches!(
            measure(&long, &short, Decimal::new(201, 2), 1_000, costs()),
            Err(SpreadError::InsufficientDepth { .. })
        ));
        Ok(())
    }

    #[test]
    fn stale_skew_and_unhealthy_inputs_fail_closed() -> Result<(), Box<dyn Error>> {
        let long = book("entropy", &[(99, 2)], &[(100, 2)], 1_000)?;
        let short = book("lighter", &[(102, 2)], &[(103, 2)], 1_600)?;
        assert!(matches!(
            measure(&long, &short, Decimal::ONE, 1_600, costs()),
            Err(SpreadError::ReceiveTimeSkew { .. })
        ));
        assert!(matches!(
            measure(&long, &long, Decimal::ONE, 3_000, costs()),
            Err(SpreadError::SameVenue)
        ));

        let pair_id = PairId::try_from("pair")?;
        let symbol = Symbol::try_from("SNDK")?;
        let mut unhealthy = health(&short);
        unhealthy.connection = FeedConnectionState::Disconnected;
        let result = SpreadEngine::measure(RouteMeasurementInput {
            pair_id: &pair_id,
            symbol: &symbol,
            long_book: &long,
            short_book: &short,
            long_health: &health(&long),
            short_health: &unhealthy,
            requested_base_quantity: BaseQty::new(Decimal::ONE)?,
            max_book_age_ms: DurationMillis(2_000),
            max_receive_skew_ms: DurationMillis(1_000),
            observed_at: UnixNanos(1_600_000_000),
            costs: costs(),
        });
        assert!(matches!(result, Err(SpreadError::VenueUnhealthy { .. })));
        Ok(())
    }

    #[test]
    fn reverse_route_is_measured_independently() -> Result<(), Box<dyn Error>> {
        let entropy = book("entropy", &[(99, 5)], &[(100, 5)], 1_000)?;
        let lighter = book("lighter", &[(102, 5)], &[(103, 5)], 1_000)?;
        let forward = measure(&entropy, &lighter, Decimal::ONE, 1_000, costs())?;
        let reverse = measure(&lighter, &entropy, Decimal::ONE, 1_000, costs())?;
        assert_eq!(
            forward.raw_executable_premium_bps.value(),
            Decimal::from(200)
        );
        assert_eq!(
            reverse.raw_executable_premium_bps.value(),
            Decimal::from(99)
                .checked_div(Decimal::from(103))
                .and_then(|value| value.checked_sub(Decimal::ONE))
                .and_then(|value| value.checked_mul(Decimal::from(10_000)))
                .expect("valid arithmetic")
        );
        assert_ne!(
            reverse.raw_executable_premium_bps.value(),
            -forward.raw_executable_premium_bps.value()
        );
        Ok(())
    }

    #[test]
    fn funding_unavailable_stays_none() -> Result<(), Box<dyn Error>> {
        let long = book("entropy", &[(99, 2)], &[(100, 2)], 1_000)?;
        let short = book("lighter", &[(102, 2)], &[(103, 2)], 1_000)?;
        let mut route_costs = costs();
        route_costs.funding_state = FundingState::Unavailable;
        let result = measure(&long, &short, Decimal::ONE, 1_000, route_costs)?;
        assert_eq!(result.funding_adjustment_bps, None);
        Ok(())
    }

    #[test]
    fn corrupt_empty_and_future_books_are_rejected() -> Result<(), Box<dyn Error>> {
        let mut long = book("entropy", &[(99, 2)], &[(100, 2)], 1_000)?;
        let short = book("lighter", &[(102, 2)], &[(103, 2)], 1_000)?;
        long.asks.clear();
        assert!(matches!(
            measure(&long, &short, Decimal::ONE, 1_000, costs()),
            Err(SpreadError::EmptyBookSide { .. })
        ));

        let mut crossed = book("entropy", &[(101, 2)], &[(100, 2)], 1_000)?;
        assert!(matches!(
            measure(&crossed, &short, Decimal::ONE, 1_000, costs()),
            Err(SpreadError::CrossedOrCorruptBook { .. })
        ));

        crossed = book("entropy", &[(99, 2)], &[(100, 2)], 1_001)?;
        assert!(matches!(
            measure(&crossed, &short, Decimal::ONE, 1_000, costs()),
            Err(SpreadError::FutureReceiveTimestamp { .. })
        ));
        Ok(())
    }

    #[test]
    fn missing_fee_is_not_treated_as_zero() -> Result<(), Box<dyn Error>> {
        let long = book("entropy", &[(99, 2)], &[(100, 2)], 1_000)?;
        let short = book("lighter", &[(102, 2)], &[(103, 2)], 1_000)?;
        let mut route_costs = costs();
        route_costs.long_taker_fee_bps = None;
        let result = measure(&long, &short, Decimal::ONE, 1_000, route_costs)?;
        assert_eq!(result.fee_bps, None);
        Ok(())
    }
}
