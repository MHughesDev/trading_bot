//! Where a deployed strategy's return diverged from its backtest (S-3.14,
//! AGENT-006 §5.6 `attribute_divergence`).
//!
//! # What this can and cannot attribute
//!
//! Gate 15's forward record carries, per bar, the realised and expected return,
//! whether the live signal reproduced the backtest's, realised against modelled
//! slippage, and realised against expected turnover. From that:
//!
//! * **signal** -- the divergence summed over bars where the live signal did *not*
//!   reproduce the backtest's. Measured, in return units.
//! * **matched residual** -- the divergence over bars where the signal did match.
//!   Whatever moved the return there was not the decision, so it is execution,
//!   timing or data. Measured, in return units, and **not split further**: the
//!   record has no timing field and no per-bar data-quality field, and slippage
//!   is recorded in basis points against a turnover in the numeraire's own
//!   units, which cannot be turned into a return without an equity figure this
//!   row does not carry. The slippage excess, turnover ratio and reject count are
//!   reported beside it as the *evidence* for the execution reading, never as a
//!   return-unit component, because a guessed conversion would make four numbers
//!   that look additive and are not.
//! * **timing**, **data** -- reported as unmeasured, by name. Set T's `ShadowBar`
//!   (checked 2026-10-03) adds no field for either: it carries the same five
//!   quantities as the forward row. What *is* measurable is the **recording lag**
//!   (`observed_at` minus the bar close), reported as evidence that the row was
//!   written promptly. It is not execution latency and is not a component: a row
//!   written an hour late says the recorder was behind, not that the fill was.
//!
//! Only observed-knowledge rows enter (D-T1): a backfilled row is not evidence
//! about what happened live.

use domain::forward::ForwardObservation;
use serde::Serialize;

