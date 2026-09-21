//! Platform health, and the one thing a maintenance window is allowed to
//! suppress (COMP-007 §3 "Platform", §4, §4.3).
//!
//! # Why this module exists at all
//!
//! [`crate::window::MaintenanceWindow`] was complete, tested, persisted, opened
//! by a route and listed by another — and **nothing anywhere consulted one**.
//! `MaintenanceWindow::suppresses` had no caller outside its own tests, and
//! `MonitorEvent::suppressed_by` had none at all, although migration 0075 gave
//! `monitor.events` a `suppressed_by_window` column to hold the answer. A window
//! that suppresses nothing is not a safety valve; it is a form the operator
//! fills in before every deploy while the deploy halts trading anyway, which is
//! precisely the "safety mechanism becomes a nuisance and then a habit of
//! ignoring it" failure §4.3 was written to prevent.
//!
//! The reason nothing consulted a window is that the thing a window suppresses
//! did not exist either: §4's platform row — *"platform heartbeat loss, monitor
//! lag > 2 cadences, DB unreachable → global halt"* — had no implementation, and
//! [`MonitorStore::stale_components`][sc] had no caller. So both halves are here,
//! and they are one mechanism: the trip, and the bounded, attributed, auditable
//! way to say "not during this deploy".
//!
//! [sc]: crate::store::MonitorStore::stale_components
//!
//! # Two rules that are not conveniences
//!
//! **A component is stale only when every instance of it is stale.** Heartbeat
//! rows are keyed `(component, instance)` and nothing deletes them, so a process
//! that restarts under a fresh instance id leaves its old row behind for ever.
//! Judging components row-by-row would mean the first restart of any component
//! arms a permanent global halt — a halt that needs a typed human reset with
//! every venue reconciling clean, triggered by an orderly deploy. One live
//! instance is the component being up.
//!
//! **The monitor does not judge its own heartbeat.** A process asserting its own
//! liveness proves nothing: if `tbot-monitor` is wedged it writes no verdict
//! either way, and if it is running its own row is fresh by construction. The
//! mechanism that answers "is the monitor alive" is the lease, in the *watched*
//! process, where the answer is worth something (§4 "fail closed"). Judging its
//! own row here would add no detection and would hand every monitor restart the
//! power to halt the book.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::family::Family;
use crate::window::{suppressing_window, MaintenanceWindow};

/// Consecutive missed beats before a component counts as lost (COMP-007 §3).
pub const MISSED_BEATS_BEFORE_HALT: i64 = 3;

/// The component name the monitor beats under, and therefore the one name this
/// module refuses to judge. See the module note.
pub const MONITOR_COMPONENT: &str = "tbot-monitor";

/// One row of `monitor.heartbeat`, with staleness already decided by the
/// database against that row's own declared cadence.
///
/// `stale` is a measurement the store made, not a duration this module
/// re-derives: each component declares its own `cadence_ms`, and a module that
/// re-derived staleness from a single global timeout would quietly mis-judge
/// every component whose cadence was not the one it assumed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentBeat {
    pub component: String,
    pub instance: String,
    pub stale: bool,
}

impl ComponentBeat {
    #[must_use]
    pub fn new(component: impl Into<String>, instance: impl Into<String>, stale: bool) -> Self {
        Self {
            component: component.into(),
            instance: instance.into(),
            stale,
        }
    }
}

/// What the platform family concluded this cadence.
///
/// Deliberately not `Option<Halt>`: a suppressed trip is a third outcome that
/// must leave a row behind, and an `Option` invites a caller to treat "nothing
/// to do" and "a window swallowed a real fault" as the same branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformVerdict {
    /// Every judged component has at least one live instance.
    Healthy,
    /// Components are lost, an open window covers platform health, and so
    /// **nothing is halted** — but the row records what would have been, and
    /// which window took the decision (§4.3: a window nobody can see is a hole
    /// nobody can audit).
    Suppressed {
        lost: Vec<String>,
        window_id: Uuid,
    },
    /// Components are lost and no window covers it: **global halt** (§4).
    Halt { lost: Vec<String> },
}

impl PlatformVerdict {
    /// Whether the platform is currently unhealthy, whatever was done about it.
    ///
    /// A suppressed fault is still a fault. The edge tracker uses this rather
    /// than equality so that a window opening mid-incident does not read as a
    /// recovery and write "resumed" into the timeline of an outage.
    #[must_use]
    pub fn is_unhealthy(&self) -> bool {
        !matches!(self, Self::Healthy)
    }

