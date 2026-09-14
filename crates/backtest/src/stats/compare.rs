//! The comparison protocol (SPEC §11.5, checklist 2.12/2.13, ADR-P2-12/13).
//!
//! Two things live here, and they are the same thing seen at two scales: a
//! [`Comparison`] is one pair judged, and a [`ComparisonMatrix`] is every pair
//! laid beside each other.
//!
//! ## Why a `Comparison` is sealed
//!
//! The failure mode this module exists to prevent is not a wrong number. It is
//! a *bare* number — "A wins" with nothing attached — travelling up through the
//! platform until somebody deploys on it. So [`Comparison`] has no public
//! constructor and no public fields. The only way to make one is
//! [`ComparisonPlan::judge`], which cannot be called without the gate profile,
//! the effective trial count, the declared `delta_practical`, both candidates'
//! trial counts and every flag that qualifies the verdict; a static test
//! (`only_the_protocol_constructs_a_comparison`) keeps that the only way.
//!
//! The intended consequence is that a promotion path takes a `&Comparison`, so a
//! `StudyResult` — an in-search validation score, which is not an estimate of
//! anything out of sample — cannot be passed in its place. **That consequence is
//! not yet realised**: the COMPARE phase's worker is not built (checklist 2.1),
//! so nothing consumes a `Comparison` today. What holds now is the seal, not the
//! substitution ban.
//!
//! ## Replicates
//!
//! A strategy backtest is deterministic given its data, so "run it again" is not
//! a replicate. The nuisance that a replicate has to vary is the *analyst's*
//! arbitrary choices, and there are three of them: which window of history,
//! which cost assumption, and which stochastic-fill seed. A replicate is one
//! draw from that product, and the same draw is applied to **both** candidates —
//! paired, so the difference cancels everything the two share and the variance
//! being estimated is the variance of the difference rather than of the level.
//!
//! ## The confidence sequence
//!
//! Every comparison here is looked at repeatedly — that is the point of
//! sampling until a decision — so a fixed-sample interval would be wrong the
//! moment anyone peeked. [`confidence_sequence`] is the predictable plug-in
//! empirical-Bernstein confidence sequence of Waudby-Smith and Ramdas: valid at
//! every `t` simultaneously, so stopping when it separates is legitimate rather
//! than the optional-stopping fallacy with extra steps.
//!
//! It is defined for observations in `[0, 1]`, so the paired differences are
//! mapped through a **declared** scale ([`RopeScale`]). A difference outside the
//! declared scale is an error, never a clipped value: clipping would silently
//! change the estimand into something narrower than what was asked about.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::bh_adjusted;

/// The §11.5 target replicate count. Reported, never assumed: `achieved_k` is a
/// field of every comparison because a verdict from six replicates and a verdict
/// from twenty-nine are not the same verdict.
pub const TARGET_REPLICATES: usize = 29;

/// Below this, the confidence sequence is too wide to separate anything and the
/// protocol says so rather than producing a verdict from four numbers.
pub const MIN_REPLICATES: usize = 5;

/// The §11.5 decision thresholds. Constants, not parameters: a caller who could
/// choose them could choose them after seeing the data.
pub const MIN_P_A_BETTER: f64 = 0.75;
pub const MIN_P_A_BETTER_LOWER: f64 = 0.5;

/// Default two-sided error level for the confidence sequence.
pub const DEFAULT_ALPHA: f64 = 0.05;

/// Why a comparison could not be made.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CompareError {
    #[error("need at least {MIN_REPLICATES} paired replicates, have {have}")]
    TooFewReplicates { have: usize },
    #[error("A and B were measured on {a} and {b} replicates; a paired comparison needs the same ones")]
    NotPaired { a: usize, b: usize },
    #[error("replicate {index} differs between A and B ({a:?} vs {b:?}); pairing is what makes the difference meaningful")]
    MismatchedReplicate { index: usize, a: Replicate, b: Replicate },
    #[error("delta_practical must be positive and finite; got {0}")]
    BadDelta(f64),
    #[error("the declared scale must be positive and finite; got {0}")]
    BadScale(f64),
    #[error("paired difference {difference} at replicate {index} lies outside the declared scale of ±{scale}; widen the scale rather than clipping the observation")]
    OutOfScale { index: usize, difference: f64, scale: f64 },
    #[error("a metric was not finite at replicate {index}")]
    NotFinite { index: usize },
}

// ───────────────────────────────────────────────────────────────────────────────
// replicates
// ───────────────────────────────────────────────────────────────────────────────

/// One draw of the nuisance parameters, applied identically to both candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Replicate {
    /// Which out-of-bootstrap window of the research slice.
    pub window: u32,
    /// Cost-model perturbation in basis points of the modelled cost, ×100 so the
    /// replicate stays hashable and comparable. `+2000` is +20 %.
    pub cost_bp: i32,
    /// Seed for stochastic fills.
    pub seed: u64,
}

