//! The gate's requirement table and the batteries it names are two lists in two
//! crates that do not depend on each other (Set S task S-8.4).
//!
//! `backtest::gates::tickets::REQUIREMENTS` names four batteries by string and
//! says which verdicts each checkpoint accepts. `mcp_server::tools::workflows`
//! declares those batteries, their closed verdict sets, and whether each can
//! actually run. Neither crate can see the other — `backtest` is the core and
//! `mcp-server` is a thin HTTP client over the platform, and a dependency in
//! either direction would be wrong.
//!
//! So the agreement is asserted here. Every one of these failures is silent in
//! production if nobody checks it:
//!
//! * a requirement naming a battery that does not exist refuses a gate forever,
//!   with a message telling the caller to run a tool the catalogue does not
//!   offer;
//! * a requirement accepting a verdict the battery cannot produce is a gate that
//!   can never be passed;
//! * a requirement accepting a verdict that is not in the battery's closed set
//!   is a typo that reads as a rule;
//! * and `enforced` drifting from runnability is the one that matters most in
//!   both directions — enforcing an un-runnable battery bricks the funnel, and
//!   *not* enforcing a runnable one is a gate quietly letting evidence through
//!   that it claims to demand.

use backtest::gates::tickets::REQUIREMENTS;
use mcp_server_lib::tools::workflows;

fn battery(name: &str) -> &'static workflows::Workflow {
    workflows::all()
        .iter()
        .find(|w| w.name == name)
        .unwrap_or_else(|| {
            panic!(
                "`backtest::gates::tickets` demands `{name}`, which no battery declares. A gate \
                 demanding a ticket nothing can mint refuses every experiment on the platform, \
                 and the refusal names a tool that does not exist."
            )
        })
}

#[test]
fn every_demanded_battery_is_one_the_catalogue_declares() {
    for r in REQUIREMENTS {
        let _ = battery(r.workflow);
    }
}

#[test]
fn every_accepted_verdict_is_one_the_battery_can_reach() {
    for r in REQUIREMENTS {
        let w = battery(r.workflow);
        for accepted in r.accept {
            assert!(
                w.verdicts.contains(accepted),
                "gate requirement `{}` accepts `{accepted}`, which is not in that battery's \
                 closed set {:?} — so this checkpoint can never be passed",
                r.workflow,
                w.verdicts
            );
        }
        assert!(
            r.accept.len() < w.verdicts.len(),
            "`{}` accepts every verdict it can produce, which is a requirement that requires \
             nothing",
            r.workflow
        );
    }
}

/// The one that is dangerous in both directions.
#[test]
fn a_requirement_is_enforced_exactly_when_its_battery_can_run() {
    for r in REQUIREMENTS {
        let w = battery(r.workflow);
        assert_eq!(
            r.enforced,
            w.is_runnable(),
            "`{}` is enforced={} and runnable={}. {}",
            r.workflow,
            r.enforced,
            w.is_runnable(),
            if w.is_runnable() {
                "The battery can now be run, so the gate must demand it: set `enforced: true` \
                 in `backtest::gates::tickets::REQUIREMENTS`."
            } else {
                "The battery is still owed a step, so demanding it would refuse every \
                 experiment on the platform. Set `enforced: false`, or build the owed step."
            }
        );
    }
}

/// A battery that is demanded and cannot run says what it is waiting for, where
/// somebody will find it.
#[test]
fn every_unenforced_requirement_has_an_owed_step_that_names_what_it_needs() {
    for r in REQUIREMENTS.iter().filter(|r| !r.enforced) {
        let w = battery(r.workflow);
        let owed = w.owed();
        assert!(
            !owed.is_empty() || !w.missing_tools().is_empty(),
            "`{}` is not enforced and is owed nothing, so nothing explains why the gate does \
             not demand it",
            r.workflow
        );
        for (label, needs) in owed {
            assert!(
                !needs.trim().is_empty(),
                "`{}`/{label} does not say what it needs",
                r.workflow
            );
        }
    }
}

/// Every battery the catalogue offers is either demanded somewhere or is
/// deliberately not a gate condition.
///
/// Pinned rather than derived: a new battery appearing with no checkpoint is
/// exactly the "built but unreached" shape this repository keeps producing, and
/// a list that fails when it changes is what makes somebody decide.
#[test]
fn every_battery_is_either_demanded_or_deliberately_not() {
    const NOT_A_GATE_CONDITION: &[(&str, &str)] = &[
        (
            "hypothesis_precheck",
            "runs before a hypothesis is registered, which is before any gate exists to \
             demand it",
        ),
        (
            "signal_identity",
            "answers what a signal is, not whether a result is real. AGENT-006 §6 does not \
             make it a gate condition, and inventing one here would be this file deciding \
             policy",
        ),
    ];
    for w in workflows::all() {
        let demanded = REQUIREMENTS.iter().any(|r| r.workflow == w.name);
        let excused = NOT_A_GATE_CONDITION.iter().any(|(n, _)| *n == w.name);
        assert!(
            demanded ^ excused,
            "`{}` is demanded={demanded} and excused={excused}. A battery that is neither is a \
             battery nothing reads; a battery that is both has two answers.",
            w.name
        );
    }
}
