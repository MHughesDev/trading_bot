//! Planted ground truth for the `judgement` suite (AGENT-009 §3, §8.8;
//! AGENT-013 §10; Set S task S-10.16).
//!
//! # Why a generator and not a fixture directory
//!
//! AGENT-009 §3 describes cases as `evals/<suite>/<case>/` with an
//! `expected.json`. For every other suite that is right: a tape is a recording
//! and a recording belongs in a file. For this suite it is wrong twice over.
//!
//! 1. **`expected.json` *is* the answer key, written to a path.** The agent has
//!    `code_execution` on some profiles and a workspace on all of them. A file
//!    whose contents say "this one is a null" is one `cat` away from making
//!    every number on the scorecard a number about file reading.
//!    `auth::scopes::NEVER_FOR_AGENTS` lists `evals.truth` for exactly this
//!    reason, and a scope is not much of a fence around a file on disk.
//! 2. **A fixture is a case nobody can vary.** The one question this suite has
//!    to answer — *can this tier tell an edge from noise* — is a question about
//!    a distribution, and a distribution needs more than one draw. A corpus you
//!    can only re-generate by hand is a corpus that gets re-generated never, and
//!    a suite fitted to twelve files is the curve-fitted backtest this platform
//!    exists to argue against.
//!
//! So the corpus is **generated, deterministically, from a seed**, and the
//! answer key lives only in the memory of the process doing the scoring. There
//! is no file to read, no row to select and no field to serialise: [`AnswerKey`]
//! implements neither `Serialize` nor `Deserialize`, and there is no public
//! accessor that returns the planted mechanism. The only thing it will do is
//! score an [`Observed`] run.
//!
//! # What is planted, and what is refused
//!
//! AGENT-009 §3's `judgement` row names eight things. Four of them are
//! properties of *data*, and this module plants all four with an answer it can
//! verify inside this crate:
//!
//! | Plant | Ground truth | How it is verified here |
//! |---|---|---|
//! | [`PlantKind::Edge`] | a signal correlated with the *next* return at a declared ρ | [`reference_t`] recovers it at or above [`REFERENCE_POWER_FLOOR`] |
//! | [`PlantKind::Null`] | a signal independent of every future return | [`reference_t`] falsely fires at or below [`REFERENCE_FALSE_POSITIVE_CEILING`] |
//! | [`PlantKind::Leak`] | one feature column *is* the next return | its `t` exceeds [`LEAK_T_FLOOR`], which no honest case in the corpus reaches |
//! | [`PlantKind::Dominance`] | two candidates on one market, one at a strictly higher ρ | [`reference_t`] ranks the planted winner first at or above [`REFERENCE_RANKING_FLOOR`] |
//!
//! The other four are properties of the **run**, not of the data — the workflow
//! it chose, where its refinement chain stopped, whether the falsifier it sealed
//! can be evaluated, whether it produced any yield — and
//! [`super::judgement::score`] already scores them. They are asserted against a
//! label, not measured against a planted mechanism, and this module says so
//! rather than dressing a label up as ground truth.
//!
//! **One thing in §3 is refused by name.** "A dead idea already in memory is
//! refused at intake" cannot be planted here. Its answer does not live in the
//! data; it lives in the semantic memory store, and that store does not exist —
//! `crates/harness/src/memory/` computes and `migrations/*_semantic_memory.sql`
//! has never been applied (Set S phase 4, S-4.2). A generator cannot plant a row
//! in a table nobody created, and planting a case whose answer this module
//! cannot verify would be exactly the failure the suite exists to catch, one
//! level up. [`super::judgement::Criterion::AvoidedTheDuplicate`] keeps scoring
//! it from a hand-written [`super::judgement::Planted`]; this corpus does not
//! generate one.
//!
//! # The corpus is one corpus, run on every tier
//!
//! §3 asks for "≥ 30 cases per tier". That is a statement about how many cases
//! each tier is *run* on, and it is served better by running the **same** cases
//! everywhere than by generating a fresh corpus per tier: two tiers scored on
//! different data are two numbers nobody may subtract, and P4's whole point is
//! that the tiers are compared. [`PlantedCorpus::generate`] therefore takes a
//! seed and a size and no tier, and [`super::discrimination::TierScore`] is what
//! varies per tier.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::judgement::{score as score_run, Observed, Planted, Score, Truth};
use crate::state::Workflow;

// ── The generator's declared constants ──────────────────────────────────────
//
// Every one of these is a number the corpus is built from, not a number read
// back off a run. They are `pub` so a reader can check the arithmetic in the
// module header against the code, and `const` so no caller can move one to make
// a corpus easier.

/// Periods per case.
///
/// 1000 is chosen so the weakest planted edge ([`RHO_LADDER`]'s first rung) has
/// a true `t` of about 3.0 — comfortably findable, and not so obvious that
/// finding it is evidence of nothing.
pub const PERIODS: usize = 1000;

/// Per-period standard deviation of the market's returns. 1% is the daily order
/// of magnitude for the assets this platform trades; the suite's arithmetic does
/// not depend on the value, only on the ratio.
pub const SIGMA: f64 = 0.01;

/// Correlations a planted edge is drawn from, between the signal's latent and
/// the **next** period's return innovation.
///
/// The ladder is deliberately shallow. A corpus of huge edges measures nothing:
/// every tier finds them, the miss rate is zero everywhere and the suite stops
/// discriminating between tiers, which is the one job it has.
pub const RHO_LADDER: [f64; 4] = [0.12, 0.16, 0.20, 0.24];

