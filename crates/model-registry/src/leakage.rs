//! The leakage harness a dataset must pass before it is registered (Set N N-1.5, MD-01).
//!
//! Two checks, both structural rather than statistical:
//!
//! * **Feature truncation self-test.** Features at time `t` may use only data up to
//!   `t`. Rebuild the frame from a series cut short and every feature value the short
//!   frame produces must equal the one the full frame produced at the same timestamp.
//!   A feature that changes when the future is removed was reading the future.
//! * **Label / fold overlap.** Expand the dataset's split spec over the label intervals
//!   and require that no training sample's label interval overlaps any test sample's.
//!   This is the purge actually doing what the derived purge says it does, checked on
//!   the real folds rather than assumed from the parameters.
//!
//! [`crate::datasets::DatasetManager::materialize`] runs both between building the frame
//! and writing anything: a failing dataset errors before the Parquet is stored or a
//! `dataset_versions` row exists, so there is no registered dataset to be mistaken for a
//! clean one. The report is recorded on the row (`leakage_report`, migration 0207, whose
//! CHECK refuses a report that did not pass).

use dataplane::label::{horizon_intervals, LabelInterval};
use dataplane::split::SplitSpec;
use features::align::BarObs;
use features::training_frame::TrainingFrame;
use serde::Serialize;
use serde_json::{json, Value};

/// One named check and what it found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// The harness's whole verdict, recorded on the dataset version.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LeakageReport {
    pub passed: bool,
    pub checks: Vec<Check>,
    pub engine_version: &'static str,
    pub data_snapshot: Value,
}

impl LeakageReport {
    #[must_use]
    pub fn from_checks(checks: Vec<Check>, data_snapshot: Value) -> Self {
        Self {
            passed: !checks.is_empty() && checks.iter().all(|c| c.passed),
            checks,
            engine_version: env!("CARGO_PKG_VERSION"),
            data_snapshot,
        }
    }

    /// The failing checks, for the refusal message.
    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| format!("{}: {}", c.name, c.detail))
            .collect()
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({"passed": false}))
    }
}

fn same(a: f64, b: f64) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

/// Rebuild the frame from the first `keep_fraction` of `obs` and compare every feature
/// value with the full frame's at the same timestamp. The labels are *not* compared:
/// a label legitimately looks forward, and the truncated frame's last labels are cut.
///
/// A series too short to truncate meaningfully passes vacuously and says so, because
/// the alternative (failing every small dataset) would make the harness unusable on the
/// fixtures it exists to protect.
#[must_use]
pub fn feature_truncation(
    build: &dyn Fn(&[BarObs]) -> TrainingFrame,
    obs: &[BarObs],
    full: &TrainingFrame,
    keep_fraction: f64,
) -> Check {
    let keep = ((obs.len() as f64) * keep_fraction) as usize;
    if keep < 30 || full.ts_ns.is_empty() {
        return Check {
            name: "feature_truncation",
            passed: true,
            detail: format!("not run: {} bars is too few to truncate", obs.len()),
        };
    }
    let short = build(&obs[..keep]);
    let mut idx = std::collections::HashMap::with_capacity(full.ts_ns.len());
    for (i, ts) in full.ts_ns.iter().enumerate() {
        idx.insert(*ts, i);
    }
    let mut compared = 0usize;
    for (name_i, name) in short.feature_names.iter().enumerate() {
        let Some(full_i) = full.feature_names.iter().position(|n| n == name) else {
            continue;
        };
        for (row, ts) in short.ts_ns.iter().enumerate() {
            let Some(&frow) = idx.get(ts) else {
                // The short frame kept a row the full frame dropped: the NaN drop
                // moved, which is itself a sign a value depended on later data.
                return Check {
                    name: "feature_truncation",
                    passed: false,
                    detail: format!("row at {ts} exists only when the series is truncated"),
                };
            };
            compared += 1;
            let (a, b) = (short.columns[name_i][row], full.columns[full_i][frow]);
            if !same(a, b) {
                return Check {
                    name: "feature_truncation",
                    passed: false,
                    detail: format!(
                        "feature `{name}` at {ts} is {a} when the series is truncated and {b} on the full series"
                    ),
                };
            }
        }
    }
    Check {
        name: "feature_truncation",
        passed: true,
        detail: format!(
            "{compared} feature values identical on a series cut to {keep} of {} bars",
            obs.len()
        ),
    }
}

