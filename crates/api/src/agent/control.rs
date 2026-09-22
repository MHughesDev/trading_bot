//! The control handle a run is driven by: cancel, and a **bounded** pause.
//!
//! # Why this exists at all
//!
//! The manager already held one `Arc<AtomicBool>` per in-process run and both
//! drivers polled it at their step boundaries. That is the whole cancel
//! mechanism, and it works: the flag is set by the route, the loop reads it
//! between steps, and the run stops where it is safe to stop.
//!
//! Pause has to be built *on that*, not beside it. A pause that sets a column
//! while the loop keeps spending is the lie the desk's deleted Pause button was
//! already telling — its handler set a local `paused` flag before issuing a
//! request that 404'd, so the conversation header read **paused** over a run
//! still buying trials. Replacing that with a column nobody reads would be the
//! same defect one layer down.
//!
//! So this type replaces the flag rather than joining it. The drivers ask it at
//! the same boundaries they used to poll the flag at, and when the run is paused
//! the ask **parks** — no model call, no tool call, nothing spent — until
//! somebody resumes or the deadline passes.
//!
//! # Why the pause is bounded, and why expiry stops rather than resumes
//!
//! A paused run holds its budget, its run-scoped service token, its slot in the
//! manager's map and its place in the drain count. An unbounded pause is
//! therefore a resource leak wearing the costume of an operator being careful.
//!
//! On expiry the run **stops**. Fail-closed is the only defensible direction: a
//! run nobody came back to must not quietly start spending again, and an
//! operator who forgot a paused run is better served finding it stopped than
//! finding it finished. That is also why the bound is a *type* — see
//! [`PauseWindow`] — rather than a comment asking callers to be reasonable.
//!
//! # What enforces the deadline
//!
//! The parked driver task itself. It is already alive, it is already awaiting,
//! and [`RunControl::hold_at_boundary`] selects the deadline against the wake-up
//! — so expiry is observed by the one thing in the system that is guaranteed to
//! be watching this run. There is no sweeper, and none is needed for a pause
//! that only exists while a driver holds the handle.
//!
//! The corollary is stated plainly so nobody has to discover it: a run this
//! process is **not** driving cannot be paused, because there is no loop to
//! hold. `AgentManager::pause` refuses such a run rather than writing a state
//! nothing would honour.
//!
//! # What this deadline does *not* promise
//!
//! It is enforced **at the next boundary**, not on the wall clock, and the two
//! differ whenever the driver is inside a long wait: a backtest poll
//! (`wait_for_backtest_inline`) or an approval that nobody has answered. A pause
//! set just before a forty-minute sweep is observed when the sweep returns, so
//! its deadline can be overshot by the length of that wait.
//!
//! That is the same contract cancel has had all along — the loop is stopped at a
//! boundary, never killed mid-write — and it is stated rather than fixed because
//! the overshoot costs nothing the pause was protecting: a run inside a tool is
//! making no model call and buying no new trial, which is exactly what a pause
//! was asked to stop. What it *does* hold for longer is the budget and the
//! lease, so the honest summary is: **the hold ends no later than the deadline
//! plus the tool call in flight when it was set.**
//!
//! Closing that gap would need a pause the *tool waits* also poll, not a
//! sweeper, and no caller has asked for one.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::watch;

/// The shortest pause worth taking. Below this the round trip costs more than
/// the hold is worth, and a zero would be a pause that expires before the loop
/// reaches its next boundary — a stop dressed as a pause.
pub const MIN_PAUSE: Duration = Duration::from_secs(10);

/// The pause an operator gets when they name no length.
pub const DEFAULT_PAUSE: Duration = Duration::from_secs(15 * 60);

/// The longest hold this platform will grant.
///
/// Two hours, not a day: everything a paused run is sitting on — budget, lease,
/// slot — is held for the whole window, and the honest question is how long a
/// desk is willing to pay for a decision nobody has made. Two hours covers
/// "stepped out"; a day covers "forgot", and forgetting is what the deadline is
/// for.
pub const MAX_PAUSE: Duration = Duration::from_secs(2 * 60 * 60);

/// A pause length that is inside the bound **by construction**.
///
/// There is no constructor that takes a `Duration`, so there is no way to write
/// a pause longer than [`MAX_PAUSE`] — including from a test. That is the
/// enforcement: not a check a later caller can route around, and not a sentence
/// in a doc comment asking them not to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PauseWindow(Duration);