/// The weaker arm of a [`PlantKind::Dominance`] pair. Real, so that ranking it
/// second is a judgement and not a coin toss between an edge and nothing.
pub const RHO_DOMINATED: f64 = 0.08;

/// The stronger arm of a dominance pair.
pub const RHO_DOMINANT: f64 = 0.24;

/// Feature columns attached to **every** case, leak or not.
///
/// Uniform width on purpose: if only the leak cases carried a feature block, the
/// block's presence would be the answer, and a run could score perfectly by
/// counting columns. The leaking column is one of these and is named from the
/// same pool as the rest.
pub const FEATURE_COLUMNS: usize = 4;

/// The `t` a reference detector must clear to call an edge. One-sided, because
/// a trading edge is a claim about a positive expectation and a significantly
/// *negative* one is not the thing being claimed.
pub const REFERENCE_T: f64 = 2.0;

/// The `t` above which a candidate cannot have come from this market honestly.
///
/// A perfect-foresight candidate on a 1% market over 1000 periods has a `t` near
/// 40; the strongest honest rung of [`RHO_LADDER`] reaches about 6. The floor
/// sits between them with room on both sides, and
/// [`tests::the_leak_separates_from_every_honest_case`] asserts the gap rather
/// than trusting this paragraph.
pub const LEAK_T_FLOOR: f64 = 25.0;

/// Share of planted edges the reference detector must recover, or the corpus is
/// measuring nothing and the suite has no floor to score a tier against.
pub const REFERENCE_POWER_FLOOR: f64 = 0.80;

/// Share of planted nulls the reference detector may falsely call. A one-sided
/// `t > 2` on a true null fires about 2.3% of the time; the ceiling carries the
/// binomial slack of a corpus this small.
pub const REFERENCE_FALSE_POSITIVE_CEILING: f64 = 0.20;

/// Share of dominance pairs the reference detector must rank correctly.
pub const REFERENCE_RANKING_FLOOR: f64 = 0.80;

// ── Determinism ─────────────────────────────────────────────────────────────

/// A seeded splitmix64, written here rather than taken as a dependency.
///
/// Not because a dependency would be worse arithmetic — it would be better — but
/// because this generator's output *is* the answer key, and a key that changes
/// when a transitive crate bumps a patch version is a key that silently
/// re-plants every case and makes a regression unattributable. The one property
/// this needs is that the same seed gives the same corpus on every machine and
/// every year, and sixteen lines of arithmetic with no dependencies has it.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Rng { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform on (0, 1), never exactly 0 — `ln(0)` in Box-Muller is an infinity
    /// that would appear in the data as a single impossible bar.
    fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / 9_007_199_254_740_992.0
    }

    /// Standard normal, Box-Muller. One of the two draws is discarded, which
    /// costs nothing here and keeps the stream position a function of the call
    /// count alone.
    pub fn normal(&mut self) -> f64 {
        let u1 = self.unit();
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

// ── What the run is allowed to see ──────────────────────────────────────────

/// One candidate signal, as the run receives it.
///
/// `signal[t]` is the position held *into* period `t + 1`, so a candidate's
/// realised return series is `signal[t] * market[t + 1]`. The id is opaque: an
/// id like `dominant` would hand over the answer to the dominance cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub signal: Vec<f64>,
}

/// One feature column, as the run receives it.
///
/// In a [`PlantKind::Leak`] case exactly one of these **is** the next period's
/// return, and nothing about its name, position or width says which.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    pub name: String,
    pub values: Vec<f64>,
}

/// Everything the run is given. Serialisable, because it has to travel to the
/// run; it carries no label the answer could be read off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseBrief {
    pub case_id: String,
    /// The workflow the question's shape calls for. A **label**, not a measured
    /// property of the data — see the module header.
    pub question_shape: Workflow,
    pub question: String,
    pub market: Vec<f64>,
    pub candidates: Vec<Candidate>,
    pub features: Vec<Feature>,
}

// ── The answer key ──────────────────────────────────────────────────────────

/// Which of the four verifiable mechanisms a case carries.
///
/// Deliberately **not** `Serialize`: this enum names the answer, and a `derive`
/// here is all it would take for it to reach a JSON body, a tape or a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlantKind {
    Edge,
    Null,
    Leak,
    Dominance,
}

/// The planted mechanism for one case.
///
/// # Where the answer key lives, and why nothing can reach it
///
/// Here, in this struct, in the memory of whatever process generated the corpus,
/// and nowhere else.
///
/// * It is **generated, never stored.** There is no `expected.json`, no
///   `eval_truth` table and no fixture. Nothing persists it, so nothing can read
///   it back.
/// * It implements **neither `Serialize` nor `Deserialize`**, so it cannot be
///   put in an HTTP response, a tool result, a tape or a scorecard artifact. That
///   is a compile error at every one of those boundaries, not a convention.
/// * Its fields are **private** and there is no accessor that returns them. The
///   only public operation is [`PlantedCase::score`], which consumes a run and
///   returns a verdict about it.
/// * Its `Debug` is **redacted**, because a `{:?}` in a `tracing` line is the
///   ordinary way a private field becomes a public string.
///
/// The residual exposure is honest and worth naming: the seed passed to
/// [`PlantedCorpus::generate`] reconstructs the whole corpus, key included.
/// Whatever process holds that seed holds the answers, so the seed belongs with
/// the scorer and behind `evals.truth` — never in a brief, a task, a workspace
/// file or a prompt. [`tests::no_brief_carries_the_seed_or_the_answer`] asserts
/// the first half of that; the second half is a deployment property this crate
/// cannot enforce and is listed as such in the report.
#[derive(Clone, PartialEq)]
pub struct AnswerKey {
    kind: PlantKind,
    /// The planted correlation, for an [`PlantKind::Edge`] or the winning arm of
    /// a [`PlantKind::Dominance`]. Zero for a null.
    rho: f64,
    /// Which feature column is the future, for a [`PlantKind::Leak`].
    leaking_feature: Option<String>,
    /// Which candidate genuinely dominates, for a [`PlantKind::Dominance`].
    winner: Option<String>,
    question_shape: Workflow,
}