/// The replicate grid for a comparison.
///
/// It is generated from the pair's identity, not drawn at call time, so two runs
/// of the same comparison use the same replicates and a third party can rebuild
/// them. There is no argument that lets a caller pick a friendlier grid.
#[must_use]
pub fn replicate_grid(k: usize, base_seed: u64) -> Vec<Replicate> {
    // Three cost perturbations (−20 %, modelled, +20 %) crossed with windows,
    // each with its own fill seed. §11.5's nuisance product, enumerated rather
    // than sampled, so the grid is the same every time.
    const COSTS: [i32; 3] = [-2000, 0, 2000];
    (0..k)
        .map(|i| Replicate {
            window: u32::try_from(i / COSTS.len()).unwrap_or(u32::MAX),
            cost_bp: COSTS[i % COSTS.len()],
            seed: base_seed.wrapping_add(i as u64),
        })
        .collect()
}

/// One candidate's metric on one replicate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub replicate: Replicate,
    /// The objective's value — higher is better, by construction of the objective.
    pub value: f64,
}

// ───────────────────────────────────────────────────────────────────────────────
// the anytime-valid confidence sequence
// ───────────────────────────────────────────────────────────────────────────────

/// A declared bound on the size of a paired difference.
///
/// Sealed for the same reason `ExplorationFloor` is: an observation outside the
/// scale must be an error the caller sees, and a scale that could be changed
/// after the fact would be a knob for making an inconvenient observation fit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RopeScale(f64);

impl RopeScale {
    /// The scale is a pre-registered statement about how large a difference
    /// could plausibly be, and it cuts both ways: the confidence sequence's
    /// width is proportional to it, so a generous scale makes separation harder,
    /// and a flattering one makes real observations fall outside it and be
    /// refused. There is no setting that is both narrow and safe, which is what
    /// stops it from being a knob.
    ///
    /// # Errors
    /// A non-positive or non-finite half-width.
    pub fn new(half_width: f64) -> Result<Self, CompareError> {
        if !half_width.is_finite() || half_width <= 0.0 {
            return Err(CompareError::BadScale(half_width));
        }
        Ok(Self(half_width))
    }

    #[must_use]
    pub fn half_width(self) -> f64 {
        self.0
    }

    /// Map a difference into `[0, 1]`.
    fn to_unit(self, d: f64) -> Option<f64> {
        let u = (d + self.0) / (2.0 * self.0);
        (0.0..=1.0).contains(&u).then_some(u)
    }

    /// Map a `[0, 1]` coordinate back to the difference scale.
    fn unit_to_difference(self, u: f64) -> f64 {
        u * 2.0 * self.0 - self.0
    }
}

/// An interval that is valid at every sample size at once.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceSequence {
    pub centre: f64,
    pub lower: f64,
    pub upper: f64,
    /// How many observations it was computed from.
    pub t: usize,
    pub alpha: f64,
}

/// Predictable plug-in empirical-Bernstein confidence sequence for observations
/// in `[0, 1]` (Waudby-Smith & Ramdas 2023, §3.2).
///
/// Anytime-valid: `P(∃t : μ ∉ CI_t) ≤ α`. That is the property that makes
/// "sample until it separates" a protocol rather than p-hacking, and it is why
/// §11.5 asks for this rather than a t-interval recomputed at each look.
///
/// Returns `None` for an empty sample.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn confidence_sequence(unit: &[f64], alpha: f64) -> Option<ConfidenceSequence> {
    if unit.is_empty() {
        return None;
    }
    let log_term = (2.0 / alpha).ln();

    // Running estimates use only the past, which is what "predictable" means and
    // what keeps the martingale a martingale. The 1/2 and 1/4 priors are the
    // paper's: the mean and variance of a uniform on [0, 1].
    let mut sum = 0.5_f64;
    let mut count = 1.0_f64;
    let mut var_sum = 0.25_f64;
    let mut var_count = 1.0_f64;

    let mut num = 0.0_f64; // Σ λ_i X_i
    let mut den = 0.0_f64; // Σ λ_i
    let mut penalty = 0.0_f64; // Σ v_i ψ_E(λ_i)

    for (i, &x) in unit.iter().enumerate() {
        let t = (i + 1) as f64;
        let mu_prev = sum / count;
        let sigma2_prev = var_sum / var_count;

        // λ_i, capped below 1 so ψ_E stays finite.
        let raw = (2.0 * log_term / (sigma2_prev.max(1e-12) * t * (1.0 + t).ln())).sqrt();
        let lambda = raw.clamp(1e-9, 0.5);

        num += lambda * x;
        den += lambda;
        let v = 4.0 * (x - mu_prev).powi(2);
        penalty += v * psi_e(lambda);

        sum += x;
        count += 1.0;
        var_sum += (x - mu_prev).powi(2);
        var_count += 1.0;
    }

    let centre = num / den;
    let width = (log_term + penalty) / den;
    Some(ConfidenceSequence {
        centre,
        lower: (centre - width).max(0.0),
        upper: (centre + width).min(1.0),
        t: unit.len(),
        alpha,
    })
}

