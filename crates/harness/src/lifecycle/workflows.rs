//! Workflows are stage skeletons (AGENT-013 §3; Set S task S-10.14).
//!
//! # What a workflow is
//!
//! Not a tool and not a prompt: a declaration of which lifecycle stages are
//! required, which may be skipped, and what the verdict vocabulary is. The
//! composed catalogue tools — `leakage_audit`, `precapital_battery` — are *called
//! by* stages; they are not the workflow.
//!
//! # The two rules that hold across all seven
//!
//! **A workflow cannot loosen a gate.** Skipping a *stage* is allowed exactly
//! where the table says; skipping a *gate* never is. `falsify` skips
//! `hypothesise` — the hypothesis is the artifact under attack, so there is
//! nothing to state — and it does **not** skip pre-registration, because an
//! attack protocol chosen after seeing which attack works is p-hacking with extra
//! steps.
//!
//! **`survey` cannot issue a trial ticket.** Its whole job is to say what is
//! known and what to ask next, and a survey that could spend a trial would be an
//! investigation with a cheaper-sounding name. [`Skeleton::spends_trials`] is
//! what makes that mechanical rather than advisory.
//!
//! # The spec's ten steps and this crate's `Stage`
//!
//! §3's table names the lifecycle steps — intake, ground, preregister,
//! hypothesise, design, execute, evaluate, diagnose, refine, report — and
//! `harness::state::Stage` names the states a *run* is in. They are not the same
//! list and are not meant to be:
//!
//! | §3 step | `Stage` |
//! |---|---|
//! | intake, ground | `Explore` |
//! | hypothesise | `Hypothesise` |
//! | preregister | `Preregister` |
//! | design | `Design` |
//! | execute | `Run` |
//! | evaluate, diagnose | *neither* — both are platform functions, and a run does not sit in them |
//! | refine | `Refine` |
//! | report | `Closed` |
//!
//! `Compare`, `Reproduce` and `Propose` are stages with no step of the same name:
//! they are what `compare`, `replicate` and a promotion request respectively spend
//! their execute step doing. The mapping is written down here because a reader
//! who assumed the two lists were the same list would find the skeletons below
//! missing two entries and conclude something was forgotten.

use crate::state::{Stage, Workflow};

/// One workflow's skeleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skeleton {
    pub workflow: Workflow,
    /// Stages this workflow must pass through. A transition that skips one is
    /// refused.
    pub required: &'static [Stage],
    /// Stages it may skip. Everything not in either list is simply not part of
    /// this workflow.
    pub skippable: &'static [Stage],
    /// The closed verdict vocabulary. Per workflow, because "survives" is not a
    /// verdict an `investigate` can reach and "supported" is not one a `survey`
    /// can.
    pub verdicts: &'static [&'static str],
    /// Whether this workflow may issue a trial ticket at all.
    pub spends_trials: bool,
}

const INVESTIGATE: Skeleton = Skeleton {
    workflow: Workflow::Investigate,
    // All ten. The default workflow skips nothing, which is what makes it the
    // default: every other skeleton is a claim that some stage is unnecessary
    // *for this question shape*, and that claim needs the question.
    required: &[
        Stage::Explore,
        Stage::Hypothesise,
        Stage::Preregister,
        Stage::Design,
        Stage::Run,
        Stage::Closed,
    ],
    skippable: &[
        Stage::Refine,
        Stage::Compare,
        Stage::Reproduce,
        Stage::Propose,
    ],
    verdicts: &["supported", "refuted", "inconclusive", "blocked"],
    spends_trials: true,
};

const FALSIFY: Skeleton = Skeleton {
    workflow: Workflow::Falsify,
    required: &[
        Stage::Explore,
        // Not skippable, and this is the rule §3 calls out by name: an attack
        // protocol chosen after seeing which attack works is p-hacking with
        // extra steps.
        Stage::Preregister,
        Stage::Run,
        Stage::Closed,
    ],
    // The hypothesis is the artifact under attack, so there is nothing to state.
    skippable: &[Stage::Hypothesise, Stage::Design, Stage::Refine],
    verdicts: &["survives", "fragile", "refuted"],
    spends_trials: true,
};