    /// The components this verdict is about, in the order they are reported.
    #[must_use]
    pub fn lost(&self) -> &[String] {
        match self {
            Self::Healthy => &[],
            Self::Suppressed { lost, .. } | Self::Halt { lost } => lost,
        }
    }

    /// The sentence the operator reads first.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Healthy => "every platform component is beating".to_owned(),
            Self::Suppressed { lost, window_id } => format!(
                "{} missed {MISSED_BEATS_BEFORE_HALT} beats; \
                 the global halt was suppressed by open maintenance window {window_id}",
                lost.join(", ")
            ),
            Self::Halt { lost } => format!(
                "{} missed {MISSED_BEATS_BEFORE_HALT} beats with no maintenance window open",
                lost.join(", ")
            ),
        }
    }
}

/// Judge the platform's heartbeats against the open windows.
///
/// Pure, and given the beats and the windows rather than reading them, so the
/// whole policy — including the two rules in the module note, which are the ones
/// that decide whether a deploy halts the book — is testable against a synthetic
/// platform rather than against a fixture nobody re-runs.
///
/// `lost` is sorted and deduplicated, so the same outage produces the same reason
/// string on every cadence and the edge tracker does not see a change where the
/// only difference is the order two rows came back in.
#[must_use]
pub fn assess(
    beats: &[ComponentBeat],
    windows: &[MaintenanceWindow],
    now: DateTime<Utc>,
) -> PlatformVerdict {
    let mut lost: Vec<String> = Vec::new();
    for beat in beats {
        // See the module note: the monitor's own liveness is the lease's
        // question, asked in the process that would be harmed by the answer.
        if beat.component == MONITOR_COMPONENT {
            continue;
        }
        if lost.contains(&beat.component) {
            continue;
        }
        // Every instance stale, not any: a component with one live instance is up,
        // however many dead rows earlier deploys left behind. The set is never
        // empty — `beat` is in it — so this cannot be the vacuous truth `all`
        // returns for nothing.
        let all_stale = beats
            .iter()
            .filter(|b| b.component == beat.component)
            .all(|b| b.stale);
        if all_stale {
            lost.push(beat.component.clone());
        }
    }
    if lost.is_empty() {
        return PlatformVerdict::Healthy;
    }
    lost.sort();
    lost.dedup();

    // Family::Platform is the only family that answers yes here, and that answer
    // is not configurable — see `Family::suppressible_by_maintenance_window`.
    match suppressing_window(windows, Family::Platform, now) {
        Some(window_id) => PlatformVerdict::Suppressed { lost, window_id },
        None => PlatformVerdict::Halt { lost },
    }
}

/// What a change in the platform's health is worth recording.
///
/// The same shape, and for the same reason, as [`crate::service::LeaseNote`]: the
/// assessment runs every cadence, and a monitor that wrote a row per cadence
/// through a twenty-minute outage would fill the timeline an operator reads under
/// pressure with two hundred and forty copies of one fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformNote {
    /// The platform became unhealthy. Act on the verdict.
    Lost,
    /// Every component is beating again. The halt, if one was taken, stays until
    /// a human resets it (§5) — this only says the cause has cleared.
    Recovered,
}

/// How the last verdict was being handled.
///
/// Three states rather than a healthy/unhealthy bit, because the transition that
/// would otherwise be lost is `Suppressed → Halt`: the window expired in the
/// middle of an outage and the halt has still not been taken. A bit would call
/// that "already unhealthy" and leave the platform trading through the rest of
/// it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Handling {
    #[default]
    Healthy,
    Suppressed,
    Halted,
}

impl Handling {
    fn of(verdict: &PlatformVerdict) -> Self {
        match verdict {
            PlatformVerdict::Healthy => Self::Healthy,
            PlatformVerdict::Suppressed { .. } => Self::Suppressed,
            PlatformVerdict::Halt { .. } => Self::Halted,
        }
    }
}

/// The edge tracker.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformWatch {
    handling: Handling,
}