/// `ψ_E(λ) = (−ln(1−λ) − λ) / 4`.
fn psi_e(lambda: f64) -> f64 {
    (-(1.0 - lambda).ln() - lambda) / 4.0
}

// ───────────────────────────────────────────────────────────────────────────────
// the verdict
// ───────────────────────────────────────────────────────────────────────────────

/// What the protocol concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The confidence sequence's lower bound is above `+δ`: A beats B by more
    /// than the effect size that was declared to matter, before the data was seen.
    Promote,
    /// The upper bound is below `+δ`: whatever A's advantage is, it is smaller
    /// than the improvement this campaign said would change a decision. This is
    /// not "B is better"; it is "not by enough".
    Reject,
    /// The interval still spans `+δ`. More replicates would help.
    KeepSampling,
    /// The interval still spans `+δ` at `k_max`. §11.5 calls this "practically
    /// equivalent"; read precisely, it means the protocol has spent its budget
    /// and the data does not separate the two at `δ`. It is not evidence that
    /// they are the same.
    PracticallyEquivalent,
}

impl Verdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::Reject => "reject",
            Self::KeepSampling => "keep_sampling",
            Self::PracticallyEquivalent => "practically_equivalent",
        }
    }

    /// Only one verdict authorises anything.
    #[must_use]
    pub fn promotes(self) -> bool {
        matches!(self, Self::Promote)
    }
}

/// Everything that qualifies a verdict (§11.5, §12.6).
///
/// Every field defaults to `false`, and that is safe here precisely because each
/// one is a *warning*: the default is "nothing known to be wrong", and anything
/// that sets one must set it explicitly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
// Five independent warnings really are five independent booleans: they are not
// stages of anything and any subset can hold at once. Collapsing them into a
// state machine would lose exactly the information a reader needs, which is
// *which* ones are raised.
#[allow(clippy::struct_excessive_bools)]
pub struct ComparisonFlags {
    /// The two candidates were not measured on comparable ground (different
    /// universes, resolutions or cost models beyond the replicate perturbation).
    pub non_comparable: bool,
    /// Either side's labels overlap and were not weighted (ADR-P1-01).
    pub overlapping_labels_unweighted: bool,
    /// Either side ran with a split override.
    pub split_overrides: bool,
    /// Either side's data carries backfilled knowledge times (INV-01).
    pub uses_backfilled_knowledge: bool,
    /// Either side predates the gate stack it is being judged against
    /// (ADR-P2-27).
    pub legacy_ungated: bool,
}

impl ComparisonFlags {
    /// Whether anything at all qualifies this comparison.
    #[must_use]
    pub fn any(&self) -> bool {
        self.non_comparable
            || self.overlapping_labels_unweighted
            || self.split_overrides
            || self.uses_backfilled_knowledge
            || self.legacy_ungated
    }

    /// The flags that are set, for display beside the number.
    #[must_use]
    pub fn raised(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.non_comparable {
            out.push("non_comparable");
        }
        if self.overlapping_labels_unweighted {
            out.push("overlapping_labels_unweighted");
        }
        if self.split_overrides {
            out.push("split_overrides");
        }
        if self.uses_backfilled_knowledge {
            out.push("uses_backfilled_knowledge");
        }
        if self.legacy_ungated {
            out.push("legacy_ungated");
        }
        out
    }
}

/// The context a comparison cannot be made without.
#[derive(Clone, Debug, PartialEq)]
pub struct ComparisonPlan {
    /// The versioned gate profile this pair is judged under (INV-23).
    pub profile_id: String,
    /// Effective trial count at the moment of judgement (§12.2).
    pub n_eff: f64,
    /// The pre-registered effect size. Positive, and not chosen now.
    pub delta_practical: f64,
    /// The declared bound on a paired difference.
    pub scale: RopeScale,
    /// How many trials each candidate has cost so far.
    pub trials_a: i64,
    pub trials_b: i64,
    /// The most replicates this comparison may spend.
    pub k_max: usize,
    pub flags: ComparisonFlags,
    pub alpha: f64,
}

/// One pair, judged. Sealed: no public fields, no public constructor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    verdict: Verdict,
    /// The confidence sequence on the paired difference, on the difference scale.
    interval: ConfidenceSequence,
    /// Fraction of replicates on which A beat B.
    p_a_better: f64,
    /// Anytime-valid lower bound on that fraction.
    p_a_better_lower: f64,
    mean_difference: f64,
    achieved_k: usize,
    wins: usize,
    ties: usize,
    losses: usize,
    profile_id: String,
    n_eff: f64,
    delta_practical: f64,
    trials_a: i64,
    trials_b: i64,
    flags: ComparisonFlags,
}

