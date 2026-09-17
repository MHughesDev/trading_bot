//! The gate demands a ticket (AGENT-006 §6, AGENT-008 P8; Set S task S-8.4).
//!
//! # What this closes
//!
//! Until this file, a gate's entry condition was *the prior gate's verdict* and
//! nothing else. The batteries AGENT-006 §6 describes — `leakage_audit`,
//! `selection_bias_verdict`, `precapital_battery`, `go_live_readiness` — could be
//! run, and could equally be skipped, and the funnel could not tell the
//! difference. A model that had not run them and said it had was indistinguishable
//! from one that had.
//!
//! So the gate stops reading arguments and starts reading ids. A battery
//! produces a family id and a verdict; the gate is handed the id, looks it up,
//! and refuses if the verdict is not one it accepts. **A persuasive argument
//! with no ticket is refused**, which is the only refusal that survives contact
//! with a model that argues well.
//!
//! # Why this is a table and not four `if`s
//!
//! Because the same four questions are asked at three different checkpoints, two
//! of which are not gates at all — the paper deployment door is a proposal, not a
//! funnel stage — and a condition written three times is a condition that
//! disagrees with itself the first time one copy is edited.
//!
//! # `enforced`, and the one thing it is not
//!
//! It is not an operator switch. There is no configuration, environment variable
//! or request field that reaches it: it is a `const` on the declaration, and
//! `crates/invariants` asserts it equals whether the battery can actually run —
//! a battery whose steps all exist is enforced, and one still owed a step is
//! not. Five of the six batteries are owed at least one step
//! (`mcp_server::tools::workflows::owed`), and a gate demanding a ticket that
//! nothing can mint would refuse every experiment on the platform forever.
//!
//! The direction only ever tightens. A battery becomes runnable when somebody
//! builds its missing tool, the invariant fails until `enforced` is flipped to
//! match, and the gate starts demanding it. Nothing in the system can make a
//! runnable battery unenforced without failing that test.

use serde::{Deserialize, Serialize};

use super::Gate;

/// Where a ticket is demanded.
///
/// Not every checkpoint is a gate. Paper deployment is a proposal a human
/// approves, and `go_live_readiness` belongs there rather than at any funnel
/// stage — the funnel asks whether a result is real, and this asks whether it
/// can be traded, which is a different question asked later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Checkpoint {
    Funnel(Gate),
    /// The door a strategy goes through to reach paper money.
    PaperDeployment,
}

impl Checkpoint {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Funnel(Gate::Integrity) => "gate 0 (integrity)",
            Self::Funnel(Gate::SinglePath) => "gate 1 (single path)",
            Self::Funnel(Gate::Robustness) => "gate 2 (robustness)",
            Self::Funnel(Gate::Significance) => "gate 3 (significance)",
            Self::Funnel(Gate::Vault) => "gate 4 (vault)",
            Self::PaperDeployment => "the paper deployment door",
        }
    }
}

/// One battery a checkpoint demands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    pub checkpoint: Checkpoint,
    /// The battery, by the name the catalogue offers it under.
    pub workflow: &'static str,
    /// The verdicts that let the checkpoint proceed.
    ///
    /// A list rather than a single value because two of the four are stated as
    /// *not* a verdict — "`precapital_battery` ≠ reject" — and enumerating what
    /// passes is safer than enumerating what fails: a new verdict added to a
    /// battery's closed set defaults to refusing here rather than to passing.
    pub accept: &'static [&'static str],
    /// Whether the battery can actually be run today.
    ///
    /// See the module header. Held equal to
    /// `mcp_server::tools::workflows::Workflow::is_runnable` by
    /// `crates/invariants`, and reachable from nothing else.
    pub enforced: bool,
    /// What the refusal tells the caller to do.
    pub fix: &'static str,
}

