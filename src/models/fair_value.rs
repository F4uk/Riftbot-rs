//! Deterministic, route-isolated midpoint reference sampling and robust rolling baseline.

use std::collections::{BTreeMap, VecDeque};

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::FairValueConfig,
    domain::{
        ids::{PairId, VenueId},
        market::{FeedHealth, VenueBook},
        numeric::{Bps, DurationMillis, UnixNanos},
    },
};

const BPS_SCALE: Decimal = Decimal::from_parts(10_000, 0, 0, false, 0);
const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Stable identity for one independently sampled route.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrientedRouteKey {
    pub pair_id: PairId,
    pub long_venue: VenueId,
    pub short_venue: VenueId,
}

impl OrientedRouteKey {
    #[must_use]
    pub fn new(pair_id: PairId, long_venue: VenueId, short_venue: VenueId) -> Self {
        Self {
            pair_id,
            long_venue,
            short_venue,
        }
    }
}

/// Why an epoch-aligned reference tick did not contribute a sample.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRejection {
    VenueUnhealthy,
    StaleBook,
    ReceiveTimeSkew,
    EmptyBookSide,
    CrossedOrCorruptBook,
    FutureReceiveTimestamp,
}

/// Deterministic output after one logical sampling tick.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FairValueSnapshot {
    pub route: OrientedRouteKey,
    pub tick_ts: UnixNanos,
    pub reference_basis_bps: Option<Bps>,
    pub midline_bps: Option<Bps>,
    pub dispersion_bps: Option<Bps>,
    pub sample_count: usize,
    pub minimum_samples: usize,
    pub warmed_up: bool,
    pub rejection: Option<ReferenceRejection>,
}

/// Books and health at one canonical logical sampling tick.
#[derive(Clone, Copy, Debug)]
pub struct ReferenceSampleInput<'a> {
    pub route: &'a OrientedRouteKey,
    pub long_book: &'a VenueBook,
    pub short_book: &'a VenueBook,
    pub long_health: &'a FeedHealth,
    pub short_health: &'a FeedHealth,
    pub tick_ts: UnixNanos,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReferenceSample {
    tick_ts: UnixNanos,
    value_bps: Bps,
}

#[derive(Clone, Debug, Default)]
struct RouteWindow {
    last_tick: Option<UnixNanos>,
    samples: VecDeque<ReferenceSample>,
}

/// Invalid sampling schedule or caller ordering.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum FairValueError {
    #[error("fair-value sampling configuration is invalid")]
    InvalidConfiguration,
    #[error("tick {tick:?} is not on the epoch-aligned {interval_ms}ms grid")]
    OffGridTick { tick: UnixNanos, interval_ms: u64 },
    #[error("route sampling tick must increase beyond {previous:?}, received {incoming:?}")]
    NonMonotonicTick {
        previous: UnixNanos,
        incoming: UnixNanos,
    },
    #[error("route book identity does not match its explicit orientation")]
    RouteIdentityMismatch,
    #[error("fixed-decimal fair-value arithmetic failed")]
    Arithmetic,
}

/// Route-isolated time-weighted robust baseline. It owns no wall clock.
#[derive(Clone, Debug)]
pub struct FairValueModel {
    sample_interval_ms: DurationMillis,
    window_duration_ms: DurationMillis,
    minimum_samples: usize,
    max_sample_age_ms: DurationMillis,
    max_receive_skew_ms: DurationMillis,
    windows: BTreeMap<OrientedRouteKey, RouteWindow>,
}

impl FairValueModel {
    pub fn new(
        config: &FairValueConfig,
        max_receive_skew_ms: DurationMillis,
    ) -> Result<Self, FairValueError> {
        let capacity = config.window_duration_ms.0 / config.sample_interval_ms.0.max(1);
        if config.sample_interval_ms.0 == 0
            || config.window_duration_ms.0 == 0
            || config.max_sample_age_ms.0 == 0
            || max_receive_skew_ms.0 == 0
            || config.minimum_samples == 0
            || u64::try_from(config.minimum_samples).unwrap_or(u64::MAX) > capacity
            || config
                .sample_interval_ms
                .0
                .checked_mul(NANOS_PER_MILLISECOND)
                .is_none()
            || config
                .window_duration_ms
                .0
                .checked_mul(NANOS_PER_MILLISECOND)
                .is_none()
        {
            return Err(FairValueError::InvalidConfiguration);
        }
        Ok(Self {
            sample_interval_ms: config.sample_interval_ms,
            window_duration_ms: config.window_duration_ms,
            minimum_samples: config.minimum_samples,
            max_sample_age_ms: config.max_sample_age_ms,
            max_receive_skew_ms,
            windows: BTreeMap::new(),
        })
    }

