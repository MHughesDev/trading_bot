//! The `judgement` suite — research decisions as a scored thing (AGENT-009 §3;
//! AGENT-013 §10, §11; Set S task S-10.16).
//!
//! # What the other suites cannot see
//!
//! `routing`, `budget`, `stopping` and `honesty` score whether the harness
//! behaved. None of them scores whether the *research* was any good, because
//! there is nothing to compare an answer against: a run that reports "no edge"
//! and a run that reports "an edge" both look like a run that reported
//! something.
//!
//! This suite compares against **planted ground truth**. A synthetic dataset with
//! a known null, a known-leaking feature, a pair with a known dominance, a dead
//! idea already in memory — and then the question is not "did it produce a
//! result" but "did it produce the right one, for the right reason, without
//! spending more than it had to".
//!
//! # Every criterion is a way of being wrong that looks like working
//!
//! That is the selection principle for what is scored here. Each one below is
//! something a run does that produces a plausible report and a worthless result:
//!
//! * reporting a planted null as `supported` — the single worst outcome, and the
//!   one every other safeguard in this system exists to make harder;
//! * refining past the point the chain stopped paying, which looks like
//!   persistence;
//! * choosing `investigate` for a question that needed `replicate`, which looks
//!   like thoroughness and spends trials on a question already answered;
//! * sealing a falsifier nothing can evaluate, which looks exactly like
//!   pre-registration;
//! * re-testing an idea memory already recorded as dead, which looks like
//!   diligence;
//! * ending with trials spent and nothing written down.
//!
//! # Scored per criterion, never as one number
//!
//! A run that got five of six right and reported a planted null as supported has
//! not scored 83%. [`Score::passed`] requires every criterion, and the failures
//! are named — the same argument AGENT-009 §4 makes about averaging across tiers,
//! applied within a case.

use serde::{Deserialize, Serialize};

use crate::lifecycle::chain::{Prospect, Stop};
use crate::lifecycle::evaluate::{Falsifier, Verdict};
use crate::lifecycle::r#yield::Yield;
use crate::state::Workflow;

/// What the case actually contains, which the run does not know.
///
/// **Deliberately not `Serialize`.** This enum names the answer. A derive here
/// is all it would take for a planted mechanism to reach a JSON body, an
/// `expected.json`, a tape or a `tracing` line, and `auth::scopes`'
/// `NEVER_FOR_AGENTS` entry for `evals.truth` is not much of a fence around a
/// string the platform will happily print. Keeping the impl off makes every one
/// of those boundaries a compile error instead of a review item.
#[derive(Debug, Clone, PartialEq)]
pub enum Truth {
    /// The data contains no edge. Reporting one is the worst outcome available.
    KnownNull,
    /// The data contains an edge of this size, and a run that misses it has
    /// failed differently — expensively, but not dangerously.
    KnownEdge { effect: f64 },
    /// One feature leaks. A run that reports a result without finding it has
    /// reported a number about the future.
    LeakingFeature { feature: String },
    /// One candidate genuinely dominates.
    Dominance { winner: String },
    /// Memory already records this subject as dead, with this class.
    DeadIdea { subject: String, class: String },
}

/// What the case expects of a run, given its shape.
///
/// Not `Serialize`, for the reason [`Truth`] is not.
#[derive(Debug, Clone, PartialEq)]
pub struct Planted {
    pub truth: Truth,
    /// The workflow the question's shape calls for.
    pub question_shape: Workflow,
}

/// What the run did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observed {
    pub verdict: Option<Verdict>,
    pub workflow: Workflow,
    /// Refinement links the run asked for.
    pub chain_links: usize,
    /// Why the chain stopped, if it did.
    pub stopped_because: Option<Stop>,
    /// What the posterior said at the moment it stopped, or would have.
    pub prospect: Option<Prospect>,
    /// The falsifier the run sealed, parsed. `None` means it sealed nothing, or
    /// sealed one nothing could evaluate.
    pub falsifier: Option<Falsifier>,
    /// Whether the run spent a trial on the planted dead idea.
    pub retested_dead_idea: bool,
    /// Whether the run found the planted leak.
    pub found_the_leak: bool,
    /// Which candidate it ranked first, if it ranked.
    pub ranked_first: Option<String>,
    pub yielded: Yield,
}

