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

/// Handle to the background batching writer.
pub struct StorageWriter {
    tx: mpsc::Sender<RawEvent>,
}

impl StorageWriter {
    /// Create and spawn the background writer task.
    pub fn new(pg: PgPool, ch: ChClient, parquet: Arc<ParquetWriter>) -> Self {
        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(writer_task(rx, pg, ch, parquet));
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

async fn writer_task(
    mut rx: mpsc::Receiver<RawEvent>,
    _pg: PgPool,
    _ch: ChClient,
    parquet: Arc<ParquetWriter>,
) {
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

    batch.clear();
}