    /// Samples at most once for a route/tick and returns the post-eviction window state.
    pub fn sample(
        &mut self,
        input: ReferenceSampleInput<'_>,
    ) -> Result<FairValueSnapshot, FairValueError> {
        let interval_nanos = self
            .sample_interval_ms
            .0
            .checked_mul(NANOS_PER_MILLISECOND)
            .ok_or(FairValueError::InvalidConfiguration)?;
        if !input.tick_ts.0.is_multiple_of(interval_nanos) {
            return Err(FairValueError::OffGridTick {
                tick: input.tick_ts,
                interval_ms: self.sample_interval_ms.0,
            });
        }
        if input.long_book.venue_id != input.route.long_venue
            || input.short_book.venue_id != input.route.short_venue
            || input.long_book.venue_id == input.short_book.venue_id
            || input.long_health.venue_id != input.route.long_venue
            || input.short_health.venue_id != input.route.short_venue
            || input.long_health.instrument_id != input.long_book.instrument_id
            || input.short_health.instrument_id != input.short_book.instrument_id
        {
            return Err(FairValueError::RouteIdentityMismatch);
        }

        let route_window = self.windows.entry(input.route.clone()).or_default();
        if let Some(previous) = route_window.last_tick
            && input.tick_ts <= previous
        {
            return Err(FairValueError::NonMonotonicTick {
                previous,
                incoming: input.tick_ts,
            });
        }
        route_window.last_tick = Some(input.tick_ts);
        evict_old(route_window, input.tick_ts, self.window_duration_ms);

        let reference = reference_basis(input, self.max_sample_age_ms, self.max_receive_skew_ms);
        let (reference_basis_bps, rejection) = match reference {
            Ok(value) => {
                route_window.samples.push_back(ReferenceSample {
                    tick_ts: input.tick_ts,
                    value_bps: value,
                });
                (Some(value), None)
            }
            Err(reason) => (None, Some(reason)),
        };
        let warmed_up = route_window.samples.len() >= self.minimum_samples;
        let (midline_bps, dispersion_bps) = if warmed_up {
            let values: Vec<_> = route_window
                .samples
                .iter()
                .map(|sample| sample.value_bps.value())
                .collect();
            let median_value = median(values.clone())?;
            let deviations = values
                .into_iter()
                .map(|value| (value - median_value).abs())
                .collect();
            (
                Some(Bps::new(median_value)),
                Some(Bps::new(median(deviations)?)),
            )
        } else {
            (None, None)
        };
        Ok(FairValueSnapshot {
            route: input.route.clone(),
            tick_ts: input.tick_ts,
            reference_basis_bps,
            midline_bps,
            dispersion_bps,
            sample_count: route_window.samples.len(),
            minimum_samples: self.minimum_samples,
            warmed_up,
            rejection,
        })
    }
}

fn evict_old(window: &mut RouteWindow, now: UnixNanos, duration: DurationMillis) {
    let duration_nanos = duration.0.saturating_mul(NANOS_PER_MILLISECOND);
    let cutoff = now.0.saturating_sub(duration_nanos);
    while window
        .samples
        .front()
        .is_some_and(|sample| sample.tick_ts.0 <= cutoff)
    {
        window.samples.pop_front();
    }
}

fn reference_basis(
    input: ReferenceSampleInput<'_>,
    max_age: DurationMillis,
    max_skew: DurationMillis,
) -> Result<Bps, ReferenceRejection> {
    if !input.long_health.is_healthy() || !input.short_health.is_healthy() {
        return Err(ReferenceRejection::VenueUnhealthy);
    }
    if input.long_book.bids.is_empty()
        || input.long_book.asks.is_empty()
        || input.short_book.bids.is_empty()
        || input.short_book.asks.is_empty()
    {
        return Err(ReferenceRejection::EmptyBookSide);
    }
    if !canonical(input.long_book) || !canonical(input.short_book) {
        return Err(ReferenceRejection::CrossedOrCorruptBook);
    }
    let long_age = age_at(input.long_book.receive_ts, input.tick_ts)?;
    let short_age = age_at(input.short_book.receive_ts, input.tick_ts)?;
    if long_age > max_age || short_age > max_age {
        return Err(ReferenceRejection::StaleBook);
    }
    let skew = DurationMillis(
        input
            .long_book
            .receive_ts
            .0
            .abs_diff(input.short_book.receive_ts.0)
            / NANOS_PER_MILLISECOND,
    );
    if skew > max_skew {
        return Err(ReferenceRejection::ReceiveTimeSkew);
    }

    let mid_long = midpoint(input.long_book)?;
    let mid_short = midpoint(input.short_book)?;
    let basis = mid_short
        .checked_div(mid_long)
        .and_then(|ratio| ratio.checked_sub(Decimal::ONE))
        .and_then(|ratio| ratio.checked_mul(BPS_SCALE))
        .ok_or(ReferenceRejection::CrossedOrCorruptBook)?;
    Ok(Bps::new(basis))
}

