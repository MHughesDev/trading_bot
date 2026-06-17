//! [`StudyEngine`] — fans a [`StudyConfig`] out into member Runs over the
//! cache-aware [`Backtest`] engine, collects a **sealed** distribution, and
//! emits a `trial_delta` (spec §1.2).
//!
//! Every member counts toward `trial_delta` — cache hits and failures included.
//! Failed/rejected members are recorded in `member_run_ids` (provenance) but
//! excluded from the metric distribution. A single config is carried forward
//! only through the pre-declared [`SelectionRule`] — never an argmax (INV-2).

use chrono::{DateTime, Duration, Utc};

use crate::rng::DetRng;
use crate::run::{
    Backtest, MetricInputs, MetricKind, MetricSet, RunConfig, RunExecutor, RunStatus, RunStore,
};

use super::config::{SelectionRule, StudyConfig, StudyConfigError, StudyKind, VarySpec};
use super::result::{Distribution, StudyResult, StudyVerdict};

/// Orchestrates the Runs of one Study.
pub struct StudyEngine;

impl StudyEngine {
    /// Run a Study against a cache-aware [`Backtest`] engine.
    ///
    /// # Errors
    /// Returns [`StudyConfigError`] if the config is invalid (the Study never
    /// runs, so it never touches the trial counter).
    pub fn run<S: RunStore, E: RunExecutor>(
        study: &StudyConfig,
        bt: &Backtest<S, E>,
    ) -> Result<StudyResult, StudyConfigError> {
        study.validate()?;

        if study.kind == StudyKind::TradeMonteCarlo {
            return Ok(trade_monte_carlo(study, bt));
        }

        // Expand the varying dimension into member configs, honoring the budget.
        let mut members = expand_members(study);
        if members.len() > study.budget.max_runs as usize {
            members.truncate(study.budget.max_runs as usize);
        }

        let mut member_ids = Vec::with_capacity(members.len());
        let mut surviving: Vec<(RunConfig, f64)> = Vec::new();
        let mut any_unsafe = false;
        for cfg in members {
            let result = bt.run(&cfg);
            member_ids.push(result.run_id.clone());
            any_unsafe |= result.unsafe_;
            if result.status == RunStatus::Ok {
                surviving.push((cfg, result.metrics.value(study.metric)));
            }
        }

        // trial_delta counts EVERY member run (cache hits + failures included).
        let trial_delta = member_ids.len() as i64;

        let values: Vec<f64> = surviving.iter().map(|(_, v)| *v).collect();
        let dist = Distribution::from_values(study.metric, values);
        let carried_forward = apply_selection_rule(study.selection_rule, &surviving, &dist);
        let verdict = verdict_for(study.kind, &dist);

        Ok(StudyResult::new(
            study.study_id.clone(),
            member_ids,
            dist,
            verdict,
            trial_delta,
            carried_forward,
            any_unsafe,
        ))
    }
}

