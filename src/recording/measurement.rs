//! Offline-only P3 measurement replay over a fully validated P2 replay report.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::{
        AppConfig, ConfigError, FundingStateConfig, MeasurementConfigFingerprint,
        measurement_config_fingerprint,
    },
    domain::{
        ids::{InstrumentId, Symbol, VenueId},
        market::{FeedHealth, VenueBook},
        numeric::{DurationMillis, UnixNanos},
        spread::{FundingState, MeasurementValidity},
    },
    market::book_store::FeedKey,
    models::{
        P3_MODEL_VERSION,
        fair_value::{
            FairValueError, FairValueModel, FairValueSnapshot, OrientedRouteKey,
            ReferenceSampleInput,
        },
        opportunity::{
            OpportunityError, OpportunityEvaluation, OpportunityInput, OpportunityModel,
        },
        spread_engine::{RouteCostInput, RouteMeasurementInput, SpreadEngine},
    },
};

use super::replay::{ReplayReport, ReplaySafety, ReplayStep};

const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Compile-time capability marker. This API accepts normalized replay state only.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementReplaySafety {
    OfflineMeasurementOnly,
}

/// An unhealthy transport state retained as deterministic evidence even if no sample tick lands in
/// the transition interval.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedStateRejection {
    pub sequence: u64,
    pub event_time: UnixNanos,
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub connection: crate::domain::market::FeedConnectionState,
    pub freshness: crate::domain::market::FeedFreshness,
    pub reason: String,
}

/// Result for one route at one canonical sample tick.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MeasurementTickOutcome {
    Evaluated {
        fair_value: FairValueSnapshot,
        evaluation: Box<OpportunityEvaluation>,
    },
    Rejected {
        fair_value: Option<FairValueSnapshot>,
        validity: MeasurementValidity,
        reason: String,
    },
}

/// One deterministic route observation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementTick {
    pub tick_ts: UnixNanos,
    pub route: OrientedRouteKey,
    pub outcome: MeasurementTickOutcome,
}

/// Machine-readable P3 evidence derived without an execution capability or wall clock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementReplayReport {
    pub source_recording_schema_version: u16,
    pub source_content_sha256: String,
    pub source_replay_end_ts: UnixNanos,
    pub safety: MeasurementReplaySafety,
    pub model_version: String,
    pub config_fingerprint: String,
    pub sample_interval_ms: DurationMillis,
    pub routes: Vec<OrientedRouteKey>,
    pub ticks: Vec<MeasurementTick>,
    pub feed_state_rejections: Vec<FeedStateRejection>,
}

#[derive(Clone, Debug)]
struct RouteDescriptor {
    key: OrientedRouteKey,
    symbol: Symbol,
    long_feed: FeedKey,
    short_feed: FeedKey,
    costs: RouteCostInput,
}

