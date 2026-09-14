//! Gate evaluators 9, 11, 12 and 13/14's verdict wrappers (SPEC §12.3; plan 2.14).
//!
//! Everything here is arithmetic over evidence the ledger already holds — the
//! stored return series, the trade list, the equity curve. None of it dispatches
//! a Run, so none of it touches the trial counter (ADR-P2-17).
//!
//! Each evaluator returns a [`GateOutcome`] rather than a bare `bool`, because a
//! gate that only says "failed" cannot be argued with. The statistic, the
//! threshold it was compared against and the reason are all part of the verdict,
//! and they are what `mlops.gate_verdict` stores.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::profile::{CrisisWindow, GateProfile};
use crate::stats::bootstrap::{self, BootstrapError};

/// One gate's answer, with everything needed to re-derive it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateOutcome {
    pub gate_no: i32,
    pub gate_name: &'static str,
    pub passed: bool,
    /// The number that decided it. `None` for a structural gate, or for one that
    /// could not run.
    pub statistic: Option<f64>,
    pub threshold: Option<f64>,
    pub detail: String,
    /// True when the gate could not be evaluated at all. **This is not a pass.**
    /// A caller that treats `!passed` as the only failure mode will read an
    /// un-runnable gate as a failure, which is right; one that treats
    /// `inconclusive` as a pass has defeated the gate.
    pub inconclusive: bool,
}

impl GateOutcome {
    fn decided(
        gate_no: i32,
        gate_name: &'static str,
        passed: bool,
        statistic: f64,
        threshold: f64,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            gate_no,
            gate_name,
            passed,
            statistic: Some(statistic),
            threshold: Some(threshold),
            detail: detail.into(),
            inconclusive: false,
        }
    }

    /// A gate with no statistic: it either holds or it does not.
    ///
    /// Public because the stack assembles structural verdicts (pre-registration)
    /// that no single evaluator owns.
    #[must_use]
    pub fn structural_pass(
        gate_no: i32,
        gate_name: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            gate_no,
            gate_name,
            passed: true,
            statistic: None,
            threshold: None,
            detail: detail.into(),
            inconclusive: false,
        }
    }

    /// [`Self::decided`], for the stack.
    #[must_use]
    pub fn decided_pub(
        gate_no: i32,
        gate_name: &'static str,
        passed: bool,
        statistic: f64,
        threshold: f64,
        detail: impl Into<String>,
    ) -> Self {
        Self::decided(gate_no, gate_name, passed, statistic, threshold, detail)
    }

    /// [`Self::inconclusive`], for the stack. Never a pass.
    #[must_use]
    pub fn inconclusive_pub(
        gate_no: i32,
        gate_name: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self::inconclusive(gate_no, gate_name, detail)
    }

    /// The gate could not run. Never a pass.
    fn inconclusive(gate_no: i32, gate_name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            gate_no,
            gate_name,
            passed: false,
            statistic: None,
            threshold: None,
            detail: detail.into(),
            inconclusive: true,
        }
    }

    /// Convert to the durable record. The caller supplies what the outcome does
    /// not know: which experiment, and the significance context INV-3 requires.
    #[must_use]
    pub fn to_record(&self, profile_id: &str) -> ledger::gates::GateRecord {
        match (self.statistic, self.threshold) {
            (Some(s), Some(t)) => ledger::gates::GateRecord::measured(
                profile_id,
                self.gate_no,
                self.gate_name,
                self.passed,
                s,
                t,
                &self.detail,
            ),
            _ => ledger::gates::GateRecord::structural(
                profile_id,
                self.gate_no,
                self.gate_name,
                self.passed,
                &self.detail,
            ),
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 9 — minimum backtest length
// ───────────────────────────────────────────────────────────────────────────────

/// What Gate 9 needs. Every field is measured, none is asserted by a caller who
/// wants to pass.
#[derive(Clone, Copy, Debug)]
pub struct LengthInputs {
    /// Observed out-of-sample Sharpe (annualized, as the gate's formula assumes).
    pub sharpe: f64,
    /// Platform-computed effective trial count (INV-22). Never self-reported.
    pub n_eff: f64,
    /// Years of out-of-sample history the result rests on.
    pub years: f64,
    /// Non-overlapping events — trades, or labels whose horizons do not overlap.
    /// Overlapping labels are *not* independent events and counting them as such
    /// is the arithmetic this gate exists to prevent.
    pub independent_events: u32,
}

/// Gate 9 — "short backtest plus many trials" (§12.3, §1.10).
///
/// `SR ≥ 1.5·√(2·ln(N_eff)/y)` is the crude, free first-pass filter: with 10 000
/// trials on 5 years, a Sharpe of 1.92 is the *expected value of noise*, so a
/// result that does not clear a multiple of that boundary has demonstrated
/// nothing. The calendar floor and the event count come from the profile;
/// `paper_v1` drops the calendar floor and keeps both statistical ones.
#[must_use]
pub fn gate_9_min_length(profile: &GateProfile, i: LengthInputs) -> GateOutcome {
    const NAME: &str = "min_backtest_length";
    let t = profile.thresholds();

    if !(i.sharpe.is_finite() && i.years.is_finite()) || i.years <= 0.0 {
        return GateOutcome::inconclusive(9, NAME, "no out-of-sample span to measure");
    }
    if i.n_eff < 1.0 {
        return GateOutcome::inconclusive(9, NAME, "N_eff is not available");
    }

    // The Sharpe expected from pure noise after N_eff looks over `years`.
    let noise_sr = (2.0 * i.n_eff.ln() / i.years).max(0.0).sqrt();
    let required = 1.5 * noise_sr;

    let mut failures: Vec<String> = Vec::new();
    if i.sharpe < required {
        failures.push(format!(
            "Sharpe {:.2} is below 1.5x the {:.2} expected from noise alone after {:.1} effective \
             looks over {:.1}y",
            i.sharpe, noise_sr, i.n_eff, i.years
        ));
    }
    if i.years < t.min_track_record_years {
        failures.push(format!(
            "{:.1}y of history is under the profile's {:.1}y floor",
            i.years, t.min_track_record_years
        ));
    }
    if i.independent_events < t.min_independent_events {
        failures.push(format!(
            "{} independent events is under the {} floor",
            i.independent_events, t.min_independent_events
        ));
    }

    let passed = failures.is_empty();
    GateOutcome::decided(
        9,
        NAME,
        passed,
        i.sharpe,
        required,
        if passed {
            format!(
                "Sharpe {:.2} clears 1.5x the {:.2} noise boundary; {:.1}y, {} independent events",
                i.sharpe, noise_sr, i.years, i.independent_events
            )
        } else {
            failures.join("; ")
        },
    )
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 11 — regime coverage
// ───────────────────────────────────────────────────────────────────────────────

/// A regime label for one out-of-sample day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolRegime {
    Low,
    Mid,
    High,
}

impl VolRegime {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low_vol",
            Self::Mid => "mid_vol",
            Self::High => "high_vol",
        }
    }
}

/// Label each day by the tercile of its trailing realized volatility.
///
/// The terciles are taken over the evaluation window itself. That is **not**
/// lookahead: nothing here is fed to a strategy. It is a description of the
/// period a result was earned in, computed after the fact, and the question it
/// answers — "did this cover more than one kind of market?" — is only meaningful
/// relative to the period in question. The rule tier of M7 (ADR-P3-02); the
/// filtered HMM replaces it later without changing this gate.
#[must_use]
pub fn vol_tercile_regimes(returns: &[(DateTime<Utc>, f64)], window: usize) -> Vec<VolRegime> {
    let n = returns.len();
    if n == 0 {
        return Vec::new();
    }
    let w = window.max(2).min(n.max(2));
    let mut vol = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(w - 1);
        let slice = &returns[lo..=i];
        let m = slice.iter().map(|(_, r)| r).sum::<f64>() / slice.len() as f64;
        let v = slice.iter().map(|(_, r)| (r - m).powi(2)).sum::<f64>() / slice.len().max(1) as f64;
        vol.push(v.sqrt());
    }
    let mut sorted = vol.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |p: f64| -> f64 {
        let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };
    let (t1, t2) = (q(1.0 / 3.0), q(2.0 / 3.0));
    vol.into_iter()
        .map(|v| {
            if v <= t1 {
                VolRegime::Low
            } else if v <= t2 {
                VolRegime::Mid
            } else {
                VolRegime::High
            }
        })
        .collect()
}

/// What Gate 11 measures, kept so the UI can show the shape rather than the bit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeCoverage {
    pub regimes_present: Vec<VolRegime>,
    /// Share of the strategy's *gains* earned in each regime.
    pub pnl_share: Vec<(VolRegime, f64)>,
    pub worst_regime_sharpe: f64,
    pub crisis_windows_covered: Vec<String>,
    /// Crisis windows the profile declares that this sample does not reach at
    /// all — reported so a short history is visibly short rather than silently
    /// counted as coverage.
    pub crisis_windows_out_of_span: Vec<String>,
    /// True when regimes were labelled from the strategy's own returns because
    /// no market series was supplied. A weaker reading, and the verdict says so.
    pub labelled_from_strategy: bool,
}