/// Fewest observed bars the report will name a dominant component from.
pub const MIN_BARS: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DivergenceError {
    #[error("no observed-knowledge rows; {backfilled} backfilled rows were excluded")]
    NoObservedRows { backfilled: usize },
    #[error("observations span more than one numeraire; they are not one series")]
    MixedNumeraire,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DivergenceReport {
    pub bars: usize,
    pub backfilled_excluded: usize,
    /// Mean of realised minus expected return per bar.
    pub mean_divergence: f64,
    /// Sum of realised minus expected return over the window.
    pub total_divergence: f64,
    /// Per-bar Sharpe (mean / sample sd, not annualised) of the stored backtest series.
    pub sharpe_backtest_per_bar: Option<f64>,
    pub sharpe_paper_per_bar: Option<f64>,
    /// `(backtest - paper) / |backtest|`, in percent. `None` when the backtest
    /// Sharpe is zero or either is not computable.
    pub decay_pct: Option<f64>,
    pub signal_mismatch_bars: usize,
    pub signal_component: f64,
    pub matched_residual: f64,
    /// Turnover-weighted `realised - modelled` slippage in bps; `None` with no turnover.
    pub slippage_excess_bps: Option<f64>,
    /// Realised over expected turnover; `None` when nothing was expected.
    pub turnover_ratio: Option<f64>,
    pub model_attributable_rejects: u32,
    /// Median seconds between a bar's close and the row recording it. Recording
    /// latency, not execution latency.
    pub recording_lag_median_s: Option<f64>,
    pub recording_lag_max_s: Option<f64>,
    /// `signal`, `matched_residual`, or `insufficient_data`.
    pub dominant: &'static str,
    pub verdict: String,
}

fn sharpe(xs: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 2 {
        return None;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    let sd = var.sqrt();
    (sd > 0.0 && sd.is_finite()).then(|| mean / sd)
}

/// Attribute the divergence of one strategy's forward observations.
///
/// # Errors
/// No observed rows, or rows in more than one numeraire.
pub fn decompose(obs: &[ForwardObservation]) -> Result<DivergenceReport, DivergenceError> {
    let observed: Vec<&ForwardObservation> =
        obs.iter().filter(|o| o.is_forward_evidence()).collect();
    let backfilled = obs.len() - observed.len();
    let Some(first) = observed.first() else {
        return Err(DivergenceError::NoObservedRows { backfilled });
    };
    if observed.iter().any(|o| o.numeraire != first.numeraire) {
        return Err(DivergenceError::MixedNumeraire);
    }
    let n = observed.len();
    let realised: Vec<f64> = observed.iter().map(|o| o.realised_return).collect();
    let expected: Vec<f64> = observed.iter().map(|o| o.expected_return).collect();
    let total: f64 = observed.iter().map(|o| o.return_divergence()).sum();
    let (mut signal, mut matched, mut mismatch) = (0.0, 0.0, 0usize);
    for o in &observed {
        if o.signal_matched {
            matched += o.return_divergence();
        } else {
            signal += o.return_divergence();
            mismatch += 1;
        }
    }
    let turnover: f64 = observed.iter().map(|o| o.realised_turnover).sum();
    let slippage_excess_bps = (turnover > 0.0).then(|| {
        observed
            .iter()
            .map(|o| (o.realised_slippage_bps - o.modelled_slippage_bps) * o.realised_turnover)
            .sum::<f64>()
            / turnover
    });
    let exp_turnover: f64 = observed.iter().map(|o| o.expected_turnover).sum();
    let turnover_ratio = (exp_turnover > 0.0).then(|| turnover / exp_turnover);
    let mut lags: Vec<f64> = observed
        .iter()
        .map(|o| (o.observed_at - o.bar).num_milliseconds() as f64 / 1000.0)
        .collect();
    lags.sort_by(f64::total_cmp);
    let lag_median = lags.get(lags.len() / 2).copied();
    let lag_max = lags.last().copied();
    let sr_b = sharpe(&expected);
    let sr_p = sharpe(&realised);
    let decay_pct = match (sr_b, sr_p) {
        (Some(b), Some(p)) if b != 0.0 => Some((b - p) / b.abs() * 100.0),
        _ => None,
    };
    let (dominant, verdict) = if n < MIN_BARS {
        (
            "insufficient_data",
            format!(
                "{n} observed bars; at least {MIN_BARS} are needed to name a dominant component"
            ),
        )
    } else if signal.abs() >= matched.abs() {
        (
            "signal",
            format!(
                "the live signal did not reproduce the backtest's on {mismatch} of {n} bars, and \
                 those bars carry {signal:+.6} of the {total:+.6} total divergence"
            ),
        )
    } else {
        (
            "matched_residual",
            format!(
                "the signal matched but the return did not: {matched:+.6} of {total:+.6} total \
                 divergence sits on bars where the decisions agreed, so the cause is execution, \
                 timing or data. Timing and data are unmeasured in this record; slippage excess is \
                 {} bps",
                slippage_excess_bps.map_or("unmeasured".to_string(), |s| format!("{s:+.2}"))
            ),
        )
    };
    Ok(DivergenceReport {
        bars: n,
        backfilled_excluded: backfilled,
        mean_divergence: total / n as f64,
        total_divergence: total,
        sharpe_backtest_per_bar: sr_b,
        sharpe_paper_per_bar: sr_p,
        decay_pct,
        signal_mismatch_bars: mismatch,
        signal_component: signal,
        matched_residual: matched,
        slippage_excess_bps,
        turnover_ratio,
        model_attributable_rejects: observed.iter().map(|o| o.model_attributable_rejects).sum(),
        recording_lag_median_s: lag_median,
        recording_lag_max_s: lag_max,
        dominant,
        verdict,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use domain::forward::ObservationSource;
    use domain::numeraire::Numeraire;
    use uuid::Uuid;

    fn obs(
        i: i64,
        realised: f64,
        expected: f64,
        matched: bool,
        src: ObservationSource,
    ) -> ForwardObservation {
        ForwardObservation {
            deployment_id: Uuid::nil(),
            strategy_id: "s".into(),
            bar: Utc.timestamp_opt(1_700_000_000 + i * 3600, 0).unwrap(),
            realised_return: realised,
            expected_return: expected,
            numeraire: Numeraire::Usd,
            source: src,
            signal_matched: matched,
            realised_slippage_bps: 6.0,
            modelled_slippage_bps: 2.0,
            realised_turnover: 100.0,
            expected_turnover: 100.0,
            model_attributable_rejects: 0,
            observed_at: Utc.timestamp_opt(1_700_000_000 + i * 3600, 0).unwrap(),
        }
    }

    /// Divergence concentrated on bars where the signal did not match is named
    /// as the signal.
    #[test]
    fn a_signal_mismatch_dominates_when_it_carries_the_divergence() {
        let o: Vec<_> = (0..40)
            .map(|i| {
                if i % 4 == 0 {
                    obs(i, -0.01, 0.01, false, ObservationSource::Observed)
                } else {
                    obs(i, 0.0101, 0.01, true, ObservationSource::Observed)
                }
            })
            .collect();
        let r = decompose(&o).unwrap();
        assert_eq!(r.dominant, "signal");
        assert_eq!(r.signal_mismatch_bars, 10);
        assert!((r.slippage_excess_bps.unwrap() - 4.0).abs() < 1e-9);
    }

    /// Divergence on bars where the decisions agreed is not blamed on the signal.
    #[test]
    fn a_matched_signal_with_missing_return_is_the_residual() {
        let o: Vec<_> = (0..40)
            .map(|i| obs(i, 0.004, 0.01, true, ObservationSource::Observed))
            .collect();
        let r = decompose(&o).unwrap();
        assert_eq!(r.dominant, "matched_residual");
        assert_eq!(r.signal_component, 0.0);
        assert!(r.matched_residual < 0.0);
    }

    /// A backfilled row is not evidence and is not counted, only reported.
    #[test]
    fn backfilled_rows_are_excluded_and_counted() {
        let mut o: Vec<_> = (0..40)
            .map(|i| obs(i, 0.01, 0.01, true, ObservationSource::Observed))
            .collect();
        o.push(obs(99, -9.0, 9.0, false, ObservationSource::Backfill));
        let r = decompose(&o).unwrap();
        assert_eq!(r.bars, 40);
        assert_eq!(r.backfilled_excluded, 1);
        assert_eq!(r.total_divergence, 0.0);
    }

    #[test]
    fn the_recording_lag_is_measured_and_not_called_execution_latency() {
        let o: Vec<_> = (0..40)
            .map(|i| obs(i, 0.01, 0.01, true, ObservationSource::Observed))
            .collect();
        let r = decompose(&o).unwrap();
        assert_eq!(r.recording_lag_median_s, Some(0.0));
        assert_eq!(r.recording_lag_max_s, Some(0.0));
    }

    #[test]
    fn too_few_bars_name_no_dominant_component() {
        let o: Vec<_> = (0..5)
            .map(|i| obs(i, 0.0, 0.01, false, ObservationSource::Observed))
            .collect();
        assert_eq!(decompose(&o).unwrap().dominant, "insufficient_data");
    }

    #[test]
    fn nothing_observed_is_an_error_not_a_zero() {
        let o = vec![obs(0, 0.0, 0.0, true, ObservationSource::Backfill)];
        assert_eq!(
            decompose(&o),
            Err(DivergenceError::NoObservedRows { backfilled: 1 })
        );
    }
}
