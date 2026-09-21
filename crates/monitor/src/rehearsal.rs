//! The drill that actually runs (COMP-007 §4.2).
//!
//! # What was missing
//!
//! [`DrillOutcome::record`] takes `detection_ms`, `propagation_ms` and
//! `notification_ms` as **arguments**, and its only caller was
//! `POST /api/monitor/drill`, which takes them from the request body. So §4.2's
//! "a drill whose propagation latency exceeds one cadence fails the deploy" was
//! arithmetic over three numbers the caller typed. Nothing anywhere tripped a
//! scope, watched the mirror, or timed anything: the mechanism that exists so the
//! fail-closed design is "an observation rather than a claim" was itself a claim.
//!
//! [`Rehearsal`] is the missing half. It trips a `drill:` scope durably, refreshes
//! an order-gate mirror from the database exactly as [`LeaseWatcher::refresh`][r]
//! does, and measures how long each step took — then hands the measured numbers
//! to `DrillOutcome::record`, so every refusal that type already enforces still
//! applies.
//!
//! [r]: crate::service::LeaseWatcher::refresh
//!
//! # The three numbers, and what each one actually measures
//!
//! - **Detection** — the breach to the state change: the durable trip committed,
//!   which is the moment the platform's answer changed.
//! - **Propagation** — the state change to the gate refusing. Measured by reading
//!   `monitor.kill_switch` and pushing it into a [`KillSwitch`] through
//!   `replace_halted`, then asking `is_active_scope` — the same three calls the
//!   watched process makes, over the same table, against the same mirror type.
//!   What it does *not* include is the watched process's own polling interval,
//!   which is why the verdict compares the result against `cadence_ms` rather
//!   than against zero.
//! - **Notification** — the state change to the record becoming readable on the
//!   surface the operator actually has. That surface is `monitor.events` through
//!   `GET /api/monitor/events`: there is no NATS publication and no desk
//!   subscriber (ADR-S7-11 declared that deviation), so this measures the channel
//!   that exists and [`DrillOutcome::notes`] records which one, rather than
//!   reporting a zero for a channel nobody built. A zero there would read as "the
//!   notification landed instantly", which is the dangerous direction.
//!
//! # Nothing real is touched, and that is checked rather than asserted
//!
//! The scope is a [`Scope::Drill`], which `KillSwitch::refuses` never consults.
//! But "the rehearsal refused nothing real" is measured, not assumed: the halted
//! set is read before and after, and any *real* scope that appears in between
//! fails the drill as an incident.

use std::time::Instant;

use chrono::Utc;
use risk::{KillSwitch, Scope};
use uuid::Uuid;

use crate::drill::{DrillOutcome, DrillRefusal, DrillTrigger};
use crate::events::{EventKind, MonitorEvent};
use crate::family::Family;
use crate::service::MONITOR_ACTOR;
use crate::store::{MonitorStore, StoreError};

/// How long a rehearsal waits for the mirror before giving up, as a multiple of
/// the cadence.
///
/// Two, from §4's "monitor lag > 2 cadences" — the point past which the platform
/// treats the monitor as gone. Waiting longer would only produce a more precise
/// measurement of a failure that was already certain at one cadence.
pub const PROPAGATION_TIMEOUT_CADENCES: u32 = 2;

/// How often the rehearsal re-reads the database while waiting.
const POLL_MS: u64 = 20;

/// The channel [`Rehearsal`] measures notification latency over, recorded on the
/// drill so the number cannot later be read as something it is not.
pub const NOTIFICATION_CHANNEL: &str =
    "notification measured as the trip becoming readable in monitor.events \
     (GET /api/monitor/events); no NATS publication or desk subscriber exists (ADR-S7-11)";