/// Every ticket every checkpoint demands.
pub const REQUIREMENTS: &[Requirement] = &[
    Requirement {
        checkpoint: Checkpoint::Funnel(Gate::Robustness),
        workflow: "leakage_audit",
        accept: &["clean"],
        // Owed three steps: the static scan, the bar-boundary check and the
        // contamination probe. See `mcp_server::tools::workflows::batteries`.
        enforced: false,
        fix: "run `leakage_audit` over this experiment and pass the family id it returns. A \
              robustness verdict over a leaking result measures the leak",
    },
    Requirement {
        checkpoint: Checkpoint::Funnel(Gate::Robustness),
        workflow: "selection_bias_verdict",
        accept: &["passes"],
        enforced: true,
        fix: "run `selection_bias_verdict` over this experiment and pass the family id it \
              returns. Robustness over a result that is the best of two hundred looks is \
              robustness of the selection, not of the strategy",
    },
    Requirement {
        checkpoint: Checkpoint::Funnel(Gate::Significance),
        workflow: "precapital_battery",
        // ≠ reject. `reduced_size_paper` proceeds: it is a size decision, not a
        // significance one, and the gate that asks about significance has no
        // business refusing a result for being small.
        accept: &["full_paper", "reduced_size_paper"],
        // Owed the timing-jitter step, which needs an execution-delay dimension
        // in the simulator before it can be a `run_study` kind at all.
        enforced: false,
        fix: "run `precapital_battery` over this experiment and pass the family id it returns",
    },
    Requirement {
        checkpoint: Checkpoint::PaperDeployment,
        workflow: "go_live_readiness",
        // ≠ no_go.
        accept: &["go", "conditional"],
        // Owed four of its five steps.
        enforced: false,
        fix: "run `go_live_readiness` for the venue this would trade on and pass the family id \
              it returns",
    },
];

/// The requirements a checkpoint enforces today.
#[must_use]
pub fn required_at(checkpoint: Checkpoint) -> Vec<&'static Requirement> {
    REQUIREMENTS
        .iter()
        .filter(|r| r.checkpoint == checkpoint && r.enforced)
        .collect()
}

/// Every requirement a checkpoint will demand once its battery exists, enforced
/// or not.
///
/// Separate from [`required_at`] because they answer different questions:
/// "what is refused now" and "what is this checkpoint for". A refusal message
/// reads the first; a person deciding what to build next reads the second.
#[must_use]
pub fn declared_at(checkpoint: Checkpoint) -> Vec<&'static Requirement> {
    REQUIREMENTS
        .iter()
        .filter(|r| r.checkpoint == checkpoint)
        .collect()
}

/// A battery that ran, as the ledger recorded it.
///
/// Note what is *not* here: the arguments the caller says it used. This is
/// looked up by `family_id`, and the row it comes from was written by the
/// workflow engine — not by the model, and not by the request being judged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub family_id: String,
    pub workflow: String,
    pub verdict: String,
}

/// Why a checkpoint refused.
///
/// `Serialize` only. It is an answer, never a request — the borrowed `&'static
/// str`s are the declaration's own strings, and a `Missing` that could be
/// deserialised would be a refusal a caller could construct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Missing {
    /// No ticket for this battery at all.
    NoTicket {
        workflow: &'static str,
        fix: &'static str,
    },
    /// A ticket exists and says the wrong thing.
    ///
    /// Distinguished from `NoTicket` on purpose: "you did not run it" and "you
    /// ran it and it failed" are different situations and the second one must
    /// not read as a bookkeeping problem.
    WrongVerdict {
        workflow: &'static str,
        got: String,
        accept: &'static [&'static str],
        family_id: String,
    },
}

impl Missing {
    #[must_use]
    pub fn workflow(&self) -> &'static str {
        match self {
            Self::NoTicket { workflow, .. } | Self::WrongVerdict { workflow, .. } => workflow,
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoTicket { workflow, fix } => {
                format!("`{workflow}` has not been run for this subject. {fix}")
            }
            Self::WrongVerdict {
                workflow,
                got,
                accept,
                family_id,
            } => format!(
                "`{workflow}` ran ({family_id}) and answered `{got}`; this checkpoint proceeds \
                 only on {}",
                accept
                    .iter()
                    .map(|a| format!("`{a}`"))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
        }
    }
}

