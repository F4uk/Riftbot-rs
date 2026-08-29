use std::{
    error::Error,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use riftbot::{
    domain::{
        ids::{InstrumentId, VenueId},
        market::{
            BookLevel, BookVersion, FeedConnectionState, FeedFreshness, FeedHealth, VenueBook,
        },
        numeric::{BaseQty, DurationMillis, Price, UnixNanos},
    },
    market::{book_store::FeedKey, normalizer::NormalizationError},
    recording::{
        recorder::{BufferedRecorder, RecorderError},
        replay::{ReplayEngine, ReplayError, ReplaySafety},
        schema::{RecordedBookLevel, RecordedEvent, RecordedMarketBook, SchemaError},
    },
};
use rust_decimal::Decimal;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestRecording(PathBuf);

impl TestRecording {
    fn new(label: &str) -> Self {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "riftbot-p2-{label}-{}-{sequence}.jsonl",
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

fn key() -> Result<FeedKey, Box<dyn Error>> {
    Ok(FeedKey::new(
        VenueId::try_from("entropy")?,
        InstrumentId::try_from("io:SNDK-USD-PERP.HYPERLIQUID")?,
    ))
}

fn book(version: u64, exchange_ts: u64, receive_ts: u64) -> Result<VenueBook, Box<dyn Error>> {
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
        exchange_ts: UnixNanos(exchange_ts),
        receive_ts: UnixNanos(receive_ts),
        age_ms: DurationMillis(0),
        version: BookVersion(version),
    })
}

fn healthy_events() -> Result<Vec<RecordedEvent>, Box<dyn Error>> {
    let key = key()?;
    let book = book(1, 1_050_000_000, 1_100_000_000)?;
    Ok(vec![
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Connected,
            UnixNanos(1_000_000_000),
        )?,
        RecordedEvent::market_book(&book)?,
        RecordedEvent::feed_health(
            UnixNanos(1_200_000_000),
            FeedHealth {
                venue_id: key.venue_id,
                instrument_id: key.instrument_id,
                connection: FeedConnectionState::Connected,
                freshness: FeedFreshness::Fresh,
                last_transition_ts: UnixNanos(1_000_000_000),
                last_exchange_ts: Some(UnixNanos(1_050_000_000)),
                last_receive_ts: Some(UnixNanos(1_100_000_000)),
                age_ms: Some(DurationMillis(100)),
            },
        )?,
    ])
}

fn write_events(recording: &TestRecording, events: &[RecordedEvent]) -> Result<(), Box<dyn Error>> {
    let recorder = BufferedRecorder::create(&recording.0, events.len() + 1)?;
    for event in events {
        recorder.try_record(event.clone())?;
    }
    let summary = recorder.shutdown()?;
    assert_eq!(summary.event_count, u64::try_from(events.len())?);
    Ok(())
}

#[test]
fn record_to_replay_round_trip() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("round-trip");
    write_events(&recording, &healthy_events()?)?;

    let report = ReplayEngine::new(DurationMillis(500))?.replay_file(&recording.0)?;
    assert_eq!(report.steps.len(), 3);
    assert_eq!(report.final_feeds.len(), 1);
    assert_eq!(
        report.final_feeds[0]
            .health
            .as_ref()
            .map(|health| health.freshness),
        Some(FeedFreshness::Fresh)
    );
    Ok(())
}

#[test]
fn identical_replay_twice_produces_identical_outputs() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("repeat");
    write_events(&recording, &healthy_events()?)?;
    let engine = ReplayEngine::new(DurationMillis(500))?;

    assert_eq!(
        engine.replay_file(&recording.0)?,
        engine.replay_file(&recording.0)?
    );
    Ok(())
}

#[test]
fn replay_preserves_exchange_and_receive_timestamps() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("timestamps");
    write_events(&recording, &healthy_events()?)?;
    let report = ReplayEngine::new(DurationMillis(500))?.replay_file(&recording.0)?;
    let replayed = report.steps[1]
        .normalized_book
        .as_ref()
        .ok_or("market step did not contain a book")?;

    assert_eq!(replayed.exchange_ts, UnixNanos(1_050_000_000));
    assert_eq!(replayed.receive_ts, UnixNanos(1_100_000_000));
    assert_eq!(replayed.version, BookVersion(1));
    assert_eq!(replayed.venue_id, key()?.venue_id);
    assert_eq!(replayed.instrument_id, key()?.instrument_id);
    Ok(())
}

