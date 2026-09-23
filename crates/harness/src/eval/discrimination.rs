//! The two error rates, the `unmeasured` answer, and the declared threshold
//! (AGENT-009 §3, §4, §8.8; DONE-4).
//!
//! # Why this is not one number
//!
//! The `judgement` suite asks a tier two questions that fail in opposite
//! directions, and an average over them is worse than useless:
//!
//! * given data with a planted edge, did it **find** the edge — the **miss
//!   rate**;
//! * given data with a planted null, did it correctly **say so** — the
//!   **false-positive rate**.
//!
//! A tier that calls every case an edge has a miss rate of zero. A tier that
//! calls every case a null has a false-positive rate of zero. Both are constant
//! answers with no judgement in them at all, and **both score about 50% on any
//! accuracy over a mixed corpus** — which is to say a single number can be
//! gamed by answering the same thing every time, and would be, because that is
//! the cheapest thing a model under a pass-rate gate can do.
//!
//! So [`Discrimination`] reports them as two numbers and there is **no method on
//! it that combines them**. Not a harmonic mean, not a balanced accuracy, not an
//! F-score. A caller that wants one number has to write the collapse itself, in
//! its own file, where a reviewer can see it.
//!
//! [`tests::a_constant_answer_fails_whichever_constant_it_is`] is the assertion
//! that this works: a tier answering `supported` to everything and a tier
//! answering `refuted` to everything both fail the threshold, each on the
//! opposite rate.
//!
//! # Why `unmeasured` is a variant and not a zero
//!
//! DONE-4's second clause — *every local tier reports a measured score or
//! `unmeasured`* — is the half that is easy to lose, because losing it looks
//! like working. A tier nobody ran, reported as `0.0`, renders in the governance
//! UI as a tier that was measured and did badly, and the response to a tier that
//! did badly is to fix the tier rather than to run the suite.
//!
//! [`TierScore`] therefore has two variants and **no numeric representation of
//! the unmeasured one**: there is no `Default`, no `From<f64>`, no `as_f64`, and
//! [`Measurement`] — the only thing that holds a rate — lives inside
//! [`TierScore::Measured`]. Rendering an unmeasured tier as a number is a
//! compile error rather than a rendering bug, which is the difference between a
//! constraint and a convention.
//!
//! # Why the threshold is declared
//!
//! A suite that derives its bar from the scores it is judging never fails
//! anybody: the bar moves down with the tier and the gate reports a pass at
//! every level of incompetence. [`Threshold`]'s fields are private and its only
//! constructor is a private `const fn` used by the declared constants in this
//! module, so a `Threshold` cannot be built from a measurement at all — again a
//! compile-time property rather than a rule in a comment.
//! [`tests::the_threshold_does_not_move_with_the_run_it_judges`] asserts the
//! consequence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::judgement::{Criterion, Observed};
use super::planted::{PlantKind, PlantedCase, PlantedCorpus, MIN_CASES_PER_TIER};
use crate::lifecycle::evaluate::Verdict;
use crate::profile::Tier;

// ── What a run called ───────────────────────────────────────────────────────

/// What the run said about whether there is an edge.
///
/// `Inconclusive` and "never reached a verdict" are the **same answer** for this
/// purpose and are kept apart from both of the others. A tier that answers
/// `inconclusive` to everything must not score a false-positive rate of zero and
/// look cautious: it never found a planted edge either, so its miss rate is
/// 1.0 and the threshold refuses it. That asymmetry is deliberate — declining to
/// call a null is free, declining to call an edge is a miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Called {
    Edge,
    NoEdge,
    NoCall,
}

impl Called {
    #[must_use]
    pub fn of(observed: &Observed) -> Self {
        match observed.verdict {
            Some(Verdict::Supported) => Called::Edge,
            Some(Verdict::Refuted) => Called::NoEdge,
            Some(Verdict::Inconclusive { .. }) | None => Called::NoCall,
        }
    }
}

// ── The two rates, and the two that are not rates ───────────────────────────

