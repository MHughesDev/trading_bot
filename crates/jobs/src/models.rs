//! The rule tiers of M1, M2 and M11 (SPEC §13.1, checklist 4.1, ADR-P4-01).
//!
//! Three internal models, each answering from arithmetic rather than from a fit,
//! because the ledger does not yet hold enough runs to fit anything honest — and
//! at this platform's scale it will not for a long time. Every answer carries the
//! [`Rung`] that produced it, so the eventual rule-versus-learned comparison is a
//! query rather than an excavation.
//!
//! * **M1 — cost.** `rows × features × steps / throughput`. A closed form over
//!   the manifest and the hardware class. It will be wrong, and it will be wrong
//!   in a *stated* way: [`CostEstimate::interval`] is a band, not a point,
//!   because a point estimate from a formula nobody validated is a number people
//!   plan against.
//! * **M2 — failure.** Pre-flight arithmetic: does the model fit in the device's
//!   memory, is the learning rate inside the envelope its optimiser survives.
//!   These are the two failures that are *predictable before the run starts*,
//!   which is the only kind worth predicting — everything else is discovered by
//!   running it.
//! * **M11 — anomaly.** Hard rules first (a non-finite loss, a loss that has not
//!   moved), then robust-z and CUSUM over the curve. Hard rules first because a
//!   NaN is not an outlier to be scored, it is a run that is over.

use ledger::internal::{choose_rung, Answer, LadderEvidence, Rung};
use serde::{Deserialize, Serialize};

// ───────────────────────────────────────────────────────────────────────────────
// M1 — cost
// ───────────────────────────────────────────────────────────────────────────────

/// What the analytic cost model reads.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostInputs {
    pub rows: u64,
    pub features: u32,
    pub steps: u32,
    /// Effective throughput of the hardware class, in row-feature products per
    /// second. Measured once per class, not guessed per run.
    pub throughput: f64,
}

/// A cost estimate as a band.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub seconds: f64,
    /// Multiplicative band around `seconds`. The analytic tier's is wide on
    /// purpose: it models arithmetic, not I/O, not contention, not the warm-up
    /// the first batch pays.
    pub band: f64,
}

impl CostEstimate {
    /// `(low, high)` seconds.
    #[must_use]
    pub fn interval(&self) -> (f64, f64) {
        (self.seconds / self.band, self.seconds * self.band)
    }
}

/// The analytic tier's band. A factor of three either way.
///
/// Honest rather than flattering: this formula counts multiply-accumulates and
/// nothing else, and a run that spends half its time waiting on the dataloader
/// will take twice what it says. A narrow band would be a claim about I/O the
/// model does not make.
pub const ANALYTIC_COST_BAND: f64 = 3.0;

/// M1's rule tier.
///
/// # Panics
/// Never: a non-positive throughput yields an infinite estimate, which
/// [`CostEstimate::interval`] reports honestly rather than dividing by zero.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn estimate_cost(
    inputs: CostInputs,
    completed_runs_in_class: i64,
) -> Answer<CostEstimate> {
    let (rung, evidence) = choose_rung("M1", completed_runs_in_class, None, false, false);
    let work = inputs.rows as f64 * f64::from(inputs.features) * f64::from(inputs.steps);
    let seconds = if inputs.throughput > 0.0 { work / inputs.throughput } else { f64::INFINITY };
    Answer {
        value: CostEstimate { seconds, band: ANALYTIC_COST_BAND },
        // The closed form is `analytic`, one rung below a tuned rule table: it
        // has no numbers in it that anyone chose.
        rung: if rung == Rung::Rule { Rung::Analytic } else { rung },
        evidence,
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// M2 — failure
// ───────────────────────────────────────────────────────────────────────────────

/// A failure the platform can see coming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredictedFailure {
    /// The working set does not fit the device.
    OutOfMemory,
    /// The learning rate is outside the envelope this optimiser survives.
    Divergence,
    /// Nothing predictable. **Not** "it will work" — most failures are only
    /// discovered by running, and this says the pre-flight found nothing.
    NothingPredictable,
}

impl PredictedFailure {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OutOfMemory => "oom",
            Self::Divergence => "nan_divergence",
            Self::NothingPredictable => "none",
        }
    }
}

