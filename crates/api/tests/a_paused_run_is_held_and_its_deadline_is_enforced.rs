//! A paused run stops spending, and the pause it is holding has an end.
//!
//! # Why this file exists
//!
//! `POST /api/agent/runs/{id}/pause` and `…/resume` answered **404** while
//! `…/cancel` answered 401. AGENT-012 requires the pause in §3.4 and makes it an
//! acceptance criterion in §19.3; §17 marked it unbuilt and the platform agreed.
//!
//! The desk's Pause button was deleted on 2026-09-21, and that did not close the
//! need — it closed the visible half. Pause was the worst of the three deleted
//! controls because it did not merely do nothing: `onPause` set a local `paused`
//! flag *before* issuing the request that 404'd, so the conversation header read
//! **paused** over a run that was still buying trials, and the Stop dialog
//! offered "Pause instead" as the gentler option. An operator was being talked
//! out of the control that worked and into the one that did not.
//!
//! The way to build that badly a second time is to write a `paused` column and
//! let the loop keep going. So the claims under test here are the two that make
//! the word true:
//!
//! 1. a paused loop **takes no steps**, and
//! 2. the deadline is enforced by something that **runs** — and on expiry the
//!    run **stops**, because a run nobody came back to must not quietly start
//!    spending again.
//!
//! And one more, because this repository's dominant defect is complete, tested
//! code with no caller: both drivers are checked to actually ask.
//!
//! ```bash
//! cargo test -j 2 -p api --test a_paused_run_is_held_and_its_deadline_is_enforced
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use api::agent::control::{Hold, PauseRefused, PauseWindow, RunControl, StopReason, MAX_PAUSE};
use api::agent::conversations::ACTIVE_RUN_STATES;
use api::agent::manager::{PauseView, TERMINAL_RUN_STATES};

// ── 1. A paused loop takes no steps ──────────────────────────────────────────

/// A stand-in for the thing both drivers do: ask at the boundary, then spend.
///
/// `steps` counts what a real loop would have paid for — one model call and one
/// tool call. It is the only number that can tell a pause from a claim.
async fn drive(control: Arc<RunControl>, steps: Arc<AtomicUsize>) -> StopReason {
    loop {
        match control.hold_at_boundary().await {
            Hold::Proceed | Hold::Resumed => {
                steps.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Hold::Stop(reason) => return reason,
        }
    }
}

#[tokio::test]
async fn a_paused_loop_spends_nothing_until_somebody_resumes_it() {
    let control = Arc::new(RunControl::new());
    let steps = Arc::new(AtomicUsize::new(0));
    let loop_task = tokio::spawn(drive(Arc::clone(&control), Arc::clone(&steps)));

    // Let it get going, so "no steps" below is a hold rather than a slow start.
    tokio::time::sleep(Duration::from_millis(40)).await;
    let before = steps.load(Ordering::SeqCst);
    assert!(
        before > 0,
        "the stand-in loop never ran, so it proves nothing"
    );

    control
        .pause(PauseWindow::default())
        .expect("pause granted");
    // One in-flight step may still land after the pause is set — the loop is
    // held at the *next* boundary, not interrupted mid-step, which is the same
    // contract cancel has and the reason a tool is never killed mid-write.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let held_at = steps.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        steps.load(Ordering::SeqCst),
        held_at,
        "the run kept spending while it was paused — which is the header saying \
         `paused` over a run still buying trials, one layer down"
    );

    control.resume().expect("resume granted");
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(
        steps.load(Ordering::SeqCst) > held_at,
        "a resumed run took no further step"
    );

    control.cancel();
    assert_eq!(
        loop_task.await.expect("join"),
        StopReason::Cancelled,
        "cancel must still leave by the same door"
    );
}

// ── 2. The deadline is enforced, and expiry stops ────────────────────────────

