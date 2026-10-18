//! The `judgement` suite as data: per-tier recorded runs, the sealed 20%, and the
//! scorecard row per (tier, workflow) (AGENT-009 §3, §4.2; Set S task S-10.16).
//!
//! [`judgement`](super::judgement) scores one run against planted truth and
//! [`planted`](super::planted) generates the truth. What neither does is *run the
//! suite*: hold forty-six cases per tier, keep a fifth of them sealed, score a
//! tier's recorded runs against them and put the result on a scorecard where the
//! gate can see it. This module is that, and it is the part with callers: the PR
//! gate test in this file, `evals/runner/run.py --suite judgement` (through the
//! `judgement_replay` binary) and the nightly `nightly_*` tests that
//! `.github/workflows/eval-nightly.yml` already runs.
//!
//! # What a "tape" is here, said plainly
//!
//! A tape is the **recorded outcome of one run** — what [`Observed`] holds: the
//! verdict, the workflow chosen, where the chain stopped, the falsifier sealed,
//! whether the planted dead idea was re-tested. Replay is exact because scoring a
//! recorded outcome against a case that is itself a pure function of a seed has no
//! sampling error.
//!
//! **The tapes in `evals/judgement/tapes/` were not recorded from a model.** They
//! are the output of five declared *reference policies* ([`TierPolicy`]) — a
//! detector with a stated `t` threshold, a stated ability to find a leak, a stated
//! habit of choosing one workflow. They exist so the machinery (scoring, sealing,
//! the scorecard row, the gate) is exercised and regression-protected today. They
//! are **not** a measurement of any real tier: their scorecards carry
//! `profile_id = reference-policy/<tier>` and `hardware = "replay: synthetic
//! reference policy, not a model"`, and the profile endpoint never reads them.
//! The live nightly — a frontier model, a credential, n >= 30 per cell — is the
//! one clause this task cannot close (gate H-2).
//!
//! # Where the answer key is, and is not
//!
//! Not in `evals/judgement/`. [`CORPUS_SEED`] reconstructs every key, so it is a
//! `const` in this source file (scorer side) and the data files carry case ids,
//! recorded outcomes and policy names, never the seed and never a planted
//! mechanism. A test reads every file in the directory and fails if the seed
//! appears. Dead-idea cases are the exception that proves the rule: what memory
//! records as dead is *shown to the run* (it is the point of the case), so those
//! are plain data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::holdout::{Holdout, HoldoutDraw};
use super::judgement::{score as score_run, Observed, Planted, Score, Truth};
use super::planted::{reference_t, CaseBrief, PlantedCorpus, LEAK_T_FLOOR};
use super::scorecard::{Scorecard, SuiteKind, SuiteScore};
use crate::lifecycle::chain::{Prospect, Stop};
use crate::lifecycle::evaluate::{Comparator, Falsifier, Verdict};
use crate::lifecycle::r#yield::Yield;
use crate::profile::Tier;
use crate::state::Workflow;

/// The corpus seed. Scorer side only; see the module header.
pub const CORPUS_SEED: u64 = 0x4A55_4447_0001;

/// Generated cases per tier. With [`dead_ideas`] this is 46 a tier, of which a
/// fifth are sealed, leaving 37 on a PR: AGENT-009 §3's "at least 30" holds for
/// the set a PR runs, which is the number that gates.
pub const CORPUS_SIZE: usize = 40;

/// The salt of this suite's standing draw.
pub const JUDGEMENT_SALT: &str = "judgement-holdout-2026-10-04-a";

/// The suite name the holdout files every case under.
pub const SUITE: &str = "judgement";

/// Every tier, in capability order.
pub const TIERS: [Tier; 5] = [
    Tier::Degraded,
    Tier::LocalSmall,
    Tier::LocalMid,
    Tier::LocalHigh,
    Tier::Frontier,
];

// ── Cases ───────────────────────────────────────────────────────────────────

/// A planted dead idea: memory already records `subject` as dead, with `class`.
///
/// Plain data, unlike a planted edge: the run is *shown* this memory, so there is
/// nothing to hide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadIdeaCase {
    pub case_id: String,
    pub subject: String,
    pub class: String,
    pub question_shape: Workflow,
}

impl DeadIdeaCase {
    fn planted(&self) -> Planted {
        Planted {
            truth: Truth::DeadIdea {
                subject: self.subject.clone(),
                class: self.class.clone(),
            },
            question_shape: self.question_shape,
        }
    }
}

