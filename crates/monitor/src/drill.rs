//! Drills (COMP-007 §4.2).
//!
//! Nobody will trip a global halt on a live book to see whether it works, so
//! without a rehearsal mechanism this system's most important path is the one path
//! never exercised — and it will be exercised for the first time on the day it
//! matters, by someone under pressure.
//!
//! A drill runs the whole trip path against a `drill:` scope: the monitor
//! evaluates, the state machine transitions, events fire, notifications send, the
//! desk banner appears marked DRILL, and the order gate is asked — and answers —
//! refusing nothing real. It measures detection, propagation and notification
//! latency, and **a drill whose propagation exceeds one cadence fails the deploy.**
//! That is the entire assertion of §4's fail-closed design; until a drill proves
//! it, the design is a claim.

use chrono::{DateTime, Utc};
use risk::Scope;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Why a drill ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrillTrigger {
    /// After a deploy touching `crates/{monitor,risk,execution}`.
    Deploy,
    /// The monthly one, which runs regardless.
    Scheduled,
    /// Somebody asked.
    Manual,
}

impl DrillTrigger {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        }
    }

    /// Whether a failing drill under this trigger blocks the deploy.
    #[must_use]
    pub fn fails_the_deploy(self) -> bool {
        matches!(self, Self::Deploy)
    }
}

/// What a rehearsal measured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrillOutcome {
    pub drill_id: Uuid,
    pub scope: Scope,
    pub trigger: DrillTrigger,
    pub started_at: DateTime<Utc>,
    pub cadence_ms: i64,
    /// Breach to state change.
    pub detection_ms: i64,
    /// State change to the runtime refusing.
    pub propagation_ms: i64,
    /// State change to the notification landing.
    pub notification_ms: i64,
    /// Whether the rehearsal refused a real order. Must always be false; see
    /// [`DrillOutcome::record`].
    pub refused_real: bool,
    pub operator: String,
    /// What the three latencies were measured over.
    ///
    /// §4.2 names detection, propagation and *notification*, and the notification
    /// channel it describes — a desk banner and a notification rule — does not
    /// exist (ADR-S7-11). A number in that column with nothing saying what it
    /// measured is a number that will be read as the thing §4.2 described, so
    /// [`crate::rehearsal::Rehearsal`] writes down which channel it timed.
    /// `None` for a drill whose numbers were supplied rather than measured.
    pub notes: Option<String>,
}

/// Why a drill could not be recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrillRefusal {
    /// The scope was not a `drill:` scope, so the rehearsal would have been real.
    NotADrillScope { scope: String },
    /// The rehearsal refused a real order. That is an incident, not a drill, and
    /// recording it as a drill would file a production outage under "training".
    RefusedRealFlow,
    AgentOperator { operator: String },
}

impl std::fmt::Display for DrillRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotADrillScope { scope } => {
                write!(f, "a drill must run against a drill: scope, not {scope}")
            }
            Self::RefusedRealFlow => {
                f.write_str("this rehearsal refused real order flow: it is an incident, not a drill")
            }
            Self::AgentOperator { operator } => write!(f, "{operator} may not run a drill"),
        }
    }
}

impl std::error::Error for DrillRefusal {}

impl DrillOutcome {
    /// Record a rehearsal.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        scope: Scope,
        trigger: DrillTrigger,
        started_at: DateTime<Utc>,
        cadence_ms: i64,
        detection_ms: i64,
        propagation_ms: i64,
        notification_ms: i64,
        refused_real: bool,
        operator: &str,
    ) -> Result<Self, DrillRefusal> {
        if operator.starts_with("agent:") {
            return Err(DrillRefusal::AgentOperator {
                operator: operator.to_owned(),
            });
        }
        if scope.affects_real_flow() {
            return Err(DrillRefusal::NotADrillScope {
                scope: scope.to_string(),
            });
        }
        if refused_real {
            return Err(DrillRefusal::RefusedRealFlow);
        }
        Ok(Self {
            drill_id: Uuid::new_v4(),
            scope,
            trigger,
            started_at,
            cadence_ms,
            detection_ms,
            propagation_ms,
            notification_ms,
            refused_real,
            operator: operator.to_owned(),
            notes: None,
        })
    }

    /// Say what the latencies were measured over.
    #[must_use]
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// The verdict, which is arithmetic rather than opinion: propagation within one
    /// cadence, or the drill failed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.propagation_ms <= self.cadence_ms
    }

    /// Whether this result blocks the deploy that triggered it.
    #[must_use]
    pub fn blocks_deploy(&self) -> bool {
        self.trigger.fails_the_deploy() && !self.passed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("timestamp")
    }

    fn drill(propagation_ms: i64, trigger: DrillTrigger) -> DrillOutcome {
        DrillOutcome::record(
            Scope::drill("global"),
            trigger,
            at(0),
            60_000,
            1_200,
            propagation_ms,
            800,
            false,
            "human:mason",
        )
        .expect("a drill-scoped rehearsal that refused nothing real")
    }

    #[test]
    fn a_fast_drill_passes() {
        assert!(drill(900, DrillTrigger::Deploy).passed());
    }

    #[test]
    fn a_slow_drill_fails_the_deploy() {
        // COMP-007 §4.2: "a drill whose propagation latency exceeds one cadence
        // fails the deploy. That is the entire assertion of §4's fail-closed
        // design, and until a drill proves it, the design is a claim."
        let d = drill(61_000, DrillTrigger::Deploy);
        assert!(!d.passed());
        assert!(d.blocks_deploy());
    }

    #[test]
    fn the_boundary_is_inclusive_of_exactly_one_cadence() {
        assert!(drill(60_000, DrillTrigger::Deploy).passed());
        assert!(!drill(60_001, DrillTrigger::Deploy).passed());
    }

    #[test]
    fn a_slow_scheduled_drill_is_still_a_failure_but_blocks_no_deploy() {
        let d = drill(61_000, DrillTrigger::Scheduled);
        assert!(!d.passed());
        assert!(!d.blocks_deploy());
    }

    #[test]
    fn a_rehearsal_against_a_real_scope_is_refused() {
        // The adversarial one. A "drill" against `global` would halt the book, and
        // the whole point of the mechanism is that it cannot.
        for scope in [Scope::Global, Scope::venue("kraken"), Scope::strategy("s-1")] {
            let err = DrillOutcome::record(
                scope.clone(),
                DrillTrigger::Manual,
                at(0),
                60_000,
                1,
                1,
                1,
                false,
                "human:mason",
            )
            .expect_err("a real scope must be refused");
            assert!(matches!(err, DrillRefusal::NotADrillScope { .. }), "{scope}");
        }
    }

    #[test]
    fn a_rehearsal_that_refused_real_flow_cannot_be_filed_as_a_drill() {
        let err = DrillOutcome::record(
            Scope::drill("global"),
            DrillTrigger::Deploy,
            at(0),
            60_000,
            1,
            1,
            1,
            true,
            "human:mason",
        )
        .expect_err("that is an incident");
        assert_eq!(err, DrillRefusal::RefusedRealFlow);
    }

    #[test]
    fn an_agent_cannot_run_a_drill() {
        let err = DrillOutcome::record(
            Scope::drill("global"),
            DrillTrigger::Manual,
            at(0),
            60_000,
            1,
            1,
            1,
            false,
            "agent:run-9",
        )
        .expect_err("an agent must be refused");
        assert!(matches!(err, DrillRefusal::AgentOperator { .. }));
    }
}
