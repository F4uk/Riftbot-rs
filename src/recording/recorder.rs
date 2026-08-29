//! Bounded, non-blocking producer path with deterministic background persistence.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    sync::mpsc::{self, SyncSender, TrySendError},
    thread::{self, JoinHandle},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::schema::{
    EventEnvelope, RECORDING_SCHEMA_VERSION, RecordedEvent, RecordingHeader, RecordingTrailer,
    SchemaError, SequencedEvent, sha256_hex,
};

/// Result returned only after every accepted event is flushed and synced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingSummary {
    pub schema_version: u16,
    pub event_count: u64,
    pub content_sha256: String,
}

/// Recorder failure. Buffer pressure is explicit and never silently drops an event.
#[derive(Debug, Error)]
pub enum RecorderError {
    #[error("recorder buffer capacity must be non-zero")]
    ZeroBufferCapacity,
    #[error("recording destination already exists: {0}")]
    DestinationExists(String),
    #[error("recorder buffer is full; event was not accepted")]
    BufferFull,
    #[error("recorder worker is no longer available")]
    WorkerStopped,
    #[error("recorder worker panicked")]
    WorkerPanicked,
    #[error("recording schema rejected event: {0}")]
    InvalidEvent(#[from] SchemaError),
    #[error("recording JSON serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("recording I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// A recorder whose hot-path API performs validation and a bounded `try_send`, never file I/O.
pub struct BufferedRecorder {
    sender: Option<SyncSender<RecordedEvent>>,
    worker: Option<JoinHandle<Result<RecordingSummary, RecorderError>>>,
}

impl BufferedRecorder {
    /// Creates a new recording without overwriting an existing file.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, RecorderError> {
        if capacity == 0 {
            return Err(RecorderError::ZeroBufferCapacity);
        }
        let path = path.as_ref();
        let file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(RecorderError::DestinationExists(path.display().to_string()));
            }
            Err(error) => return Err(RecorderError::Io(error)),
        };
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let worker = thread::Builder::new()
            .name("riftbot-recorder".to_owned())
            .spawn(move || write_recording(file, receiver))?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    /// Attempts to enqueue one event without waiting. `BufferFull` means it was not accepted.
    pub fn try_record(&self, event: RecordedEvent) -> Result<(), RecorderError> {
        event.validate()?;
        let Some(sender) = &self.sender else {
            return Err(RecorderError::WorkerStopped);
        };
        match sender.try_send(event) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(RecorderError::BufferFull),
            Err(TrySendError::Disconnected(_)) => Err(RecorderError::WorkerStopped),
        }
    }

    /// Closes input, drains FIFO, writes the integrity trailer, flushes, syncs, and joins.
    pub fn shutdown(mut self) -> Result<RecordingSummary, RecorderError> {
        self.sender.take();
        let Some(worker) = self.worker.take() else {
            return Err(RecorderError::WorkerStopped);
        };
        worker.join().map_err(|_| RecorderError::WorkerPanicked)?
    }
}

fn write_recording(
    file: File,
    receiver: mpsc::Receiver<RecordedEvent>,
) -> Result<RecordingSummary, RecorderError> {
    let mut writer = BufWriter::new(file);
    let mut content_hasher = Sha256::new();
    let header = serde_json::to_vec(&RecordingHeader::v1())?;
    write_hashed_line(&mut writer, &mut content_hasher, &header)?;

    let mut event_count = 0_u64;
    for event in receiver {
        event_count = event_count
            .checked_add(1)
            .ok_or_else(|| std::io::Error::other("recording event count overflow"))?;
        let payload = SequencedEvent {
            sequence: event_count,
            event,
        };
        let payload_bytes = serde_json::to_vec(&payload)?;
        let envelope = EventEnvelope {
            sequence: payload.sequence,
            event: payload.event,
            checksum_sha256: sha256_hex(&payload_bytes),
        };
        let line = serde_json::to_vec(&envelope)?;
        write_hashed_line(&mut writer, &mut content_hasher, &line)?;
    }

    let content_sha256 = format!("{:x}", content_hasher.finalize());
    let trailer = RecordingTrailer {
        schema_version: RECORDING_SCHEMA_VERSION,
        event_count,
        content_sha256: content_sha256.clone(),
    };
    write_line(&mut writer, &serde_json::to_vec(&trailer)?)?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(RecordingSummary {
        schema_version: RECORDING_SCHEMA_VERSION,
        event_count,
        content_sha256,
    })
}

fn write_hashed_line(
    writer: &mut BufWriter<File>,
    hasher: &mut Sha256,
    line: &[u8],
) -> Result<(), std::io::Error> {
    write_line(writer, line)?;
    hasher.update(line);
    hasher.update(b"\n");
    Ok(())
}

fn write_line(writer: &mut BufWriter<File>, line: &[u8]) -> Result<(), std::io::Error> {
    writer.write_all(line)?;
    writer.write_all(b"\n")
}