/// What the pre-flight reads.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreflightInputs {
    /// Parameters × bytes per parameter × the optimiser's state multiplier.
    pub working_set_bytes: u64,
    pub device_bytes: u64,
    pub learning_rate: f64,
    /// Whether the optimiser adapts its own step size. Adam tolerates roughly
    /// two orders of magnitude more than plain SGD before it diverges.
    pub adaptive_optimiser: bool,
}

/// The fraction of device memory a run may plan to occupy.
///
/// Eighty per cent. The remainder is fragmentation, the allocator's caching and
/// the activation peak that no parameter count predicts — a run planned at 100 %
/// of the card fails, reliably, at a point that looks random.
pub const DEVICE_HEADROOM: f64 = 0.8;

/// Learning-rate envelopes, by optimiser.
pub const SGD_LR_MAX: f64 = 1.0;
pub const ADAPTIVE_LR_MAX: f64 = 0.1;

/// M2's rule tier.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn predict_failure(
    inputs: PreflightInputs,
    runs_observed: i64,
    per_class: Option<i64>,
) -> Answer<PredictedFailure> {
    let (rung, evidence) = choose_rung("M2", runs_observed, per_class, false, false);
    let budget = inputs.device_bytes as f64 * DEVICE_HEADROOM;
    let lr_max = if inputs.adaptive_optimiser { ADAPTIVE_LR_MAX } else { SGD_LR_MAX };

    let value = if inputs.device_bytes > 0 && inputs.working_set_bytes as f64 > budget {
        PredictedFailure::OutOfMemory
    } else if !inputs.learning_rate.is_finite()
        || inputs.learning_rate <= 0.0
        || inputs.learning_rate > lr_max
    {
        PredictedFailure::Divergence
    } else {
        PredictedFailure::NothingPredictable
    };
    Answer { value, rung: if rung == Rung::Rule { Rung::Rule } else { rung }, evidence }
}

// ───────────────────────────────────────────────────────────────────────────────
// M11 — anomaly
// ───────────────────────────────────────────────────────────────────────────────

/// What a curve looks wrong in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anomaly {
    /// A loss that is NaN or infinite. Not an outlier — the run is over.
    NonFinite,
    /// The loss has not moved. Either nothing is learning or nothing is
    /// connected, and both are worth stopping for.
    Flat,
    /// A single step far outside the curve's own robust spread.
    Spike,
    /// A sustained drift the step-by-step view does not show. CUSUM catches the
    /// slow divergence that a per-step threshold never triggers on.
    Drift,
    None,
}

impl Anomaly {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NonFinite => "non_finite",
            Self::Flat => "flat",
            Self::Spike => "spike",
            Self::Drift => "drift",
            Self::None => "none",
        }
    }

    /// Whether this should stop the run rather than merely flag it.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::NonFinite)
    }
}

/// Robust-z threshold. Six median-absolute-deviations, not three: a training
/// curve's step-to-step variation is heavy-tailed, and three would fire on every
/// second run.
pub const SPIKE_Z: f64 = 6.0;

/// CUSUM decision interval, in MADs of the curve's own increments.
pub const CUSUM_H: f64 = 5.0;

/// CUSUM slack: the rise, in MADs per step, that is absorbed before anything
/// accumulates. Half a MAD — enough that ordinary noise never drifts, small
/// enough that a persistent climb fires within about ten steps.
pub const CUSUM_K: f64 = 0.5;

/// The window a spike is judged against its neighbours in. Odd, so the window
/// has a true middle.
pub const SPIKE_WINDOW: usize = 11;

/// The relative movement below which a curve counts as flat.
pub const FLAT_TOLERANCE: f64 = 1e-9;