/// Nothing else is running in this test. If the deadline were a column waiting
/// for a sweeper, this would hang — which is the point of asserting it here
/// rather than asserting that a column was written.
#[tokio::test]
async fn an_expired_pause_stops_the_run_and_the_loop_is_what_notices() {
    let control = Arc::new(RunControl::new());
    let steps = Arc::new(AtomicUsize::new(0));
    let loop_task = tokio::spawn(drive(Arc::clone(&control), Arc::clone(&steps)));
    tokio::time::sleep(Duration::from_millis(30)).await;

    control
        .pause(PauseWindow::default())
        .expect("pause granted");
    // The floor on a real pause is ten seconds, which is longer than a test
    // should sit still for. `expire_now` brings the deadline forward — the one
    // direction that cannot widen the bound — so what follows is exactly the
    // state a ten-second pause reaches ten seconds later.
    assert!(control.expire_now(), "there was no pause to expire");

    let reason = tokio::time::timeout(Duration::from_secs(5), loop_task)
        .await
        .expect("the deadline was never enforced: nothing woke the parked run")
        .expect("join");
    assert_eq!(reason, StopReason::PauseExpired);

    // Stopped, not resumed. Fail-closed is the whole decision.
    assert!(control.is_cancelled());
    assert_eq!(control.paused_until(), None);
    assert!(
        control.resume().is_err(),
        "an expired pause must not be resumable — a run nobody came back to does \
         not start spending again"
    );

    let after = steps.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        steps.load(Ordering::SeqCst),
        after,
        "the run spent something after its pause expired"
    );
}

#[test]
fn the_bound_is_a_type_rather_than_a_request_the_caller_is_asked_to_respect() {
    // There is no constructor that takes an arbitrary Duration, so this is the
    // longest pause that can be represented — from a route, from a test, from
    // anywhere.
    assert_eq!(
        PauseWindow::from_seconds(i64::MAX).as_duration(),
        MAX_PAUSE,
        "a pause longer than the bound was representable"
    );
    assert_eq!(PauseWindow::clamped(Duration::MAX).as_duration(), MAX_PAUSE);
}

#[test]
fn a_second_pause_is_refused_rather_than_silently_extending_the_first() {
    let control = RunControl::new();
    let first = control.pause(PauseWindow::default()).expect("first");
    assert_eq!(
        control.pause(PauseWindow::default()),
        Err(PauseRefused::AlreadyPaused),
        "a pause that renews itself is the unbounded pause the deadline prevents"
    );
    assert_eq!(control.paused_until(), Some(first));
}

// ── 3. Both drivers really ask ───────────────────────────────────────────────

fn crate_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The guard this repository most needs. A `RunControl` with a lovely test suite
/// and no caller is the shape of defect that keeps being found here, and for
/// this feature it is indistinguishable from the bug being fixed: a pause the
/// loop never asks about is a pause that only exists in the response body.
#[test]
fn every_driver_asks_the_control_at_its_step_boundary() {
    for driver in ["agent/driver.rs", "agent/local_driver.rs"] {
        let src = read(&crate_src().join(driver));
        assert!(
            src.contains("hold_at_boundary"),
            "{driver} never asks the control to hold, so a pause would set a flag while \
             this loop kept spending — the exact lie the desk's deleted button told"
        );
        assert!(
            !src.contains("AtomicBool"),
            "{driver} still polls a bare cancel flag somewhere; a second control the pause \
             does not reach is a boundary that can ignore it"
        );
    }
}

/// The routes exist and are reachable, and `escalate` deliberately does not.
#[test]
fn pause_and_resume_are_mounted_and_escalate_is_not() {
    let router = read(&crate_src().join("routes/mod.rs"));
    for path in [
        "/api/agent/runs/{id}/pause",
        "/api/agent/runs/{id}/resume",
        "/api/agent/runs/{id}/cancel",
    ] {
        assert!(router.contains(path), "{path} is not mounted");
    }
    assert!(
        !router.contains("escalate"),
        "escalate is struck by ADR-D-27, not scheduled: a mid-run tier change carries its \
         own budget consequences and has no caller. Mounting it needs that decision reversed."
    );
}

