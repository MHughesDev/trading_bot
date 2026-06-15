use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Flat,
}

/// Canonical inference output a strategy runtime consumes (kind-agnostic envelope).
/// `magnitude` is always `Decimal` (ADR-0002). `confidence` is 0..1 calibration, not money.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Forecast {
    pub model_id: String,
    pub version: u32,
    pub instrument_id: String,
    pub direction: Direction,
    /// ADR-0002: decimal, never f64.
    pub magnitude: Decimal,
    /// 0..1 calibration metric — not money, so f64 is intentional here.
    pub confidence: f64,
    pub horizon: String,
    pub produced_at: DateTime<Utc>,
}