/// Expand a Study's `VarySpec` into the member configs to run. Each member is a
/// *new* Run with its own content-addressed id (`rehashed`).
fn expand_members(study: &StudyConfig) -> Vec<RunConfig> {
    let base = &study.base_config;
    match &study.vary {
        VarySpec::Params { grid } => grid
            .iter()
            .map(|overrides| {
                let mut cfg = base.clone();
                for (k, v) in overrides {
                    cfg.params.insert(k.clone(), v.clone());
                }
                cfg.rehashed()
            })
            .collect(),
        VarySpec::Neighborhood {
            param,
            center,
            step,
            k,
        } => {
            let k = i64::from(*k);
            (-k..=k)
                .map(|i| {
                    let value = center + i as f64 * step;
                    let mut cfg = base.clone();
                    let json_value = serde_json::Number::from_f64(value)
                        .map_or(serde_json::Value::Null, serde_json::Value::Number);
                    cfg.params.insert(param.clone(), json_value);
                    cfg.rehashed()
                })
                .collect()
        }
        VarySpec::DataWindows { windows } => windows
            .iter()
            .map(|(start, end)| {
                let mut cfg = base.clone();
                cfg.data_slice.start = *start;
                cfg.data_slice.end = *end;
                cfg.rehashed()
            })
            .collect(),
        VarySpec::CpcvGroups { n_groups, k_test } => {
            let group_windows =
                split_windows(base.data_slice.start, base.data_slice.end, *n_groups);
            cpcv_assignments(*n_groups as usize, *k_test as usize)
                .into_iter()
                .map(|split| {
                    // The OOS member evaluates over the bounding window of its
                    // test groups; train/test disjointness is asserted by the
                    // property test on `cpcv_assignments`.
                    let (lo, hi) = bounding_window(&group_windows, &split.test);
                    let mut cfg = base.clone();
                    cfg.data_slice.start = lo;
                    cfg.data_slice.end = hi;
                    // Distinguish otherwise-identical windows by a per-split seed
                    // so members do not collapse onto one run_id.
                    cfg.seed = base.seed.wrapping_add(split_seed(&split.test));
                    cfg.rehashed()
                })
                .collect()
        }
        VarySpec::Seeds { n } => (0..*n)
            .map(|i| {
                let mut cfg = base.clone();
                cfg.seed = base.seed.wrapping_add(u64::from(i)).wrapping_add(1);
                cfg.rehashed()
            })
            .collect(),
        VarySpec::CostLadder { cost_model_refs } => cost_model_refs
            .iter()
            .map(|cm| {
                let mut cfg = base.clone();
                cfg.cost_model_ref.clone_from(cm);
                cfg.rehashed()
            })
            .collect(),
        VarySpec::Regimes { windows } => windows
            .iter()
            .map(|(start, end, _label)| {
                let mut cfg = base.clone();
                cfg.data_slice.start = *start;
                cfg.data_slice.end = *end;
                cfg.rehashed()
            })
            .collect(),
        // TradeMonteCarlo is handled before expansion.
        VarySpec::TradeResamples { .. } => Vec::new(),
    }
}

/// `trade_monte_carlo`: run the base config once, then block-bootstrap-resample
/// the executed trade sequence to produce a distribution of path-dependent risk
/// (max drawdown). Answers "how lucky was this particular ordering?".
fn trade_monte_carlo<S: RunStore, E: RunExecutor>(
    study: &StudyConfig,
    bt: &Backtest<S, E>,
) -> StudyResult {
    let base_result = bt.run(&study.base_config);
    let member_ids = vec![base_result.run_id.clone()];

    let (n, block) = match study.vary {
        VarySpec::TradeResamples { n, block } => (n.max(1), block.max(1)),
        _ => (1, 1),
    };

    // Per-trade pseudo-returns (pnl as a fraction of a unit base).
    use rust_decimal::prelude::ToPrimitive;
    let trade_returns: Vec<f64> = base_result
        .trades
        .iter()
        .map(|t| t.pnl.to_f64().unwrap_or(0.0))
        .collect();

    let mut drawdowns = Vec::with_capacity(n as usize);
    if !trade_returns.is_empty() {
        let mut rng = DetRng::new(study.base_config.seed);
        for _ in 0..n {
            let resampled = block_bootstrap(&trade_returns, block as usize, &mut rng);
            let m = MetricSet::compute(&MetricInputs {
                equity_returns: &resampled,
                trades: &[],
                benchmark_returns: None,
                net_exposure: None,
                periods_per_year: 252.0,
            });
            drawdowns.push(m.max_drawdown);
        }
    }

    let dist = Distribution::from_values(MetricKind::MaxDrawdown, drawdowns);
    let verdict = StudyVerdict {
        summary: format!(
            "trade-ordering Monte Carlo: median MaxDD {:.4}, worst-5% {:.4}",
            dist.median, dist.worst_5pct
        ),
        positive_median: dist.median > 0.0,
        survivable_worst5: dist.worst_5pct > -1.0,
        plateau: None,
    };
    // The base run is the only Run; resamples are pure analysis, not Runs.
    StudyResult::new(
        study.study_id.clone(),
        member_ids,
        dist,
        verdict,
        1,
        None,
        base_result.unsafe_,
    )
}

/// Block bootstrap: draw consecutive blocks of length `block` (with wraparound)
/// until the resample matches the source length.
fn block_bootstrap(source: &[f64], block: usize, rng: &mut DetRng) -> Vec<f64> {
    let n = source.len();
    let block = block.max(1).min(n.max(1));
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let start = rng.below(n);
        for j in 0..block {
            if out.len() >= n {
                break;
            }
            out.push(source[(start + j) % n]);
        }
    }
    out
}

