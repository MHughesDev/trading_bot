//! The scorecard and the non-inferiority gate (AGENT-009 §4, §4.1, P2, P4;
//! Set S tasks S-4.11, S-4.17).
//!
//! # The gate obeys the statistics it demands of strategies
//!
//! It would be indefensible for the spec that requires a deflated Sharpe ratio
//! of a strategy to gate *itself* on a bare two-point difference in a pass rate.
//! So the arithmetic splits on whether the measurement has sampling error at all:
//!
//! * **Deterministic replay suites are exact.** A case passes or it does not.
//!   Any newly failing case blocks the merge, full stop — no threshold, no
//!   interval, because there is nothing to estimate.
//! * **Stochastic live suites use an interval.** Regression means the upper bound
//!   of a one-sided 95% Wilson interval on the *difference* is below zero, with
//!   a minimum of n = 30 per suite per profile. Below that the suite reports
//!   [`SuiteVerdict::Underpowered`] and can neither block nor clear.
//! * **Cost is exact** — tokens and dollars are counted, not sampled — and needs
//!   no interval.
//!
//! Set L measured what the tight margin costs: with perfect agreement between
//! arms, deciding a two-point margin needs ~189 paired trials per arm at $6–8
//! each. `evals/gates.yaml` is therefore set to 5 points with `spec_margin: 0.02`
//! recording the target. **A gate that always reports `UNDERPOWERED` is a gate
//! everyone learns to ignore**, which is worse than a wider margin honestly
//! declared.
//!
//! # Per tier, never on average
//!
//! P4. An average is how a local tier quietly dies: a frontier gain of six points
//! and a 7B loss of four averages to an improvement, ships, and nobody finds out
//! until the local tier is unusable. [`gate`] evaluates **every profile
//! separately** and a single regressed tier blocks, whatever the others did.
//!
//! # Two ways around the gate, both closed
//!
//! 1. **Add a profile.** A profile with no prior scorecard cannot be gated
//!    against itself, so its first run is labelled [`SuiteVerdict::Baseline`] and
//!    the profile reports `unmeasured` until two scorecards exist. Otherwise
//!    introducing a profile is the obvious way to pass.
//! 2. **Let a suite go flaky.** A suite whose flakiness exceeds 0.15 is reported
//!    [`SuiteVerdict::Unstable`] and **excluded from gating** — a gate that fires
//!    at random spends the team's trust, and the response to a random gate is to
//!    stop believing all of them.
//!
//! # Benjamini-Hochberg, because this runs on every PR
//!
//! The gate is a repeated test across (profile × suite) and across PRs. Without
//! a correction, a platform with six profiles and eight suites will block an
//! honest PR roughly every other week on noise alone. [`benjamini_hochberg`]
//! controls the false-discovery rate within a PR, and [`Scorecard::evaluations`]
//! records the count per harness version so the platform can see its own
//! multiple-testing exposure — the discipline the trial ledger applies to
//! research, applied to the harness that judges it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── Failure attribution (P2) ────────────────────────────────────────────────

/// Which rung a failure belongs to. "The agent failed" is not a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    /// The wrong namespace, or the right tool never reached.
    Routing,
    /// The tool was routed and lost its slot in the exposure budget.
    Exposure,
    /// The reply did not parse, or violated the schema it was sent.
    Decode,
    /// The validation ladder rejected a well-formed call.
    Validation,
    /// The tool itself failed or returned something unusable.
    Tool,
    /// The run had the knowledge and did not use it.
    Memory,
    /// Everything above worked and the model chose badly. The only rung that is
    /// about the model.
    ModelChoice,
}

impl Rung {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Rung::Routing => "routing",
            Rung::Exposure => "exposure",
            Rung::Decode => "decode",
            Rung::Validation => "validation",
            Rung::Tool => "tool",
            Rung::Memory => "memory",
            Rung::ModelChoice => "model_choice",
        }
    }

    #[must_use]
    pub fn all() -> &'static [Rung] {
        &[
            Rung::Routing,
            Rung::Exposure,
            Rung::Decode,
            Rung::Validation,
            Rung::Tool,
            Rung::Memory,
            Rung::ModelChoice,
        ]
    }
}

