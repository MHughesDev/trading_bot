//! Platform-computed effective number of trials (SPEC §12.4, INV-22).
//!
//! `N_eff` is computed only here, from the return series stored against every trial
//! in a tenant's whole ledger — gate failures and exploration draws included — and
//! the result is a sealed type: [`NEff`] has no public constructor and does not
//! deserialize, so a caller cannot hand a gate a trial count of its choosing.
//!
//! Method: each series is compounded to UTC-daily returns; pairwise Pearson
//! correlation is taken over overlapping days; trials are clustered by
//! average-linkage agglomeration on distance `1 − ρ` (nearest-neighbour chain,
//! O(n²)), cut at `ρ = CLUSTER_CORRELATION`. `N_eff` is the number of clusters.
//! Pairs with too little overlap to estimate a correlation count as independent —
//! the direction that makes deflation harsher, never more lenient (ADR-P0-21).

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::LedgerError;

/// Average correlation at or above which trials are the same test.
pub const CLUSTER_CORRELATION: f64 = 0.5;
/// Overlapping daily observations required before a correlation is trusted.
pub const MIN_OVERLAP_DAYS: usize = 20;

/// An out-of-sample per-period return series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReturnSeries {
    pub timestamps: Vec<DateTime<Utc>>,
    pub returns: Vec<f64>,
}

impl ReturnSeries {
    /// Simple returns between consecutive equity points. Non-positive or non-finite
    /// equity breaks the chain at that point rather than inventing a return.
    #[must_use]
    pub fn from_equity(curve: &[(DateTime<Utc>, f64)]) -> Self {
        let mut s = Self { timestamps: Vec::new(), returns: Vec::new() };
        for w in curve.windows(2) {
            let ((_, prev), (t, now)) = (w[0], w[1]);
            if prev > 0.0 && prev.is_finite() && now.is_finite() && t > w[0].0 {
                s.timestamps.push(t);
                s.returns.push(now / prev - 1.0);
            }
        }
        s
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.returns.is_empty()
    }

    /// # Errors
    /// Mismatched lengths, unordered timestamps or non-finite returns.
    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.timestamps.len() != self.returns.len() {
            return Err(LedgerError::Invalid("return series timestamps and values differ in length".into()));
        }
        if self.timestamps.windows(2).any(|w| w[1] <= w[0]) {
            return Err(LedgerError::Invalid("return series timestamps must be strictly increasing".into()));
        }
        if self.returns.iter().any(|r| !r.is_finite()) {
            return Err(LedgerError::Invalid("return series contains a non-finite return".into()));
        }
        Ok(())
    }

    /// Canonical digest of the series, chained into the trial's terminal event.
    #[must_use]
    pub fn digest(&self) -> String {
        dataplane::content_hash(&(
            self.timestamps.iter().map(|t| t.timestamp_nanos_opt().unwrap_or(i64::MAX)).collect::<Vec<_>>(),
            self.returns.iter().map(|r| r.to_bits()).collect::<Vec<_>>(),
        ))
        .unwrap_or_default()
    }

    /// Compound to one return per UTC day.
    fn daily(&self) -> BTreeMap<NaiveDate, f64> {
        let mut out: BTreeMap<NaiveDate, f64> = BTreeMap::new();
        for (t, r) in self.timestamps.iter().zip(&self.returns) {
            let g = out.entry(t.date_naive()).or_insert(1.0);
            *g *= 1.0 + r;
        }
        for g in out.values_mut() {
            *g -= 1.0;
        }
        out
    }
}

/// The effective number of independent trials for a tenant. Minted only by a
/// ledger (`TrialLedger::n_eff`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct NEff {
    value: f64,
    series: usize,
    trials_counted: usize,
}

impl NEff {
    /// The effective trial count, ≥ 1.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Series that entered the clustering.
    #[must_use]
    pub fn series(&self) -> usize {
        self.series
    }

    /// Every trial on the tenant's ledger at computation time, whatever its outcome.
    #[must_use]
    pub fn trials_counted(&self) -> usize {
        self.trials_counted
    }

    pub(crate) fn compute(trials_counted: usize, series: &[ReturnSeries]) -> Self {
        let clusters = cluster_count(series);
        #[allow(clippy::cast_precision_loss)]
        let value = (clusters.max(1)) as f64;
        Self { value, series: series.len(), trials_counted }
    }
}

#[allow(clippy::cast_precision_loss)]
fn correlation(a: &BTreeMap<NaiveDate, f64>, b: &BTreeMap<NaiveDate, f64>) -> f64 {
    let pairs: Vec<(f64, f64)> = a.iter().filter_map(|(d, x)| b.get(d).map(|y| (*x, *y))).collect();
    if pairs.len() < MIN_OVERLAP_DAYS {
        return 0.0;
    }
    // Bit-identical over a trustworthy overlap is the same test, even when the
    // series has no variance and Pearson's ρ is undefined.
    if pairs.iter().all(|(x, y)| x.to_bits() == y.to_bits()) {
        return 1.0;
    }
    let n = pairs.len() as f64;
    let (mx, my) = (pairs.iter().map(|p| p.0).sum::<f64>() / n, pairs.iter().map(|p| p.1).sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in &pairs {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx).powi(2);
        syy += (y - my).powi(2);
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return 0.0;
    }
    (sxy / (sxx * syy).sqrt()).clamp(-1.0, 1.0)
}