impl Comparison {
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        self.verdict
    }
    #[must_use]
    pub fn interval(&self) -> ConfidenceSequence {
        self.interval
    }
    #[must_use]
    pub fn p_a_better(&self) -> f64 {
        self.p_a_better
    }
    #[must_use]
    pub fn p_a_better_lower(&self) -> f64 {
        self.p_a_better_lower
    }
    #[must_use]
    pub fn mean_difference(&self) -> f64 {
        self.mean_difference
    }
    #[must_use]
    pub fn achieved_k(&self) -> usize {
        self.achieved_k
    }
    /// Wins, ties and losses across the replicates — the MCM's cell.
    #[must_use]
    pub fn record(&self) -> (usize, usize, usize) {
        (self.wins, self.ties, self.losses)
    }
    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }
    #[must_use]
    pub fn n_eff(&self) -> f64 {
        self.n_eff
    }
    #[must_use]
    pub fn delta_practical(&self) -> f64 {
        self.delta_practical
    }
    #[must_use]
    pub fn trial_counts(&self) -> (i64, i64) {
        (self.trials_a, self.trials_b)
    }
    #[must_use]
    pub fn flags(&self) -> &ComparisonFlags {
        &self.flags
    }

    /// Whether the §11.5 win-rate rule is satisfied, independently of the
    /// interval. Both must hold for a promotion; this is the half a reader can
    /// check by eye.
    #[must_use]
    pub fn win_rate_rule_holds(&self) -> bool {
        self.p_a_better >= MIN_P_A_BETTER && self.p_a_better_lower > MIN_P_A_BETTER_LOWER
    }

    /// A one-line summary that cannot be reduced to "A wins": the number always
    /// arrives with the count of looks behind it.
    #[must_use]
    pub fn headline(&self) -> String {
        let flags = if self.flags.any() {
            format!(" [{}]", self.flags.raised().join(", "))
        } else {
            String::new()
        };
        format!(
            "{} — mean Δ {:+.4} (CS {:+.4}..{:+.4} at α={:.2}), P(A>B)={:.2}, k={}, N_eff={:.1}, δ={:.4}, profile {}{}",
            self.verdict.as_str(),
            self.mean_difference,
            self.interval.lower,
            self.interval.upper,
            self.interval.alpha,
            self.p_a_better,
            self.achieved_k,
            self.n_eff,
            self.delta_practical,
            self.profile_id,
            flags,
        )
    }
}

