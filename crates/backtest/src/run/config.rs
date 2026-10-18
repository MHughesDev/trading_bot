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
// Independent protections, each recorded on its own: folding them into an enum
// would lose that two were disabled at once.
#[allow(clippy::struct_excessive_bools)]
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
    /// Models answered without the point-in-time rule (`optimistic_models`,
    /// Set U U-5.4): a version could answer for decisions inside the data it
    /// was trained on. Skipped from the hash when off, so every config written
    /// before this flag existed keeps its `run_id`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub models_optimistic: bool,
}

impl UnsafeFlags {
    /// True if any protection was disabled.
    #[must_use]
    pub fn any(self) -> bool {
        self.costs_disabled
            || self.counter_disabled
            || self.holdout_unlocked
            || self.models_optimistic
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
    /// Present iff this Run executes in a **null world**: the bars are passed
    /// through the named null generator before the strategy sees them (spec
    /// §2.1). It is part of the content hash, so a null-world Run can never
    /// share a `run_id` with the real Run it is the null for, and a stored
    /// result always says which world produced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null_world: Option<NullWorld>,
    /// Present iff this Run backtests a trading strategy **deployed on a
    /// selection** (Set U U-6.5): a portfolio run, one account across every
    /// member over time. Part of the content hash — and absent from it when
    /// `None`, so every Run that existed before it keeps its `run_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<domain::strategy_graph::deployment::DeploymentTarget>,
    /// Bars every decided order waits before it reaches the venue — what a
    /// timing-jitter sensitivity study varies. 0, the default, is the
    /// platform's own timing; part of the content hash only when set, so every
    /// Run that existed before it keeps its `run_id`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fill_delay_bars: u32,
    /// The Run's trading costs, scaled by this many basis points of themselves,
    /// hundredths of a percent (`+2000` is +20 %, `-2000` is -20 %). What a
    /// cost-perturbation replicate varies (SPEC §11.5's nuisance product). 0, the
    /// default, is the modelled cost; part of the content hash only when set, so
    /// every Run that existed before it keeps its `run_id`.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub cost_scale_bp: i32,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_i32(n: &i32) -> bool {
    *n == 0
}

/// The null this Run's data was generated under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NullWorld {
    /// The null object, carrying its own `preserves`/`destroys` hypothesis.
    pub null: crate::nulls::Null,
    /// The draw. Members of a permutation-null Study vary exactly this.
    pub seed: u64,
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
            null_world: &'a Option<NullWorld>,
            #[serde(skip_serializing_if = "Option::is_none")]
            target: &'a Option<domain::strategy_graph::deployment::DeploymentTarget>,
            #[serde(skip_serializing_if = "is_zero")]
            fill_delay_bars: &'a u32,
            #[serde(skip_serializing_if = "is_zero_i32")]
            cost_scale_bp: &'a i32,
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
            null_world: &self.null_world,
            target: &self.target,
            fill_delay_bars: &self.fill_delay_bars,
            cost_scale_bp: &self.cost_scale_bp,
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

    /// Derive the same Run **in a null world** (spec §2.1).
    ///
    /// The result is a different Run with a different `run_id`, because the data
    /// it sees is different data. That is the whole point: a null-world result
    /// must never be mistakable for the real one, and a p-value computed against
    /// a "null distribution" whose members are the real Run under different
    /// seeds is not a p-value at all.
    #[must_use]
    pub fn in_null_world(mut self, null: crate::nulls::Null, seed: u64) -> Self {
        self.seed = seed;
        self.null_world = Some(NullWorld { null, seed });
        self.rehashed()
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
    target: Option<domain::strategy_graph::deployment::DeploymentTarget>,
    fill_delay_bars: u32,
    cost_scale_bp: i32,
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
            target: None,
            fill_delay_bars: 0,
            cost_scale_bp: 0,
        }
    }

    /// Backtest the strategy deployed on a target (Set U U-6.5). A selection
    /// target makes this a portfolio run.
    #[must_use]
    pub fn target(
        mut self,
        target: Option<domain::strategy_graph::deployment::DeploymentTarget>,
    ) -> Self {
        self.target = target;
        self
    }

    /// Hold every decided order `bars` bars before it reaches the venue.
    #[must_use]
    pub const fn fill_delay_bars(mut self, bars: u32) -> Self {
        self.fill_delay_bars = bars;
        self
    }

    /// Scale the Run's trading costs by `bp` hundredths of a percent
    /// (`+2000` = +20 %).
    #[must_use]
    pub const fn cost_scale_bp(mut self, bp: i32) -> Self {
        self.cost_scale_bp = bp;
        self
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
        self.unsafe_flags.models_optimistic |= flags.models_optimistic;
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
            // The builder builds real-world Runs. A null world is applied by
            // `in_null_world`, which is how a null member is derived from the
            // config it is the null for.
            null_world: None,
            target: self.target,
            fill_delay_bars: self.fill_delay_bars,
            cost_scale_bp: self.cost_scale_bp,
        };
        cfg.run_id = cfg.compute_id();
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set U U-6.5: a selection target is part of a Run's identity — and its
    /// absence is not, so every Run that existed before targets keeps its id.
    #[test]
    fn a_target_changes_the_run_id_and_no_target_changes_nothing() {
        use domain::strategy_graph::deployment::DeploymentTarget;
        let slice = || {
            DataSlice::new(
                "selection:abc",
                chrono::Utc::now() - chrono::Duration::days(30),
                chrono::Utc::now(),
                EvalResolution::Day1,
            )
        };
        let plain = RunConfigBuilder::new("s", "v", slice(), "c", "z", "snap").build();
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("target"), "{json}");
        let target: DeploymentTarget = serde_json::from_value(
            serde_json::json!({"kind": "selection", "selection": "abc", "top_n": 5}),
        )
        .unwrap();
        let mut aimed = plain.clone();
        aimed.target = Some(target);
        let aimed = aimed.rehashed();
        assert_ne!(aimed.run_id, plain.run_id);
        assert!(aimed.id_is_valid());
        let mut back = aimed.clone();
        back.target = None;
        assert_eq!(
            back.rehashed().run_id,
            plain.run_id,
            "absence hashes as before"
        );
    }
    #[test]
    fn a_cost_scale_is_a_different_run_and_its_absence_hashes_as_before() {
        let plain = RunConfigBuilder::new("s", "v", slice(), "c", "z", "snap").build();
        assert!(!serde_json::to_string(&plain).unwrap().contains("cost_scale"));
        let dearer = RunConfigBuilder::new("s", "v", slice(), "c", "z", "snap")
            .cost_scale_bp(2000)
            .build();
        let cheaper = RunConfigBuilder::new("s", "v", slice(), "c", "z", "snap")
            .cost_scale_bp(-2000)
            .build();
        assert_ne!(dearer.run_id, plain.run_id);
        assert_ne!(cheaper.run_id, dearer.run_id);
        assert!(dearer.id_is_valid());
        let mut back = dearer.clone();
        back.cost_scale_bp = 0;
        assert_eq!(back.rehashed().run_id, plain.run_id);
    }

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

    /// Set U U-2.2: the simulated venue's fill policy is part of the run's
    /// identity, so a run filled at the decision bar's close (optimistic) can
    /// never share an id — or a cached result — with one filled at the next open.
    #[test]
    fn two_runs_with_different_fill_policies_get_different_ids() {
        let next_open = builder().fill_model(FillModel::NextBarOpen).build();
        let current_close = builder().fill_model(FillModel::CurrentClose).build();
        assert_ne!(next_open.run_id, current_close.run_id);
        assert!(next_open.id_is_valid() && current_close.id_is_valid());
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
