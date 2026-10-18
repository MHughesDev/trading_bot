//! Every decision `research-session` holds has a caller in the driver.
//!
//! # Why this exists
//!
//! `crates/research-session` reached 276 passing tests with five modules that
//! nothing in production ever called. `sources::ingest`, `couplings::evaluate`,
//! `charter::admit`, `replenish::on_drain` and the whole of `execute` were
//! built, tested and unreached: `api::session_driver::SessionWorker` called
//! `schedule`, `harvest`, `stall` and `digest`, and nothing else.
//!
//! That is this repository's dominant defect shape and it is invisible to every
//! other kind of test. The modules' own tests pass — they are correct. The
//! driver's tests pass — it does what it does. Nothing fails, and the control
//! does not exist.
//!
//! # What this checks, and what it deliberately does not
//!
//! Two things, and neither is "the module works" — the module's own 276 tests
//! already hold that:
//!
//! 1. **The entry point is called.** Doc comments are stripped first, because
//!    the previous shape of this bug was a driver whose *module header*
//!    described four modules it called and one it did not.
//! 2. **Every refusal reaches its own branch.** `Verdict::CannotDecide`,
//!    `Fired::NotEvaluable`, `LeaseRefusal` and each `HoldReason` exist because
//!    an absence is not a no; collapsing one into "continue" removes the
//!    control while leaving every test green. The variant lists below are built
//!    by an exhaustive `match`, so a new variant does not compile until
//!    somebody decides what the driver does with it.
//!
//! It does not check that the branch does the *right* thing. A branch that
//! reaches a distinct arm and then writes the wrong event is a bug this cannot
//! see; a branch that does not exist is the bug that actually happened.
//!
//! # The unwired list is the useful half
//!
//! [`UNWIRED`] is a module entry point that deliberately has no caller here,
//! beside the exact symbol that is missing. That makes a gap a reviewable claim
//! rather than a discovery, and closing one is how a phase closes.

use std::path::{Path, PathBuf};

use research_session::couplings::{PauseReason, Verdict};
use research_session::execute::{DrainAt, HoldReason, Step};
use research_session::harvest::SkipReason;
use research_session::replenish::Drain;
use research_session::sources::Fired;

/// The decisions that must have a caller, and the call that would be it.
///
/// Each row is `(what the module decides, the text of the call)`. The second is
/// matched against the driver's code with its comments stripped, so a mention
/// in prose does not count as a caller — which is the exact way the last
/// version of this gap hid.
const MUST_BE_CALLED: &[(&str, &str)] = &[
    (
        "§7 rows 8-10: the kill switch, the tier and the platform-defect breaker",
        "couplings::evaluate(",
    ),
    (
        "§7 rows 1-2 and 8-10: what the driver does this iteration",
        "execute::plan(",
    ),
    ("§6.3: the child's budget", "execute::lease("),
    ("§7 row 3: is the session finished", "execute::stop("),
    (
        "§7 row 2: where the session stands against its day budget",
        "execute::day_budget(",
    ),
    (
        "§2.1: the day budget as a bound on a lease in this unit",
        "execute::day_bound(",
    ),
    (
        "§2.1: the total budget as a bound on a lease in this unit",
        "execute::total_bound(",
    ),
    (
        // `ingest_for`, not `ingest`, and the distinction is the fix. `ingest`
        // derives the session from the agenda it is handed, so on a board with
        // nothing non-terminal it has no session to build items under — and the
        // driver used to skip all five subscriptions in exactly that case. That is
        // the iteration where they matter most: the loop answers
        // `NothingClaimable` rather than obeying it so the sources run *before*
        // `replenish::on_drain` decides, and a drained session under
        // `replenish = false` would otherwise complete for good over a `World`
        // nobody read. Asserting the named-session door is what keeps the guard
        // from coming back.
        "§4.3-4.7: the five subscriptions, on every iteration, including a drained board",
        "sources::ingest_for(",
    ),
    (
        "§4.9 and §7 rows 4-5: what a drained agenda means",
        "replenish::on_drain(",
    ),
    // Closed 2026-10-03 (S-11.3, S-11.4, S-11.5). Each was an `UNWIRED` row.
    (
        "§5.1: novelty from both inputs, the precheck factor beside the signal",
        "select::novelty_from(",
    ),
    (
        "P3: the scorecard of which layer caught each duplicate, on the digest",
        "DedupeScorecard::from_log(",
    ),
    (
        "§5.1: the selection policy's conformance reader, before anything is selected",
        "SelectionPolicy::current().nonconformances(",
    ),
    (
        "§6.3: the per-task cap in the unit a run is metered in",
        "execute::per_task_bound(",
    ),
    (
        "§12.1 / §12.4: one child, one lease, and the committed leases summed",
        "self.record_lease(",
    ),
    (
        "§12.4: the committed leases of running children, read back",
        "self.committed_leases(",
    ),
    (
        "§6.2: the child's brief — standing constraints, sibling verdicts, why now",
        "execute::brief(",
    ),
    // Closed 2026-10-03, wave 2, on RUNTIME's manager API.
    (
        "§6.3: the lease's trial count is the child's cap — it may not borrow",
        ".send_turn(",
    ),
    (
        "§7 rows 1, 8, 10: live children are stopped at a step boundary, not killed",
        ".drain_run(",
    ),
    (
        "§7 row 1: the next window resumes suspended children before claiming anything new",
        ".resume_drained(",
    ),
    (
        "§7 row 10: the breaker's pause files a `platform_defect` entry",
        ".entry_claim(",
    ),
    // Closed 2026-10-04 (Set S phase 11 finish).
    (
        "§4.9: a survey's proposals are absorbed into the agenda with their estimates",
        "replenish::absorb(",
    ),
    (
        "§8: a harvested proposal is an instance of a standing rule that always asks",
        "AlwaysAsks::for_proposal_kind(",
    ),
    (
        "§8: whether a human is asked, under this session's autonomy",
        ".asks(autonomy)",
    ),
    (
        "§4.7: regime flips from the regime record",
        "session_regime::flips(",
    ),
    (
        "§4.5: the unresolved contradiction pairs",
        "session_contradictions::open_pairs(",
    ),
    (
        "P3 / §12.2: two sessions, one question, one item",
        "session_dedupe::offer(",
    ),
    (
        "§7 rows 8-10: the driver wakes on the monitor's notification, not only on its poll",
        "Wake::open(",
    ),
    (
        "§7 row 9: a recorded degraded or withdrawn standing, or a removed credential, pauses",
        "session_profile::health_from(",
    ),
    (
        "§2.1 / §7 row 2: the day budget in trials, read from its own table",
        "session_day::trials_per_day(",
    ),
    (
        "§6.1: up to `max_concurrent_runs` children are awaited together",
        "tokio::join!(",
    ),
];