#[test]
fn replay_preserves_disconnect_reconnect_recovery_sequence() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("reconnect");
    let key = key()?;
    let events = vec![
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Connected,
            UnixNanos(1_000_000_000),
        )?,
        RecordedEvent::market_book(&book(1, 1_050_000_000, 1_100_000_000)?)?,
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Disconnected,
            UnixNanos(2_000_000_000),
        )?,
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Reconnecting,
            UnixNanos(3_000_000_000),
        )?,
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Connected,
            UnixNanos(4_000_000_000),
        )?,
        RecordedEvent::market_book(&book(2, 4_050_000_000, 4_100_000_000)?)?,
    ];
    write_events(&recording, &events)?;
    let report = ReplayEngine::new(DurationMillis(500))?.replay_file(&recording.0)?;

    let states: Vec<_> = report
        .steps
        .iter()
        .map(|step| {
            step.resulting_health
                .as_ref()
                .map(|health| (health.connection, health.freshness))
        })
        .collect();
    assert_eq!(
        states,
        vec![
            Some((
                FeedConnectionState::Connected,
                FeedFreshness::AwaitingRecovery
            )),
            Some((FeedConnectionState::Connected, FeedFreshness::Fresh)),
            Some((
                FeedConnectionState::Disconnected,
                FeedFreshness::AwaitingRecovery
            )),
            Some((
                FeedConnectionState::Reconnecting,
                FeedFreshness::AwaitingRecovery
            )),
            Some((
                FeedConnectionState::Connected,
                FeedFreshness::AwaitingRecovery
            )),
            Some((FeedConnectionState::Connected, FeedFreshness::Fresh)),
        ]
    );
    Ok(())
}

#[test]
fn stale_calculation_is_deterministic_from_recorded_observation_time() -> Result<(), Box<dyn Error>>
{
    let recording = TestRecording::new("stale");
    let key = key()?;
    let events = vec![
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Connected,
            UnixNanos(1_000_000_000),
        )?,
        RecordedEvent::market_book(&book(1, 1_050_000_000, 1_100_000_000)?)?,
        RecordedEvent::feed_health(
            UnixNanos(1_601_000_000),
            FeedHealth {
                venue_id: key.venue_id,
                instrument_id: key.instrument_id,
                connection: FeedConnectionState::Connected,
                freshness: FeedFreshness::Stale,
                last_transition_ts: UnixNanos(1_000_000_000),
                last_exchange_ts: Some(UnixNanos(1_050_000_000)),
                last_receive_ts: Some(UnixNanos(1_100_000_000)),
                age_ms: Some(DurationMillis(501)),
            },
        )?,
    ];
    write_events(&recording, &events)?;
    let engine = ReplayEngine::new(DurationMillis(500))?;

    let first = engine.replay_file(&recording.0)?;
    let second = engine.replay_file(&recording.0)?;
    assert_eq!(first, second);
    assert_eq!(
        first.final_feeds[0]
            .health
            .as_ref()
            .map(|health| (health.freshness, health.age_ms)),
        Some((FeedFreshness::Stale, Some(DurationMillis(501))))
    );
    Ok(())
}

#[test]
fn invalid_schema_version_is_rejected() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("unknown-version");
    fs::write(
        &recording.0,
        b"{\"format\":\"riftbot-recording\",\"schema_version\":999}\n{}\n",
    )?;

    assert!(matches!(
        ReplayEngine::new(DurationMillis(500))?.replay_file(&recording.0),
        Err(ReplayError::UnsupportedSchemaVersion(999))
    ));
    Ok(())
}

#[test]
fn truncated_and_corrupt_records_are_rejected() -> Result<(), Box<dyn Error>> {
    let valid = TestRecording::new("valid-integrity");
    write_events(&valid, &healthy_events()?)?;
    let bytes = fs::read(&valid.0)?;
    let engine = ReplayEngine::new(DurationMillis(500))?;

    let truncated = TestRecording::new("truncated");
    fs::write(&truncated.0, &bytes[..bytes.len() - 5])?;
    assert!(matches!(
        engine.replay_file(&truncated.0),
        Err(ReplayError::IncompleteRecording)
    ));

    let corrupt = TestRecording::new("corrupt");
    let mut corrupt_bytes = bytes;
    let needle = b"\"quantity\":\"1\"";
    let start = corrupt_bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .ok_or("quantity field not found")?;
    corrupt_bytes[start + needle.len() - 2] = b'2';
    fs::write(&corrupt.0, corrupt_bytes)?;
    assert!(matches!(
        engine.replay_file(&corrupt.0),
        Err(ReplayError::EventChecksumMismatch { .. })
    ));
    Ok(())
}

