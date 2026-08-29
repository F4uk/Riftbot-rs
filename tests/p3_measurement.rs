use std::{
    error::Error,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use riftbot::{
    config::{FundingStateConfig, parse_toml},
    domain::{
        ids::{InstrumentId, VenueId},
        market::{
            BookLevel, BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook,
        },
        numeric::{BaseQty, Bps, DurationMillis, Price, UnixNanos},
        spread::MeasurementValidity,
    },
    market::book_store::FeedKey,
    recording::{
        measurement::{MeasurementReplayEngine, MeasurementReplaySafety, MeasurementTickOutcome},
        recorder::BufferedRecorder,
        replay::ReplayEngine,
        schema::RecordedEvent,
    },
};
use rust_decimal::Decimal;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const EXAMPLE: &str = include_str!("../config/example.toml");

struct TestRecording(PathBuf);

impl TestRecording {
    fn new(label: &str) -> Self {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "riftbot-p3-{label}-{}-{sequence}.jsonl",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for TestRecording {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn key(venue: &str) -> Result<FeedKey, Box<dyn Error>> {
    let instrument = match venue {
        "entropy" => "io:SNDK-USD-PERP.HYPERLIQUID",
        "lighter" => "SNDK-PERP.LIGHTER",
        _ => return Err(format!("unsupported fixture venue {venue}").into()),
    };
    Ok(FeedKey::new(
        VenueId::try_from(venue)?,
        InstrumentId::try_from(instrument)?,
    ))
}

fn book(
    venue: &str,
    version: u64,
    bid: Decimal,
    ask: Decimal,
    receive_ts: u64,
) -> Result<VenueBook, Box<dyn Error>> {
    let key = key(venue)?;
    Ok(VenueBook {
        venue_id: key.venue_id,
        instrument_id: key.instrument_id,
        bids: vec![BookLevel {
            price: Price::new(bid)?,
            quantity: BaseQty::new(Decimal::ONE)?,
        }],
        asks: vec![BookLevel {
            price: Price::new(ask)?,
            quantity: BaseQty::new(Decimal::ONE)?,
        }],
        exchange_ts: UnixNanos(receive_ts - 1_000_000),
        receive_ts: UnixNanos(receive_ts),
        age_ms: DurationMillis(0),
        version: BookVersion(version),
    })
}

fn health_assertion(
    transition_ts: u64,
    book: &VenueBook,
    observed_at: u64,
) -> Result<RecordedEvent, Box<dyn Error>> {
    Ok(RecordedEvent::feed_health(
        UnixNanos(observed_at),
        FeedHealth {
            venue_id: book.venue_id.clone(),
            instrument_id: book.instrument_id.clone(),
            connection: FeedConnectionState::Connected,
            freshness: FeedFreshness::Fresh,
            last_transition_ts: UnixNanos(transition_ts),
            last_exchange_ts: Some(book.exchange_ts),
            last_receive_ts: Some(book.receive_ts),
            age_ms: Some(DurationMillis(
                (observed_at - book.receive_ts.0) / 1_000_000,
            )),
        },
    )?)
}

fn write_events(recording: &TestRecording, events: &[RecordedEvent]) -> Result<(), Box<dyn Error>> {
    let recorder = BufferedRecorder::create(&recording.0, events.len() + 1)?;
    for event in events {
        recorder.try_record(event.clone())?;
    }
    recorder.shutdown()?;
    Ok(())
}

fn healthy_two_tick_events() -> Result<Vec<RecordedEvent>, Box<dyn Error>> {
    let entropy_key = key("entropy")?;
    let lighter_key = key("lighter")?;
    let entropy_first = book(
        "entropy",
        1,
        Decimal::from(99),
        Decimal::from(101),
        900_000_000,
    )?;
    let lighter_first = book(
        "lighter",
        1,
        Decimal::from(101),
        Decimal::from(103),
        950_000_000,
    )?;
    let entropy_second = book(
        "entropy",
        2,
        Decimal::from(100),
        Decimal::from(102),
        1_900_000_000,
    )?;
    let lighter_second = book(
        "lighter",
        2,
        Decimal::from(102),
        Decimal::from(104),
        1_950_000_000,
    )?;
    Ok(vec![
        RecordedEvent::feed_connection(
            &entropy_key,
            FeedConnectionState::Connected,
            UnixNanos(100_000_000),
        )?,
        RecordedEvent::feed_connection(
            &lighter_key,
            FeedConnectionState::Connected,
            UnixNanos(110_000_000),
        )?,
        RecordedEvent::market_book(&entropy_first)?,
        RecordedEvent::market_book(&lighter_first)?,
        RecordedEvent::market_book(&entropy_second)?,
        RecordedEvent::market_book(&lighter_second)?,
        health_assertion(100_000_000, &entropy_second, 2_000_000_000)?,
        health_assertion(110_000_000, &lighter_second, 2_000_000_000)?,
    ])
}

fn measurement_config(
    funding: FundingStateConfig,
) -> Result<riftbot::config::AppConfig, Box<dyn Error>> {
    let mut config = parse_toml(EXAMPLE)?;
    config.fair_value.minimum_samples = 2;
    config.fair_value.window_duration_ms = DurationMillis(4_000);
    for venue in &mut config.venues {
        if matches!(venue.id.as_str(), "entropy" | "lighter") {
            venue.taker_fee_bps = Some(Bps::new(Decimal::ONE));
        }
    }
    for route in &mut config.funding.routes {
        route.state = funding;
        route.adjustment_bps = match funding {
            FundingStateConfig::Available => Some(Bps::new(Decimal::ZERO)),
            FundingStateConfig::Unavailable | FundingStateConfig::Disabled => None,
        };
    }
    config.validate()?;
    Ok(config)
}

fn replay_measurement(
    recording: &TestRecording,
    config: riftbot::config::AppConfig,
) -> Result<riftbot::recording::measurement::MeasurementReplayReport, Box<dyn Error>> {
    let replay = ReplayEngine::new(config.market_data.stale_after_ms)?.replay_file(&recording.0)?;
    Ok(MeasurementReplayEngine::new(config)?.analyze(&replay)?)
}

#[test]
fn identical_measurement_replay_twice_is_identical() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("deterministic");
    write_events(&recording, &healthy_two_tick_events()?)?;
    let config = measurement_config(FundingStateConfig::Disabled)?;
    let replay = ReplayEngine::new(config.market_data.stale_after_ms)?.replay_file(&recording.0)?;
    let engine = MeasurementReplayEngine::new(config)?;
    assert_eq!(engine.analyze(&replay)?, engine.analyze(&replay)?);
    assert_eq!(
        engine.safety(),
        MeasurementReplaySafety::OfflineMeasurementOnly
    );
    Ok(())
}

#[test]
fn both_oriented_routes_have_independent_reference_and_executable_math()
-> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("routes");
    write_events(&recording, &healthy_two_tick_events()?)?;
    let report = replay_measurement(
        &recording,
        measurement_config(FundingStateConfig::Disabled)?,
    )?;
    assert_eq!(report.routes.len(), 2);
    let tick_two: Vec<_> = report
        .ticks
        .iter()
        .filter(|tick| tick.tick_ts == UnixNanos(2_000_000_000))
        .collect();
    assert_eq!(tick_two.len(), 2);
    let facts: Vec<_> = tick_two
        .into_iter()
        .map(|tick| match &tick.outcome {
            MeasurementTickOutcome::Evaluated {
                fair_value,
                evaluation,
            } => Ok((
                fair_value.reference_basis_bps.expect("sample").value(),
                evaluation.opportunity.raw_executable_premium_bps.value(),
            )),
            MeasurementTickOutcome::Rejected { reason, .. } => {
                Err(format!("unexpected rejection: {reason}"))
            }
        })
        .collect::<Result<_, _>>()?;
    assert_ne!(facts[1].0, -facts[0].0);
    assert_ne!(facts[1].1, -facts[0].1);
    Ok(())
}

#[test]
fn funding_unavailable_is_visible_and_fails_closed() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("funding");
    write_events(&recording, &healthy_two_tick_events()?)?;
    let report = replay_measurement(
        &recording,
        measurement_config(FundingStateConfig::Unavailable)?,
    )?;
    let final_ticks: Vec<_> = report
        .ticks
        .iter()
        .filter(|tick| tick.tick_ts == UnixNanos(2_000_000_000))
        .collect();
    assert_eq!(final_ticks.len(), 2);
    for tick in final_ticks {
        let MeasurementTickOutcome::Evaluated { evaluation, .. } = &tick.outcome else {
            return Err("expected evaluated measurement facts".into());
        };
        assert_eq!(
            evaluation.opportunity.validity,
            MeasurementValidity::FundingUnavailable
        );
        assert_eq!(evaluation.opportunity.tradable_edge_bps, None);
        assert!(!evaluation.opportunity.increase_risk_economically_allowed);
    }
    Ok(())
}

#[test]
fn changed_measurement_config_changes_fingerprint_and_output() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("fingerprint");
    write_events(&recording, &healthy_two_tick_events()?)?;
    let base = measurement_config(FundingStateConfig::Disabled)?;
    let first = replay_measurement(&recording, base.clone())?;
    let mut changed = base;
    changed.market_data.execution_buffer_bps = Bps::new(Decimal::from(7));
    let second = replay_measurement(&recording, changed)?;
    assert_ne!(first.config_fingerprint, second.config_fingerprint);
    assert_ne!(first.ticks, second.ticks);
    Ok(())
}