/// The six dead-idea cases.
#[must_use]
pub fn dead_ideas() -> Vec<DeadIdeaCase> {
    [
        ("ema_cross_v1", "costs_dominate"),
        ("rsi_meanrev_btc_1h", "weak_effect"),
        ("funding_carry_perp", "regime_dependent"),
        ("overnight_drift_spy", "weak_effect"),
        ("vol_breakout_eth_15m", "costs_dominate"),
        ("pairs_btc_eth_zscore", "unstable_across_folds"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (subject, class))| DeadIdeaCase {
        case_id: format!("judgement/dead_{:02}", i + 1),
        subject: (*subject).into(),
        class: (*class).into(),
        question_shape: Workflow::Investigate,
    })
    .collect()
}

/// The generated corpus.
#[must_use]
pub fn corpus() -> PlantedCorpus {
    PlantedCorpus::generate(CORPUS_SEED, CORPUS_SIZE)
}

/// Every case id, generated first, dead ideas after, in a stable order.
#[must_use]
pub fn case_ids() -> Vec<String> {
    let mut ids: Vec<String> = corpus()
        .cases()
        .iter()
        .map(|c| c.case_id().to_string())
        .collect();
    ids.extend(dead_ideas().into_iter().map(|d| d.case_id));
    ids
}

/// The standing draw: a fifth of the suite sealed by hash (S-4.17's mechanism,
/// [`Holdout::draw`]).
#[must_use]
pub fn holdout() -> Holdout {
    let input: Vec<(String, String)> = case_ids()
        .into_iter()
        .map(|id| (SUITE.to_string(), id))
        .collect();
    Holdout::draw(
        &input,
        HoldoutDraw {
            salt: JUDGEMENT_SALT.into(),
            requested_by: "set-s-s-10.16".into(),
            drawn_on: "2026-10-04".into(),
            corpus_size: input.len(),
        },
    )
}

// ── Recorded runs ───────────────────────────────────────────────────────────

/// What a run did, as a file. [`Observed`] is not `Serialize` on purpose (it sits
/// next to a [`Truth`]); this is the half that is safe to write down, because it
/// holds nothing the run did not already know.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedRun {
    pub verdict: Option<Verdict>,
    pub workflow: Workflow,
    pub chain_links: usize,
    pub stopped_because: Option<Stop>,
    pub prospect: Option<Prospect>,
    pub falsifier: Option<Falsifier>,
    pub retested_dead_idea: bool,
    pub found_the_leak: bool,
    pub ranked_first: Option<String>,
    pub yielded: Yield,
}

impl RecordedRun {
    #[must_use]
    pub fn observed(&self) -> Observed {
        Observed {
            verdict: self.verdict,
            workflow: self.workflow,
            chain_links: self.chain_links,
            stopped_because: self.stopped_because.clone(),
            prospect: self.prospect,
            falsifier: self.falsifier.clone(),
            retested_dead_idea: self.retested_dead_idea,
            found_the_leak: self.found_the_leak,
            ranked_first: self.ranked_first.clone(),
            yielded: self.yielded.clone(),
        }
    }
}

/// One tier's recorded runs, open or sealed half.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierTape {
    pub tier: String,
    /// The reference policy that produced these runs. Never a model name.
    pub policy: String,
    /// `synthetic_reference_policy` until a live run replaces it.
    pub provenance: String,
    pub sealed: bool,
    pub runs: BTreeMap<String, RecordedRun>,
}

/// A reference policy: the declared behaviour a tier's synthetic tape follows.
///
/// Five numbers and booleans a reader can check against the tape; none is
/// derived from a score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierPolicy {
    pub name: &'static str,
    /// The `t` this policy needs to call an edge.
    pub t_call: f64,
    /// Whether it scans features for the leak.
    pub finds_leaks: bool,
    /// Whether it chooses the workflow the question calls for.
    pub honours_shape: bool,
    /// Whether it re-tests an idea memory already records as dead, and how often
    /// (every nth dead-idea case; 0 = never).
    pub retests_every: usize,
}

#[must_use]
pub fn policy(tier: Tier) -> TierPolicy {
    match tier {
        Tier::Frontier => TierPolicy {
            name: "reference-frontier-v1",
            t_call: 2.0,
            finds_leaks: true,
            honours_shape: true,
            retests_every: 0,
        },
        Tier::LocalHigh => TierPolicy {
            name: "reference-local-high-v1",
            t_call: 2.5,
            finds_leaks: true,
            honours_shape: true,
            retests_every: 0,
        },
        Tier::LocalMid => TierPolicy {
            name: "reference-local-mid-v1",
            t_call: 3.0,
            finds_leaks: true,
            honours_shape: true,
            retests_every: 3,
        },
        Tier::LocalSmall => TierPolicy {
            name: "reference-local-small-v1",
            t_call: 3.5,
            finds_leaks: false,
            honours_shape: false,
            retests_every: 2,
        },
        Tier::Degraded => TierPolicy {
            name: "reference-degraded-v1",
            t_call: 1.0,
            finds_leaks: false,
            honours_shape: false,
            retests_every: 1,
        },
    }
}

