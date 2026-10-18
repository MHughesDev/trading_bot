//! The dataset an agent *declares* (Set N, FEAT-006 §3).
//!
//! There are two things called `DatasetSpec`, and this file is the one that is not
//! `dataplane::dataset::DatasetSpec`. That one is the **resolved** spec the ML pack
//! hashes into a `dataset_id` (surrogate instrument ids, the point-in-time anchor,
//! the quality mask, the feature-DAG hash, the image digest). This one is what a
//! training run **declares** before any of that is resolved: universe, features,
//! transforms, sampling, target, weights, an optional sequence window and the
//! cross-validation scheme. It validates, hashes canonically and *lowers* to a
//! [`DatasetRequest`], and [`DatasetManager::plan`](crate::datasets::DatasetManager::plan)
//! resolves that into the existing spec. Nothing here re-implements the resolved
//! spec or the `dataset_id` (ADR-N-01).
//!
//! # Purge and embargo are derived, and the type has nowhere to put one
//!
//! INV-12 and INV-15: a fold boundary is a consequence of the label horizon and the
//! feature look-back, not a number someone types. [`DatasetDeclaration`] has no
//! `purge` or `embargo` field and `deny_unknown_fields` makes a request that sends
//! one an error. [`Derived`] computes both:
//!
//! * **purge** is the label horizon (purge on the label's *end*);
//! * **embargo** is `max(dataplane::split::compute_embargo, ceil(1% of the sample))`
//!   — the ML pack's pipeline-derived figure and FEAT-006's 1% floor, whichever is
//!   larger — and for bars of five minutes or finer, at least 24 hours of bars
//!   (ADR-N-03), because at that resolution 1% of a short sample is inside the
//!   autocorrelation time of the returns.
//!
//! The look-back is *estimated conservatively* from the declaration: the largest of
//! the declared lags and every integer argument in a feature expression, plus the
//! sequence window. Over-estimating lengthens the embargo, which is the safe error.
//!
//! # No silent defaults
//!
//! `weights` is required: ADR-P2-22 made sample weighting a declared choice, and a
//! default of `none` would reintroduce the thing that decision removed.