/// A decision with no caller here, and the symbol that would have to exist.
///
/// A row is a claim somebody can check. Removing one is how the gap closes;
/// adding one without naming the missing symbol is how this list stops meaning
/// anything.
const UNWIRED: &[(&str, &str)] = &[
    (
        "charter::admit",
        "Not this file's: its call site is session *creation*, and it is called \
         there - `routes::research_sessions::admit_at_creation`, with a \
         `charter::Profile` built from what the platform has recorded \
         (`session_profile::charter_profile`: `profile_standing` and \
         `profile_judgement`, 0163). `autonomous` is granted only on a recorded, \
         passing `judgement`. MISSING: a *producer* - nothing yet runs the \
         `judgement` suite and posts its scorecard to \
         `POST /api/research/profiles/{profile}/scorecard`; until it does every \
         profile is unmeasured and `autonomous` is refused (ADR-P11-04).",
    ),
    (
        "execute::redirect",
        "§6.4's steer is a human action on a live item and belongs on a route, \
         not in the loop: `routes::research_sessions::redirect_item` calls it and \
         `POST /api/research/sessions/{id}/agenda/{item}/redirect` is registered. \
         MISSING: a desk *Steer* control that posts to it (S-11.13).",
    ),
    (
        "couplings::refuses_ring",
        "AGENT-013 §6.8's ring-2 refusal is already enforced where a proposal \
         is created and decided: `routes::proposals` checks \
         `routes::lifecycle::kill_switch_tripped` before approving one. A \
         second copy in the driver would be two definitions of one rule. \
         MISSING: nothing — this is a decision, not a gap.",
    ),
];

fn driver_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session_driver.rs")
}