fn falsifier() -> Falsifier {
    Falsifier {
        metric: "sharpe_after_costs".into(),
        comparator: Comparator::AtMost,
        threshold: 0.0,
    }
}

fn productive(verdict: Verdict) -> Yield {
    let mut y = Yield {
        trials_spent: 3,
        ..Yield::default()
    };
    match verdict {
        Verdict::Supported => y.findings_established = 1,
        Verdict::Refuted => y.record_negative(true),
        Verdict::Inconclusive { .. } => y.questions_opened = 1,
    }
    y
}

/// What a policy does with a generated brief. A function of the brief alone: the
/// answer key is never an input, so a tape records a run and not the answer.
#[must_use]
pub fn run_policy(p: &TierPolicy, brief: &CaseBrief) -> RecordedRun {
    let workflow = if p.honours_shape {
        brief.question_shape
    } else {
        Workflow::Investigate
    };

    // A feature that is the next return has a `t` no honest column reaches.
    let leak_found = p.finds_leaks
        && brief.features.iter().any(|f| {
            // The feature's sign is the position it would imply, exactly as a
            // candidate built from it would hold one; raw magnitudes would
            // understate a leak's `t` (a squared return has a heavy tail).
            let sign: Vec<f64> = f
                .values
                .iter()
                .map(|v| if *v >= 0.0 { 1.0 } else { -1.0 })
                .collect();
            reference_t(&brief.market, &sign).is_some_and(|r| r.t_stat.abs() > LEAK_T_FLOOR)
        });

    let mut ranking: Vec<(String, f64)> = brief
        .candidates
        .iter()
        .filter_map(|c| reference_t(&brief.market, &c.signal).map(|r| (c.id.clone(), r.t_stat)))
        .collect();
    ranking.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let best = ranking.first().cloned();

    let calls_edge = best.as_ref().is_some_and(|(_, t)| *t > p.t_call);
    let verdict = if leak_found || !calls_edge {
        Verdict::Refuted
    } else {
        Verdict::Supported
    };

    let ranked_first = if brief.candidates.len() > 1 && calls_edge {
        best.map(|(id, _)| id)
    } else {
        None
    };

    RecordedRun {
        verdict: Some(verdict),
        workflow,
        chain_links: 0,
        stopped_because: None,
        prospect: None,
        falsifier: Some(falsifier()),
        retested_dead_idea: false,
        found_the_leak: leak_found,
        ranked_first,
        yielded: productive(verdict),
    }
}

/// What a policy does with a dead idea.
#[must_use]
pub fn run_dead_idea(p: &TierPolicy, index: usize) -> RecordedRun {
    let retests = p.retests_every != 0 && (index + 1) % p.retests_every == 0;
    RecordedRun {
        verdict: Some(Verdict::Refuted),
        workflow: Workflow::Investigate,
        chain_links: 0,
        stopped_because: None,
        prospect: None,
        falsifier: Some(falsifier()),
        retested_dead_idea: retests,
        found_the_leak: false,
        ranked_first: None,
        // A re-test spends trials and re-learns the known negative: still a
        // negative, but not a new class. An avoided duplicate spends nothing.
        yielded: if retests {
            let mut y = Yield {
                trials_spent: 3,
                ..Yield::default()
            };
            y.record_negative(false);
            y
        } else {
            Yield {
                questions_closed: 1,
                ..Yield::default()
            }
        },
    }
}

/// Record every tier's tape, split into the open file and the sealed file.
///
/// The split is by [`holdout`]: the PR path loads only the open files, so a
/// sealed run is not merely unscored on a PR, it is never parsed.
#[must_use]
pub fn record_tapes() -> BTreeMap<(String, bool), TierTape> {
    let corpus = corpus();
    let dead = dead_ideas();
    let h = holdout();
    let mut out = BTreeMap::new();
    for tier in TIERS {
        let p = policy(tier);
        let mut open = BTreeMap::new();
        let mut sealed = BTreeMap::new();
        for case in corpus.cases() {
            let run = run_policy(&p, case.brief());
            let id = case.case_id().to_string();
            if h.is_sealed(&id) {
                sealed.insert(id, run);
            } else {
                open.insert(id, run);
            }
        }
        for (i, d) in dead.iter().enumerate() {
            let run = run_dead_idea(&p, i);
            if h.is_sealed(&d.case_id) {
                sealed.insert(d.case_id.clone(), run);
            } else {
                open.insert(d.case_id.clone(), run);
            }
        }
        for (is_sealed, runs) in [(false, open), (true, sealed)] {
            out.insert(
                (tier.as_str().to_string(), is_sealed),
                TierTape {
                    tier: tier.as_str().to_string(),
                    policy: p.name.into(),
                    provenance: "synthetic_reference_policy".into(),
                    sealed: is_sealed,
                    runs,
                },
            );
        }
    }
    out
}

