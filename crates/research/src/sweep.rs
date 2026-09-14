//! The sweep loop (FEAT-003 §7): sampler-driven batches of sealed Studies, then
//! one neighbourhood Study whose pre-declared selection rule is the **only**
//! thing carried forward.
//!
//! INV-2 in code: per-sample scores live in this function's locals and the
//! sampler; the [`SweepReport`] exposes a sealed distribution, a surface
//! description and the rule's output — never a ranked list, never an argmax.

use std::collections::{BTreeMap, BTreeSet};

use backtest::run::{Objective, ParamMap};
use backtest::study::{Distribution, SelectionRule};
use backtest::suite::{ParamBatchOutcome, ParamBatchSpec, StudyView};
use domain::strategy_def::StrategyDefinition;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sampler::{RandomSampler, Sampler, SamplerKind, TpeSampler};
use crate::space::{Narrow, SearchSpace, SpaceError};
use crate::surface::{Sensitivity, SurfaceSummary};

/// Attempts to re-draw an infeasible / duplicate point before giving up on it.
const MAX_REDRAWS: usize = 20;
/// Neighbourhood cube: vary at most this many dims (3^k members).
const MAX_CUBE_DIMS: usize = 3;
/// Neighbourhood half-width in unit-cube coordinates (one surface bin).
const CUBE_STEP: f64 = 1.0 / 8.0;

/// What the API layer provides to a sweep.
pub trait SweepBackend: Send + Sync {
    /// The stored definition for a strategy slug.
    fn definition(&self, strategy_ref: &str) -> Result<StrategyDefinition, String>;
    /// Run one sealed `ParameterSweep` Study and return the sampler-facing
    /// per-member metrics (see `SuiteManager::run_param_batch`).
    fn run_batch(
        &self,
        experiment: Uuid,
        spec: ParamBatchSpec,
    ) -> Result<ParamBatchOutcome, String>;
}

/// Progress sink (the API layer persists these for the UI / agent).
pub trait SweepObserver: Send + Sync {
    fn progress(&self, done: u32, planned: u32, note: &str);
    /// Poll for cancellation between batches.
    fn cancelled(&self) -> bool {
        false
    }
}