/// Failures by rung, for one suite.
pub type Attribution = BTreeMap<String, usize>;

// ── Suites ──────────────────────────────────────────────────────────────────

/// Whether a suite's number has sampling error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteKind {
    /// A tape through the loop. Exact.
    Replay,
    /// A model in the loop. Sampled.
    Live,
}

/// AGENT-009 §4.1's flakiness ceiling. Above it a suite cannot gate.
pub const FLAKINESS_CEILING: f64 = 0.15;

/// §4's minimum n for a live suite to decide anything.
pub const MIN_LIVE_N: usize = 30;

/// One suite's result for one profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuiteScore {
    pub suite: String,
    pub kind: SuiteKind,
    pub passed: usize,
    pub n: usize,
    /// Spread across the k = 3 repeats (§4.1). Zero for a replay suite, which
    /// has no repeats to spread across — and a replay suite with non-zero
    /// flakiness is the harness having become non-deterministic, which is itself
    /// the finding.
    #[serde(default)]
    pub flakiness: f64,
    #[serde(default)]
    pub attribution: Attribution,
    /// Ids of the cases that failed. A pass rate without these is a number
    /// nobody can act on.
    #[serde(default)]
    pub failed_cases: Vec<String>,
    /// Whether this score covers the sealed set (§4.2). Sealed scores never come
    /// from a PR run.
    #[serde(default)]
    pub sealed: bool,
}

impl SuiteScore {
    #[must_use]
    pub fn pass_rate(&self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        self.passed as f64 / self.n as f64
    }

    /// Whether this suite is allowed to decide anything.
    #[must_use]
    pub fn can_gate(&self) -> bool {
        if self.n == 0 {
            return false;
        }
        match self.kind {
            SuiteKind::Replay => self.flakiness <= f64::EPSILON,
            SuiteKind::Live => self.flakiness <= FLAKINESS_CEILING && self.n >= MIN_LIVE_N,
        }
    }
}

/// One harness version's measurement of one profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scorecard {
    pub harness_version: String,
    pub profile_id: String,
    pub tier: String,
    /// The box it was earned on. A faster box is not a better harness.
    #[serde(default)]
    pub hardware: String,
    pub suites: Vec<SuiteScore>,
    /// Tokens and USD. Exact, so no interval.
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub cost_tokens: u64,
    #[serde(default)]
    pub median_steps: f64,
    /// AGENT-005 §10.2. `None` until a live session has run — which needs a
    /// credential the platform does not have (S-4.14, roadmap gate H-2).
    #[serde(default)]
    pub cache_hit_rate: Option<f64>,
    /// AGENT-007 §4's `memory_retrievals.cited` share.
    #[serde(default)]
    pub memory_recall: Option<f64>,
    #[serde(default)]
    pub hitl_precision: Option<f64>,
    /// How many times the gate has been evaluated for this harness version. The
    /// platform's own multiple-testing exposure, recorded rather than ignored.
    #[serde(default)]
    pub evaluations: u32,
}

impl Scorecard {
    #[must_use]
    pub fn suite(&self, name: &str) -> Option<&SuiteScore> {
        self.suites.iter().find(|s| s.suite == name)
    }

    /// Whether any claim on this profile is licensed by measurement (P3).
    ///
    /// A profile with no gating suite reports `unmeasured` in the UI, and
    /// S-4.13's acceptance is that an uncalibrated profile says so rather than
    /// carrying prose.
    #[must_use]
    pub fn is_measured(&self) -> bool {
        self.suites.iter().any(SuiteScore::can_gate)
    }
}

// ── Verdicts ────────────────────────────────────────────────────────────────