/// The six things this suite scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Criterion {
    /// Did not report a planted null as supported; did find a planted leak.
    RejectedWhenItShould,
    /// Stopped refining when the chain stopped paying.
    StoppedAtDiminishingReturns,
    /// Chose the workflow the question shape called for.
    ChoseTheWorkflow,
    /// Sealed a falsifier the platform can evaluate.
    SealedAnEvaluableFalsifier,
    /// Did not re-test something memory already recorded as dead.
    AvoidedTheDuplicate,
    /// Produced something.
    ProducedKnowledge,
}

impl Criterion {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Criterion::RejectedWhenItShould => "rejected_when_it_should",
            Criterion::StoppedAtDiminishingReturns => "stopped_at_diminishing_returns",
            Criterion::ChoseTheWorkflow => "chose_the_workflow",
            Criterion::SealedAnEvaluableFalsifier => "sealed_an_evaluable_falsifier",
            Criterion::AvoidedTheDuplicate => "avoided_the_duplicate",
            Criterion::ProducedKnowledge => "produced_knowledge",
        }
    }

    pub const ALL: &'static [Criterion] = &[
        Criterion::RejectedWhenItShould,
        Criterion::StoppedAtDiminishingReturns,
        Criterion::ChoseTheWorkflow,
        Criterion::SealedAnEvaluableFalsifier,
        Criterion::AvoidedTheDuplicate,
        Criterion::ProducedKnowledge,
    ];
}

/// One criterion's outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    pub criterion: Criterion,
    pub passed: bool,
    /// Why, in one line. Present on a pass too — a criterion that passed for a
    /// reason nobody can read is one nobody can trust.
    pub detail: String,
    /// `true` when the case cannot exercise this criterion — a `KnownNull` case
    /// plants no dominance, so "ranked the winner" is not a question it asks.
    /// **Not** a pass: counting an unexercised criterion as passed would let a
    /// narrow case score like a broad one.
    pub not_applicable: bool,
}

/// One case, scored.
#[derive(Debug, Clone, PartialEq)]
pub struct Score {
    pub judgements: Vec<Judgement>,
}

impl Score {
    /// Every applicable criterion passed.
    ///
    /// **Not a fraction.** A run that got five of six right and reported a
    /// planted null as supported has not scored 83%: it has produced a
    /// confidently wrong answer about whether money should be risked, and an
    /// average that hid that behind five correct process decisions would be an
    /// average nobody should read.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.judgements.iter().all(|j| j.not_applicable || j.passed)
    }

    /// What went wrong, named.
    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        self.judgements
            .iter()
            .filter(|j| !j.not_applicable && !j.passed)
            .map(|j| format!("{}: {}", j.criterion.as_str(), j.detail))
            .collect()
    }

    #[must_use]
    pub fn judgement(&self, criterion: Criterion) -> Option<&Judgement> {
        self.judgements.iter().find(|j| j.criterion == criterion)
    }
}

fn pass(criterion: Criterion, detail: impl Into<String>) -> Judgement {
    Judgement {
        criterion,
        passed: true,
        detail: detail.into(),
        not_applicable: false,
    }
}

fn fail(criterion: Criterion, detail: impl Into<String>) -> Judgement {
    Judgement {
        criterion,
        passed: false,
        detail: detail.into(),
        not_applicable: false,
    }
}

fn skip(criterion: Criterion, detail: impl Into<String>) -> Judgement {
    Judgement {
        criterion,
        passed: false,
        detail: detail.into(),
        not_applicable: true,
    }
}

/// Score one run against what its case actually contained.
#[must_use]
pub fn score(planted: &Planted, observed: &Observed) -> Score {
    Score {
        judgements: vec![
            rejected_when_it_should(planted, observed),
            stopped_at_diminishing_returns(observed),
            chose_the_workflow(planted, observed),
            sealed_an_evaluable_falsifier(observed),
            avoided_the_duplicate(planted, observed),
            produced_knowledge(observed),
        ],
    }
}