impl PlatformWatch {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record this verdict and say what, if anything, is worth writing down.
    ///
    /// A verdict that *changes* how the fault is being handled — an unsuppressed
    /// halt following a suppressed one — is a new decision and reports `Lost`
    /// again, so the halt it takes carries a record. A window opened *after* the
    /// halt was taken reports nothing: the halt stands until a human resets it
    /// (§5), and a window cannot undo one.
    pub fn observe(&mut self, verdict: &PlatformVerdict) -> Option<PlatformNote> {
        let was = self.handling;
        let now = Handling::of(verdict);
        self.handling = now;
        match (was, now) {
            (Handling::Healthy, Handling::Healthy) => None,
            (Handling::Healthy, _) | (Handling::Suppressed, Handling::Halted) => {
                Some(PlatformNote::Lost)
            }
            (_, Handling::Healthy) => Some(PlatformNote::Recovered),
            _ => None,
        }
    }

    /// Whether the last verdict observed was unhealthy.
    #[must_use]
    pub fn is_unhealthy(&self) -> bool {
        self.handling != Handling::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("timestamp")
    }

    fn window() -> MaintenanceWindow {
        MaintenanceWindow::open("human:mason", "rolling deploy of the API", 15, at(0))
            .expect("a reasoned, bounded window opens")
    }

    #[test]
    fn a_platform_with_every_component_beating_is_healthy() {
        let beats = [
            ComponentBeat::new("api", "api-1", false),
            ComponentBeat::new("jobs", "jobs-1", false),
        ];
        assert_eq!(assess(&beats, &[], at(60)), PlatformVerdict::Healthy);
    }

    #[test]
    fn a_lost_component_halts_the_platform_globally() {
        // COMP-007 §3, §4: "missed 3 heartbeats → global halt".
        let beats = [
            ComponentBeat::new("api", "api-1", true),
            ComponentBeat::new("jobs", "jobs-1", false),
        ];
        assert_eq!(
            assess(&beats, &[], at(60)),
            PlatformVerdict::Halt {
                lost: vec!["api".to_owned()]
            }
        );
    }

    #[test]
    fn one_live_instance_is_the_component_being_up() {
        // The rule that decides whether an ordinary deploy halts the book. Nothing
        // deletes heartbeat rows, so a process that restarts under a fresh
        // instance id leaves its old row stale for ever; judging rows instead of
        // components would arm a permanent global halt on the first restart of
        // anything.
        let beats = [
            ComponentBeat::new("api", "api-old", true),
            ComponentBeat::new("api", "api-new", false),
        ];
        assert_eq!(assess(&beats, &[], at(60)), PlatformVerdict::Healthy);
    }

    #[test]
    fn every_instance_stale_is_the_component_being_down() {
        // The other direction, so the rule above cannot be satisfied by never
        // reporting anything lost.
        let beats = [
            ComponentBeat::new("api", "api-old", true),
            ComponentBeat::new("api", "api-new", true),
        ];
        assert_eq!(
            assess(&beats, &[], at(60)),
            PlatformVerdict::Halt {
                lost: vec!["api".to_owned()]
            }
        );
    }

    #[test]
    fn the_monitor_never_judges_its_own_heartbeat() {
        // A process asserting its own liveness proves nothing, and the lease is
        // the mechanism that asks the question where the answer is worth
        // something. Asserted with the monitor's row *alone* and stale, which is
        // exactly the state a restarted monitor leaves behind.
        let beats = [ComponentBeat::new(MONITOR_COMPONENT, "boot-1", true)];
        assert_eq!(assess(&beats, &[], at(60)), PlatformVerdict::Healthy);

        // And it is not merely absent from the list: adding a genuinely lost
        // component must still halt, so the skip cannot be hiding everything.
        let beats = [
            ComponentBeat::new(MONITOR_COMPONENT, "boot-1", true),
            ComponentBeat::new("api", "api-1", true),
        ];
        assert_eq!(
            assess(&beats, &[], at(60)),
            PlatformVerdict::Halt {
                lost: vec!["api".to_owned()]
            }
        );
    }

    #[test]
    fn an_open_window_suppresses_the_platform_halt_and_records_which_one() {
        // COMP-007 §4.3. The window is the difference between a deploy and an
        // outage, and the row is the difference between a valve and a hole.
        let w = window();
        let beats = [ComponentBeat::new("api", "api-1", true)];
        assert_eq!(
            assess(&beats, std::slice::from_ref(&w), at(60)),
            PlatformVerdict::Suppressed {
                lost: vec!["api".to_owned()],
                window_id: w.window_id,
            }
        );
    }