/// Gate 11 — regime coverage (§12.3, §2.4 of the financial-ML reference).
///
/// `market` is the series the **regimes** are read from; `strategy` is the
/// series the **P&L** is attributed with. They are different things and
/// conflating them breaks the gate in a specific way: a volatility-targeted
/// strategy has near-constant return volatility by construction, so labelling
/// regimes from its own returns finds one regime no matter what the market did.
/// A regime is a property of the market. Pass the underlying's (or the
/// benchmark's) returns, aligned day-for-day.
///
/// Four checks, and the third is the one that catches the most self-deception:
/// many "robust" strategies turn out to have made all their money in one
/// three-month window.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn gate_11_regime_coverage(
    profile: &GateProfile,
    asset_class: &str,
    market: &[(DateTime<Utc>, f64)],
    strategy: &[(DateTime<Utc>, f64)],
) -> GateOutcome {
    const NAME: &str = "regime_coverage";
    let t = profile.thresholds();

    let Some(ac) = profile.asset_class(asset_class) else {
        return GateOutcome::inconclusive(
            11,
            NAME,
            format!(
                "profile {} declares no crisis windows for asset class '{asset_class}'; a crisis \
                 count of zero from an undeclared list is not coverage",
                profile.profile_id()
            ),
        );
    };
    if strategy.len() < 60 {
        return GateOutcome::inconclusive(
            11,
            NAME,
            format!("{} out-of-sample days is too few to label regimes", strategy.len()),
        );
    }
    let labelled_from_strategy = market.is_empty();
    let source = if labelled_from_strategy { strategy } else { market };
    if source.len() != strategy.len() {
        return GateOutcome::inconclusive(
            11,
            NAME,
            format!(
                "the market series has {} days and the strategy {}; regimes can only be attributed \
                 across aligned series",
                source.len(),
                strategy.len()
            ),
        );
    }

    let labels = vol_tercile_regimes(source, 21);
    let coverage = measure_coverage(strategy, &labels, &ac.crisis_windows, labelled_from_strategy);

    if coverage.pnl_share.is_empty() {
        return GateOutcome::inconclusive(
            11,
            NAME,
            "the strategy earned nothing over the sample; there is no P&L to attribute",
        );
    }

    let mut failures: Vec<String> = Vec::new();
    let distinct = coverage.regimes_present.len();
    if distinct < t.walk_forward_min_regimes as usize {
        failures.push(format!(
            "{distinct} distinct volatility regimes, under the {} required",
            t.walk_forward_min_regimes
        ));
    }
    // "≥ 2 crisis windows in OOS **when the data spans them**" — a sample that
    // predates every declared crisis is short, not reckless, and the verdict says
    // which it is.
    let reachable = ac.crisis_windows.len() - coverage.crisis_windows_out_of_span.len();
    if reachable >= 2 && coverage.crisis_windows_covered.len() < 2 {
        failures.push(format!(
            "{} of {reachable} reachable crisis windows covered, under the 2 required",
            coverage.crisis_windows_covered.len()
        ));
    }
    let concentration = coverage
        .pnl_share
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0_f64, f64::max);
    if concentration > t.max_single_regime_pnl_share {
        let worst = coverage
            .pnl_share
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map_or("?", |(r, _)| r.as_str());
        failures.push(format!(
            "{:.0}% of gains came from {worst} alone, over the {:.0}% cap",
            concentration * 100.0,
            t.max_single_regime_pnl_share * 100.0
        ));
    }
    if coverage.worst_regime_sharpe <= -0.5 {
        failures.push(format!(
            "worst-regime Sharpe {:.2} is at or below -0.5",
            coverage.worst_regime_sharpe
        ));
    }

    let passed = failures.is_empty();
    let span_note = if coverage.crisis_windows_out_of_span.is_empty() {
        String::new()
    } else {
        format!(
            " ({} declared crisis windows predate this sample: {})",
            coverage.crisis_windows_out_of_span.len(),
            coverage.crisis_windows_out_of_span.join(", ")
        )
    };
    let source_note = if labelled_from_strategy {
        " [regimes labelled from the strategy's own returns — no market series supplied, which \
         understates regime variety for a volatility-targeted strategy]"
    } else {
        ""
    };
    GateOutcome::decided(
        11,
        NAME,
        passed,
        concentration,
        t.max_single_regime_pnl_share,
        if passed {
            format!(
                "{distinct} regimes, largest share {:.0}%, worst-regime Sharpe {:.2}, crises \
                 covered: {}{span_note}{source_note}",
                concentration * 100.0,
                coverage.worst_regime_sharpe,
                if coverage.crisis_windows_covered.is_empty() {
                    "none".to_string()
                } else {
                    coverage.crisis_windows_covered.join(", ")
                }
            )
        } else {
            format!("{}{span_note}{source_note}", failures.join("; "))
        },
    )
}