/// The driver's source with every comment removed.
///
/// Doc comments are the point: the module header of the version this test was
/// written against described the four modules it called *and* named the five it
/// did not, so a scan over the raw text would have found every symbol and
/// reported a driver that called nothing.
fn code_only(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            out.push('\n');
            continue;
        }
        // An end-of-line comment. Crude — a `//` inside a string literal would
        // truncate the line — and crude in the safe direction: it can hide a
        // call from this scan and report a gap that is not there, never the
        // reverse.
        match line.find("//") {
            Some(at) => out.push_str(&line[..at]),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

#[test]
fn every_decision_the_crate_holds_has_a_caller_in_the_driver() {
    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    let code = code_only(&source);

    let mut unreached = Vec::new();
    for (what, call) in MUST_BE_CALLED {
        if !code.contains(call) {
            unreached.push(format!("`{call}` — {what}"));
        }
    }
    assert!(
        unreached.is_empty(),
        "these decisions are built, tested and unreached. A module with no caller is \
         indistinguishable from a working one: its own tests pass, the driver's tests pass, and \
         the control does not exist.\n  {}",
        unreached.join("\n  ")
    );
}

#[test]
fn the_driver_does_not_merely_mention_the_modules_in_prose() {
    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    // The header names every module, wired or not, so the scan that matters is
    // the one over code. If this ever fails, `code_only` has stopped stripping
    // comments and the test above has stopped meaning anything.
    assert!(
        source.contains("//!"),
        "the driver has a module header, so the strip is load-bearing"
    );
    let code = code_only(&source);
    assert!(
        !code.contains("//!"),
        "`code_only` left a doc comment in, so the scan above would read prose as a call"
    );
}

/// Every `HoldReason` `plan` can return is named in the driver.
///
/// Built from an exhaustive `match`: a ninth hold does not compile until
/// somebody has decided what the driver does with it. That is the forcing
/// function — the list cannot rot into a list somebody maintains by looking.
fn hold_reasons() -> Vec<(&'static str, &'static str)> {
    [
        HoldReason::WindowClosed,
        HoldReason::DayBudgetExhausted,
        HoldReason::DayBudgetUnreadable { why: String::new() },
        HoldReason::Paused(Vec::new()),
        HoldReason::CannotDecide {
            missing: Vec::new(),
        },
        HoldReason::ScheduleIndeterminate,
        HoldReason::NoSlots,
        HoldReason::NothingClaimable,
    ]
    .iter()
    .map(|r| {
        let marker = match r {
            HoldReason::WindowClosed => "HoldReason::WindowClosed",
            HoldReason::DayBudgetExhausted => "HoldReason::DayBudgetExhausted",
            HoldReason::DayBudgetUnreadable { .. } => "HoldReason::DayBudgetUnreadable",
            HoldReason::Paused(_) => "HoldReason::Paused",
            HoldReason::CannotDecide { .. } => "HoldReason::CannotDecide",
            HoldReason::ScheduleIndeterminate => "HoldReason::ScheduleIndeterminate",
            HoldReason::NoSlots => "HoldReason::NoSlots",
            HoldReason::NothingClaimable => "HoldReason::NothingClaimable",
        };
        (r.as_str(), marker)
    })
    .collect()
}

fn plan_steps() -> Vec<&'static str> {
    [
        Step::Drain {
            at: DrainAt::Now,
            children: Vec::new(),
        },
        Step::Resume(Vec::new()),
        Step::Claim { slots: 1 },
        Step::Hold {
            why: HoldReason::NoSlots,
            re_arms_at: None,
        },
    ]
    .iter()
    .map(|s| match s {
        Step::Drain { .. } => "execute::Step::Drain",
        Step::Resume(_) => "execute::Step::Resume",
        Step::Claim { .. } => "execute::Step::Claim",
        Step::Hold { .. } => "execute::Step::Hold",
    })
    .collect()
}

fn drains() -> Vec<&'static str> {
    [
        Drain::NotDrained,
        Drain::Waiting { reason: "" },
        Drain::Complete,
    ]
    .iter()
    .map(|d| match d {
        Drain::NotDrained => "Drain::NotDrained",
        Drain::Waiting { .. } => "Drain::Waiting",
        Drain::Complete => "Drain::Complete",
        Drain::Replenish(_) => "Drain::Replenish",
    })
    .chain(std::iter::once("Drain::Replenish"))
    .collect()
}

