//! ClickHouse client + batched insert helpers.
pub mod bars;
pub mod features;
pub mod trades;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChError {
    #[error("clickhouse: {0}")]
    Client(String),
    #[error("insert: {0}")]
    Insert(String),
}

pub type ChClient = clickhouse::Client;

pub fn connect(url: &str) -> ChClient {
    clickhouse::Client::default().with_url(url)
}