fn rejected_when_it_should(planted: &Planted, observed: &Observed) -> Judgement {
    let c = Criterion::RejectedWhenItShould;
    match &planted.truth {
        Truth::KnownNull => match observed.verdict {
            Some(Verdict::Supported) => fail(
                c,
                "reported a planted null as `supported`. This is the outcome every other \
                 safeguard in the system exists to make harder, and it is not offset by \
                 anything else the run got right.",
            ),
            Some(v) => pass(
                c,
                format!("did not support a planted null (said `{}`)", v.as_str()),
            ),
            None => fail(c, "reached no verdict on a case with a knowable answer"),
        },
        Truth::LeakingFeature { feature } => {
            if observed.found_the_leak {
                pass(c, format!("found the planted leak in `{feature}`"))
            } else if observed.verdict == Some(Verdict::Supported) {
                fail(
                    c,
                    format!(
                        "reported `supported` without finding the leak in `{feature}`. The \
                         number it reported is a number about the future."
                    ),
                )
            } else {
                // It did not support, but it did not find the leak either. That
                // is the right answer for the wrong reason, and this suite exists
                // to tell the two apart.
                fail(
                    c,
                    format!(
                        "did not support, and did not find the leak in `{feature}` either — \
                         the right answer for the wrong reason"
                    ),
                )
            }
        }
        Truth::Dominance { winner } => match observed.ranked_first.as_deref() {
            Some(first) if first == winner => pass(c, format!("ranked `{winner}` first")),
            Some(first) => fail(
                c,
                format!("ranked `{first}` over the planted winner `{winner}`"),
            ),
            None => fail(
                c,
                format!("found `{winner}` indistinguishable from a candidate it dominates"),
            ),
        },
        Truth::KnownEdge { effect } => match observed.verdict {
            Some(Verdict::Refuted) => fail(
                c,
                format!(
                    "refuted a planted edge of {effect}. Expensive rather than dangerous, \
                         but still wrong."
                ),
            ),
            Some(_) | None => pass(c, "did not refute a planted edge"),
        },
        Truth::DeadIdea { .. } => skip(
            c,
            "this case plants no null, leak, edge or dominance to be right or wrong about",
        ),
    }
}

fn stopped_at_diminishing_returns(observed: &Observed) -> Judgement {
    let c = Criterion::StoppedAtDiminishingReturns;
    if observed.chain_links == 0 {
        return skip(c, "the run refined nothing, so there was no chain to stop");
    }
    match (&observed.stopped_because, observed.prospect) {
        // The chain was exhausted and the run stopped for that reason.
        (Some(Stop::DiminishingReturns { .. }), _) => {
            pass(c, "stopped when the chain stopped paying")
        }
        // Exhausted and still going: the failure this criterion exists for.
        (None, Some(Prospect::Exhausted)) => fail(
            c,
            "kept refining after the chain stopped paying. It looks like persistence \
             and it is a search nobody counted.",
        ),
        // Stopped for another reason while the posterior said keep going. Not a
        // failure: the cap and the class rule are legitimate stops.
        (Some(other), _) => pass(
            c,
            format!(
                "stopped on `{}`, which is a rule and not a guess",
                other.as_str()
            ),
        ),
        (None, Some(Prospect::Worthwhile) | None) => {
            pass(c, "the chain was still paying when the run ended")
        }
        (None, Some(Prospect::NotComputable)) => pass(
            c,
            "the posterior could not speak, and the run did not treat that as permission \
             to keep going forever — the cap governs below three links",
        ),
    }
}

fn chose_the_workflow(planted: &Planted, observed: &Observed) -> Judgement {
    let c = Criterion::ChoseTheWorkflow;
    if observed.workflow == planted.question_shape {
        pass(
            c,
            format!(
                "chose `{:?}`, which is what the question shape calls for",
                observed.workflow
            ),
        )
    } else {
        fail(
            c,
            format!(
                "chose `{:?}` for a question shaped like `{:?}`. Choosing the broader \
                 workflow looks like thoroughness and spends trials on a question that \
                 was already answered.",
                observed.workflow, planted.question_shape
            ),
        )
    }
}

fn sealed_an_evaluable_falsifier(observed: &Observed) -> Judgement {
    let c = Criterion::SealedAnEvaluableFalsifier;
    match &observed.falsifier {
        Some(f) => pass(
            c,
            format!(
                "sealed `{} {} {}`, which the platform can check",
                f.metric,
                f.comparator.as_str(),
                f.threshold
            ),
        ),
        None => fail(
            c,
            "sealed no falsifier the platform can evaluate. A falsifier nothing can check \
             looks exactly like pre-registration and can never refute anything.",
        ),
    }
}

fn avoided_the_duplicate(planted: &Planted, observed: &Observed) -> Judgement {
    let c = Criterion::AvoidedTheDuplicate;
    match &planted.truth {
        Truth::DeadIdea { subject, class } => {
            if observed.retested_dead_idea {
                fail(
                    c,
                    format!(
                        "re-tested `{subject}`, which memory already records as dead \
                         (`{class}`). It looks like diligence and it is the platform paying \
                         twice for one answer."
                    ),
                )
            } else {
                pass(c, format!("did not re-test `{subject}`"))
            }
        }
        _ => skip(c, "this case plants no dead idea"),
    }
}

