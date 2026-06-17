//! [`RunConfig`] — the full, reproducible input to one Run (spec §1.1).
//!
//! A `RunConfig` is built through [`RunConfigBuilder`] so its `run_id` is always
//! the computed content hash of every other field — never a caller-supplied
//! value. Identical configs collide on `run_id` (cache hit); any field change is
//! a new id. INV-1 (skeptical defaults) is enforced here: a config is `unsafe`
//! only if a default protection was explicitly disabled, and that bit never
//! clears.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::id::{canonical_json, RunId};

/// The eval-resolution lattice. The base resolution is always 1m (spec §1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalResolution {
    #[serde(rename = "1m")]
    Min1,
    #[serde(rename = "5m")]
    Min5,
    #[serde(rename = "10m")]
    Min10,
    #[serde(rename = "15m")]
    Min15,
    #[serde(rename = "30m")]
    Min30,
    #[serde(rename = "1h")]
    Hour1,
    #[serde(rename = "1d")]
    Day1,
}

/// How higher-timeframe bars are constructed. The only honest construction is
/// `close_stamped`: a bar is complete (and addressable) only at its close
/// (Gate 0 enforces this — spec §2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Construction {
    CloseStamped,
}

/// How orders fill against bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillModel {
    NextBarOpen,
    CurrentClose,
    LimitProb,
    PessimisticIntrabar,
}

/// A pinned data slice: symbol set, window, base/eval resolution, construction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSlice {
    /// Symbol-set reference, pinned by membership-calendar version.
    pub universe_ref: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Always 1m — the foundation resolution. Serialized for transparency.
    #[serde(default = "base_resolution_1m")]
    pub base_resolution: String,
    pub eval_resolution: EvalResolution,
    pub construction: Construction,
}

fn base_resolution_1m() -> String {
    "1m".to_string()
}

impl DataSlice {
    /// Build a 1m-founded, close-stamped slice over `[start, end)`.
    #[must_use]
    pub fn new(
        universe_ref: impl Into<String>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        eval_resolution: EvalResolution,
    ) -> Self {
        Self {
            universe_ref: universe_ref.into(),
            start,
            end,
            base_resolution: base_resolution_1m(),
            eval_resolution,
            construction: Construction::CloseStamped,
        }
    }

    /// True if this slice's window overlaps `other` (used by the holdout-vault
    /// guard — a research Study may not address the locked tail).
    #[must_use]
    pub fn overlaps(&self, other: &DataSlice) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Which default protections (if any) were disabled. All `false` by default
/// (INV-1). Any `true` makes the config permanently `unsafe`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsafeFlags {
    /// Costs were zeroed/removed (the cost floor disabled).
    #[serde(default)]
    pub costs_disabled: bool,
    /// The global trial counter was bypassed.
    #[serde(default)]
    pub counter_disabled: bool,
    /// The holdout lock was opened outside the vault path.
    #[serde(default)]
    pub holdout_unlocked: bool,
}

impl UnsafeFlags {
    /// True if any protection was disabled.
    #[must_use]
    pub fn any(self) -> bool {
        self.costs_disabled || self.counter_disabled || self.holdout_unlocked
    }
}

/// A parameter map. `BTreeMap` keeps keys in a canonical order so the `run_id`
/// hash is insensitive to insertion order.
pub type ParamMap = BTreeMap<String, serde_json::Value>;

/// The full reproducible input to one Run (spec §1.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    /// Deterministic content hash of every other field. Set by the builder.
    pub run_id: RunId,
    /// Strategy reference (slug/id), pinned by `strategy_version`.
    pub strategy_ref: String,
    /// Exact strategy code/definition version hash.
    pub strategy_version: String,
    /// The specific parameter set for THIS run.
    pub params: ParamMap,
    pub data_slice: DataSlice,
    /// Commission/slippage/spread/borrow/latency profile reference.
    pub cost_model_ref: String,
    pub fill_model: FillModel,
    pub sizing_ref: String,
    /// Controls all stochastic elements in THIS run.
    pub seed: u64,
    /// Point-in-time data version; guarantees reproducibility.
    pub data_snapshot: String,
    /// True if any default protection was disabled (INV-1). Never clears.
    #[serde(default)]
    pub unsafe_: bool,
    /// Which protection(s) were disabled.
    #[serde(default)]
    pub unsafe_flags: UnsafeFlags,
}