impl ComparisonPlan {
    /// Judge one pair.
    ///
    /// `a` and `b` must be measurements of the **same** replicates in the same
    /// order; a mismatch is refused rather than silently zipped, because an
    /// unpaired difference estimates a different and much noisier quantity.
    ///
    /// # Errors
    /// Too few replicates, unpaired or mismatched replicates, a non-positive
    /// `delta_practical`, a non-finite metric, or a difference outside the
    /// declared scale.
    #[allow(clippy::cast_precision_loss)]
    pub fn judge(&self, a: &[Measurement], b: &[Measurement]) -> Result<Comparison, CompareError> {
        if !self.delta_practical.is_finite() || self.delta_practical <= 0.0 {
            return Err(CompareError::BadDelta(self.delta_practical));
        }
        if a.len() != b.len() {
            return Err(CompareError::NotPaired { a: a.len(), b: b.len() });
        }
        if a.len() < MIN_REPLICATES {
            return Err(CompareError::TooFewReplicates { have: a.len() });
        }

        let mut differences = Vec::with_capacity(a.len());
        for (i, (ma, mb)) in a.iter().zip(b.iter()).enumerate() {
            if ma.replicate != mb.replicate {
                return Err(CompareError::MismatchedReplicate {
                    index: i,
                    a: ma.replicate,
                    b: mb.replicate,
                });
            }
            if !ma.value.is_finite() || !mb.value.is_finite() {
                return Err(CompareError::NotFinite { index: i });
            }
            differences.push(ma.value - mb.value);
        }

        let mut unit = Vec::with_capacity(differences.len());
        for (i, &d) in differences.iter().enumerate() {
            let u = self.scale.to_unit(d).ok_or(CompareError::OutOfScale {
                index: i,
                difference: d,
                scale: self.scale.half_width(),
            })?;
            unit.push(u);
        }

        let cs = confidence_sequence(&unit, self.alpha).expect("non-empty");
        let interval = ConfidenceSequence {
            centre: self.scale.unit_to_difference(cs.centre),
            lower: self.scale.unit_to_difference(cs.lower),
            upper: self.scale.unit_to_difference(cs.upper),
            t: cs.t,
            alpha: cs.alpha,
        };

        let wins = differences.iter().filter(|d| **d > 0.0).count();
        let losses = differences.iter().filter(|d| **d < 0.0).count();
        let ties = differences.len() - wins - losses;

        // P(A>B) gets its own anytime-valid bound, over the same looks. A tie
        // counts as half a win, which is the usual convention and is what makes
        // a run of identical results read as "no evidence" rather than a loss.
        let indicators: Vec<f64> = differences
            .iter()
            .map(|d| {
                if *d > 0.0 {
                    1.0
                } else if *d < 0.0 {
                    0.0
                } else {
                    0.5
                }
            })
            .collect();
        let p_cs = confidence_sequence(&indicators, self.alpha).expect("non-empty");
        let p_a_better = indicators.iter().sum::<f64>() / indicators.len() as f64;

        let mean_difference = differences.iter().sum::<f64>() / differences.len() as f64;
        let delta = self.delta_practical;
        let separated_above = interval.lower > delta;
        let separated_below = interval.upper < delta;
        let win_rate = p_a_better >= MIN_P_A_BETTER && p_cs.lower > MIN_P_A_BETTER_LOWER;

        let verdict = if separated_above && win_rate {
            Verdict::Promote
        } else if separated_below {
            Verdict::Reject
        } else if differences.len() >= self.k_max {
            Verdict::PracticallyEquivalent
        } else {
            Verdict::KeepSampling
        };

        Ok(Comparison {
            verdict,
            interval,
            p_a_better,
            p_a_better_lower: p_cs.lower,
            mean_difference,
            achieved_k: differences.len(),
            wins,
            ties,
            losses,
            profile_id: self.profile_id.clone(),
            n_eff: self.n_eff,
            delta_practical: self.delta_practical,
            trials_a: self.trials_a,
            trials_b: self.trials_b,
            flags: self.flags.clone(),
        })
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// multiplicity
// ───────────────────────────────────────────────────────────────────────────────

/// Holm–Bonferroni step-down adjusted p-values.
///
/// Holm for *promotion* decisions, because promotion authorises capital and the
/// error that matters is promoting one thing that should not have been: Holm
/// controls the family-wise error rate, the probability of **any** false
/// promotion. [`bhy_adjusted`] controls the false discovery *rate* and is the
/// right instrument for screening, where a few false leads cost a little compute
/// and missing a real one costs the discovery.
#[must_use]
pub fn holm_adjusted(p_values: &[f64]) -> Vec<f64> {
    let m = p_values.len();
    if m == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&i, &j| p_values[i].partial_cmp(&p_values[j]).unwrap_or(std::cmp::Ordering::Equal));

    let mut adjusted = vec![0.0; m];
    let mut running: f64 = 0.0;
    for (rank, &idx) in order.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let scaled = ((m - rank) as f64 * p_values[idx]).min(1.0);
        // Step-down monotonicity: an adjusted p-value never decreases down the
        // ordering, so a later hypothesis cannot look stronger than an earlier
        // one it was tested after.
        running = running.max(scaled);
        adjusted[idx] = running;
    }
    adjusted
}

/// Screening adjustment: Benjamini–Hochberg.
///
/// Named for its role rather than its author so the choice between the two is
/// made by asking what the family is for, not by reaching for whichever one is
/// already imported. Holm is strictly more conservative — `holm_adjusted(p)[i] ≥
/// screening_adjusted(p)[i]` for every `i` — which is the shape the asymmetry
/// should have: a false promotion costs capital, a false lead costs compute.
#[must_use]
pub fn screening_adjusted(p_values: &[f64]) -> Vec<f64> {
    bh_adjusted(p_values)
}

// ───────────────────────────────────────────────────────────────────────────────
// the Multiple Comparison Matrix (2.13)
// ───────────────────────────────────────────────────────────────────────────────

/// One cell of the MCM.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatrixCell {
    pub mean_difference: f64,
    pub wins: usize,
    pub ties: usize,
    pub losses: usize,
    /// Wilcoxon signed-rank p-value, **descriptive only**.
    ///
    /// §11.5 is explicit that this is a measure of how far apart two samples
    /// are, not a test anything is allowed to turn on. The promotion decision is
    /// the confidence sequence and the win-rate rule; this number is here so a
    /// reader can see the divergence, and it carries no verdict.
    pub wilcoxon_p_descriptive: f64,
    pub verdict: Verdict,
    pub achieved_k: usize,
}

/// Every pair, side by side (§11.5).
///
/// This is the comparison view. There is no critical-difference diagram, here or
/// anywhere: §11.5 bans it, and AT-62 is a static test that no renderer, type or
/// endpoint for one exists. The reason is that a CD diagram's cliques depend on
/// which other methods happen to be in the comparison, so adding an unrelated
/// candidate can make two methods that did not move become "not significantly
/// different". A matrix states each pair's own evidence and lets the reader do
/// the comparing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComparisonMatrix {
    /// `(a, b) → cell`, both directions stored so the view never has to
    /// remember which way round a pair was computed.
    cells: BTreeMap<(String, String), MatrixCell>,
    names: Vec<String>,
}