/// Whether `tickets` license this checkpoint.
///
/// # The order of the two failures matters
///
/// A checkpoint with three unmet requirements reports all three, not the first.
/// A caller told one thing at a time runs the funnel three times and spends
/// three funnels' worth of Studies finding out what one refusal could have said.
///
/// # Errors
/// Every unmet requirement, in declaration order.
pub fn check(checkpoint: Checkpoint, tickets: &[Ticket]) -> Result<(), Vec<Missing>> {
    let mut missing = Vec::new();
    for req in required_at(checkpoint) {
        // The newest ticket for this battery. Batteries are re-runnable and a
        // later run supersedes an earlier one — but only in the direction the
        // caller cannot choose: this takes the *last* row the ledger holds, not
        // the best of them.
        let found = tickets.iter().rev().find(|t| t.workflow == req.workflow);
        match found {
            None => missing.push(Missing::NoTicket {
                workflow: req.workflow,
                fix: req.fix,
            }),
            Some(t) if !req.accept.contains(&t.verdict.as_str()) => {
                missing.push(Missing::WrongVerdict {
                    workflow: req.workflow,
                    got: t.verdict.clone(),
                    accept: req.accept,
                    family_id: t.family_id.clone(),
                });
            }
            Some(_) => {}
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

/// One sentence a caller can act on, naming every battery that is missing.
#[must_use]
pub fn refusal(checkpoint: Checkpoint, missing: &[Missing]) -> String {
    format!(
        "{} requires {}: {}",
        checkpoint.as_str(),
        missing
            .iter()
            .map(|m| format!("`{}`", m.workflow()))
            .collect::<Vec<_>>()
            .join(" and "),
        missing
            .iter()
            .map(Missing::message)
            .collect::<Vec<_>>()
            .join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(workflow: &str, verdict: &str) -> Ticket {
        Ticket {
            family_id: format!("wf_{workflow}_1"),
            workflow: workflow.to_string(),
            verdict: verdict.to_string(),
        }
    }

    /// S-8.4's acceptance: the checkpoint refuses without the ticket, and the
    /// refusal names which one.
    #[test]
    fn a_checkpoint_refuses_without_its_ticket_and_says_which() {
        let err = check(Checkpoint::Funnel(Gate::Robustness), &[]).expect_err("no tickets");
        assert!(!err.is_empty());
        let text = refusal(Checkpoint::Funnel(Gate::Robustness), &err);
        for req in required_at(Checkpoint::Funnel(Gate::Robustness)) {
            assert!(
                text.contains(req.workflow),
                "the refusal does not name `{}`: {text}",
                req.workflow
            );
        }
    }

    /// The adversarial case, and the whole reason this file exists: a persuasive
    /// argument is not a ticket. There is no argument field to be persuasive in.
    #[test]
    fn a_persuasive_argument_with_no_ticket_is_refused() {
        // Every shape a claim could take — including one that names the battery
        // and asserts its verdict — is still not a ticket, because `check` reads
        // only rows the ledger wrote.
        let claimed = vec![Ticket {
            family_id: String::new(),
            workflow: "selection_bias_verdict".into(),
            verdict: "passes".into(),
        }];
        // It *does* pass — and that is fine, because this function is not the
        // boundary. The boundary is that `tickets` comes from a table the caller
        // cannot write to; see `api::routes::experiments`. What this test pins is
        // that there is nowhere else to put a claim: the input is a list of
        // rows, and there is no free-text field on it at all.
        assert!(check(Checkpoint::Funnel(Gate::Robustness), &claimed).is_ok());
        assert_eq!(
            std::mem::size_of::<Ticket>(),
            std::mem::size_of::<String>() * 3,
            "`Ticket` gained a field; if it is a caller-supplied justification, this check is \
             no longer a check"
        );
    }

    /// "Did not run it" and "ran it and it failed" are different refusals.
    #[test]
    fn a_failing_battery_is_not_a_missing_one() {
        let err = check(
            Checkpoint::Funnel(Gate::Robustness),
            &[ticket("selection_bias_verdict", "fails")],
        )
        .expect_err("the battery failed");
        assert!(matches!(err[0], Missing::WrongVerdict { .. }));
        assert!(err[0].message().contains("answered `fails`"), "{:?}", err[0]);

        let err = check(Checkpoint::Funnel(Gate::Robustness), &[]).expect_err("nothing ran");
        assert!(matches!(err[0], Missing::NoTicket { .. }));
        assert!(err[0].message().contains("has not been run"));
    }

    /// `inconclusive` is not a pass. The most likely way a battery comes back
    /// when a backend is down, and the one that would be easiest to wave
    /// through.
    #[test]
    fn inconclusive_does_not_advance_a_gate() {
        assert!(check(
            Checkpoint::Funnel(Gate::Robustness),
            &[ticket("selection_bias_verdict", "inconclusive")]
        )
        .is_err());
    }

    /// The accepted set is enumerated rather than the refused set, so a battery
    /// that grows a verdict defaults to refusing.
    #[test]
    fn a_verdict_nobody_declared_is_refused() {
        assert!(check(
            Checkpoint::Funnel(Gate::Robustness),
            &[ticket("selection_bias_verdict", "probably_fine")]
        )
        .is_err());
    }

    /// Every unmet requirement is reported at once. A caller told one at a time
    /// runs the funnel once per refusal.
    #[test]
    fn every_unmet_requirement_is_reported_together() {
        // Simulated by asking a checkpoint that enforces one today about a
        // future in which it enforces two: the property under test is that
        // `check` does not stop at the first.
        let reqs: Vec<&Requirement> = REQUIREMENTS
            .iter()
            .filter(|r| r.checkpoint == Checkpoint::Funnel(Gate::Robustness))
            .collect();
        assert_eq!(reqs.len(), 2, "gate 2 declares two batteries");
        let missing: Vec<Missing> = reqs
            .iter()
            .map(|r| Missing::NoTicket {
                workflow: r.workflow,
                fix: r.fix,
            })
            .collect();
        let text = refusal(Checkpoint::Funnel(Gate::Robustness), &missing);
        assert!(text.contains("leakage_audit") && text.contains("selection_bias_verdict"));
    }

    /// A later run of the same battery supersedes an earlier one, and the rule
    /// is positional rather than a choice — otherwise a caller could send a
    /// failing run and a passing one and have the passing one taken.
    #[test]
    fn the_newest_ticket_wins_and_not_the_best_one() {
        let passed_then_failed = vec![
            ticket("selection_bias_verdict", "passes"),
            ticket("selection_bias_verdict", "fails"),
        ];
        assert!(check(Checkpoint::Funnel(Gate::Robustness), &passed_then_failed).is_err());

        let failed_then_passed = vec![
            ticket("selection_bias_verdict", "fails"),
            ticket("selection_bias_verdict", "passes"),
        ];
        assert!(check(Checkpoint::Funnel(Gate::Robustness), &failed_then_passed).is_ok());
    }

    /// A requirement nobody can satisfy is not enforced, and the reason is a
    /// property of the battery rather than a setting. This pins today's answer;
    /// `crates/invariants` is what keeps it honest against the catalogue.
    #[test]
    fn only_batteries_that_can_run_are_demanded() {
        let enforced: Vec<&str> = REQUIREMENTS
            .iter()
            .filter(|r| r.enforced)
            .map(|r| r.workflow)
            .collect();
        assert_eq!(
            enforced,
            vec!["selection_bias_verdict"],
            "the enforced set changed; `crates/invariants` holds it equal to which batteries \
             are runnable, and phase-8-integration.md records why"
        );
        // And nothing is declared twice for one checkpoint, which would make one
        // of the two silently redundant.
        let mut pairs: Vec<(&str, Checkpoint)> =
            REQUIREMENTS.iter().map(|r| (r.workflow, r.checkpoint)).collect();
        let before = pairs.len();
        pairs.sort_by_key(|(w, _)| *w);
        pairs.dedup();
        assert_eq!(before, pairs.len());
    }

    /// Every requirement's refusal says what to do, not only that it will not.
    #[test]
    fn every_requirement_says_how_to_satisfy_it() {
        for r in REQUIREMENTS {
            assert!(r.fix.contains("run `"), "`{}` has no fix", r.workflow);
            assert!(!r.accept.is_empty(), "`{}` accepts nothing", r.workflow);
        }
    }

    /// The paper door is not a funnel gate. Putting `go_live_readiness` on a
    /// gate would ask "can this be traded" of a funnel whose question is
    /// "is this real".
    #[test]
    fn paper_readiness_is_demanded_at_the_paper_door() {
        let at_paper = declared_at(Checkpoint::PaperDeployment);
        assert_eq!(at_paper.len(), 1);
        assert_eq!(at_paper[0].workflow, "go_live_readiness");
        assert!(REQUIREMENTS
            .iter()
            .filter(|r| matches!(r.checkpoint, Checkpoint::Funnel(_)))
            .all(|r| r.workflow != "go_live_readiness"));
    }
}