/// What the gate concluded about one (profile × suite).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum SuiteVerdict {
    /// A newly failing deterministic case, or a live suite whose interval
    /// clears zero downward. Blocks.
    Regressed {
        detail: String,
        /// Empty for a replay suite: there is nothing to estimate.
        interval: Option<(f64, f64)>,
    },
    Improved { delta: f64 },
    Unchanged,
    /// Not enough n to decide its own margin (§4). Reported, does not gate.
    Underpowered { n: usize, needed: usize },
    /// Flakiness above the ceiling (§4.1). Excluded from gating.
    Unstable { flakiness: f64 },
    /// First measurement of this profile. Establishes a baseline, gates nothing.
    Baseline,
}

impl SuiteVerdict {
    #[must_use]
    pub fn blocks(&self) -> bool {
        matches!(self, SuiteVerdict::Regressed { .. })
    }

    #[must_use]
    pub fn is_improvement(&self) -> bool {
        matches!(self, SuiteVerdict::Improved { .. })
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            SuiteVerdict::Regressed { .. } => "REGRESSED",
            SuiteVerdict::Improved { .. } => "IMPROVED",
            SuiteVerdict::Unchanged => "UNCHANGED",
            SuiteVerdict::Underpowered { .. } => "UNDERPOWERED",
            SuiteVerdict::Unstable { .. } => "UNSTABLE",
            SuiteVerdict::Baseline => "BASELINE",
        }
    }
}

/// The whole gate's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateVerdict {
    /// `(profile_id, suite)` → verdict. Per tier, never averaged.
    pub per_suite: BTreeMap<String, SuiteVerdict>,
    pub blocked: bool,
    /// Why, in one line each. What a reviewer reads.
    pub reasons: Vec<String>,
    /// Profiles that report `unmeasured`: no scorecard, or no suite that can
    /// gate. Named rather than counted, because "three tiers unmeasured" is not
    /// something anyone can act on.
    pub unmeasured: Vec<String>,
    /// How many (profile × suite) comparisons this gate made. The input to the
    /// Benjamini-Hochberg correction and to the exposure record.
    pub comparisons: usize,
    /// True when nothing regressed and nothing improved — §4 says such a PR is
    /// labelled `neutral` with a reason rather than passed silently.
    pub neutral: bool,
}

impl GateVerdict {
    /// One line, for a CI summary.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.blocked {
            format!("BLOCKED: {}", self.reasons.join("; "))
        } else if self.neutral {
            "NEUTRAL: nothing regressed and nothing improved — label the PR `neutral` with a reason"
                .to_string()
        } else {
            format!(
                "PASS: {} comparisons, {} unmeasured profile(s)",
                self.comparisons,
                self.unmeasured.len()
            )
        }
    }
}

// ── The statistics ──────────────────────────────────────────────────────────

/// One-sided 95% Wilson score interval for a binomial proportion.
///
/// Wilson rather than the normal approximation because the pass rates that
/// matter here are near 1.0, where the normal interval runs past 100% and
/// silently reports impossible bounds.
#[must_use]
pub fn wilson(passed: usize, n: usize, z: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let n_f = n as f64;
    let p = passed as f64 / n_f;
    let denom = 1.0 + z * z / n_f;
    let centre = (p + z * z / (2.0 * n_f)) / denom;
    let margin = z * ((p * (1.0 - p) / n_f) + (z * z / (4.0 * n_f * n_f))).sqrt() / denom;
    ((centre - margin).max(0.0), (centre + margin).min(1.0))
}

/// The one-sided 95% z.
pub const Z_95: f64 = 1.645;

/// An interval on the *difference* in pass rate, candidate minus baseline.
///
/// Newcombe's method: build each arm's Wilson interval and combine. It is the
/// right tool for two independent proportions, and — unlike differencing two
/// point estimates — it does not report a difference more precisely than either
/// number it was built from.
#[must_use]
pub fn difference_interval(
    cand_passed: usize,
    cand_n: usize,
    base_passed: usize,
    base_n: usize,
) -> (f64, f64) {
    let (l1, u1) = wilson(cand_passed, cand_n, Z_95);
    let (l2, u2) = wilson(base_passed, base_n, Z_95);
    let p1 = cand_passed as f64 / cand_n.max(1) as f64;
    let p2 = base_passed as f64 / base_n.max(1) as f64;
    let d = p1 - p2;
    let lower = d - ((p1 - l1).powi(2) + (u2 - p2).powi(2)).sqrt();
    let upper = d + ((u1 - p1).powi(2) + (p2 - l2).powi(2)).sqrt();
    (lower.max(-1.0), upper.min(1.0))
}