/// The tallies behind the suite's numbers.
///
/// Public fields, because every rate below is `numerator / denominator` and a
/// reader who cannot see both has to trust the rate. Note what is **not** here:
/// any field or method that mixes the edge tallies with the null tallies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Discrimination {
    /// Cases with a planted edge that the tier was run on.
    pub edges_planted: usize,
    /// Planted edges reported as no edge.
    pub edges_called_null: usize,
    /// Planted edges the run never called either way. **Counted as a miss** —
    /// see [`Called`].
    pub edges_not_called: usize,

    /// Cases with a planted null that the tier was run on.
    pub nulls_planted: usize,
    /// Planted nulls reported as an edge. The outcome every other safeguard in
    /// this system exists to make harder.
    pub nulls_called_edge: usize,
    /// Planted nulls the run never called either way. Not a false positive — it
    /// claimed nothing — and not a correct rejection either, so it is carried
    /// separately rather than folded into the rate.
    pub nulls_not_called: usize,

    /// Cases with a planted leaking feature that the tier was run on.
    pub leaks_planted: usize,
    /// Planted leaks the run did not find.
    pub leaks_missed: usize,

    /// Dominance pairs the tier was run on.
    pub pairs_planted: usize,
    /// Pairs where the planted winner was not ranked first.
    pub pairs_misranked: usize,
}

impl Discrimination {
    /// Planted edges the tier failed to find, over planted edges.
    ///
    /// `None` when no edge was planted, because a rate over an empty
    /// denominator is not zero — it is nothing, and `0.0` here would read as a
    /// tier that missed nothing.
    #[must_use]
    pub fn miss_rate(&self) -> Option<f64> {
        (self.edges_planted > 0).then(|| {
            (self.edges_called_null + self.edges_not_called) as f64 / self.edges_planted as f64
        })
    }

    /// Planted nulls the tier called an edge, over planted nulls.
    #[must_use]
    pub fn false_positive_rate(&self) -> Option<f64> {
        (self.nulls_planted > 0).then(|| self.nulls_called_edge as f64 / self.nulls_planted as f64)
    }

    /// Planted leaks the tier did not find, over planted leaks.
    #[must_use]
    pub fn leak_miss_rate(&self) -> Option<f64> {
        (self.leaks_planted > 0).then(|| self.leaks_missed as f64 / self.leaks_planted as f64)
    }

    /// Dominance pairs ranked wrongly, over pairs.
    #[must_use]
    pub fn misranking_rate(&self) -> Option<f64> {
        (self.pairs_planted > 0).then(|| self.pairs_misranked as f64 / self.pairs_planted as f64)
    }

    /// How often the tier declined to answer at all, over cases where an answer
    /// was available.
    ///
    /// Reported because a tier that never calls anything can otherwise reach a
    /// false-positive rate of zero and look careful; here it is visible as the
    /// abstention it is.
    #[must_use]
    pub fn no_call_rate(&self) -> Option<f64> {
        let n = self.edges_planted + self.nulls_planted;
        (n > 0).then(|| (self.edges_not_called + self.nulls_not_called) as f64 / n as f64)
    }

    /// Tally one run of one case.
    fn observe(&mut self, case: &PlantedCase, observed: &Observed) {
        match case.key_kind() {
            PlantKind::Edge => {
                self.edges_planted += 1;
                match Called::of(observed) {
                    Called::Edge => {}
                    Called::NoEdge => self.edges_called_null += 1,
                    Called::NoCall => self.edges_not_called += 1,
                }
            }
            PlantKind::Null => {
                self.nulls_planted += 1;
                match Called::of(observed) {
                    Called::Edge => self.nulls_called_edge += 1,
                    Called::NoEdge => {}
                    Called::NoCall => self.nulls_not_called += 1,
                }
            }
            PlantKind::Leak => {
                self.leaks_planted += 1;
                if !observed.found_the_leak {
                    self.leaks_missed += 1;
                }
            }
            PlantKind::Dominance => {
                self.pairs_planted += 1;
                if observed.ranked_first.as_deref() != case.planted_winner() {
                    self.pairs_misranked += 1;
                }
            }
        }
    }
}

// ── The declared threshold ──────────────────────────────────────────────────

/// The bar a tier is judged against.
///
/// **Constructible only inside this module.** The fields are private and the
/// only constructor is a private `const fn`, so there is no way to build a
/// `Threshold` out of a set of scores — which is the one thing that would turn
/// this suite into a gate that never fails anybody. A new tier's bar is a source
/// edit to this file, visible in a diff, with the clause it comes from written
/// beside it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Threshold {
    max_miss_rate: f64,
    max_false_positive_rate: f64,
    max_leak_miss_rate: f64,
    max_misranking_rate: f64,
    min_edges: usize,
    min_nulls: usize,
    min_cases: usize,
    /// Where the numbers come from. Prose, and it is allowed to be prose because
    /// nothing reads it — it is for the reviewer who wants to check a bar
    /// against the clause that set it.
    declared_in: &'static str,
}