#[test]
fn replay_ticks_come_only_from_recorded_logical_time() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("logical-time");
    write_events(&recording, &healthy_two_tick_events()?)?;
    let report = replay_measurement(
        &recording,
        measurement_config(FundingStateConfig::Disabled)?,
    )?;
    let timestamps: Vec<_> = report.ticks.iter().map(|tick| tick.tick_ts).collect();
    assert_eq!(
        timestamps,
        vec![
            UnixNanos(1_000_000_000),
            UnixNanos(1_000_000_000),
            UnixNanos(2_000_000_000),
            UnixNanos(2_000_000_000),
        ]
    );
    assert_eq!(report.source_replay_end_ts, UnixNanos(2_000_000_000));
    Ok(())
}

#[test]
fn reconnect_states_are_retained_as_fail_closed_evidence() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("reconnect");
    let entropy_key = key("entropy")?;
    let lighter_key = key("lighter")?;
    let entropy_first = book(
        "entropy",
        1,
        Decimal::from(99),
        Decimal::from(101),
        900_000_000,
    )?;
    let lighter_first = book(
        "lighter",
        1,
        Decimal::from(101),
        Decimal::from(103),
        950_000_000,
    )?;
    let lighter_second = book(
        "lighter",
        2,
        Decimal::from(102),
        Decimal::from(104),
        1_900_000_000,
    )?;
    let lighter_third = book(
        "lighter",
        3,
        Decimal::from(103),
        Decimal::from(105),
        3_100_000_000,
    )?;
    let entropy_recovery = book(
        "entropy",
        2,
        Decimal::from(100),
        Decimal::from(102),
        3_100_000_000,
    )?;
    let events = vec![
        RecordedEvent::feed_connection(
            &entropy_key,
            FeedConnectionState::Connected,
            UnixNanos(100_000_000),
        )?,
        RecordedEvent::feed_connection(
            &lighter_key,
            FeedConnectionState::Connected,
            UnixNanos(110_000_000),
        )?,
        RecordedEvent::market_book(&entropy_first)?,
        RecordedEvent::market_book(&lighter_first)?,
        RecordedEvent::feed_connection(
            &entropy_key,
            FeedConnectionState::Disconnected,
            UnixNanos(1_500_000_000),
        )?,
        RecordedEvent::market_book(&lighter_second)?,
        RecordedEvent::feed_connection(
            &entropy_key,
            FeedConnectionState::Reconnecting,
            UnixNanos(2_000_000_000),
        )?,
        RecordedEvent::feed_connection(
            &entropy_key,
            FeedConnectionState::Connected,
            UnixNanos(2_500_000_000),
        )?,
        RecordedEvent::market_book(&entropy_recovery)?,
        RecordedEvent::market_book(&lighter_third)?,
        health_assertion(2_500_000_000, &entropy_recovery, 4_000_000_000)?,
        health_assertion(110_000_000, &lighter_third, 4_000_000_000)?,
    ];
    write_events(&recording, &events)?;
    let report = replay_measurement(
        &recording,
        measurement_config(FundingStateConfig::Disabled)?,
    )?;
    assert!(report.feed_state_rejections.iter().any(|state| {
        state.venue_id.as_str() == "entropy"
            && state.connection == FeedConnectionState::Disconnected
    }));
    assert!(report.feed_state_rejections.iter().any(|state| {
        state.venue_id.as_str() == "entropy"
            && state.connection == FeedConnectionState::Reconnecting
    }));
    assert!(report.ticks.iter().any(|tick| {
        tick.tick_ts == UnixNanos(2_000_000_000)
            && matches!(
                tick.outcome,
                MeasurementTickOutcome::Rejected {
                    validity: MeasurementValidity::VenueUnhealthy,
                    ..
                }
            )
    }));
    Ok(())
}
