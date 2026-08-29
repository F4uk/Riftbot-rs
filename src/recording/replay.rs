//! Strict recording validation and deterministic offline replay through the P1 market path.

use std::{collections::BTreeSet, fs, path::Path};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    domain::{
        ids::{InstrumentId, VenueId},
        market::{FeedHealth, VenueBook},
        numeric::{DurationMillis, UnixNanos},
    },
    market::book_store::{BookStore, BookStoreError},
};

use super::schema::{
    EventEnvelope, RECORDING_FORMAT, RECORDING_SCHEMA_VERSION, RecordedEvent, RecordingHeader,
    RecordingTrailer, SchemaError, SequencedEvent, sha256_hex,
};

const MAX_RECORDING_BYTES: u64 = 256 * 1024 * 1024;

/// Compile-time P2 replay capability: market state only, with no execution dependency or hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaySafety {
    OfflineMarketDataOnly,
}

/// Kind of deterministic state transition produced for one recorded input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayStepKind {
    MarketBook,
    FeedConnection,
    FeedHealth,
}

/// Comparable output for one input event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayStep {
    pub sequence: u64,
    pub event_time: UnixNanos,
    pub kind: ReplayStepKind,
    pub normalized_book: Option<VenueBook>,
    pub resulting_health: Option<FeedHealth>,
}

/// Final normalized state for one feed at the deterministic replay end time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayFeedState {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub book: Option<VenueBook>,
    pub health: Option<FeedHealth>,
}

/// Complete deterministic result; equality covers event order and final normalized state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayReport {
    pub schema_version: u16,
    pub content_sha256: String,
    pub replay_end_ts: UnixNanos,
    pub safety: ReplaySafety,
    pub steps: Vec<ReplayStep>,
    pub final_feeds: Vec<ReplayFeedState>,
}

/// Offline replay configuration. It intentionally cannot accept an execution client.
#[derive(Clone, Copy, Debug)]
pub struct ReplayEngine {
    stale_after_ms: DurationMillis,
}

impl ReplayEngine {
    pub fn new(stale_after_ms: DurationMillis) -> Result<Self, ReplayError> {
        BookStore::new(stale_after_ms).map_err(ReplayError::StoreInitialization)?;
        Ok(Self { stale_after_ms })
    }

    #[must_use]
    pub const fn safety(&self) -> ReplaySafety {
        ReplaySafety::OfflineMarketDataOnly
    }

    /// Fully validates integrity/schema before applying anything to a fresh `BookStore`.
    pub fn replay_file(&self, path: impl AsRef<Path>) -> Result<ReplayReport, ReplayError> {
        let recording = load_recording(path.as_ref())?;
        self.replay(recording)
    }