#[test]
fn invalid_normalized_domain_data_is_rejected() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("invalid-domain");
    let recorder = BufferedRecorder::create(&recording.0, 2)?;
    let key = key()?;
    let invalid = RecordedEvent::MarketBook {
        book: RecordedMarketBook {
            venue_id: key.venue_id,
            instrument_id: key.instrument_id,
            bids: vec![RecordedBookLevel {
                price: Decimal::new(102, 0),
                quantity: Decimal::ONE,
            }],
            asks: vec![RecordedBookLevel {
                price: Decimal::new(101, 0),
                quantity: Decimal::ONE,
            }],
            exchange_ts: UnixNanos(1_000_000_000),
            receive_ts: UnixNanos(1_100_000_000),
            version: BookVersion(1),
        },
    };

    assert!(matches!(
        recorder.try_record(invalid),
        Err(RecorderError::InvalidEvent(SchemaError::Normalization(
            NormalizationError::CrossedBook { .. }
        )))
    ));
    assert_eq!(recorder.shutdown()?.event_count, 0);
    Ok(())
}

#[test]
fn replay_has_no_live_execution_capability() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("offline-only");
    write_events(&recording, &healthy_events()?)?;
    let engine = ReplayEngine::new(DurationMillis(500))?;

    assert_eq!(engine.safety(), ReplaySafety::OfflineMarketDataOnly);
    assert_eq!(
        engine.replay_file(&recording.0)?.safety,
        ReplaySafety::OfflineMarketDataOnly
    );
    Ok(())
}

#[test]
fn recorder_shutdown_flushes_all_accepted_buffered_records_in_order() -> Result<(), Box<dyn Error>>
{
    let recording = TestRecording::new("shutdown-flush");
    let recorder = BufferedRecorder::create(&recording.0, 2)?;
    let mut accepted_versions = Vec::new();
    for version in 1..=100 {
        let event = RecordedEvent::market_book(&book(
            version,
            1_000_000_000 + version,
            2_000_000_000 + version,
        )?)?;
        match recorder.try_record(event) {
            Ok(()) => accepted_versions.push(BookVersion(version)),
            Err(RecorderError::BufferFull) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let summary = recorder.shutdown()?;
    assert!(!accepted_versions.is_empty());
    assert_eq!(summary.event_count, u64::try_from(accepted_versions.len())?);

    let report = ReplayEngine::new(DurationMillis(500))?.replay_file(&recording.0)?;
    let replayed_versions: Vec<_> = report
        .steps
        .iter()
        .filter_map(|step| step.normalized_book.as_ref().map(|book| book.version))
        .collect();
    assert_eq!(replayed_versions, accepted_versions);
    Ok(())
}

#[test]
fn inconsistent_recorded_health_fails_closed() -> Result<(), Box<dyn Error>> {
    let recording = TestRecording::new("health-mismatch");
    let key = key()?;
    let events = vec![
        RecordedEvent::feed_connection(
            &key,
            FeedConnectionState::Connected,
            UnixNanos(1_000_000_000),
        )?,
        RecordedEvent::feed_health(
            UnixNanos(1_100_000_000),
            FeedHealth {
                venue_id: key.venue_id,
                instrument_id: key.instrument_id,
                connection: FeedConnectionState::Connected,
                freshness: FeedFreshness::Fresh,
                last_transition_ts: UnixNanos(1_000_000_000),
                last_exchange_ts: None,
                last_receive_ts: None,
                age_ms: None,
            },
        )?,
    ];
    write_events(&recording, &events)?;

    assert!(matches!(
        ReplayEngine::new(DurationMillis(500))?.replay_file(&recording.0),
        Err(ReplayError::HealthMismatch { sequence: 2, .. })
    ));
    Ok(())
}
