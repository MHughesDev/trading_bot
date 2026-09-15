//! PURE technical indicator / feature computation.
//!
//! Same purity contract as `builders`: no I/O, no side-effects, no wall-clock reads.
//! The same code runs identically live and in replay.
//!
//! Float math is acceptable for indicator values — feature values are *versioned*
//! and recorded at their `available_time`. Every feature is a windowed
//! implementation resolved through [`runtime`]: one implementation per feature,
//! evaluated by one function, for backfill and live alike (INV-13, INV-14).

pub mod align;
pub mod consistency;
pub mod ema;
pub mod feature_sets;
pub mod leakage;
pub mod leakage_harness;
pub mod registry;
pub mod rsi;
pub mod runtime;
pub mod training_frame;
pub mod walk_forward;

use chrono::{DateTime, Utc};

/// Publication lag, in milliseconds, of anything derived from a closed bar.
///
/// Zero, and named rather than written as a literal in fifty places: the moment a
/// family reads a source that is *published* after the period it describes — a
/// filing, a macro release, a revision-bearing series — its declared lag is the
/// source's lag, and the PIT check has something to check. A family over a lagged
/// source that copied this constant would be claiming the source arrives
/// instantly, which is the leak DATA-005 exists to prevent.
pub const PIT_LAG_BAR_DERIVED_MS: u64 = 0;
pub use dataplane::feature::{
    p99_relative_diff, ConsistencyDiff, Diagnosis, Feature, FeatureDef, FeatureError, FeatureRow, FeatureRuntime, MemoryServeLog, ServeLog,
    ServeRecord,
};
pub use align::{densify_bars, densify_to_master_clock, BarObs, MasterClockFrame};
pub use ema::{Ema, EMA_FEATURE_VERSION};
pub use feature_sets::{
    is_known as is_known_feature, list_feature_sets, resolve as resolve_feature_set,
    validate_features, validate_user_spec, FeatureSetSpec, REGISTRY_VERSION,
};
pub use registry::{families as feature_families, list as list_features, resolve as resolve_feature, CostClass, Family, Phase};
pub use rsi::{Rsi, RSI_FEATURE_VERSION};
pub use training_frame::{
    align_higher_tf, build_aligned_training_frame, build_training_frame, devol, fit_sigma, label_horizon_bars, HigherTfBar,
    OhlcvRow, TrainingFrame,
};
pub use walk_forward::{walk_forward_folds, Fold, FoldError};

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