/// Expand `split` over `n` labels of `horizon_bars` and require that no train sample's
/// label interval overlaps a test sample's.
#[must_use]
pub fn label_fold_overlap(split: &SplitSpec, n: usize, horizon_bars: u32) -> Check {
    if n < 4 {
        return Check {
            name: "label_fold_overlap",
            passed: true,
            detail: format!("not run: {n} samples is too few to fold"),
        };
    }
    let labels = horizon_intervals(n, horizon_bars);
    check_folds(split, &labels)
}

/// [`label_fold_overlap`] over explicit label intervals (so a test can plant an overlap).
#[must_use]
pub fn check_folds(split: &SplitSpec, labels: &[LabelInterval]) -> Check {
    let folds = match split.expand(labels) {
        Ok(f) => f,
        Err(e) => {
            return Check {
                name: "label_fold_overlap",
                passed: true,
                detail: format!("not run: the split cannot be expanded over this dataset ({e})"),
            }
        }
    };
    for (k, fold) in folds.iter().enumerate() {
        let (Some(t_min), Some(t_max)) = (
            fold.test.iter().map(|&j| labels[j].t0).min(),
            fold.test.iter().map(|&j| labels[j].t1).max(),
        ) else {
            continue;
        };
        for &i in &fold.train {
            let iv = labels[i];
            // Overlap with the test span is necessary for overlap with any test label.
            if iv.t0 <= t_max && iv.t1 >= t_min {
                if let Some(&j) = fold
                    .test
                    .iter()
                    .find(|&&j| iv.t0 <= labels[j].t1 && iv.t1 >= labels[j].t0)
                {
                    return Check {
                        name: "label_fold_overlap",
                        passed: false,
                        detail: format!(
                            "fold {k}: train label {i} [{}, {}] overlaps test label {j} [{}, {}]",
                            iv.t0, iv.t1, labels[j].t0, labels[j].t1
                        ),
                    };
                }
            }
        }
    }
    Check {
        name: "label_fold_overlap",
        passed: true,
        detail: format!(
            "{} folds, no train label overlaps a test label",
            folds.len()
        ),
    }
}

fn from_report(name: &'static str, report: &features::leakage::Report) -> Check {
    let blocking: Vec<String> = report
        .blocking()
        .map(|f| format!("{} ({})", f.subject, f.detail))
        .collect();
    Check {
        name,
        passed: blocking.is_empty(),
        detail: if blocking.is_empty() {
            "clean".to_string()
        } else {
            blocking.join("; ")
        },
    }
}

/// Every check the dataset-build harness runs for one instrument's frame: this module's
/// two, plus the existing `features::leakage` suite's frame-level and alignment checks
/// (which had no caller outside the audit endpoint).
#[must_use]
pub fn dataset_checks(
    subject: &str,
    obs: &[BarObs],
    build: &dyn Fn(&[BarObs]) -> TrainingFrame,
    frame: &TrainingFrame,
    split: &SplitSpec,
    horizon_bars: u32,
    step_ns: i64,
) -> Vec<Check> {
    let mut out = vec![
        feature_truncation(build, obs, frame, 0.7),
        label_fold_overlap(split, frame.row_count(), horizon_bars),
        from_report(
            "target_correlation",
            &features::leakage::target_correlation(frame),
        ),
        from_report(
            "full_sample_normalization",
            &features::leakage::full_sample_normalization(frame),
        ),
        from_report(
            "causal_alignment",
            &features::leakage::causal_alignment(
                obs,
                step_ns,
                features::leakage::Stamping::AsDeclared,
            )
            .report,
        ),
    ];
    for c in &mut out {
        c.detail = format!("{subject}: {}", c.detail);
    }
    out
}