impl RunConfig {
    /// Recompute the content hash of this config's fields (excluding `run_id`).
    /// The builder calls this; exposed for verification/tests.
    #[must_use]
    pub fn compute_id(&self) -> RunId {
        // A borrow-view of every field EXCEPT run_id, in a stable order.
        #[derive(Serialize)]
        struct Hashable<'a> {
            strategy_ref: &'a str,
            strategy_version: &'a str,
            params: &'a ParamMap,
            data_slice: &'a DataSlice,
            cost_model_ref: &'a str,
            fill_model: &'a FillModel,
            sizing_ref: &'a str,
            seed: u64,
            data_snapshot: &'a str,
            unsafe_: bool,
            unsafe_flags: &'a UnsafeFlags,
        }
        let view = Hashable {
            strategy_ref: &self.strategy_ref,
            strategy_version: &self.strategy_version,
            params: &self.params,
            data_slice: &self.data_slice,
            cost_model_ref: &self.cost_model_ref,
            fill_model: &self.fill_model,
            sizing_ref: &self.sizing_ref,
            seed: self.seed,
            data_snapshot: &self.data_snapshot,
            unsafe_: self.unsafe_,
            unsafe_flags: &self.unsafe_flags,
        };
        let bytes = canonical_json(&view).expect("RunConfig is always JSON-serializable");
        RunId::from_canonical_bytes(&bytes)
    }

    /// True iff `run_id` matches the hash of the current fields. A stored config
    /// whose id no longer matches has been tampered with.
    #[must_use]
    pub fn id_is_valid(&self) -> bool {
        self.run_id == self.compute_id()
    }

    /// Recompute `run_id` after mutating fields. Used by Studies (Phase 1) when
    /// deriving a varied member config from a base config — the derived config
    /// is a *new* Run with its own content-addressed id.
    #[must_use]
    pub fn rehashed(mut self) -> Self {
        self.run_id = self.compute_id();
        self
    }
}

/// Builder that guarantees `run_id` is the computed hash (J-0.2/J-0.3) and that
/// `unsafe` is set whenever a protection is disabled (J-0.9).
#[derive(Clone, Debug)]
pub struct RunConfigBuilder {
    strategy_ref: String,
    strategy_version: String,
    params: ParamMap,
    data_slice: DataSlice,
    cost_model_ref: String,
    fill_model: FillModel,
    sizing_ref: String,
    seed: u64,
    data_snapshot: String,
    unsafe_flags: UnsafeFlags,
}

impl RunConfigBuilder {
    /// Start a config with skeptical defaults (INV-1): real costs, counter and
    /// holdout lock on, `unsafe = false`.
    #[must_use]
    pub fn new(
        strategy_ref: impl Into<String>,
        strategy_version: impl Into<String>,
        data_slice: DataSlice,
        cost_model_ref: impl Into<String>,
        sizing_ref: impl Into<String>,
        data_snapshot: impl Into<String>,
    ) -> Self {
        Self {
            strategy_ref: strategy_ref.into(),
            strategy_version: strategy_version.into(),
            params: ParamMap::new(),
            data_slice,
            cost_model_ref: cost_model_ref.into(),
            fill_model: FillModel::NextBarOpen,
            sizing_ref: sizing_ref.into(),
            seed: 0,
            data_snapshot: data_snapshot.into(),
            unsafe_flags: UnsafeFlags::default(),
        }
    }

    /// Set the parameter map.
    #[must_use]
    pub fn params(mut self, params: ParamMap) -> Self {
        self.params = params;
        self
    }

    /// Set the fill model.
    #[must_use]
    pub fn fill_model(mut self, fill_model: FillModel) -> Self {
        self.fill_model = fill_model;
        self
    }

    /// Set the run seed.
    #[must_use]
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Disable a default protection. This is the *only* way to make a config
    /// `unsafe`, and it can never be undone on the resulting config.
    #[must_use]
    pub fn disable_protection(mut self, flags: UnsafeFlags) -> Self {
        self.unsafe_flags.costs_disabled |= flags.costs_disabled;
        self.unsafe_flags.counter_disabled |= flags.counter_disabled;
        self.unsafe_flags.holdout_unlocked |= flags.holdout_unlocked;
        self
    }

