//! TODO(Phase 1): Storage backends split by access pattern.
//! Postgres=transactional, ClickHouse=time-series, Parquet=raw archive, Redis=latest-state cache.
pub mod postgres;
pub mod clickhouse;
pub mod parquet;
pub mod redis;
pub mod writer;