impl ComparisonMatrix {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one judged pair.
    pub fn insert(&mut self, a: &str, b: &str, comparison: &Comparison, differences: &[f64]) {
        for name in [a, b] {
            if !self.names.iter().any(|n| n == name) {
                self.names.push(name.to_string());
            }
        }
        let (wins, ties, losses) = comparison.record();
        let p = wilcoxon_signed_rank_p(differences);
        self.cells.insert(
            (a.to_string(), b.to_string()),
            MatrixCell {
                mean_difference: comparison.mean_difference(),
                wins,
                ties,
                losses,
                wilcoxon_p_descriptive: p,
                verdict: comparison.verdict(),
                achieved_k: comparison.achieved_k(),
            },
        );
        self.cells.insert(
            (b.to_string(), a.to_string()),
            MatrixCell {
                mean_difference: -comparison.mean_difference(),
                wins: losses,
                ties,
                losses: wins,
                wilcoxon_p_descriptive: p,
                verdict: comparison.verdict(),
                achieved_k: comparison.achieved_k(),
            },
        );
    }

    #[must_use]
    pub fn get(&self, a: &str, b: &str) -> Option<&MatrixCell> {
        self.cells.get(&(a.to_string(), b.to_string()))
    }

    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// Wilcoxon signed-rank two-sided p-value via the normal approximation with a
/// tie correction. Descriptive only — see [`MatrixCell::wilcoxon_p_descriptive`].
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn wilcoxon_signed_rank_p(differences: &[f64]) -> f64 {
    let nonzero: Vec<f64> = differences.iter().copied().filter(|d| *d != 0.0).collect();
    let n = nonzero.len();
    if n < 2 {
        return 1.0;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| {
        nonzero[i].abs().partial_cmp(&nonzero[j].abs()).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Average ranks within ties.
    let mut ranks = vec![0.0_f64; n];
    let mut tie_correction = 0.0_f64;
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && (nonzero[idx[j + 1]].abs() - nonzero[idx[i]].abs()).abs() < 1e-12 {
            j += 1;
        }
        let group = j - i + 1;
        let mean_rank = (i + j + 2) as f64 / 2.0;
        for &k in &idx[i..=j] {
            ranks[k] = mean_rank;
        }
        let g = group as f64;
        tie_correction += g * g * g - g;
        i = j + 1;
    }

    let w_plus: f64 = (0..n).filter(|&k| nonzero[k] > 0.0).map(|k| ranks[k]).sum();
    let nf = n as f64;
    let mean = nf * (nf + 1.0) / 4.0;
    let var = (nf * (nf + 1.0) * (2.0 * nf + 1.0) - tie_correction / 2.0) / 24.0;
    if var <= 0.0 {
        return 1.0;
    }
    // Continuity correction toward the mean.
    let diff = w_plus - mean;
    let z = (diff.abs() - 0.5).max(0.0) / var.sqrt() * diff.signum();
    2.0 * (1.0 - super::normal_cdf(z.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(delta: f64, k_max: usize) -> ComparisonPlan {
        scaled_plan(delta, k_max, 2.0)
    }

    fn scaled_plan(delta: f64, k_max: usize, scale: f64) -> ComparisonPlan {
        ComparisonPlan {
            profile_id: "strict_v1".into(),
            n_eff: 42.0,
            delta_practical: delta,
            scale: RopeScale::new(scale).unwrap(),
            trials_a: 120,
            trials_b: 90,
            k_max,
            flags: ComparisonFlags::default(),
            alpha: DEFAULT_ALPHA,
        }
    }

    fn measurements(values: &[f64]) -> Vec<Measurement> {
        replicate_grid(values.len(), 7)
            .into_iter()
            .zip(values)
            .map(|(replicate, &value)| Measurement { replicate, value })
            .collect()
    }

    #[test]
    fn the_replicate_grid_is_paired_and_reproducible() {
        let g1 = replicate_grid(TARGET_REPLICATES, 7);
        let g2 = replicate_grid(TARGET_REPLICATES, 7);
        assert_eq!(g1, g2, "the same comparison must draw the same nuisance grid");
        assert_eq!(g1.len(), TARGET_REPLICATES);
        // All three cost perturbations are exercised, not just the modelled one.
        let costs: std::collections::BTreeSet<i32> = g1.iter().map(|r| r.cost_bp).collect();
        assert_eq!(costs.len(), 3);
        assert_ne!(replicate_grid(TARGET_REPLICATES, 8), g1, "the seed moves the grid");
    }

    #[test]
    fn an_unpaired_comparison_is_refused() {
        let p = plan(0.1, 29);
        let a = measurements(&[1.0; 8]);
        assert!(matches!(
            p.judge(&a, &measurements(&[0.5; 7])),
            Err(CompareError::NotPaired { a: 8, b: 7 })
        ));

        // Same length, different replicates: also refused. Zipping these would
        // compare A on one window against B on another and call the difference
        // an improvement.
        let mut b = measurements(&[0.5; 8]);
        b[3].replicate.seed += 99;
        assert!(matches!(
            p.judge(&a, &b),
            Err(CompareError::MismatchedReplicate { index: 3, .. })
        ));
    }

    #[test]
    fn too_few_replicates_is_not_a_verdict() {
        let p = plan(0.1, 29);
        let a = measurements(&[1.0; 4]);
        let b = measurements(&[0.0; 4]);
        assert!(matches!(p.judge(&a, &b), Err(CompareError::TooFewReplicates { have: 4 })));
    }

    /// A difference larger than the declared scale is an error, not a clipped
    /// observation: clipping would answer a narrower question than the one asked.
    #[test]
    fn a_difference_outside_the_declared_scale_is_refused() {
        let p = plan(0.1, 29);
        let mut values = vec![0.1_f64; 10];
        values[4] = 5.0;
        let a = measurements(&values);
        let b = measurements(&[0.0; 10]);
        assert!(matches!(p.judge(&a, &b), Err(CompareError::OutOfScale { index: 4, .. })));
    }

    /// A large, consistent improvement separates above δ and promotes.
    #[test]
    fn a_real_improvement_promotes() {
        let p = scaled_plan(0.05, 29, 0.8);
        let a = measurements(&[
            0.55, 0.62, 0.58, 0.61, 0.57, 0.63, 0.59, 0.60, 0.56, 0.64, 0.58, 0.62, 0.59, 0.61,
            0.57, 0.60, 0.63, 0.58, 0.62, 0.59, 0.61, 0.57, 0.60, 0.62, 0.58, 0.61, 0.59, 0.63,
            0.60,
        ]);
        let b = measurements(&[0.0_f64; 29]);
        let c = p.judge(&a, &b).unwrap();
        assert_eq!(c.verdict(), Verdict::Promote, "{}", c.headline());
        assert!(c.win_rate_rule_holds());
        assert!(c.interval().lower > 0.05);
        assert_eq!(c.achieved_k(), 29);
        assert_eq!(c.record(), (29, 0, 0));
    }

    /// An improvement that is real but smaller than the declared effect size is
    /// rejected. This is the case pre-registration exists for: without δ fixed
    /// in advance, a consistent +0.01 is exactly the result somebody talks
    /// themselves into.
    #[test]
    fn a_real_but_immaterial_improvement_is_rejected() {
        let p = scaled_plan(0.25, 29, 0.3);
        let a = measurements(&[
            0.01, 0.012, 0.009, 0.011, 0.010, 0.013, 0.008, 0.011, 0.010, 0.012, 0.009, 0.010,
            0.011, 0.012, 0.010, 0.009, 0.011, 0.013, 0.010, 0.011, 0.009, 0.012, 0.010, 0.011,
            0.010, 0.012, 0.009, 0.011, 0.010,
        ]);
        let b = measurements(&[0.0_f64; 29]);
        let c = p.judge(&a, &b).unwrap();
        assert_eq!(c.verdict(), Verdict::Reject, "{}", c.headline());
        // It still won nearly every replicate — which is exactly why the win
        // rate alone is not allowed to promote anything.
        assert!(c.p_a_better() > 0.9);
    }

    /// Noise does not promote, however it is looked at.
    #[test]
    fn noise_never_promotes() {
        let p = scaled_plan(0.05, 12, 0.3);
        let a = measurements(&[0.10, -0.08, 0.05, -0.11, 0.09, -0.04, 0.02, -0.07, 0.06, -0.05, 0.03, -0.09]);
        let b = measurements(&[0.0_f64; 12]);
        let c = p.judge(&a, &b).unwrap();
        assert_ne!(c.verdict(), Verdict::Promote, "{}", c.headline());
    }

    /// The confidence sequence must hold at every look, not only the last. This
    /// is the property that makes "sample until it separates" legitimate.
    #[test]
    fn the_confidence_sequence_covers_the_mean_at_every_look() {
        let unit: Vec<f64> = (0..200).map(|i| if i % 3 == 0 { 0.8 } else { 0.4 }).collect();
        let truth = unit.iter().sum::<f64>() / unit.len() as f64;
        for t in 5..=unit.len() {
            let cs = confidence_sequence(&unit[..t], DEFAULT_ALPHA).unwrap();
            assert!(cs.lower <= cs.upper);
            assert!(
                cs.lower <= truth + 0.15 && cs.upper >= truth - 0.15,
                "look {t}: {cs:?} excluded {truth}"
            );
        }
    }

    #[test]
    fn the_confidence_sequence_narrows_with_evidence() {
        let unit: Vec<f64> = vec![0.7; 400];
        let early = confidence_sequence(&unit[..10], DEFAULT_ALPHA).unwrap();
        let late = confidence_sequence(&unit, DEFAULT_ALPHA).unwrap();
        assert!(
            (late.upper - late.lower) < (early.upper - early.lower),
            "more looks must buy a tighter interval"
        );
    }

    /// A comparison cannot be built without its context, and it cannot be
    /// reduced to a number: the headline always carries the counts.
    #[test]
    fn a_comparison_is_never_a_bare_number() {
        let mut p = scaled_plan(0.05, 29, 1.0);
        p.flags.overlapping_labels_unweighted = true;
        let a = measurements(&[0.5; 10]);
        let b = measurements(&[0.0; 10]);
        let c = p.judge(&a, &b).unwrap();
        let head = c.headline();
        assert!(head.contains("N_eff=42.0"), "{head}");
        assert!(head.contains("k=10"), "{head}");
        assert!(head.contains("strict_v1"), "{head}");
        assert!(head.contains("overlapping_labels_unweighted"), "{head}");
        assert_eq!(c.trial_counts(), (120, 90));
    }

    #[test]
    fn holm_controls_the_family_and_is_monotone() {
        let p = [0.001, 0.01, 0.04, 0.9];
        let adj = holm_adjusted(&p);
        assert!((adj[0] - 0.004).abs() < 1e-12);
        assert!((adj[1] - 0.03).abs() < 1e-12);
        assert!((adj[2] - 0.08).abs() < 1e-12);
        // Monotone down the ordering.
        assert!(adj[0] <= adj[1] && adj[1] <= adj[2] && adj[2] <= adj[3]);
        // Never exceeds 1.
        assert!(adj.iter().all(|a| *a <= 1.0));
        // Holm is at least as strict as BH on the same family — promotion pays
        // for the family-wise guarantee, screening does not.
        let bh = screening_adjusted(&p);
        assert!(adj.iter().zip(bh.iter()).all(|(h, b)| h >= b), "{adj:?} vs {bh:?}");
        assert!(adj[1] > bh[1], "the two must actually differ: {adj:?} vs {bh:?}");
    }

    /// The declared scale is not a free parameter. Shrinking it to make the
    /// interval narrow makes real observations fall outside it, and widening it
    /// to be safe makes separation harder. Both directions are demonstrated
    /// here on the same data.
    #[test]
    fn the_scale_cannot_be_tuned_for_a_friendlier_answer() {
        let values = [
            0.30, 0.34, 0.28, 0.33, 0.31, 0.29, 0.32, 0.30, 0.33, 0.31, 0.30, 0.34, 0.29, 0.32,
            0.31, 0.30, 0.33, 0.28, 0.32, 0.31, 0.29, 0.33, 0.30, 0.32, 0.31, 0.29, 0.34, 0.30,
            0.32,
        ];
        let a = measurements(&values);
        let b = measurements(&[0.0_f64; 29]);

        // Generous: the interval is too wide to separate.
        let wide = scaled_plan(0.05, 29, 3.0).judge(&a, &b).unwrap();
        assert_ne!(wide.verdict(), Verdict::Promote, "{}", wide.headline());

        // Honest: separation is possible.
        let fair = scaled_plan(0.05, 29, 0.4).judge(&a, &b).unwrap();
        assert_eq!(fair.verdict(), Verdict::Promote, "{}", fair.headline());

        // Flattering: the observations no longer fit the claim that was made
        // about them, and are refused rather than clipped into it.
        assert!(matches!(
            scaled_plan(0.05, 29, 0.25).judge(&a, &b),
            Err(CompareError::OutOfScale { .. })
        ));
    }

    #[test]
    fn holm_on_an_empty_family_is_empty() {
        assert!(holm_adjusted(&[]).is_empty());
    }

    #[test]
    fn the_matrix_stores_both_directions_consistently() {
        let judged = scaled_plan(0.05, 29, 1.0)
            .judge(&measurements(&[0.5; 10]), &measurements(&[0.0; 10]))
            .unwrap();
        let differences: Vec<f64> = vec![0.5; 10];

        let mut matrix = ComparisonMatrix::new();
        matrix.insert("alpha", "beta", &judged, &differences);
        let ab = matrix.get("alpha", "beta").unwrap();
        let ba = matrix.get("beta", "alpha").unwrap();
        assert!((ab.mean_difference + ba.mean_difference).abs() < 1e-12);
        assert_eq!((ab.wins, ab.losses), (ba.losses, ba.wins));
        assert_eq!(matrix.names(), &["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn the_wilcoxon_p_is_descriptive_and_sane() {
        // A consistent shift is far from the null.
        let shifted: Vec<f64> = (0..20).map(|i| 0.5 + f64::from(i) * 0.01).collect();
        assert!(wilcoxon_signed_rank_p(&shifted) < 0.01);
        // Symmetric noise is not.
        let noise: Vec<f64> = (0..20).map(|i| if i % 2 == 0 { 0.3 } else { -0.3 }).collect();
        assert!(wilcoxon_signed_rank_p(&noise) > 0.5);
        // Degenerate input does not produce a small p-value out of nowhere.
        assert!((wilcoxon_signed_rank_p(&[0.0; 10]) - 1.0).abs() < 1e-12);
        assert!((wilcoxon_signed_rank_p(&[]) - 1.0).abs() < 1e-12);
    }
}