impl std::fmt::Debug for AnswerKey {
    /// Redacted on purpose. A `{:?}` in a log line is the ordinary way a private
    /// field becomes a public string, and this one is the answer key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnswerKey(<sealed: evals.truth>)")
    }
}

impl AnswerKey {
    /// The scoring vocabulary this key speaks, built on demand and never stored.
    fn planted(&self) -> Planted {
        let truth = match self.kind {
            PlantKind::Edge => Truth::KnownEdge { effect: self.rho },
            PlantKind::Null => Truth::KnownNull,
            PlantKind::Leak => Truth::LeakingFeature {
                feature: self
                    .leaking_feature
                    .clone()
                    .unwrap_or_else(|| "<unset>".to_string()),
            },
            PlantKind::Dominance => Truth::Dominance {
                winner: self.winner.clone().unwrap_or_else(|| "<unset>".to_string()),
            },
        };
        Planted {
            truth,
            question_shape: self.question_shape,
        }
    }

    /// The answer, in plain text. **Test-only**, and that is the point: the
    /// production build has no way to render this at all, so
    /// [`tests::no_brief_carries_the_seed_or_the_answer`] can prove the tokens
    /// it looks for are real discriminators without the tokens ever becoming
    /// reachable outside `cfg(test)`.
    #[cfg(test)]
    fn reveal(&self) -> String {
        format!(
            "{:?} rho={} leak={:?} winner={:?}",
            self.kind, self.rho, self.leaking_feature, self.winner
        )
    }

    #[cfg(test)]
    fn kind(&self) -> PlantKind {
        self.kind
    }
}

/// One planted case: what the run sees, and what it does not.
#[derive(Debug, Clone)]
pub struct PlantedCase {
    brief: CaseBrief,
    key: AnswerKey,
}

impl PlantedCase {
    /// What the run is given.
    #[must_use]
    pub fn brief(&self) -> &CaseBrief {
        &self.brief
    }

    #[must_use]
    pub fn case_id(&self) -> &str {
        &self.brief.case_id
    }

    /// Score one run of this case against what the case actually contained.
    ///
    /// The only way out of [`AnswerKey`]: a verdict about a run, never the
    /// mechanism that produced the verdict.
    #[must_use]
    pub fn score(&self, observed: &Observed) -> Score {
        score_run(&self.key.planted(), observed)
    }

    pub(super) fn key_kind(&self) -> PlantKind {
        self.key.kind
    }

    pub(super) fn planted_winner(&self) -> Option<&str> {
        self.key.winner.as_deref()
    }
}

// ── The corpus ──────────────────────────────────────────────────────────────

/// AGENT-009 §3: at least this many cases are run on each tier.
pub const MIN_CASES_PER_TIER: usize = 30;

/// The composition of a corpus, as a share of its size.
///
/// Edges and nulls in equal number, because the two error rates this suite
/// reports are estimated separately and an imbalance makes one of them the
/// noisier number for no reason.
const EDGE_SHARE: f64 = 0.39;
const NULL_SHARE: f64 = 0.39;
const LEAK_SHARE: f64 = 0.11;

/// A generated corpus of planted cases.
#[derive(Debug, Clone)]
pub struct PlantedCorpus {
    seed: u64,
    cases: Vec<PlantedCase>,
}

impl PlantedCorpus {
    /// Generate `size` cases from `seed`.
    ///
    /// Deterministic: the same `(seed, size)` gives byte-identical briefs and an
    /// identical key on every machine, so a score is reproducible and a
    /// regression is attributable to the change that caused it rather than to
    /// the draw.
    ///
    /// # Panics
    ///
    /// Never. `size` below [`MIN_CASES_PER_TIER`] is allowed — a small corpus is
    /// a legitimate thing to generate for a test — and it is
    /// [`super::discrimination`] that refuses to *report a score* from one.
    #[must_use]
    pub fn generate(seed: u64, size: usize) -> Self {
        let mut rng = Rng::seeded(seed);

        let edges = ((size as f64) * EDGE_SHARE).round() as usize;
        let nulls = ((size as f64) * NULL_SHARE).round() as usize;
        let leaks = ((size as f64) * LEAK_SHARE).round() as usize;
        let dominance = size.saturating_sub(edges + nulls + leaks);

        let mut plan: Vec<PlantKind> = Vec::with_capacity(size);
        plan.extend(std::iter::repeat_n(PlantKind::Edge, edges));
        plan.extend(std::iter::repeat_n(PlantKind::Null, nulls));
        plan.extend(std::iter::repeat_n(PlantKind::Leak, leaks));
        plan.extend(std::iter::repeat_n(PlantKind::Dominance, dominance));

        // Shuffle so the kind is not a function of the position. A corpus whose
        // first twelve cases are always the edges is one an ordering can pass.
        for i in (1..plan.len()).rev() {
            let j = rng.below(i + 1);
            plan.swap(i, j);
        }

        let cases = plan
            .into_iter()
            .enumerate()
            .map(|(i, kind)| build_case(seed, i, kind, &mut rng))
            .collect();

        PlantedCorpus { seed, cases }
    }