    fn replay(&self, recording: LoadedRecording) -> Result<ReplayReport, ReplayError> {
        let mut store =
            BookStore::new(self.stale_after_ms).map_err(ReplayError::StoreInitialization)?;
        let mut keys = BTreeSet::new();
        let mut steps = Vec::with_capacity(recording.events.len());
        let mut replay_end_ts = UnixNanos(0);

        for input in recording.events {
            input
                .event
                .validate()
                .map_err(|source| ReplayError::InvalidEvent {
                    sequence: input.sequence,
                    source,
                })?;
            let key = input.event.feed_key();
            keys.insert(key.clone());
            let event_time = input.event.event_time();
            replay_end_ts = replay_end_ts.max(event_time);

            let (kind, normalized_book, resulting_health) = match input.event {
                RecordedEvent::MarketBook { book } => {
                    let normalized =
                        book.to_domain()
                            .map_err(|source| ReplayError::InvalidEvent {
                                sequence: input.sequence,
                                source,
                            })?;
                    store
                        .update(normalized.clone())
                        .map_err(|source| ReplayError::BookStore {
                            sequence: input.sequence,
                            source,
                        })?;
                    let health = store.health(&key, event_time);
                    (ReplayStepKind::MarketBook, Some(normalized), health)
                }
                RecordedEvent::FeedConnection { connection } => {
                    store
                        .set_connection_state(
                            key.clone(),
                            connection.state,
                            connection.transition_ts,
                        )
                        .map_err(|source| ReplayError::BookStore {
                            sequence: input.sequence,
                            source,
                        })?;
                    let health = store.health(&key, event_time);
                    (ReplayStepKind::FeedConnection, None, health)
                }
                RecordedEvent::FeedHealth { health } => {
                    let actual = store.health(&key, health.observed_at);
                    let expected = health.expected.to_domain();
                    if actual.as_ref() != Some(&expected) {
                        return Err(ReplayError::HealthMismatch {
                            sequence: input.sequence,
                            expected: Box::new(expected),
                            actual: Box::new(actual),
                        });
                    }
                    (ReplayStepKind::FeedHealth, None, actual)
                }
            };
            steps.push(ReplayStep {
                sequence: input.sequence,
                event_time,
                kind,
                normalized_book,
                resulting_health,
            });
        }

        let final_feeds = keys
            .into_iter()
            .map(|key| ReplayFeedState {
                venue_id: key.venue_id.clone(),
                instrument_id: key.instrument_id.clone(),
                book: store.book(&key, replay_end_ts),
                health: store.health(&key, replay_end_ts),
            })
            .collect();
        Ok(ReplayReport {
            schema_version: RECORDING_SCHEMA_VERSION,
            content_sha256: recording.content_sha256,
            replay_end_ts,
            safety: self.safety(),
            steps,
            final_feeds,
        })
    }
}

#[derive(Debug)]
struct LoadedRecording {
    events: Vec<SequencedEvent>,
    content_sha256: String,
}

fn load_recording(path: &Path) -> Result<LoadedRecording, ReplayError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_RECORDING_BYTES {
        return Err(ReplayError::RecordingTooLarge {
            actual: metadata.len(),
            maximum: MAX_RECORDING_BYTES,
        });
    }
    let bytes = fs::read(path)?;
    let actual_size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if actual_size > MAX_RECORDING_BYTES {
        return Err(ReplayError::RecordingTooLarge {
            actual: actual_size,
            maximum: MAX_RECORDING_BYTES,
        });
    }
    if bytes.is_empty() {
        return Err(ReplayError::EmptyRecording);
    }
    if !bytes.ends_with(b"\n") {
        return Err(ReplayError::IncompleteRecording);
    }
    let lines: Vec<&[u8]> = bytes[..bytes.len() - 1]
        .split(|byte| *byte == b'\n')
        .collect();
    if lines.len() < 2 || lines.iter().any(|line| line.is_empty()) {
        return Err(ReplayError::IncompleteRecording);
    }

    let header: RecordingHeader = parse_json_line(lines[0], 1)?;
    if header.format != RECORDING_FORMAT {
        return Err(ReplayError::InvalidFormat(header.format));
    }
    ensure_supported_version(header.schema_version)?;

    let trailer_line = lines.len();
    let trailer: RecordingTrailer = parse_json_line(lines[trailer_line - 1], trailer_line)?;
    ensure_supported_version(trailer.schema_version)?;
    if trailer.schema_version != header.schema_version {
        return Err(ReplayError::VersionMismatch {
            header: header.schema_version,
            trailer: trailer.schema_version,
        });
    }

    let actual_event_count =
        u64::try_from(lines.len() - 2).map_err(|_| ReplayError::IncompleteRecording)?;
    if trailer.event_count != actual_event_count {
        return Err(ReplayError::EventCountMismatch {
            declared: trailer.event_count,
            actual: actual_event_count,
        });
    }

    let mut content_hasher = Sha256::new();
    content_hasher.update(lines[0]);
    content_hasher.update(b"\n");
    let mut events = Vec::with_capacity(lines.len() - 2);
    for (offset, line) in lines[1..trailer_line - 1].iter().enumerate() {
        let line_number = offset + 2;
        let envelope: EventEnvelope = parse_json_line(line, line_number)?;
        let expected_sequence = u64::try_from(offset)
            .map_err(|_| ReplayError::IncompleteRecording)?
            .checked_add(1)
            .ok_or(ReplayError::IncompleteRecording)?;
        if envelope.sequence != expected_sequence {
            return Err(ReplayError::NonContiguousSequence {
                expected: expected_sequence,
                actual: envelope.sequence,
            });
        }
        let payload = SequencedEvent {
            sequence: envelope.sequence,
            event: envelope.event,
        };
        let payload_bytes = serde_json::to_vec(&payload)?;
        let actual_checksum = sha256_hex(&payload_bytes);
        if envelope.checksum_sha256 != actual_checksum {
            return Err(ReplayError::EventChecksumMismatch {
                sequence: envelope.sequence,
            });
        }
        payload
            .event
            .validate()
            .map_err(|source| ReplayError::InvalidEvent {
                sequence: payload.sequence,
                source,
            })?;
        events.push(payload);
        content_hasher.update(line);
        content_hasher.update(b"\n");
    }
    let actual_content_sha256 = format!("{:x}", content_hasher.finalize());
    if trailer.content_sha256 != actual_content_sha256 {
        return Err(ReplayError::ContentChecksumMismatch);
    }

    Ok(LoadedRecording {
        events,
        content_sha256: actual_content_sha256,
    })
}

