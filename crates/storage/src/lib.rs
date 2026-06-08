//! Storage backends split by access pattern.
//! Postgres=transactional, ClickHouse=time-series, Parquet=raw archive, Redis=latest-state cache.
pub mod clickhouse;
pub mod parquet;
pub mod postgres;
pub mod redis;
pub mod writer;

pub use postgres::PgPool;
pub use writer::StorageWriter;