// ── Scoring ─────────────────────────────────────────────────────────────────

/// One case's outcome, with the workflow the case calls for.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseOutcome {
    pub case_id: String,
    /// The workflow the **question** calls for (the denominator's key), not the
    /// one the run chose. A cell keyed by what the run chose would let a run move
    /// itself out of the cell it fails in.
    pub workflow: Workflow,
    pub passed: bool,
    pub failures: Vec<String>,
}

/// Score a tape against the suite's cases.
///
/// A run for a case id this suite does not contain is **refused**, not dropped:
/// a scorer that silently ignores a row reports a rate over a denominator
/// nobody chose.
///
/// # Errors
///
/// An id the suite does not know, or a sealed tape handed to a caller that said
/// it was a PR run.
pub fn score_tape(tape: &TierTape, for_pr: bool) -> Result<Vec<CaseOutcome>, String> {
    if for_pr && tape.sealed {
        return Err("a sealed tape was handed to a PR run (AGENT-009 §4.2)".into());
    }
    let corpus = corpus();
    let dead = dead_ideas();
    let h = holdout();
    let mut out = Vec::new();
    for (id, run) in &tape.runs {
        if for_pr && h.is_sealed(id) {
            return Err(format!("`{id}` is sealed and may not run on a PR"));
        }
        if h.is_sealed(id) != tape.sealed {
            return Err(format!(
                "`{id}` is in the {} tape but the standing draw says it is {}",
                if tape.sealed { "sealed" } else { "open" },
                if h.is_sealed(id) { "sealed" } else { "open" },
            ));
        }
        let observed = run.observed();
        let (workflow, score): (Workflow, Score) = if let Some(c) = corpus.case(id) {
            (c.brief().question_shape, c.score(&observed))
        } else if let Some(d) = dead.iter().find(|d| &d.case_id == id) {
            (d.question_shape, score_run(&d.planted(), &observed))
        } else {
            return Err(format!("`{id}` is not a case of the judgement suite"));
        };
        out.push(CaseOutcome {
            case_id: id.clone(),
            workflow,
            passed: score.passed(),
            failures: score.failures(),
        });
    }
    Ok(out)
}

/// `judgement/<workflow>`: the suite name of one cell of the row.
#[must_use]
pub fn cell_name(w: Workflow) -> String {
    format!("{SUITE}/{}", w.as_str())
}

/// The scorecard of one tier: one replay suite per workflow, open cells and (when
/// given) sealed cells beside them so [`super::scorecard::fitted_to_its_evals`]
/// has something to compare.
///
/// `profile_id` is `reference-policy/<tier>` and never a real profile: see the
/// module header.
#[must_use]
pub fn scorecard(tier: Tier, open: &[CaseOutcome], sealed: Option<&[CaseOutcome]>) -> Scorecard {
    let mut suites = cells(open, false);
    if let Some(s) = sealed {
        suites.extend(cells(s, true));
    }
    Scorecard {
        harness_version: env!("CARGO_PKG_VERSION").to_string(),
        profile_id: format!("reference-policy/{}", tier.as_str()),
        tier: tier.as_str().to_string(),
        hardware: "replay: synthetic reference policy, not a model".to_string(),
        suites,
        cost_usd: 0.0,
        cost_tokens: 0,
        median_steps: 0.0,
        cache_hit_rate: None,
        memory_recall: None,
        hitl_precision: None,
        utility_rate: None,
        over_estimate_rate: None,
        evaluations: 1,
        barren: Vec::new(),
    }
}

fn cells(outcomes: &[CaseOutcome], sealed: bool) -> Vec<SuiteScore> {
    let mut by: BTreeMap<&'static str, (Workflow, SuiteScore)> = BTreeMap::new();
    for o in outcomes {
        let (_, s) = by.entry(o.workflow.as_str()).or_insert_with(|| {
            (
                o.workflow,
                SuiteScore {
                    suite: cell_name(o.workflow),
                    kind: SuiteKind::Replay,
                    passed: 0,
                    n: 0,
                    flakiness: 0.0,
                    attribution: BTreeMap::new(),
                    failed_cases: Vec::new(),
                    sealed,
                },
            )
        });
        s.n += 1;
        if o.passed {
            s.passed += 1;
        } else {
            s.failed_cases.push(o.case_id.clone());
            for f in &o.failures {
                // Attribution is by criterion: which of the six ways of being
                // wrong that look like working.
                let criterion = f.split(':').next().unwrap_or("unknown").to_string();
                *s.attribution.entry(criterion).or_default() += 1;
            }
        }
    }
    by.into_values().map(|(_, s)| s).collect()
}

// ── Reading the committed tapes ─────────────────────────────────────────────