    #[test]
    fn an_expired_window_suppresses_nothing() {
        let w = window();
        let beats = [ComponentBeat::new("api", "api-1", true)];
        assert_eq!(
            assess(&beats, std::slice::from_ref(&w), at(1_000)),
            PlatformVerdict::Halt {
                lost: vec!["api".to_owned()]
            }
        );
    }

    #[test]
    fn a_suppressed_verdict_still_names_the_fault_and_the_window() {
        let w = window();
        let beats = [ComponentBeat::new("api", "api-1", true)];
        let v = assess(&beats, std::slice::from_ref(&w), at(60));
        let reason = v.reason();
        assert!(reason.contains("api"), "{reason}");
        assert!(reason.contains(&w.window_id.to_string()), "{reason}");
        assert!(v.is_unhealthy(), "a suppressed fault is still a fault");
        assert_eq!(v.lost(), ["api".to_owned()]);
    }

    #[test]
    fn a_halt_reason_names_every_lost_component() {
        let beats = [
            ComponentBeat::new("api", "api-1", true),
            ComponentBeat::new("jobs", "jobs-1", true),
        ];
        let v = assess(&beats, &[], at(60));
        let reason = v.reason();
        assert!(reason.contains("api") && reason.contains("jobs"), "{reason}");
        assert_eq!(v.lost(), ["api".to_owned(), "jobs".to_owned()]);
    }

    #[test]
    fn an_empty_platform_is_healthy_rather_than_lost() {
        // Nothing beats into `monitor.heartbeat` but the monitor today. "No rows"
        // must read as "nothing to judge" and never as "everything is missing" —
        // the second would global-halt a platform on its first boot.
        assert_eq!(assess(&[], &[], at(60)), PlatformVerdict::Healthy);
    }

    #[test]
    fn a_fault_is_recorded_on_its_edges_and_not_once_per_cadence() {
        let mut watch = PlatformWatch::new();
        let halt = PlatformVerdict::Halt {
            lost: vec!["api".to_owned()],
        };
        assert_eq!(watch.observe(&halt), Some(PlatformNote::Lost));
        assert_eq!(watch.observe(&halt), None);
        assert_eq!(watch.observe(&halt), None);
        assert_eq!(
            watch.observe(&PlatformVerdict::Healthy),
            Some(PlatformNote::Recovered)
        );
        assert_eq!(watch.observe(&PlatformVerdict::Healthy), None);
    }

    #[test]
    fn a_window_expiring_mid_outage_is_a_new_decision_and_not_a_recovery() {
        // The case that would lose a halt: a suppressed fault followed by the same
        // fault unsuppressed. If the tracker treated both as "already unhealthy"
        // the expiry of the window would silently leave the platform unhalted
        // through the rest of the outage.
        let mut watch = PlatformWatch::new();
        let suppressed = PlatformVerdict::Suppressed {
            lost: vec!["api".to_owned()],
            window_id: Uuid::new_v4(),
        };
        let halt = PlatformVerdict::Halt {
            lost: vec!["api".to_owned()],
        };
        assert_eq!(watch.observe(&suppressed), Some(PlatformNote::Lost));
        assert_eq!(watch.observe(&suppressed), None);
        assert_eq!(
            watch.observe(&halt),
            Some(PlatformNote::Lost),
            "the window expired and the halt has not been taken yet"
        );
        assert!(watch.is_unhealthy());
    }

    #[test]
    fn a_window_opened_after_the_halt_does_not_undo_it() {
        // §5: a halt is released by a typed human reset and by nothing else. A
        // window opened afterwards suppresses the *next* trip, not the one in
        // force, and must not write "recovered" into the timeline of an outage
        // that is still running.
        let mut watch = PlatformWatch::new();
        let halt = PlatformVerdict::Halt {
            lost: vec!["api".to_owned()],
        };
        let suppressed = PlatformVerdict::Suppressed {
            lost: vec!["api".to_owned()],
            window_id: Uuid::new_v4(),
        };
        assert_eq!(watch.observe(&halt), Some(PlatformNote::Lost));
        assert_eq!(watch.observe(&suppressed), None);
        assert!(watch.is_unhealthy());
    }

    #[test]
    fn a_healthy_start_records_nothing() {
        let mut watch = PlatformWatch::new();
        assert_eq!(watch.observe(&PlatformVerdict::Healthy), None);
        assert!(!watch.is_unhealthy());
    }
}