/// The handlers take the auth extractor. `every_mounted_route_refuses_an_anonymous_caller`
/// enforces this across the router; it is asserted here too because these two
/// routes accept a *control* over a run that is spending money, and "public by
/// omission" is how this crate publishes a handler by accident.
#[test]
fn neither_control_answers_a_caller_who_is_nobody() {
    let src = read(&crate_src().join("agent/routes.rs"));
    for handler in ["pub async fn pause_run(", "pub async fn resume_run("] {
        let at = src
            .find(handler)
            .unwrap_or_else(|| panic!("{handler} is gone"));
        let signature = &src[at..at + 400.min(src.len() - at)];
        assert!(
            signature.contains("token: BearerToken"),
            "{handler} takes no BearerToken, so it answers anyone"
        );
    }
}

// ── 4. A pause a reader can find ─────────────────────────────────────────────
//
// The control works and the routes are mounted, and that is still not the whole
// fix. The bug was a header that said `paused` from a local flag with no server
// answer to check it against. If the pause is visible only in the reply to the
// click that set it, the next page load is back to guessing — so the run reads
// have to carry it, and they have to be honest about the case where this
// replica did not measure.

/// The rule this platform states everywhere and breaks in exactly this shape:
/// an absent measurement rendered as a zero. A run this process is not driving
/// was not observed; reporting it as `paused: false` would be a claim, in the
/// one field an operator would use to decide whether a run is spending money.
#[test]
fn a_run_this_process_did_not_observe_is_null_and_not_false() {
    let (paused, until) = PauseView::Unobserved.as_json();
    assert!(
        paused.is_null(),
        "an unobserved run was reported as not paused, which is a measurement \
         nobody took rendered as a fact"
    );
    assert!(until.is_null());

    // The three this process *can* answer.
    let deadline = chrono::Utc::now() + chrono::Duration::minutes(15);
    let (paused, until) = PauseView::Held(deadline).as_json();
    assert_eq!(paused, serde_json::Value::Bool(true));
    assert_eq!(until, serde_json::json!(deadline.to_rfc3339()));
    assert_eq!(
        PauseView::Running.as_json(),
        (serde_json::Value::Bool(false), serde_json::Value::Null)
    );
    assert_eq!(
        PauseView::Finished.as_json(),
        (serde_json::Value::Bool(false), serde_json::Value::Null),
        "a finished run holds no pause, and that absence this process can assert"
    );
}

/// The reads carry it. Without this the pause state dies with the response that
/// created it, which is the deleted button's defect with the sides swapped.
#[test]
fn the_run_reads_report_the_pause_rather_than_only_the_pause_reply() {
    let src = read(&crate_src().join("agent/routes.rs"));
    let at = src
        .find("fn run_row_to_json(")
        .expect("the run serialiser is gone");
    // To the end of the serialiser, not a fixed window: the run row is wide and
    // a window short enough to be safe would miss the fields being asserted.
    let end = src[at..]
        .find(
            "
pub async fn list_runs",
        )
        .map_or(src.len(), |o| at + o);
    let body = &src[at..end];
    assert!(
        body.contains("pause_view"),
        "GET /api/agent/runs/{{id}} does not ask whether the run is held, so a \
         client that reloads cannot tell a paused run from a spending one"
    );
    assert!(
        body.contains("\"paused\""),
        "the run reads carry no `paused` field"
    );
    assert!(
        body.contains("\"paused_until\""),
        "the run reads carry no deadline, so `paused` would be the unbounded \
         pause the whole decision exists to prevent"
    );
}