impl Threshold {
    const fn declare(
        max_miss_rate: f64,
        max_false_positive_rate: f64,
        max_leak_miss_rate: f64,
        max_misranking_rate: f64,
        declared_in: &'static str,
    ) -> Self {
        Threshold {
            max_miss_rate,
            max_false_positive_rate,
            max_leak_miss_rate,
            max_misranking_rate,
            // Below ten of either kind a rate moves ten points per case and the
            // interval is wider than every bar above it.
            min_edges: 10,
            min_nulls: 10,
            min_cases: MIN_CASES_PER_TIER,
            declared_in,
        }
    }

    #[must_use]
    pub fn max_miss_rate(&self) -> f64 {
        self.max_miss_rate
    }

    #[must_use]
    pub fn max_false_positive_rate(&self) -> f64 {
        self.max_false_positive_rate
    }

    #[must_use]
    pub fn declared_in(&self) -> &'static str {
        self.declared_in
    }

    /// The bar for a tier.
    ///
    /// The frontier's bar is the strict one because DONE-4 is written about the
    /// frontier and because AGENT-014 §8 reads this row before a profile may run
    /// continuous research unattended. The local tiers are held to a looser bar
    /// on the false-positive side and a much looser one on misses: a 7B that
    /// finds only half the planted edges is a slow researcher, and a 7B that
    /// calls a quarter of the planted nulls real is a dangerous one. P4's
    /// argument — no tier is scored on another tier's number — with the
    /// asymmetry that matters written into the constant.
    #[must_use]
    pub fn for_tier(tier: Tier) -> Self {
        match tier {
            Tier::Frontier => FRONTIER,
            Tier::LocalHigh | Tier::LocalMid => LOCAL,
            Tier::LocalSmall | Tier::Degraded => SMALL,
        }
    }
}

/// DONE-4's bar. A frontier profile that misses a fifth of the planted edges or
/// calls a tenth of the planted nulls real does not run research unattended.
pub const FRONTIER: Threshold = Threshold::declare(
    0.20,
    0.10,
    0.00,
    0.20,
    "AGENT-009 §3 (`judgement`), DONE-4; AGENT-014 §8 reads this row",
);

/// `local_mid` and `local_high`.
pub const LOCAL: Threshold = Threshold::declare(
    0.40,
    0.20,
    0.34,
    0.40,
    "AGENT-009 §3, P4 — a local tier is scored against its own bar, never the frontier's",
);

/// `local_small` and `degraded`. Slow is tolerated; confidently wrong is not,
/// and the false-positive bar barely moves.
pub const SMALL: Threshold = Threshold::declare(
    0.60,
    0.25,
    0.50,
    0.50,
    "AGENT-009 §3, P4; D-18 — a degraded host runs what it can and refuses what it cannot",
);

// ── Measured, or not ────────────────────────────────────────────────────────

/// Why a tier has no score.
///
/// Every variant names something a reader can act on. "Unmeasured" on its own is
/// the same dead end as "the agent failed".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum NotMeasured {
    /// The suite has never been run on this tier. The honest state of every
    /// local tier today: running a case needs a recorded trajectory through the
    /// planted data, which needs the live-run credential (Set S S-4.14, roadmap
    /// gate H-2).
    NeverRun,
    /// Fewer cases than a rate can be estimated from.
    TooFewCases { ran: usize, needed: usize },
    /// One of the two error directions has no planted cases, so one of the two
    /// rates does not exist — and a `judgement` score is both rates or it is
    /// nothing.
    OnlyOneErrorDirection { edges: usize, nulls: usize },
}

impl NotMeasured {
    /// One line, for the governance page.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            NotMeasured::NeverRun => {
                "the `judgement` suite has not been run on this tier".to_string()
            }
            NotMeasured::TooFewCases { ran, needed } => {
                format!("{ran} case(s) run, {needed} needed before a rate means anything")
            }
            NotMeasured::OnlyOneErrorDirection { edges, nulls } => format!(
                "{edges} planted edge(s) and {nulls} planted null(s): a tier scored on one \
                 error direction can pass by answering the same thing every time"
            ),
        }
    }
}