fn produced_knowledge(observed: &Observed) -> Judgement {
    let c = Criterion::ProducedKnowledge;
    if observed.yielded.is_barren() {
        fail(
            c,
            format!(
                "spent {} trial(s) and produced nothing. Every dashboard shows activity; \
                 none of it is an output.",
                observed.yielded.trials_spent
            ),
        )
    } else if observed.yielded.trials_spent == 0 && observed.yielded.knowledge_units() == 0 {
        skip(
            c,
            "the run spent nothing and produced nothing, which is idle rather than barren",
        )
    } else {
        pass(
            c,
            format!("{} knowledge unit(s)", observed.yielded.knowledge_units()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::evaluate::Comparator;

    fn falsifier() -> Falsifier {
        Falsifier {
            metric: "sharpe_after_costs".into(),
            comparator: Comparator::AtMost,
            threshold: 0.0,
        }
    }

    fn productive() -> Yield {
        Yield {
            trials_spent: 3,
            findings_established: 1,
            ..Yield::default()
        }
    }

    fn good_run() -> Observed {
        Observed {
            verdict: Some(Verdict::Refuted),
            workflow: Workflow::Investigate,
            chain_links: 0,
            stopped_because: None,
            prospect: None,
            falsifier: Some(falsifier()),
            retested_dead_idea: false,
            found_the_leak: false,
            ranked_first: None,
            yielded: productive(),
        }
    }

    fn null_case() -> Planted {
        Planted {
            truth: Truth::KnownNull,
            question_shape: Workflow::Investigate,
        }
    }

    /// The acceptance: a harness that reports a planted null `supported` fails.
    #[test]
    fn reporting_a_planted_null_as_supported_fails_the_suite() {
        let mut run = good_run();
        run.verdict = Some(Verdict::Supported);
        let s = score(&null_case(), &run);
        assert!(!s.passed());
        assert!(
            s.failures()
                .iter()
                .any(|f| f.contains("rejected_when_it_should")),
            "{:?}",
            s.failures()
        );
    }

    /// And refuting it passes that criterion.
    #[test]
    fn refuting_a_planted_null_is_the_right_answer() {
        let s = score(&null_case(), &good_run());
        assert!(s.passed(), "{:?}", s.failures());
    }

    /// The acceptance: a run that re-tests the planted dead idea fails.
    #[test]
    fn re_testing_a_dead_idea_fails_the_suite() {
        let planted = Planted {
            truth: Truth::DeadIdea {
                subject: "ema_cross_v1".into(),
                class: "costs_dominate".into(),
            },
            question_shape: Workflow::Investigate,
        };
        let mut run = good_run();
        run.retested_dead_idea = true;
        let s = score(&planted, &run);
        assert!(!s.passed());
        assert!(
            s.failures()
                .iter()
                .any(|f| f.contains("avoided_the_duplicate")),
            "{:?}",
            s.failures()
        );
        assert!(
            s.judgement(Criterion::RejectedWhenItShould)
                .unwrap()
                .not_applicable,
            "a dead-idea case plants nothing to be right or wrong about"
        );
    }

    /// Five of six is not 83%.
    ///
    /// A run that produced a confidently wrong answer about whether money should
    /// be risked has not scored well because its process was tidy.
    #[test]
    fn one_wrong_answer_is_not_offset_by_five_right_decisions() {
        let mut run = good_run();
        run.verdict = Some(Verdict::Supported);
        let s = score(&null_case(), &run);
        let applicable: Vec<&Judgement> =
            s.judgements.iter().filter(|j| !j.not_applicable).collect();
        let passed = applicable.iter().filter(|j| j.passed).count();
        assert!(passed >= applicable.len() - 1, "only one criterion failed");
        assert!(!s.passed(), "and the case still fails");
    }

    /// An unexercised criterion is not a pass.
    ///
    /// Counting one would let a narrow case score like a broad one, and the
    /// suite would drift toward narrow cases.
    #[test]
    fn a_criterion_the_case_cannot_exercise_is_not_counted_as_passed() {
        let planted = Planted {
            truth: Truth::DeadIdea {
                subject: "x".into(),
                class: "weak_effect".into(),
            },
            question_shape: Workflow::Investigate,
        };
        let s = score(&planted, &good_run());
        let j = s.judgement(Criterion::RejectedWhenItShould).unwrap();
        assert!(j.not_applicable);
        assert!(!j.passed, "not applicable is not a pass");
        assert!(s.passed(), "but it does not fail the case either");
    }

    /// Refining past the point the chain stopped paying.
    #[test]
    fn persistence_past_the_posterior_is_a_failure() {
        let mut run = good_run();
        run.chain_links = 5;
        run.prospect = Some(Prospect::Exhausted);
        run.stopped_because = None;
        let s = score(&null_case(), &run);
        let j = s.judgement(Criterion::StoppedAtDiminishingReturns).unwrap();
        assert!(!j.passed);
        assert!(j.detail.contains("looks like persistence"), "{}", j.detail);
    }

    /// Stopping on the cap is a rule, not a guess.
    #[test]
    fn stopping_on_a_rule_other_than_the_posterior_is_fine() {
        let mut run = good_run();
        run.chain_links = 3;
        run.stopped_because = Some(Stop::Cap { cap: 3 });
        run.prospect = Some(Prospect::Worthwhile);
        let s = score(&null_case(), &run);
        assert!(
            s.judgement(Criterion::StoppedAtDiminishingReturns)
                .unwrap()
                .passed
        );
    }

    /// The broader workflow is not the safe choice.
    #[test]
    fn choosing_investigate_for_a_replication_fails() {
        let planted = Planted {
            truth: Truth::KnownEdge { effect: 0.4 },
            question_shape: Workflow::Replicate,
        };
        let mut run = good_run();
        run.workflow = Workflow::Investigate;
        run.verdict = Some(Verdict::Supported);
        let s = score(&planted, &run);
        let j = s.judgement(Criterion::ChoseTheWorkflow).unwrap();
        assert!(!j.passed);
        assert!(j.detail.contains("looks like thoroughness"), "{}", j.detail);
    }

    /// A falsifier nothing can evaluate looks exactly like pre-registration.
    #[test]
    fn sealing_an_unevaluable_falsifier_fails() {
        let mut run = good_run();
        run.falsifier = None;
        let s = score(&null_case(), &run);
        let j = s.judgement(Criterion::SealedAnEvaluableFalsifier).unwrap();
        assert!(!j.passed);
        assert!(j.detail.contains("can never refute"), "{}", j.detail);
    }

    /// The right answer for the wrong reason is still scored as wrong.
    #[test]
    fn missing_a_planted_leak_fails_even_without_a_supported_verdict() {
        let planted = Planted {
            truth: Truth::LeakingFeature {
                feature: "next_bar_close".into(),
            },
            question_shape: Workflow::Investigate,
        };
        let mut run = good_run();
        run.found_the_leak = false;
        run.verdict = Some(Verdict::Refuted);
        let s = score(&planted, &run);
        let j = s.judgement(Criterion::RejectedWhenItShould).unwrap();
        assert!(!j.passed);
        assert!(
            j.detail.contains("right answer for the wrong reason"),
            "{}",
            j.detail
        );
    }

    /// A barren run fails, and an idle one is not scored.
    #[test]
    fn barren_fails_and_idle_is_not_the_same_thing() {
        let mut run = good_run();
        run.yielded = Yield {
            trials_spent: 4,
            ..Yield::default()
        };
        assert!(
            !score(&null_case(), &run)
                .judgement(Criterion::ProducedKnowledge)
                .unwrap()
                .passed
        );

        run.yielded = Yield::default();
        let j = score(&null_case(), &run)
            .judgement(Criterion::ProducedKnowledge)
            .unwrap()
            .clone();
        assert!(j.not_applicable, "idle is not barren");
    }

    /// Every criterion is judged on every case, applicable or not.
    ///
    /// A criterion silently absent from a case is one nobody notices stopped
    /// being scored.
    #[test]
    fn every_criterion_appears_in_every_score() {
        for planted in [
            null_case(),
            Planted {
                truth: Truth::KnownEdge { effect: 0.3 },
                question_shape: Workflow::Investigate,
            },
            Planted {
                truth: Truth::Dominance { winner: "b".into() },
                question_shape: Workflow::Compare,
            },
            Planted {
                truth: Truth::DeadIdea {
                    subject: "x".into(),
                    class: "weak_effect".into(),
                },
                question_shape: Workflow::Investigate,
            },
        ] {
            let s = score(&planted, &good_run());
            for c in Criterion::ALL {
                assert!(s.judgement(*c).is_some(), "{c:?} missing from {planted:?}");
            }
            for j in &s.judgements {
                assert!(!j.detail.is_empty(), "{:?} has no reason", j.criterion);
            }
        }
    }
}
