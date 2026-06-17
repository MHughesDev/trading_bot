//! [`StudyResult`] — the distribution-first, **best-member-sealed** product of a
//! Study (spec §1.2, INV-2 / ADR-002).
//!
//! There is deliberately **no** `best_run`, `argmax`, or metric-ranked member
//! list. `member_run_ids` is returned in insertion order for provenance only.
//! A single config may be carried forward solely via the Study's pre-declared
//! [`SelectionRule`](super::config::SelectionRule), whose output is stored in
//! `carried_forward` — never an accessor the user reaches into for the peak.

use serde::{Deserialize, Serialize};

use crate::run::{MetricKind, RunConfig, RunId};

/// An empirical distribution over one metric across a Study's members.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Distribution {
    pub metric: MetricKind,
    /// The raw metric values (surviving, non-failed members), insertion order.
    pub dist: Vec<f64>,
    pub median: f64,
    /// Inter-quartile range `[q25, q75]`.
    pub iqr: [f64; 2],
    /// 5th-percentile (low) tail — what you should plan around (spec §1.2).
    pub worst_5pct: f64,
    /// Dispersion (sample standard deviation).
    pub spread: f64,
}

impl Distribution {
    /// Build a distribution from raw values. Empty input yields an all-zero
    /// distribution (a Study that produced no surviving members).
    #[must_use]
    pub fn from_values(metric: MetricKind, values: Vec<f64>) -> Self {
        if values.is_empty() {
            return Self {
                metric,
                dist: values,
                median: 0.0,
                iqr: [0.0, 0.0],
                worst_5pct: 0.0,
                spread: 0.0,
            };
        }
        let median = percentile(&values, 0.50);
        let q25 = percentile(&values, 0.25);
        let q75 = percentile(&values, 0.75);
        let worst_5pct = percentile(&values, 0.05);
        let n = values.len();
        let mean = values.iter().sum::<f64>() / n as f64;
        let spread = if n > 1 {
            (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
        } else {
            0.0
        };
        Self {
            metric,
            dist: values,
            median,
            iqr: [q25, q75],
            worst_5pct,
            spread,
        }
    }
}

/// Linear-interpolation percentile (`q` in `[0,1]`) over an unsorted slice.
#[must_use]
pub fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.len() == 1 {
        return v[0];
    }
    let rank = q.clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        v[lo]
    } else {
        let frac = rank - lo as f64;
        v[lo] * (1.0 - frac) + v[hi] * frac
    }
}

/// A study-kind-specific summary the funnel (Phase 4) reads. Shape, not a single
/// number: gates compare median + worst-5% + plateau, never a peak.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudyVerdict {
    /// Human-readable one-line summary, logged.
    pub summary: String,
    /// Distribution median > 0 (where "positive performance" is meaningful).
    pub positive_median: bool,
    /// Worst-5% above the survivability threshold the gate planned around.
    pub survivable_worst5: bool,
    /// For neighborhood/parameter studies: a broad plateau (true) vs an isolated
    /// spike (false); `None` where the notion does not apply.
    pub plateau: Option<bool>,
}

/// The distribution-first, sealed product of a Study (spec §1.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StudyResult {
    pub study_id: String,
    /// Provenance only — NOT ranked, NOT promotable. Insertion order.
    member_run_ids: Vec<RunId>,
    pub distribution: Distribution,
    pub verdict: StudyVerdict,
    /// How many Runs this Study added to the global trial counter (Phase 2).
    pub trial_delta: i64,
    /// INV-2: always true; the best member is not addressable through any field.
    pub sealed: bool,
    /// The pre-declared selection rule's output, if any — the *only* way one
    /// config is carried forward. Never an argmax.
    pub carried_forward: Option<RunConfig>,
    /// True if any member Run was `unsafe` (INV-1 propagation).
    pub unsafe_: bool,
}

impl StudyResult {
    /// Construct a sealed result. `sealed` is forced `true`; there is no API to
    /// unseal or to expose a metric-ranked member.
    #[must_use]
    pub fn new(
        study_id: String,
        member_run_ids: Vec<RunId>,
        distribution: Distribution,
        verdict: StudyVerdict,
        trial_delta: i64,
        carried_forward: Option<RunConfig>,
        unsafe_: bool,
    ) -> Self {
        Self {
            study_id,
            member_run_ids,
            distribution,
            verdict,
            trial_delta,
            sealed: true,
            carried_forward,
            unsafe_,
        }
    }

    /// Member run ids in **insertion order** (provenance/audit only). This is
    /// deliberately not sorted and there is no companion that returns the
    /// best-performing member (INV-2).
    #[must_use]
    pub fn members(&self) -> &[RunId] {
        &self.member_run_ids
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_ordered() {
        let v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert!((percentile(&v, 0.0) - 1.0).abs() < 1e-12);
        assert!((percentile(&v, 1.0) - 5.0).abs() < 1e-12);
        assert!((percentile(&v, 0.5) - 3.0).abs() < 1e-12);
        assert!(percentile(&v, 0.25) <= percentile(&v, 0.75));
    }

    #[test]
    fn distribution_stats_on_a_fixture() {
        let d = Distribution::from_values(MetricKind::Sharpe, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
        assert!((d.median - 2.0).abs() < 1e-12);
        assert!(d.worst_5pct <= d.median);
        assert!(d.iqr[0] <= d.iqr[1]);
        assert!(d.spread > 0.0);
    }

    #[test]
    fn empty_distribution_is_zeroed() {
        let d = Distribution::from_values(MetricKind::Sharpe, vec![]);
        assert_eq!(d.median, 0.0);
        assert_eq!(d.worst_5pct, 0.0);
    }
}