/// A tier's measured `judgement` score.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Measurement {
    pub tier: String,
    /// The corpus this was measured on. A score against an unnamed corpus is not
    /// reproducible.
    pub corpus_seed: u64,
    pub discrimination: Discrimination,
    /// The share of run cases where **every applicable criterion** passed
    /// ([`super::judgement::Score::passed`]). Carried alongside the two rates,
    /// never instead of them.
    pub criteria_pass_rate: f64,
    /// Which criteria failed, and how often. `Rung`-style attribution for the
    /// six things §3 scores: "the tier failed" is not a finding.
    pub criteria_failures: BTreeMap<String, usize>,
    /// The bar, carried with the score so a reader never has to go and find it —
    /// and so a change to the bar is visible in the artifact's diff.
    pub threshold: Threshold,
}

impl Measurement {
    /// Whether the tier is at or above its declared bar.
    ///
    /// Every rate is checked separately and any one of them failing fails the
    /// tier. There is no total, and there is no rate that can be traded against
    /// another.
    #[must_use]
    pub fn meets_threshold(&self) -> bool {
        self.shortfalls().is_empty()
    }

    /// What is below the bar, named. Empty means it passed.
    #[must_use]
    pub fn shortfalls(&self) -> Vec<String> {
        let d = &self.discrimination;
        let t = &self.threshold;
        let mut out = Vec::new();
        if let Some(r) = d.miss_rate() {
            if r > t.max_miss_rate {
                out.push(format!(
                    "miss rate {r:.2} exceeds {:.2}: it did not find {} of {} planted edge(s)",
                    t.max_miss_rate,
                    d.edges_called_null + d.edges_not_called,
                    d.edges_planted
                ));
            }
        }
        if let Some(r) = d.false_positive_rate() {
            if r > t.max_false_positive_rate {
                out.push(format!(
                    "false-positive rate {r:.2} exceeds {:.2}: it reported {} of {} planted \
                     null(s) as an edge",
                    t.max_false_positive_rate, d.nulls_called_edge, d.nulls_planted
                ));
            }
        }
        if let Some(r) = d.leak_miss_rate() {
            if r > t.max_leak_miss_rate {
                out.push(format!(
                    "leak miss rate {r:.2} exceeds {:.2}: {} of {} planted leak(s) went \
                     unfound",
                    t.max_leak_miss_rate, d.leaks_missed, d.leaks_planted
                ));
            }
        }
        if let Some(r) = d.misranking_rate() {
            if r > t.max_misranking_rate {
                out.push(format!(
                    "misranking rate {r:.2} exceeds {:.2}: {} of {} planted winner(s) were not \
                     ranked first",
                    t.max_misranking_rate, d.pairs_misranked, d.pairs_planted
                ));
            }
        }
        out
    }
}

/// A tier's `judgement` result: measured, or explicitly not.
///
/// **There is no third state and no numeric form of the second.** An unmeasured
/// tier cannot be rendered as a rate, averaged into a summary or compared
/// against a threshold, because [`Measurement`] — the only type that holds a
/// number — is reachable only through [`TierScore::Measured`]. That is DONE-4's
/// second clause as a type rather than as a documented convention.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TierScore {
    Unmeasured(NotMeasured),
    Measured(Measurement),
}

impl TierScore {
    #[must_use]
    pub fn measurement(&self) -> Option<&Measurement> {
        match self {
            TierScore::Measured(m) => Some(m),
            TierScore::Unmeasured(_) => None,
        }
    }

    /// One line for the governance page. An unmeasured tier says so in words; it
    /// never renders as a number.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            TierScore::Unmeasured(why) => format!("unmeasured — {}", why.detail()),
            TierScore::Measured(m) => {
                let d = &m.discrimination;
                format!(
                    "miss {} / false-positive {} over {} edge(s) and {} null(s) — {}",
                    d.miss_rate().map_or("n/a".into(), |r| format!("{r:.2}")),
                    d.false_positive_rate()
                        .map_or("n/a".into(), |r| format!("{r:.2}")),
                    d.edges_planted,
                    d.nulls_planted,
                    if m.meets_threshold() {
                        "at threshold".to_string()
                    } else {
                        m.shortfalls().join("; ")
                    }
                )
            }
        }
    }
}