/// Why a rehearsal could not be run, or could not be believed.
#[derive(Debug, thiserror::Error)]
pub enum RehearsalError {
    #[error("database: {0}")]
    Store(#[from] StoreError),
    /// The trip never reached the mirror. **Not** recordable as a slow drill: a
    /// propagation latency that was never observed is not a large one, and
    /// writing the timeout in its place would put a measured-looking number in
    /// the column the deploy gate reads.
    #[error(
        "the drill scope {scope} never reached the order gate's mirror after {waited_ms}ms \
         ({PROPAGATION_TIMEOUT_CADENCES} cadences); propagation was not measured, so this \
         drill has no verdict"
    )]
    PropagationNotObserved { scope: String, waited_ms: i64 },
    /// The record never became readable where the operator reads. Same rule as
    /// above and for the same reason: an unobserved latency is not a long one.
    #[error(
        "the drill record for {scope} was not readable on the event surface after {waited_ms}ms; \
         notification was not measured, so this drill has no verdict"
    )]
    NotificationNotObserved { scope: String, waited_ms: i64 },
    /// A real scope became halted during the rehearsal.
    #[error("the rehearsal halted real scopes {scopes:?}: that is an incident, not a drill")]
    RefusedRealFlow { scopes: Vec<String> },
    #[error(transparent)]
    Refused(#[from] DrillRefusal),
}

/// One rehearsal of the whole trip path.
///
/// Owns its own [`KillSwitch`] mirror. That is not a shortcut around the process
/// boundary — a mirror is per-process by construction, the watched process builds
/// exactly this type the same way, and what the drill is rehearsing is the path
/// from the durable row to a mirror, which is the part that can break. The mirror
/// is deliberately **not** armed with `require_monitor_lease`: arming is
/// irreversible and a rehearsal must leave nothing behind.
pub struct Rehearsal {
    store: MonitorStore,
    gate: KillSwitch,
    cadence_ms: i64,
    deadline_ms: i64,
    operator: String,
}

impl Rehearsal {
    /// Build a rehearsal.
    ///
    /// # Errors
    /// Refuses an `agent:` operator here as well as in [`DrillOutcome::record`],
    /// so a rehearsal an agent asked for cannot even start.
    pub fn new(
        store: MonitorStore,
        cadence_ms: i64,
        operator: &str,
    ) -> Result<Self, DrillRefusal> {
        if operator.starts_with("agent:") {
            return Err(DrillRefusal::AgentOperator {
                operator: operator.to_owned(),
            });
        }
        Ok(Self {
            store,
            gate: KillSwitch::new(false),
            cadence_ms,
            deadline_ms: cadence_ms.saturating_mul(i64::from(PROPAGATION_TIMEOUT_CADENCES)),
            operator: operator.to_owned(),
        })
    }

    /// Give up after `deadline_ms` instead of after two cadences.
    ///
    /// **The monitor's own loop must set this, and it is not a tuning knob.** A
    /// rehearsal blocks whoever is awaiting it, and the monitor renews its lease
    /// on the same loop — so a rehearsal allowed to wait two cadences would lapse
    /// the lease of the process running it and halt the whole platform while
    /// rehearsing how the platform is halted. That is the funniest possible
    /// outage and it would be entirely self-inflicted, so
    /// [`MonitorService::run_scheduled_drill`][s] passes a bound strictly inside
    /// its renewal interval and a test holds it there.
    ///
    /// The judging bar is unaffected: `passed()` still compares propagation
    /// against `cadence_ms`. A deadline only decides when the rehearsal stops
    /// waiting and reports *no verdict*, which is never a claim that the drill
    /// failed.
    ///
    /// [s]: crate::service::MonitorService::run_scheduled_drill
    #[must_use]
    pub fn within(mut self, deadline_ms: i64) -> Self {
        self.deadline_ms = deadline_ms.max(1);
        self
    }

    /// The bound this rehearsal will not wait past.
    #[must_use]
    pub fn deadline_ms(&self) -> i64 {
        self.deadline_ms
    }