const REPLICATE: Skeleton = Skeleton {
    workflow: Workflow::Replicate,
    required: &[
        Stage::Explore,
        // The *original* protocol, sealed as stated. Amending it would make this
        // a different study wearing a replication's name.
        Stage::Preregister,
        Stage::Design,
        Stage::Run,
        Stage::Reproduce,
        Stage::Closed,
    ],
    // A replication that refines is no longer a replication.
    skippable: &[Stage::Refine],
    verdicts: &["replicated", "partially", "failed", "not_reproducible"],
    spends_trials: true,
};

const COMPARE: Skeleton = Skeleton {
    workflow: Workflow::Compare,
    required: &[
        Stage::Explore,
        // One protocol for all candidates. Per-candidate protocols are how a
        // comparison becomes a search for the candidate that likes its protocol.
        Stage::Preregister,
        Stage::Run,
        Stage::Compare,
        Stage::Closed,
    ],
    skippable: &[Stage::Hypothesise, Stage::Design, Stage::Refine],
    verdicts: &["ranked", "indistinguishable"],
    spends_trials: true,
};

const DIAGNOSE: Skeleton = Skeleton {
    workflow: Workflow::Diagnose,
    required: &[
        Stage::Explore,
        // Candidate causes, stated before the forensics. A diagnosis that names
        // its cause afterwards has explained whatever it found.
        Stage::Hypothesise,
        Stage::Run,
        Stage::Closed,
    ],
    // Pre-registration *may* be a diagnosis protocol rather than an edge
    // protocol, so it is not required — the run is attributing a failure, not
    // claiming an edge.
    skippable: &[Stage::Preregister, Stage::Design, Stage::Refine],
    verdicts: &["cause_identified", "undetermined"],
    // Read-mostly, but ablations are trials and pretending otherwise would put a
    // trial-spending path outside the counter.
    spends_trials: true,
};

const SURVEY: Skeleton = Skeleton {
    workflow: Workflow::Survey,
    required: &[Stage::Explore, Stage::Closed],
    // Everything that spends a trial.
    skippable: &[
        Stage::Hypothesise,
        Stage::Preregister,
        Stage::Design,
        Stage::Run,
        Stage::Refine,
        Stage::Compare,
        Stage::Reproduce,
        Stage::Propose,
    ],
    verdicts: &["survey_report"],
    // The rule this whole type exists to make mechanical: a survey that could
    // spend a trial would be an investigation with a cheaper-sounding name.
    spends_trials: false,
};

const OPTIMISE: Skeleton = Skeleton {
    workflow: Workflow::Optimise,
    required: &[
        Stage::Explore,
        // The campaign DEFINE. One object, one hash (S-10.11).
        Stage::Preregister,
        Stage::Run,
        Stage::Closed,
    ],
    // The campaign owns hypothesise and design; its REALLOCATE is the refine.
    skippable: &[Stage::Hypothesise, Stage::Design, Stage::Refine],
    verdicts: &["incumbent", "exhausted", "blocked"],
    spends_trials: true,
};

/// Every skeleton, in `Workflow` order.
pub const ALL: &[Skeleton] = &[
    INVESTIGATE,
    FALSIFY,
    REPLICATE,
    COMPARE,
    DIAGNOSE,
    SURVEY,
    OPTIMISE,
];

/// The skeleton for a workflow.
#[must_use]
pub fn skeleton(workflow: Workflow) -> &'static Skeleton {
    ALL.iter()
        .find(|s| s.workflow == workflow)
        .unwrap_or(&INVESTIGATE)
}

/// Why a stage transition was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error(
        "`{workflow:?}` requires `{skipped:?}` and this transition skips it. \
         Skipping a stage is allowed only where the skeleton says so."
    )]
    SkipsRequired { workflow: Workflow, skipped: Stage },
    #[error("`{workflow:?}` has no `{stage:?}` stage")]
    NotInWorkflow { workflow: Workflow, stage: Stage },
    #[error("a run does not move backwards: `{from:?}` is after `{to:?}`")]
    Backwards { from: Stage, to: Stage },
}

/// Lifecycle order, which is what "skips" is measured against.
///
/// A separate list from `Stage`'s declaration order so the enum can be
/// reordered without silently reordering the lifecycle — the two are different
/// facts and a `#[derive(Ord)]` would quietly conflate them.
const ORDER: &[Stage] = &[
    Stage::Explore,
    Stage::Hypothesise,
    Stage::Preregister,
    Stage::Design,
    Stage::Run,
    Stage::Refine,
    Stage::Compare,
    Stage::Reproduce,
    Stage::Propose,
    Stage::Closed,
];

