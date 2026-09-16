//! The event record (COMP-007 §6, P5).
//!
//! Trip, scale, halt, reset: who or what, which metric, which limit, the values,
//! the strategy, the time. This row is the UI's timeline and the audit's evidence,
//! and §8.5 requires it to exist *before* the first refused order — so it is
//! written as part of the same transaction that changes the state, not afterwards
//! as a log line.

use chrono::{DateTime, Utc};
use risk::Scope;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::family::Family;

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A single-cadence excursion. Recorded, acted on by nothing (§4.1).
    Blip,
    Warning,
    Breach,
    ScaledDown,
    ScaleReleased,
    Halted,
    Resumed,
    Tripped,
    Reset,
    WindowOpened,
    WindowClosed,
    Drill,
}

impl EventKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blip => "blip",
            Self::Warning => "warning",
            Self::Breach => "breach",
            Self::ScaledDown => "scaled_down",
            Self::ScaleReleased => "scale_released",
            Self::Halted => "halted",
            Self::Resumed => "resumed",
            Self::Tripped => "tripped",
            Self::Reset => "reset",
            Self::WindowOpened => "window_opened",
            Self::WindowClosed => "window_closed",
            Self::Drill => "drill",
        }
    }

    /// Whether this kind counts toward the monitor's own precision (§4.1).
    ///
    /// A blip is not an alarm and must not be scored as one — counting them would
    /// bury every family's precision in readings nobody was ever shown. A drill is
    /// not an alarm either: it is a rehearsal the platform asked for.
    #[must_use]
    pub fn is_scoreable_alarm(self) -> bool {
        matches!(
            self,
            Self::Warning | Self::Breach | Self::ScaledDown | Self::Halted | Self::Tripped
        )
    }
}

/// One state change.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorEvent {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub scope: Scope,
    pub family: Family,
    pub kind: EventKind,
    pub metric: Option<String>,
    pub observed_value: Option<f64>,
    pub limit_value: Option<f64>,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    /// `monitor` | `human:<user>` | `drill`. Never `agent:*`.
    pub actor: String,
    pub reason: Option<String>,
    pub is_drill: bool,
    pub suppressed_by_window: Option<Uuid>,
}

/// An actor string an agent supplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentActorRefused(pub String);

impl std::fmt::Display for AgentActorRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} may not author a monitor event", self.0)
    }
}

impl std::error::Error for AgentActorRefused {}

impl MonitorEvent {
    /// Build an event.
    ///
    /// Fallible on exactly one thing: the actor. Making it a `Result` rather than a
    /// silent sanitisation means a caller that somehow assembled an `agent:` actor
    /// has to deal with it, instead of finding out later that the row it thought it
    /// wrote is not in the table.
    pub fn new(
        scope: Scope,
        family: Family,
        kind: EventKind,
        actor: &str,
    ) -> Result<Self, AgentActorRefused> {
        if actor.starts_with("agent:") {
            return Err(AgentActorRefused(actor.to_owned()));
        }
        let is_drill = matches!(scope, Scope::Drill(_));
        Ok(Self {
            event_id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            scope,
            family,
            kind,
            metric: None,
            observed_value: None,
            limit_value: None,
            from_state: None,
            to_state: None,
            actor: actor.to_owned(),
            reason: None,
            is_drill,
            suppressed_by_window: None,
        })
    }

    #[must_use]
    pub fn with_metric(mut self, metric: &str, value: f64, limit: Option<f64>) -> Self {
        self.metric = Some(metric.to_owned());
        self.observed_value = Some(value);
        self.limit_value = limit;
        self
    }

    #[must_use]
    pub fn with_states(mut self, from: &str, to: &str) -> Self {
        self.from_state = Some(from.to_owned());
        self.to_state = Some(to.to_owned());
        self
    }

    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    #[must_use]
    pub fn suppressed_by(mut self, window_id: Uuid) -> Self {
        self.suppressed_by_window = Some(window_id);
        self
    }

    /// Whether this row carries everything §4's acceptance asks an event to carry.
    #[must_use]
    pub fn names_metric_value_limit_and_scope(&self) -> bool {
        self.metric.is_some() && self.observed_value.is_some() && self.limit_value.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_breach_event_names_the_metric_the_value_and_the_limit() {
        // COMP-007 §8.4: "the event names the metric, values and limit". An alarm
        // that says only "breach" costs the operator the first five minutes of
        // every incident.
        let e = MonitorEvent::new(
            Scope::strategy("s-1"),
            Family::Execution,
            EventKind::ScaledDown,
            "monitor",
        )
        .expect("monitor is not an agent")
        .with_metric("fill_rate", 0.41, Some(0.80));
        assert!(e.names_metric_value_limit_and_scope());
        assert_eq!(e.scope.to_string(), "strategy:s-1");
    }

    #[test]
    fn an_agent_cannot_author_an_event() {
        // COMP-007 §8.6, at the type level as well as in the CHECK constraint.
        let err = MonitorEvent::new(Scope::Global, Family::Pnl, EventKind::Tripped, "agent:run-9")
            .expect_err("an agent actor must be refused");
        assert_eq!(err.0, "agent:run-9");
    }

    #[test]
    fn a_drill_scope_marks_the_event_a_drill_without_being_asked() {
        // So that a rehearsal cannot be recorded as a real trip by forgetting a
        // flag — which is how a drill ends up in the precision numerator and in
        // the incident review.
        let e = MonitorEvent::new(Scope::drill("global"), Family::Platform, EventKind::Drill, "human:mason")
            .expect("human");
        assert!(e.is_drill);
    }

    #[test]
    fn a_real_scope_is_never_marked_a_drill() {
        let e = MonitorEvent::new(Scope::Global, Family::Platform, EventKind::Tripped, "monitor")
            .expect("monitor");
        assert!(!e.is_drill);
    }

    #[test]
    fn blips_and_drills_do_not_count_toward_the_monitors_precision() {
        assert!(!EventKind::Blip.is_scoreable_alarm());
        assert!(!EventKind::Drill.is_scoreable_alarm());
        assert!(!EventKind::Reset.is_scoreable_alarm());
        assert!(EventKind::Breach.is_scoreable_alarm());
        assert!(EventKind::Halted.is_scoreable_alarm());
    }
}