/// The open tapes, compiled in. The sealed ones are deliberately **not**: a PR
/// build does not contain them.
const OPEN_TAPES: [(&str, &str); 5] = [
    (
        "degraded",
        include_str!("../../../../evals/judgement/tapes/degraded.open.json"),
    ),
    (
        "local_small",
        include_str!("../../../../evals/judgement/tapes/local_small.open.json"),
    ),
    (
        "local_mid",
        include_str!("../../../../evals/judgement/tapes/local_mid.open.json"),
    ),
    (
        "local_high",
        include_str!("../../../../evals/judgement/tapes/local_high.open.json"),
    ),
    (
        "frontier",
        include_str!("../../../../evals/judgement/tapes/frontier.open.json"),
    ),
];

fn tier_of(name: &str) -> Tier {
    TIERS
        .iter()
        .copied()
        .find(|t| t.as_str() == name)
        .unwrap_or(Tier::Degraded)
}

/// The scorecards a PR produces: one per tier, open set only.
///
/// # Panics
///
/// If a committed tape does not parse or names a sealed case — a broken suite is
/// a failed build, not a quiet empty scorecard.
#[must_use]
pub fn pr_scorecards() -> Vec<Scorecard> {
    OPEN_TAPES
        .iter()
        .map(|(tier, text)| {
            let tape: TierTape = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("evals/judgement/tapes/{tier}.open.json: {e}"));
            let outcomes = score_tape(&tape, true).unwrap_or_else(|e| panic!("{tier}: {e}"));
            scorecard(tier_of(tier), &outcomes, None)
        })
        .collect()
}