/// Both drivers must leave a trace that says *which* stop this was. Status is
/// `cancelled` for both on purpose — a sixth terminal word would be a value
/// every existing reader fails to recognise — so the distinction has to land in
/// `agent_runs.error`, and a run lost to its deadline must not write the same
/// row as one an operator stopped.
#[test]
fn an_expired_pause_says_so_on_the_run_row_in_both_drivers() {
    assert!(
        StopReason::PauseExpired.error_text().is_some(),
        "an expired pause carries no explanation to write"
    );
    assert!(
        StopReason::Cancelled.error_text().is_none(),
        "a plain cancel never carried an error and must not start"
    );
    assert_eq!(
        StopReason::PauseExpired.terminal_status(),
        StopReason::Cancelled.terminal_status(),
        "the two stops must land on the same status word; the distinction is the error"
    );

    let frontier = read(&crate_src().join("agent/driver.rs"));
    assert!(
        frontier.contains("reason.error_text()"),
        "agent/driver.rs drops the stop reason, so an expired pause writes a bare \
         `cancelled` and the operator cannot tell they lost the run to the deadline"
    );
    let local = read(&crate_src().join("agent/local_driver.rs"));
    assert!(
        local.contains("rec.stop_reason = Some(reason)") && local.contains("self.stop_reason"),
        "agent/local_driver.rs routes its stop through the governor's one `Cancel` \
         input (correctly) but never carries the reason to `finish`, so an expired \
         pause and a Stop write byte-identical rows"
    );
}

// ── 5. The status words, checked rather than assumed ─────────────────────────

/// `pause`, `resume` and the run reads all branch on "is this run over". That
/// branch is only as good as the list behind it, and this repository has already
/// paid once for a branch keyed on a word the column never takes: a driver
/// tested `ended == "succeeded"`, so every finished child took the failure path.
///
/// So the list is checked against the drivers instead of being trusted. Every
/// status either driver writes must be in exactly one of the two lists — a word
/// in neither is a run that is over and that `pause_view` would report as
/// unknown forever, and a word in both is two readers disagreeing.
#[test]
fn the_terminal_list_holds_every_status_the_drivers_write() {
    for s in TERMINAL_RUN_STATES {
        assert!(
            !ACTIVE_RUN_STATES.contains(s),
            "`{s}` is in both lists, so the sidebar's dot and the pause gate disagree"
        );
    }

    let local = read(&crate_src().join("agent/local_driver.rs"));
    let at = local
        .find("let (status, summary, error) = match outcome {")
        .expect("the local driver's outcome→status mapping is gone");
    let end = local[at..]
        .find("\n        };")
        .map_or(local.len(), |o| at + o);
    // Prose first: the arms' other string literals are sentences, and the
    // comments quote status words on purpose.
    let arms: String = local[at..end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut found: Vec<String> = Vec::new();
    let mut rest = arms.as_str();
    while let Some(open) = rest.find('"') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('"') else { break };
        let lit = &rest[..close];
        rest = &rest[close + 1..];
        // A status is one bare word. Everything else in these arms is a
        // sentence, a format template or a field path.
        if !lit.is_empty() && lit.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            found.push(lit.to_string());
        }
    }
    assert!(
        found.len() >= 5,
        "found only {found:?} in the local driver's mapping; the scan stopped matching \
         and would pass without checking anything"
    );
    for status in &found {
        assert!(
            TERMINAL_RUN_STATES.contains(&status.as_str()),
            "the local driver ends a run `{status}` and nothing calls that terminal, so \
             `pause` would offer to hold a run that finished and the run reads would \
             report its pause as unknown forever"
        );
    }
    // The two the five-word list used to miss, named so a future edit that drops
    // them has to argue with this line rather than with a silence.
    for over in ["suspended", "offered"] {
        assert!(
            found.contains(&over.to_string()),
            "the local driver no longer ends a run `{over}`; if that is deliberate, take \
             it out of TERMINAL_RUN_STATES too"
        );
    }

    // The frontier driver's terminal words, which it writes as literals.
    let frontier = read(&crate_src().join("agent/driver.rs"));
    for status in ["failed", "cancelled", "completed"] {
        if frontier.contains(&format!("ctx.finish(\"{status}\"")) {
            assert!(
                TERMINAL_RUN_STATES.contains(&status),
                "agent/driver.rs ends a run `{status}` and nothing calls that terminal"
            );
        }
    }
    // And the one the pause writes, through the type rather than a literal.
    assert!(
        TERMINAL_RUN_STATES.contains(&StopReason::PauseExpired.terminal_status()),
        "an expired pause lands on a status no reader calls terminal"
    );
}