use chrono::{DateTime, Utc};
use dataplane::label::{LabelKind, LabelSpec, SampleWeightMethod};
use dataplane::split::{compute_embargo, EmbargoInputs, SplitKind, SplitSpec, TrainWindow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::datasets::DatasetRequest;

// ── the declaration ──────────────────────────────────────────────────────────

/// What the agent writes. Every field is explicit except the ones with a stated default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetDeclaration {
    pub universe: Universe,
    /// `1m | 5m | 15m | 30m | 1h | 4h | 1d`.
    pub timeframe: String,
    pub window: Window,
    pub features: Vec<String>,
    #[serde(default)]
    pub transforms: Transforms,
    #[serde(default)]
    pub sampling: Sampling,
    pub target: Target,
    /// Required: there is no default weighting.
    pub weights: Weights,
    #[serde(default)]
    pub sequence: Option<Sequence>,
    pub cv: Cv,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Universe {
    List(Vec<String>),
    Named {
        universe: String,
        #[serde(default)]
        as_of_membership: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transforms {
    /// Winsorise at this tail fraction, in `(0, 0.5)`.
    #[serde(default)]
    pub winsorize: Option<f64>,
    #[serde(default)]
    pub scaler: Scaler,
    /// Lagged copies of each feature, in bars.
    #[serde(default)]
    pub lags: Vec<u32>,
}

/// Fitted per fold, never on the whole sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scaler {
    #[default]
    None,
    PerFoldStandard,
    PerFoldRobust,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Sampling {
    #[default]
    EveryBar,
    /// A symmetric CUSUM filter with threshold `h` in multiples of sigma.
    Cusum { h_sigma: f64 },
    /// Only the bars at which a strategy version would have entered.
    StrategyEvents { strategy_version: String },
}

/// Where a triple barrier looks for a touch: the dataplane's own type, so the declaration and
/// the labeller cannot drift (N-1.4).
pub use dataplane::barrier::BarrierPath;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    ForwardReturn {
        horizon_bars: u32,
        #[serde(default)]
        log: bool,
    },
    RealizedVol {
        horizon_bars: u32,
    },
    TripleBarrier {
        pt_sigma: f64,
        sl_sigma: f64,
        max_bars: u32,
        #[serde(default)]
        path: BarrierPath,
    },
    MetaLabel {
        strategy_version: String,
        pt_sigma: f64,
        sl_sigma: f64,
        max_bars: u32,
        #[serde(default)]
        path: BarrierPath,
    },
    Rank {
        horizon_bars: u32,
    },
    Direction {
        horizon_bars: u32,
    },
}

impl Target {
    /// The label's horizon in bars: how far its end is from its start.
    #[must_use]
    pub const fn horizon_bars(&self) -> u32 {
        match self {
            Self::ForwardReturn { horizon_bars, .. }
            | Self::RealizedVol { horizon_bars }
            | Self::Rank { horizon_bars }
            | Self::Direction { horizon_bars } => *horizon_bars,
            Self::TripleBarrier { max_bars, .. } | Self::MetaLabel { max_bars, .. } => *max_bars,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Weights {
    Uniqueness,
    TimeDecay { half_life_days: f64 },
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequence {
    pub window: u32,
    pub stride: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalkMode {
    Expanding,
    Rolling,
}

/// Purge and embargo are not here. See the module docs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Cv {
    WalkForward {
        train_days: u32,
        test_days: u32,
        refit_days: u32,
        mode: WalkMode,
    },
    PurgedKfold {
        k: u32,
    },
    Cpcv {
        n: u32,
        k: u32,
    },
}

// ── validation ───────────────────────────────────────────────────────────────

/// A refusal naming the field.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {reason}")]
pub struct DeclarationError {
    pub field: String,
    pub reason: String,
}

fn bad(field: &str, reason: impl Into<String>) -> DeclarationError {
    DeclarationError {
        field: field.into(),
        reason: reason.into(),
    }
}

/// Bars in a day for a supported timeframe.
#[must_use]
pub fn bars_per_day(timeframe: &str) -> Option<u32> {
    Some(match timeframe {
        "1m" => 1440,
        "5m" => 288,
        "15m" => 96,
        "30m" => 48,
        "1h" => 24,
        "4h" => 6,
        "1d" => 1,
        _ => return None,
    })
}

/// What the declaration implies, computed and never typed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Derived {
    pub horizon_bars: u32,
    /// Purge on the label's end: the horizon.
    pub purge_bars: u32,
    pub embargo_bars: u32,
    /// The pipeline-derived figure before the 1% and fine-resolution floors.
    pub computed_embargo_bars: u32,
    pub max_lookback_bars: u32,
    pub expected_bars: u64,
    /// Which floor raised the embargo above the computed figure, if one did.
    pub floor: Option<&'static str>,
}

/// A declaration that passed every check, with what was derived from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Validated {
    /// With the window clipped to the cutoff and a listed universe sorted and de-duplicated.
    pub declaration: DatasetDeclaration,
    /// True when the declared end was after the project cutoff and was clipped to it.
    pub clipped_to_cutoff: bool,
    pub derived: Derived,
    /// `sha256` of the canonical declaration. The resolved `dataset_id` is a different thing.
    pub declaration_hash: String,
}

/// Every integer argument appearing in a feature expression: `ema(close,21)` gives 21.
/// An over-estimate of the look-back, which can only lengthen the embargo.
fn expression_lookback(expr: &str) -> u32 {
    let mut best = 0u32;
    let mut cur = String::new();
    for c in expr.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            // A digit run inside an identifier (`art_9f3`) is not a window.
            if let Ok(n) = cur.parse::<u32>() {
                best = best.max(n);
            }
            cur.clear();
        }
    }
    best
}

impl DatasetDeclaration {
    /// Validate, clip to the project cutoff, canonicalise and derive.
    ///
    /// # Errors
    /// The first violated rule, naming its field.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self, cutoff: Option<DateTime<Utc>>) -> Result<Validated, DeclarationError> {
        let mut d = self.clone();

        match &mut d.universe {
            Universe::List(v) => {
                if v.is_empty() || v.iter().any(|s| s.trim().is_empty()) {
                    return Err(bad("universe", "name at least one instrument, none blank"));
                }
                v.sort();
                v.dedup();
            }
            Universe::Named { universe, .. } => {
                if universe.trim().is_empty() {
                    return Err(bad("universe", "a named universe has a name"));
                }
            }
        }
        let Some(per_day) = bars_per_day(&d.timeframe) else {
            return Err(bad(
                "timeframe",
                format!(
                    "`{}` is not one of 1m, 5m, 15m, 30m, 1h, 4h, 1d",
                    d.timeframe
                ),
            ));
        };
        if d.window.start >= d.window.end {
            return Err(bad("window", "start must be before end"));
        }
        let mut clipped = false;
        if let Some(c) = cutoff {
            if d.window.start >= c {
                return Err(bad(
                    "window",
                    "the window starts at or after the project cutoff",
                ));
            }
            if d.window.end > c {
                d.window.end = c;
                clipped = true;
            }
        }
        if d.features.is_empty() || d.features.iter().any(|f| f.trim().is_empty()) {
            return Err(bad("features", "declare at least one feature, none blank"));
        }
        if let Some(w) = d.transforms.winsorize {
            if !(w > 0.0 && w < 0.5) {
                return Err(bad("transforms.winsorize", "a tail fraction in (0, 0.5)"));
            }
        }
        {
            let l = &d.transforms.lags;
            if l.contains(&0) || l.windows(2).any(|w| w[0] >= w[1]) {
                return Err(bad("transforms.lags", "positive, strictly increasing lags"));
            }
        }
        match &d.sampling {
            Sampling::Cusum { h_sigma } if !(h_sigma.is_finite() && *h_sigma > 0.0) => {
                return Err(bad("sampling.h_sigma", "a positive threshold"));
            }
            Sampling::StrategyEvents { strategy_version } if strategy_version.trim().is_empty() => {
                return Err(bad(
                    "sampling.strategy_version",
                    "name the strategy version",
                ));
            }
            _ => {}
        }
        let barrier_ok = |pt: f64, sl: f64, max_bars: u32, field: &str| {
            if !(pt.is_finite() && pt > 0.0 && sl.is_finite() && sl > 0.0) {
                return Err(bad(field, "pt_sigma and sl_sigma must be positive"));
            }
            if max_bars == 0 {
                return Err(bad(field, "max_bars must be at least 1"));
            }
            Ok(())
        };
        match &d.target {
            Target::TripleBarrier {
                pt_sigma,
                sl_sigma,
                max_bars,
                ..
            } => barrier_ok(*pt_sigma, *sl_sigma, *max_bars, "target")?,
            Target::MetaLabel {
                strategy_version,
                pt_sigma,
                sl_sigma,
                max_bars,
                ..
            } => {
                barrier_ok(*pt_sigma, *sl_sigma, *max_bars, "target")?;
                if strategy_version.trim().is_empty() {
                    return Err(bad(
                        "target.strategy_version",
                        "a meta-label names its primary strategy",
                    ));
                }
                if !matches!(d.sampling, Sampling::StrategyEvents { .. }) {
                    return Err(bad(
                        "sampling",
                        "a meta_label target labels the primary strategy's events, so sampling must be strategy_events",
                    ));
                }
            }
            t => {
                if t.horizon_bars() == 0 {
                    return Err(bad("target.horizon_bars", "at least 1"));
                }
            }
        }
        if let Weights::TimeDecay { half_life_days } = d.weights {
            if !(half_life_days.is_finite() && half_life_days > 0.0) {
                return Err(bad("weights.half_life_days", "a positive half-life"));
            }
        }
        match d.cv {
            Cv::WalkForward {
                train_days,
                test_days,
                refit_days,
                ..
            } if train_days == 0 || test_days == 0 || refit_days == 0 => {
                return Err(bad("cv", "train, test and refit days must all be positive"));
            }
            Cv::PurgedKfold { k } if k < 2 => return Err(bad("cv.k", "at least 2 folds")),
            Cv::Cpcv { n, k } if !(k >= 1 && n > k) => {
                return Err(bad("cv", "cpcv needs 1 <= k < n"));
            }
            _ => {}
        }

        let span_days = (d.window.end - d.window.start).num_days().max(0) as u64;
        let expected_bars = span_days * u64::from(per_day);
        if let Cv::WalkForward {
            train_days,
            test_days,
            ..
        } = d.cv
        {
            if span_days < u64::from(train_days) + u64::from(test_days) {
                return Err(bad(
                    "cv",
                    format!(
                        "the window is {span_days} days and one train plus one test is {}",
                        train_days + test_days
                    ),
                ));
            }
        }
        if let Some(s) = d.sequence {
            if s.window == 0 || s.stride == 0 {
                return Err(bad("sequence", "window and stride must be positive"));
            }
            if let Cv::WalkForward { train_days, .. } = d.cv {
                let train_bars = u64::from(train_days) * u64::from(per_day);
                if u64::from(s.window) > train_bars {
                    return Err(bad(
                        "sequence.window",
                        format!(
                            "a window of {} bars is longer than a training span of {train_bars}",
                            s.window
                        ),
                    ));
                }
            }
        }

        // Derived: never typed.
        let horizon = d.target.horizon_bars();
        let lookback = d
            .features
            .iter()
            .map(|f| expression_lookback(f))
            .chain(d.transforms.lags.iter().copied())
            .max()
            .unwrap_or(0)
            .max(d.sequence.map_or(0, |s| s.window.saturating_sub(1)));
        let computed = compute_embargo(&EmbargoInputs {
            horizon_bars: horizon,
            max_lookback_bars: lookback,
            max_knowledge_lag_ms: 0,
            settlement_lag_bars: 0,
        });
        let one_pct = (expected_bars.div_ceil(100)).min(u64::from(u32::MAX)) as u32;
        let fine_floor = if per_day >= 288 { per_day } else { 0 };
        let mut embargo = computed;
        let mut floor = None;
        if one_pct > embargo {
            embargo = one_pct;
            floor = Some("one_percent_of_sample");
        }
        if fine_floor > embargo {
            embargo = fine_floor;
            floor = Some("fine_resolution_24h");
        }
        let derived = Derived {
            horizon_bars: horizon,
            purge_bars: horizon,
            embargo_bars: embargo,
            computed_embargo_bars: computed,
            max_lookback_bars: lookback,
            expected_bars,
            floor,
        };

        let declaration_hash =
            hex_sha256(&serde_json::to_vec(&d).expect("a declaration is JSON-representable"));
        Ok(Validated {
            declaration: d,
            clipped_to_cutoff: clipped,
            derived,
            declaration_hash,
        })
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

// ── specification counting (N-1.6) ───────────────────────────────────────────

/// How many specifications `child` adds relative to `parent`.
///
/// Every feature, lag or transform added relative to the parent is one more thing
/// the data was asked, and BACKTEST_SUITE v2 §3 counts it against the owning
/// Experiment. **Removing one never lowers the count**: a specification that was
/// tried and dropped was still tried. The function is therefore a count of
/// additions only, and a child that only removes returns zero.
#[must_use]
pub fn specification_delta(parent: &DatasetDeclaration, child: &DatasetDeclaration) -> u32 {
    let mut n = 0;
    for f in &child.features {
        if !parent.features.contains(f) {
            n += 1;
        }
    }
    for l in &child.transforms.lags {
        if !parent.transforms.lags.contains(l) {
            n += 1;
        }
    }
    if child.transforms.winsorize != parent.transforms.winsorize
        && child.transforms.winsorize.is_some()
    {
        n += 1;
    }
    if child.transforms.scaler != parent.transforms.scaler
        && child.transforms.scaler != Scaler::None
    {
        n += 1;
    }
    if child.sampling != parent.sampling && child.sampling != Sampling::EveryBar {
        n += 1;
    }
    n
}

// ── lowering ─────────────────────────────────────────────────────────────────

/// A field the existing dataset planner cannot yet honour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub field: &'static str,
    pub why: String,
}

/// What the lowering needs that a declaration does not carry.
#[derive(Clone, Debug)]
pub struct LoweringContext {
    pub universe_spec_id: String,
    pub output_prefix: String,
}

/// The prefix that names an existing, versioned feature set: `set:<ref>`.
pub const FEATURE_SET_PREFIX: &str = "set:";

/// Declared fields the *trainer* executes after the planner has materialised the rows,
/// so the planner does not need to (and a declaration using them is not "unsupported").
///
/// [`Validated::lower`] assumes none; the training path uses [`Validated::lower_for_trainer`],
/// which passes [`TrainerExecutes::all`] and returns the names of the fields it handed over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // one flag per executed feature; a capability set, not a state machine
pub struct TrainerExecutes {
    /// Sequence windows are built from rows by the trainer (N-2.5).
    pub sequence: bool,
    /// `cusum` and `strategy_events` sampling subsample the rows (N-1.7).
    pub sampling: bool,
    /// Winsorise, scaler and lags are applied to the feature columns.
    pub transforms: bool,
    /// `direction` and `rank` targets are functions of the horizon-return label.
    pub target_transforms: bool,
}

impl TrainerExecutes {
    /// What the trainer in `apps/model-trainer` implements today.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            sequence: true,
            sampling: true,
            transforms: true,
            target_transforms: true,
        }
    }
}