fn parse_json_line<'a, T>(line: &'a [u8], line_number: usize) -> Result<T, ReplayError>
where
    T: serde::Deserialize<'a>,
{
    serde_json::from_slice(line).map_err(|source| ReplayError::InvalidJson {
        line: line_number,
        source,
    })
}

fn ensure_supported_version(version: u16) -> Result<(), ReplayError> {
    if version != RECORDING_SCHEMA_VERSION {
        return Err(ReplayError::UnsupportedSchemaVersion(version));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("recording I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("recording JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("recording is empty")]
    EmptyRecording,
    #[error("recording is incomplete or is missing its final newline/trailer")]
    IncompleteRecording,
    #[error("recording size {actual} exceeds limit {maximum}")]
    RecordingTooLarge { actual: u64, maximum: u64 },
    #[error("invalid JSON on recording line {line}: {source}")]
    InvalidJson {
        line: usize,
        source: serde_json::Error,
    },
    #[error("unrecognized recording format: {0}")]
    InvalidFormat(String),
    #[error("unsupported recording schema version: {0}")]
    UnsupportedSchemaVersion(u16),
    #[error("header schema {header} differs from trailer schema {trailer}")]
    VersionMismatch { header: u16, trailer: u16 },
    #[error("trailer declares {declared} events but file contains {actual}")]
    EventCountMismatch { declared: u64, actual: u64 },
    #[error("event sequence is not contiguous: expected {expected}, received {actual}")]
    NonContiguousSequence { expected: u64, actual: u64 },
    #[error("event {sequence} checksum does not match its payload")]
    EventChecksumMismatch { sequence: u64 },
    #[error("recording content checksum does not match its trailer")]
    ContentChecksumMismatch,
    #[error("recorded event {sequence} is invalid: {source}")]
    InvalidEvent { sequence: u64, source: SchemaError },
    #[error("could not initialize deterministic store: {0}")]
    StoreInitialization(BookStoreError),
    #[error("recorded event {sequence} was rejected by BookStore: {source}")]
    BookStore {
        sequence: u64,
        source: BookStoreError,
    },
    #[error("recorded health assertion at event {sequence} did not match replay state")]
    HealthMismatch {
        sequence: u64,
        expected: Box<FeedHealth>,
        actual: Box<Option<FeedHealth>>,
    },
}