/// Apply the pre-declared selection rule. Never an argmax: `MedianStableCentroid`
/// returns the member closest to the median; `WorstCaseRobust` the member
/// closest to the worst-5%.
fn apply_selection_rule(
    rule: SelectionRule,
    surviving: &[(RunConfig, f64)],
    dist: &Distribution,
) -> Option<RunConfig> {
    if surviving.is_empty() {
        return None;
    }
    let target = match rule {
        SelectionRule::None => return None,
        SelectionRule::MedianStableCentroid => dist.median,
        SelectionRule::WorstCaseRobust => dist.worst_5pct,
    };
    surviving
        .iter()
        .min_by(|a, b| {
            (a.1 - target)
                .abs()
                .partial_cmp(&(b.1 - target).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(cfg, _)| cfg.clone())
}

/// Build the study-kind-specific verdict from the sealed distribution.
fn verdict_for(kind: StudyKind, dist: &Distribution) -> StudyVerdict {
    let positive_median = dist.median > 0.0;
    let survivable_worst5 = dist.worst_5pct > 0.0;
    let plateau = match kind {
        StudyKind::Neighborhood | StudyKind::ParameterSweep => Some(is_plateau(dist)),
        _ => None,
    };
    let summary = format!(
        "{kind:?}: median {:.4}, IQR [{:.4}, {:.4}], worst-5% {:.4}, spread {:.4}",
        dist.median, dist.iqr[0], dist.iqr[1], dist.worst_5pct, dist.spread
    );
    StudyVerdict {
        summary,
        positive_median,
        survivable_worst5,
        plateau,
    }
}

/// A broad plateau = low dispersion relative to the median (robust); an isolated
/// spike = high relative dispersion (overfit). Uses the coefficient of variation.
fn is_plateau(dist: &Distribution) -> bool {
    if dist.median <= 0.0 {
        return false;
    }
    (dist.spread / dist.median) < 0.5
}

// ── pure window/combination helpers ─────────────────────────────────────────

/// Split `[start, end)` into `n` contiguous equal windows.
fn split_windows(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    n: u32,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let n = i64::from(n.max(1));
    let total = (end - start).num_seconds().max(0);
    let step = total / n;
    (0..n)
        .map(|i| {
            let lo = start + Duration::seconds(step * i);
            let hi = if i == n - 1 {
                end
            } else {
                start + Duration::seconds(step * (i + 1))
            };
            (lo, hi)
        })
        .collect()
}

fn bounding_window(
    windows: &[(DateTime<Utc>, DateTime<Utc>)],
    groups: &[usize],
) -> (DateTime<Utc>, DateTime<Utc>) {
    let lo = groups
        .iter()
        .map(|&g| windows[g].0)
        .min()
        .unwrap_or(windows[0].0);
    let hi = groups
        .iter()
        .map(|&g| windows[g].1)
        .max()
        .unwrap_or(windows[0].1);
    (lo, hi)
}

fn split_seed(test: &[usize]) -> u64 {
    test.iter().fold(0u64, |acc, &g| {
        acc.wrapping_mul(31).wrapping_add(g as u64 + 1)
    })
}

/// A combinatorial purged CV split: which groups are test vs train.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpcvSplit {
    pub test: Vec<usize>,
    pub train: Vec<usize>,
}

/// All C(n, k) train/test group assignments. `test` and `train` partition
/// `0..n` with no overlap (the property the leakage discipline rests on).
#[must_use]
pub fn cpcv_assignments(n_groups: usize, k_test: usize) -> Vec<CpcvSplit> {
    combinations(n_groups, k_test)
        .into_iter()
        .map(|test| {
            let train: Vec<usize> = (0..n_groups).filter(|g| !test.contains(g)).collect();
            CpcvSplit { test, train }
        })
        .collect()
}

/// All k-combinations of `0..n` (lexicographic).
#[must_use]
pub fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    if k == 0 || k > n {
        return out;
    }
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        out.push(idx.clone());
        // advance like an odometer
        let mut i = k;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if idx[i] != i + n - k {
                break;
            }
        }
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::run::executor::{daily_curve, map_sim_result};
    use crate::run::{
        Backtest, ClosureExecutor, ComputeCost, DataSlice, EvalResolution, InMemoryRunStore,
        ParamMap, RunConfig, RunConfigBuilder, ENGINE_VERSION,
    };
    use crate::study::config::{StudyBudget, StudyConfig};
    use chrono::TimeZone;
    use serde_json::json;

    fn base() -> RunConfig {
        let s = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 12, 31, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        RunConfigBuilder::new("s", "v", s, "c", "z", "snap").build()
    }

    /// An executor whose return level is a function of the `fast` param: a sharp
    /// peak at 12 (spike) unless `plateau` is set, in which case it is flat-high.
    fn objective_executor(plateau: bool) -> impl RunExecutor {
        ClosureExecutor(move |cfg: &RunConfig| {
            let fast = cfg
                .params
                .get("fast")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let level = if plateau {
                0.01 // flat, high-ish daily return everywhere
            } else if (fast - 12.0).abs() < 0.5 {
                0.05 // isolated spike at fast==12
            } else {
                0.0001
            };
            let curve = daily_curve(&[100.0, 100.0 * (1.0 + level), 100.0 * (1.0 + level).powi(2)]);
            map_sim_result(
                cfg,
                curve,
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        })
    }

    fn sweep_cfg(grid: Vec<ParamMap>) -> StudyConfig {
        StudyConfig {
            study_id: "sweep".into(),
            kind: StudyKind::ParameterSweep,
            base_config: base(),
            vary: VarySpec::Params { grid },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "how does perf vary across params?".into(),
            selection_rule: SelectionRule::MedianStableCentroid,
        }
    }

    fn param(fast: f64) -> ParamMap {
        let mut p = ParamMap::new();
        p.insert("fast".into(), json!(fast));
        p
    }

    #[test]
    fn combinations_count_is_binomial() {
        assert_eq!(combinations(5, 2).len(), 10);
        assert_eq!(
            combinations(4, 2),
            vec![
                vec![0, 1],
                vec![0, 2],
                vec![0, 3],
                vec![1, 2],
                vec![1, 3],
                vec![2, 3]
            ]
        );
        assert!(combinations(3, 0).is_empty());
        assert!(combinations(2, 3).is_empty());
    }

    #[test]
    fn cpcv_train_and_test_are_disjoint_partitions() {
        for split in cpcv_assignments(6, 2) {
            for t in &split.test {
                assert!(!split.train.contains(t), "test group leaked into train");
            }
            assert_eq!(split.test.len() + split.train.len(), 6);
        }
    }

    #[test]
    fn sweep_counts_every_member_and_seals() {
        let bt = Backtest::new(InMemoryRunStore::new(), objective_executor(true));
        let grid: Vec<ParamMap> = (0..50).map(|i| param(f64::from(i))).collect();
        let res = StudyEngine::run(&sweep_cfg(grid), &bt).unwrap();
        assert_eq!(res.members().len(), 50);
        assert_eq!(res.trial_delta, 50);
        assert!(res.sealed);
    }

    #[test]
    fn rerunning_uses_cache_but_still_reports_full_trial_delta() {
        let bt = Backtest::new(InMemoryRunStore::new(), objective_executor(true));
        let grid: Vec<ParamMap> = (0..10).map(|i| param(f64::from(i))).collect();
        let cfg = sweep_cfg(grid);
        let first = StudyEngine::run(&cfg, &bt).unwrap();
        let again = StudyEngine::run(&cfg, &bt).unwrap();
        assert_eq!(first.trial_delta, 10);
        assert_eq!(again.trial_delta, 10, "cache hits still count");
    }

    #[test]
    fn neighborhood_detects_plateau_vs_spike() {
        // Plateau objective → low relative spread → plateau true.
        let bt = Backtest::new(InMemoryRunStore::new(), objective_executor(true));
        let study = StudyConfig {
            study_id: "nbhd".into(),
            kind: StudyKind::Neighborhood,
            base_config: base(),
            vary: VarySpec::Neighborhood {
                param: "fast".into(),
                center: 12.0,
                step: 1.0,
                k: 5,
            },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "is it a plateau?".into(),
            selection_rule: SelectionRule::None,
        };
        let plateau_res = StudyEngine::run(&study, &bt).unwrap();
        assert_eq!(plateau_res.verdict.plateau, Some(true));

        // Spike objective → high relative spread → plateau false.
        let bt2 = Backtest::new(InMemoryRunStore::new(), objective_executor(false));
        let spike_res = StudyEngine::run(&study, &bt2).unwrap();
        assert_eq!(spike_res.verdict.plateau, Some(false));
    }

    #[test]
    fn selection_rule_returns_median_not_peak() {
        // Spike objective: peak at fast==12 (level 0.05); rest near zero. The
        // MedianStableCentroid rule must NOT return the peak.
        let bt = Backtest::new(InMemoryRunStore::new(), objective_executor(false));
        let grid: Vec<ParamMap> = (8..=16).map(|i| param(f64::from(i))).collect();
        let res = StudyEngine::run(&sweep_cfg(grid), &bt).unwrap();
        let carried = res.carried_forward.expect("a config is carried forward");
        let fast = carried
            .params
            .get("fast")
            .and_then(serde_json::Value::as_f64)
            .unwrap();
        assert_ne!(fast, 12.0, "must not carry the peak forward");
    }

    #[test]
    fn permutation_study_requires_null_to_run() {
        let bt = Backtest::new(InMemoryRunStore::new(), objective_executor(true));
        let study = StudyConfig {
            study_id: "perm".into(),
            kind: StudyKind::PermutationNull,
            base_config: base(),
            vary: VarySpec::Seeds { n: 5 },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "is it real?".into(),
            selection_rule: SelectionRule::None,
        };
        assert_eq!(
            StudyEngine::run(&study, &bt).err(),
            Some(StudyConfigError::MissingNull)
        );
    }

    #[test]
    fn cpcv_runs_all_combinations() {
        let bt = Backtest::new(InMemoryRunStore::new(), objective_executor(true));
        let study = StudyConfig {
            study_id: "cpcv".into(),
            kind: StudyKind::Cpcv,
            base_config: base(),
            vary: VarySpec::CpcvGroups {
                n_groups: 6,
                k_test: 2,
            },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "robust across history?".into(),
            selection_rule: SelectionRule::None,
        };
        let res = StudyEngine::run(&study, &bt).unwrap();
        assert_eq!(res.members().len(), 15); // C(6,2)
        assert_eq!(res.trial_delta, 15);
    }

    #[test]
    fn cost_sweep_finds_where_edge_dies() {
        // Higher (later) cost models drive return negative in this executor.
        let exec = ClosureExecutor(|cfg: &RunConfig| {
            let level = match cfg.cost_model_ref.as_str() {
                "cost:optimistic" => 0.02,
                "cost:mid" => 0.005,
                _ => -0.01,
            };
            let curve = daily_curve(&[100.0, 100.0 * (1.0 + level)]);
            map_sim_result(
                cfg,
                curve,
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        });
        let bt = Backtest::new(InMemoryRunStore::new(), exec);
        let study = StudyConfig {
            study_id: "cost".into(),
            kind: StudyKind::CostSweep,
            base_config: base(),
            vary: VarySpec::CostLadder {
                cost_model_refs: vec![
                    "cost:optimistic".into(),
                    "cost:mid".into(),
                    "cost:pessimistic".into(),
                ],
            },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "where does the edge die?".into(),
            selection_rule: SelectionRule::None,
        };
        let res = StudyEngine::run(&study, &bt).unwrap();
        assert_eq!(res.members().len(), 3);
        assert!(
            res.distribution.worst_5pct < 0.0,
            "pessimistic costs kill it"
        );
    }

    #[test]
    fn trade_monte_carlo_distribution_over_orderings() {
        use crate::run::executor::sample_trade;
        use rust_decimal_macros::dec;
        // A base run with autocorrelated trade pnls.
        let exec = ClosureExecutor(|cfg: &RunConfig| {
            let trades = vec![
                sample_trade(dec!(0.10)),
                sample_trade(dec!(0.10)),
                sample_trade(dec!(-0.30)),
                sample_trade(dec!(0.05)),
                sample_trade(dec!(-0.20)),
                sample_trade(dec!(0.08)),
            ];
            map_sim_result(
                cfg,
                daily_curve(&[100.0, 100.0]),
                vec![],
                trades,
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        });
        let bt = Backtest::new(InMemoryRunStore::new(), exec);
        let study = StudyConfig {
            study_id: "tmc".into(),
            kind: StudyKind::TradeMonteCarlo,
            base_config: base(),
            vary: VarySpec::TradeResamples { n: 500, block: 2 },
            metric: MetricKind::MaxDrawdown,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "how lucky was the ordering?".into(),
            selection_rule: SelectionRule::None,
        };
        let res = StudyEngine::run(&study, &bt).unwrap();
        assert_eq!(res.trial_delta, 1, "only the base run is a Run");
        assert_eq!(res.distribution.dist.len(), 500);
        // The worst-case ordering must be at least as bad as the median.
        assert!(res.distribution.worst_5pct <= res.distribution.median);
    }
}