/// Invalid P3 replay configuration or deterministic model failure.
#[derive(Debug, Error)]
pub enum MeasurementReplayError {
    #[error("configuration validation failed: {0}")]
    Config(#[from] ConfigError),
    #[error("measurement config fingerprint failed: {0}")]
    Fingerprint(#[from] serde_json::Error),
    #[error("P3 replay requires one selected V1 pair")]
    MissingPair,
    #[error("selected pair venue or instrument mapping is incomplete")]
    IncompletePairMapping,
    #[error("funding route is missing for {long_venue} -> {short_venue}")]
    MissingFundingRoute {
        long_venue: VenueId,
        short_venue: VenueId,
    },
    #[error("P2 replay report is not offline market-data-only")]
    UnsafeSourceReplay,
    #[error("P2 replay report contains no events")]
    EmptySourceReplay,
    #[error("logical sampling interval overflow")]
    SamplingIntervalOverflow,
    #[error("fair-value model failed: {0}")]
    FairValue(#[from] FairValueError),
    #[error("opportunity model failed: {0}")]
    Opportunity(#[from] OpportunityError),
}

/// Rebuilds P3 measurement state from a P2 report. Every `analyze` call starts from empty state.
#[derive(Clone, Debug)]
pub struct MeasurementReplayEngine {
    config: AppConfig,
    fingerprint: MeasurementConfigFingerprint,
    routes: Vec<RouteDescriptor>,
}

impl MeasurementReplayEngine {
    pub fn new(config: AppConfig) -> Result<Self, MeasurementReplayError> {
        config.validate()?;
        let fingerprint = measurement_config_fingerprint(&config)?;
        let routes = route_descriptors(&config)?;
        Ok(Self {
            config,
            fingerprint,
            routes,
        })
    }

    #[must_use]
    pub const fn safety(&self) -> MeasurementReplaySafety {
        MeasurementReplaySafety::OfflineMeasurementOnly
    }

    pub fn analyze(
        &self,
        source: &ReplayReport,
    ) -> Result<MeasurementReplayReport, MeasurementReplayError> {
        if source.safety != ReplaySafety::OfflineMarketDataOnly {
            return Err(MeasurementReplayError::UnsafeSourceReplay);
        }
        if source.steps.is_empty() {
            return Err(MeasurementReplayError::EmptySourceReplay);
        }
        let interval_nanos = self
            .config
            .fair_value
            .sample_interval_ms
            .0
            .checked_mul(NANOS_PER_MILLISECOND)
            .ok_or(MeasurementReplayError::SamplingIntervalOverflow)?;
        let mut ordered_steps: Vec<&ReplayStep> = source.steps.iter().collect();
        ordered_steps.sort_by_key(|step| (step.event_time, step.sequence));
        let first_event = ordered_steps[0].event_time.0;
        let first_tick = first_event
            .div_ceil(interval_nanos)
            .checked_mul(interval_nanos)
            .ok_or(MeasurementReplayError::SamplingIntervalOverflow)?;
        let last_tick = source.replay_end_ts.0 / interval_nanos * interval_nanos;

        let mut fair_value = FairValueModel::new(
            &self.config.fair_value,
            self.config.market_data.max_receive_skew_ms,
        )?;
        let opportunity = OpportunityModel::new(&self.config.regime, &self.fingerprint)?;
        let mut books: BTreeMap<FeedKey, VenueBook> = BTreeMap::new();
        let mut health: BTreeMap<FeedKey, FeedHealth> = BTreeMap::new();
        let feed_state_rejections = collect_feed_state_rejections(&source.steps);
        let mut ticks = Vec::new();
        let mut step_index = 0_usize;
        let mut tick = first_tick;
        while tick <= last_tick {
            while step_index < ordered_steps.len() && ordered_steps[step_index].event_time.0 <= tick
            {
                apply_step(ordered_steps[step_index], &mut books, &mut health);
                step_index += 1;
            }
            let tick_ts = UnixNanos(tick);
            for route in &self.routes {
                ticks.push(measure_tick(
                    route,
                    tick_ts,
                    &books,
                    &health,
                    &mut fair_value,
                    &opportunity,
                    &self.config,
                )?);
            }
            let Some(next) = tick.checked_add(interval_nanos) else {
                break;
            };
            tick = next;
        }

        Ok(MeasurementReplayReport {
            source_recording_schema_version: source.schema_version,
            source_content_sha256: source.content_sha256.clone(),
            source_replay_end_ts: source.replay_end_ts,
            safety: self.safety(),
            model_version: P3_MODEL_VERSION.to_owned(),
            config_fingerprint: self.fingerprint.to_string(),
            sample_interval_ms: self.config.fair_value.sample_interval_ms,
            routes: self.routes.iter().map(|route| route.key.clone()).collect(),
            ticks,
            feed_state_rejections,
        })
    }
}

fn route_descriptors(config: &AppConfig) -> Result<Vec<RouteDescriptor>, MeasurementReplayError> {
    let pair = config
        .pair
        .as_ref()
        .ok_or(MeasurementReplayError::MissingPair)?;
    let orientations = [(0_usize, 1_usize), (1_usize, 0_usize)];
    orientations
        .into_iter()
        .map(|(long, short)| {
            let long_venue = pair.venues[long].clone();
            let short_venue = pair.venues[short].clone();
            let long_fee = config
                .venues
                .iter()
                .find(|venue| venue.id == long_venue)
                .ok_or(MeasurementReplayError::IncompletePairMapping)?
                .taker_fee_bps;
            let short_fee = config
                .venues
                .iter()
                .find(|venue| venue.id == short_venue)
                .ok_or(MeasurementReplayError::IncompletePairMapping)?
                .taker_fee_bps;
            let funding = config
                .funding
                .routes
                .iter()
                .find(|route| route.long_venue == long_venue && route.short_venue == short_venue)
                .ok_or_else(|| MeasurementReplayError::MissingFundingRoute {
                    long_venue: long_venue.clone(),
                    short_venue: short_venue.clone(),
                })?;
            Ok(RouteDescriptor {
                key: OrientedRouteKey::new(
                    pair.id.clone(),
                    long_venue.clone(),
                    short_venue.clone(),
                ),
                symbol: pair.symbol.clone(),
                long_feed: FeedKey::new(long_venue, pair.instruments[long].clone()),
                short_feed: FeedKey::new(short_venue, pair.instruments[short].clone()),
                costs: RouteCostInput {
                    long_taker_fee_bps: long_fee,
                    short_taker_fee_bps: short_fee,
                    execution_buffer_bps: config.market_data.execution_buffer_bps,
                    funding_state: match funding.state {
                        FundingStateConfig::Unavailable => FundingState::Unavailable,
                        FundingStateConfig::Disabled => FundingState::Disabled,
                        FundingStateConfig::Available => FundingState::Available,
                    },
                    funding_adjustment_bps: funding.adjustment_bps,
                },
            })
        })
        .collect()
}

fn apply_step(
    step: &ReplayStep,
    books: &mut BTreeMap<FeedKey, VenueBook>,
    health: &mut BTreeMap<FeedKey, FeedHealth>,
) {
    if let Some(book) = &step.normalized_book {
        books.insert(
            FeedKey::new(book.venue_id.clone(), book.instrument_id.clone()),
            book.clone(),
        );
    }
    if let Some(feed_health) = &step.resulting_health {
        health.insert(
            FeedKey::new(
                feed_health.venue_id.clone(),
                feed_health.instrument_id.clone(),
            ),
            feed_health.clone(),
        );
    }
}

fn measure_tick(
    route: &RouteDescriptor,
    tick_ts: UnixNanos,
    books: &BTreeMap<FeedKey, VenueBook>,
    health: &BTreeMap<FeedKey, FeedHealth>,
    fair_value: &mut FairValueModel,
    opportunity: &OpportunityModel,
    config: &AppConfig,
) -> Result<MeasurementTick, MeasurementReplayError> {
    let Some(long_book) = books.get(&route.long_feed) else {
        return Ok(missing_tick(route, tick_ts, "long_book_missing"));
    };
    let Some(short_book) = books.get(&route.short_feed) else {
        return Ok(missing_tick(route, tick_ts, "short_book_missing"));
    };
    let Some(long_health) = health.get(&route.long_feed) else {
        return Ok(missing_tick(route, tick_ts, "long_health_missing"));
    };
    let Some(short_health) = health.get(&route.short_feed) else {
        return Ok(missing_tick(route, tick_ts, "short_health_missing"));
    };

    let fair_snapshot = fair_value.sample(ReferenceSampleInput {
        route: &route.key,
        long_book,
        short_book,
        long_health,
        short_health,
        tick_ts,
    })?;
    let executable = SpreadEngine::measure(RouteMeasurementInput {
        pair_id: &route.key.pair_id,
        symbol: &route.symbol,
        long_book,
        short_book,
        long_health,
        short_health,
        requested_base_quantity: config.market_data.requested_base_quantity,
        max_book_age_ms: config.market_data.stale_after_ms,
        max_receive_skew_ms: config.market_data.max_receive_skew_ms,
        observed_at: tick_ts,
        costs: route.costs,
    });
    let outcome = match executable {
        Ok(executable) => MeasurementTickOutcome::Evaluated {
            fair_value: fair_snapshot.clone(),
            evaluation: Box::new(opportunity.evaluate(OpportunityInput {
                executable,
                fair_value: fair_snapshot,
                other_explicit_risk_costs_bps: Vec::new(),
            })?),
        },
        Err(error) => MeasurementTickOutcome::Rejected {
            fair_value: Some(fair_snapshot),
            validity: error.validity(),
            reason: error.to_string(),
        },
    };
    Ok(MeasurementTick {
        tick_ts,
        route: route.key.clone(),
        outcome,
    })
}

fn missing_tick(route: &RouteDescriptor, tick_ts: UnixNanos, reason: &str) -> MeasurementTick {
    MeasurementTick {
        tick_ts,
        route: route.key.clone(),
        outcome: MeasurementTickOutcome::Rejected {
            fair_value: None,
            validity: MeasurementValidity::VenueUnhealthy,
            reason: reason.to_owned(),
        },
    }
}

fn collect_feed_state_rejections(steps: &[ReplayStep]) -> Vec<FeedStateRejection> {
    steps
        .iter()
        .filter_map(|step| {
            let health = step.resulting_health.as_ref()?;
            (!health.is_healthy()).then(|| FeedStateRejection {
                sequence: step.sequence,
                event_time: step.event_time,
                venue_id: health.venue_id.clone(),
                instrument_id: health.instrument_id.clone(),
                connection: health.connection,
                freshness: health.freshness,
                reason: format!(
                    "transport_{:?}_freshness_{:?}",
                    health.connection, health.freshness
                )
                .to_lowercase(),
            })
        })
        .collect()
}