// ── Scoring a tier ──────────────────────────────────────────────────────────

/// Score one tier's runs over a planted corpus.
///
/// `runs` is keyed by case id. Cases the tier was not run on are simply absent
/// and reduce the denominators — which is why the minimum-case check below
/// exists: a tier run on four easy cases must report `unmeasured`, not a
/// flattering rate.
#[must_use]
pub fn score_tier(
    tier: Tier,
    corpus: &PlantedCorpus,
    runs: &BTreeMap<String, Observed>,
) -> TierScore {
    let threshold = Threshold::for_tier(tier);

    let mut discrimination = Discrimination::default();
    let mut criteria_failures: BTreeMap<String, usize> = BTreeMap::new();
    let mut passed_all = 0usize;
    let mut ran = 0usize;

    for case in corpus.cases() {
        let Some(observed) = runs.get(case.case_id()) else {
            continue;
        };
        ran += 1;
        discrimination.observe(case, observed);

        let score = case.score(observed);
        if score.passed() {
            passed_all += 1;
        }
        for c in Criterion::ALL {
            if let Some(j) = score.judgement(*c) {
                if !j.not_applicable && !j.passed {
                    *criteria_failures.entry(c.as_str().to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    if ran == 0 {
        return TierScore::Unmeasured(NotMeasured::NeverRun);
    }
    if ran < threshold.min_cases {
        return TierScore::Unmeasured(NotMeasured::TooFewCases {
            ran,
            needed: threshold.min_cases,
        });
    }
    if discrimination.edges_planted < threshold.min_edges
        || discrimination.nulls_planted < threshold.min_nulls
    {
        return TierScore::Unmeasured(NotMeasured::OnlyOneErrorDirection {
            edges: discrimination.edges_planted,
            nulls: discrimination.nulls_planted,
        });
    }

    TierScore::Measured(Measurement {
        tier: tier.as_str().to_string(),
        corpus_seed: corpus.seed(),
        discrimination,
        criteria_pass_rate: passed_all as f64 / ran as f64,
        criteria_failures,
        threshold,
    })
}

// ── DONE-4 ──────────────────────────────────────────────────────────────────

/// The tiers DONE-4 is written about.
///
/// Listed rather than derived from whatever profiles happen to be configured: a
/// tier that quietly stops appearing in the config would otherwise drop out of
/// the check that exists to notice tiers nobody measured.
pub const TIERS_IN_SCOPE: [Tier; 5] = [
    Tier::Degraded,
    Tier::LocalSmall,
    Tier::LocalMid,
    Tier::LocalHigh,
    Tier::Frontier,
];

/// Whether DONE-4 is met.
///
/// > The `judgement` suite scores at or above threshold on the frontier tier,
/// > and every local tier reports a measured score or `unmeasured`.
///
/// The second clause looks vacuous against [`TierScore`] — the type has no third
/// state — and it is not, because the way a tier fails it is by being **absent
/// from the report entirely**. A tier nobody ran and nobody listed is the one
/// that reads as fine. So this refuses a map that is missing a tier, by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Done4 {
    pub met: bool,
    /// Why not, in one line each.
    pub reasons: Vec<String>,
    /// Tiers reporting `unmeasured`, named. Not a failure of DONE-4 for a local
    /// tier — it is the second clause being satisfied honestly — and always a
    /// failure for the frontier.
    pub unmeasured: Vec<String>,
}

/// Evaluate DONE-4 over a per-tier report.
#[must_use]
pub fn done_4(scores: &BTreeMap<String, TierScore>) -> Done4 {
    let mut reasons = Vec::new();
    let mut unmeasured = Vec::new();

    for tier in TIERS_IN_SCOPE {
        let name = tier.as_str();
        let Some(score) = scores.get(name) else {
            reasons.push(format!(
                "{name} is absent from the report: a tier nobody listed is a tier nobody \
                 measured, and it reads as fine"
            ));
            continue;
        };
        match score {
            TierScore::Unmeasured(why) => {
                unmeasured.push(name.to_string());
                if tier == Tier::Frontier {
                    reasons.push(format!(
                        "the frontier tier is unmeasured ({}), and DONE-4's first clause is \
                         about the frontier",
                        why.detail()
                    ));
                }
            }
            TierScore::Measured(m) => {
                if tier == Tier::Frontier && !m.meets_threshold() {
                    for s in m.shortfalls() {
                        reasons.push(format!("frontier: {s}"));
                    }
                }
            }
        }
    }

    Done4 {
        met: reasons.is_empty(),
        reasons,
        unmeasured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::evaluate::{Comparator, Falsifier};
    use crate::lifecycle::r#yield::Yield;
    use crate::state::Workflow;

    fn corpus() -> PlantedCorpus {
        PlantedCorpus::standard(0x1234_5678)
    }

    fn base(verdict: Option<Verdict>) -> Observed {
        Observed {
            verdict,
            workflow: Workflow::Investigate,
            falsifier: Some(Falsifier {
                metric: "sharpe_after_costs".into(),
                comparator: Comparator::AtMost,
                threshold: 0.0,
            }),
            yielded: Yield {
                trials_spent: 2,
                findings_established: 1,
                ..Yield::default()
            },
            ..Observed::default()
        }
    }

    /// A run that answers every case correctly, given the key.
    fn omniscient(corpus: &PlantedCorpus) -> BTreeMap<String, Observed> {
        corpus
            .cases()
            .iter()
            .map(|case| {
                let mut o = match case.key_kind() {
                    PlantKind::Edge => base(Some(Verdict::Supported)),
                    PlantKind::Null | PlantKind::Leak => base(Some(Verdict::Refuted)),
                    PlantKind::Dominance => base(Some(Verdict::Supported)),
                };
                o.workflow = case.brief().question_shape;
                o.found_the_leak = case.key_kind() == PlantKind::Leak;
                if case.key_kind() == PlantKind::Dominance {
                    o.ranked_first = case.planted_winner().map(str::to_string);
                }
                (case.case_id().to_string(), o)
            })
            .collect()
    }

    /// A run that gives the same answer to everything.
    fn constant(corpus: &PlantedCorpus, verdict: Verdict) -> BTreeMap<String, Observed> {
        corpus
            .cases()
            .iter()
            .map(|case| {
                let mut o = base(Some(verdict));
                o.workflow = case.brief().question_shape;
                o.found_the_leak = false;
                (case.case_id().to_string(), o)
            })
            .collect()
    }

    /// **The point of the whole module.** A single accuracy over a mixed corpus
    /// can be gamed by a constant answer; two rates cannot, and each constant
    /// fails on the opposite one.
    #[test]
    fn a_constant_answer_fails_whichever_constant_it_is() {
        let c = corpus();

        let always_edge = score_tier(Tier::Frontier, &c, &constant(&c, Verdict::Supported));
        let m = always_edge
            .measurement()
            .expect("the whole corpus was run, so it is measured");
        assert_eq!(m.discrimination.miss_rate(), Some(0.0), "it missed no edge");
        assert_eq!(
            m.discrimination.false_positive_rate(),
            Some(1.0),
            "and called every planted null real"
        );
        assert!(!m.meets_threshold());
        assert!(
            m.shortfalls()
                .iter()
                .any(|s| s.contains("false-positive rate")),
            "{:?}",
            m.shortfalls()
        );

        let always_null = score_tier(Tier::Frontier, &c, &constant(&c, Verdict::Refuted));
        let m = always_null.measurement().expect("measured");
        assert_eq!(
            m.discrimination.false_positive_rate(),
            Some(0.0),
            "it raised no false alarm"
        );
        assert_eq!(m.discrimination.miss_rate(), Some(1.0), "and found nothing");
        assert!(!m.meets_threshold());
        assert!(
            m.shortfalls().iter().any(|s| s.contains("miss rate")),
            "{:?}",
            m.shortfalls()
        );
    }

    /// Abstention is not caution. A tier that never calls anything must not
    /// reach a false-positive rate of zero and look careful.
    #[test]
    fn answering_inconclusive_to_everything_is_scored_as_missing_every_edge() {
        let c = corpus();
        let runs: BTreeMap<String, Observed> = c
            .cases()
            .iter()
            .map(|case| (case.case_id().to_string(), base(None)))
            .collect();
        let m = score_tier(Tier::Frontier, &c, &runs)
            .measurement()
            .expect("measured")
            .clone();
        assert_eq!(m.discrimination.miss_rate(), Some(1.0));
        assert_eq!(m.discrimination.false_positive_rate(), Some(0.0));
        assert_eq!(m.discrimination.no_call_rate(), Some(1.0));
        assert!(!m.meets_threshold(), "abstaining is not passing");
    }

    /// And a tier that actually judges passes.
    #[test]
    fn a_tier_that_gets_it_right_is_at_threshold() {
        let c = corpus();
        let score = score_tier(Tier::Frontier, &c, &omniscient(&c));
        let m = score.measurement().expect("measured");
        assert_eq!(m.discrimination.miss_rate(), Some(0.0));
        assert_eq!(m.discrimination.false_positive_rate(), Some(0.0));
        assert_eq!(m.discrimination.leak_miss_rate(), Some(0.0));
        assert_eq!(m.discrimination.misranking_rate(), Some(0.0));
        assert!(m.meets_threshold(), "{:?}", m.shortfalls());
        assert!(
            m.criteria_pass_rate > 0.0,
            "a corpus scored with the key should not fail every criterion"
        );
    }

    /// A tier nobody ran reports `unmeasured` and **has no number at all** — not
    /// a zero, not a default. The type is the enforcement.
    #[test]
    fn a_tier_nobody_ran_has_no_number_to_report() {
        let c = corpus();
        let score = score_tier(Tier::LocalSmall, &c, &BTreeMap::new());
        assert_eq!(score, TierScore::Unmeasured(NotMeasured::NeverRun));
        assert!(score.measurement().is_none());
        assert!(score.summary().starts_with("unmeasured"));

        // The distinction is in the serialised form too, since that is what the
        // governance page renders.
        let json = serde_json::to_string(&score).unwrap();
        assert!(json.contains("unmeasured"), "{json}");
        assert!(
            !json.contains("0.0"),
            "an unmeasured tier rendered a rate: {json}"
        );
    }

    /// A tier run on a handful of cases reports `unmeasured` rather than a
    /// flattering rate over four easy ones.
    #[test]
    fn a_tier_run_on_too_few_cases_is_unmeasured_not_perfect() {
        let c = corpus();
        let full = omniscient(&c);
        let few: BTreeMap<String, Observed> = full.into_iter().take(4).collect();
        let score = score_tier(Tier::Frontier, &c, &few);
        assert!(
            matches!(
                score,
                TierScore::Unmeasured(NotMeasured::TooFewCases { ran: 4, .. })
            ),
            "{score:?}"
        );
    }

    /// A corpus with only one error direction cannot produce a `judgement`
    /// score, however well the tier did on the half that exists.
    #[test]
    fn one_error_direction_is_not_a_score() {
        // A corpus of 30 whose null cases are all withheld from the run.
        let c = corpus();
        let mut runs = omniscient(&c);
        for case in c.cases() {
            if case.key_kind() == PlantKind::Null {
                runs.remove(case.case_id());
            }
        }
        // Pad the denominator so it is the direction, not the count, that fails.
        let score = score_tier(Tier::Frontier, &c, &runs);
        assert!(
            matches!(
                score,
                TierScore::Unmeasured(
                    NotMeasured::OnlyOneErrorDirection { nulls: 0, .. }
                        | NotMeasured::TooFewCases { .. }
                )
            ),
            "{score:?}"
        );
    }

    /// The bar does not move with the run it judges.
    #[test]
    fn the_threshold_does_not_move_with_the_run_it_judges() {
        let c = corpus();
        let good = score_tier(Tier::Frontier, &c, &omniscient(&c));
        let bad = score_tier(Tier::Frontier, &c, &constant(&c, Verdict::Supported));
        assert_eq!(
            good.measurement().unwrap().threshold,
            bad.measurement().unwrap().threshold,
            "a threshold that differed between a good run and a bad one would be derived \
             from the run"
        );
        assert_eq!(good.measurement().unwrap().threshold, FRONTIER);
        assert!(!FRONTIER.declared_in().is_empty());
    }

    /// Each tier is judged against its own bar (P4), and the asymmetry is the
    /// one that matters: slow is tolerated, confidently wrong is not.
    #[test]
    fn a_local_tier_is_judged_against_its_own_bar_and_the_asymmetry_holds() {
        assert!(SMALL.max_miss_rate() > FRONTIER.max_miss_rate());
        assert!(SMALL.max_false_positive_rate() > FRONTIER.max_false_positive_rate());
        assert!(
            SMALL.max_false_positive_rate() - FRONTIER.max_false_positive_rate()
                < SMALL.max_miss_rate() - FRONTIER.max_miss_rate(),
            "the false-positive bar must loosen far less than the miss bar, or a degraded \
             tier is licensed to be confidently wrong"
        );
        for t in TIERS_IN_SCOPE {
            let th = Threshold::for_tier(t);
            assert!(
                th.max_false_positive_rate() < 0.5,
                "{t:?} may call coin flips"
            );
            assert!(!th.declared_in().is_empty(), "{t:?} has a bar from nowhere");
        }
    }

    /// DONE-4's second clause has teeth: a tier missing from the report fails
    /// it, because a tier nobody listed is a tier nobody measured.
    #[test]
    fn a_tier_absent_from_the_report_fails_done_4() {
        let c = corpus();
        let mut scores = BTreeMap::new();
        scores.insert(
            Tier::Frontier.as_str().to_string(),
            score_tier(Tier::Frontier, &c, &omniscient(&c)),
        );
        let verdict = done_4(&scores);
        assert!(!verdict.met);
        assert_eq!(
            verdict.reasons.len(),
            TIERS_IN_SCOPE.len() - 1,
            "{:?}",
            verdict.reasons
        );
        assert!(verdict.reasons.iter().all(|r| r.contains("absent")));
    }

    /// And it is met when the frontier is measured at threshold and every other
    /// tier says `unmeasured` out loud — which is the honest state today.
    #[test]
    fn an_unmeasured_local_tier_satisfies_done_4_and_an_unmeasured_frontier_does_not() {
        let c = corpus();
        let mut scores = BTreeMap::new();
        for t in TIERS_IN_SCOPE {
            scores.insert(
                t.as_str().to_string(),
                TierScore::Unmeasured(NotMeasured::NeverRun),
            );
        }
        let verdict = done_4(&scores);
        assert!(
            !verdict.met,
            "an unmeasured frontier fails the first clause"
        );
        assert_eq!(verdict.unmeasured.len(), TIERS_IN_SCOPE.len());

        scores.insert(
            Tier::Frontier.as_str().to_string(),
            score_tier(Tier::Frontier, &c, &omniscient(&c)),
        );
        let verdict = done_4(&scores);
        assert!(verdict.met, "{:?}", verdict.reasons);
        assert_eq!(verdict.unmeasured.len(), TIERS_IN_SCOPE.len() - 1);

        // A measured-but-below-threshold frontier fails, and says on which rate.
        scores.insert(
            Tier::Frontier.as_str().to_string(),
            score_tier(Tier::Frontier, &c, &constant(&c, Verdict::Supported)),
        );
        let verdict = done_4(&scores);
        assert!(!verdict.met);
        assert!(
            verdict.reasons.iter().any(|r| r.contains("false-positive")),
            "{:?}",
            verdict.reasons
        );
    }

    /// A rate over an empty denominator is `None`, never `0.0`. The same
    /// argument the scorecard makes about the barren rate, one level down.
    #[test]
    fn an_empty_denominator_is_none_and_not_zero() {
        let d = Discrimination::default();
        assert!(d.miss_rate().is_none());
        assert!(d.false_positive_rate().is_none());
        assert!(d.leak_miss_rate().is_none());
        assert!(d.misranking_rate().is_none());
        assert!(d.no_call_rate().is_none());
    }

    /// Criterion failures are attributed, not counted. "The tier failed" is not
    /// a finding.
    #[test]
    fn the_failing_criteria_are_named() {
        let c = corpus();
        let mut runs = omniscient(&c);
        for o in runs.values_mut() {
            o.falsifier = None;
        }
        let m = score_tier(Tier::Frontier, &c, &runs)
            .measurement()
            .expect("measured")
            .clone();
        assert_eq!(
            m.criteria_failures
                .get(Criterion::SealedAnEvaluableFalsifier.as_str())
                .copied(),
            Some(c.len()),
            "{:?}",
            m.criteria_failures
        );
        assert!(
            m.criteria_pass_rate < 1.0,
            "every case failed a criterion, so the pass rate cannot be 1.0"
        );
        // And the two rates are untouched by it: a process failure is not a
        // judgement failure and must not be laundered into one.
        assert_eq!(m.discrimination.miss_rate(), Some(0.0));
        assert_eq!(m.discrimination.false_positive_rate(), Some(0.0));
    }
}