impl PauseWindow {
    /// Clamps a requested length into `[MIN_PAUSE, MAX_PAUSE]`.
    ///
    /// Clamped rather than rejected: the caller asked to pause, and refusing the
    /// whole request over a number returns the operator to a running run, which
    /// is the outcome they were trying to avoid. The response says what they
    /// actually got.
    #[must_use]
    pub fn clamped(requested: Duration) -> Self {
        Self(requested.clamp(MIN_PAUSE, MAX_PAUSE))
    }

    /// Seconds off the wire, where a negative or absurd number is a caller
    /// mistake rather than a request.
    #[must_use]
    pub fn from_seconds(requested: i64) -> Self {
        let secs = u64::try_from(requested).unwrap_or(0);
        Self::clamped(Duration::from_secs(secs))
    }

    #[must_use]
    pub fn as_duration(self) -> Duration {
        self.0
    }

    #[must_use]
    pub fn as_secs(self) -> u64 {
        self.0.as_secs()
    }
}

impl Default for PauseWindow {
    fn default() -> Self {
        Self::clamped(DEFAULT_PAUSE)
    }
}

/// Why a boundary decided the run must not continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// Somebody cancelled it.
    Cancelled,
    /// It was paused and nobody came back before the deadline.
    PauseExpired,
}

impl StopReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::PauseExpired => "pause_expired",
        }
    }

    /// What `agent_runs.status` becomes.
    ///
    /// `cancelled` for both, deliberately. An expired pause **is** a stop that a
    /// human set in motion, and inventing a sixth terminal word would put a
    /// value in the column that every existing reader — the sidebar's running
    /// dot, `ACTIVE_RUN_STATES`, the restart sweep, `cancel`'s own terminal
    /// list — would fail to recognise. This repository has already paid for that
    /// once: a driver tested `ended == "succeeded"`, a value `agent_runs.status`
    /// never takes, so every finished child took the failure branch.
    ///
    /// The distinction that matters is kept where it can be read without
    /// breaking anyone: [`Self::error_text`] lands in `agent_runs.error`.
    #[must_use]
    pub fn terminal_status(self) -> &'static str {
        "cancelled"
    }

    /// The `agent_runs.error` line, or `None` for a plain cancel — which needs
    /// no explanation and never carried one.
    #[must_use]
    pub fn error_text(self) -> Option<&'static str> {
        match self {
            Self::Cancelled => None,
            Self::PauseExpired => Some(
                "paused and not resumed before the deadline; stopped rather than resumed, so a \
                 run nobody came back to does not start spending again",
            ),
        }
    }
}

/// What a step boundary learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Not paused, not cancelled. Take the step.
    Proceed,
    /// It was paused, and it was resumed. Take the step.
    Resumed,
    /// Do not take the step.
    Stop(StopReason),
}

/// Why a pause was not granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseRefused {
    /// The run is already stopping. Pausing it would be a hold on something
    /// that is not going to take another step anyway.
    Stopping,
    /// Already paused. Reported rather than silently extended: an extension is a
    /// second decision, and a pause that quietly renews itself is the unbounded
    /// pause this type exists to prevent.
    AlreadyPaused,
}

/// Why a resume did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeRefused {
    /// It is stopping; there is nothing to resume it into.
    Stopping,
    /// It was not paused.
    NotPaused,
}

/// The state both the routes and the loop read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
    stopping: bool,
    /// The wall-clock deadline while paused.
    paused_until: Option<DateTime<Utc>>,
}

/// One run's control handle, shared by the manager, the routes and the driver.
#[derive(Debug)]
pub struct RunControl {
    /// A `watch` rather than a `Notify`, for one reason worth stating: the
    /// parked loop reads the state and then waits, and with a notification
    /// primitive a resume landing between those two lines is a wake-up nobody
    /// is listening for — a run that stays parked until its deadline and then
    /// stops, having been resumed. `borrow_and_update` marks the value seen, so
    /// `changed()` cannot miss the edit that follows it.
    tx: watch::Sender<State>,
}