fn measure_coverage(
    strategy: &[(DateTime<Utc>, f64)],
    labels: &[VolRegime],
    crises: &[CrisisWindow],
    labelled_from_strategy: bool,
) -> RegimeCoverage {
    let mut present: Vec<VolRegime> = labels.to_vec();
    present.sort_unstable();
    present.dedup();

    // "No single regime contributes more than 50% of cumulative OOS P&L" is about
    // where the *money* came from, so the denominator is the sum of the regimes
    // that made money, not the gross activity. Using |returns| instead would make
    // every constant-size strategy fail on a market whose volatility varies,
    // because gross activity concentrates wherever volatility is highest — which
    // says nothing about concentration of profit.
    let mut nets: Vec<(VolRegime, f64)> = Vec::new();
    let mut worst = f64::INFINITY;
    for regime in &present {
        let slice: Vec<f64> = strategy
            .iter()
            .zip(labels)
            .filter(|(_, l)| *l == regime)
            .map(|((_, r), _)| *r)
            .collect();
        nets.push((*regime, slice.iter().sum::<f64>()));
        if slice.len() >= 2 {
            let m = slice.iter().sum::<f64>() / slice.len() as f64;
            let v = slice.iter().map(|r| (r - m).powi(2)).sum::<f64>() / (slice.len() as f64 - 1.0);
            if v > 0.0 {
                worst = worst.min(m / v.sqrt() * 252.0_f64.sqrt());
            }
        }
    }
    if !worst.is_finite() {
        worst = 0.0;
    }
    let gains: f64 = nets.iter().map(|(_, n)| n.max(0.0)).sum();
    let shares: Vec<(VolRegime, f64)> = if gains > 0.0 {
        nets.iter().map(|(r, n)| (*r, n.max(0.0) / gains)).collect()
    } else {
        Vec::new()
    };

    let (first, last) = (
        strategy.first().map(|(d, _)| d.date_naive()),
        strategy.last().map(|(d, _)| d.date_naive()),
    );
    let mut covered = Vec::new();
    let mut out_of_span = Vec::new();
    for c in crises {
        let overlaps_sample = match (first, last) {
            (Some(f), Some(l)) => c.from <= l && c.to >= f,
            _ => false,
        };
        if !overlaps_sample {
            out_of_span.push(c.label.clone());
            continue;
        }
        // Present only if the sample actually has observations inside the window,
        // not merely if the window falls between the endpoints.
        let days_inside = strategy
            .iter()
            .filter(|(d, _)| {
                let dd: NaiveDate = d.date_naive();
                dd >= c.from && dd <= c.to
            })
            .count();
        if days_inside >= 5 {
            covered.push(c.label.clone());
        }
    }

    RegimeCoverage {
        regimes_present: present,
        pnl_share: shares,
        worst_regime_sharpe: worst,
        crisis_windows_covered: covered,
        crisis_windows_out_of_span: out_of_span,
        labelled_from_strategy,
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 12 — perturbation robustness and concentration
// ───────────────────────────────────────────────────────────────────────────────

/// What Gate 12 needs.
#[derive(Clone, Debug)]
pub struct PerturbationInputs<'a> {
    /// The centre configuration's metric.
    pub center: f64,
    /// `(parameter value, metric)` across the neighbourhood sweep, in parameter
    /// order. The neighbourhood Study already produces this.
    pub neighbourhood: &'a [(f64, f64)],
    /// `(instrument, absolute P&L)` — from the per-instrument Parquet artifact
    /// (plan 5.2), never from a metric series.
    pub per_instrument_pnl: &'a [(String, f64)],
}

/// Gate 12 — flat optima generalize; spikes do not (§12.3).
///
/// Three checks: the median over ±perturbations holds up, no parameter cliff, and
/// no single instrument carries the result. A single-instrument strategy passes
/// the third trivially, and the verdict says so rather than silently counting it
/// as diversification.
#[must_use]
pub fn gate_12_perturbation(profile: &GateProfile, i: &PerturbationInputs<'_>) -> GateOutcome {
    const NAME: &str = "perturbation_robustness";
    let t = profile.thresholds();

    if i.neighbourhood.len() < 3 {
        return GateOutcome::inconclusive(
            12,
            NAME,
            "a neighbourhood needs at least three points to have a shape",
        );
    }

    let mut metrics: Vec<f64> = i.neighbourhood.iter().map(|(_, m)| *m).collect();
    metrics.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = metrics[metrics.len() / 2];

    let mut failures: Vec<String> = Vec::new();
    // "Median Sharpe over ±20% perturbations ≥ 0.7 × center Sharpe." Only
    // meaningful when the centre is positive; a negative centre makes the ratio
    // reverse sign and is a failure for a different reason.
    if i.center > 0.0 && median < 0.7 * i.center {
        failures.push(format!(
            "median neighbourhood metric {median:.3} is under 70% of the centre's {:.3}",
            i.center
        ));
    }

    // A cliff: one step of the parameter moves the metric by more than the whole
    // neighbourhood's spread. That is a spike, not an optimum.
    let spread = metrics.last().unwrap_or(&0.0) - metrics.first().unwrap_or(&0.0);
    let mut worst_step = 0.0_f64;
    for w in i.neighbourhood.windows(2) {
        let d = (w[1].1 - w[0].1).abs();
        worst_step = worst_step.max(d);
    }
    let cliff = spread > 0.0 && worst_step > spread * 0.6;
    if cliff {
        failures.push(format!(
            "a single parameter step moves the metric by {worst_step:.3} of a {spread:.3} total \
             spread — a cliff, not a plateau"
        ));
    }

    // Concentration.
    //
    // The profile's cap is 20%, which cannot be met by a universe of fewer than
    // five instruments: on three, the largest share is at least a third no matter
    // how evenly the P&L falls. So the effective cap is the looser of the
    // profile's bar and `1/n` — the check measures concentration *beyond what the
    // universe forces*, which is the only thing a strategy can control. When the
    // universe binds, the verdict says so rather than passing quietly
    // (ADR-P2-28).
    let total: f64 = i.per_instrument_pnl.iter().map(|(_, p)| p.abs()).sum();
    let n_instruments = i.per_instrument_pnl.len();
    let mut concentration = 0.0;
    let mut concentrated_in = String::new();
    let universe_floor = if n_instruments > 0 { 1.0 / n_instruments as f64 } else { 1.0 };
    let effective_cap = t.max_single_instrument_pnl_share.max(universe_floor);
    if n_instruments > 1 && total > 0.0 {
        for (name, pnl) in i.per_instrument_pnl {
            let share = pnl.abs() / total;
            if share > concentration {
                concentration = share;
                concentrated_in.clone_from(name);
            }
        }
        if concentration > effective_cap {
            failures.push(format!(
                "{:.0}% of P&L came from {concentrated_in} alone, over the {:.0}% cap",
                concentration * 100.0,
                effective_cap * 100.0
            ));
        }
    }

    let single_instrument_note = if n_instruments <= 1 {
        " (single-instrument strategy: the concentration check does not apply)".to_string()
    } else if universe_floor > t.max_single_instrument_pnl_share {
        format!(
            " (a {n_instruments}-instrument universe cannot go below {:.0}%, so the profile's \
             {:.0}% cap is relaxed to that floor)",
            universe_floor * 100.0,
            t.max_single_instrument_pnl_share * 100.0
        )
    } else {
        String::new()
    };
    let passed = failures.is_empty();
    GateOutcome::decided(
        12,
        NAME,
        passed,
        median,
        0.7 * i.center,
        if passed {
            format!(
                "median {median:.3} vs centre {:.3}, no cliff, largest instrument share {:.0}%{single_instrument_note}",
                i.center,
                concentration * 100.0
            )
        } else {
            format!("{}{single_instrument_note}", failures.join("; "))
        },
    )
}

// ───────────────────────────────────────────────────────────────────────────────
// Gates 13 and 14 — verdict wrappers over the bootstrap statistics
// ───────────────────────────────────────────────────────────────────────────────

/// Gate 13 — stationary-bootstrap 5th-percentile Sharpe > 0 (§12.3).
#[must_use]
pub fn gate_13_stationary_bootstrap(
    profile: &GateProfile,
    returns: &[f64],
    seed: u64,
) -> GateOutcome {
    const NAME: &str = "stationary_bootstrap";
    let t = profile.thresholds();
    match bootstrap::bootstrap_sharpe(returns, bootstrap::MIN_BOOTSTRAP_RESAMPLES, seed) {
        Ok(b) => {
            let passed = b.p05 > t.bootstrap_p05_sharpe_gt;
            GateOutcome::decided(
                13,
                NAME,
                passed,
                b.p05,
                t.bootstrap_p05_sharpe_gt,
                format!(
                    "5th-percentile Sharpe {:.3} over {} stationary-bootstrap resamples \
                     (observed {:.3}, median {:.3}, mean block {:.1} obs)",
                    b.p05, b.resamples, b.observed, b.median, b.mean_block
                ),
            )
        }
        Err(e) => GateOutcome::inconclusive(13, NAME, format!("bootstrap unavailable: {e}")),
    }
}

/// Gate 14 — Romano–Wolf stepdown against the full candidate family (§12.3).
///
/// `family` must be **every** candidate the research programme generated, not
/// the ones the agent liked. That is the whole point: the test is only honest
/// over the family that was actually searched.
#[must_use]
pub fn gate_14_romano_wolf(
    profile: &GateProfile,
    family: &[Vec<f64>],
    candidate_index: usize,
    seed: u64,
) -> GateOutcome {
    const NAME: &str = "romano_wolf";
    let t = profile.thresholds();
    match bootstrap::romano_wolf(family, t.romano_wolf_p_lt, bootstrap::MIN_STEPDOWN_RESAMPLES, seed)
    {
        Ok(sd) => match sd.for_index(candidate_index) {
            Some(v) => GateOutcome::decided(
                14,
                NAME,
                v.rejected,
                v.adjusted_p,
                t.romano_wolf_p_lt,
                format!(
                    "stepdown-adjusted p {:.4} (t = {:.2}) against a family of {}; {} of the \
                     family survived at alpha = {}",
                    v.adjusted_p,
                    v.t_stat,
                    family.len(),
                    sd.rejected_count(),
                    t.romano_wolf_p_lt
                ),
            ),
            None => GateOutcome::inconclusive(
                14,
                NAME,
                format!("candidate {candidate_index} is not in the family that was tested"),
            ),
        },
        Err(BootstrapError::FamilyTooSmall) => GateOutcome::inconclusive(
            14,
            NAME,
            "a family-wise test needs the whole candidate family; fewer than two were supplied",
        ),
        Err(e) => GateOutcome::inconclusive(14, NAME, format!("stepdown unavailable: {e}")),
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 2 — the leakage suite, wired
// ───────────────────────────────────────────────────────────────────────────────

/// The latest `mlops.leakage_run` for a subject, read as the gate reads it.
///
/// `checks_run` matters as much as the counts. A check that did not run is
/// absent from the row rather than recorded as a pass (migration 0049), so a
/// suite that silently stopped running half its checks would otherwise report a
/// clean sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct LeakageRun {
    pub subject: String,
    pub checks_run: Vec<String>,
    pub blocking_count: i64,
    pub flag_count: i64,
    pub finished_at: DateTime<Utc>,
}

/// Gate 2 — the leakage suite passed, in full, recently enough to be about this
/// dataset (§12.3).
///
/// `required_checks` is the suite's own manifest of what it can run. Any of them
/// missing from the stored row makes the gate **inconclusive**: nobody knows
/// whether the strategy leaks, and "nobody knows" is not "no".
///
/// `max_age` guards the other direction. A clean leakage run from before the
/// dataset was rebuilt is a statement about a different dataset.
#[must_use]
pub fn gate_2_leakage_suite(
    profile: &GateProfile,
    run: Option<&LeakageRun>,
    required_checks: &[&str],
    now: DateTime<Utc>,
    max_age: chrono::Duration,
) -> GateOutcome {
    const NAME: &str = "leakage_suite";
    let t = profile.thresholds();

    let Some(run) = run else {
        return GateOutcome::inconclusive(
            2,
            NAME,
            "no leakage run is recorded for this subject; the suite exists and has not been run",
        );
    };
    if now.signed_duration_since(run.finished_at) > max_age {
        return GateOutcome::inconclusive(
            2,
            NAME,
            format!(
                "the newest leakage run finished {} and is older than the {} the gate accepts; \
                 a clean result from before the dataset changed is about a different dataset",
                run.finished_at.format("%Y-%m-%d %H:%M"),
                humanise(max_age)
            ),
        );
    }

    let ran: BTreeSet<&str> = run.checks_run.iter().map(String::as_str).collect();
    let missing: Vec<&str> = required_checks.iter().copied().filter(|c| !ran.contains(c)).collect();
    if !missing.is_empty() {
        return GateOutcome::inconclusive(
            2,
            NAME,
            format!("the suite did not run {missing:?}; a check that did not run is not a check that passed"),
        );
    }

    // The pass rate is over the *blocking* findings: a flag is information, a
    // blocking finding is a leak.
    let total = required_checks.len() as f64;
    let clean = (total - run.blocking_count as f64).max(0.0);
    let rate = if total > 0.0 { clean / total } else { 0.0 };
    let passed = run.blocking_count == 0 && rate >= t.leakage_suite_pass_rate;

    GateOutcome::decided(
        2,
        NAME,
        passed,
        rate,
        t.leakage_suite_pass_rate,
        format!(
            "{} checks ran on `{}`: {} blocking, {} flagged",
            run.checks_run.len(),
            run.subject,
            run.blocking_count,
            run.flag_count
        ),
    )
}

fn humanise(d: chrono::Duration) -> String {
    let hours = d.num_hours();
    if hours >= 48 {
        format!("{} days", hours / 24)
    } else {
        format!("{hours} hours")
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 3 — cost sensitivity, wired to the cost ladder
// ───────────────────────────────────────────────────────────────────────────────

/// One rung of the cost ladder: the multiple of modelled costs that was applied,
/// and the net metric the strategy earned at it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostRung {
    pub multiple: f64,
    pub net_metric: f64,
}

/// The cost multiple at which the edge reaches zero, by linear interpolation
/// between the two rungs that bracket it.
///
/// `None` when the ladder never crosses zero — the edge survives everything that
/// was tried, which is not the same as surviving everything, so the gate treats
/// it as "beyond the measured range" rather than as an infinite breakeven.
#[must_use]
pub fn breakeven_multiple(ladder: &[CostRung]) -> Option<f64> {
    let mut rungs: Vec<CostRung> = ladder
        .iter()
        .copied()
        .filter(|r| r.multiple.is_finite() && r.net_metric.is_finite() && r.multiple > 0.0)
        .collect();
    if rungs.len() < 2 {
        return None;
    }
    rungs.sort_by(|a, b| a.multiple.total_cmp(&b.multiple));
    if rungs[0].net_metric <= 0.0 {
        // Already dead at the cheapest cost assumption tried.
        return Some(rungs[0].multiple);
    }
    for pair in rungs.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        if lo.net_metric > 0.0 && hi.net_metric <= 0.0 {
            let span = lo.net_metric - hi.net_metric;
            let t = if span.abs() < 1e-12 { 0.0 } else { lo.net_metric / span };
            return Some(lo.multiple + t * (hi.multiple - lo.multiple));
        }
    }
    None
}

/// Gate 3 — the edge survives a multiple of its modelled costs (§12.3).
///
/// The number that matters is not "is it profitable after costs" — every
/// backtest that reaches a gate is — but *how wrong the cost model has to be*
/// before the edge disappears. Three times is the bar because cost models are
/// routinely off by a factor of two on the instruments and sizes where it
/// matters, and an edge that dies at 1.5× was never an edge, it was an estimate
/// of the spread.
#[must_use]
pub fn gate_3_cost_sensitivity(profile: &GateProfile, ladder: &[CostRung]) -> GateOutcome {
    const NAME: &str = "cost_sensitivity";
    let t = profile.thresholds();

    if ladder.len() < 2 {
        return GateOutcome::inconclusive(
            3,
            NAME,
            "a cost ladder needs at least two rungs; one cost assumption is not a sensitivity",
        );
    }
    let highest = ladder.iter().map(|r| r.multiple).fold(f64::MIN, f64::max);

    match breakeven_multiple(ladder) {
        Some(b) => GateOutcome::decided(
            3,
            NAME,
            b >= t.min_breakeven_cost_multiple,
            b,
            t.min_breakeven_cost_multiple,
            format!(
                "the edge reaches zero at {b:.2}× modelled costs across {} rungs",
                ladder.len()
            ),
        ),
        None if highest >= t.min_breakeven_cost_multiple => GateOutcome::decided(
            3,
            NAME,
            true,
            highest,
            t.min_breakeven_cost_multiple,
            format!(
                "the edge survives every rung tried, the highest being {highest:.2}× modelled costs; \
                 the breakeven is beyond the measured range and is reported as {highest:.2}×"
            ),
        ),
        None => GateOutcome::inconclusive(
            3,
            NAME,
            format!(
                "the ladder tops out at {highest:.2}× and the edge has not died; the gate needs a \
                 rung at or beyond {:.1}× to say anything",
                t.min_breakeven_cost_multiple
            ),
        ),
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 4 — capacity and participation
// ───────────────────────────────────────────────────────────────────────────────

/// The square-root impact coefficient. Almgren et al.'s empirical constant, and
/// the reason this gate does not need a venue's order book: temporary impact
/// scales as `σ_d · √(Q/ADV)` across markets, and the constant in front is the
/// part that varies least.
pub const IMPACT_COEFFICIENT: f64 = 0.6;

/// What Gate 4 measures. Everything comes from bars and the trade list; nothing
/// is asserted by the caller.
#[derive(Clone, Debug)]
pub struct CapacityInputs<'a> {
    /// 20-day dollar ADV per instrument, from bars.
    pub adv_usd: &'a BTreeMap<String, f64>,
    /// Daily return volatility per instrument, from the same bars.
    pub daily_vol: &'a BTreeMap<String, f64>,
    /// Largest single-day traded notional per instrument, from the trade list.
    pub peak_daily_notional: &'a BTreeMap<String, f64>,
    /// `(AUM, net Sharpe)` from re-running the strategy at increasing size.
    /// Sorted or not; this sorts. Five to eight points is the intended shape,
    /// and each one is a counted run.
    pub sharpe_by_aum: &'a [(f64, f64)],
    /// The AUM the headline result was earned at.
    pub base_aum_usd: f64,
    /// Whether the ADV figures come from a single venue. They usually do on free
    /// data, and a single-venue ADV *understates* the real one, which makes this
    /// gate conservative rather than wrong — but the verdict has to say so.
    pub single_venue: bool,
}

/// Modelled temporary impact of trading `quantity_usd` against `adv_usd`, in
/// return units: `0.6 · σ_d · √(Q/ADV)`.
#[must_use]
pub fn square_root_impact(quantity_usd: f64, adv_usd: f64, daily_vol: f64) -> Option<f64> {
    (adv_usd > 0.0 && daily_vol.is_finite() && daily_vol >= 0.0 && quantity_usd >= 0.0)
        .then(|| IMPACT_COEFFICIENT * daily_vol * (quantity_usd / adv_usd).sqrt())
}

/// The AUM at which net Sharpe first falls to half its base value, by linear
/// interpolation between the measured points.
///
/// `None` when the curve never gets there — which is not a pass by default: the
/// gate reports that capacity is beyond the range that was measured, and a claim
/// about behaviour outside the measured range is not evidence.
#[must_use]
pub fn capacity_at_half_sharpe(curve: &[(f64, f64)]) -> Option<f64> {
    let mut points: Vec<(f64, f64)> = curve.iter().copied().filter(|(a, s)| a.is_finite() && s.is_finite() && *a > 0.0).collect();
    if points.len() < 2 {
        return None;
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let base = points[0].1;
    if base <= 0.0 {
        return None;
    }
    let target = base / 2.0;
    for pair in points.windows(2) {
        let (a0, s0) = pair[0];
        let (a1, s1) = pair[1];
        if s0 > target && s1 <= target {
            let span = s0 - s1;
            let t = if span.abs() < 1e-12 { 0.0 } else { (s0 - target) / span };
            return Some(a0 + t * (a1 - a0));
        }
    }
    None
}

/// Gate 4 — capacity and participation (§12.3).
///
/// Three questions, and they fail for different reasons:
///
/// * **Participation.** The largest day's notional against ADV. Above the hard
///   cap the backtest's fills are fiction — the strategy could not have traded
///   that much without moving the price it assumed.
/// * **Capacity.** The AUM at which net Sharpe halves. A strategy whose base
///   size is already most of its capacity has nowhere to go and is one crowded
///   quarter from being uneconomic.
/// * **Impact.** The modelled cost of the largest day, reported so the first two
///   numbers can be read against something physical.
#[must_use]
pub fn gate_4_capacity(profile: &GateProfile, i: &CapacityInputs<'_>) -> GateOutcome {
    const NAME: &str = "capacity_adv";
    let t = profile.thresholds();

    if i.adv_usd.is_empty() || i.peak_daily_notional.is_empty() {
        return GateOutcome::inconclusive(4, NAME, "no ADV or trade-list notional was supplied");
    }

    // Participation, per instrument, worst case.
    let mut worst: Option<(String, f64)> = None;
    for (instrument, notional) in i.peak_daily_notional {
        let Some(adv) = i.adv_usd.get(instrument).copied().filter(|a| *a > 0.0) else {
            return GateOutcome::inconclusive(
                4,
                NAME,
                format!("{instrument} was traded but has no ADV; capacity cannot be assessed on a guess"),
            );
        };
        let share = notional / adv;
        if worst.as_ref().is_none_or(|(_, w)| share > *w) {
            worst = Some((instrument.clone(), share));
        }
    }
    let (worst_name, worst_share) = worst.expect("non-empty");

    let venue_note = if i.single_venue {
        " (single-venue ADV, which understates the real one — this gate is conservative here)"
    } else {
        ""
    };

    if worst_share > t.adv_hard {
        return GateOutcome::decided(
            4,
            NAME,
            false,
            worst_share,
            t.adv_hard,
            format!(
                "{worst_name} takes {:.1}% of ADV on its largest day, over the {:.0}% hard cap: \
                 the backtest's fills assume a price this size would have moved{venue_note}",
                worst_share * 100.0,
                t.adv_hard * 100.0
            ),
        );
    }

    let Some(capacity) = capacity_at_half_sharpe(i.sharpe_by_aum) else {
        return GateOutcome::inconclusive(
            4,
            NAME,
            format!(
                "net Sharpe never halves across the {} AUM points measured; capacity is beyond the \
                 measured range, which is not the same as being large enough",
                i.sharpe_by_aum.len()
            ),
        );
    };
    let fraction = if capacity > 0.0 { i.base_aum_usd / capacity } else { f64::INFINITY };

    let impact = i
        .peak_daily_notional
        .get(&worst_name)
        .and_then(|q| {
            let adv = i.adv_usd.get(&worst_name)?;
            let vol = i.daily_vol.get(&worst_name)?;
            square_root_impact(*q, *adv, *vol)
        })
        .map_or_else(|| "unmodelled".to_string(), |x| format!("{:.1} bp", x * 10_000.0));

    let soft = if worst_share > t.adv_soft {
        format!(
            "; participation {:.1}% is over the {:.0}% soft cap and the result should be read at a smaller size",
            worst_share * 100.0,
            t.adv_soft * 100.0
        )
    } else {
        String::new()
    };

    let passed = fraction <= t.max_capacity_fraction_at_half_sharpe;
    GateOutcome::decided(
        4,
        NAME,
        passed,
        fraction,
        t.max_capacity_fraction_at_half_sharpe,
        format!(
            "base AUM ${:.0} is {:.0}% of the ${:.0} at which net Sharpe halves; worst participation \
             {worst_name} {:.1}% of ADV, modelled impact {impact}{soft}{venue_note}",
            i.base_aum_usd,
            fraction * 100.0,
            capacity,
            worst_share * 100.0
        ),
    )
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 10 — factor attribution
// ───────────────────────────────────────────────────────────────────────────────

/// What Gate 10 regresses.
#[derive(Clone, Debug)]
pub struct FactorInputs<'a> {
    /// The strategy's **net** returns, per period. Gross returns regressed on
    /// factors produce an alpha that pays no commission.
    pub strategy_net: &'a [f64],
    /// The factor returns, aligned period-for-period with `strategy_net`.
    pub factors: &'a BTreeMap<String, Vec<f64>>,
    /// Which factor is the market. Gate 10's neutrality check reads this one.
    pub market_factor: String,
    /// Whether the strategy claims to be market-neutral. A claim that is not
    /// made is not checked, but a claim that is made is.
    pub neutral_claim: bool,
}

/// What the regression found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactorFit {
    pub alpha: f64,
    pub alpha_t: f64,
    pub r_squared: f64,
    pub betas: BTreeMap<String, f64>,
    /// Newey–West lag length actually used.
    pub hac_lags: usize,
}

/// OLS with Newey–West HAC standard errors.
///
/// HAC rather than plain OLS standard errors because strategy returns are
/// autocorrelated and heteroskedastic, and both inflate t-statistics in the same
/// direction — toward finding alpha. The lag length is Newey and West's own
/// automatic rule, `floor(4·(n/100)^(2/9))`, so it is not a knob either.
///
/// Returns `None` when the design is rank-deficient or there is not enough data.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    // `y`, `X`, `n`, `k` and the sandwich's parts are the names every textbook
    // statement of this estimator uses. Spelling them out would make the code
    // harder to check against the algebra it implements, not easier.
    clippy::many_single_char_names
)]
pub fn newey_west_fit(y: &[f64], factors: &BTreeMap<String, Vec<f64>>) -> Option<FactorFit> {
    let n = y.len();
    let k = factors.len() + 1; // intercept
    if n < 3 * k || n < 30 {
        return None;
    }
    if factors.values().any(|f| f.len() != n) {
        return None;
    }

    // Design matrix: intercept first, then factors in name order.
    let names: Vec<&String> = factors.keys().collect();
    let x: Vec<Vec<f64>> = (0..n)
        .map(|r| {
            let mut row = Vec::with_capacity(k);
            row.push(1.0);
            for name in &names {
                row.push(factors[*name][r]);
            }
            row
        })
        .collect();

    // (X'X) and X'y
    let mut xtx = vec![vec![0.0_f64; k]; k];
    let mut xty = vec![0.0_f64; k];
    for r in 0..n {
        for a in 0..k {
            xty[a] += x[r][a] * y[r];
            for b in 0..k {
                xtx[a][b] += x[r][a] * x[r][b];
            }
        }
    }
    let xtx_inv = invert(&xtx)?;
    let beta: Vec<f64> = (0..k).map(|a| (0..k).map(|b| xtx_inv[a][b] * xty[b]).sum()).collect();

    // Residuals and R².
    let fitted: Vec<f64> = (0..n).map(|r| (0..k).map(|a| x[r][a] * beta[a]).sum()).collect();
    let resid: Vec<f64> = (0..n).map(|r| y[r] - fitted[r]).collect();
    let ybar = y.iter().sum::<f64>() / n as f64;
    let ss_tot: f64 = y.iter().map(|v| (v - ybar).powi(2)).sum();
    let ss_res: f64 = resid.iter().map(|e| e * e).sum();
    let r_squared = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };

    // Newey–West meat matrix with a Bartlett kernel.
    let lags = (4.0 * (n as f64 / 100.0).powf(2.0 / 9.0)).floor().max(0.0) as usize;
    let mut meat = vec![vec![0.0_f64; k]; k];
    for r in 0..n {
        for a in 0..k {
            for b in 0..k {
                meat[a][b] += resid[r] * resid[r] * x[r][a] * x[r][b];
            }
        }
    }
    for l in 1..=lags {
        let w = 1.0 - (l as f64) / (lags as f64 + 1.0);
        for r in l..n {
            for a in 0..k {
                for b in 0..k {
                    let cross = resid[r] * resid[r - l] * (x[r][a] * x[r - l][b] + x[r - l][a] * x[r][b]);
                    meat[a][b] += w * cross;
                }
            }
        }
    }

    // Sandwich: (X'X)⁻¹ · meat · (X'X)⁻¹, alpha's variance is element (0, 0).
    let mid: Vec<Vec<f64>> = (0..k)
        .map(|a| (0..k).map(|b| (0..k).map(|c| xtx_inv[a][c] * meat[c][b]).sum()).collect())
        .collect();
    let var_alpha: f64 = (0..k).map(|c| mid[0][c] * xtx_inv[c][0]).sum();
    if !(var_alpha.is_finite() && var_alpha > 0.0) {
        return None;
    }

    let betas = names.iter().enumerate().map(|(idx, name)| ((*name).clone(), beta[idx + 1])).collect();
    Some(FactorFit {
        alpha: beta[0],
        alpha_t: beta[0] / var_alpha.sqrt(),
        r_squared,
        betas,
        hac_lags: lags,
    })
}

