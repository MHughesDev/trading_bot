//! Core domain types — the irreversible foundation.
//!
//! No internal dependencies. All prices and sizes are newtypes over `Decimal`
//! with no `From<f64>` — the compiler enforces money safety.
//!
//! TODO(Phase 0): implement all types per DATA-001, DATA-002, DATA-003.

pub mod envelope;
pub mod timestamp;
pub mod money;
pub mod trust;
pub mod instrument;
pub mod ids;
pub mod lanes;
pub mod payloads;
pub mod order;
pub mod position;
pub mod strategy_def;
pub mod error;
