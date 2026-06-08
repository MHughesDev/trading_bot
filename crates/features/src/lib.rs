//! PURE technical indicator / feature computation.
//!
//! Same purity contract as `builders`: no I/O, no side-effects, no wall-clock reads.
//! The same code runs identically live and in replay.
//!
//! Float math is acceptable for indicator values — feature values are *versioned*
//! and recorded at their `available_time`, so replay sees the exact values live
//! produced rather than recomputing them.

pub mod ema;
pub mod rsi;
pub mod window;

pub use ema::{Ema, EMA_FEATURE_VERSION};
pub use rsi::{Rsi, RSI_FEATURE_VERSION};
pub use window::Window;

use chrono::{DateTime, Utc};

/// A computed indicator value carrying its algorithm version and availability time.
///
/// `feature_version` is incremented whenever the computation logic changes; replays
/// can compare this to the recorded version to detect algorithm drift.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureValue {
    /// Feature name, e.g. `"ema_7"` or `"rsi_14"`.
    pub name: String,
    /// Computed float value.
    pub value: f64,
    /// Monotonically increasing algorithm version.
    pub feature_version: u32,
    /// `available_time` of the event that produced this value.
    pub available_time: DateTime<Utc>,
}

impl FeatureValue {
    pub fn new(
        name: impl Into<String>,
        value: f64,
        feature_version: u32,
        available_time: DateTime<Utc>,
    ) -> Self {
        Self {
            name: name.into(),
            value,
            feature_version,
            available_time,
        }
    }
}
