//! Purged MDA and clustered feature importance (Set N, FEAT-006 §7, N-3.3).
//!
//! Mean-decrease-impurity importance is computed from a tree's own training
//! splits, so it rewards a feature for having been *used*, on data the model has
//! already memorised, and it splits the credit for one signal across every
//! correlated copy of it. MDA (mean decrease accuracy) asks the question the
//! scorecard cares about instead: **take the held-out data, destroy one feature's
//! information by permuting it, and see how much worse the model's out-of-sample
//! loss gets.**
//!
//! # Purged
//!
//! The held-out rows of each fold are the caller's: they come from a
//! `dataplane::split::SplitSpec::expand`, which already removed the rows whose
//! labels overlap the training window (purge on the label's end) and the embargo
//! after it. This function never chooses a test row, so it cannot reintroduce the
//! overlap that purging exists to remove, and a fold with no test rows is an error
//! rather than a silent zero.
//!
//! # Clustered
//!
//! Permuting one of five near-copies of a signal changes nothing — the model reads
//! the other four — so each copy looks unimportant while the signal is not.
//! [`correlation_clusters`] groups features whose absolute correlation exceeds a
//! threshold (single linkage), and [`purged_mda`] permutes a *whole cluster
//! together*, so the importance belongs to the information and not to a column.
//!
//! # Honest uncertainty
//!
//! Each fold contributes one importance estimate (itself the mean over `n_repeats`
//! permutations), and the reported standard error is across folds. A cluster whose
//! interval contains zero has not been shown to matter, and the result says so
//! rather than ranking it by a point estimate.

use backtest::rng::DetRng;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ImportanceError {
    #[error("no folds")]
    NoFolds,
    #[error("fold {0} has no test rows")]
    EmptyFold(usize),
    #[error("a row index is out of range or the rows are ragged")]
    BadRows,
    #[error("a cluster names a feature that does not exist, or two clusters share one")]
    BadClusters,
    #[error("n_repeats must be at least 1")]
    NoRepeats,
    #[error("a value is not finite")]
    NonFinite,
}

/// One fold: a fitted model and the held-out rows it is scored on.
pub struct FoldModel<'a> {
    /// Row indices into `x` and `y`, already purged and embargoed by the caller.
    pub test_rows: &'a [usize],
    /// The fitted model's prediction for one feature row.
    pub predict: &'a dyn Fn(&[f64]) -> f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MdaConfig {
    pub n_repeats: usize,
    pub seed: u64,
}

/// The importance of one cluster of features.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClusterImportance {
    pub features: Vec<usize>,
    /// Mean increase in out-of-sample squared error when the cluster is permuted.
    pub mean_increase: f64,
    /// Standard error of that mean across folds. `None` with a single fold.
    pub se: Option<f64>,
    /// Whether `mean_increase` is more than two standard errors above zero. With a
    /// single fold there is no error to compare against, so this is `false`.
    pub distinguishable_from_zero: bool,
}