    /// A corpus of the size AGENT-009 §3 requires.
    #[must_use]
    pub fn standard(seed: u64) -> Self {
        Self::generate(seed, MIN_CASES_PER_TIER + 6)
    }

    #[must_use]
    pub fn cases(&self) -> &[PlantedCase] {
        &self.cases
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cases.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    #[must_use]
    pub fn case(&self, case_id: &str) -> Option<&PlantedCase> {
        self.cases.iter().find(|c| c.case_id() == case_id)
    }

    /// What the runs are given — and all they are given.
    pub fn briefs(&self) -> impl Iterator<Item = &CaseBrief> {
        self.cases.iter().map(PlantedCase::brief)
    }

    /// `(case_id, suite)` pairs for [`super::holdout::Holdout::draw`], so the
    /// sealed 20% of §4.2 applies to this suite like every other.
    #[must_use]
    pub fn holdout_input(&self) -> Vec<(String, String)> {
        self.cases
            .iter()
            .map(|c| (c.case_id().to_string(), "judgement".to_string()))
            .collect()
    }

    /// The seed this corpus came from.
    ///
    /// Held behind `evals.truth` with the scorer: it reconstructs the key. It is
    /// exposed because a scorecard has to record *which* corpus produced a
    /// number — a score against an unnamed corpus is not reproducible — and the
    /// recording surface is the scorecard artifact, which no agent session
    /// reads.
    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Score every run submitted against the case it claims to answer.
    ///
    /// Runs keyed by a case id this corpus does not contain are returned as
    /// `unknown` rather than ignored: a scorer that silently drops a row it does
    /// not recognise reports a pass rate over a denominator nobody chose.
    #[must_use]
    pub fn score_all(&self, runs: &BTreeMap<String, Observed>) -> ScoredCorpus {
        let mut scored = Vec::new();
        for case in &self.cases {
            if let Some(observed) = runs.get(case.case_id()) {
                scored.push((case.case_id().to_string(), case.score(observed)));
            }
        }
        let unknown = runs
            .keys()
            .filter(|id| self.case(id).is_none())
            .cloned()
            .collect();
        ScoredCorpus {
            scored,
            not_run: self
                .cases
                .iter()
                .filter(|c| !runs.contains_key(c.case_id()))
                .map(|c| c.case_id().to_string())
                .collect(),
            unknown,
        }
    }
}

/// Every case's per-criterion score, plus what did not line up.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCorpus {
    pub scored: Vec<(String, Score)>,
    /// Cases the tier was never run on. Named, not counted: a tier that skipped
    /// the hard half of the corpus and scored the rest is exactly what
    /// `unmeasured` exists to catch.
    pub not_run: Vec<String>,
    /// Runs submitted for a case id this corpus has no record of.
    pub unknown: Vec<String>,
}

impl ScoredCorpus {
    /// Cases where every applicable criterion passed, over cases actually run.
    ///
    /// **Not the suite's score.** It is one of the several numbers
    /// [`super::discrimination::Measurement`] carries, and on its own it is the
    /// mixed-set average AGENT-009 §4 refuses: a tier that calls every case an
    /// edge and a tier that calls every case a null can land on the same value
    /// here. The two numbers that tell them apart are the miss rate and the
    /// false-positive rate, reported separately and never combined.
    #[must_use]
    pub fn criteria_pass_rate(&self) -> Option<f64> {
        if self.scored.is_empty() {
            return None;
        }
        let passed = self.scored.iter().filter(|(_, s)| s.passed()).count();
        Some(passed as f64 / self.scored.len() as f64)
    }
}

// ── Building one case ───────────────────────────────────────────────────────

/// Opaque, stable, and carrying no hint of the kind.
fn case_id(seed: u64, index: usize) -> String {
    let mut h = Rng::seeded(seed ^ (index as u64).wrapping_mul(0x51_7C_C1_B7_27_22_0A_95));
    format!("judgement/{:012x}", h.next_u64() >> 16)
}

/// Feature names, drawn from one pool for every case so that the leaking column
/// is not identifiable by its name.
const FEATURE_POOL: [&str; 8] = [
    "f_carry", "f_basis", "f_flow", "f_skew", "f_disp", "f_term", "f_tilt", "f_vol",
];