fn midpoint(book: &VenueBook) -> Result<Decimal, ReferenceRejection> {
    book.bids[0]
        .price
        .value()
        .checked_add(book.asks[0].price.value())
        .and_then(|sum| sum.checked_div(Decimal::from(2_u8)))
        .ok_or(ReferenceRejection::CrossedOrCorruptBook)
}

fn canonical(book: &VenueBook) -> bool {
    book.bids[0].price.value() < book.asks[0].price.value()
        && book
            .bids
            .windows(2)
            .all(|window| window[0].price.value() > window[1].price.value())
        && book
            .asks
            .windows(2)
            .all(|window| window[0].price.value() < window[1].price.value())
}

fn age_at(receive_ts: UnixNanos, tick_ts: UnixNanos) -> Result<DurationMillis, ReferenceRejection> {
    tick_ts
        .0
        .checked_sub(receive_ts.0)
        .map(|nanos| DurationMillis(nanos / NANOS_PER_MILLISECOND))
        .ok_or(ReferenceRejection::FutureReceiveTimestamp)
}

fn median(mut values: Vec<Decimal>) -> Result<Decimal, FairValueError> {
    if values.is_empty() {
        return Err(FairValueError::Arithmetic);
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        Ok(values[middle])
    } else {
        values[middle - 1]
            .checked_add(values[middle])
            .and_then(|sum| sum.checked_div(Decimal::from(2_u8)))
            .ok_or(FairValueError::Arithmetic)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rust_decimal::Decimal;

    use super::{
        FairValueError, FairValueModel, OrientedRouteKey, ReferenceRejection, ReferenceSampleInput,
    };
    use crate::{
        config::FairValueConfig,
        domain::{
            ids::{InstrumentId, PairId, VenueId},
            market::{
                BookLevel, BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook,
            },
            numeric::{BaseQty, DurationMillis, Price, UnixNanos},
        },
    };

    fn config(minimum_samples: usize) -> FairValueConfig {
        FairValueConfig {
            sample_interval_ms: DurationMillis(1_000),
            window_duration_ms: DurationMillis(4_000),
            minimum_samples,
            max_sample_age_ms: DurationMillis(1_500),
        }
    }

    fn book(
        venue: &str,
        bid: Decimal,
        ask: Decimal,
        receive_ms: u64,
    ) -> Result<VenueBook, Box<dyn Error>> {
        Ok(VenueBook {
            venue_id: VenueId::try_from(venue)?,
            instrument_id: InstrumentId::try_from(format!("SNDK-PERP.{venue}"))?,
            bids: vec![BookLevel {
                price: Price::new(bid)?,
                quantity: BaseQty::new(Decimal::ONE)?,
            }],
            asks: vec![BookLevel {
                price: Price::new(ask)?,
                quantity: BaseQty::new(Decimal::ONE)?,
            }],
            exchange_ts: UnixNanos(receive_ms * 1_000_000 - 1),
            receive_ts: UnixNanos(receive_ms * 1_000_000),
            age_ms: DurationMillis(0),
            version: BookVersion(receive_ms),
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

    fn route(long: &str, short: &str) -> Result<OrientedRouteKey, Box<dyn Error>> {
        Ok(OrientedRouteKey::new(
            PairId::try_from("sndk_pair")?,
            VenueId::try_from(long)?,
            VenueId::try_from(short)?,
        ))
    }

    fn sample(
        model: &mut FairValueModel,
        route: &OrientedRouteKey,
        long: &VenueBook,
        short: &VenueBook,
        tick_ms: u64,
    ) -> Result<super::FairValueSnapshot, FairValueError> {
        model.sample(ReferenceSampleInput {
            route,
            long_book: long,
            short_book: short,
            long_health: &health(long),
            short_health: &health(short),
            tick_ts: UnixNanos(tick_ms * 1_000_000),
        })
    }

    #[test]
    fn midpoint_reference_warms_up_to_robust_median() -> Result<(), Box<dyn Error>> {
        let mut model = FairValueModel::new(&config(3), DurationMillis(500))?;
        let route = route("entropy", "lighter")?;
        for (tick, short_mid) in [(1_000, 101), (2_000, 102), (3_000, 200)] {
            let long = book("entropy", Decimal::new(995, 1), Decimal::new(1005, 1), tick)?;
            let short = book(
                "lighter",
                Decimal::from(short_mid) - Decimal::new(5, 1),
                Decimal::from(short_mid) + Decimal::new(5, 1),
                tick,
            )?;
            let result = sample(&mut model, &route, &long, &short, tick)?;
            if tick < 3_000 {
                assert!(!result.warmed_up);
                assert_eq!(result.midline_bps, None);
            } else {
                assert!(result.warmed_up);
                assert_eq!(
                    result.midline_bps.map(crate::domain::numeric::Bps::value),
                    Some(Decimal::from(200))
                );
            }
        }
        Ok(())
    }

    #[test]
    fn update_frequency_cannot_add_samples() -> Result<(), Box<dyn Error>> {
        let mut model = FairValueModel::new(&config(2), DurationMillis(500))?;
        let route = route("entropy", "lighter")?;
        let long = book("entropy", Decimal::from(99), Decimal::from(101), 1_000)?;
        let short = book("lighter", Decimal::from(100), Decimal::from(102), 1_000)?;
        let first = sample(&mut model, &route, &long, &short, 1_000)?;
        assert_eq!(first.sample_count, 1);
        assert!(matches!(
            sample(&mut model, &route, &long, &short, 1_000),
            Err(FairValueError::NonMonotonicTick { .. })
        ));
        assert!(matches!(
            sample(&mut model, &route, &long, &short, 1_500),
            Err(FairValueError::OffGridTick { .. })
        ));
        Ok(())
    }

    #[test]
    fn reverse_route_has_an_independent_non_negated_basis() -> Result<(), Box<dyn Error>> {
        let mut model = FairValueModel::new(&config(1), DurationMillis(500))?;
        let entropy = book("entropy", Decimal::from(99), Decimal::from(101), 1_000)?;
        let lighter = book("lighter", Decimal::from(101), Decimal::from(103), 1_000)?;
        let forward_route = route("entropy", "lighter")?;
        let reverse_route = route("lighter", "entropy")?;
        let forward = sample(&mut model, &forward_route, &entropy, &lighter, 1_000)?;
        let reverse = sample(&mut model, &reverse_route, &lighter, &entropy, 1_000)?;
        let forward_value = forward.reference_basis_bps.expect("sample").value();
        let reverse_value = reverse.reference_basis_bps.expect("sample").value();
        assert_ne!(reverse_value, -forward_value);
        assert!(forward_value > Decimal::ZERO);
        assert!(reverse_value < Decimal::ZERO);
        Ok(())
    }

    #[test]
    fn stale_tick_is_rejected_without_backfill() -> Result<(), Box<dyn Error>> {
        let mut model = FairValueModel::new(&config(1), DurationMillis(500))?;
        let route = route("entropy", "lighter")?;
        let long = book("entropy", Decimal::from(99), Decimal::from(101), 1_000)?;
        let short = book("lighter", Decimal::from(101), Decimal::from(103), 1_000)?;
        let rejected = sample(&mut model, &route, &long, &short, 3_000)?;
        assert_eq!(rejected.sample_count, 0);
        assert_eq!(rejected.rejection, Some(ReferenceRejection::StaleBook));
        assert!(!rejected.warmed_up);
        Ok(())
    }

    #[test]
    fn duration_window_evicts_deterministically() -> Result<(), Box<dyn Error>> {
        let mut model = FairValueModel::new(&config(1), DurationMillis(500))?;
        let route = route("entropy", "lighter")?;
        for tick in [1_000, 2_000, 3_000, 4_000, 5_000] {
            let long = book("entropy", Decimal::from(99), Decimal::from(101), tick)?;
            let short = book("lighter", Decimal::from(100), Decimal::from(102), tick)?;
            let result = sample(&mut model, &route, &long, &short, tick)?;
            if tick == 5_000 {
                assert_eq!(result.sample_count, 4);
            }
        }
        Ok(())
    }
}
