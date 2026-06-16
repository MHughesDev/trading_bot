//! PURE walk-forward fold generator (Set I, Phase 0, ADR-0017).
//!
//! Given the length of a dataset index (sorted by `available_time`) and a
//! [`WalkForwardSpec`], produce concrete **train / calibration / test** index
//! ranges per fold with **purge** at every role boundary and **embargo** between
//! folds. This lives in the PURE `features` crate so it has no I/O, is trivially
//! unit-testable, and produces identical splits live and in replay — Rust owns
//! the geometry and hands index ranges to the sidecar, which never picks its own
//! split (leakage-safety by construction).
//!
//! Roles march forward through the index:
//!
//! ```text
//!  … train …│purge│ cal …│purge│ test …│ embargo │ (next fold) …
//! ```
//!
//! **Purge is enforced to be ≥ the label horizon** internally
//! (`effective_purge = max(spec.purge_bars, horizon_bars)`), so no train/cal row's
//! forward label window `[i, i+horizon]` can ever reach into a later role — the
//! leakage property holds regardless of how the spec author set `purge_bars`.

use std::ops::Range;

use domain::model_def::cv::{WalkForwardSpec, WindowMode};

/// One walk-forward fold: half-open index ranges into the dataset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    /// Zero-based fold ordinal.
    pub index: u32,
    pub train: Range<usize>,
    pub cal: Range<usize>,
    pub test: Range<usize>,
}

/// Why fold generation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldError {
    /// `spec.folds == 0` or a zero-length role; the spec is structurally invalid.
    InvalidSpec(String),
    /// The dataset is too short to fit every fold's test window.
    InsufficientHistory { needed: usize, have: usize },
}

impl std::fmt::Display for FoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FoldError::InvalidSpec(m) => write!(f, "invalid walk-forward spec: {m}"),
            FoldError::InsufficientHistory { needed, have } => {
                write!(f, "insufficient history: need {needed} bars, have {have}")
            }
        }
    }
}

impl std::error::Error for FoldError {}