/// The nightly promotion scorecards: open and sealed cells on one card, read from
/// `dir` (`evals/judgement/tapes`). Reaches the sealed files, so it is never
/// called on a PR path.
///
/// # Errors
///
/// A missing or unparsable file, or a tape that fails [`score_tape`].
pub fn nightly_scorecards(dir: &std::path::Path) -> Result<Vec<Scorecard>, String> {
    let mut cards = Vec::new();
    for tier in TIERS {
        let read = |half: &str| -> Result<TierTape, String> {
            let path = dir.join(format!("{}.{half}.json", tier.as_str()));
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
        };
        let open = score_tape(&read("open")?, false)?;
        let sealed = score_tape(&read("sealed")?, false)?;
        cards.push(scorecard(tier, &open, Some(&sealed)));
    }
    Ok(cards)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::planted::PlantKind;
    use crate::eval::scorecard::{fitted_to_its_evals, gate};

    const BASELINE: &str = include_str!("../../../../evals/results/judgement-baseline.json");

    fn dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals/judgement")
    }

    fn open_tape(tier: Tier) -> TierTape {
        let (_, text) = OPEN_TAPES
            .iter()
            .find(|(t, _)| *t == tier.as_str())
            .unwrap();
        serde_json::from_str(text).unwrap()
    }

    fn baseline() -> Vec<Scorecard> {
        serde_json::from_str(BASELINE).expect("the committed judgement baseline parses")
    }

    // ── Acceptance 1: at least 30 cases per tier ────────────────────────────

    #[test]
    fn every_tier_runs_at_least_thirty_cases_on_a_pr_and_has_a_sealed_fifth() {
        let h = holdout();
        let total = case_ids().len();
        assert_eq!(total, CORPUS_SIZE + dead_ideas().len());
        let sealed = h.sealed.len();
        assert!(
            (total as f64 * 0.15..=total as f64 * 0.25).contains(&(sealed as f64)),
            "{sealed} sealed of {total} is not about a fifth"
        );
        for tier in TIERS {
            let tape = open_tape(tier);
            assert!(
                tape.runs.len() >= 30,
                "{} runs {} cases on a PR; AGENT-009 §3 asks for at least 30",
                tier.as_str(),
                tape.runs.len()
            );
            assert_eq!(tape.runs.len() + sealed, total);
        }
    }

    #[test]
    fn the_corpus_exercises_every_planted_kind_and_every_workflow_the_row_names() {
        let c = corpus();
        for kind in [
            PlantKind::Edge,
            PlantKind::Null,
            PlantKind::Leak,
            PlantKind::Dominance,
        ] {
            assert!(c.cases().iter().any(|x| x.key_kind() == kind), "{kind:?}");
        }
        let cards = pr_scorecards();
        for card in &cards {
            let names: Vec<&str> = card.suites.iter().map(|s| s.suite.as_str()).collect();
            for w in [Workflow::Investigate, Workflow::Falsify, Workflow::Compare] {
                assert!(
                    names.contains(&cell_name(w).as_str()),
                    "{} lacks {}",
                    card.profile_id,
                    cell_name(w)
                );
            }
        }
    }

    // ── Acceptance 2: a planted null reported supported fails the suite ─────

    #[test]
    fn a_harness_that_reports_a_planted_null_supported_fails_the_suite() {
        let c = corpus();
        let null = c
            .cases()
            .iter()
            .find(|x| x.key_kind() == PlantKind::Null)
            .expect("a null case");
        let id = null.case_id().to_string();
        let h = holdout();

        // Take the frontier policy's honest run of that case and corrupt the one
        // thing the acceptance names.
        let mut tape = if h.is_sealed(&id) {
            // A sealed null would not be reachable on a PR; use a scratch tape.
            TierTape {
                sealed: false,
                runs: BTreeMap::new(),
                ..open_tape(Tier::Frontier)
            }
        } else {
            open_tape(Tier::Frontier)
        };
        let mut run = run_policy(&policy(Tier::Frontier), null.brief());
        assert_eq!(run.verdict, Some(Verdict::Refuted), "the honest run refutes");
        run.verdict = Some(Verdict::Supported);
        run.yielded = productive(Verdict::Supported);
        if h.is_sealed(&id) {
            // Score through the case directly: the suite's verdict is the same
            // function.
            let s = null.score(&run.observed());
            assert!(!s.passed());
            assert!(s
                .failures()
                .iter()
                .any(|f| f.contains("rejected_when_it_should")));
            return;
        }
        tape.runs.insert(id.clone(), run);
        let outcomes = score_tape(&tape, true).unwrap();
        let o = outcomes.iter().find(|o| o.case_id == id).unwrap();
        assert!(!o.passed, "{o:?}");
        assert!(o
            .failures
            .iter()
            .any(|f| f.starts_with("rejected_when_it_should")));

        // And it shows on the card, in the cell it belongs to, attributed.
        let card = scorecard(Tier::Frontier, &outcomes, None);
        let cell = card.suite(&cell_name(Workflow::Investigate)).unwrap();
        assert!(cell.failed_cases.contains(&id));
        assert!(cell.attribution.contains_key("rejected_when_it_should"));
    }

    // ── Acceptance 3: re-testing the planted dead idea fails ────────────────

    #[test]
    fn a_run_that_re_tests_the_planted_dead_idea_fails_the_suite() {
        let dead = dead_ideas();
        let h = holdout();
        let d = dead
            .iter()
            .find(|d| !h.is_sealed(&d.case_id))
            .expect("an open dead-idea case");
        let mut tape = open_tape(Tier::Frontier);
        assert!(
            score_tape(&tape, true)
                .unwrap()
                .iter()
                .find(|o| o.case_id == d.case_id)
                .unwrap()
                .passed,
            "the frontier reference policy avoids the duplicate"
        );
        let run = tape.runs.get_mut(&d.case_id).unwrap();
        run.retested_dead_idea = true;
        run.yielded = Yield {
            trials_spent: 3,
            negatives: 1,
            ..Yield::default()
        };
        let o = score_tape(&tape, true)
            .unwrap()
            .into_iter()
            .find(|o| o.case_id == d.case_id)
            .unwrap();
        assert!(!o.passed);
        assert!(o
            .failures
            .iter()
            .any(|f| f.starts_with("avoided_the_duplicate")));
    }

    // ── Acceptance 4: the scorecard shows judgement per (tier, workflow) ────

    #[test]
    fn the_scorecard_shows_judgement_per_tier_and_per_workflow() {
        let cards = pr_scorecards();
        assert_eq!(cards.len(), TIERS.len(), "one card per tier");
        for tier in TIERS {
            let card = cards
                .iter()
                .find(|c| c.tier == tier.as_str())
                .unwrap_or_else(|| panic!("no card for {}", tier.as_str()));
            assert_eq!(card.profile_id, format!("reference-policy/{}", tier.as_str()));
            assert!(
                card.hardware.contains("not a model"),
                "a synthetic row must say so"
            );
            let judged: Vec<&SuiteScore> = card
                .suites
                .iter()
                .filter(|s| s.suite.starts_with("judgement/"))
                .collect();
            assert!(judged.len() >= 3, "{}: {} cells", card.profile_id, judged.len());
            let n: usize = judged.iter().map(|s| s.n).sum();
            assert_eq!(n, open_tape(tier).runs.len(), "every open run is in a cell");
            assert!(judged.iter().all(|s| s.can_gate() && !s.sealed));
        }
        // The cells differ by tier — the row is not one number. The frontier
        // reference policy must out-judge the degraded one.
        let rate = |t: Tier| -> f64 {
            let c = cards.iter().find(|c| c.tier == t.as_str()).unwrap();
            let (p, n) = c
                .suites
                .iter()
                .fold((0, 0), |(p, n), s| (p + s.passed, n + s.n));
            p as f64 / n as f64
        };
        assert!(rate(Tier::Frontier) > rate(Tier::Degraded));
    }

    // ── Acceptance 5: the sealed 20% applies ────────────────────────────────

    #[test]
    fn the_set_a_pr_runs_contains_no_sealed_case() {
        let h = holdout();
        for tier in TIERS {
            let tape = open_tape(tier);
            assert!(!tape.sealed);
            for id in tape.runs.keys() {
                assert!(!h.is_sealed(id), "{id} is sealed and is in the PR tape");
            }
        }
        for card in pr_scorecards() {
            for cell in &card.suites {
                assert!(!cell.sealed);
                for id in &cell.failed_cases {
                    assert!(!h.is_sealed(id));
                }
            }
        }
    }

    #[test]
    fn a_sealed_tape_or_a_sealed_id_is_refused_on_a_pr_path() {
        let all = record_tapes();
        let sealed = all.get(&("frontier".to_string(), true)).unwrap();
        assert!(!sealed.runs.is_empty());
        assert!(score_tape(sealed, true).is_err(), "a sealed tape on a PR");

        // The same ids smuggled into an open tape are caught too.
        let mut smuggled = open_tape(Tier::Frontier);
        let (id, run) = sealed.runs.iter().next().unwrap();
        smuggled.runs.insert(id.clone(), run.clone());
        assert!(score_tape(&smuggled, true).is_err(), "a sealed id on a PR");
    }

    #[test]
    fn a_run_for_a_case_the_suite_does_not_contain_is_refused_not_dropped() {
        let mut tape = open_tape(Tier::Frontier);
        let run = tape.runs.values().next().unwrap().clone();
        tape.runs.insert("judgement/not_a_case".into(), run);
        assert!(score_tape(&tape, true).is_err());
    }

    /// Nightly: the sealed half is scored, on the same card as the open half, and
    /// a version fitted to its open set would be caught.
    #[test]
    #[ignore = "nightly: runs the sealed judgement cases, never on a PR (S-4.17)"]
    fn nightly_the_sealed_judgement_cases_score_beside_the_open_ones() {
        let cards = nightly_scorecards(&dir().join("tapes")).expect("sealed tapes read");
        assert_eq!(cards.len(), TIERS.len());
        for card in &cards {
            assert!(card.suites.iter().any(|s| s.sealed), "{}", card.profile_id);
            assert!(card.suites.iter().any(|s| !s.sealed));
            // A reference policy is not fitted to its evals; a fitted version is.
            assert_eq!(fitted_to_its_evals(card), None, "{}", card.profile_id);
        }
        let mut fitted = cards[0].clone();
        for s in fitted.suites.iter_mut().filter(|s| s.sealed) {
            s.passed = 0;
        }
        for s in fitted.suites.iter_mut().filter(|s| !s.sealed) {
            s.passed = s.n;
        }
        assert!(fitted_to_its_evals(&fitted).is_some());
    }

    #[test]
    #[ignore = "nightly: the sealed tape files are current with the policy that recorded them"]
    fn nightly_the_sealed_tapes_match_the_policy_that_recorded_them() {
        for ((tier, sealed), tape) in record_tapes() {
            if !sealed {
                continue;
            }
            let path = dir().join("tapes").join(format!("{tier}.sealed.json"));
            let on_disk: TierTape =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(on_disk, tape, "{} is stale", path.display());
        }
    }

    // ── The gate, and the tapes' own integrity ──────────────────────────────

    #[test]
    fn the_judgement_scorecard_a_pr_produces_does_not_regress_against_the_baseline() {
        let v = gate(&baseline(), &pr_scorecards());
        assert!(!v.blocked, "{}", v.summary());
        assert!(v.comparisons > 0, "a gate that compared nothing passed nothing");
    }

    #[test]
    fn a_newly_failing_judgement_case_blocks_the_gate() {
        let mut cand = pr_scorecards();
        let card = cand.iter_mut().find(|c| c.tier == "frontier").unwrap();
        let cell = card
            .suites
            .iter_mut()
            .find(|s| s.suite == cell_name(Workflow::Investigate))
            .unwrap();
        cell.failed_cases.push("judgement/a_case_that_just_started_failing".into());
        cell.passed = cell.passed.saturating_sub(1);
        let v = gate(&baseline(), &cand);
        assert!(v.blocked, "{}", v.summary());
    }

    #[test]
    fn the_committed_open_tapes_are_what_the_policies_record() {
        let fresh = record_tapes();
        for tier in TIERS {
            let want = fresh.get(&(tier.as_str().to_string(), false)).unwrap();
            assert_eq!(
                &open_tape(tier),
                want,
                "evals/judgement/tapes/{}.open.json is stale: run \
                 `cargo test -p harness --lib -- --ignored write_the_judgement_files`",
                tier.as_str()
            );
        }
    }

    #[test]
    fn the_committed_baseline_is_what_the_open_tapes_score() {
        assert_eq!(baseline(), pr_scorecards(), "regenerate the baseline deliberately");
    }

    #[test]
    fn the_policies_differ_where_the_tiers_are_supposed_to_and_all_are_labelled_synthetic() {
        for tier in TIERS {
            assert_eq!(open_tape(tier).provenance, "synthetic_reference_policy");
            assert!(policy(tier).name.starts_with("reference-"));
        }
        // The dominance and leak cases separate a tier that honours the
        // question's shape from one that does not.
        let f = pr_scorecards();
        let small = f.iter().find(|c| c.tier == "local_small").unwrap();
        assert!(
            small
                .suites
                .iter()
                .any(|s| s.attribution.contains_key("chose_the_workflow")),
            "a policy that always investigates must fail `chose_the_workflow` somewhere"
        );
    }

    /// The answer key stays off disk: neither the seed nor a planted mechanism
    /// appears in anything under `evals/judgement/`.
    #[test]
    fn nothing_under_evals_judgement_carries_the_seed_or_a_planted_mechanism() {
        fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(p).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(&dir(), &mut files);
        assert!(files.len() >= 11, "{} files", files.len());
        let seed_dec = CORPUS_SEED.to_string();
        let seed_hex = format!("{CORPUS_SEED:x}");
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap().to_lowercase();
            for needle in [
                seed_dec.as_str(),
                seed_hex.as_str(),
                "knownnull",
                "known_null",
                "answer_key",
                "leaking_feature",
                "\"rho\"",
                "planted_winner",
            ] {
                assert!(!text.contains(needle), "{} contains {needle:?}", f.display());
            }
        }
    }

    #[test]
    fn a_recorded_run_round_trips_through_json() {
        for tape in record_tapes().values() {
            let s = serde_json::to_string(tape).unwrap();
            let back: TierTape = serde_json::from_str(&s).unwrap();
            assert_eq!(&back, tape);
        }
    }

    /// Regenerate everything under `evals/judgement/` and the baseline.
    /// Deliberate, never automatic.
    #[test]
    #[ignore = "writes evals/judgement/** and evals/results/judgement-baseline.json; run deliberately"]
    fn write_the_judgement_files() {
        let root = dir();
        let tapes = root.join("tapes");
        std::fs::create_dir_all(&tapes).unwrap();
        let pretty = |v: &dyn erased::Ser| -> String {
            let mut t = v.render();
            t.push('\n');
            t
        };
        for ((tier, sealed), tape) in record_tapes() {
            let name = format!("{tier}.{}.json", if sealed { "sealed" } else { "open" });
            std::fs::write(tapes.join(name), pretty(&tape)).unwrap();
        }
        let ids = holdout();
        let manifest = serde_json::json!({
            "suite": SUITE,
            "spec": "AGENT-009 section 3; Set S S-10.16",
            "provenance": "synthetic_reference_policy: these tapes were NOT recorded from a model; the live run is gate H-2",
            "cases_per_tier": case_ids().len(),
            "generated_cases": CORPUS_SIZE,
            "dead_idea_cases": dead_ideas().len(),
            "sealed_cases": ids.sealed.len(),
            "open_cases_per_tier": case_ids().len() - ids.sealed.len(),
            "holdout": { "salt": ids.draw.salt, "requested_by": ids.draw.requested_by,
                         "drawn_on": ids.draw.drawn_on, "corpus_size": ids.draw.corpus_size },
            "tiers": TIERS.iter().map(|t| serde_json::json!({
                "tier": t.as_str(), "policy": policy(*t).name,
                "open_tape": format!("tapes/{}.open.json", t.as_str()),
                "sealed_tape": format!("tapes/{}.sealed.json", t.as_str()),
            })).collect::<Vec<_>>(),
            "scorecard_cells": "judgement/<workflow>, one replay suite per workflow per tier card",
            "truth_location": "not on disk: the corpus seed is a const in crates/harness/src/eval/judgement_suite.rs (scorer side, evals.truth)",
        });
        std::fs::write(root.join("manifest.json"), pretty(&manifest)).unwrap();
        std::fs::write(root.join("dead_ideas.json"), pretty(&dead_ideas())).unwrap();
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../evals/results/judgement-baseline.json"),
            pretty(&pr_scorecards_from_fresh()),
        )
        .unwrap();
    }

    fn pr_scorecards_from_fresh() -> Vec<Scorecard> {
        let fresh = record_tapes();
        TIERS
            .iter()
            .map(|t| {
                let tape = fresh.get(&(t.as_str().to_string(), false)).unwrap();
                scorecard(*t, &score_tape(tape, true).unwrap(), None)
            })
            .collect()
    }

    mod erased {
        pub trait Ser {
            fn render(&self) -> String;
        }
        impl<T: serde::Serialize> Ser for T {
            fn render(&self) -> String {
                serde_json::to_string_pretty(self).unwrap()
            }
        }
    }
}