/// Benjamini-Hochberg: which of these p-values survive at `q`.
///
/// Returns the indices that are still significant after the correction. The gate
/// runs on every PR across (profile × suite), so it is a repeated test, and a
/// platform with six profiles and eight suites blocks an honest PR on noise
/// roughly every other week without this.
#[must_use]
pub fn benjamini_hochberg(p_values: &[f64], q: f64) -> Vec<usize> {
    let m = p_values.len();
    if m == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|a, b| {
        p_values[*a]
            .partial_cmp(&p_values[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut k = 0usize;
    for (rank, idx) in order.iter().enumerate() {
        let threshold = q * (rank + 1) as f64 / m as f64;
        if p_values[*idx] <= threshold {
            k = rank + 1;
        }
    }
    order.into_iter().take(k).collect()
}

/// A one-sided p-value that the candidate is no worse, from the difference
/// interval. Approximate and monotone, which is all BH needs of it.
fn p_value_of(lower: f64, upper: f64) -> f64 {
    let d = (lower + upper) / 2.0;
    let half = (upper - lower) / 2.0;
    if half <= f64::EPSILON {
        return if d < 0.0 { 0.0 } else { 1.0 };
    }
    // z at which zero sits, converted through a logistic approximation to the
    // normal tail. Exact enough to rank; never used as a reported number.
    //
    // The sign matters and is easy to get backwards: a strongly NEGATIVE
    // difference is strong evidence of a regression, so it must yield a SMALL
    // p-value. Inverted, the gate silently stops blocking anything — which it
    // did, and the `a_live_suite_that_clears_its_interval_downward_blocks` test
    // is what caught it.
    let z = d / (half / Z_95);
    1.0 / (1.0 + (-1.702 * z).exp())
}

/// AGENT-009 §4's false-discovery rate for the gate.
pub const GATE_Q: f64 = 0.10;

/// Evaluate the gate for a set of profiles (P4).
///
/// `baselines` and `candidates` are keyed by profile id. A profile in
/// `candidates` with no baseline establishes one and gates nothing.
#[must_use]
pub fn gate(baselines: &[Scorecard], candidates: &[Scorecard]) -> GateVerdict {
    let mut per_suite = BTreeMap::new();
    let mut reasons = Vec::new();
    let mut unmeasured = Vec::new();
    let mut blocked = false;
    let mut improved_any = false;
    let mut comparisons = 0usize;

    // Live-suite comparisons are collected first and corrected together: BH is a
    // statement about a family of tests, so applying it per profile would defeat
    // the point.
    let mut live: Vec<(String, f64, f64, f64, &SuiteScore)> = Vec::new();

    for cand in candidates {
        let base = baselines.iter().find(|b| b.profile_id == cand.profile_id);
        if !cand.is_measured() {
            unmeasured.push(cand.profile_id.clone());
        }
        let Some(base) = base else {
            // Closed door #1. A new profile cannot pass by having nothing to
            // regress against.
            for s in &cand.suites {
                per_suite.insert(
                    format!("{}/{}", cand.profile_id, s.suite),
                    SuiteVerdict::Baseline,
                );
            }
            if !unmeasured.contains(&cand.profile_id) {
                unmeasured.push(cand.profile_id.clone());
            }
            reasons.push(format!(
                "{} has no prior scorecard: this run is a baseline and gates nothing",
                cand.profile_id
            ));
            continue;
        };
        // Comparability (gates.yaml): a run compared against a baseline from a
        // different tier measures the tier.
        if base.tier != cand.tier {
            blocked = true;
            reasons.push(format!(
                "{}: baseline tier {} does not match candidate tier {} — refusing to compare",
                cand.profile_id, base.tier, cand.tier
            ));
            continue;
        }

        for s in &cand.suites {
            if s.sealed {
                // §4.2: the sealed set never runs on a PR, so it never enters
                // the PR gate. `promotion` reads it instead.
                continue;
            }
            let key = format!("{}/{}", cand.profile_id, s.suite);
            let Some(b) = base.suite(&s.suite) else {
                per_suite.insert(key, SuiteVerdict::Baseline);
                continue;
            };
            comparisons += 1;

            // Closed door #2.
            if s.flakiness > FLAKINESS_CEILING {
                per_suite.insert(
                    key,
                    SuiteVerdict::Unstable {
                        flakiness: s.flakiness,
                    },
                );
                reasons.push(format!(
                    "{}/{}: flakiness {:.2} exceeds {FLAKINESS_CEILING} — excluded from gating \
                     until fixed or made deterministic",
                    cand.profile_id, s.suite, s.flakiness
                ));
                continue;
            }

            match s.kind {
                SuiteKind::Replay => {
                    // Exact. A newly failing case blocks, full stop.
                    let newly: Vec<&String> = s
                        .failed_cases
                        .iter()
                        .filter(|c| !b.failed_cases.contains(*c))
                        .collect();
                    if !newly.is_empty() {
                        blocked = true;
                        per_suite.insert(
                            key,
                            SuiteVerdict::Regressed {
                                detail: format!("newly failing: {newly:?}"),
                                interval: None,
                            },
                        );
                        reasons.push(format!(
                            "{}/{} (replay, exact): {} newly failing case(s) {newly:?}",
                            cand.profile_id,
                            s.suite,
                            newly.len()
                        ));
                    } else if s.pass_rate() > b.pass_rate() {
                        improved_any = true;
                        per_suite.insert(
                            key,
                            SuiteVerdict::Improved {
                                delta: s.pass_rate() - b.pass_rate(),
                            },
                        );
                    } else {
                        per_suite.insert(key, SuiteVerdict::Unchanged);
                    }
                }
                SuiteKind::Live => {
                    if s.n < MIN_LIVE_N {
                        per_suite.insert(
                            key,
                            SuiteVerdict::Underpowered {
                                n: s.n,
                                needed: MIN_LIVE_N,
                            },
                        );
                        reasons.push(format!(
                            "{}/{}: n = {} below {MIN_LIVE_N} — reported, does not gate",
                            cand.profile_id, s.suite, s.n
                        ));
                        continue;
                    }
                    let (lo, hi) = difference_interval(s.passed, s.n, b.passed, b.n);
                    live.push((key, lo, hi, p_value_of(lo, hi), s));
                }
            }
        }
    }

    // Benjamini-Hochberg over the live family.
    let p_values: Vec<f64> = live.iter().map(|(_, _, _, p, _)| *p).collect();
    let significant = benjamini_hochberg(&p_values, GATE_Q);
    for (i, (key, lo, hi, _, s)) in live.iter().enumerate() {
        // Regression: the upper bound of the one-sided interval is below zero.
        if *hi < 0.0 && significant.contains(&i) {
            blocked = true;
            per_suite.insert(
                key.clone(),
                SuiteVerdict::Regressed {
                    detail: format!(
                        "live suite, 95% interval on the difference is [{lo:.3}, {hi:.3}], \
                         entirely below zero at n = {}",
                        s.n
                    ),
                    interval: Some((*lo, *hi)),
                },
            );
            reasons.push(format!("{key}: interval [{lo:.3}, {hi:.3}] is below zero"));
        } else if *lo > 0.0 {
            improved_any = true;
            per_suite.insert(
                key.clone(),
                SuiteVerdict::Improved {
                    delta: (lo + hi) / 2.0,
                },
            );
        } else {
            // A point estimate that moved without a significant interval is
            // reported and does not gate (§4).
            per_suite.insert(key.clone(), SuiteVerdict::Unchanged);
        }
    }

    unmeasured.sort();
    unmeasured.dedup();
    GateVerdict {
        neutral: !blocked && !improved_any && comparisons > 0,
        per_suite,
        blocked,
        reasons,
        unmeasured,
        comparisons,
    }
}

/// The nightly promotion check (§4.2): is this version fitted to its own evals?
///
/// A version whose **open**-set rate exceeds its **sealed**-set rate by more than
/// the flakiness band has been tuned against the cases it can see. It is not
/// promoted, whatever the open set says.
///
/// This is the identical argument the platform makes to a researcher who wants
/// one more look at a holdout, and a rule the platform will not apply to itself
/// is a rule it does not believe.
#[must_use]
pub fn fitted_to_its_evals(card: &Scorecard) -> Option<String> {
    for open in card.suites.iter().filter(|s| !s.sealed) {
        let Some(sealed) = card
            .suites
            .iter()
            .find(|s| s.sealed && s.suite == open.suite)
        else {
            continue;
        };
        let band = open.flakiness.max(sealed.flakiness).max(FLAKINESS_CEILING);
        let gap = open.pass_rate() - sealed.pass_rate();
        if gap > band {
            return Some(format!(
                "{}: open set {:.2}, sealed set {:.2} — a gap of {:.2} beyond the flakiness band \
                 of {:.2}. This version is fitted to the cases it can see.",
                open.suite,
                open.pass_rate(),
                sealed.pass_rate(),
                gap,
                band
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replay(suite: &str, passed: usize, n: usize, failed: &[&str]) -> SuiteScore {
        SuiteScore {
            suite: suite.into(),
            kind: SuiteKind::Replay,
            passed,
            n,
            flakiness: 0.0,
            attribution: Attribution::new(),
            failed_cases: failed.iter().map(|s| (*s).to_string()).collect(),
            sealed: false,
        }
    }

    fn live(suite: &str, passed: usize, n: usize) -> SuiteScore {
        SuiteScore {
            suite: suite.into(),
            kind: SuiteKind::Live,
            passed,
            n,
            flakiness: 0.05,
            attribution: Attribution::new(),
            failed_cases: Vec::new(),
            sealed: false,
        }
    }

    fn card(profile: &str, tier: &str, suites: Vec<SuiteScore>) -> Scorecard {
        Scorecard {
            harness_version: "S4".into(),
            profile_id: profile.into(),
            tier: tier.into(),
            hardware: "GTX 1080 Ti".into(),
            suites,
            cost_usd: 0.0,
            cost_tokens: 0,
            median_steps: 4.0,
            cache_hit_rate: None,
            memory_recall: None,
            hitl_precision: None,
            evaluations: 0,
        }
    }

    /// S-4.11's first acceptance, and the whole point of P4: a frontier gain
    /// does not buy a 7B loss.
    #[test]
    fn a_regression_on_the_7b_blocks_even_when_the_frontier_improves() {
        let base = vec![
            card("qwen2.5-7b", "degraded", vec![replay("routing", 10, 10, &[])]),
            card("claude-opus-5", "frontier", vec![replay("routing", 8, 10, &["r/x", "r/y"])]),
        ];
        let cand = vec![
            card(
                "qwen2.5-7b",
                "degraded",
                vec![replay("routing", 9, 10, &["routing/searched_tool_keeps_its_slot"])],
            ),
            card("claude-opus-5", "frontier", vec![replay("routing", 10, 10, &[])]),
        ];
        let v = gate(&base, &cand);
        assert!(v.blocked, "{:#?}", v);
        assert!(v.per_suite["qwen2.5-7b/routing"].blocks());
        assert!(v.per_suite["claude-opus-5/routing"].is_improvement());
        assert!(
            v.summary().contains("searched_tool_keeps_its_slot"),
            "{}",
            v.summary()
        );
    }

    /// §4: a replay suite has no sampling error, so there is no threshold. One
    /// newly failing case out of a hundred blocks.
    #[test]
    fn one_newly_failing_replay_case_blocks_with_no_threshold() {
        let base = vec![card("p", "t", vec![replay("routing", 100, 100, &[])])];
        let cand = vec![card("p", "t", vec![replay("routing", 99, 100, &["routing/c1"])])];
        assert!(gate(&base, &cand).blocked);
    }

    /// S-4.11's second acceptance: a 3-point move on a live suite with n = 12 is
    /// reported and does not gate.
    #[test]
    fn a_three_point_move_at_n_of_twelve_is_underpowered_and_does_not_gate() {
        let base = vec![card("p", "t", vec![live("memory", 10, 12)])];
        let cand = vec![card("p", "t", vec![live("memory", 9, 12)])];
        let v = gate(&base, &cand);
        assert!(!v.blocked, "{v:#?}");
        assert_eq!(
            v.per_suite["p/memory"],
            SuiteVerdict::Underpowered {
                n: 12,
                needed: MIN_LIVE_N
            }
        );
        assert!(v.reasons.iter().any(|r| r.contains("does not gate")));
    }

    /// A live regression large enough to clear its own interval does block.
    /// Without this the interval rule would be a way of never blocking.
    #[test]
    fn a_live_suite_that_clears_its_interval_downward_blocks() {
        let base = vec![card("p", "t", vec![live("memory", 95, 100)])];
        let cand = vec![card("p", "t", vec![live("memory", 45, 100)])];
        let v = gate(&base, &cand);
        assert!(v.blocked, "{v:#?}");
        match &v.per_suite["p/memory"] {
            SuiteVerdict::Regressed { interval, .. } => {
                let (_, hi) = interval.expect("a live regression reports its interval");
                assert!(hi < 0.0);
            }
            other => panic!("expected a regression with an interval, got {other:?}"),
        }
    }

    /// S-4.11's third acceptance, and closed door #1.
    #[test]
    fn a_brand_new_profile_cannot_pass_by_having_nothing_to_regress_against() {
        let base = vec![card("p", "t", vec![replay("routing", 10, 10, &[])])];
        let cand = vec![
            card("p", "t", vec![replay("routing", 10, 10, &[])]),
            card("brand-new", "t", vec![replay("routing", 10, 10, &[])]),
        ];
        let v = gate(&base, &cand);
        assert_eq!(v.per_suite["brand-new/routing"], SuiteVerdict::Baseline);
        assert!(
            v.unmeasured.contains(&"brand-new".to_string()),
            "{:?}",
            v.unmeasured
        );
        assert!(v.reasons.iter().any(|r| r.contains("gates nothing")));
    }

    /// Closed door #2. A gate that fires at random spends the team's trust.
    #[test]
    fn an_unstable_suite_can_neither_block_nor_clear() {
        let mut s = live("memory", 40, 100);
        s.flakiness = 0.31;
        let base = vec![card("p", "t", vec![live("memory", 95, 100)])];
        let cand = vec![card("p", "t", vec![s])];
        let v = gate(&base, &cand);
        assert!(!v.blocked, "an unstable suite must not block: {v:#?}");
        assert_eq!(
            v.per_suite["p/memory"],
            SuiteVerdict::Unstable { flakiness: 0.31 }
        );
        assert!(!v.per_suite["p/memory"].is_improvement());
    }

    /// A replay suite whose repeats disagree is the harness having become
    /// non-deterministic, which is itself the finding.
    #[test]
    fn a_replay_suite_with_any_flakiness_cannot_gate() {
        let mut s = replay("routing", 10, 10, &[]);
        s.flakiness = 0.02;
        assert!(!s.can_gate());
    }

    /// The comparability rule from gates.yaml: a local run measured against a
    /// frontier baseline fails every margin, and widening the margins to fix it
    /// would weaken the gate for frontier too.
    #[test]
    fn comparing_across_tiers_is_refused_rather_than_measured() {
        let base = vec![card("p", "frontier", vec![replay("routing", 10, 10, &[])])];
        let cand = vec![card("p", "degraded", vec![replay("routing", 10, 10, &[])])];
        let v = gate(&base, &cand);
        assert!(v.blocked);
        assert!(v.reasons[0].contains("refusing to compare"), "{:?}", v.reasons);
    }

    #[test]
    fn nothing_moving_is_reported_neutral_rather_than_passed_silently() {
        let base = vec![card("p", "t", vec![replay("routing", 10, 10, &[])])];
        let cand = vec![card("p", "t", vec![replay("routing", 10, 10, &[])])];
        let v = gate(&base, &cand);
        assert!(!v.blocked);
        assert!(v.neutral);
        assert!(v.summary().contains("NEUTRAL"), "{}", v.summary());
    }

    /// §4.2's promotion rule, and the argument the platform makes to a
    /// researcher who wants one more look.
    #[test]
    fn a_version_fitted_to_the_open_set_fails_promotion() {
        let mut open = replay("routing", 100, 100, &[]);
        open.flakiness = 0.0;
        let mut sealed = replay("routing", 60, 100, &[]);
        sealed.sealed = true;
        let c = card("p", "t", vec![open, sealed]);
        let why = fitted_to_its_evals(&c).expect("a 40-point gap is fitting");
        assert!(why.contains("fitted to the cases it can see"), "{why}");

        let mut ok_open = replay("routing", 95, 100, &[]);
        ok_open.flakiness = 0.05;
        let mut ok_sealed = replay("routing", 90, 100, &[]);
        ok_sealed.sealed = true;
        assert!(fitted_to_its_evals(&card("p", "t", vec![ok_open, ok_sealed])).is_none());
    }

    /// §4.2: the sealed set never runs on a PR, so it never reaches the PR gate.
    #[test]
    fn a_sealed_suite_never_enters_the_pr_gate() {
        let mut sealed = replay("routing", 1, 100, &["a", "b"]);
        sealed.sealed = true;
        let base = vec![card("p", "t", vec![replay("routing", 10, 10, &[])])];
        let cand = vec![card("p", "t", vec![replay("routing", 10, 10, &[]), sealed])];
        let v = gate(&base, &cand);
        assert!(!v.blocked, "the sealed set must not gate a PR: {v:#?}");
        assert_eq!(v.comparisons, 1);
    }

    #[test]
    fn wilson_stays_inside_zero_and_one_at_the_edges() {
        let (lo, hi) = wilson(30, 30, Z_95);
        assert!(lo > 0.8 && hi <= 1.0, "({lo}, {hi})");
        let (lo, hi) = wilson(0, 30, Z_95);
        assert!(lo >= 0.0 && hi < 0.2, "({lo}, {hi})");
    }

    #[test]
    fn benjamini_hochberg_admits_the_small_and_rejects_the_large() {
        let p = vec![0.001, 0.02, 0.6, 0.9];
        let sig = benjamini_hochberg(&p, 0.10);
        assert!(sig.contains(&0), "{sig:?}");
        assert!(!sig.contains(&3), "{sig:?}");
        assert!(benjamini_hochberg(&[], 0.1).is_empty());
    }

    /// The difference interval must not be narrower than the arms it came from.
    #[test]
    fn the_difference_interval_is_wider_than_either_arm() {
        let (lo, hi) = difference_interval(90, 100, 95, 100);
        let (al, ah) = wilson(90, 100, Z_95);
        assert!(hi - lo > (ah - al) * 0.9, "({lo},{hi}) vs ({al},{ah})");
        assert!(lo < 0.0 && hi > 0.0, "5 points at n=100 does not decide");
    }

    #[test]
    fn a_profile_with_no_gating_suite_reports_unmeasured() {
        let mut s = live("memory", 5, 5);
        s.flakiness = 0.0;
        let c = card("p", "t", vec![s]);
        assert!(!c.is_measured(), "n = 5 licenses no claim");
        let mut ok = replay("routing", 10, 10, &[]);
        ok.flakiness = 0.0;
        assert!(card("p", "t", vec![ok]).is_measured());
    }
}