/// A lowered declaration: the planner's request and what was handed to the trainer.
#[derive(Clone, Debug)]
pub struct Lowered {
    pub request: DatasetRequest,
    /// Declaration fields the trainer executes, by name (`sequence`, `sampling`, `transforms`, `target`).
    pub trainer_fields: Vec<&'static str>,
}

impl Validated {
    /// Lower to a [`DatasetRequest`] for the fields the planner supports, and say
    /// so — naming each field — for the ones it does not, rather than dropping them.
    ///
    /// # Errors
    /// Every unsupported field, all at once, so a caller fixes them in one pass.
    pub fn lower(&self, ctx: &LoweringContext) -> Result<DatasetRequest, Vec<Unsupported>> {
        self.lower_with(ctx, TrainerExecutes::default())
            .map(|l| l.request)
    }

    /// [`Self::lower`] for the training path: the fields the trainer executes are handed
    /// over (and named in `trainer_fields`) instead of refused.
    ///
    /// # Errors
    /// Every field neither the planner nor the trainer can honour.
    pub fn lower_for_trainer(&self, ctx: &LoweringContext) -> Result<Lowered, Vec<Unsupported>> {
        self.lower_with(ctx, TrainerExecutes::all())
    }

    #[allow(clippy::too_many_lines)]
    fn lower_with(
        &self,
        ctx: &LoweringContext,
        exec: TrainerExecutes,
    ) -> Result<Lowered, Vec<Unsupported>> {
        let d = &self.declaration;
        let mut un: Vec<Unsupported> = Vec::new();
        let mut trainer_fields: Vec<&'static str> = Vec::new();

        let instruments = match &d.universe {
            Universe::List(v) => v.clone(),
            Universe::Named { .. } => {
                un.push(Unsupported {
                    field: "universe",
                    why: "a named universe needs a membership resolution the planner does not do; list the instruments".into(),
                });
                Vec::new()
            }
        };
        let feature_set_ref = if let [one] = d.features.as_slice() {
            one.strip_prefix(FEATURE_SET_PREFIX).map(str::to_string)
        } else {
            None
        };
        if feature_set_ref.is_none() {
            un.push(Unsupported {
                field: "features",
                why: format!(
                    "the planner takes one named feature set (`{FEATURE_SET_PREFIX}<ref>`); free-form expressions need the DATA-006 batch engine"
                ),
            });
        }
        // CUSUM is sampled natively by the dataset build (dataplane::sampling, N-1.7), so it is
        // no longer a refused or trainer-handed field; strategy events need the strategy's
        // entry timestamps, which the trainer resolves.
        let native_sampling = match &d.sampling {
            Sampling::Cusum { h_sigma } => Some(dataplane::sampling::SamplingSpec::Cusum {
                h_sigma: *h_sigma,
            }),
            _ => None,
        };
        if !matches!(d.sampling, Sampling::EveryBar) && native_sampling.is_none() {
            if exec.sampling {
                trainer_fields.push("sampling");
            } else {
                un.push(Unsupported {
                    field: "sampling",
                    why: "only every_bar is built (N-1.7 adds cusum and strategy events)".into(),
                });
            }
        }
        if d.sequence.is_some() {
            if exec.sequence {
                trainer_fields.push("sequence");
            } else {
                un.push(Unsupported {
                    field: "sequence",
                    why: "sequence windows are not materialised by the planner yet (N-2.5)".into(),
                });
            }
        }
        if d.transforms.winsorize.is_some()
            || d.transforms.scaler != Scaler::None
            || !d.transforms.lags.is_empty()
        {
            if exec.transforms {
                trainer_fields.push("transforms");
            } else {
                un.push(Unsupported {
                    field: "transforms",
                    why: "transforms are applied by the feature set, not by the planner".into(),
                });
            }
        }

        let mut barrier_path: Option<BarrierPath> = None;
        let (kind, pt_sl) = match &d.target {
            Target::ForwardReturn { .. } => (Some(LabelKind::HorizonReturn), vec![]),
            Target::TripleBarrier {
                pt_sigma,
                sl_sigma,
                path,
                ..
            } => {
                barrier_path = Some(*path);
                (Some(LabelKind::TripleBarrier), vec![*pt_sigma, *sl_sigma])
            }
            Target::MetaLabel { .. } => (Some(LabelKind::MetaLabel), vec![]),
            // A direction or rank target is a function of the horizon return, which the
            // trainer applies to the `label` column; the planner materialises the return.
            Target::Direction { .. } | Target::Rank { .. } if exec.target_transforms => {
                trainer_fields.push("target");
                (Some(LabelKind::HorizonReturn), vec![])
            }
            other => {
                un.push(Unsupported {
                    field: "target",
                    why: format!("{} labels are not built", target_name(other)),
                });
                (None, vec![])
            }
        };

        let weight = match d.weights {
            Weights::Uniqueness => SampleWeightMethod::Uniqueness,
            Weights::TimeDecay { .. } => SampleWeightMethod::TimeDecay,
            Weights::None => SampleWeightMethod::None,
        };

        if !un.is_empty() {
            return Err(un);
        }
        let label_spec = LabelSpec {
            label_spec_id: String::new(),
            kind: kind.expect("checked above"),
            horizon_bars: self.derived.horizon_bars,
            pt_sl_multiples: pt_sl,
            vol_estimator: None,
            min_return_threshold: None,
            sample_weight_method: weight,
            code_hash: format!("declaration:{}", self.declaration_hash),
            barrier_path,
            sampling: native_sampling,
        }
        .content_keyed();

        let inputs = EmbargoInputs {
            horizon_bars: self.derived.horizon_bars,
            max_lookback_bars: self.derived.max_lookback_bars,
            max_knowledge_lag_ms: 0,
            settlement_lag_bars: 0,
        };
        let per_day = u64::from(bars_per_day(&d.timeframe).expect("validated"));
        let (split_kind, n_folds, n_test_groups, window) = match d.cv {
            Cv::WalkForward {
                train_days,
                test_days,
                mode,
                ..
            } => {
                let span = (d.window.end - d.window.start).num_days().max(0) as u64;
                let folds = (span.saturating_sub(u64::from(train_days)) / u64::from(test_days))
                    .max(1) as usize;
                let tw = match mode {
                    WalkMode::Expanding => TrainWindow::Expanding,
                    WalkMode::Rolling => {
                        TrainWindow::Rolling((u64::from(train_days) * per_day) as usize)
                    }
                };
                (SplitKind::WalkForward, folds, 1, tw)
            }
            Cv::PurgedKfold { k } => (
                SplitKind::PurgedKfold,
                k as usize,
                1,
                TrainWindow::Expanding,
            ),
            Cv::Cpcv { n, k } => (
                SplitKind::Cpcv,
                n as usize,
                k as usize,
                TrainWindow::Expanding,
            ),
        };
        let mut split = SplitSpec::new(String::new(), split_kind, n_folds, n_test_groups, &inputs);
        split.train_window = window;
        // The pipeline-derived embargo is the floor; the 1% and fine-resolution
        // floors only raise it, which `set_embargo` permits without a reason.
        split
            .set_embargo(self.derived.embargo_bars, None)
            .expect("raising an embargo never needs a reason");
        let split = split.content_keyed();

        let request = DatasetRequest {
            universe_spec_id: ctx.universe_spec_id.clone(),
            feature_set_ref: feature_set_ref.expect("checked above"),
            instruments,
            timeframe: d.timeframe.clone(),
            start: d.window.start,
            end: d.window.end,
            label_spec,
            split_spec: split,
            quality_exclusion_mask: crate::datasets::default_exclusion_mask(),
            adjustment_policy: dataplane::corporate::AdjustmentPolicy::Unadjusted,
            output_prefix: ctx.output_prefix.clone(),
        };
        Ok(Lowered {
            request,
            trainer_fields,
        })
    }
}