/// Compute walk-forward folds over an index of `index_len` rows (sorted by
/// `available_time`). `horizon_bars` is the label look-ahead in base-timeframe
/// bars; it sets the minimum purge.
pub fn walk_forward_folds(
    index_len: usize,
    spec: &WalkForwardSpec,
    horizon_bars: u64,
) -> Result<Vec<Fold>, FoldError> {
    if spec.folds == 0 {
        return Err(FoldError::InvalidSpec("folds must be ≥ 1".into()));
    }
    if spec.train_bars == 0 || spec.cal_bars == 0 || spec.test_bars == 0 {
        return Err(FoldError::InvalidSpec(
            "train_bars, cal_bars and test_bars must all be > 0".into(),
        ));
    }

    let train_bars = spec.train_bars as usize;
    let cal_bars = spec.cal_bars as usize;
    let test_bars = spec.test_bars as usize;
    let embargo = spec.embargo_bars as usize;
    // Enforce purge ≥ horizon so label windows never cross a role boundary.
    let purge = (spec.purge_bars as usize).max(horizon_bars as usize);

    // Each fold is laid end-to-end: train · purge · cal · purge · test · embargo.
    // The stride advances the fold's anchor forward by the full block + embargo,
    // so test windows never overlap. Expanding mode reuses the same cal/test
    // anchors but grows train back to the start of the index.
    let block = train_bars + purge + cal_bars + purge + test_bars;
    let stride = block + embargo;

    let mut folds = Vec::with_capacity(spec.folds as usize);
    for f in 0..spec.folds {
        let base = (f as usize) * stride;

        let train_anchor_end = base + train_bars; // rolling train end / expanding cutoff
        let cal_start = train_anchor_end + purge;
        let cal_end = cal_start + cal_bars;
        let test_start = cal_end + purge;
        let test_end = test_start + test_bars;

        let train = match spec.mode {
            WindowMode::Rolling => base..train_anchor_end,
            // Expanding: from the very start up to the same purge-gap before cal.
            WindowMode::Expanding => 0..train_anchor_end,
        };

        folds.push(Fold {
            index: f,
            train,
            cal: cal_start..cal_end,
            test: test_start..test_end,
        });
    }

    // The whole series must fit; the last fold's test is the furthest reach.
    let needed = folds.last().map(|fold| fold.test.end).unwrap_or(0);
    if needed > index_len {
        return Err(FoldError::InsufficientHistory {
            needed,
            have: index_len,
        });
    }

    Ok(folds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(mode: WindowMode, folds: u32) -> WalkForwardSpec {
        WalkForwardSpec {
            mode,
            folds,
            train_bars: 100,
            cal_bars: 20,
            test_bars: 20,
            purge_bars: 5,
            embargo_bars: 5,
        }
    }

    #[test]
    fn rejects_zero_folds() {
        let mut s = spec(WindowMode::Expanding, 5);
        s.folds = 0;
        assert!(matches!(
            walk_forward_folds(10_000, &s, 5),
            Err(FoldError::InvalidSpec(_))
        ));
    }

    #[test]
    fn errors_when_history_too_short() {
        let s = spec(WindowMode::Rolling, 5);
        assert!(matches!(
            walk_forward_folds(50, &s, 5),
            Err(FoldError::InsufficientHistory { .. })
        ));
    }

    #[test]
    fn produces_requested_fold_count() {
        let s = spec(WindowMode::Expanding, 5);
        let folds = walk_forward_folds(10_000, &s, 5).unwrap();
        assert_eq!(folds.len(), 5);
        assert_eq!(folds[0].index, 0);
        assert_eq!(folds[4].index, 4);
    }

    #[test]
    fn expanding_train_starts_at_zero_and_grows() {
        let s = spec(WindowMode::Expanding, 5);
        let folds = walk_forward_folds(10_000, &s, 5).unwrap();
        for fold in &folds {
            assert_eq!(fold.train.start, 0, "expanding train always starts at 0");
        }
        // Train window grows strictly across folds.
        for w in folds.windows(2) {
            assert!(w[1].train.end > w[0].train.end);
        }
    }

    #[test]
    fn rolling_train_is_fixed_length_and_slides() {
        let s = spec(WindowMode::Rolling, 5);
        let folds = walk_forward_folds(10_000, &s, 5).unwrap();
        for fold in &folds {
            assert_eq!(fold.train.len(), 100, "rolling train is fixed length");
        }
        for w in folds.windows(2) {
            assert!(
                w[1].train.start > w[0].train.start,
                "rolling train slides forward"
            );
        }
    }

    /// Property sweep: across many spec shapes and both modes, every generated
    /// fold must satisfy the leakage invariants.
    #[test]
    fn leakage_and_disjointness_invariants_hold() {
        let horizons = [1u64, 5, 13];
        let modes = [WindowMode::Expanding, WindowMode::Rolling];
        for &mode in &modes {
            for &horizon in &horizons {
                for folds in 1..=6u32 {
                    for &(tr, ca, te, pu, em) in &[
                        (100u64, 20u64, 20u64, 0u64, 13u64),
                        (250, 50, 40, 7, 13),
                        (80, 10, 30, 20, 20),
                    ] {
                        let s = WalkForwardSpec {
                            mode,
                            folds,
                            train_bars: tr,
                            cal_bars: ca,
                            test_bars: te,
                            purge_bars: pu,
                            embargo_bars: em.max(horizon),
                        };
                        let generated = walk_forward_folds(1_000_000, &s, horizon).unwrap();
                        assert_eq!(generated.len() as u32, folds);

                        for fold in &generated {
                            // (a) Roles are ordered and non-empty.
                            assert!(fold.train.end <= fold.cal.start);
                            assert!(fold.cal.end <= fold.test.start);
                            assert!(!fold.train.is_empty());
                            assert!(!fold.cal.is_empty());
                            assert!(!fold.test.is_empty());

                            // (b) Within a fold no index appears in two roles.
                            assert!(fold.train.end <= fold.cal.start);
                            assert!(fold.cal.end <= fold.test.start);

                            // (c) No train/cal row's label window [i, i+horizon]
                            //     reaches into a later role.
                            let h = horizon as usize;
                            if let Some(last_train) = fold.train.clone().last() {
                                assert!(last_train + h < fold.cal.start);
                            }
                            if let Some(last_cal) = fold.cal.clone().last() {
                                assert!(last_cal + h < fold.test.start);
                            }
                        }

                        // (d) Embargo gap between consecutive folds' test→train
                        //     (meaningful for rolling, where train slides forward).
                        if mode == WindowMode::Rolling {
                            for w in generated.windows(2) {
                                assert!(
                                    w[1].train.start >= w[0].test.end + s.embargo_bars as usize
                                );
                            }
                        }
                        // Test windows never overlap across folds, either mode.
                        for w in generated.windows(2) {
                            assert!(w[1].test.start >= w[0].test.end);
                        }
                    }
                }
            }
        }
    }
}