/// M11's rule tier over one learning curve.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn detect_anomaly(curve: &[f64], curves_observed: i64) -> Answer<Anomaly> {
    let (rung, evidence) = choose_rung("M11", curves_observed, None, false, false);
    let value = classify_curve(curve);
    Answer { value, rung, evidence }
}

fn classify_curve(curve: &[f64]) -> Anomaly {
    // Hard rules first. A non-finite loss is not a point to be scored against
    // the others; every statistic computed after it is also non-finite.
    if curve.iter().any(|v| !v.is_finite()) {
        return Anomaly::NonFinite;
    }
    if curve.len() < 4 {
        return Anomaly::None;
    }
    let first = curve[0];
    let scale = curve.iter().fold(0.0_f64, |a, b| a.max(b.abs())).max(1.0);
    if curve.iter().all(|v| (v - first).abs() < FLAT_TOLERANCE * scale) {
        return Anomaly::Flat;
    }

    // A spike is a point far from its **neighbours**, not from the curve's mean
    // increment. Two reasons it has to be local:
    //
    //   * a healthy curve decelerates, so its early increments are large and its
    //     late ones tiny — scoring increments against one spread calls the whole
    //     first half anomalous;
    //   * a single wild value contributes a huge increment *and* a huge one back,
    //     which inflates the spread enough to hide itself. That is the classic
    //     masking failure, and it hides exactly the case this is for.
    let residuals = local_residuals(curve, SPIKE_WINDOW);
    let scale = curve.iter().fold(0.0_f64, |a, b| a.max(b.abs())).max(1.0);
    let spiked = if residuals.is_empty() {
        // Too short to have an interior. Not "no spike" as a finding — nothing
        // was looked at, and CUSUM below still sees the whole curve.
        false
    } else {
        match median_absolute_deviation(&residuals) {
            Some(mad) => residuals.iter().any(|r| r.abs() > SPIKE_Z * mad),
            // A perfectly smooth curve has zero residuals and no spread to
            // score against. Anything that stands out at all against the
            // curve's own magnitude is then the only outlier there is.
            None => residuals.iter().any(|r| r.abs() > 1e-3 * scale),
        }
    };
    if spiked {
        return Anomaly::Spike;
    }

    // CUSUM on the increments, one-sided **upward**, centred on zero.
    //
    // Zero, not the observed mean increment: centring on the mean would subtract
    // out the very drift being looked for. A loss that keeps rising is the
    // failure; a loss that keeps falling is the job, and it accumulates nothing.
    let diffs: Vec<f64> = curve.windows(2).map(|w| w[1] - w[0]).collect();
    let Some(step_mad) = median_absolute_deviation(&diffs) else {
        return Anomaly::None;
    };
    let mut high = 0.0_f64;
    for d in &diffs {
        high = (high + d / step_mad - CUSUM_K).max(0.0);
        if high > CUSUM_H {
            return Anomaly::Drift;
        }
    }
    Anomaly::None
}

/// Each point minus the median of a centred window around it.
///
/// For a monotone curve the window median *is* the point, so a smooth run has
/// residuals of zero — which is the property that keeps a decelerating loss from
/// reading as a hundred anomalies.
fn local_residuals(curve: &[f64], window: usize) -> Vec<f64> {
    let half = window / 2;
    if curve.len() <= window {
        return Vec::new();
    }
    // Only points with a full window on both sides are judged. A truncated
    // window is not centred on its point, so a perfectly smooth curve would show
    // a large residual at each end and read as two spikes — an artefact of where
    // the curve happens to start, not of the run.
    (half..curve.len() - half)
        .map(|i| curve[i] - median(&curve[i - half..=i + half]))
        .collect()
}

fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len() % 2 == 0 {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

/// `1.4826 · median(|x − median(x)|)` — the MAD scaled to estimate a Gaussian
/// standard deviation. `None` when it is zero, because dividing by it would turn
/// every point into an infinite z-score.
fn median_absolute_deviation(xs: &[f64]) -> Option<f64> {
    let m = median(xs);
    let deviations: Vec<f64> = xs.iter().map(|x| (x - m).abs()).collect();
    let mad = 1.4826 * median(&deviations);
    (mad > 0.0).then_some(mad)
}

/// A decision's tier, for the ledger. Re-exported so a caller recording an
/// answer does not have to reach into two crates.
#[must_use]
pub fn tier_of<T>(answer: &Answer<T>) -> ledger::DecisionTier {
    answer.decision_tier()
}

/// The evidence behind a tier choice, for the decision row's rationale.
#[must_use]
pub fn evidence_of<T>(answer: &Answer<T>) -> LadderEvidence {
    answer.evidence
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── M1 ──────────────────────────────────────────────────────────────────

    #[test]
    fn the_cost_estimate_is_a_band_and_says_so() {
        let a = estimate_cost(
            CostInputs { rows: 1_000_000, features: 50, steps: 10, throughput: 1e8 },
            0,
        );
        assert_eq!(a.rung, Rung::Analytic);
        assert_eq!(a.decision_tier(), ledger::DecisionTier::Rule);
        assert!((a.value.seconds - 5.0).abs() < 1e-9, "{:?}", a.value);
        let (low, high) = a.value.interval();
        assert!(low < a.value.seconds && a.value.seconds < high);
        assert!((high / low - 9.0).abs() < 1e-9, "a factor of three either way");
    }

    #[test]
    fn cost_scales_with_every_input_it_claims_to_read() {
        let base = CostInputs { rows: 1_000, features: 10, steps: 2, throughput: 1.0 };
        let one = estimate_cost(base, 0).value.seconds;
        let more_rows = estimate_cost(CostInputs { rows: 2_000, ..base }, 0).value.seconds;
        let more_steps = estimate_cost(CostInputs { steps: 4, ..base }, 0).value.seconds;
        let faster = estimate_cost(CostInputs { throughput: 2.0, ..base }, 0).value.seconds;
        assert!((more_rows / one - 2.0).abs() < 1e-9);
        assert!((more_steps / one - 2.0).abs() < 1e-9);
        assert!((faster / one - 0.5).abs() < 1e-9);
    }

    /// Zero throughput is an unknown hardware class, and the honest answer is
    /// "unbounded" rather than a division by zero or a made-up default.
    #[test]
    fn an_unknown_hardware_class_does_not_produce_a_number() {
        let a = estimate_cost(
            CostInputs { rows: 10, features: 1, steps: 1, throughput: 0.0 },
            0,
        );
        assert!(a.value.seconds.is_infinite());
    }

    #[test]
    fn m1_stays_analytic_until_its_trigger_is_met_and_a_model_exists() {
        let one = CostInputs { rows: 1, features: 1, steps: 1, throughput: 1.0 };
        assert!(!estimate_cost(one, 199).evidence.met(), "199 runs is not 200");
        assert!(estimate_cost(one, 200).evidence.met());
        // Met or not, nothing is fitted, so the answer is still analytic.
        assert_eq!(estimate_cost(one, 5_000).rung, Rung::Analytic);
    }

    // ── M2 ──────────────────────────────────────────────────────────────────

    fn preflight() -> PreflightInputs {
        PreflightInputs {
            working_set_bytes: 4 << 30,
            device_bytes: 11 << 30,
            learning_rate: 1e-3,
            adaptive_optimiser: true,
        }
    }

    #[test]
    fn a_working_set_that_does_not_fit_is_predicted_oom() {
        let a = predict_failure(
            PreflightInputs { working_set_bytes: 10 << 30, ..preflight() },
            0,
            None,
        );
        assert_eq!(a.value, PredictedFailure::OutOfMemory);
        assert_eq!(a.value.as_str(), "oom", "the prediction names the TerminalReason it expects");
    }

    /// Eighty per cent, not a hundred: a run planned to fill the card fails at a
    /// point that looks random.
    #[test]
    fn the_memory_check_leaves_headroom() {
        let just_over = (11u64 << 30) as f64 * DEVICE_HEADROOM + 1.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let a = predict_failure(
            PreflightInputs { working_set_bytes: just_over as u64, ..preflight() },
            0,
            None,
        );
        assert_eq!(a.value, PredictedFailure::OutOfMemory);
    }

    #[test]
    fn the_learning_rate_envelope_depends_on_the_optimiser() {
        let adam = predict_failure(
            PreflightInputs { learning_rate: 0.5, adaptive_optimiser: true, ..preflight() },
            0,
            None,
        );
        assert_eq!(adam.value, PredictedFailure::Divergence);
        let sgd = predict_failure(
            PreflightInputs { learning_rate: 0.5, adaptive_optimiser: false, ..preflight() },
            0,
            None,
        );
        assert_eq!(sgd.value, PredictedFailure::NothingPredictable);
    }

    /// "Nothing predictable" is not "it will work". Most failures are only
    /// discovered by running, and the pre-flight claims nothing about them.
    #[test]
    fn a_clean_preflight_predicts_nothing_rather_than_success() {
        let a = predict_failure(preflight(), 0, None);
        assert_eq!(a.value, PredictedFailure::NothingPredictable);
        assert_eq!(a.value.as_str(), "none");
    }

    // ── M11 ─────────────────────────────────────────────────────────────────

    #[test]
    fn a_non_finite_loss_is_terminal_not_an_outlier() {
        let a = detect_anomaly(&[1.0, 0.8, f64::NAN, 0.5], 0);
        assert_eq!(a.value, Anomaly::NonFinite);
        assert!(a.value.is_terminal());
        // And it is found before any statistic is computed over the curve.
        let all_nan = detect_anomaly(&[f64::INFINITY; 8], 0);
        assert_eq!(all_nan.value, Anomaly::NonFinite);
    }

    #[test]
    fn a_curve_that_never_moves_is_flat() {
        let a = detect_anomaly(&[0.693; 40], 0);
        assert_eq!(a.value, Anomaly::Flat);
        assert!(!a.value.is_terminal(), "flat is worth stopping for, not a crash");
    }

    #[test]
    fn a_healthy_curve_is_not_an_anomaly() {
        let curve: Vec<f64> = (0..60).map(|i| 1.0 / (1.0 + f64::from(i) * 0.05)).collect();
        assert_eq!(detect_anomaly(&curve, 0).value, Anomaly::None);
    }

    #[test]
    fn one_wild_step_is_a_spike() {
        let mut curve: Vec<f64> = (0..60).map(|i| 1.0 - f64::from(i) * 0.01).collect();
        curve[30] = 50.0;
        assert_eq!(detect_anomaly(&curve, 0).value, Anomaly::Spike);
    }

    /// The slow divergence a per-step threshold never fires on.
    #[test]
    fn a_sustained_rise_is_drift() {
        let mut r = 0.0_f64;
        let curve: Vec<f64> = (0..200)
            .map(|i| {
                r += 0.01;
                // A gentle, persistent climb with a little noise.
                0.5 + r + if i % 2 == 0 { 0.001 } else { -0.001 }
            })
            .collect();
        let a = detect_anomaly(&curve, 0);
        assert!(matches!(a.value, Anomaly::Drift | Anomaly::Spike), "{:?}", a.value);
    }

    #[test]
    fn a_curve_too_short_to_judge_is_not_judged() {
        assert_eq!(detect_anomaly(&[1.0, 0.9], 0).value, Anomaly::None);
        assert_eq!(detect_anomaly(&[], 0).value, Anomaly::None);
    }

    #[test]
    fn every_answer_carries_its_tier_and_its_evidence() {
        let a = detect_anomaly(&[1.0, 0.9, 0.8, 0.7], 42);
        assert_eq!(tier_of(&a), ledger::DecisionTier::Rule);
        assert_eq!(evidence_of(&a).observations, 42);
        assert_eq!(evidence_of(&a).required, 300);
        assert!(evidence_of(&a).describe().contains("42 of 300"));
    }
}
