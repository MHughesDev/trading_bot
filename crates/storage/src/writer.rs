//! Batching storage consumer — 10 k events OR 100 ms, whichever comes first.
//!
//! Redis is used *only* as a last-value cache for the UI gateway (`set_latest`).
//! Write-path dedup is handled entirely in-process by `DedupRing`, eliminating
//! the 10 k sequential Redis RTTs that previously occurred on every flush.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use ahash::AHashSet;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::warn;
use uuid::Uuid;

use super::clickhouse::ChClient;
use super::parquet::ParquetWriter;
use super::postgres::PgPool;

const BATCH_SIZE: usize = 10_000;
const FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// Capacity of the in-process dedup ring (number of event IDs retained).
/// At ~40 bytes per UUID string this is ~2.5 MB of overhead.
const DEDUP_CAPACITY: usize = 64_000;

// ---------------------------------------------------------------------------
// In-process bounded dedup ring
// ---------------------------------------------------------------------------

/// Fixed-capacity dedup structure.  When full it evicts the oldest entry so
/// memory stays bounded regardless of event volume.
struct DedupRing {
    seen: AHashSet<String>,
    ring: VecDeque<String>,
    capacity: usize,
}

impl DedupRing {
    fn new(capacity: usize) -> Self {
        Self {
            seen: AHashSet::with_capacity(capacity),
            ring: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Returns `true` if `id` was already seen (duplicate — caller should skip).
    /// Returns `false` and records `id` if this is the first occurrence.
    fn is_seen_or_insert(&mut self, id: &str) -> bool {
        if self.seen.contains(id) {
            return true;
        }
        if self.ring.len() >= self.capacity {
            if let Some(evicted) = self.ring.pop_front() {
                self.seen.remove(&evicted);
            }
        }
        self.ring.push_back(id.to_owned());
        self.seen.insert(id.to_owned());
        false
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// A single raw normalized event ready to be archived.
pub struct RawEvent {
    pub lane: String,
    pub venue_id: String,
    pub instrument_id: String,
    pub date: chrono::NaiveDate,
    pub event_id: String,
    pub raw_json: Vec<u8>,
}

impl RawEvent {
    /// The archive row for one normalized trade envelope: the envelope as JSON
    /// (`EventEnvelope` is `Serialize`), partitioned by lane, venue, instrument
    /// and the UTC date of its **exchange** timestamp.
    ///
    /// `event_id` is `venue/instrument/source/sequence` — unique per source
    /// stream, which is what [`DedupRing`] needs. `None` when the envelope
    /// cannot be named or dated (an unknown interned id or an out-of-range
    /// timestamp): such an event is not archived into a guessed partition.
    #[must_use]
    pub fn from_envelope(lane: &str, env: &domain::EventEnvelope) -> Option<Self> {
        let venue = domain::interned::venue_name(env.venue_id)?;
        let instrument = domain::interned::instrument_name(env.instrument_id)?;
        let source = domain::interned::source_name(env.source_id)?;
        let date = chrono::DateTime::from_timestamp_nanos(env.timestamp_ns).date_naive();
        let raw_json = serde_json::to_vec(env).ok()?;
        Some(Self {
            lane: lane.to_owned(),
            venue_id: venue.clone(),
            instrument_id: instrument.clone(),
            date,
            event_id: format!("{venue}/{instrument}/{source}/{}", env.sequence),
            raw_json,
        })
    }
}

/// Handle to the background batching writer.
pub struct StorageWriter {
    tx: mpsc::Sender<RawEvent>,
}

impl StorageWriter {
    /// Create and spawn the background writer task.
    pub fn new(pg: PgPool, ch: ChClient, parquet: Arc<ParquetWriter>) -> Self {
        let (tx, rx) = mpsc::channel(100_000);
        let _ = (pg, ch);
        tokio::spawn(archive_task(rx, parquet));
        Self { tx }
    }

    /// A writer that only archives to Parquet. The Postgres and ClickHouse
    /// handles `new` takes are unused by the writer task (it has only ever
    /// written Parquet), so the ingest path should not have to hold them.
    pub fn archive_only(parquet: Arc<ParquetWriter>) -> Self {
        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(archive_task(rx, parquet));
        Self { tx }
    }

    /// Submit an event for batched writing.
    ///
    /// Non-blocking; returns `false` if the channel is full (back-pressure signal).
    pub async fn submit(&self, event: RawEvent) -> bool {
        self.tx.try_send(event).is_ok()
    }
}

// ---------------------------------------------------------------------------
// Background task
// ---------------------------------------------------------------------------

async fn archive_task(mut rx: mpsc::Receiver<RawEvent>, parquet: Arc<ParquetWriter>) {
    let mut batch: Vec<RawEvent> = Vec::with_capacity(BATCH_SIZE);
    let mut ticker = interval(FLUSH_INTERVAL);
    let mut dedup = DedupRing::new(DEDUP_CAPACITY);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(e) => {
                        batch.push(e);
                        if batch.len() >= BATCH_SIZE {
                            flush_batch(&mut batch, &parquet, &mut dedup).await;
                        }
                    }
                    None => {
                        // Channel closed — flush remaining and exit.
                        if !batch.is_empty() {
                            flush_batch(&mut batch, &parquet, &mut dedup).await;
                        }
                        return;
                    }
                }
            }
            _ = ticker.tick() => {
                if !batch.is_empty() {
                    flush_batch(&mut batch, &parquet, &mut dedup).await;
                }
            }
        }
    }
}