#[test]
fn every_refusal_reaches_a_branch_of_its_own() {
    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    let code = code_only(&source);

    let mut missing = Vec::new();
    for (word, marker) in hold_reasons() {
        if !code.contains(marker) {
            missing.push(format!(
                "`{marker}` (`{word}`): `execute::plan` can return this hold and the driver has \
                 no arm for it, so it falls into whichever arm was written last"
            ));
        }
    }
    for marker in plan_steps() {
        if !code.contains(marker) {
            missing.push(format!(
                "`{marker}`: a step the plan returns and the driver never names. A step silently \
                 dropped is a control the operator believes in and nothing performs"
            ));
        }
    }
    for marker in drains() {
        if !code.contains(marker) {
            missing.push(format!(
                "`{marker}`: `on_drain` can answer this and the driver does not say what it does \
                 about it"
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "a refusal is not a pass, and collapsing one into \"continue\" removes the control \
         without failing anything:\n  {}",
        missing.join("\n  ")
    );
}

/// The lease refusal is a refusal, not a `None` the driver shrugs at.
///
/// `execute::lease` returns `Result<Lease, LeaseRefusal>` and every variant of
/// the error is a refusal to start a run — §6.3's *"it may not borrow"* has no
/// meaning if a lease can be issued without knowing what is left. The failure
/// this guards against is the one-character kind: `.ok()`, `.unwrap_or`, or a
/// `let _ =` that turns a ceiling into a suggestion.
#[test]
fn a_refused_lease_starts_no_child() {
    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    let code = code_only(&source);

    let at = code
        .find("execute::lease(")
        .expect("the driver sizes a lease");
    let after = &code[at..(at + 400).min(code.len())];
    for swallow in [
        ".ok()",
        ".unwrap_or",
        ".unwrap_or_default()",
        ".ok().flatten()",
    ] {
        assert!(
            !after.contains(swallow),
            "`{swallow}` next to `execute::lease` turns a refusal into a missing value, and a \
             missing value is how a child gets started on a budget nobody read"
        );
    }
    assert!(
        code.contains("Err(refusal)") || code.contains("Err(e) => "),
        "the refusal must reach a branch that says what happens instead of a run"
    );
    // The refusal is written down. A control nobody can see the operation of is
    // a control nobody can audit.
    assert!(
        code.contains("lease_refused"),
        "a refused lease is recorded on the session's log, not swallowed"
    );
}

/// The three-valued answers stay three-valued on the way to the log.
///
/// `Fired::NotEvaluable` and `SkipReason::NotEvaluable` are the shape of "the
/// world has no answer on file". They reach a human through
/// `SkipReason::as_str`, which is why the driver writes that rather than a word
/// of its own: a second vocabulary is a second chance to print `no` for
/// `unanswerable`.
#[test]
fn an_unanswerable_subscription_is_written_down_rather_than_read_as_no() {
    assert!(
        !Fired::NotEvaluable {
            missing: String::new()
        }
        .inserts(),
        "an unanswerable condition puts nothing on the agenda"
    );
    assert_ne!(
        Fired::NotEvaluable {
            missing: String::new()
        }
        .as_str(),
        Fired::No.as_str(),
        "and it is not the same word on the log"
    );

    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    let code = code_only(&source);
    assert!(
        code.contains("s.why.as_str()"),
        "the skip reason reaches the log through `SkipReason::as_str`, not through a word this \
         driver made up"
    );
    assert_ne!(
        SkipReason::NotEvaluable.as_str(),
        SkipReason::OutOfScope.as_str()
    );
}

/// `CannotDecide` is not a pause, and `Paused` is not a failed read.
///
/// Both stop the iteration and only one is a state a human clears. A driver
/// that recorded the first as the second would leave somebody clearing a pause
/// nothing caused; one that recorded the second as the first would resume a
/// session into the thing that was wrong with it.
#[test]
fn an_unread_coupling_and_a_pause_are_not_the_same_event() {
    let cannot = Verdict::CannotDecide {
        missing: vec!["the kill switch".into()],
    };
    let paused = Verdict::Paused(vec![PauseReason::KillSwitch]);
    assert!(!cannot.may_claim() && !paused.may_claim());
    assert!(!cannot.is_a_transition());
    assert!(paused.is_a_transition());

    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    let code = code_only(&source);
    // The pause reaches the log as a transition; the failed read reaches it as
    // a note, which the fold does not move a state on.
    assert!(
        code.contains("EventKind::SessionPaused"),
        "a coupling pause is a state the log records"
    );
    let at = code
        .find("HoldReason::CannotDecide")
        .expect("the driver has an arm for an unread coupling");
    let arm = &code[at..(at + 700).min(code.len())];
    assert!(
        arm.contains("EventKind::Note"),
        "an unread coupling is a note, not a transition: {arm}"
    );
    assert!(
        !arm.contains("EventKind::SessionPaused"),
        "an unread coupling must not be recorded as a pause"
    );
}

/// The unwired list is a list of claims, not a list of silences.
#[test]
fn every_unwired_decision_names_the_symbol_that_is_missing() {
    let source = std::fs::read_to_string(driver_path()).expect("the driver's source");
    let code = code_only(&source);
    for (symbol, why) in UNWIRED {
        assert!(
            why.contains("MISSING:"),
            "`{symbol}` is unwired and the row does not say what is missing"
        );
        let called = symbol
            .rsplit("::")
            .next()
            .map(|f| code.contains(&format!("{f}(")))
            .unwrap_or(false);
        assert!(
            !called,
            "`{symbol}` is on the unwired list and the driver calls it. Delete the row: a stale \
             exemption is worse than no list, because it reads as a decision somebody made."
        );
    }
}
