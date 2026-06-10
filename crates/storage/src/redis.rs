//! Latest-state cache (latest:{lane}:{instrument}) + seen-set dedup.

use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RedisError {
    #[error("redis: {0}")]
    Client(#[from] redis::RedisError),
}

pub struct RedisClient {
    conn: MultiplexedConnection,
}

impl RedisClient {
    pub async fn connect(url: &str) -> Result<Self, RedisError> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self { conn })
    }

    /// Set the latest state for a `(lane, instrument_id)` key. TTL = 3600 s.
    pub async fn set_latest(
        &mut self,
        lane: &str,
        instrument_id: &str,
        value: &[u8],
    ) -> Result<(), RedisError> {
        let key = format!("latest:{lane}:{instrument_id}");
        self.conn.set_ex::<_, _, ()>(&key, value, 3600).await?;
        Ok(())
    }

    /// Get the latest state for a `(lane, instrument_id)` key.
    pub async fn get_latest(
        &mut self,
        lane: &str,
        instrument_id: &str,
    ) -> Result<Option<Vec<u8>>, RedisError> {
        let key = format!("latest:{lane}:{instrument_id}");
        let val: Option<Vec<u8>> = self.conn.get(&key).await?;
        Ok(val)
    }

    /// Check if an `event_id` has been seen (dedup). Returns `true` if already seen.
    ///
    /// Uses per-event keys with a TTL instead of a single monolithic set so
    /// Redis memory is bounded (H-5).  The dedup window matches the Parquet
    /// replay redelivery guarantee (24 h).
    pub async fn seen(&mut self, event_id: &str) -> Result<bool, RedisError> {
        let key = format!("seen:event:{event_id}");
        let exists: bool = self.conn.exists(&key).await?;
        Ok(exists)
    }

    /// Mark `event_id` as seen with a 24-hour TTL.
    pub async fn mark_seen(&mut self, event_id: &str) -> Result<(), RedisError> {
        let key = format!("seen:event:{event_id}");
        // SET key 1 EX 86400 NX — atomic set-if-not-exists with TTL.
        redis::cmd("SET")
            .arg(&key)
            .arg(1_u8)
            .arg("EX")
            .arg(86_400_u64)
            .arg("NX")
            .exec_async(&mut self.conn)
            .await?;
        Ok(())
    }
}