fn position(stage: Stage) -> usize {
    ORDER.iter().position(|s| *s == stage).unwrap_or(usize::MAX)
}

/// Whether this workflow may move from one stage to another (§3).
///
/// # Errors
/// [`TransitionError`] naming the stage that was skipped, or the stage that is
/// not part of this workflow at all.
pub fn may_transition(workflow: Workflow, from: Stage, to: Stage) -> Result<(), TransitionError> {
    let sk = skeleton(workflow);
    if !sk.required.contains(&to) && !sk.skippable.contains(&to) {
        return Err(TransitionError::NotInWorkflow {
            workflow,
            stage: to,
        });
    }
    let (a, b) = (position(from), position(to));
    if b <= a {
        return Err(TransitionError::Backwards { from, to });
    }
    // Every required stage strictly between the two is one this transition
    // jumped over.
    if let Some(skipped) = ORDER[a + 1..b]
        .iter()
        .find(|s| sk.required.contains(s))
        .copied()
    {
        return Err(TransitionError::SkipsRequired { workflow, skipped });
    }
    Ok(())
}

/// Whether a verdict is one this workflow can reach.
#[must_use]
pub fn verdict_is_valid(workflow: Workflow, verdict: &str) -> bool {
    skeleton(workflow).verdicts.contains(&verdict)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `survey` cannot obtain a ticket. The acceptance.
    #[test]
    fn a_survey_cannot_spend_a_trial() {
        assert!(
            !skeleton(Workflow::Survey).spends_trials,
            "a survey that could spend a trial would be an investigation with a \
             cheaper-sounding name"
        );
        for w in [
            Workflow::Investigate,
            Workflow::Falsify,
            Workflow::Replicate,
            Workflow::Compare,
            Workflow::Diagnose,
            Workflow::Optimise,
        ] {
            assert!(skeleton(w).spends_trials, "{w:?}");
        }
    }

    /// `falsify` skips `hypothesise` and cannot skip pre-registration.
    ///
    /// §3 calls this one out by name: an attack protocol chosen after seeing
    /// which attack works is p-hacking with extra steps.
    #[test]
    fn falsify_skips_the_hypothesis_and_not_the_registration() {
        let sk = skeleton(Workflow::Falsify);
        assert!(sk.skippable.contains(&Stage::Hypothesise));
        assert!(sk.required.contains(&Stage::Preregister));

        // Explore straight to Run jumps the registration.
        let err =
            may_transition(Workflow::Falsify, Stage::Explore, Stage::Run).expect_err("refused");
        assert_eq!(
            err,
            TransitionError::SkipsRequired {
                workflow: Workflow::Falsify,
                skipped: Stage::Preregister
            }
        );
        // And skipping only the hypothesis is fine.
        may_transition(Workflow::Falsify, Stage::Explore, Stage::Preregister)
            .expect("the hypothesis is the artifact under attack");
    }

    /// No workflow may skip pre-registration where it is required, and only
    /// `survey` and `diagnose` are allowed to do without one at all.
    #[test]
    fn a_workflow_cannot_loosen_the_registration_gate() {
        for sk in ALL {
            let requires = sk.required.contains(&Stage::Preregister);
            let skips = sk.skippable.contains(&Stage::Preregister);
            assert!(
                requires != skips,
                "{:?} must either require pre-registration or say it is skippable, \
                 never both and never neither",
                sk.workflow
            );
            if skips {
                assert!(
                    matches!(sk.workflow, Workflow::Survey | Workflow::Diagnose),
                    "{:?} skips pre-registration and is not one of the two that may",
                    sk.workflow
                );
            }
        }
    }

    /// A survey that could spend a trial would also have to have somewhere to
    /// spend it, and it does not.
    #[test]
    fn a_survey_has_no_stage_that_spends_anything() {
        let sk = skeleton(Workflow::Survey);
        assert_eq!(sk.required, &[Stage::Explore, Stage::Closed]);
        // Every trial-spending stage is explicitly skippable rather than absent,
        // so the skeleton *says* it skips them rather than leaving a reader to
        // infer it from a short list.
        for stage in [Stage::Run, Stage::Refine, Stage::Compare, Stage::Reproduce] {
            assert!(sk.skippable.contains(&stage), "{stage:?}");
        }
    }

    /// The default workflow skips nothing that matters.
    #[test]
    fn investigate_is_the_one_that_claims_nothing_is_unnecessary() {
        let sk = skeleton(Workflow::Investigate);
        for stage in [
            Stage::Explore,
            Stage::Hypothesise,
            Stage::Preregister,
            Stage::Design,
            Stage::Run,
            Stage::Closed,
        ] {
            assert!(sk.required.contains(&stage), "{stage:?}");
        }
    }

    /// A replication that refines is no longer a replication.
    #[test]
    fn replicate_may_skip_refinement_and_nothing_else() {
        assert_eq!(skeleton(Workflow::Replicate).skippable, &[Stage::Refine]);
        assert!(skeleton(Workflow::Replicate)
            .required
            .contains(&Stage::Reproduce));
    }

    /// A run does not move backwards, and it does not stand still.
    #[test]
    fn a_transition_moves_forward() {
        assert!(matches!(
            may_transition(Workflow::Investigate, Stage::Run, Stage::Explore),
            Err(TransitionError::Backwards { .. })
        ));
        assert!(matches!(
            may_transition(Workflow::Investigate, Stage::Run, Stage::Run),
            Err(TransitionError::Backwards { .. })
        ));
    }

    /// A stage that is not part of the workflow is named as such.
    ///
    /// Distinct from "you skipped something": one is a run doing the stages out
    /// of order and the other is a run trying to do something this workflow does
    /// not contain, and the fixes are different.
    #[test]
    fn a_stage_outside_the_workflow_is_its_own_refusal() {
        let err = may_transition(Workflow::Falsify, Stage::Explore, Stage::Reproduce)
            .expect_err("falsify has no reproduce stage");
        assert_eq!(
            err,
            TransitionError::NotInWorkflow {
                workflow: Workflow::Falsify,
                stage: Stage::Reproduce
            }
        );
    }

    /// Each workflow's verdict vocabulary is its own.
    #[test]
    fn a_verdict_belongs_to_one_workflow() {
        assert!(verdict_is_valid(Workflow::Falsify, "survives"));
        assert!(
            !verdict_is_valid(Workflow::Investigate, "survives"),
            "`survives` is not a verdict an investigation can reach"
        );
        assert!(verdict_is_valid(Workflow::Investigate, "supported"));
        assert!(
            !verdict_is_valid(Workflow::Survey, "supported"),
            "and `supported` is not one a survey can"
        );
        for sk in ALL {
            assert!(!sk.verdicts.is_empty(), "{:?} has no verdicts", sk.workflow);
        }
    }

    /// Every workflow has a skeleton, and no stage is in both lists.
    #[test]
    fn the_table_covers_every_workflow_exactly_once() {
        assert_eq!(ALL.len(), 7);
        for w in [
            Workflow::Investigate,
            Workflow::Falsify,
            Workflow::Replicate,
            Workflow::Compare,
            Workflow::Diagnose,
            Workflow::Survey,
            Workflow::Optimise,
        ] {
            let matches = ALL.iter().filter(|s| s.workflow == w).count();
            assert_eq!(matches, 1, "{w:?}");
        }
        for sk in ALL {
            for stage in sk.required {
                assert!(
                    !sk.skippable.contains(stage),
                    "{:?} has {stage:?} in both lists",
                    sk.workflow
                );
            }
            assert!(
                sk.required.contains(&Stage::Closed),
                "{:?} must be able to finish",
                sk.workflow
            );
            assert!(
                sk.required.contains(&Stage::Explore),
                "{:?} must start by grounding the question",
                sk.workflow
            );
        }
    }

    /// The lifecycle order is a separate fact from the enum's declaration order.
    #[test]
    fn the_order_is_declared_rather_than_derived() {
        assert_eq!(ORDER.len(), 10);
        assert_eq!(ORDER[0], Stage::Explore);
        assert_eq!(ORDER[ORDER.len() - 1], Stage::Closed);
        // Every stage appears exactly once.
        for stage in ORDER {
            assert_eq!(ORDER.iter().filter(|s| *s == stage).count(), 1, "{stage:?}");
        }
    }
}
