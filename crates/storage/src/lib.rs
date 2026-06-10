//! Storage backends split by access pattern.
//! Postgres=transactional, ClickHouse=time-series, Parquet=raw archive, Redis=latest-state cache.
pub mod automation;
pub mod clickhouse;
pub mod ledger;
pub mod parquet;
pub mod pnl;
pub mod postgres;
pub mod redis;
pub mod strategy_manifest;
pub mod writer;

pub use postgres::PgPool;
pub use writer::StorageWriter;
