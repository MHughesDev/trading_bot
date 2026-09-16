//! Maintenance windows (COMP-007 §4.3).
//!
//! A rolling deploy looks exactly like a platform heartbeat loss, and §4 says a
//! heartbeat loss is a global halt. Without a window every deploy halts trading,
//! and every operator learns to pre-emptively reset the switch — which is how a
//! safety mechanism becomes a nuisance and then a habit of ignoring it.
//!
//! What a window may suppress is **not** configurable. It covers platform health
//! and nothing else: reconciliation mismatches, venue errors, data staleness and
//! every strategy-level breach are facts about the market and the book, and a
//! deploy is no reason to stop believing them.

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::family::Family;

/// Default window length. Long enough for a rolling deploy, short enough that
/// forgetting to close one is measured in minutes.
pub const DEFAULT_WINDOW_MINUTES: i64 = 15;

/// The longest a window may be opened for, enforced here and by a CHECK in
/// migration 0078. A window that can be opened for a week is a switch that can be
/// left off.
pub const MAX_WINDOW_HOURS: i64 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceWindow {
    pub window_id: Uuid,
    pub opened_by: String,
    pub reason: String,
    pub opened_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// Why a window was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowRefusal {
    ReasonMissing,
    TooLong { requested_minutes: i64 },
    NonPositiveDuration,
    AgentOpener { opened_by: String },
}

impl std::fmt::Display for WindowRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReasonMissing => f.write_str("a maintenance window requires a reason"),
            Self::TooLong { requested_minutes } => write!(
                f,
                "a maintenance window may not exceed {MAX_WINDOW_HOURS}h; {requested_minutes}m requested"
            ),
            Self::NonPositiveDuration => f.write_str("a maintenance window must last at least a minute"),
            Self::AgentOpener { opened_by } => {
                write!(f, "{opened_by} may not open a maintenance window")
            }
        }
    }
}

impl std::error::Error for WindowRefusal {}

impl MaintenanceWindow {
    /// Open a window. Bounded and attributed, or refused.
    pub fn open(
        opened_by: &str,
        reason: &str,
        minutes: i64,
        now: DateTime<Utc>,
    ) -> Result<Self, WindowRefusal> {
        if opened_by.starts_with("agent:") {
            return Err(WindowRefusal::AgentOpener {
                opened_by: opened_by.to_owned(),
            });
        }
        if reason.trim().is_empty() {
            return Err(WindowRefusal::ReasonMissing);
        }
        if minutes <= 0 {
            return Err(WindowRefusal::NonPositiveDuration);
        }
        if minutes > MAX_WINDOW_HOURS * 60 {
            return Err(WindowRefusal::TooLong {
                requested_minutes: minutes,
            });
        }
        Ok(Self {
            window_id: Uuid::new_v4(),
            opened_by: opened_by.to_owned(),
            reason: reason.to_owned(),
            opened_at: now,
            expires_at: now + Duration::minutes(minutes),
            closed_at: None,
        })
    }

    /// Whether the window is open at `now`.
    ///
    /// Auto-expiring: a window nobody closed stops suppressing on its own. The
    /// alternative — a window that stays open until someone remembers — is the
    /// exact failure this whole mechanism exists to avoid, one level up.
    #[must_use]
    pub fn is_open(&self, now: DateTime<Utc>) -> bool {
        self.closed_at.is_none() && now < self.expires_at
    }

    /// Whether this window suppresses a trip in `family` at `now`.
    #[must_use]
    pub fn suppresses(&self, family: Family, now: DateTime<Utc>) -> bool {
        self.is_open(now) && family.suppressible_by_maintenance_window()
    }
}

/// Whether any of these windows suppresses `family` at `now`, and which one.
#[must_use]
pub fn suppressing_window(
    windows: &[MaintenanceWindow],
    family: Family,
    now: DateTime<Utc>,
) -> Option<Uuid> {
    windows
        .iter()
        .find(|w| w.suppresses(family, now))
        .map(|w| w.window_id)
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
    fn a_window_suppresses_a_platform_trip() {
        assert!(window().suppresses(Family::Platform, at(60)));
    }

    #[test]
    fn a_window_suppresses_nothing_else() {
        // COMP-007 §4.3, and the adversarial half of S-7.13: a deploy is no reason
        // to stop believing the venue, the feed, or the book. Asserted over every
        // family so that a new one cannot become suppressible by omission.
        let w = window();
        for family in Family::all() {
            if family == Family::Platform {
                continue;
            }
            assert!(
                !w.suppresses(family, at(60)),
                "{family} must not be suppressible by a maintenance window"
            );
        }
    }

    #[test]
    fn a_window_expires_on_its_own() {
        let w = window();
        assert!(w.is_open(at(899)));
        assert!(!w.is_open(at(900)));
        assert!(!w.suppresses(Family::Platform, at(900)));
    }

    #[test]
    fn a_closed_window_stops_suppressing_immediately() {
        let mut w = window();
        w.closed_at = Some(at(10));
        assert!(!w.suppresses(Family::Platform, at(11)));
    }

    #[test]
    fn a_window_cannot_be_opened_for_a_week() {
        let err = MaintenanceWindow::open("human:mason", "migration", 10_080, at(0))
            .expect_err("a week-long window must be refused");
        assert!(matches!(err, WindowRefusal::TooLong { .. }));
    }

    #[test]
    fn a_window_without_a_reason_is_refused() {
        assert_eq!(
            MaintenanceWindow::open("human:mason", "  ", 15, at(0)),
            Err(WindowRefusal::ReasonMissing)
        );
    }

    #[test]
    fn an_agent_cannot_open_a_window() {
        // A window an agent can open is a global halt an agent can suppress,
        // which is the same authority as tripping the switch, spelled backwards.
        let err = MaintenanceWindow::open("agent:run-9", "deploy", 15, at(0))
            .expect_err("an agent must be refused");
        assert!(matches!(err, WindowRefusal::AgentOpener { .. }));
    }

    #[test]
    fn an_open_window_is_attributed_and_findable() {
        // §4.3: "a window nobody can see is a hole nobody can audit".
        let ws = vec![window()];
        let id = suppressing_window(&ws, Family::Platform, at(60)).expect("open");
        assert_eq!(id, ws[0].window_id);
        assert_eq!(ws[0].opened_by, "human:mason");
        assert!(!ws[0].reason.is_empty());
        assert_eq!(suppressing_window(&ws, Family::Venue, at(60)), None);
    }
}