/// Groups of features whose pairwise `|correlation|` exceeds `threshold`, by single
/// linkage. Every feature is in exactly one group; a feature correlated with
/// nothing is a group of one. Groups are sorted, and so are their members.
///
/// # Errors
/// Ragged or non-finite data.
pub fn correlation_clusters(
    x: &[Vec<f64>],
    threshold: f64,
) -> Result<Vec<Vec<usize>>, ImportanceError> {
    let d = x.first().map_or(0, Vec::len);
    if x.iter().any(|r| r.len() != d) {
        return Err(ImportanceError::BadRows);
    }
    if x.iter().flatten().any(|v| !v.is_finite()) {
        return Err(ImportanceError::NonFinite);
    }
    let n = x.len() as f64;
    let mean: Vec<f64> = (0..d)
        .map(|j| x.iter().map(|r| r[j]).sum::<f64>() / n)
        .collect();
    let sd: Vec<f64> = (0..d)
        .map(|j| (x.iter().map(|r| (r[j] - mean[j]).powi(2)).sum::<f64>() / n).sqrt())
        .collect();
    let mut parent: Vec<usize> = (0..d).collect();
    fn find(p: &mut [usize], a: usize) -> usize {
        let mut r = a;
        while p[r] != r {
            r = p[r];
        }
        let mut c = a;
        while p[c] != r {
            let next = p[c];
            p[c] = r;
            c = next;
        }
        r
    }
    for a in 0..d {
        for b in (a + 1)..d {
            if sd[a] > 0.0 && sd[b] > 0.0 {
                let cov = x
                    .iter()
                    .map(|r| (r[a] - mean[a]) * (r[b] - mean[b]))
                    .sum::<f64>()
                    / n;
                if (cov / (sd[a] * sd[b])).abs() > threshold {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    if ra != rb {
                        parent[rb.max(ra)] = rb.min(ra);
                    }
                }
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for j in 0..d {
        let r = find(&mut parent, j);
        groups.entry(r).or_default().push(j);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort();
    Ok(out)
}

/// Purged, clustered MDA with squared-error loss.
///
/// For each fold: score the model on its test rows, then for each cluster and each
/// repeat permute that cluster's columns **together** (one shared row permutation)
/// across the test rows and score again. The importance of a cluster in a fold is
/// the mean over repeats of `(permuted loss - base loss)`.
///
/// # Errors
/// No folds, an empty fold, ragged or non-finite data, malformed clusters, or no repeats.
pub fn purged_mda(
    x: &[Vec<f64>],
    y: &[f64],
    folds: &[FoldModel<'_>],
    clusters: &[Vec<usize>],
    cfg: MdaConfig,
) -> Result<Vec<ClusterImportance>, ImportanceError> {
    if folds.is_empty() {
        return Err(ImportanceError::NoFolds);
    }
    if cfg.n_repeats == 0 {
        return Err(ImportanceError::NoRepeats);
    }
    let d = x.first().map_or(0, Vec::len);
    if x.len() != y.len() || x.iter().any(|r| r.len() != d) {
        return Err(ImportanceError::BadRows);
    }
    if x.iter().flatten().any(|v| !v.is_finite()) || y.iter().any(|v| !v.is_finite()) {
        return Err(ImportanceError::NonFinite);
    }
    let mut seen = vec![false; d];
    for c in clusters {
        if c.is_empty() {
            return Err(ImportanceError::BadClusters);
        }
        for &j in c {
            if j >= d || seen[j] {
                return Err(ImportanceError::BadClusters);
            }
            seen[j] = true;
        }
    }
    for (i, f) in folds.iter().enumerate() {
        if f.test_rows.is_empty() {
            return Err(ImportanceError::EmptyFold(i));
        }
        if f.test_rows.iter().any(|&r| r >= x.len()) {
            return Err(ImportanceError::BadRows);
        }
    }

    let loss = |predict: &dyn Fn(&[f64]) -> f64, rows: &[Vec<f64>], ys: &[f64]| -> f64 {
        rows.iter()
            .zip(ys)
            .map(|(r, y)| (predict(r) - y).powi(2))
            .sum::<f64>()
            / ys.len() as f64
    };

    // per_fold[c][k] = importance of cluster c in fold k.
    let mut per_fold: Vec<Vec<f64>> = vec![Vec::with_capacity(folds.len()); clusters.len()];
    let mut rng = DetRng::new(cfg.seed);
    for fold in folds {
        let rows: Vec<Vec<f64>> = fold.test_rows.iter().map(|&r| x[r].clone()).collect();
        let ys: Vec<f64> = fold.test_rows.iter().map(|&r| y[r]).collect();
        let base = loss(fold.predict, &rows, &ys);
        for (ci, cluster) in clusters.iter().enumerate() {
            let mut total = 0.0;
            for _ in 0..cfg.n_repeats {
                // One permutation for the whole cluster, Fisher-Yates.
                let mut perm: Vec<usize> = (0..rows.len()).collect();
                for i in (1..perm.len()).rev() {
                    perm.swap(i, rng.below(i + 1));
                }
                let mut shuffled = rows.clone();
                for (i, &p) in perm.iter().enumerate() {
                    for &j in cluster {
                        shuffled[i][j] = rows[p][j];
                    }
                }
                total += loss(fold.predict, &shuffled, &ys) - base;
            }
            per_fold[ci].push(total / cfg.n_repeats as f64);
        }
    }

    Ok(summarize(clusters, &per_fold))
}

/// Aggregate per-fold importances: `per_fold[c][k]` is cluster `c`'s importance in fold
/// `k`. The standard error is across folds, and a cluster is distinguishable from zero only
/// when its mean is more than two standard errors above it; with one fold there is no error
/// and nothing is claimed.
///
/// This is the half of [`purged_mda`] that does not need a model, so that importances the
/// trainer computed against its own fitted models (which the platform cannot call row by
/// row) are summarised by the same rule rather than a second copy of it.
///
/// # Errors
/// A cluster count that does not match `per_fold`, or a non-finite value.
pub fn summarize_fold_importances(
    clusters: &[Vec<usize>],
    per_fold: &[Vec<f64>],
) -> Result<Vec<ClusterImportance>, ImportanceError> {
    if clusters.len() != per_fold.len() || per_fold.iter().any(Vec::is_empty) {
        return Err(ImportanceError::BadClusters);
    }
    if per_fold.iter().flatten().any(|v| !v.is_finite()) {
        return Err(ImportanceError::NonFinite);
    }
    Ok(summarize(clusters, per_fold))
}

fn summarize(clusters: &[Vec<usize>], per_fold: &[Vec<f64>]) -> Vec<ClusterImportance> {
    clusters
        .iter()
        .zip(per_fold)
        .map(|(c, v)| {
            let k = v.len() as f64;
            let mean = v.iter().sum::<f64>() / k;
            let se = (v.len() > 1).then(|| {
                let var = v.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / (k - 1.0);
                (var / k).sqrt()
            });
            ClusterImportance {
                features: c.clone(),
                mean_increase: mean,
                se,
                distinguishable_from_zero: se.is_some_and(|s| mean > 2.0 * s),
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::many_single_char_names)]
mod tests {
    use super::*;

    /// y = x0 + noise; x1 is a near-copy of x0; x2 is noise; x3 is independent noise.
    fn data(n: usize, seed: u64) -> (Vec<Vec<f64>>, Vec<f64>) {
        let mut r = DetRng::new(seed);
        let mut g = || {
            let u1 = r.next_f64().max(1e-300);
            let u2 = r.next_f64();
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        };
        let mut x = Vec::new();
        let mut y = Vec::new();
        for _ in 0..n {
            let a = g();
            x.push(vec![a, a + 0.05 * g(), g(), g()]);
            y.push(a + 0.3 * g());
        }
        (x, y)
    }

    fn folds_of<'a>(rows: &'a [Vec<usize>], p: &'a dyn Fn(&[f64]) -> f64) -> Vec<FoldModel<'a>> {
        rows.iter()
            .map(|r| FoldModel {
                test_rows: r,
                predict: p,
            })
            .collect()
    }

    fn blocks(n: usize, k: usize) -> Vec<Vec<usize>> {
        (0..k)
            .map(|f| (f * n / k..(f + 1) * n / k).collect())
            .collect()
    }

    #[test]
    fn near_copies_are_one_cluster_and_independent_noise_is_not() {
        let (x, _) = data(600, 1);
        let c = correlation_clusters(&x, 0.9).unwrap();
        assert_eq!(c, vec![vec![0, 1], vec![2], vec![3]]);
        // At an impossible threshold every feature stands alone.
        assert_eq!(correlation_clusters(&x, 1.0).unwrap().len(), 4);
        assert!(correlation_clusters(&[vec![1.0], vec![1.0, 2.0]], 0.5).is_err());
    }

    #[test]
    fn the_informative_cluster_ranks_first_and_the_noise_cluster_contains_zero() {
        let (x, y) = data(1200, 2);
        // The model reads only x0, as a fitted model that found the signal would.
        let model = |r: &[f64]| r[0];
        let rows = blocks(1200, 6);
        let folds = folds_of(&rows, &model);
        let clusters = correlation_clusters(&x, 0.9).unwrap();
        let imp = purged_mda(
            &x,
            &y,
            &folds,
            &clusters,
            MdaConfig {
                n_repeats: 5,
                seed: 9,
            },
        )
        .unwrap();
        let by = |f: usize| imp.iter().find(|c| c.features.contains(&f)).unwrap();
        assert!(by(0).distinguishable_from_zero, "{:?}", by(0));
        assert!(
            by(0).mean_increase > 1.0,
            "permuting the signal at least doubles the error"
        );
        // The noise clusters are not distinguishable from zero: they have no effect on this model.
        assert!(!by(2).distinguishable_from_zero);
        assert!(
            by(2).mean_increase.abs() < 1e-12,
            "the model never reads x2, so permuting it changes nothing"
        );
        assert!(!by(3).distinguishable_from_zero);
        // Ranking: the signal's cluster is the largest.
        let top = imp
            .iter()
            .max_by(|a, b| a.mean_increase.total_cmp(&b.mean_increase))
            .unwrap();
        assert!(top.features.contains(&0));
    }

    #[test]
    fn clustering_credits_the_information_not_the_column() {
        let (x, y) = data(1200, 3);
        // A model that reads the average of the two copies.
        let model = |r: &[f64]| 0.5 * (r[0] + r[1]);
        let rows = blocks(1200, 6);
        let folds = folds_of(&rows, &model);
        let cfg = MdaConfig {
            n_repeats: 5,
            seed: 4,
        };

        // Permuting one copy at a time: each looks only half as important...
        let singles: Vec<Vec<usize>> = (0..4).map(|j| vec![j]).collect();
        let one_by_one = purged_mda(&x, &y, &folds, &singles, cfg).unwrap();
        // ...while permuting them together shows the signal's real weight.
        let grouped =
            purged_mda(&x, &y, &folds, &correlation_clusters(&x, 0.9).unwrap(), cfg).unwrap();
        let joint = grouped
            .iter()
            .find(|c| c.features == vec![0, 1])
            .unwrap()
            .mean_increase;
        assert!(
            joint > one_by_one[0].mean_increase * 1.5,
            "joint {joint} vs single {}",
            one_by_one[0].mean_increase
        );
        assert!(joint > one_by_one[1].mean_increase * 1.5);
    }

    #[test]
    fn the_same_seed_gives_the_same_importance_and_another_seed_a_different_one() {
        let (x, y) = data(400, 5);
        let model = |r: &[f64]| r[0];
        let rows = blocks(400, 4);
        let folds = folds_of(&rows, &model);
        let cl = vec![vec![0], vec![2]];
        let a = purged_mda(
            &x,
            &y,
            &folds,
            &cl,
            MdaConfig {
                n_repeats: 3,
                seed: 1,
            },
        )
        .unwrap();
        let b = purged_mda(
            &x,
            &y,
            &folds,
            &cl,
            MdaConfig {
                n_repeats: 3,
                seed: 1,
            },
        )
        .unwrap();
        let c = purged_mda(
            &x,
            &y,
            &folds,
            &cl,
            MdaConfig {
                n_repeats: 3,
                seed: 2,
            },
        )
        .unwrap();
        assert_eq!(a, b);
        assert_ne!(a[0].mean_increase, c[0].mean_increase);
    }

    #[test]
    fn a_single_fold_has_no_error_so_nothing_is_claimed() {
        let (x, y) = data(300, 6);
        let model = |r: &[f64]| r[0];
        let rows = vec![(0..300).collect::<Vec<_>>()];
        let folds = folds_of(&rows, &model);
        let imp = purged_mda(
            &x,
            &y,
            &folds,
            &[vec![0]],
            MdaConfig {
                n_repeats: 3,
                seed: 1,
            },
        )
        .unwrap();
        assert!(imp[0].se.is_none());
        assert!(!imp[0].distinguishable_from_zero);
        assert!(imp[0].mean_increase > 0.0);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_number() {
        let (x, y) = data(100, 7);
        let model = |r: &[f64]| r[0];
        let rows = blocks(100, 2);
        let folds = folds_of(&rows, &model);
        let cfg = MdaConfig {
            n_repeats: 2,
            seed: 1,
        };
        assert_eq!(
            purged_mda(&x, &y, &[], &[vec![0]], cfg),
            Err(ImportanceError::NoFolds)
        );
        assert_eq!(
            purged_mda(
                &x,
                &y,
                &folds,
                &[vec![0]],
                MdaConfig {
                    n_repeats: 0,
                    seed: 1
                }
            ),
            Err(ImportanceError::NoRepeats)
        );
        assert_eq!(
            purged_mda(&x, &y, &folds, &[vec![9]], cfg),
            Err(ImportanceError::BadClusters)
        );
        assert_eq!(
            purged_mda(&x, &y, &folds, &[vec![0], vec![0, 1]], cfg),
            Err(ImportanceError::BadClusters)
        );
        let empty: Vec<usize> = vec![];
        let bad = [FoldModel {
            test_rows: &empty,
            predict: &model,
        }];
        assert_eq!(
            purged_mda(&x, &y, &bad, &[vec![0]], cfg),
            Err(ImportanceError::EmptyFold(0))
        );
        let oob = vec![1000usize];
        let bad = [FoldModel {
            test_rows: &oob,
            predict: &model,
        }];
        assert_eq!(
            purged_mda(&x, &y, &bad, &[vec![0]], cfg),
            Err(ImportanceError::BadRows)
        );
        assert_eq!(
            purged_mda(&x, &y[..50], &folds, &[vec![0]], cfg),
            Err(ImportanceError::BadRows)
        );
    }
}
