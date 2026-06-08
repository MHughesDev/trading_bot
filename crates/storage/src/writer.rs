//! Batching storage consumer — 10 k events OR 100 ms, whichever comes first.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::warn;
use uuid::Uuid;

use super::clickhouse::ChClient;
use super::parquet::ParquetWriter;
use super::postgres::PgPool;
use super::redis::RedisClient;

const BATCH_SIZE: usize = 10_000;
const FLUSH_INTERVAL: Duration = Duration::from_millis(100);

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
    pub fn new(
        pg: PgPool,
        ch: ChClient,
        parquet: Arc<ParquetWriter>,
        redis: Arc<Mutex<RedisClient>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(writer_task(rx, pg, ch, parquet, redis));
        Self { tx }
    }

    /// Submit an event for batched writing.
    ///
    /// Non-blocking; returns `false` if the channel is full (back-pressure signal).
    pub async fn submit(&self, event: RawEvent) -> bool {
        self.tx.try_send(event).is_ok()
    }
}

async fn writer_task(
    mut rx: mpsc::Receiver<RawEvent>,
    _pg: PgPool,
    _ch: ChClient,
    parquet: Arc<ParquetWriter>,
    redis: Arc<Mutex<RedisClient>>,
) {
    let mut batch: Vec<RawEvent> = Vec::with_capacity(BATCH_SIZE);
    let mut ticker = interval(FLUSH_INTERVAL);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(e) => {
                        batch.push(e);
                        if batch.len() >= BATCH_SIZE {
                            flush_batch(&mut batch, &parquet, &redis).await;
                        }
                    }
                    None => {
                        // Channel closed — flush remaining and exit.
                        if !batch.is_empty() {
                            flush_batch(&mut batch, &parquet, &redis).await;
                        }
                        return;
                    }
                }
            }
            _ = ticker.tick() => {
                if !batch.is_empty() {
                    flush_batch(&mut batch, &parquet, &redis).await;
                }
            }
        }
    }
}

async fn flush_batch(
    batch: &mut Vec<RawEvent>,
    parquet: &Arc<ParquetWriter>,
    redis: &Arc<Mutex<RedisClient>>,
) {
    // Group events by (lane, venue_id, instrument_id, date).
    let mut groups: HashMap<(&str, &str, &str, chrono::NaiveDate), Vec<usize>> = HashMap::new();
    for (idx, event) in batch.iter().enumerate() {
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
        let rows: Vec<Vec<u8>> = indices.iter().map(|&i| batch[i].raw_json.clone()).collect();
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

    // Mark every event as seen in Redis (best-effort).
    {
        let mut guard = redis.lock().await;
        for event in batch.iter() {
            if let Err(e) = guard.mark_seen(&event.event_id).await {
                warn!(event_id = %event.event_id, error = %e, "redis mark_seen failed");
            }
        }
    }

    batch.clear();
}