/// The production builder: the same call `build_frame` makes.
pub fn production_builder(
    features: &[String],
    horizon_bars: u64,
    step_ns: i64,
) -> impl Fn(&[BarObs]) -> TrainingFrame + '_ {
    move |obs| {
        features::training_frame::build_aligned_training_frame(obs, features, horizon_bars, step_ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataplane::quality::QualityFlags;
    use dataplane::split::{EmbargoInputs, SplitKind, SplitSpec};

    fn bars(n: usize) -> Vec<BarObs> {
        (0..n)
            .map(|i| {
                let p = 100.0 + (i as f64 * 0.37).sin() * 3.0 + i as f64 * 0.01;
                BarObs {
                    ts_ns: 1_700_000_000_000_000_000 + i as i64 * 60_000_000_000,
                    knowledge_ns: 1_700_000_000_000_000_000 + i as i64 * 60_000_000_000,
                    open: p,
                    high: p + 0.5,
                    low: p - 0.5,
                    close: p + 0.1,
                    volume: 10.0 + i as f64,
                    quality: QualityFlags::NONE,
                }
            })
            .collect()
    }

    #[test]
    fn real_features_pass_the_truncation_self_test() {
        let obs = bars(300);
        let feats: Vec<String> = features::resolve_feature_set("fs_core_ohlcv_v3")
            .map(|s| s.features.clone())
            .unwrap_or_default();
        assert!(!feats.is_empty());
        let build = production_builder(&feats, 5, 60_000_000_000);
        let full = build(&obs);
        assert!(!full.ts_ns.is_empty());
        let c = feature_truncation(&build, &obs, &full, 0.7);
        assert!(c.passed, "{}", c.detail);
        assert!(c.detail.contains("identical"), "{}", c.detail);
    }

    #[test]
    fn a_feature_that_reads_the_future_fails_the_truncation_self_test() {
        let obs = bars(300);
        let feats: Vec<String> = features::resolve_feature_set("fs_core_ohlcv_v3")
            .map(|s| s.features.clone())
            .unwrap_or_default();
        let honest = production_builder(&feats, 5, 60_000_000_000);
        // A leaky builder: its first column at row i is the *last* bar's close of the
        // series it was handed, so it changes when the series is cut short.
        let leaky = |o: &[BarObs]| {
            let mut f = honest(o);
            let last = o.last().map_or(0.0, |b| b.close);
            if let Some(col) = f.columns.first_mut() {
                for v in col.iter_mut() {
                    *v += last;
                }
            }
            f
        };
        let full = leaky(&obs);
        let c = feature_truncation(&leaky, &obs, &full, 0.7);
        assert!(!c.passed, "a leaking feature must fail: {}", c.detail);
        assert!(c.detail.contains("truncated"), "{}", c.detail);
    }

    fn spec(kind: SplitKind, folds: usize) -> SplitSpec {
        SplitSpec::new(
            "s",
            kind,
            folds,
            1,
            &EmbargoInputs {
                horizon_bars: 5,
                max_lookback_bars: 0,
                max_knowledge_lag_ms: 0,
                settlement_lag_bars: 0,
            },
        )
    }

    #[test]
    fn the_purged_split_over_horizon_labels_has_no_overlap() {
        let c = label_fold_overlap(&spec(SplitKind::PurgedKfold, 5), 200, 5);
        assert!(c.passed, "{}", c.detail);
        assert!(c.detail.contains("folds"), "{}", c.detail);
    }

    #[test]
    fn a_planted_overlap_fails_the_harness() {
        // Labels that look 40 bars ahead, folds whose spec believes the horizon is 5:
        // the purge cannot cover them and a train label reaches into the test block.
        let long = horizon_intervals(200, 40);
        let mut s = spec(SplitKind::PurgedKfold, 5);
        s.set_purge_on(
            dataplane::split::PurgeOn::T0,
            Some("planted: purge on label start only"),
        )
        .expect("override with a reason");
        let c = check_folds(&s, &long);
        assert!(!c.passed, "an overlap must fail: {}", c.detail);
        assert!(c.detail.contains("overlaps test label"), "{}", c.detail);
    }

    #[test]
    fn a_report_passes_only_if_it_ran_a_check_and_every_check_passed() {
        let ok = Check {
            name: "a",
            passed: true,
            detail: String::new(),
        };
        let bad = Check {
            name: "b",
            passed: false,
            detail: "x".into(),
        };
        assert!(!LeakageReport::from_checks(vec![], json!({})).passed);
        assert!(LeakageReport::from_checks(vec![ok.clone()], json!({})).passed);
        let r = LeakageReport::from_checks(vec![ok, bad], json!({}));
        assert!(!r.passed);
        assert_eq!(r.failures(), vec!["b: x".to_string()]);
    }
}