/// Number of clusters after average-linkage agglomeration cut at
/// `1 − CLUSTER_CORRELATION`, via the nearest-neighbour chain algorithm.
fn cluster_count(series: &[ReturnSeries]) -> usize {
    let n = series.len();
    if n <= 1 {
        return n;
    }
    let daily: Vec<_> = series.iter().map(ReturnSeries::daily).collect();
    let mut dist = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let d = 1.0 - correlation(&daily[i], &daily[j]);
            dist[i][j] = d;
            dist[j][i] = d;
        }
    }
    let mut size = vec![1usize; n];
    let mut active = vec![true; n];
    let mut heights = Vec::with_capacity(n - 1);
    let mut chain: Vec<usize> = Vec::new();
    let mut remaining = n;
    while remaining > 1 {
        if chain.is_empty() {
            chain.push(active.iter().position(|a| *a).expect("an active cluster remains"));
        }
        let a = *chain.last().expect("chain is non-empty");
        let prev = chain.len().checked_sub(2).map(|i| chain[i]);
        // Nearest active neighbour; ties prefer the chain predecessor so the chain terminates.
        let mut best = prev;
        let mut best_d = prev.map_or(f64::INFINITY, |p| dist[a][p]);
        for (c, is_active) in active.iter().enumerate() {
            if *is_active && c != a && dist[a][c] < best_d {
                best_d = dist[a][c];
                best = Some(c);
            }
        }
        let b = best.expect("another active cluster exists");
        if Some(b) == prev {
            chain.pop();
            chain.pop();
            heights.push(best_d);
            // Lance–Williams update for average linkage: merge b into a.
            #[allow(clippy::cast_precision_loss)]
            let (sa, sb) = (size[a] as f64, size[b] as f64);
            for c in 0..n {
                if active[c] && c != a && c != b {
                    let d = (sa * dist[a][c] + sb * dist[b][c]) / (sa + sb);
                    dist[a][c] = d;
                    dist[c][a] = d;
                }
            }
            size[a] += size[b];
            active[b] = false;
            remaining -= 1;
        } else {
            chain.push(b);
        }
    }
    let cut = 1.0 - CLUSTER_CORRELATION;
    n - heights.iter().filter(|h| **h <= cut + 1e-12).count()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // N_eff values are whole cluster counts; exact comparison is the assertion.
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    struct Lcg(u64);
    impl Lcg {
        #[allow(clippy::cast_precision_loss)]
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64) - 0.5
        }
    }

    fn series(values: &[f64]) -> ReturnSeries {
        let t0 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        ReturnSeries { timestamps: (0..values.len()).map(|i| t0 + Duration::days(i as i64)).collect(), returns: values.to_vec() }
    }

    fn noise(seed: u64, n: usize, scale: f64) -> Vec<f64> {
        let mut g = Lcg(seed);
        (0..n).map(|_| g.next() * scale).collect()
    }

    #[test]
    fn a_sweep_over_one_idea_is_one_test() {
        let base = noise(1, 250, 0.02);
        let copies: Vec<ReturnSeries> = (0..60)
            .map(|k| series(&base.iter().zip(noise(100 + k, 250, 0.004)).map(|(b, e)| b + e).collect::<Vec<_>>()))
            .collect();
        assert_eq!(NEff::compute(60, &copies).value(), 1.0);
    }

    #[test]
    fn independent_ideas_are_counted_separately() {
        let ideas: Vec<ReturnSeries> = (0..40).map(|k| series(&noise(7 + k * 13, 250, 0.02))).collect();
        let n = NEff::compute(40, &ideas).value();
        assert!(n >= 38.0, "independent series must not be pooled: {n}");
    }

    #[test]
    fn two_ideas_swept_are_two_tests() {
        let a = noise(3, 300, 0.02);
        let b = noise(4, 300, 0.02);
        let mut all = Vec::new();
        for k in 0..25 {
            all.push(series(&a.iter().zip(noise(500 + k, 300, 0.003)).map(|(x, e)| x + e).collect::<Vec<_>>()));
            all.push(series(&b.iter().zip(noise(900 + k, 300, 0.003)).map(|(x, e)| x + e).collect::<Vec<_>>()));
        }
        assert_eq!(NEff::compute(50, &all).value(), 2.0);
    }

    /// AT-28: every trial is counted, gate failures included; too little overlap
    /// never pools series.
    #[test]
    fn counts_every_trial_and_never_pools_on_thin_overlap() {
        let failing: Vec<ReturnSeries> = (0..100).map(|k| series(&noise(k, 30, 0.01))).collect();
        let n = NEff::compute(100, &failing);
        assert_eq!(n.trials_counted(), 100);
        assert_eq!(n.series(), 100);
        let short = noise(9, 10, 0.01);
        let thin = vec![series(&short), series(&short)];
        assert_eq!(NEff::compute(2, &thin).value(), 2.0, "identical but unestimable ⇒ independent");
        assert_eq!(NEff::compute(5, &[]).value(), 1.0);
    }

    #[test]
    fn identical_series_are_one_test_even_without_variance() {
        let flat = vec![series(&[0.001; 60]), series(&[0.001; 60]), series(&[0.001; 60])];
        assert_eq!(NEff::compute(3, &flat).value(), 1.0);
        let other_flat = vec![series(&[0.001; 60]), series(&[0.002; 60])];
        assert_eq!(NEff::compute(2, &other_flat).value(), 2.0, "different constant series share nothing estimable");
    }

    #[test]
    fn equity_to_returns_skips_broken_points() {
        let t0 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let curve = vec![(t0, 100.0), (t0 + Duration::days(1), 110.0), (t0 + Duration::days(2), 0.0), (t0 + Duration::days(3), 50.0)];
        let s = ReturnSeries::from_equity(&curve);
        assert_eq!(s.returns.len(), 2);
        assert!((s.returns[0] - 0.1).abs() < 1e-12);
        assert!(s.validate().is_ok());
        assert_ne!(s.digest(), series(&[0.1]).digest());
    }
}