const fn target_name(t: &Target) -> &'static str {
    match t {
        Target::ForwardReturn { .. } => "forward_return",
        Target::RealizedVol { .. } => "realized_vol",
        Target::TripleBarrier { .. } => "triple_barrier",
        Target::MetaLabel { .. } => "meta_label",
        Target::Rank { .. } => "rank",
        Target::Direction { .. } => "direction",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    fn base() -> DatasetDeclaration {
        DatasetDeclaration {
            universe: Universe::List(vec!["ETH-USD".into(), "BTC-USD".into()]),
            timeframe: "1h".into(),
            window: Window {
                start: day(2021, 1, 1),
                end: day(2026, 3, 31),
            },
            features: vec!["ema(close,21)/close - 1".into(), "yang_zhang(20)".into()],
            transforms: Transforms {
                winsorize: Some(0.01),
                scaler: Scaler::PerFoldStandard,
                lags: vec![1, 2, 4],
            },
            sampling: Sampling::EveryBar,
            target: Target::ForwardReturn {
                horizon_bars: 24,
                log: true,
            },
            weights: Weights::Uniqueness,
            sequence: None,
            cv: Cv::WalkForward {
                train_days: 365,
                test_days: 30,
                refit_days: 30,
                mode: WalkMode::Expanding,
            },
        }
    }

    fn err_field(d: &DatasetDeclaration) -> String {
        d.validate(None).unwrap_err().field
    }

    #[test]
    fn a_good_declaration_validates_and_derives() {
        let v = base().validate(None).unwrap();
        assert_eq!(v.derived.horizon_bars, 24);
        assert_eq!(v.derived.purge_bars, 24, "purge is the label horizon");
        // 24 + 21 (the widest window in a feature) + 0 + 0.
        assert_eq!(v.derived.computed_embargo_bars, 45);
        // 1% of ~46k bars is larger.
        assert!(v.derived.embargo_bars > 400);
        assert_eq!(v.derived.floor, Some("one_percent_of_sample"));
        assert_eq!(v.declaration_hash.len(), 64);
        assert!(!v.clipped_to_cutoff);
    }

    #[test]
    fn no_field_can_set_purge_or_embargo() {
        let mut j = serde_json::to_value(base()).unwrap();
        j["embargo_bars"] = serde_json::json!(5);
        assert!(serde_json::from_value::<DatasetDeclaration>(j.clone()).is_err());
        j.as_object_mut().unwrap().remove("embargo_bars");
        j["purge_bars"] = serde_json::json!(1);
        assert!(serde_json::from_value::<DatasetDeclaration>(j).is_err());
        // Nor inside a nested object.
        let mut cv = serde_json::to_value(base()).unwrap();
        cv["cv"]["embargo"] = serde_json::json!(3);
        assert!(serde_json::from_value::<DatasetDeclaration>(cv).is_err());
    }

    #[test]
    fn weights_are_required_and_never_defaulted() {
        let mut j = serde_json::to_value(base()).unwrap();
        j.as_object_mut().unwrap().remove("weights");
        let e = serde_json::from_value::<DatasetDeclaration>(j).unwrap_err();
        assert!(e.to_string().contains("weights"), "{e}");
    }

    #[test]
    fn the_hash_ignores_instrument_order_and_follows_every_field() {
        let a = base().validate(None).unwrap().declaration_hash;
        let mut swapped = base();
        swapped.universe =
            Universe::List(vec!["BTC-USD".into(), "ETH-USD".into(), "BTC-USD".into()]);
        assert_eq!(a, swapped.validate(None).unwrap().declaration_hash);

        let mut changed = base();
        changed.features.push("hurst(logret(close),256)".into());
        assert_ne!(a, changed.validate(None).unwrap().declaration_hash);
        let mut w = base();
        w.weights = Weights::None;
        assert_ne!(a, w.validate(None).unwrap().declaration_hash);
        let mut t = base();
        t.timeframe = "4h".into();
        assert_ne!(a, t.validate(None).unwrap().declaration_hash);
    }

    #[test]
    fn a_window_past_the_cutoff_is_clipped_and_says_so() {
        let v = base().validate(Some(day(2025, 1, 1))).unwrap();
        assert!(v.clipped_to_cutoff);
        assert_eq!(v.declaration.window.end, day(2025, 1, 1));
        assert!(base().validate(Some(day(2020, 6, 1))).is_err());
        // The clip changes what is declared, so it changes the hash.
        assert_ne!(
            v.declaration_hash,
            base().validate(None).unwrap().declaration_hash
        );
    }

    #[test]
    fn each_validity_rule_names_its_field() {
        let mut d = base();
        d.universe = Universe::List(vec![]);
        assert_eq!(err_field(&d), "universe");
        let mut d = base();
        d.timeframe = "7m".into();
        assert_eq!(err_field(&d), "timeframe");
        let mut d = base();
        d.window.end = d.window.start;
        assert_eq!(err_field(&d), "window");
        let mut d = base();
        d.features.clear();
        assert_eq!(err_field(&d), "features");
        let mut d = base();
        d.transforms.winsorize = Some(0.6);
        assert_eq!(err_field(&d), "transforms.winsorize");
        let mut d = base();
        d.transforms.lags = vec![2, 2];
        assert_eq!(err_field(&d), "transforms.lags");
        let mut d = base();
        d.target = Target::ForwardReturn {
            horizon_bars: 0,
            log: false,
        };
        assert_eq!(err_field(&d), "target.horizon_bars");
        let mut d = base();
        d.weights = Weights::TimeDecay {
            half_life_days: 0.0,
        };
        assert_eq!(err_field(&d), "weights.half_life_days");
        let mut d = base();
        d.cv = Cv::PurgedKfold { k: 1 };
        assert_eq!(err_field(&d), "cv.k");
        let mut d = base();
        d.cv = Cv::Cpcv { n: 3, k: 3 };
        assert_eq!(err_field(&d), "cv");
        let mut d = base();
        d.sampling = Sampling::Cusum { h_sigma: 0.0 };
        assert_eq!(err_field(&d), "sampling.h_sigma");
    }

    #[test]
    fn a_meta_label_needs_strategy_event_sampling() {
        let mut d = base();
        d.target = Target::MetaLabel {
            strategy_version: "sv1".into(),
            pt_sigma: 2.0,
            sl_sigma: 1.0,
            max_bars: 48,
            path: BarrierPath::HighLow,
        };
        assert_eq!(err_field(&d), "sampling");
        d.sampling = Sampling::StrategyEvents {
            strategy_version: "sv1".into(),
        };
        assert!(d.validate(None).is_ok());
    }

    #[test]
    fn a_sequence_window_longer_than_the_training_span_is_refused() {
        let mut d = base();
        d.timeframe = "1d".into();
        d.sequence = Some(Sequence {
            window: 400,
            stride: 1,
        });
        assert_eq!(err_field(&d), "sequence.window");
        d.sequence = Some(Sequence {
            window: 64,
            stride: 1,
        });
        let v = d.validate(None).unwrap();
        assert!(v.derived.max_lookback_bars >= 63, "the window is look-back");
    }

    #[test]
    fn a_window_too_short_for_one_fold_is_refused() {
        let mut d = base();
        d.window.end = d.window.start + chrono::Duration::days(100);
        assert_eq!(err_field(&d), "cv");
    }

    #[test]
    fn embargo_is_derived_on_a_table_of_cases() {
        // (timeframe, horizon, feature, lags, window days, expected embargo, floor)
        let cases: [(&str, u32, &str, Vec<u32>, i64, u32, Option<&str>); 4] = [
            // 1h, 400 days: 9,600 bars, 1% = 96 > 24 + 21.
            (
                "1h",
                24,
                "ema(close,21)",
                vec![],
                400,
                96,
                Some("one_percent_of_sample"),
            ),
            // A long look-back beats the 1% floor: 48 + 200.
            ("1h", 48, "sma(close,200)", vec![], 400, 248, None),
            // Lags count as look-back.
            (
                "1h",
                10,
                "ret(close)",
                vec![5, 30],
                400,
                96,
                Some("one_percent_of_sample"),
            ),
            // Fine resolution: at 5m the 24h floor (288) is the binding one.
            (
                "5m",
                12,
                "ema(close,5)",
                vec![],
                400,
                1152,
                Some("one_percent_of_sample"),
            ),
        ];
        for (tf, h, feat, lags, days, want, floor) in cases {
            let mut d = base();
            d.timeframe = tf.into();
            d.window = Window {
                start: day(2024, 1, 1),
                end: day(2024, 1, 1) + chrono::Duration::days(days),
            };
            d.features = vec![feat.into()];
            d.transforms.lags = lags;
            d.target = Target::ForwardReturn {
                horizon_bars: h,
                log: false,
            };
            let v = d.validate(None).unwrap();
            assert_eq!(v.derived.embargo_bars, want, "{tf} h={h} {feat}");
            assert_eq!(v.derived.floor, floor, "{tf} h={h} {feat}");
            // Never below the label horizon plus one, whatever else.
            assert!(v.derived.embargo_bars > h);
        }
    }

    #[test]
    fn the_fine_resolution_floor_is_24_hours_of_bars() {
        let mut d = base();
        d.timeframe = "5m".into();
        d.window = Window {
            start: day(2024, 1, 1),
            end: day(2024, 1, 1) + chrono::Duration::days(40),
        };
        d.features = vec!["ret(close)".into()];
        d.transforms.lags = vec![];
        d.cv = Cv::WalkForward {
            train_days: 20,
            test_days: 5,
            refit_days: 5,
            mode: WalkMode::Rolling,
        };
        d.target = Target::ForwardReturn {
            horizon_bars: 3,
            log: false,
        };
        let v = d.validate(None).unwrap();
        // 40 days of 5m bars is 11,520; 1% is 116, below the 288-bar day.
        assert_eq!(v.derived.embargo_bars, 288);
        assert_eq!(v.derived.floor, Some("fine_resolution_24h"));
    }

    #[test]
    fn expression_lookback_reads_window_arguments_not_identifier_digits() {
        assert_eq!(expression_lookback("ema(close,21)/close - 1"), 21);
        assert_eq!(expression_lookback("hurst(logret(close),256)"), 256);
        assert_eq!(
            expression_lookback("prediction(art_9f3).sigma"),
            9,
            "over-estimates, which is safe"
        );
        assert_eq!(expression_lookback("close"), 0);
    }

    #[test]
    fn specification_count_rises_on_additions_and_never_falls_on_removals() {
        let parent = base();
        let mut child = base();
        child.features.push("hurst(logret(close),256)".into());
        child.features.push("rv(5)".into());
        assert_eq!(specification_delta(&parent, &child), 2);
        child.transforms.lags.push(8);
        assert_eq!(specification_delta(&parent, &child), 3);
        // A child that only removes adds nothing, and nothing is subtracted.
        let mut thin = base();
        thin.features.truncate(1);
        thin.transforms.lags.clear();
        assert_eq!(specification_delta(&parent, &thin), 0);
        assert_eq!(specification_delta(&parent, &parent), 0);
    }

    // ── lowering ──

    fn ctx() -> LoweringContext {
        LoweringContext {
            universe_spec_id: "uni:1".into(),
            output_prefix: "ds/x".into(),
        }
    }

    #[test]
    fn a_supported_declaration_lowers_with_its_derived_embargo() {
        let mut d = base();
        d.features = vec!["set:core_v1".into()];
        d.transforms = Transforms::default();
        let v = d.validate(None).unwrap();
        let req = v.lower(&ctx()).unwrap();
        assert_eq!(req.feature_set_ref, "core_v1");
        assert_eq!(
            req.instruments,
            vec!["BTC-USD".to_string(), "ETH-USD".to_string()]
        );
        assert_eq!(req.label_spec.horizon_bars, 24);
        assert_eq!(
            req.label_spec.sample_weight_method,
            SampleWeightMethod::Uniqueness
        );
        assert_eq!(req.split_spec.embargo_bars(), v.derived.embargo_bars);
        assert_eq!(
            req.split_spec.computed_embargo_bars(),
            v.derived.computed_embargo_bars
        );
        // Lowering is deterministic.
        let again = v.lower(&ctx()).unwrap();
        assert_eq!(req.label_spec.label_spec_id, again.label_spec.label_spec_id);
        assert_eq!(req.split_spec.split_spec_id, again.split_spec.split_spec_id);
        assert_ne!(req.label_spec.label_spec_id, "");
    }

    #[allow(clippy::many_single_char_names)]
    #[test]
    fn unsupported_fields_are_named_not_dropped() {
        let mut d = base(); // free-form expressions, transforms
        d.sampling = Sampling::StrategyEvents {
            strategy_version: "sv1".into(),
        };
        d.sequence = Some(Sequence {
            window: 64,
            stride: 1,
        });
        d.universe = Universe::Named {
            universe: "crypto_top20".into(),
            as_of_membership: true,
        };
        d.target = Target::RealizedVol { horizon_bars: 24 };
        let un = d.validate(None).unwrap().lower(&ctx()).unwrap_err();
        let fields: Vec<&str> = un.iter().map(|u| u.field).collect();
        for f in [
            "universe",
            "features",
            "sampling",
            "sequence",
            "transforms",
            "target",
        ] {
            assert!(fields.contains(&f), "{f} missing from {fields:?}");
        }
        // The high/low triple barrier lowers now (N-1.4) and the spec records the path.
        let mut t = base();
        t.features = vec!["set:core_v1".into()];
        t.transforms = Transforms::default();
        t.target = Target::TripleBarrier {
            pt_sigma: 2.0,
            sl_sigma: 1.0,
            max_bars: 48,
            path: BarrierPath::HighLow,
        };
        let r = t.validate(None).unwrap().lower(&ctx()).unwrap();
        assert_eq!(r.label_spec.barrier_path, Some(BarrierPath::HighLow));
        // CUSUM is sampled natively (N-1.7): it lowers into the label spec, no trainer hand-off.
        let mut cu = t.clone();
        cu.sampling = Sampling::Cusum { h_sigma: 3.0 };
        let l = cu.validate(None).unwrap().lower_for_trainer(&ctx()).unwrap();
        assert_eq!(
            l.request.label_spec.sampling,
            Some(dataplane::sampling::SamplingSpec::Cusum { h_sigma: 3.0 })
        );
        assert!(!l.trainer_fields.contains(&"sampling"));
        assert!(cu.validate(None).unwrap().lower(&ctx()).is_ok());
        let mut c = t.clone();
        c.target = Target::TripleBarrier {
            pt_sigma: 2.0,
            sl_sigma: 1.0,
            max_bars: 48,
            path: BarrierPath::Close,
        };
        let rc = c.validate(None).unwrap().lower(&ctx()).unwrap();
        assert_eq!(rc.label_spec.barrier_path, Some(BarrierPath::Close));
        assert_ne!(
            rc.label_spec.label_spec_id, r.label_spec.label_spec_id,
            "the path is part of the label identity"
        );
    }

    #[test]
    fn the_cv_scheme_maps_to_the_split_kind() {
        let mut d = base();
        d.features = vec!["set:core_v1".into()];
        d.transforms = Transforms::default();
        d.cv = Cv::Cpcv { n: 6, k: 2 };
        let r = d.validate(None).unwrap().lower(&ctx()).unwrap();
        assert_eq!(r.split_spec.kind, SplitKind::Cpcv);
        assert_eq!((r.split_spec.n_folds, r.split_spec.n_test_groups), (6, 2));
        d.cv = Cv::WalkForward {
            train_days: 365,
            test_days: 30,
            refit_days: 30,
            mode: WalkMode::Rolling,
        };
        let r = d.validate(None).unwrap().lower(&ctx()).unwrap();
        assert!(matches!(
            r.split_spec.train_window,
            TrainWindow::Rolling(8760)
        ));
    }
}