impl Default for RunControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RunControl {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tx: watch::channel(State {
                stopping: false,
                paused_until: None,
            })
            .0,
        }
    }

    /// Ask the run to stop at its next boundary. Wakes a paused run so it stops
    /// now rather than at its deadline.
    pub fn cancel(&self) {
        self.tx.send_modify(|s| {
            s.stopping = true;
            s.paused_until = None;
        });
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.tx.borrow().stopping
    }

    /// The deadline if this run is paused.
    #[must_use]
    pub fn paused_until(&self) -> Option<DateTime<Utc>> {
        self.tx.borrow().paused_until
    }

    /// Hold the run from its next boundary until `window` elapses or somebody
    /// resumes it. Returns the deadline the loop will actually honour.
    ///
    /// # Errors
    ///
    /// [`PauseRefused`] when the run is already stopping or already paused.
    pub fn pause(&self, window: PauseWindow) -> Result<DateTime<Utc>, PauseRefused> {
        let mut outcome = Err(PauseRefused::Stopping);
        self.tx.send_if_modified(|s| {
            if s.stopping {
                outcome = Err(PauseRefused::Stopping);
                return false;
            }
            if s.paused_until.is_some() {
                outcome = Err(PauseRefused::AlreadyPaused);
                return false;
            }
            let deadline = Utc::now()
                + chrono::Duration::from_std(window.as_duration())
                    .unwrap_or_else(|_| chrono::Duration::seconds(0));
            s.paused_until = Some(deadline);
            outcome = Ok(deadline);
            true
        });
        outcome
    }

    /// Let a paused run take its next step.
    ///
    /// # Errors
    ///
    /// [`ResumeRefused`] when the run is stopping or was not paused.
    pub fn resume(&self) -> Result<(), ResumeRefused> {
        let mut outcome = Err(ResumeRefused::NotPaused);
        self.tx.send_if_modified(|s| {
            if s.stopping {
                outcome = Err(ResumeRefused::Stopping);
                return false;
            }
            if s.paused_until.is_none() {
                outcome = Err(ResumeRefused::NotPaused);
                return false;
            }
            s.paused_until = None;
            outcome = Ok(());
            true
        });
        outcome
    }

    /// Bring a paused run's deadline forward to now, so its next boundary
    /// observes the expiry immediately.
    ///
    /// This can only move a hold toward *stopping*: it does not extend one, it
    /// cannot create one, and there is no argument it could carry that would let
    /// a caller exceed [`MAX_PAUSE`]. So it opens no hole in the bound, and it
    /// is what "end this hold now" means — ending a hold is stopping the run,
    /// because that is what the deadline does.
    ///
    /// Returns whether there was a pause to end.
    pub fn expire_now(&self) -> bool {
        let mut ended = false;
        self.tx.send_if_modified(|s| {
            if s.paused_until.is_none() {
                return false;
            }
            s.paused_until = Some(Utc::now());
            ended = true;
            true
        });
        ended
    }

    /// The step boundary. **This is the whole mechanism**: while the run is
    /// paused this future does not resolve, so the loop takes no step, makes no
    /// model call and buys no trial.
    ///
    /// On expiry it flips the run to stopping itself — the deadline is enforced
    /// here, by the task that is already awaiting it, and not by a column
    /// somebody hopes a sweeper reads.
    pub async fn hold_at_boundary(&self) -> Hold {
        let mut rx = self.tx.subscribe();
        let mut ever_paused = false;
        loop {
            let state = *rx.borrow_and_update();
            if state.stopping {
                return Hold::Stop(StopReason::Cancelled);
            }
            let Some(deadline) = state.paused_until else {
                return if ever_paused {
                    Hold::Resumed
                } else {
                    Hold::Proceed
                };
            };
            ever_paused = true;

            let remaining = (deadline - Utc::now()).to_std().unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                // Expiry. Written through the same channel a cancel uses, so a
                // run that timed out and a run that was cancelled leave the loop
                // by one path and cannot diverge.
                self.tx.send_modify(|s| {
                    s.stopping = true;
                    s.paused_until = None;
                });
                return Hold::Stop(StopReason::PauseExpired);
            }

            tokio::select! {
                // `changed()` after `borrow_and_update` cannot miss an edit made
                // between the two.
                r = rx.changed() => {
                    if r.is_err() {
                        // The sender is gone, which means the manager dropped
                        // this run's handle. Stopping is the fail-closed
                        // reading: nobody can resume it now.
                        return Hold::Stop(StopReason::Cancelled);
                    }
                }
                () = tokio::time::sleep(remaining) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_pause_longer_than_the_bound_is_unrepresentable() {
        let absurd = PauseWindow::clamped(Duration::from_secs(60 * 60 * 24 * 30));
        assert_eq!(absurd.as_duration(), MAX_PAUSE);
        // And from the wire, including the shapes a caller gets wrong.
        assert_eq!(PauseWindow::from_seconds(-1).as_duration(), MIN_PAUSE);
        assert_eq!(PauseWindow::from_seconds(0).as_duration(), MIN_PAUSE);
        assert_eq!(PauseWindow::from_seconds(i64::MAX).as_duration(), MAX_PAUSE);
        assert_eq!(PauseWindow::default().as_duration(), DEFAULT_PAUSE);
    }

    #[tokio::test]
    async fn an_unpaused_run_proceeds_and_a_cancelled_one_stops() {
        let c = RunControl::new();
        assert_eq!(c.hold_at_boundary().await, Hold::Proceed);
        c.cancel();
        assert_eq!(
            c.hold_at_boundary().await,
            Hold::Stop(StopReason::Cancelled)
        );
    }

    #[test]
    fn a_pause_is_refused_twice_and_on_a_stopping_run() {
        let c = RunControl::new();
        assert!(c.pause(PauseWindow::default()).is_ok());
        assert_eq!(
            c.pause(PauseWindow::default()),
            Err(PauseRefused::AlreadyPaused)
        );
        assert!(c.resume().is_ok());
        assert_eq!(c.resume(), Err(ResumeRefused::NotPaused));
        c.cancel();
        assert_eq!(c.pause(PauseWindow::default()), Err(PauseRefused::Stopping));
        assert_eq!(c.resume(), Err(ResumeRefused::Stopping));
    }

    #[tokio::test]
    async fn the_boundary_parks_while_paused_and_carries_on_when_resumed() {
        let c = Arc::new(RunControl::new());
        c.pause(PauseWindow::default()).expect("pause");
        let held = Arc::clone(&c);
        let task = tokio::spawn(async move { held.hold_at_boundary().await });

        // Still parked: the point of the whole thing.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!task.is_finished(), "a paused run took its next step");

        c.resume().expect("resume");
        assert_eq!(task.await.expect("join"), Hold::Resumed);
    }

    #[tokio::test]
    async fn a_cancel_wakes_a_paused_run_rather_than_waiting_out_its_deadline() {
        let c = Arc::new(RunControl::new());
        c.pause(PauseWindow::default()).expect("pause");
        let held = Arc::clone(&c);
        let task = tokio::spawn(async move { held.hold_at_boundary().await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        c.cancel();
        assert_eq!(
            task.await.expect("join"),
            Hold::Stop(StopReason::Cancelled)
        );
    }

    /// The deadline is enforced by the thing that is awaiting it, and this is
    /// the test that says so: nothing else runs in this process.
    #[tokio::test]
    async fn an_expired_pause_stops_the_run_and_does_not_resume_it() {
        let c = Arc::new(RunControl::new());
        // A deadline already in the past is the expiry case without the wait.
        c.tx.send_modify(|s| {
            s.paused_until = Some(Utc::now() - chrono::Duration::seconds(1));
        });
        assert_eq!(
            c.hold_at_boundary().await,
            Hold::Stop(StopReason::PauseExpired)
        );
        assert!(c.is_cancelled(), "expiry must leave the run stopping");
        assert_eq!(c.paused_until(), None);
        // And it stays stopped: a later resume cannot un-expire it.
        assert_eq!(c.resume(), Err(ResumeRefused::Stopping));
        assert_eq!(
            c.hold_at_boundary().await,
            Hold::Stop(StopReason::Cancelled)
        );
    }

    #[tokio::test]
    async fn a_real_deadline_elapses_without_anyone_touching_the_handle() {
        let c = RunControl::new();
        // Past the bound's floor on purpose: this is the wall clock, not a mock.
        c.tx.send_modify(|s| {
            s.paused_until = Some(Utc::now() + chrono::Duration::milliseconds(120));
        });
        let started = std::time::Instant::now();
        assert_eq!(
            c.hold_at_boundary().await,
            Hold::Stop(StopReason::PauseExpired)
        );
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "it stopped before its deadline"
        );
    }

    #[test]
    fn an_expired_pause_lands_on_a_status_value_the_platform_already_knows() {
        // The `ended == "succeeded"` bug, guarded: a terminal word this column
        // never takes is a branch nobody reaches.
        let known = ["queued", "running", "waiting_backtest", "awaiting_approval",
                     "completed", "failed", "cancelled", "fenced", "refused"];
        assert!(known.contains(&StopReason::PauseExpired.terminal_status()));
        assert!(known.contains(&StopReason::Cancelled.terminal_status()));
        assert!(StopReason::PauseExpired.error_text().is_some());
        assert!(StopReason::Cancelled.error_text().is_none());
    }
}