pub struct NoopObserver;
impl SweepObserver for NoopObserver {
    fn progress(&self, _done: u32, _planned: u32, _note: &str) {}
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SweepRequest {
    pub experiment_id: Uuid,
    pub strategy_ref: String,
    pub objective: Objective,
    /// Agent-supplied tightening of declared ranges (never widening).
    #[serde(default)]
    pub narrowing: BTreeMap<String, Narrow>,
    #[serde(default)]
    pub sampler: SamplerKind,
    /// Total sampled Runs (the neighbourhood cube is on top of this).
    #[serde(default = "default_max_runs")]
    pub max_runs: u32,
    #[serde(default = "default_batch")]
    pub batch_size: u32,
    #[serde(default)]
    pub seed: u64,
    /// Logged before running (Set J requires a question per Study).
    pub question: String,
    /// Values for parameters *not* in the space (held fixed), e.g. a prior
    /// carried-forward set.
    #[serde(default)]
    pub base_params: Option<ParamMap>,
}

fn default_max_runs() -> u32 {
    40
}
fn default_batch() -> u32 {
    8
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RacingInfo {
    pub enabled: bool,
    pub reason: String,
}

/// The sealed product of a sweep.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SweepReport {
    pub experiment_id: Uuid,
    pub strategy_ref: String,
    pub objective: Objective,
    pub sampler: SamplerKind,
    /// Every Study this sweep created, in order (batches, then neighbourhood).
    pub study_ids: Vec<String>,
    /// The neighbourhood Study's sealed distribution.
    pub distribution: Distribution,
    /// The neighbourhood Study's `MedianStableCentroid` member — the only
    /// carry-forward. `None` if no member survived.
    pub carried_forward: Option<ParamMap>,
    /// Set J's plateau verdict on the neighbourhood (dispersion/median < 0.5).
    pub neighbourhood_plateau: Option<bool>,
    pub surface: SurfaceSummary,
    pub trials_consumed: u32,
    pub n_sampled: u32,
    pub n_feasible: u32,
    pub n_failed_runs: u32,
    pub racing: RacingInfo,
    /// Constraint violations seen, by message, with counts — tells the agent
    /// *why* samples scored `-inf` (e.g. "42 trades < minimum 50" ×31).
    pub violation_counts: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SweepError {
    #[error(transparent)]
    Space(#[from] SpaceError),
    #[error("strategy: {0}")]
    Strategy(String),
    #[error("objective: {0}")]
    Objective(String),
    #[error("backend: {0}")]
    Backend(String),
    #[error("cancelled after {0} runs")]
    Cancelled(u32),
}

fn make_sampler(kind: SamplerKind, seed: u64) -> Box<dyn Sampler> {
    match kind {
        SamplerKind::Random => Box::new(RandomSampler::new(seed)),
        SamplerKind::Tpe => Box::new(TpeSampler::new(seed)),
    }
}

fn canonical(p: &ParamMap) -> String {
    serde_json::to_string(p).unwrap_or_default()
}

/// Run a sweep. Blocking; call from a blocking thread.
///
/// # Errors
/// [`SweepError`].
pub fn run_sweep(
    backend: &dyn SweepBackend,
    req: &SweepRequest,
    observer: &dyn SweepObserver,
) -> Result<SweepReport, SweepError> {
    req.objective.validate().map_err(SweepError::Objective)?;
    let def = backend
        .definition(&req.strategy_ref)
        .map_err(SweepError::Strategy)?;
    let space = SearchSpace::from_definition(&def, &req.narrowing)?;
    let mut sampler = make_sampler(req.sampler, req.seed);
    let prefix = format!("sweep-{}", Uuid::new_v4().simple());

    let planned = req.max_runs.max(1);
    let batch = req.batch_size.clamp(1, 64) as usize;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut samples: Vec<(ParamMap, f64)> = Vec::new();
    let mut study_ids = Vec::new();
    let (mut trials, mut n_failed, mut n_feasible) = (0u32, 0u32, 0u32);
    let mut violations: BTreeMap<String, u32> = BTreeMap::new();
    let mut batch_idx = 0u32;

    while (samples.len() as u32) < planned {
        if observer.cancelled() {
            return Err(SweepError::Cancelled(trials));
        }
        let want = batch.min((planned - samples.len() as u32) as usize);
        // Draw feasible, unseen points; re-draw the rest a bounded number of times.
        let mut grid: Vec<ParamMap> = Vec::with_capacity(want);
        let mut redraws = 0;
        while grid.len() < want && redraws < MAX_REDRAWS {
            let proposals = sampler.propose(&space, want - grid.len());
            let mut any = false;
            for u in proposals {
                let p = space.decode(&u);
                let key = canonical(&p);
                if !space.feasible(&p) {
                    sampler.observe(&u, f64::NEG_INFINITY);
                    continue;
                }
                if seen.insert(key) {
                    grid.push(p);
                    any = true;
                }
            }
            if !any {
                redraws += 1;
            }
        }
        if grid.is_empty() {
            break; // space exhausted (small int/enum spaces)
        }
        let spec = ParamBatchSpec {
            study_id: format!("{prefix}-b{batch_idx}"),
            grid: grid.clone(),
            metric: req.objective.primary,
            selection_rule: SelectionRule::None,
            question: format!("{} (batch {batch_idx})", req.question),
            base_params: req.base_params.clone(),
        };
        let out = backend
            .run_batch(req.experiment_id, spec)
            .map_err(SweepError::Backend)?;
        study_ids.push(out.view.study_id.clone());
        trials += out.view.trial_delta.max(0) as u32;
        for (params, metrics) in out.members {
            let score = match metrics {
                Some(m) => {
                    for v in req.objective.violations(&m) {
                        *violations.entry(v).or_default() += 1;
                    }
                    let s = req.objective.score(&m);
                    if s.is_finite() {
                        n_feasible += 1;
                    }
                    s
                }
                None => {
                    n_failed += 1;
                    f64::NEG_INFINITY
                }
            };
            sampler.observe(&space.encode(&params), score);
            samples.push((params, score));
        }
        batch_idx += 1;
        observer.progress(
            samples.len() as u32,
            planned,
            &format!("batch {batch_idx} done"),
        );
    }

    // ── Neighbourhood cube → the only carry-forward ─────────────────────────
    // Centre = centroid of the top-quartile feasible samples. Choosing where to
    // look is exploration; what comes back is the sealed Study's rule output.
    let surface = SurfaceSummary::build(&space, &samples);
    let (view, carried_forward) = {
        let mut finite: Vec<&(ParamMap, f64)> =
            samples.iter().filter(|(_, s)| s.is_finite()).collect();
        if finite.is_empty() {
            (None, None)
        } else {
            finite.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let k = ((finite.len() as f64 * 0.25).ceil() as usize).max(1);
            let d = space.n_dims();
            let mut centre = vec![0.0; d];
            for (p, _) in &finite[..k] {
                for (i, v) in space.encode(p).iter().enumerate() {
                    centre[i] += v / k as f64;
                }
            }
            // Vary the most sensitive numeric dims, hold the rest at the centre.
            let mut order: Vec<usize> = (0..d).filter(|&i| space.dims[i].is_numeric()).collect();
            let rank = |s: Sensitivity| match s {
                Sensitivity::High => 2,
                Sensitivity::Medium => 1,
                Sensitivity::Low => 0,
            };
            order.sort_by_key(|&i| std::cmp::Reverse(rank(surface.per_param[i].sensitivity)));
            let vary: Vec<usize> = order.into_iter().take(MAX_CUBE_DIMS).collect();
            let mut grid: Vec<ParamMap> = Vec::new();
            let mut keys = BTreeSet::new();
            let n_members = 3usize.pow(vary.len() as u32);
            for m in 0..n_members {
                let mut u = centre.clone();
                let mut rem = m;
                for &i in &vary {
                    let off = (rem % 3) as f64 - 1.0;
                    rem /= 3;
                    u[i] = (u[i] + off * CUBE_STEP).clamp(0.0, 1.0);
                }
                let p = space.decode(&u);
                if space.feasible(&p) && keys.insert(canonical(&p)) {
                    grid.push(p);
                }
            }
            if grid.is_empty() {
                (None, None)
            } else {
                let spec = ParamBatchSpec {
                    study_id: format!("{prefix}-neighbourhood"),
                    grid,
                    metric: req.objective.primary,
                    selection_rule: SelectionRule::MedianStableCentroid,
                    question: format!("{} (neighbourhood: is the region a plateau?)", req.question),
                    base_params: req.base_params.clone(),
                };
                let out = backend
                    .run_batch(req.experiment_id, spec)
                    .map_err(SweepError::Backend)?;
                trials += out.view.trial_delta.max(0) as u32;
                study_ids.push(out.view.study_id.clone());
                let cf = out.carried_forward.clone();
                (Some(out.view), cf)
            }
        }
    };
    let (distribution, plateau) = match &view {
        Some(v) => (v.distribution.clone(), v.verdict.plateau),
        None => (
            Distribution::from_values(req.objective.primary, Vec::new()),
            None,
        ),
    };
    let _: Option<&StudyView> = view.as_ref();

    Ok(SweepReport {
        experiment_id: req.experiment_id,
        strategy_ref: req.strategy_ref.clone(),
        objective: req.objective.clone(),
        sampler: req.sampler,
        study_ids,
        distribution,
        carried_forward,
        neighbourhood_plateau: plateau,
        surface,
        trials_consumed: trials,
        n_sampled: samples.len() as u32,
        n_feasible,
        n_failed_runs: n_failed,
        racing: RacingInfo {
            enabled: false,
            reason:
                "fidelity racing requires a calibration with rho >= 0.8 (P5); full fidelity used"
                    .into(),
        },
        violation_counts: violations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use backtest::run::{Constraint, MetricKind, MetricSet};
    use backtest::study::{StudyKind, StudyVerdict};
    use serde_json::json;
    use std::sync::Mutex;

    /// A backend that scores a quadratic in (fast, gate) with a known optimum
    /// and mimics the suite: every member is a Run, failures allowed.
    struct Fake {
        calls: Mutex<Vec<ParamBatchSpec>>,
    }

    fn def() -> StrategyDefinition {
        serde_json::from_value(json!({
            "strategy_id": "ema_p", "definition_version": "1.0", "asset_class": "x",
            "parameters": {
                "fast": { "type": "int", "default": 10, "min": 2, "max": 60 },
                "gate": { "type": "float", "default": 0.5, "min": 0.0, "max": 1.0 },
                "exit": { "type": "enum", "default": "a", "choices": ["a", "b"] }
            },
            "inputs": [], "nodes": [], "actions": []
        }))
        .unwrap()
    }

    fn metrics_for(p: &ParamMap) -> MetricSet {
        let fast = p["fast"].as_f64().unwrap();
        let gate = p["gate"].as_f64().unwrap();
        let sortino = 3.0 - ((fast - 20.0) / 20.0).powi(2) - (gate - 0.3).powi(2);
        MetricSet {
            sortino,
            n_trades: if fast > 50.0 { 5 } else { 120 },
            max_drawdown: -0.1,
            ..MetricSet::empty()
        }
    }

    impl SweepBackend for Fake {
        fn definition(&self, _s: &str) -> Result<StrategyDefinition, String> {
            Ok(def())
        }
        fn run_batch(&self, _e: Uuid, spec: ParamBatchSpec) -> Result<ParamBatchOutcome, String> {
            self.calls.lock().unwrap().push(spec.clone());
            let members: Vec<(ParamMap, Option<MetricSet>)> = spec
                .grid
                .iter()
                .map(|p| (p.clone(), Some(metrics_for(p))))
                .collect();
            let values: Vec<f64> = members
                .iter()
                .filter_map(|(_, m)| m.map(|m| m.value(spec.metric)))
                .collect();
            let dist = Distribution::from_values(spec.metric, values.clone());
            let carried = if spec.selection_rule == SelectionRule::MedianStableCentroid {
                // Closest-to-median member, as the engine does.
                members
                    .iter()
                    .min_by(|a, b| {
                        let va = (a.1.unwrap().value(spec.metric) - dist.median).abs();
                        let vb = (b.1.unwrap().value(spec.metric) - dist.median).abs();
                        va.partial_cmp(&vb).unwrap()
                    })
                    .map(|(p, _)| p.clone())
            } else {
                None
            };
            let view = StudyView {
                study_id: spec.study_id.clone(),
                kind: StudyKind::ParameterSweep,
                metric: spec.metric,
                question: spec.question.clone(),
                trial_delta: spec.grid.len() as i64,
                sealed: true,
                distribution: dist.clone(),
                verdict: StudyVerdict {
                    summary: String::new(),
                    positive_median: dist.median > 0.0,
                    survivable_worst5: dist.worst_5pct > 0.0,
                    plateau: Some(dist.median > 0.0 && dist.spread / dist.median < 0.5),
                },
                members: (0..spec.grid.len()).map(|i| format!("run-{i}")).collect(),
                selection_rule: spec.selection_rule,
                carried_forward: carried.is_some(),
                carried_forward_params: carried.clone(),
                unsafe_flag: false,
            };
            Ok(ParamBatchOutcome {
                view,
                members,
                carried_forward: carried,
            })
        }
    }

    fn request() -> SweepRequest {
        SweepRequest {
            experiment_id: Uuid::new_v4(),
            strategy_ref: "ema_p".into(),
            objective: Objective {
                primary: MetricKind::Sortino,
                constraints: vec![Constraint::MinTrades { value: 50 }],
                aggregate: Default::default(),
            },
            narrowing: BTreeMap::new(),
            sampler: SamplerKind::Tpe,
            max_runs: 40,
            batch_size: 8,
            seed: 3,
            question: "does an EMA cross beat hold?".into(),
            base_params: None,
        }
    }

    #[test]
    fn sweep_carries_forward_a_stable_centroid_near_the_optimum() {
        let fake = Fake {
            calls: Mutex::new(Vec::new()),
        };
        let report = run_sweep(&fake, &request(), &NoopObserver).unwrap();
        assert_eq!(report.n_sampled, 40);
        assert!(report.trials_consumed > 40, "neighbourhood adds trials");
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), report.study_ids.len());
        // Only the last Study declares a selection rule; batches never do.
        assert!(calls[..calls.len() - 1]
            .iter()
            .all(|c| c.selection_rule == SelectionRule::None));
        assert_eq!(
            calls.last().unwrap().selection_rule,
            SelectionRule::MedianStableCentroid
        );
        let cf = report.carried_forward.expect("carried forward");
        let fast = cf["fast"].as_f64().unwrap();
        assert!(
            (8.0..=32.0).contains(&fast),
            "fast={fast} not near optimum 20"
        );
        // Constraint violations are explained.
        assert!(
            report.violation_counts.keys().any(|k| k.contains("trades")) || report.n_feasible == 40
        );
        assert!(report.surface.text.contains("fast"));
    }

    /// INV-2 property: the serialized report carries no per-sample score and no
    /// list ranked by score — only sealed distribution moments and the rule's
    /// single output.
    #[test]
    fn report_never_exposes_ranked_samples() {
        let fake = Fake {
            calls: Mutex::new(Vec::new()),
        };
        let report = run_sweep(&fake, &request(), &NoopObserver).unwrap();
        let json = serde_json::to_value(&report).unwrap();
        // No array of (params, score) pairs anywhere.
        fn walk(v: &serde_json::Value, path: &str) {
            match v {
                serde_json::Value::Array(items) => {
                    for (i, it) in items.iter().enumerate() {
                        if let serde_json::Value::Object(o) = it {
                            assert!(
                                !(o.contains_key("score") || o.contains_key("metrics")),
                                "ranked sample leaked at {path}[{i}]"
                            );
                        }
                        walk(it, &format!("{path}[{i}]"));
                    }
                }
                serde_json::Value::Object(o) => {
                    assert!(!o.contains_key("samples"), "samples leaked at {path}");
                    assert!(!o.contains_key("best"), "best leaked at {path}");
                    for (k, x) in o {
                        walk(x, &format!("{path}.{k}"));
                    }
                }
                _ => {}
            }
        }
        walk(&json, "$");
        // The distribution exposes moments and raw values in insertion order —
        // that is Set J's sealed shape, unchanged.
        assert!(json["distribution"]["median"].is_number());
    }

    #[test]
    fn cancellation_is_honoured() {
        struct Cancel;
        impl SweepObserver for Cancel {
            fn progress(&self, _d: u32, _p: u32, _n: &str) {}
            fn cancelled(&self) -> bool {
                true
            }
        }
        let fake = Fake {
            calls: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            run_sweep(&fake, &request(), &Cancel),
            Err(SweepError::Cancelled(0))
        ));
    }
}
