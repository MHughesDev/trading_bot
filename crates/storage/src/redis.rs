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

    /// Check if an `event_id` has been seen (dedup). Returns `true` if already in the set.
    pub async fn seen(&mut self, event_id: &str) -> Result<bool, RedisError> {
        let result: i64 = self.conn.sismember("seen:events", event_id).await?;
        Ok(result == 1)
    }

    /// Mark `event_id` as seen.
    pub async fn mark_seen(&mut self, event_id: &str) -> Result<(), RedisError> {
        self.conn.sadd::<_, _, ()>("seen:events", event_id).await?;
        Ok(())
    }
}