fn build_case(seed: u64, index: usize, kind: PlantKind, rng: &mut Rng) -> PlantedCase {
    // The market's innovations. `z[t]` drives `market[t]`; a signal formed at
    // `t` earns `market[t + 1]`, so an edge is a correlation with `z[t + 1]`.
    let z: Vec<f64> = (0..PERIODS).map(|_| rng.normal()).collect();
    let market: Vec<f64> = z.iter().map(|v| v * SIGMA).collect();

    let names = pick_features(rng);
    let mut features: Vec<Feature> = names
        .iter()
        .map(|name| Feature {
            name: (*name).to_string(),
            values: (0..PERIODS).map(|_| rng.normal()).collect(),
        })
        .collect();

    let (candidates, key) = match kind {
        PlantKind::Edge => {
            let rho = RHO_LADDER[rng.below(RHO_LADDER.len())];
            let id = opaque_id(rng);
            (
                vec![Candidate {
                    id,
                    signal: correlated_signal(&z, rho, rng),
                }],
                AnswerKey {
                    kind,
                    rho,
                    leaking_feature: None,
                    winner: None,
                    question_shape: Workflow::Investigate,
                },
            )
        }
        PlantKind::Null => {
            let id = opaque_id(rng);
            (
                vec![Candidate {
                    id,
                    signal: correlated_signal(&z, 0.0, rng),
                }],
                AnswerKey {
                    kind,
                    rho: 0.0,
                    leaking_feature: None,
                    winner: None,
                    question_shape: Workflow::Investigate,
                },
            )
        }
        PlantKind::Leak => {
            // One column *is* the next return. Which one is chosen at random, so
            // position is not the answer either.
            let which = rng.below(FEATURE_COLUMNS);
            let mut leaked = Vec::with_capacity(PERIODS);
            for t in 0..PERIODS {
                leaked.push(if t + 1 < PERIODS { market[t + 1] } else { 0.0 });
            }
            features[which].values = leaked;
            let leaking_feature = features[which].name.clone();
            let id = opaque_id(rng);
            let signal: Vec<f64> = features[which]
                .values
                .iter()
                .map(|v| if *v >= 0.0 { 1.0 } else { -1.0 })
                .collect();
            (
                vec![Candidate { id, signal }],
                AnswerKey {
                    kind,
                    rho: 1.0,
                    leaking_feature: Some(leaking_feature),
                    winner: None,
                    question_shape: Workflow::Falsify,
                },
            )
        }
        PlantKind::Dominance => {
            let a = opaque_id(rng);
            let b = opaque_id(rng);
            // Which id gets the stronger arm is drawn, so "the first one wins" is
            // not a strategy.
            let dominant_first = rng.below(2) == 0;
            let (strong, weak) = if dominant_first {
                (a.clone(), b.clone())
            } else {
                (b.clone(), a.clone())
            };
            let mut candidates = vec![
                Candidate {
                    id: strong.clone(),
                    signal: correlated_signal(&z, RHO_DOMINANT, rng),
                },
                Candidate {
                    id: weak,
                    signal: correlated_signal(&z, RHO_DOMINATED, rng),
                },
            ];
            if !dominant_first {
                candidates.swap(0, 1);
            }
            (
                candidates,
                AnswerKey {
                    kind,
                    rho: RHO_DOMINANT,
                    leaking_feature: None,
                    winner: Some(strong),
                    question_shape: Workflow::Compare,
                },
            )
        }
    };

    let question = question_for(kind, &candidates);
    PlantedCase {
        brief: CaseBrief {
            case_id: case_id(seed, index),
            question_shape: key.question_shape,
            question,
            market,
            candidates,
            features,
        },
        key,
    }
}

/// The brief's prose.
///
/// Phrased identically across kinds wherever it can be: a question that says
/// "check whether this spurious signal holds up" would be the answer key written
/// in English.
fn question_for(kind: PlantKind, candidates: &[Candidate]) -> String {
    match kind {
        PlantKind::Dominance => format!(
            "Two candidate signals, `{}` and `{}`, were run on the same market. \
             Which, if either, should the platform carry forward?",
            candidates[0].id, candidates[1].id
        ),
        _ => format!(
            "Candidate signal `{}` was run on this market. Does it have an edge \
             after costs, and what would falsify that?",
            candidates[0].id
        ),
    }
}

fn opaque_id(rng: &mut Rng) -> String {
    format!("cand_{:08x}", rng.next_u64() >> 32)
}

fn pick_features(rng: &mut Rng) -> Vec<&'static str> {
    let mut pool: Vec<&'static str> = FEATURE_POOL.to_vec();
    for i in (1..pool.len()).rev() {
        let j = rng.below(i + 1);
        pool.swap(i, j);
    }
    pool.truncate(FEATURE_COLUMNS);
    pool
}

/// A ±1 signal at `t` whose latent correlates `rho` with the innovation of
/// `t + 1`.
///
/// `rho = 0` is the null: the latent is independent noise, so the signal is
/// uncorrelated with every future return by construction, not by a draw that
/// happened to come out flat.
fn correlated_signal(z: &[f64], rho: f64, rng: &mut Rng) -> Vec<f64> {
    let orthogonal = (1.0 - rho * rho).max(0.0).sqrt();
    (0..z.len())
        .map(|t| {
            let future = if t + 1 < z.len() { z[t + 1] } else { 0.0 };
            let latent = rho * future + orthogonal * rng.normal();
            if latent >= 0.0 {
                1.0
            } else {
                -1.0
            }
        })
        .collect()
}

// ── The reference detector ──────────────────────────────────────────────────
//
// Not part of the suite's scoring. It exists so the corpus can be checked: a
// planted edge no correct method can recover is a case that measures nothing,
// and a planted null a correct method routinely "finds" is a case that fails
// every tier for free. This is the oracle test AGENT-006 asks of a strategy,
// applied to the eval corpus.

/// A candidate's realised performance, as a plain statistic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reference {
    pub mean: f64,
    pub t_stat: f64,
    pub n: usize,
}