    /// Finalize: compute `run_id` over every field and derive `unsafe`.
    #[must_use]
    pub fn build(self) -> RunConfig {
        let unsafe_ = self.unsafe_flags.any();
        let mut cfg = RunConfig {
            run_id: RunId::from_canonical_bytes(b""), // placeholder, replaced below
            strategy_ref: self.strategy_ref,
            strategy_version: self.strategy_version,
            params: self.params,
            data_slice: self.data_slice,
            cost_model_ref: self.cost_model_ref,
            fill_model: self.fill_model,
            sizing_ref: self.sizing_ref,
            seed: self.seed,
            data_snapshot: self.data_snapshot,
            unsafe_,
            unsafe_flags: self.unsafe_flags,
        };
        cfg.run_id = cfg.compute_id();
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn slice() -> DataSlice {
        DataSlice::new(
            "univ:btc@v1",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap(),
            EvalResolution::Hour1,
        )
    }

    fn builder() -> RunConfigBuilder {
        RunConfigBuilder::new(
            "ema_cross",
            "v-abc",
            slice(),
            "cost:floor",
            "sizing:fixed",
            "snap:1",
        )
    }

    #[test]
    fn round_trips_serde() {
        let cfg = builder().build();
        let s = serde_json::to_string(&cfg).unwrap();
        let back: RunConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn builder_computes_a_valid_id() {
        let cfg = builder().build();
        assert!(cfg.id_is_valid());
    }

    #[test]
    fn identical_configs_collide() {
        let a = builder().seed(7).build();
        let b = builder().seed(7).build();
        assert_eq!(a.run_id, b.run_id);
    }

    #[test]
    fn any_field_change_is_a_new_id() {
        let base = builder().seed(7).build();
        assert_ne!(base.run_id, builder().seed(8).build().run_id);
        assert_ne!(
            base.run_id,
            builder()
                .fill_model(FillModel::PessimisticIntrabar)
                .build()
                .run_id
        );
        let mut p = ParamMap::new();
        p.insert("fast".into(), json!(12));
        assert_ne!(base.run_id, builder().seed(7).params(p).build().run_id);
    }

    #[test]
    fn param_key_order_irrelevant_to_id() {
        let mut p1 = ParamMap::new();
        p1.insert("fast".into(), json!(12));
        p1.insert("slow".into(), json!(26));
        // BTreeMap normalizes order, but build two from differently-ordered
        // inserts to prove the contract end-to-end.
        let mut p2 = ParamMap::new();
        p2.insert("slow".into(), json!(26));
        p2.insert("fast".into(), json!(12));
        assert_eq!(
            builder().params(p1).build().run_id,
            builder().params(p2).build().run_id
        );
    }

    #[test]
    fn default_config_is_safe() {
        let cfg = builder().build();
        assert!(!cfg.unsafe_);
        assert!(!cfg.unsafe_flags.any());
    }

    #[test]
    fn disabling_costs_flags_unsafe_and_survives_round_trip() {
        let cfg = builder()
            .disable_protection(UnsafeFlags {
                costs_disabled: true,
                ..Default::default()
            })
            .build();
        assert!(cfg.unsafe_);
        let back: RunConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(back.unsafe_);
        assert!(back.unsafe_flags.costs_disabled);
    }

    #[test]
    fn unsafe_changes_the_id() {
        let safe = builder().build();
        let unsafe_cfg = builder()
            .disable_protection(UnsafeFlags {
                counter_disabled: true,
                ..Default::default()
            })
            .build();
        assert_ne!(safe.run_id, unsafe_cfg.run_id);
    }

    #[test]
    fn slice_overlap_detection() {
        let a = slice();
        let mut b = slice();
        b.start = Utc.with_ymd_and_hms(2024, 5, 1, 0, 0, 0).unwrap();
        b.end = Utc.with_ymd_and_hms(2024, 8, 1, 0, 0, 0).unwrap();
        assert!(a.overlaps(&b));
        let mut c = slice();
        c.start = Utc.with_ymd_and_hms(2024, 7, 1, 0, 0, 0).unwrap();
        c.end = Utc.with_ymd_and_hms(2024, 9, 1, 0, 0, 0).unwrap();
        assert!(!a.overlaps(&c));
    }
}