/// Gauss–Jordan inversion of a small symmetric matrix.
#[allow(clippy::many_single_char_names)]
fn invert(m: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let k = m.len();
    let mut a: Vec<Vec<f64>> = m.to_vec();
    let mut inv: Vec<Vec<f64>> = (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();
    for col in 0..k {
        let pivot = (col..k).max_by(|&r1, &r2| a[r1][col].abs().total_cmp(&a[r2][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        inv.swap(col, pivot);
        let d = a[col][col];
        for j in 0..k {
            a[col][j] /= d;
            inv[col][j] /= d;
        }
        for r in 0..k {
            if r == col {
                continue;
            }
            let f = a[r][col];
            if f == 0.0 {
                continue;
            }
            for j in 0..k {
                a[r][j] -= f * a[col][j];
                inv[r][j] -= f * inv[col][j];
            }
        }
    }
    Some(inv)
}

/// Gate 10 — factor attribution (§12.3, ADR-P2-15).
///
/// The question is whether the strategy is doing anything the factor battery is
/// not already doing. Three answers have to line up:
///
/// * `t(α) ≥ 3.0`, Newey–West. Three, not two, because the trial counter says
///   how many candidates were looked at and 2.0 is the threshold that produced
///   the replication crisis.
/// * `R² < 0.7`. A regression that explains most of the variance has found a
///   levered factor portfolio with a name.
/// * `|β_market| ≤ 0.3` **when the strategy claims neutrality**. An unmade claim
///   is not checked; a made one is.
///
/// `asset_class` selects the declared battery from the profile. A strategy
/// regressed against the wrong battery has been tested against factors it was
/// never exposed to, so the battery has to match.
#[must_use]
pub fn gate_10_factor_attribution(
    profile: &GateProfile,
    asset_class: &str,
    i: &FactorInputs<'_>,
) -> GateOutcome {
    const NAME: &str = "factor_attribution";
    let t = profile.thresholds();

    let Some(declared) = profile.asset_class(asset_class) else {
        return GateOutcome::inconclusive(
            10,
            NAME,
            format!(
                "profile {} declares no factor battery for asset class `{asset_class}`; a strategy \
                 cannot be attributed against factors nobody chose",
                profile.profile_id()
            ),
        );
    };
    let supplied: BTreeSet<&str> = i.factors.keys().map(String::as_str).collect();
    let required: BTreeSet<&str> = declared.factor_battery.iter().map(String::as_str).collect();
    if supplied != required {
        let missing: Vec<&str> = required.difference(&supplied).copied().collect();
        let extra: Vec<&str> = supplied.difference(&required).copied().collect();
        return GateOutcome::inconclusive(
            10,
            NAME,
            format!(
                "the battery does not match the one `{asset_class}` declares (missing {missing:?}, \
                 unexpected {extra:?}); attribution against a different battery answers a different question"
            ),
        );
    }

    let Some(fit) = newey_west_fit(i.strategy_net, i.factors) else {
        return GateOutcome::inconclusive(
            10,
            NAME,
            format!(
                "the attribution regression is not estimable from {} net returns against {} factors",
                i.strategy_net.len(),
                i.factors.len()
            ),
        );
    };

    let mut failures: Vec<String> = Vec::new();
    if fit.alpha_t < t.alpha_t_stat_gte {
        failures.push(format!(
            "t(α) = {:.2} < {:.1}: the alpha is not distinguishable from the battery's noise",
            fit.alpha_t, t.alpha_t_stat_gte
        ));
    }
    if fit.r_squared >= t.factor_r2_lt {
        failures.push(format!(
            "R² = {:.2} ≥ {:.2}: most of the return is the battery, which is a factor portfolio with a strategy's name on it",
            fit.r_squared, t.factor_r2_lt
        ));
    }
    if i.neutral_claim {
        let beta = i.factors.get(&i.market_factor).map_or(f64::NAN, |_| {
            fit.betas.get(&i.market_factor).copied().unwrap_or(f64::NAN)
        });
        if !beta.is_finite() {
            return GateOutcome::inconclusive(
                10,
                NAME,
                format!("the strategy claims market neutrality but `{}` is not in the battery", i.market_factor),
            );
        }
        if beta.abs() > MAX_NEUTRAL_MARKET_BETA {
            failures.push(format!(
                "β({}) = {beta:+.2}, over ±{MAX_NEUTRAL_MARKET_BETA:.1} for a strategy that claims to be market-neutral",
                i.market_factor
            ));
        }
    }

    let summary = format!(
        "t(α) = {:.2} (NW {} lags), R² = {:.2}, betas {:?}",
        fit.alpha_t, fit.hac_lags, fit.r_squared, fit.betas
    );
    GateOutcome::decided(
        10,
        NAME,
        failures.is_empty(),
        fit.alpha_t,
        t.alpha_t_stat_gte,
        if failures.is_empty() { summary } else { format!("{summary}; {}", failures.join("; ")) },
    )
}

/// §12.3's neutrality bound. Not in the profile because it is part of what the
/// word "neutral" means, not a threshold a profile gets to soften.
pub const MAX_NEUTRAL_MARKET_BETA: f64 = 0.3;

// ───────────────────────────────────────────────────────────────────────────────
// Gate 15 — paper / shadow
// ───────────────────────────────────────────────────────────────────────────────

/// What the forward test measured, from the reconciliation crate.
#[derive(Clone, Copy, Debug)]
pub struct PaperInputs {
    /// Fraction of live signals that reproduce the backtest's signal exactly.
    pub signal_match: f64,
    /// Realized slippage over modelled slippage.
    pub slippage_ratio: f64,
    /// Realized turnover over expected turnover.
    pub turnover_ratio: f64,
    /// Rejects attributable to the model (not the venue, not connectivity).
    pub model_attributable_rejects: u32,
    pub observed_days: u32,
    /// Whether the kill criteria were written into the campaign **before**
    /// deployment (§12.6).
    pub kill_criteria_registered: bool,
}

/// Gate 15 — paper/shadow agreement (§12.3, §12.6).
///
/// The forward test is the only gate whose evidence cannot be manufactured by
/// re-running anything, which is exactly why it is here. Four measurements, and
/// one precondition that is not a measurement at all: the kill criteria have to
/// have been registered **before** the strategy was deployed. Deciding when to
/// stop after watching it run is not a stopping rule, it is a narrative, and the
/// gate refuses to produce a verdict without one rather than passing the other
/// four checks and calling that a result.
#[must_use]
pub fn gate_15_paper_shadow(profile: &GateProfile, i: PaperInputs) -> GateOutcome {
    const NAME: &str = "paper_shadow";
    let t = profile.thresholds();

    if !i.kill_criteria_registered {
        return GateOutcome::inconclusive(
            15,
            NAME,
            "no kill criteria were registered before deployment; a stopping rule chosen after \
             watching the result is not a stopping rule (§12.6)",
        );
    }
    if i.observed_days == 0 {
        return GateOutcome::inconclusive(15, NAME, "no forward-test days have been observed");
    }

    let mut failures: Vec<String> = Vec::new();
    if i.signal_match < t.paper_signal_match_gte {
        failures.push(format!(
            "signal reproduction {:.2}% < {:.2}%: the live system is not running the strategy that was tested",
            i.signal_match * 100.0,
            t.paper_signal_match_gte * 100.0
        ));
    }
    if i.slippage_ratio > t.paper_slippage_ratio_lte {
        failures.push(format!(
            "slippage {:.2}× modelled, over {:.2}×",
            i.slippage_ratio, t.paper_slippage_ratio_lte
        ));
    }
    if (i.turnover_ratio - 1.0).abs() > t.paper_turnover_tolerance {
        failures.push(format!(
            "turnover {:+.0}% against expectation, outside ±{:.0}%",
            (i.turnover_ratio - 1.0) * 100.0,
            t.paper_turnover_tolerance * 100.0
        ));
    }
    if i.model_attributable_rejects > t.paper_model_rejects_max {
        failures.push(format!(
            "{} model-attributable rejects, over {}",
            i.model_attributable_rejects, t.paper_model_rejects_max
        ));
    }

    let summary = format!(
        "{} days forward: signal {:.2}%, slippage {:.2}×, turnover {:+.0}%, {} model rejects",
        i.observed_days,
        i.signal_match * 100.0,
        i.slippage_ratio,
        (i.turnover_ratio - 1.0) * 100.0,
        i.model_attributable_rejects
    );
    GateOutcome::decided(
        15,
        NAME,
        failures.is_empty(),
        i.signal_match,
        t.paper_signal_match_gte,
        if failures.is_empty() { summary } else { format!("{summary}; {}", failures.join("; ")) },
    )
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 16 — the capital ramp
// ───────────────────────────────────────────────────────────────────────────────

/// Gate 16 — the requested rung is one the evidence supports (§12.3, ADR-P2-14).
///
/// This gate does not *grant* capital; [`ledger::capital::authorise`] does, and
/// it cannot be called without a capital-authorising profile and sixteen passes.
/// What this does is state the verdict in the gate stack's own terms, so a
/// reader of the sixteen sees why the answer was no.
///
/// `verdicts` is every gate's `(number, passed)` including this one's
/// prerequisites; `current` is the rung the registry holds today.
#[must_use]
pub fn gate_16_capital_ramp(
    profile: &GateProfile,
    current: ledger::AllowedFraction,
    verdicts: &[(i32, bool)],
) -> GateOutcome {
    const NAME: &str = "capital_ramp";
    match ledger::capital::authorise(profile.profile_id(), profile.authorises_capital(), verdicts) {
        Ok(authority) => {
            let next = current.raise(&authority);
            GateOutcome::decided(
                16,
                NAME,
                true,
                next.value(),
                current.value(),
                format!(
                    "sixteen of sixteen passed under {}: {} → {} of intended size",
                    profile.profile_id(),
                    current.as_str(),
                    next.as_str()
                ),
            )
        }
        Err(why) => GateOutcome::decided(
            16,
            NAME,
            false,
            current.value(),
            current.value(),
            format!("stays at {} — {why}", current.as_str()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn profile_with_crises() -> GateProfile {
        GateProfile::strict_v1().with_asset_class(
            "crypto",
            super::super::profile::AssetClassGates {
                factor_battery: vec!["cmkt".into()],
                crisis_windows: vec![
                    CrisisWindow {
                        label: "covid".into(),
                        from: NaiveDate::from_ymd_opt(2020, 3, 1).unwrap(),
                        to: NaiveDate::from_ymd_opt(2020, 4, 1).unwrap(),
                    },
                    CrisisWindow {
                        label: "ftx".into(),
                        from: NaiveDate::from_ymd_opt(2022, 11, 1).unwrap(),
                        to: NaiveDate::from_ymd_opt(2022, 12, 1).unwrap(),
                    },
                ],
            },
        )
    }

    /// Daily returns from `start`, with a per-day closure so a test can plant a
    /// volatility regime or a concentrated P&L window.
    fn daily(start: NaiveDate, n: usize, mut f: impl FnMut(usize) -> f64) -> Vec<(DateTime<Utc>, f64)> {
        (0..n)
            .map(|i| {
                let d = start + chrono::Duration::days(i as i64);
                (
                    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()),
                    f(i),
                )
            })
            .collect()
    }

    // ── Gate 9 ───────────────────────────────────────────────────────────────

    /// P-01, as a gate: 10 000 looks over 5 years expect a Sharpe of ~1.92 from
    /// noise alone, so 1.9 must fail and nothing about it is close.
    #[test]
    fn gate_9_rejects_the_sharpe_that_noise_would_have_produced() {
        let p = GateProfile::strict_v1();
        let out = gate_9_min_length(
            &p,
            LengthInputs { sharpe: 1.9, n_eff: 10_000.0, years: 5.0, independent_events: 500 },
        );
        assert!(!out.passed, "{}", out.detail);
        assert!(out.detail.contains("noise"));
    }

    #[test]
    fn gate_9_accepts_a_strong_result_on_a_long_sample_with_few_looks() {
        let p = GateProfile::strict_v1();
        let out = gate_9_min_length(
            &p,
            LengthInputs { sharpe: 2.5, n_eff: 12.0, years: 8.0, independent_events: 900 },
        );
        assert!(out.passed, "{}", out.detail);
    }

    /// `paper_v1` drops the calendar floor and keeps the statistical ones — the
    /// whole reason it exists (ADR-P2-14).
    /// `paper_v1` drops the calendar floor and keeps every statistical one. The
    /// noise boundary in particular gets *harder* on a short sample, not easier:
    /// 12 looks over 10 months needs a Sharpe of 3.7 where 8 years needs 0.8.
    /// That is the point — `paper_v1` is a different purpose, not a softer bar.
    #[test]
    fn paper_v1_drops_only_the_calendar_floor_at_gate_9() {
        let short = LengthInputs { sharpe: 4.0, n_eff: 12.0, years: 0.8, independent_events: 400 };
        let strict = gate_9_min_length(&GateProfile::strict_v1(), short);
        assert!(!strict.passed, "a 10-month sample cannot pass strict_v1");
        assert!(strict.detail.contains("5.0y floor"));

        let paper = gate_9_min_length(&GateProfile::paper_v1(), short);
        assert!(paper.passed, "{}", paper.detail);

        // The noise boundary still binds under paper_v1, and binds harder on a
        // short sample than it would on a long one.
        let weak = LengthInputs { sharpe: 2.5, ..short };
        let out = gate_9_min_length(&GateProfile::paper_v1(), weak);
        assert!(!out.passed, "paper_v1 does not relax the noise boundary");
        assert!(out.detail.contains("noise"));

        // And so does the event floor.
        let thin = LengthInputs { independent_events: 50, ..short };
        assert!(!gate_9_min_length(&GateProfile::paper_v1(), thin).passed);
    }

    #[test]
    fn gate_9_without_a_span_is_inconclusive_not_a_pass() {
        let out = gate_9_min_length(
            &GateProfile::strict_v1(),
            LengthInputs { sharpe: 3.0, n_eff: 5.0, years: 0.0, independent_events: 9999 },
        );
        assert!(out.inconclusive);
        assert!(!out.passed, "an un-runnable gate is never a pass");
    }

    // ── Gate 11 ──────────────────────────────────────────────────────────────

    #[test]
    fn vol_terciles_find_a_planted_volatility_regime() {
        let r = daily(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(), 300, |i| {
            let amp = if (100..200).contains(&i) { 0.05 } else { 0.002 };
            if i % 2 == 0 { amp } else { -amp }
        });
        let labels = vol_tercile_regimes(&r, 21);
        assert_eq!(labels.len(), 300);
        // The high-vol middle is labelled high; the quiet start is not.
        assert_eq!(labels[150], VolRegime::High);
        assert_ne!(labels[20], VolRegime::High);
    }

    /// The check that catches the most self-deception: all the money made in one
    /// window.
    #[test]
    fn gate_11_rejects_a_result_earned_entirely_in_one_regime() {
        let p = profile_with_crises();
        // The market cycles through three volatility regimes.
        let market = daily(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(), 400, |i| {
            let amp = match (i / 40) % 3 { 0 => 0.004, 1 => 0.012, _ => 0.030 };
            if i % 2 == 0 { amp } else { -amp }
        });
        let r = daily(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(), 400, |i| {
            // Flat everywhere except a violent, profitable 40-day window.
            if (180..220).contains(&i) { 0.05 } else { 0.00001 }
        });
        let out = gate_11_regime_coverage(&p, "crypto", &market, &r);
        assert!(!out.passed, "{}", out.detail);
        assert!(out.detail.contains("of gains came from"), "{}", out.detail);
    }

    /// A result earned across three sustained volatility regimes, covering both
    /// declared crises, with no regime carrying it. Volatility has to vary over
    /// *time* for terciles to separate — alternating amplitudes day to day gives
    /// a constant rolling vol and therefore one regime, which is why the fixture
    /// is built from blocks.
    #[test]
    fn gate_11_passes_a_result_spread_across_regimes() {
        let p = profile_with_crises();
        // Nov 2019 → ~Mar 2023: spans both the covid and FTX windows.
        // The *market* cycles through three volatility regimes every 120 days.
        let market = daily(NaiveDate::from_ymd_opt(2019, 11, 1).unwrap(), 1_250, |i| {
            let amp = match (i / 120) % 3 { 0 => 0.004, 1 => 0.012, _ => 0.028 };
            if i % 2 == 0 { amp } else { -amp }
        });
        // A volatility-targeted strategy: the same modest edge in every regime,
        // which is exactly the shape that looks like one regime if you label from
        // the strategy's own returns.
        let r = daily(NaiveDate::from_ymd_opt(2019, 11, 1).unwrap(), 1_250, |i| {
            if i % 7 < 4 { 0.006 } else { -0.0055 }
        });
        let out = gate_11_regime_coverage(&p, "crypto", &market, &r);
        assert!(out.passed, "{}", out.detail);
        assert!(out.detail.contains("3 regimes"), "{}", out.detail);
        assert!(out.detail.contains("covid"), "{}", out.detail);
    }

    /// A sample that predates every declared crisis is *short*, not reckless —
    /// and the verdict must distinguish those.
    #[test]
    fn gate_11_says_when_a_crisis_window_is_outside_the_sample() {
        let p = profile_with_crises();
        let market = daily(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), 300, |i| {
            let amp = match (i / 40) % 3 { 0 => 0.004, 1 => 0.012, _ => 0.030 };
            if i % 2 == 0 { amp } else { -amp }
        });
        let r = daily(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), 300, |i| {
            if i % 5 == 0 { -0.004 } else { 0.002 }
        });
        let out = gate_11_regime_coverage(&p, "crypto", &market, &r);
        assert!(out.detail.contains("predate this sample"), "{}", out.detail);
    }

    /// An asset class with no declared crisis windows cannot be judged — and a
    /// count of zero from an undeclared list is not coverage.
    #[test]
    fn gate_11_on_an_undeclared_asset_class_is_inconclusive() {
        let r = daily(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(), 200, |_| 0.001);
        let out = gate_11_regime_coverage(&GateProfile::strict_v1(), "crypto", &r, &r);
        assert!(out.inconclusive);
        assert!(!out.passed);
    }

    /// The reason `market` and `strategy` are separate arguments: a
    /// volatility-targeted strategy has near-constant own-volatility, so
    /// labelling regimes from its own returns finds one regime however varied the
    /// market was. The verdict says which source it used.
    #[test]
    fn labelling_regimes_from_the_strategy_understates_variety_and_says_so() {
        let p = profile_with_crises();
        let flat = daily(NaiveDate::from_ymd_opt(2019, 11, 1).unwrap(), 1_250, |i| {
            if i % 7 < 4 { 0.006 } else { -0.0055 }
        });
        let out = gate_11_regime_coverage(&p, "crypto", &[], &flat);
        assert!(
            out.detail.contains("no market series supplied"),
            "the weaker reading must be declared: {}",
            out.detail
        );
    }

    #[test]
    fn a_misaligned_market_series_is_inconclusive() {
        let p = profile_with_crises();
        let market = daily(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(), 100, |_| 0.01);
        let r = daily(NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(), 200, |_| 0.001);
        let out = gate_11_regime_coverage(&p, "crypto", &market, &r);
        assert!(out.inconclusive);
        assert!(!out.passed);
    }

    // ── Gate 12 ──────────────────────────────────────────────────────────────

    #[test]
    fn gate_12_rejects_a_parameter_spike() {
        let p = GateProfile::strict_v1();
        let nbhd = [(8.0, 0.1), (9.0, 0.1), (10.0, 2.0), (11.0, 0.1), (12.0, 0.1)];
        let out = gate_12_perturbation(
            &p,
            &PerturbationInputs {
                center: 2.0,
                neighbourhood: &nbhd,
                per_instrument_pnl: &[("BTC-USD".into(), 1.0)],
            },
        );
        assert!(!out.passed, "{}", out.detail);
        assert!(out.detail.contains("cliff"));
    }

    /// A cap of 20% cannot be met by a two-instrument universe, so the gate
    /// relaxes to what the universe allows and says that it did (ADR-P2-28).
    /// Without this, every strategy on a small universe fails a diversification
    /// test it had no way to pass.
    #[test]
    fn gate_12_relaxes_the_concentration_cap_to_what_the_universe_allows() {
        let p = GateProfile::strict_v1();
        let nbhd = [(8.0, 1.8), (9.0, 1.9), (10.0, 2.0), (11.0, 1.95), (12.0, 1.85)];
        let out = gate_12_perturbation(
            &p,
            &PerturbationInputs {
                center: 2.0,
                neighbourhood: &nbhd,
                per_instrument_pnl: &[("BTC-USD".into(), 5.0), ("ETH-USD".into(), 5.0)],
            },
        );
        assert!(out.passed, "{}", out.detail);
        assert!(out.detail.contains("cannot go below 50%"), "{}", out.detail);

        // But a two-instrument strategy carried by one of them still fails.
        let lopsided = gate_12_perturbation(
            &p,
            &PerturbationInputs {
                center: 2.0,
                neighbourhood: &nbhd,
                per_instrument_pnl: &[("BTC-USD".into(), 9.5), ("ETH-USD".into(), 0.5)],
            },
        );
        assert!(!lopsided.passed, "{}", lopsided.detail);
    }

    #[test]
    fn gate_12_accepts_a_plateau() {
        let p = GateProfile::strict_v1();
        let nbhd = [(8.0, 1.8), (9.0, 1.9), (10.0, 2.0), (11.0, 1.95), (12.0, 1.85)];
        let out = gate_12_perturbation(
            &p,
            &PerturbationInputs {
                center: 2.0,
                neighbourhood: &nbhd,
                per_instrument_pnl: &[
                    ("BTC-USD".into(), 2.0),
                    ("ETH-USD".into(), 2.0),
                    ("SOL-USD".into(), 2.0),
                    ("ADA-USD".into(), 2.0),
                    ("DOT-USD".into(), 2.0),
                    ("AVAX-USD".into(), 2.0),
                ],
            },
        );
        assert!(out.passed, "{}", out.detail);
        assert!(!out.detail.contains("cannot go below"), "the profile cap binds here");
    }

    #[test]
    fn gate_12_rejects_a_result_carried_by_one_instrument() {
        let p = GateProfile::strict_v1();
        let nbhd = [(8.0, 1.8), (9.0, 1.9), (10.0, 2.0), (11.0, 1.95), (12.0, 1.85)];
        let out = gate_12_perturbation(
            &p,
            &PerturbationInputs {
                center: 2.0,
                neighbourhood: &nbhd,
                per_instrument_pnl: &[
                    ("BTC-USD".into(), 9.0),
                    ("ETH-USD".into(), 0.5),
                    ("SOL-USD".into(), 0.5),
                ],
            },
        );
        assert!(!out.passed, "{}", out.detail);
        assert!(out.detail.contains("BTC-USD"));
    }

    /// A single-instrument strategy passes the concentration check trivially, and
    /// the verdict says so rather than implying diversification.
    #[test]
    fn a_single_instrument_strategy_is_told_the_check_did_not_apply() {
        let p = GateProfile::strict_v1();
        let nbhd = [(8.0, 1.8), (9.0, 1.9), (10.0, 2.0), (11.0, 1.95), (12.0, 1.85)];
        let out = gate_12_perturbation(
            &p,
            &PerturbationInputs {
                center: 2.0,
                neighbourhood: &nbhd,
                per_instrument_pnl: &[("BTC-USD".into(), 10.0)],
            },
        );
        assert!(out.passed);
        assert!(out.detail.contains("does not apply"));
    }

    // ── 13 / 14 wrappers, and the record ─────────────────────────────────────

    #[test]
    fn gate_13_reports_the_percentile_and_the_block_it_used() {
        let mut rng = crate::rng::DetRng::new(4);
        let r: Vec<f64> = (0..400)
            .map(|_| 0.004 + (rng.next_f64() - 0.5) * 0.01)
            .collect();
        let out = gate_13_stationary_bootstrap(&GateProfile::strict_v1(), &r, 1);
        assert!(out.passed, "{}", out.detail);
        assert!(out.detail.contains("mean block"));
    }

    #[test]
    fn gate_13_on_a_short_series_is_inconclusive_not_a_pass() {
        let out = gate_13_stationary_bootstrap(&GateProfile::strict_v1(), &[0.01; 5], 1);
        assert!(out.inconclusive);
        assert!(!out.passed);
    }

    #[test]
    fn gate_14_needs_the_whole_family() {
        let out = gate_14_romano_wolf(&GateProfile::strict_v1(), &[vec![0.01; 100]], 0, 1);
        assert!(out.inconclusive);
        assert!(out.detail.contains("whole candidate family"));
    }

    /// A verdict becomes a durable record carrying its profile, its statistic and
    /// its threshold — the three things that make "passed" checkable later.
    #[test]
    fn an_outcome_converts_to_a_record_that_carries_its_bar() {
        let out = GateOutcome::decided(13, "stationary_bootstrap", true, 0.4, 0.0, "d");
        let rec = out.to_record("strict_v1").for_experiment("exp-1");
        assert_eq!(rec.gate_no, 13);
        assert_eq!(rec.profile_id, "strict_v1");
        assert_eq!(rec.statistic, Some(0.4));
        assert_eq!(rec.threshold, Some(0.0));
        assert!(rec.validate().is_ok());

        // A structural verdict carries no statistic, and that is legal.
        let s = GateOutcome::inconclusive(11, "regime_coverage", "no labels");
        let rec = s.to_record("strict_v1").for_experiment("exp-1");
        assert!(rec.statistic.is_none());
        assert!(!rec.passed, "an inconclusive gate records as not passed");
    }

    // ── Gate 4 — capacity ───────────────────────────────────────────────────

    fn usd(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn square_root_impact_scales_with_the_root_of_participation() {
        let a = square_root_impact(1_000_000.0, 100_000_000.0, 0.02).unwrap();
        let b = square_root_impact(4_000_000.0, 100_000_000.0, 0.02).unwrap();
        assert!((b / a - 2.0).abs() < 1e-9, "four times the size is twice the impact");
        assert!(square_root_impact(1.0, 0.0, 0.02).is_none(), "no ADV, no model");
    }

    #[test]
    fn capacity_is_read_off_the_measured_curve_and_not_beyond_it() {
        let curve = [(1e6, 2.0), (5e6, 1.6), (1e7, 1.2), (2e7, 0.9), (4e7, 0.5)];
        let half = capacity_at_half_sharpe(&curve).expect("the curve crosses");
        assert!(half > 1e7 && half < 4e7, "got {half}");

        // A curve that never halves does not get extrapolated into one that does.
        assert_eq!(capacity_at_half_sharpe(&[(1e6, 2.0), (2e6, 1.9), (4e6, 1.8)]), None);
        assert_eq!(capacity_at_half_sharpe(&[(1e6, 2.0)]), None);
    }

    #[test]
    fn gate_4_refuses_a_backtest_that_could_not_have_been_traded() {
        let p = GateProfile::strict_v1();
        let adv = usd(&[("BTC-USD", 1e9), ("ETH-USD", 5e8)]);
        let vol = usd(&[("BTC-USD", 0.03), ("ETH-USD", 0.04)]);
        // 20 % of ADV in a day: the fills assumed a price this size would move.
        let peak = usd(&[("BTC-USD", 2e8), ("ETH-USD", 1e6)]);
        let out = gate_4_capacity(
            &p,
            &CapacityInputs {
                adv_usd: &adv,
                daily_vol: &vol,
                peak_daily_notional: &peak,
                sharpe_by_aum: &[(1e6, 2.0), (1e7, 1.0)],
                base_aum_usd: 1e6,
                single_venue: true,
            },
        );
        assert!(!out.passed && !out.inconclusive, "{}", out.detail);
        assert!(out.detail.contains("hard cap"), "{}", out.detail);
    }

    #[test]
    fn gate_4_passes_a_strategy_with_room_to_grow() {
        let p = GateProfile::strict_v1();
        let adv = usd(&[("BTC-USD", 1e9)]);
        let vol = usd(&[("BTC-USD", 0.03)]);
        let peak = usd(&[("BTC-USD", 2e6)]); // 0.2 % of ADV
        let out = gate_4_capacity(
            &p,
            &CapacityInputs {
                adv_usd: &adv,
                daily_vol: &vol,
                peak_daily_notional: &peak,
                // Sharpe halves only at 100× the base size.
                sharpe_by_aum: &[(1e6, 2.0), (1e7, 1.8), (5e7, 1.4), (1e8, 1.0), (2e8, 0.6)],
                base_aum_usd: 1e6,
                single_venue: true,
            },
        );
        assert!(out.passed, "{}", out.detail);
        assert!(out.detail.contains("single-venue"), "the caveat must survive: {}", out.detail);
    }

    #[test]
    fn gate_4_is_inconclusive_rather_than_passing_when_capacity_was_never_reached() {
        let p = GateProfile::strict_v1();
        let adv = usd(&[("BTC-USD", 1e9)]);
        let vol = usd(&[("BTC-USD", 0.03)]);
        let peak = usd(&[("BTC-USD", 1e6)]);
        let out = gate_4_capacity(
            &p,
            &CapacityInputs {
                adv_usd: &adv,
                daily_vol: &vol,
                peak_daily_notional: &peak,
                sharpe_by_aum: &[(1e6, 2.0), (1e7, 1.95)],
                base_aum_usd: 1e6,
                single_venue: false,
            },
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
    }

    #[test]
    fn gate_4_will_not_assess_an_instrument_it_has_no_adv_for() {
        let p = GateProfile::strict_v1();
        let adv = usd(&[("BTC-USD", 1e9)]);
        let vol = usd(&[("BTC-USD", 0.03)]);
        let peak = usd(&[("BTC-USD", 1e6), ("DOGE-USD", 1e6)]);
        let out = gate_4_capacity(
            &p,
            &CapacityInputs {
                adv_usd: &adv,
                daily_vol: &vol,
                peak_daily_notional: &peak,
                sharpe_by_aum: &[(1e6, 2.0), (1e7, 0.5)],
                base_aum_usd: 1e6,
                single_venue: true,
            },
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
        assert!(out.detail.contains("DOGE-USD"), "{}", out.detail);
    }

    // ── Gate 10 — factor attribution ────────────────────────────────────────

    fn crypto_profile() -> GateProfile {
        GateProfile::strict_v1().with_asset_class(
            "crypto",
            super::super::profile::AssetClassGates {
                factor_battery: ["CMKT", "CSMB", "CMOM"].iter().map(ToString::to_string).collect(),
                crisis_windows: Vec::new(),
            },
        )
    }

    fn factor_series(n: usize, seed: u64) -> Vec<f64> {
        let mut r = crate::rng::DetRng::new(seed);
        (0..n).map(|_| (r.next_f64() - 0.5) * 0.04).collect()
    }

    fn battery(n: usize) -> BTreeMap<String, Vec<f64>> {
        [("CMKT", 1_u64), ("CSMB", 2), ("CMOM", 3)]
            .into_iter()
            .map(|(k, s)| (k.to_string(), factor_series(n, s)))
            .collect()
    }

    #[test]
    fn gate_10_will_not_run_against_a_battery_nobody_declared() {
        let p = GateProfile::strict_v1(); // no asset classes attached
        let f = battery(400);
        let out = gate_10_factor_attribution(
            &p,
            "crypto",
            &FactorInputs {
                strategy_net: &factor_series(400, 9),
                factors: &f,
                market_factor: "CMKT".into(),
                neutral_claim: false,
            },
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
    }

    #[test]
    fn gate_10_refuses_a_battery_that_is_not_the_declared_one() {
        let p = crypto_profile();
        let mut f = battery(400);
        f.remove("CMOM");
        let out = gate_10_factor_attribution(
            &p,
            "crypto",
            &FactorInputs {
                strategy_net: &factor_series(400, 9),
                factors: &f,
                market_factor: "CMKT".into(),
                neutral_claim: false,
            },
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
        assert!(out.detail.contains("CMOM"), "{}", out.detail);
    }

    #[test]
    fn gate_10_calls_a_levered_factor_portfolio_what_it_is() {
        let p = crypto_profile();
        let f = battery(500);
        // Two units of the market and nothing else: no alpha, enormous R².
        let strategy: Vec<f64> = (0..500).map(|i| 2.0 * f["CMKT"][i]).collect();
        let out = gate_10_factor_attribution(
            &p,
            "crypto",
            &FactorInputs {
                strategy_net: &strategy,
                factors: &f,
                market_factor: "CMKT".into(),
                neutral_claim: false,
            },
        );
        assert!(!out.passed && !out.inconclusive, "{}", out.detail);
        assert!(out.detail.contains("R²"), "{}", out.detail);
    }

    #[test]
    fn gate_10_passes_a_real_alpha_that_the_battery_does_not_explain() {
        let p = crypto_profile();
        let f = battery(600);
        // A steady edge plus a little idiosyncratic noise, uncorrelated with
        // everything in the battery.
        let noise = factor_series(600, 42);
        let strategy: Vec<f64> = (0..600).map(|i| 0.0015 + 0.1 * noise[i]).collect();
        let out = gate_10_factor_attribution(
            &p,
            "crypto",
            &FactorInputs {
                strategy_net: &strategy,
                factors: &f,
                market_factor: "CMKT".into(),
                neutral_claim: true,
            },
        );
        assert!(out.passed, "{}", out.detail);
        assert!(out.statistic.unwrap() >= 3.0, "{}", out.detail);
    }

    #[test]
    fn gate_10_checks_a_neutrality_claim_that_was_actually_made() {
        let p = crypto_profile();
        let f = battery(600);
        let noise = factor_series(600, 77);
        // Half a unit of market beta on top of a genuine edge, with enough
        // idiosyncratic return that the battery does not explain most of the
        // variance — otherwise the R² check fires first and this test would be
        // checking the wrong thing.
        let strategy: Vec<f64> =
            (0..600).map(|i| 0.0015 + 0.5 * f["CMKT"][i] + 0.6 * noise[i]).collect();

        let claimed = gate_10_factor_attribution(
            &p,
            "crypto",
            &FactorInputs {
                strategy_net: &strategy,
                factors: &f,
                market_factor: "CMKT".into(),
                neutral_claim: true,
            },
        );
        assert!(!claimed.passed, "a neutral claim with β=0.5 must fail: {}", claimed.detail);
        assert!(claimed.detail.contains("market-neutral"), "{}", claimed.detail);

        // The same strategy making no such claim is judged on alpha alone.
        let unclaimed = gate_10_factor_attribution(
            &p,
            "crypto",
            &FactorInputs {
                strategy_net: &strategy,
                factors: &f,
                market_factor: "CMKT".into(),
                neutral_claim: false,
            },
        );
        assert!(unclaimed.passed, "{}", unclaimed.detail);
    }

    #[test]
    fn the_hac_correction_is_not_optional_on_autocorrelated_residuals() {
        // A persistent residual inflates a naive t-statistic; Newey–West is what
        // stops that from reading as alpha.
        let n = 400;
        let f = battery(n);
        let mut persistent = vec![0.0_f64; n];
        let shock = factor_series(n, 5);
        for i in 1..n {
            persistent[i] = 0.9 * persistent[i - 1] + shock[i];
        }
        let strategy: Vec<f64> = (0..n).map(|i| 0.0005 + 0.01 * persistent[i]).collect();
        let fit = newey_west_fit(&strategy, &f).expect("estimable");
        assert!(fit.hac_lags > 0, "the automatic lag rule must pick a positive lag at n={n}");
    }

    // ── Gate 15 — paper / shadow ────────────────────────────────────────────

    fn good_paper() -> PaperInputs {
        PaperInputs {
            signal_match: 0.995,
            slippage_ratio: 1.2,
            turnover_ratio: 1.05,
            model_attributable_rejects: 0,
            observed_days: 60,
            kill_criteria_registered: true,
        }
    }

    #[test]
    fn gate_15_passes_a_forward_test_that_matches_the_backtest() {
        let out = gate_15_paper_shadow(&GateProfile::strict_v1(), good_paper());
        assert!(out.passed, "{}", out.detail);
        assert!(out.detail.contains("60 days"), "{}", out.detail);
    }

    /// Unregistered kill criteria make the gate *inconclusive*, not failing and
    /// certainly not passing: the other four numbers may be fine, but a stopping
    /// rule chosen after the fact is not a stopping rule.
    #[test]
    fn gate_15_will_not_produce_a_verdict_without_pre_registered_kill_criteria() {
        let out = gate_15_paper_shadow(
            &GateProfile::strict_v1(),
            PaperInputs { kill_criteria_registered: false, ..good_paper() },
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
        assert!(out.detail.contains("kill criteria"), "{}", out.detail);
    }

    #[test]
    fn gate_15_fails_each_way_a_live_system_can_diverge() {
        let p = GateProfile::strict_v1();
        let cases = [
            PaperInputs { signal_match: 0.90, ..good_paper() },
            PaperInputs { slippage_ratio: 3.0, ..good_paper() },
            PaperInputs { turnover_ratio: 1.8, ..good_paper() },
            PaperInputs { turnover_ratio: 0.2, ..good_paper() },
            PaperInputs { model_attributable_rejects: 9, ..good_paper() },
        ];
        for case in cases {
            let out = gate_15_paper_shadow(&p, case);
            assert!(!out.passed && !out.inconclusive, "{case:?} → {}", out.detail);
        }
    }

    #[test]
    fn gate_15_has_nothing_to_say_before_the_first_day() {
        let out = gate_15_paper_shadow(
            &GateProfile::strict_v1(),
            PaperInputs { observed_days: 0, ..good_paper() },
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
    }

    // ── Gate 16 — the capital ramp ──────────────────────────────────────────

    fn sixteen_passes() -> Vec<(i32, bool)> {
        (1..=16).map(|g| (g, true)).collect()
    }

    /// AT-65 ⛔ — sixteen passes under `paper_v1` still authorise nothing.
    #[test]
    fn at65_paper_v1_cannot_authorise_capital() {
        let out = gate_16_capital_ramp(
            &GateProfile::paper_v1(),
            ledger::AllowedFraction::default(),
            &sixteen_passes(),
        );
        assert!(!out.passed, "{}", out.detail);
        assert!(out.detail.contains("does not authorise capital"), "{}", out.detail);
        assert!((out.statistic.unwrap() - 0.0).abs() < f64::EPSILON);

        // The same evidence under strict_v1 moves exactly one rung.
        let strict = gate_16_capital_ramp(
            &GateProfile::strict_v1(),
            ledger::AllowedFraction::default(),
            &sixteen_passes(),
        );
        assert!(strict.passed, "{}", strict.detail);
        assert!((strict.statistic.unwrap() - 0.10).abs() < 1e-12, "{}", strict.detail);
    }

    #[test]
    fn gate_16_refuses_to_move_on_an_incomplete_stack() {
        let mut partial = sixteen_passes();
        partial.truncate(15);
        let out = gate_16_capital_ramp(
            &GateProfile::strict_v1(),
            ledger::AllowedFraction::Ten,
            &partial,
        );
        assert!(!out.passed, "{}", out.detail);
        assert!((out.statistic.unwrap() - 0.10).abs() < 1e-12, "it stays where it was");
    }

    #[test]
    fn gate_16_refuses_on_a_single_failed_gate() {
        let mut one_bad = sixteen_passes();
        one_bad[3] = (4, false);
        let out = gate_16_capital_ramp(
            &GateProfile::strict_v1(),
            ledger::AllowedFraction::TwentyFive,
            &one_bad,
        );
        assert!(!out.passed, "{}", out.detail);
        assert!((out.statistic.unwrap() - 0.25).abs() < 1e-12);
    }

    // ── Gate 2 — the leakage suite, wired ───────────────────────────────────

    const SUITE: [&str; 5] = [
        "causal_access",
        "random_label",
        "snapshot_reproducibility",
        "cv_wf_gap",
        "synthetic_injection",
    ];

    fn leakage_run(blocking: i64, checks: &[&str], age_hours: i64) -> LeakageRun {
        LeakageRun {
            subject: "ds:abc".into(),
            checks_run: checks.iter().map(ToString::to_string).collect(),
            blocking_count: blocking,
            flag_count: 2,
            finished_at: Utc::now() - chrono::Duration::hours(age_hours),
        }
    }

    #[test]
    fn gate_2_passes_a_complete_clean_recent_suite() {
        let out = gate_2_leakage_suite(
            &GateProfile::strict_v1(),
            Some(&leakage_run(0, &SUITE, 2)),
            &SUITE,
            Utc::now(),
            chrono::Duration::hours(72),
        );
        assert!(out.passed, "{}", out.detail);
    }

    #[test]
    fn gate_2_fails_on_a_blocking_finding() {
        let out = gate_2_leakage_suite(
            &GateProfile::strict_v1(),
            Some(&leakage_run(1, &SUITE, 2)),
            &SUITE,
            Utc::now(),
            chrono::Duration::hours(72),
        );
        assert!(!out.passed && !out.inconclusive, "{}", out.detail);
    }

    /// A check that did not run is not a check that passed. This is the failure
    /// the wiring exists to prevent: a suite that quietly stopped running half
    /// its checks would otherwise report a clean sheet.
    #[test]
    fn gate_2_is_inconclusive_when_a_check_did_not_run() {
        let out = gate_2_leakage_suite(
            &GateProfile::strict_v1(),
            Some(&leakage_run(0, &SUITE[..3], 2)),
            &SUITE,
            Utc::now(),
            chrono::Duration::hours(72),
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
        assert!(out.detail.contains("cv_wf_gap"), "{}", out.detail);
    }

    #[test]
    fn gate_2_will_not_accept_a_stale_clean_run_or_a_missing_one() {
        let stale = gate_2_leakage_suite(
            &GateProfile::strict_v1(),
            Some(&leakage_run(0, &SUITE, 400)),
            &SUITE,
            Utc::now(),
            chrono::Duration::hours(72),
        );
        assert!(stale.inconclusive && !stale.passed, "{}", stale.detail);

        let none = gate_2_leakage_suite(
            &GateProfile::strict_v1(),
            None,
            &SUITE,
            Utc::now(),
            chrono::Duration::hours(72),
        );
        assert!(none.inconclusive && !none.passed, "{}", none.detail);
    }

    // ── Gate 3 — cost sensitivity ───────────────────────────────────────────

    fn ladder(points: &[(f64, f64)]) -> Vec<CostRung> {
        points.iter().map(|(m, v)| CostRung { multiple: *m, net_metric: *v }).collect()
    }

    #[test]
    fn the_breakeven_is_interpolated_between_the_rungs_that_bracket_it() {
        let b = breakeven_multiple(&ladder(&[(1.0, 0.20), (2.0, 0.10), (3.0, -0.10)])).unwrap();
        assert!((b - 2.5).abs() < 1e-9, "got {b}");

        // Dead at the cheapest assumption tried.
        let d = breakeven_multiple(&ladder(&[(1.0, -0.05), (2.0, -0.20)])).unwrap();
        assert!((d - 1.0).abs() < 1e-12);

        // Never crosses: not extrapolated into a breakeven.
        assert_eq!(breakeven_multiple(&ladder(&[(1.0, 0.2), (2.0, 0.19)])), None);
        assert_eq!(breakeven_multiple(&ladder(&[(1.0, 0.2)])), None);
    }

    #[test]
    fn gate_3_refuses_an_edge_that_dies_before_three_times_costs() {
        let out = gate_3_cost_sensitivity(
            &GateProfile::strict_v1(),
            &ladder(&[(1.0, 0.20), (1.5, 0.05), (2.0, -0.05)]),
        );
        assert!(!out.passed && !out.inconclusive, "{}", out.detail);
        assert!(out.statistic.unwrap() < 3.0);
    }

    #[test]
    fn gate_3_passes_an_edge_that_survives_a_wrong_cost_model() {
        let out = gate_3_cost_sensitivity(
            &GateProfile::strict_v1(),
            &ladder(&[(1.0, 0.30), (2.0, 0.22), (4.0, 0.06), (5.0, -0.02)]),
        );
        assert!(out.passed, "{}", out.detail);
        assert!(out.statistic.unwrap() >= 3.0);
    }

    /// A ladder that stops below the bar cannot answer the question, and saying
    /// "it survived everything we tried" when everything we tried was 2× is the
    /// answer that would flatter it.
    #[test]
    fn gate_3_is_inconclusive_when_the_ladder_stops_short() {
        let out = gate_3_cost_sensitivity(
            &GateProfile::strict_v1(),
            &ladder(&[(1.0, 0.30), (2.0, 0.22)]),
        );
        assert!(out.inconclusive && !out.passed, "{}", out.detail);

        // A ladder that reaches past the bar without dying does pass, and says
        // the breakeven is beyond what was measured.
        let far = gate_3_cost_sensitivity(
            &GateProfile::strict_v1(),
            &ladder(&[(1.0, 0.30), (3.0, 0.20), (5.0, 0.11)]),
        );
        assert!(far.passed, "{}", far.detail);
        assert!(far.detail.contains("beyond the measured range"), "{}", far.detail);
    }

    #[test]
    fn gate_3_needs_more_than_one_cost_assumption() {
        let out = gate_3_cost_sensitivity(&GateProfile::strict_v1(), &ladder(&[(1.0, 0.3)]));
        assert!(out.inconclusive && !out.passed, "{}", out.detail);
    }
}