    /// Run the whole path and record what it measured.
    ///
    /// The label is only a suffix; the scope is always a [`Scope::Drill`] and is
    /// built here rather than taken, so no caller can hand this a real one. It is
    /// made unique so two rehearsals cannot observe each other's rows.
    ///
    /// # Errors
    /// [`RehearsalError`], and a timed-out rehearsal is an error rather than a
    /// failed drill — see [`RehearsalError::PropagationNotObserved`].
    pub async fn run(
        &self,
        trigger: DrillTrigger,
        label: &str,
    ) -> Result<DrillOutcome, RehearsalError> {
        let scope = Scope::drill(format!("{label}-{}", Uuid::new_v4()));
        let before = self.store.halted_scopes().await?;
        let started_at = Utc::now();
        let clock = Instant::now();

        // 1. Detection: the breach to the state change. The trip writes the event
        //    and the switch row in one transaction, record first (§8.5), so the
        //    moment it commits is the moment the platform's answer changed.
        let event = MonitorEvent::new(scope.clone(), Family::Platform, EventKind::Drill, MONITOR_ACTOR)
            .map_err(|e| StoreError::Invalid(e.to_string()))?
            .with_reason(format!(
                "drill ({}) — rehearsing the trip path against {scope}",
                trigger.as_str()
            ));
        self.store.trip(&event, "drill rehearsal").await?;
        let detection_ms = elapsed_ms(clock);

        // 2. Propagation: the state change to the gate refusing, over the same
        //    table, the same read and the same mirror call the watched process
        //    uses.
        let timeout_ms = self.deadline_ms;
        let mut propagation_ms = None;
        let mut after = before.clone();
        while elapsed_ms(clock) <= timeout_ms {
            after = self.store.halted_scopes().await?;
            self.gate.replace_halted(&after);
            if self.gate.is_active_scope(&scope) {
                propagation_ms = Some(elapsed_ms(clock).saturating_sub(detection_ms));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
        }

        // 3. Notification: the record becoming readable where the operator reads.
        let mut notification_ms = None;
        while elapsed_ms(clock) <= timeout_ms {
            if self.store.event_is_visible(event.event_id).await? {
                notification_ms = Some(elapsed_ms(clock).saturating_sub(detection_ms));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
        }

        // Whatever happened, put the scope back and clear the mirror. A drill that
        // left a halted row behind would be indistinguishable from a real trip on
        // every surface that lists them.
        let end = self.store.end_drill(&scope).await;
        if let Ok(scopes) = self.store.halted_scopes().await {
            self.gate.replace_halted(&scopes);
        }
        end?;

        // Measured, not assumed: any real scope that became halted while the
        // rehearsal ran is an incident, and filing it as a drill would put a
        // production outage under "training".
        let real: Vec<String> = after
            .iter()
            .filter(|s| s.affects_real_flow() && !before.contains(s))
            .map(ToString::to_string)
            .collect();
        if !real.is_empty() {
            return Err(RehearsalError::RefusedRealFlow { scopes: real });
        }

        let Some(propagation_ms) = propagation_ms else {
            return Err(RehearsalError::PropagationNotObserved {
                scope: scope.to_string(),
                waited_ms: timeout_ms,
            });
        };
        let Some(notification_ms) = notification_ms else {
            return Err(RehearsalError::NotificationNotObserved {
                scope: scope.to_string(),
                waited_ms: timeout_ms,
            });
        };

        let outcome = DrillOutcome::record(
            scope,
            trigger,
            started_at,
            self.cadence_ms,
            detection_ms,
            propagation_ms,
            notification_ms,
            false,
            &self.operator,
        )?
        .with_notes(NOTIFICATION_CHANNEL);
        self.store.record_drill(&outcome).await?;
        Ok(outcome)
    }
}

/// Whole milliseconds since `clock`, saturating into the type the schema stores.
fn elapsed_ms(clock: Instant) -> i64 {
    i64::try_from(clock.elapsed().as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_agent_cannot_even_start_a_rehearsal() {
        // Two refusals for one rule, and deliberately so: `DrillOutcome::record`
        // refuses at the end, which is after the rehearsal has already tripped a
        // scope and written rows. This one refuses before anything happens.
        let store = MonitorStore::new(sqlx::PgPool::connect_lazy("postgres://x/y").expect("lazy"));
        match Rehearsal::new(store, 60_000, "agent:run-9") {
            Err(DrillRefusal::AgentOperator { operator }) => assert_eq!(operator, "agent:run-9"),
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(_) => panic!("an agent must be refused a rehearsal"),
        }
    }

    #[test]
    fn a_rehearsal_names_the_channel_its_notification_number_came_from() {
        // The number is real and it is not the number §4.2 imagined. A drill row
        // that carried `notification_ms` with no note would be read as the desk
        // banner's latency, which nothing measures because no desk subscribes.
        assert!(NOTIFICATION_CHANNEL.contains("monitor.events"));
        assert!(NOTIFICATION_CHANNEL.contains("ADR-S7-11"));
    }

    #[test]
    fn a_rehearsal_that_never_propagated_has_no_verdict_rather_than_a_bad_one() {
        // The error exists so that a timeout cannot be written into
        // `propagation_ms` and then compared against the cadence. `passed()` is
        // `propagation_ms <= cadence_ms`, so a 120_000ms timeout recorded as a
        // measurement would *look* like a measured failure — and, worse, a
        // shorter timeout would look like a measured pass.
        let err = RehearsalError::PropagationNotObserved {
            scope: "drill:global-1".to_owned(),
            waited_ms: 120_000,
        };
        let text = err.to_string();
        assert!(text.contains("drill:global-1"), "{text}");
        assert!(text.contains("no verdict"), "{text}");
    }

    #[tokio::test]
    async fn a_rehearsal_on_the_monitors_loop_cannot_cost_it_its_lease() {
        // The hazard this bound exists for, asserted over a spread of cadences
        // rather than at one. A rehearsal blocks whoever awaits it and the
        // monitor renews its lease on that same loop, so a rehearsal that could
        // outlast a renewal would halt the entire platform — for real — while
        // rehearsing how the platform is halted.
        for cadence_ms in [1_000, 5_000, 60_000, 300_000] {
            let terms = crate::lease::LeaseTerms::new("holder".to_owned(), cadence_ms);
            let store =
                MonitorStore::new(sqlx::PgPool::connect_lazy("postgres://x/y").expect("lazy"));
            let rehearsal = Rehearsal::new(store, cadence_ms, MONITOR_ACTOR)
                .expect("the monitor is not an agent")
                .within(crate::service::MonitorService::drill_deadline_ms(&terms));
            assert!(
                rehearsal.deadline_ms() < terms.renew_interval_ms(),
                "at a {cadence_ms}ms cadence a rehearsal may wait {}ms while the lease is \
                 renewed every {}ms",
                rehearsal.deadline_ms(),
                terms.renew_interval_ms()
            );
            assert!(rehearsal.deadline_ms() >= 1, "a deadline of zero never measures anything");
        }
    }

    #[tokio::test]
    async fn a_rehearsal_nobody_bounds_waits_the_two_cadences_the_platform_allows() {
        // The default, for a caller that is *not* holding a lease — CI, or the
        // API route. Two cadences is §4's "monitor lag > 2 cadences", the point
        // past which the runtime has already given the monitor up.
        let store = MonitorStore::new(sqlx::PgPool::connect_lazy("postgres://x/y").expect("lazy"));
        let rehearsal = Rehearsal::new(store, 60_000, "human:mason").expect("human");
        assert_eq!(rehearsal.deadline_ms(), 120_000);
    }

    #[test]
    fn the_timeout_is_the_lag_the_platform_already_calls_dead() {
        // COMP-007 §4: "monitor lag > 2 cadences" is a global halt. A rehearsal
        // that waited longer than that would be measuring a platform the runtime
        // has already given up on.
        assert_eq!(PROPAGATION_TIMEOUT_CADENCES, 2);
    }
}