impl Reference {
    /// Whether a naive detector would call this an edge.
    #[must_use]
    pub fn calls_an_edge(&self) -> bool {
        self.t_stat > REFERENCE_T
    }
}

/// `t` on the realised returns of `signal` against `market`.
///
/// Returns `None` rather than a number when there is nothing to compute from —
/// a zero `t` on an empty series is a measurement nobody made, and this module
/// does not report one.
#[must_use]
pub fn reference_t(market: &[f64], signal: &[f64]) -> Option<Reference> {
    let n = market.len().min(signal.len()).saturating_sub(1);
    if n < 2 {
        return None;
    }
    let returns: Vec<f64> = (0..n).map(|t| signal[t] * market[t + 1]).collect();
    let n_f = n as f64;
    let mean = returns.iter().sum::<f64>() / n_f;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n_f - 1.0);
    let sd = var.sqrt();
    if sd <= f64::EPSILON {
        return None;
    }
    Some(Reference {
        mean,
        t_stat: mean / (sd / n_f.sqrt()),
        n,
    })
}

/// Every candidate in a brief, scored by [`reference_t`], best first.
#[must_use]
pub fn reference_ranking(brief: &CaseBrief) -> Vec<(String, Reference)> {
    let mut ranked: Vec<(String, Reference)> = brief
        .candidates
        .iter()
        .filter_map(|c| reference_t(&brief.market, &c.signal).map(|r| (c.id.clone(), r)))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.t_stat
            .partial_cmp(&a.1.t_stat)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> PlantedCorpus {
        PlantedCorpus::standard(0x5EED_0001)
    }

    fn of_kind(c: &PlantedCorpus, kind: PlantKind) -> Vec<&PlantedCase> {
        c.cases()
            .iter()
            .filter(|case| case.key.kind() == kind)
            .collect()
    }

    /// The whole point of a generator over a fixture directory: the same seed
    /// gives the same corpus, so a score is reproducible and a regression is
    /// attributable to the change rather than to the draw.
    #[test]
    fn the_same_seed_gives_the_same_corpus_and_a_different_one_does_not() {
        let a = PlantedCorpus::standard(7);
        let b = PlantedCorpus::standard(7);
        let c = PlantedCorpus::standard(8);

        let briefs =
            |p: &PlantedCorpus| serde_json::to_string(&p.briefs().collect::<Vec<_>>()).unwrap();
        assert_eq!(briefs(&a), briefs(&b), "same seed, same corpus");
        assert_ne!(
            briefs(&a),
            briefs(&c),
            "a different seed must actually vary the data, or the seed is decoration"
        );
        // And the keys move with it, or a "different" corpus would carry the
        // same answers under new data.
        let keys = |p: &PlantedCorpus| {
            p.cases()
                .iter()
                .map(|case| case.key.reveal())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&a), keys(&b));
        assert_ne!(keys(&a), keys(&c));
    }

    /// AGENT-009 §3: at least thirty cases per tier, and **both error directions
    /// present in quantity**. A corpus of thirty-five edges and one null can
    /// report a false-positive rate of 0% or 100% and nothing in between.
    #[test]
    fn a_standard_corpus_has_thirty_cases_and_both_directions() {
        let c = corpus();
        assert!(c.len() >= MIN_CASES_PER_TIER, "{} cases", c.len());
        assert!(of_kind(&c, PlantKind::Edge).len() >= 10);
        assert!(of_kind(&c, PlantKind::Null).len() >= 10);
        assert!(!of_kind(&c, PlantKind::Leak).is_empty());
        assert!(!of_kind(&c, PlantKind::Dominance).is_empty());
    }

    /// Seeds the corpus-validity checks run over.
    ///
    /// **Not one seed.** A generator whose planted edges happen to be findable
    /// on the draw somebody tried is the fitted backtest this platform exists to
    /// argue against, one level up, and a single-seed check is how it would look
    /// while it happened. Eight seeds give the rates below a denominator in the
    /// hundreds rather than the teens.
    const VALIDATION_SEEDS: [u64; 8] = [
        0x5EED_0001,
        0x5EED_0002,
        0x0BAD_C0DE,
        0xF00D_1234,
        1,
        7,
        0xDEAD_BEEF,
        0x1234_5678,
    ];

    /// The oracle test on the corpus itself.
    ///
    /// A planted edge no correct method can recover is a case that measures
    /// nothing; a planted null a correct method routinely "finds" fails every
    /// tier for free. Both would look like a working suite, and neither would be
    /// visible in any number the suite reports — which is why the corpus gets
    /// the oracle test AGENT-006 demands of a strategy.
    #[test]
    fn the_planted_edges_are_findable_and_the_planted_nulls_are_not() {
        let (mut edges, mut edges_found) = (0usize, 0usize);
        let (mut nulls, mut nulls_found) = (0usize, 0usize);

        for seed in VALIDATION_SEEDS {
            let c = PlantedCorpus::standard(seed);
            for case in c.cases() {
                let b = case.brief();
                let calls = reference_t(&b.market, &b.candidates[0].signal)
                    .is_some_and(|r| r.calls_an_edge());
                match case.key.kind() {
                    PlantKind::Edge => {
                        edges += 1;
                        edges_found += usize::from(calls);
                    }
                    PlantKind::Null => {
                        nulls += 1;
                        nulls_found += usize::from(calls);
                    }
                    _ => {}
                }
            }
        }

        assert!(edges >= 100 && nulls >= 100, "{edges} edges, {nulls} nulls");
        let power = edges_found as f64 / edges as f64;
        let false_positives = nulls_found as f64 / nulls as f64;
        assert!(
            power >= REFERENCE_POWER_FLOOR,
            "a reference detector recovers only {power:.3} of {edges} planted edges — the              corpus is measuring nothing"
        );
        assert!(
            false_positives <= REFERENCE_FALSE_POSITIVE_CEILING,
            "a reference detector calls {false_positives:.3} of {nulls} planted nulls an edge              — the corpus fails every tier for free"
        );
        assert!(
            power > false_positives + 0.4,
            "power {power:.3} and false positives {false_positives:.3} are not separated: a              corpus that cannot be discriminated cannot discriminate between tiers"
        );
    }

    /// The leak is separable from every honest case by a margin, or
    /// [`LEAK_T_FLOOR`] is a number somebody made up.
    #[test]
    fn the_leak_separates_from_every_honest_case() {
        for seed in VALIDATION_SEEDS {
            leak_separates_on(&PlantedCorpus::standard(seed));
        }
    }

    fn leak_separates_on(c: &PlantedCorpus) {
        let t_of = |case: &PlantedCase| {
            let b = case.brief();
            reference_t(&b.market, &b.candidates[0].signal)
                .expect("a case always has a computable t")
                .t_stat
        };

        let leaks = of_kind(c, PlantKind::Leak);
        assert!(!leaks.is_empty(), "no leak cases to separate");
        for case in &leaks {
            assert!(
                t_of(case) > LEAK_T_FLOOR,
                "a perfect-foresight candidate scored t={:.1}, below the floor",
                t_of(case)
            );
        }
        for case in c.cases() {
            if case.key.kind() == PlantKind::Leak {
                continue;
            }
            assert!(
                t_of(case) < LEAK_T_FLOOR,
                "an honest case scored t={:.1}, above the leak floor — the floor does not \
                 separate anything",
                t_of(case)
            );
        }
    }

    /// The planted winner really does dominate, by the same statistic a run
    /// would use.
    #[test]
    fn the_planted_winner_wins_under_a_reference_ranking() {
        let (mut pairs, mut correct) = (0usize, 0usize);
        for seed in VALIDATION_SEEDS {
            let c = PlantedCorpus::standard(seed);
            for case in of_kind(&c, PlantKind::Dominance) {
                pairs += 1;
                let ranked = reference_ranking(case.brief());
                if ranked.first().map(|(id, _)| id.as_str()) == case.planted_winner() {
                    correct += 1;
                }
            }
        }
        assert!(pairs >= 24, "only {pairs} dominance pairs to rank");
        let rate = correct as f64 / pairs as f64;
        assert!(
            rate >= REFERENCE_RANKING_FLOOR,
            "a reference ranking gets the planted winner right only {rate:.2} of the time              over {pairs} pairs — a pair a correct method cannot rank is a pair that fails              every tier for free"
        );
    }

    /// The brief's prose is a function of its **shape**, not of its kind.
    ///
    /// This is the property that makes the wording safe. Every single-candidate
    /// case — edge, null and leak alike — is asked the identical question, so a
    /// run cannot read the answer off the phrasing, and a future edit that
    /// helpfully rewrote one kind's question would fail here rather than
    /// quietly halving the suite's difficulty.
    #[test]
    fn the_brief_prose_is_a_function_of_shape_not_of_kind() {
        let c = PlantedCorpus::generate(0xA11C_E5EE_D001, 120);
        let template = |case: &PlantedCase| {
            let mut q = case.brief().question.clone();
            for cand in &case.brief().candidates {
                q = q.replace(&cand.id, "<candidate>");
            }
            q
        };

        let mut by_kind: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
        for case in c.cases() {
            by_kind
                .entry(format!("{:?}", case.key.kind()))
                .or_default()
                .insert(template(case));
        }
        for (kind, templates) in &by_kind {
            assert_eq!(
                templates.len(),
                1,
                "{kind} has {} phrasings",
                templates.len()
            );
        }

        let single = ["Edge", "Null", "Leak"];
        let first = by_kind
            .get(single[0])
            .and_then(|t| t.iter().next())
            .expect("the corpus has edge cases");
        for kind in single {
            assert_eq!(
                by_kind.get(kind).and_then(|t| t.iter().next()),
                Some(first),
                "{kind} is asked a different question from {}, so the wording is the answer",
                single[0]
            );
        }
        assert_ne!(
            by_kind.get("Dominance").and_then(|t| t.iter().next()),
            Some(first),
            "a two-candidate case must read differently, or the test is comparing nothing"
        );
    }

    /// No identifier the run receives carries the answer.
    ///
    /// Identifiers — the case id, the candidate ids, the feature names — are the
    /// part of a brief that travels: into a log line, a tool call, a workspace
    /// filename, a finding. The test also asserts each token it hunts for is a
    /// **real discriminator**, present in the key's own rendering, because a
    /// search for a string that appears nowhere passes trivially and proves
    /// nothing.
    #[test]
    fn no_identifier_in_a_brief_carries_the_answer() {
        let seed = 0xA11C_E5EE_D001_u64;
        let c = PlantedCorpus::generate(seed, 36);

        let mut identifiers = String::new();
        for case in c.cases() {
            identifiers.push_str(case.case_id());
            identifiers.push(' ');
            for cand in &case.brief().candidates {
                identifiers.push_str(&cand.id);
                identifiers.push(' ');
            }
            for f in &case.brief().features {
                identifiers.push_str(&f.name);
                identifiers.push(' ');
            }
            identifiers.push_str(&case.brief().question);
            identifiers.push(' ');
        }

        for token in [
            "null", "leak", "dominan", "winner", "rho", "truth", "answer", "expected",
        ] {
            assert!(
                !identifiers.contains(token),
                "an agent-visible identifier contains {token:?}"
            );
        }
        assert!(
            !identifiers.contains(&format!("{seed}"))
                && !identifiers.contains(&format!("{seed:x}")),
            "a brief carries the seed, which reconstructs the whole key"
        );

        // And the tokens are discriminating: they are in the key.
        let keys: String = c.cases().iter().map(|case| case.key.reveal()).collect();
        for token in ["Edge", "Null", "Leak", "Dominance", "rho", "winner"] {
            assert!(
                keys.contains(token),
                "{token:?} appears in neither the brief nor the key, so looking for it in the                  brief proved nothing"
            );
        }
    }

    /// A case id that said `edge_0007` would hand over the answer to anyone who
    /// read a log line, and ids are the one part of a case that travels
    /// everywhere.
    #[test]
    fn a_case_id_says_nothing_about_its_kind() {
        let c = corpus();
        for case in c.cases() {
            let id = case.case_id();
            for token in ["edge", "null", "leak", "dom", "win"] {
                assert!(!id.contains(token), "case id {id:?} leaks its kind");
            }
        }
        let ids: std::collections::BTreeSet<&str> =
            c.cases().iter().map(PlantedCase::case_id).collect();
        assert_eq!(
            ids.len(),
            c.len(),
            "case ids must be unique or scoring maps collide"
        );
    }

    /// The leak is not identifiable by the shape of the brief: every case
    /// carries the same number of columns, drawn from the same names, and the
    /// leaking column is not always in the same place.
    #[test]
    fn a_leak_case_is_not_identifiable_by_its_shape() {
        let c = PlantedCorpus::generate(0xFEED, 120);
        for case in c.cases() {
            assert_eq!(
                case.brief().features.len(),
                FEATURE_COLUMNS,
                "an uneven feature block would make the block's width the answer"
            );
            for f in &case.brief().features {
                assert!(
                    FEATURE_POOL.contains(&f.name.as_str()),
                    "feature {:?} is not from the shared pool",
                    f.name
                );
            }
        }
        let positions: std::collections::BTreeSet<usize> = c
            .cases()
            .iter()
            .filter(|case| case.key.kind() == PlantKind::Leak)
            .filter_map(|case| {
                let leaking = case.key.leaking_feature.as_deref()?;
                case.brief().features.iter().position(|f| f.name == leaking)
            })
            .collect();
        assert!(
            positions.len() > 1,
            "the leaking column is always at position {positions:?} — position is the answer"
        );
    }

    /// Scoring reports what it did not score. A scorer that silently drops the
    /// cases a tier skipped publishes a pass rate over a denominator nobody
    /// chose.
    #[test]
    fn cases_never_run_and_runs_with_no_case_are_both_named() {
        let c = PlantedCorpus::generate(3, 30);
        let mut runs = BTreeMap::new();
        runs.insert(c.cases()[0].case_id().to_string(), Observed::default());
        runs.insert("judgement/not-a-case".to_string(), Observed::default());

        let scored = c.score_all(&runs);
        assert_eq!(scored.scored.len(), 1);
        assert_eq!(scored.not_run.len(), c.len() - 1);
        assert_eq!(scored.unknown, vec!["judgement/not-a-case".to_string()]);
        assert!(
            scored.criteria_pass_rate().is_some(),
            "one case was run, so there is a rate"
        );
        assert!(
            c.score_all(&BTreeMap::new()).criteria_pass_rate().is_none(),
            "no runs is `None`, never 0.0"
        );
    }

    /// A key is redacted in `Debug`, because `{:?}` in a `tracing` line is the
    /// ordinary way a private field becomes a public string.
    #[test]
    fn the_key_does_not_print_itself() {
        let c = corpus();
        let printed = format!("{:?}", c.cases()[0]);
        assert!(printed.contains("<sealed: evals.truth>"), "{printed}");
        for token in ["Edge", "Null", "Leak", "Dominance", "rho"] {
            assert!(!printed.contains(token), "Debug leaks {token:?}: {printed}");
        }
    }

    /// The brief's every series is the same length, and the market is not flat —
    /// a generator that emitted a constant would pass "the null is not detected"
    /// for the wrong reason.
    #[test]
    fn the_generated_data_is_real_data() {
        let c = corpus();
        for case in c.cases() {
            let b = case.brief();
            assert_eq!(b.market.len(), PERIODS);
            assert!(b.market.iter().all(|v| v.is_finite()), "a non-finite bar");
            let spread = b.market.iter().fold(f64::NAN, |a: f64, v| a.max(*v))
                - b.market.iter().fold(f64::NAN, |a: f64, v| a.min(*v));
            assert!(spread > SIGMA, "a flat market: spread {spread}");
            for cand in &b.candidates {
                assert_eq!(cand.signal.len(), PERIODS);
                assert!(
                    cand.signal.iter().any(|s| *s > 0.0) && cand.signal.iter().any(|s| *s < 0.0),
                    "a constant signal has no decision in it"
                );
            }
            for f in &b.features {
                assert_eq!(f.values.len(), PERIODS);
            }
        }
    }
}