async fn flush_batch(
    batch: &mut Vec<RawEvent>,
    parquet: &Arc<ParquetWriter>,
    dedup: &mut DedupRing,
) {
    // Group events by (lane, venue_id, instrument_id, date), deduplicating
    // in-process via DedupRing instead of issuing Redis round-trips per event.
    let mut groups: HashMap<(&str, &str, &str, chrono::NaiveDate), Vec<usize>> = HashMap::new();
    for (idx, event) in batch.iter().enumerate() {
        if dedup.is_seen_or_insert(&event.event_id) {
            continue; // duplicate — skip without touching Redis
        }
        groups
            .entry((
                &event.lane,
                &event.venue_id,
                &event.instrument_id,
                event.date,
            ))
            .or_default()
            .push(idx);
    }

    for ((lane, venue_id, instrument_id, date), indices) in &groups {
        // Build a slice of references rather than cloning each row.
        let rows: Vec<&[u8]> = indices
            .iter()
            .map(|&i| batch[i].raw_json.as_slice())
            .collect();
        let batch_id = Uuid::new_v4().to_string();

        if let Err(e) = parquet
            .write_batch(lane, venue_id, instrument_id, *date, &batch_id, &rows)
            .await
        {
            warn!(
                lane,
                venue_id,
                instrument_id,
                %date,
                error = %e,
                "parquet write_batch failed"
            );
        }
    }

    observability::metrics::inc(
        "archive_flushes_total",
        "Raw-event archive batches flushed to Parquet.",
        &[],
    );
    batch.clear();
}

#[cfg(test)]
mod from_envelope_tests {
    use super::*;

    #[test]
    fn an_envelope_becomes_a_dated_partitioned_row_with_a_stable_id() {
        let env = domain::EventEnvelope::new(
            domain::interned::intern_instrument("ARCHIVE-TEST-USD"),
            domain::interned::intern_venue("archive_test_venue"),
            domain::interned::intern_source("archive_test_ws"),
            7,
            1_780_000_000_000_000_000,
            vec![1, 2, 3],
        );
        let ev = RawEvent::from_envelope("market.trades", &env).expect("nameable");
        assert_eq!(ev.venue_id, "archive_test_venue");
        assert_eq!(ev.instrument_id, "ARCHIVE-TEST-USD");
        assert_eq!(
            ev.event_id,
            "archive_test_venue/ARCHIVE-TEST-USD/archive_test_ws/7"
        );
        assert_eq!(ev.date.to_string(), "2026-05-28");
        assert!(!ev.raw_json.is_empty());
    }
}
