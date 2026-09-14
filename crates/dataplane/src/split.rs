//! Split specifications: computed embargo, purge on `t1`, recorded overrides
//! (SPEC §3.5, §12.2; INV-15).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::label::LabelInterval;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitKind {
    WalkForward,
    PurgedKfold,
    Cpcv,
    Holdout,
    Sealed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "bars")]
pub enum TrainWindow {
    Expanding,
    Rolling(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeOn {
    /// Label END. The default and the only correct one.
    T1,
    /// Label start. Under-purges by the whole label horizon; override only.
    T0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbargoInputs {
    pub horizon_bars: u32,
    pub max_lookback_bars: u32,
    pub max_knowledge_lag_ms: u64,
    pub settlement_lag_bars: u32,
}

/// `horizon + max_lookback + ceil(max_lag_ms / 60_000) + settlement`, minimum
/// `horizon + 1`. Derived from the pipeline, never typed.
#[must_use]
pub fn compute_embargo(i: &EmbargoInputs) -> u32 {
    let lag_bars = i.max_knowledge_lag_ms.div_ceil(60_000) as u32;
    let e = i.horizon_bars + i.max_lookback_bars + lag_bars + i.settlement_lag_bars;
    e.max(i.horizon_bars + 1)
}

/// A recorded deviation from a computed/default value. Surfaces in every
/// comparison involving a trial that uses the spec.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SplitOverride {
    pub field: String,
    pub computed: String,
    pub requested: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitSpec {
    pub split_spec_id: String,
    pub kind: SplitKind,
    pub n_folds: usize,
    pub n_test_groups: usize,
    pub train_window: TrainWindow,
    embargo_bars: u32,
    computed_embargo_bars: u32,
    purge_on: PurgeOn,
    pub min_train_bars: usize,
    pub regime_stratified: bool,
    overrides: Vec<SplitOverride>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SplitError {
    #[error("lowering embargo_bars from computed {computed} to {requested} requires a written reason")]
    LoweringWithoutReason { computed: u32, requested: u32 },
    #[error("purge_on=t0 requires a written reason: it under-purges by the label horizon")]
    T0WithoutReason,
    #[error("invalid fold configuration: {0}")]
    Folds(String),
}

impl SplitSpec {
    /// The embargo is computed here; there is no constructor that accepts a typed one.
    #[must_use]
    pub fn new(split_spec_id: impl Into<String>, kind: SplitKind, n_folds: usize, n_test_groups: usize, inputs: &EmbargoInputs) -> Self {
        let e = compute_embargo(inputs);
        Self {
            split_spec_id: split_spec_id.into(),
            kind,
            n_folds,
            n_test_groups,
            train_window: TrainWindow::Expanding,
            embargo_bars: e,
            computed_embargo_bars: e,
            purge_on: PurgeOn::T1,
            min_train_bars: 0,
            regime_stratified: false,
            overrides: Vec::new(),
        }
    }

    /// Raise freely; lowering records an override and needs a reason.
    ///
    /// # Errors
    /// Lowering without a reason.
    pub fn set_embargo(&mut self, bars: u32, reason: Option<&str>) -> Result<(), SplitError> {
        if bars < self.computed_embargo_bars {
            let reason = reason.filter(|r| !r.trim().is_empty()).ok_or(SplitError::LoweringWithoutReason {
                computed: self.computed_embargo_bars,
                requested: bars,
            })?;
            self.overrides.push(SplitOverride {
                field: "embargo_bars".into(),
                computed: self.computed_embargo_bars.to_string(),
                requested: bars.to_string(),
                reason: reason.into(),
            });
        }
        self.embargo_bars = bars;
        Ok(())
    }

    ///
    /// # Errors
    /// `t0` without a reason.
    pub fn set_purge_on(&mut self, purge: PurgeOn, reason: Option<&str>) -> Result<(), SplitError> {
        if purge == PurgeOn::T0 {
            let reason = reason.filter(|r| !r.trim().is_empty()).ok_or(SplitError::T0WithoutReason)?;
            self.overrides.push(SplitOverride { field: "purge_on".into(), computed: "t1".into(), requested: "t0".into(), reason: reason.into() });
        }
        self.purge_on = purge;
        Ok(())
    }

    /// Re-key the spec by its own content (see [`crate::label::LabelSpec::content_keyed`]).
    ///
    /// # Panics
    /// Never in practice: every field is JSON-representable.
    #[must_use]
    pub fn content_keyed(mut self) -> Self {
        self.split_spec_id = String::new();
        let id = crate::hash::content_hash(&self).expect("split spec serializes");
        self.split_spec_id = id;
        self
    }

    #[must_use]
    pub fn embargo_bars(&self) -> u32 {
        self.embargo_bars
    }

    #[must_use]
    pub fn computed_embargo_bars(&self) -> u32 {
        self.computed_embargo_bars
    }

    #[must_use]
    pub fn purge_on(&self) -> PurgeOn {
        self.purge_on
    }

    #[must_use]
    pub fn overrides(&self) -> &[SplitOverride] {
        &self.overrides
    }

    /// Comparison-output flag (INV-15).
    #[must_use]
    pub fn comparison_flags(&self) -> Vec<String> {
        self.overrides.iter().map(|o| format!("override:{}={} (computed {}): {}", o.field, o.requested, o.computed, o.reason)).collect()
    }

    /// Expand to folds over `labels` (one per sample, in time order).
    ///
    /// # Errors
    /// Inconsistent fold parameters.
    pub fn expand(&self, labels: &[LabelInterval]) -> Result<Vec<Fold>, SplitError> {
        let n = labels.len();
        let groups = match self.kind {
            SplitKind::Cpcv | SplitKind::PurgedKfold | SplitKind::WalkForward => self.n_folds,
            SplitKind::Holdout | SplitKind::Sealed => 2,
        };
        if groups < 2 || groups > n {
            return Err(SplitError::Folds(format!("{groups} groups for {n} samples")));
        }
        let bounds: Vec<(usize, usize)> = (0..groups).map(|g| (g * n / groups, (g + 1) * n / groups)).collect();
        let test_sets: Vec<Vec<usize>> = match self.kind {
            SplitKind::Cpcv => {
                if self.n_test_groups == 0 || self.n_test_groups >= groups {
                    return Err(SplitError::Folds("cpcv requires 0 < n_test_groups < n_folds".into()));
                }
                combinations(groups, self.n_test_groups)
            }
            SplitKind::PurgedKfold => (0..groups).map(|g| vec![g]).collect(),
            SplitKind::WalkForward => (1..groups).map(|g| vec![g]).collect(),
            SplitKind::Holdout | SplitKind::Sealed => vec![vec![1]],
        };
        let mut folds = Vec::new();
        for tg in test_sets {
            let test: Vec<usize> = tg.iter().flat_map(|&g| bounds[g].0..bounds[g].1).collect();
            let blocks: Vec<(usize, usize)> = tg.iter().map(|&g| bounds[g]).collect();
            let first_test = blocks.iter().map(|b| b.0).min().unwrap_or(0);
            let mut train = Vec::new();
            for (i, iv) in labels.iter().enumerate() {
                if test.binary_search(&i).is_ok() {
                    continue;
                }
                if self.kind == SplitKind::WalkForward || self.kind == SplitKind::Holdout || self.kind == SplitKind::Sealed {
                    if i >= first_test {
                        continue;
                    }
                    if let TrainWindow::Rolling(w) = self.train_window {
                        if first_test.saturating_sub(i) > w {
                            continue;
                        }
                    }
                }
                if blocks.iter().any(|&(s, e)| self.purged(labels, *iv, s, e) || self.embargoed(i, e)) {
                    continue;
                }
                train.push(i);
            }
            if train.len() >= self.min_train_bars {
                folds.push(Fold { train, test });
            }
        }
        Ok(folds)
    }

    fn purged(&self, labels: &[LabelInterval], iv: LabelInterval, test_start: usize, test_end: usize) -> bool {
        let t_start = labels[test_start].t0;
        let t_end = labels[test_end - 1].t1;
        match self.purge_on {
            PurgeOn::T1 => iv.t0 <= t_end && iv.t1 >= t_start,
            PurgeOn::T0 => iv.t0 >= t_start && iv.t0 <= t_end,
        }
    }

    fn embargoed(&self, i: usize, test_end: usize) -> bool {
        i >= test_end && i < test_end + self.embargo_bars as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub train: Vec<usize>,
    pub test: Vec<usize>,
}

fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    fn rec(start: usize, n: usize, k: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if cur.len() == k {
            out.push(cur.clone());
            return;
        }
        for i in start..n {
            cur.push(i);
            rec(i + 1, n, k, cur, out);
            cur.pop();
        }
    }
    let mut out = Vec::new();
    rec(0, n, k, &mut Vec::new(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::label::horizon_intervals;

    /// AT-17.
    #[test]
    fn embargo_is_93_for_the_spec_example() {
        let i = EmbargoInputs { horizon_bars: 30, max_lookback_bars: 60, max_knowledge_lag_ms: 120_000, settlement_lag_bars: 1 };
        assert_eq!(compute_embargo(&i), 93);
        let mut s = SplitSpec::new("s", SplitKind::PurgedKfold, 5, 0, &i);
        assert_eq!(s.embargo_bars(), 93);
        assert_eq!(s.set_embargo(50, None), Err(SplitError::LoweringWithoutReason { computed: 93, requested: 50 }));
        s.set_embargo(50, Some("latency study")).unwrap();
        assert_eq!(s.comparison_flags().len(), 1, "the override surfaces in comparisons");
        s.set_embargo(200, None).unwrap();
    }

    #[test]
    fn minimum_is_horizon_plus_one() {
        let i = EmbargoInputs { horizon_bars: 5, max_lookback_bars: 0, max_knowledge_lag_ms: 0, settlement_lag_bars: 0 };
        assert_eq!(compute_embargo(&i), 6);
    }

    /// AT-18: purging on t1 removes a sample whose label ends in the test window
    /// even though it started before; purging on t0 misses it.
    #[test]
    fn purge_on_t1_catches_what_t0_misses() {
        let labels = horizon_intervals(100, 10);
        let inputs = EmbargoInputs { horizon_bars: 10, max_lookback_bars: 0, max_knowledge_lag_ms: 0, settlement_lag_bars: 0 };
        let s1 = SplitSpec::new("t1", SplitKind::PurgedKfold, 5, 0, &inputs);
        let folds = s1.expand(&labels).unwrap();
        // Fold 2 tests samples 40..60; sample 35 (label 35..45) overlaps it.
        let f2 = &folds[2];
        assert_eq!(f2.test.first(), Some(&40));
        assert!(!f2.train.contains(&35), "t1 purge removes the overlapping label");

        let mut s0 = SplitSpec::new("t0", SplitKind::PurgedKfold, 5, 0, &inputs);
        assert_eq!(s0.set_purge_on(PurgeOn::T0, None), Err(SplitError::T0WithoutReason));
        s0.set_purge_on(PurgeOn::T0, Some("demonstration")).unwrap();
        let folds0 = s0.expand(&labels).unwrap();
        assert!(folds0[2].train.contains(&35), "t0 purge under-purges by the horizon");
    }

    #[test]
    fn embargo_drops_post_test_samples() {
        let labels = horizon_intervals(100, 1);
        let inputs = EmbargoInputs { horizon_bars: 1, max_lookback_bars: 5, max_knowledge_lag_ms: 0, settlement_lag_bars: 0 };
        let s = SplitSpec::new("e", SplitKind::PurgedKfold, 5, 0, &inputs);
        let folds = s.expand(&labels).unwrap();
        let f0 = &folds[0]; // test 0..20
        assert!(!f0.train.contains(&20) && !f0.train.contains(&25));
        assert!(f0.train.contains(&30));
    }

    #[test]
    fn cpcv_paths() {
        let labels = horizon_intervals(60, 1);
        let inputs = EmbargoInputs { horizon_bars: 1, max_lookback_bars: 0, max_knowledge_lag_ms: 0, settlement_lag_bars: 0 };
        let s = SplitSpec::new("c", SplitKind::Cpcv, 6, 2, &inputs);
        let folds = s.expand(&labels).unwrap();
        assert_eq!(folds.len(), 15);
        for f in &folds {
            assert!(f.train.iter().all(|i| !f.test.contains(i)));
        }
    }

    #[test]
    fn walk_forward_trains_only_on_the_past() {
        let labels = horizon_intervals(50, 1);
        let inputs = EmbargoInputs { horizon_bars: 1, max_lookback_bars: 0, max_knowledge_lag_ms: 0, settlement_lag_bars: 0 };
        let s = SplitSpec::new("w", SplitKind::WalkForward, 5, 0, &inputs);
        for f in s.expand(&labels).unwrap() {
            let t0 = f.test[0];
            assert!(f.train.iter().all(|&i| i < t0));
        }
    }
}
