//! The four fail-closed drills (Set T T-2.4; COMP-007 §8; Set S round-3 review:
//! "kill-switch drills, without which fail-closed is a claim").
//!
//! # What each drill proves, and against what
//!
//! | drill                 | asserts                                                                 |
//! |-----------------------|-------------------------------------------------------------------------|
//! | `monitor_lost`        | the real lease, if its monitor died now, closes the order gate within one cadence |
//! | `divergence_planted`  | a planted position divergence halts that strategy before its next order, and only it |
//! | `kill_switch_tripped` | a trip reaches the gate within a cadence through the real tables, every running deployment is refused, and no agent can reset it |
//! | `service_restored`    | afterwards the real gate is open again: a valid lease and no drill residue |
//!
//! None of them stops real order flow. The first reads the **real** lease row
//! and replays its expiry against a scratch gate, so it measures the production
//! numbers without waiting for the production monitor to die. The second plants
//! the divergence into a scratch gate's own halt registry through the real
//! detector. The third trips a `drill:` scope through the real
//! `monitor.kill_switch` path ([`monitor::rehearsal::Rehearsal`]), which refuses
//! to file a rehearsal that halted anything real.
//!
//! # The venue class is asserted, not assumed
//!
//! [`run_all`] refuses before it runs anything if any running deployment is not
//! paper, and `monitor.drill_run.venue_class` admits nothing but `'paper'`, so a
//! drill against live capital cannot even be recorded.
//!
//! # A failed drill is a defect, not a retry
//!
//! [`MonitorStore::record_drill_run`] files a `monitor.platform_defect` for a
//! failed drill before it writes the run, and the schema refuses a failed run
//! that names no defect. The caller logs a failure at `error`, which is the
//! alert channel that exists today.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Duration, Utc};
use domain::money::{Price, Size};
use domain::numeraire::Numeraire;
use domain::order::{OrderIntent, OrderType, Side};
use monitor::control::{ResetRefusal, ResetRequest};
use monitor::defects::{DrillKind, DrillRun};
use monitor::lease::LeaseStatus;
use monitor::store::{MonitorStore, StoreError};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate, Scope};
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

use crate::divergence::{DivergenceMonitor, DivergenceVerdict};
use crate::forward::{DeploymentSpec, DivergenceThresholds, PaperVenue};

/// How often the schedule runs the suite.
pub const DRILL_SUITE_INTERVAL_DAYS: i64 = 7;

/// The actor a scheduled run is recorded under.
pub const SCHEDULE_ACTOR: &str = "drill-schedule";

/// Why the suite refused to run at all.
#[derive(Debug, thiserror::Error)]
pub enum DrillSuiteError {
    /// A running deployment is not paper. Drills are never run where capital is real.
    #[error("deployment {deployment} runs in {environment}; drills run only where every deployment is paper")]
    NotPaper {
        deployment: String,
        environment: String,
    },
    /// An agent asked. Drills, like resets, are a human's or the schedule's.
    #[error("{0} may not run the fail-closed drills")]
    AgentRunner(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// One recorded drill.
#[derive(Clone, Debug)]
pub struct RecordedDrill {
    pub run_id: Uuid,
    pub drill: DrillKind,
    pub passed: bool,
    pub duration_ms: i64,
    pub failure: Option<String>,
    pub defect_id: Option<Uuid>,
}

/// Whether the schedule should run the suite now: any drill never run, or any
/// drill's last run older than [`DRILL_SUITE_INTERVAL_DAYS`].
#[must_use]
pub fn suite_is_due(last: &[(String, DateTime<Utc>, bool)], now: DateTime<Utc>) -> bool {
    DrillKind::all().iter().any(|d| {
        last.iter()
            .find(|(name, ..)| name == d.as_str())
            .is_none_or(|(_, at, _)| now - *at >= Duration::days(DRILL_SUITE_INTERVAL_DAYS))
    })
}

fn ctx() -> GateContext {
    GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_str("50000").expect("price")),
        Decimal::from_str("0.01").expect("tick"),
        Decimal::from_str("0.001").expect("lot"),
        Decimal::ZERO,
        true,
        0,
        0,
    )
}

fn intent(strategy_id: Option<&str>) -> OrderIntent {
    OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").expect("size"),
        None,
        strategy_id.map(ToOwned::to_owned),
    )
}

fn elapsed_ms(clock: Instant) -> i64 {
    i64::try_from(clock.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// The outcome of one drill before it is recorded.
struct Outcome {
    passed: bool,
    measurements: serde_json::Value,
    failure: Option<String>,
}

/// Drill 1: the monitor dies. Pure over the lease it is given, so it is tested
/// without a database.
fn monitor_lost(lease: Option<&monitor::lease::Lease>, now: DateTime<Utc>) -> Outcome {
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    let Some(lease) = lease else {
        ks.observe_lease(LeaseStatus::of(None, now).permits_orders());
        let refused = gate.check(intent(None), &ctx()).is_err();
        return Outcome {
            passed: refused,
            measurements: json!({
                "lease": "missing",
                "gate_refuses_now": refused,
                "note": "no monitor has renewed a lease: the gate is already closed",
            }),
            failure: (!refused)
                .then(|| "no lease exists and the gate still admitted an order".to_owned()),
        };
    };
    // The moment the monitor died is its last renewal; the gate closes when the
    // lease it left behind expires. That interval is the whole of §8.1.
    let closes_after_ms = (lease.expires_at - lease.renewed_at).num_milliseconds();
    ks.observe_lease(
        LeaseStatus::of(Some(lease), lease.expires_at - Duration::milliseconds(1)).permits_orders(),
    );
    let open_before_expiry = gate.check(intent(None), &ctx()).is_ok();
    ks.observe_lease(
        LeaseStatus::of(Some(lease), lease.expires_at + Duration::milliseconds(1)).permits_orders(),
    );
    let refused_after = gate.check(intent(None), &ctx()).is_err();
    let within_cadence = closes_after_ms <= lease.cadence_ms;
    let passed = refused_after && within_cadence;
    let failure = if !refused_after {
        Some("an expired lease did not close the order gate".to_owned())
    } else if !within_cadence {
        Some(format!(
            "the gate closes {closes_after_ms} ms after the monitor's last renewal, more than one {} ms cadence",
            lease.cadence_ms
        ))
    } else {
        None
    };
    Outcome {
        passed,
        measurements: json!({
            "lease_holder": lease.holder,
            "cadence_ms": lease.cadence_ms,
            "gate_closes_after_ms": closes_after_ms,
            "open_before_expiry": open_before_expiry,
            "refused_after_expiry": refused_after,
            "lease_valid_now": LeaseStatus::of(Some(lease), now).permits_orders(),
        }),
        failure,
    }
}

/// Drill 2: a planted position divergence. Pure: a scratch gate, the real
/// detector, the real halt registry type.
fn divergence_planted() -> Outcome {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    let strategy = format!("drill-divergence-{}", Uuid::new_v4());
    let bystander = format!("drill-bystander-{}", Uuid::new_v4());
    let spec = DeploymentSpec {
        deployment_id: Uuid::new_v4(),
        strategy_id: strategy.clone(),
        trial_id: Uuid::new_v4(),
        venue: PaperVenue::Platform,
        numeraire: Numeraire::Usd,
        bar_seconds: 60,
        thresholds: DivergenceThresholds {
            max_abs_return_divergence: 0.01,
            max_cum_return_divergence: 0.03,
            max_position_divergence: Decimal::new(1, 6),
            min_signal_match: 0.99,
        },
    };
    let mut detector = match DivergenceMonitor::new(spec, Arc::clone(gate.halt_registry())) {
        Ok(d) => d,
        Err(e) => {
            return Outcome {
                passed: false,
                measurements: json!({}),
                failure: Some(format!("the drill's own deployment spec was refused: {e}")),
            }
        }
    };
    let admitted_before = gate.check(intent(Some(&strategy)), &ctx()).is_ok();
    // `on_position` also trips the kill switch it is handed, globally — which is
    // the account-level half of a position divergence. A throwaway switch takes
    // that half, so this drill isolates the per-strategy halt.
    let account_switch = Arc::new(KillSwitch::new(false));
    let clock = Instant::now();
    let verdict = detector.on_position("BTC-USD", Decimal::ONE, Decimal::ZERO, &account_switch);
    let refused = gate.check(intent(Some(&strategy)), &ctx()).is_err();
    let halt_to_refusal_us = i64::try_from(clock.elapsed().as_micros()).unwrap_or(i64::MAX);
    let bystander_admitted = gate.check(intent(Some(&bystander)), &ctx()).is_ok();
    let halted = matches!(verdict, DivergenceVerdict::Halted(_));
    let passed = admitted_before && halted && refused && bystander_admitted;
    let failure = (!passed).then(|| {
        format!(
            "admitted before {admitted_before}, halted {halted}, refused after {refused}, \
             other strategy still trading {bystander_admitted}"
        )
    });
    Outcome {
        passed,
        measurements: json!({
            "admitted_before": admitted_before,
            "halted": halted,
            "next_order_refused": refused,
            "halt_to_refusal_us": halt_to_refusal_us,
            "other_strategy_admitted": bystander_admitted,
            "account_switch_tripped": account_switch.is_active(),
        }),
        failure,
    }
}

/// Drill 3's in-memory half: a global trip refuses every running deployment,
/// and an agent cannot reset it. Pure.
fn every_deployment_stops(strategy_ids: &[String]) -> (bool, bool, serde_json::Value) {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    let synthetic = [format!("drill-fleet-{}", Uuid::new_v4())];
    let ids: &[String] = if strategy_ids.is_empty() {
        &synthetic
    } else {
        strategy_ids
    };
    ks.trip();
    let refused: Vec<bool> = ids
        .iter()
        .map(|id| gate.check(intent(Some(id)), &ctx()).is_err())
        .collect();
    let all_refused = refused.iter().all(|r| *r);
    let agent_refused = matches!(
        ResetRequest {
            scope: Scope::Global,
            typed_confirmation: "global".to_owned(),
            reason: "drill: an agent tries to reset".to_owned(),
            actor: "agent:drill".to_owned(),
            all_venues_reconciled: true,
        }
        .validate(None, Utc::now()),
        Err(ResetRefusal::NotHuman { .. })
    );
    (
        all_refused,
        agent_refused,
        json!({
            "deployments_checked": ids.len(),
            "deployments_refused": refused.iter().filter(|r| **r).count(),
            "agent_reset_refused": agent_refused,
        }),
    )
}

/// Run all four drills, record each, and return what they found.
///
/// `run_by` is [`SCHEDULE_ACTOR`] or `human:<id>`. Each drill is recorded even
/// if a later one cannot run, so a partial suite leaves a partial record rather
/// than none.
///
/// # Errors
/// [`DrillSuiteError::NotPaper`] before anything runs, an agent runner, or a
/// store failure while recording.
pub async fn run_all(
    store: &MonitorStore,
    scheduled: bool,
    run_by: &str,
) -> Result<Vec<RecordedDrill>, DrillSuiteError> {
    if run_by.starts_with("agent:") || run_by.trim().is_empty() {
        return Err(DrillSuiteError::AgentRunner(run_by.to_owned()));
    }
    let deployments = store.running_deployments().await?;
    if let Some(d) = deployments.iter().find(|d| d.environment != "paper") {
        return Err(DrillSuiteError::NotPaper {
            deployment: d.strategy_id.clone(),
            environment: d.environment.clone(),
        });
    }
    let strategy_ids: Vec<String> = deployments.iter().map(|d| d.strategy_id.clone()).collect();
    let mut out = Vec::new();

    // 1. The monitor dies.
    let started = Utc::now();
    let lease = store.read_lease().await?;
    let cadence_ms = lease.as_ref().map_or_else(
        || i64::try_from(monitor::DEFAULT_CADENCE_MS).unwrap_or(60_000),
        |l| l.cadence_ms,
    );
    let o = monitor_lost(lease.as_ref(), started);
    out.push(record(store, DrillKind::MonitorLost, scheduled, run_by, started, o).await?);

    // 2. A divergence is planted.
    let started = Utc::now();
    let o = divergence_planted();
    out.push(
        record(
            store,
            DrillKind::DivergencePlanted,
            scheduled,
            run_by,
            started,
            o,
        )
        .await?,
    );

    // 3. The kill switch trips: through the real tables, at a drill scope.
    let started = Utc::now();
    let (all_refused, agent_refused, mut measurements) = every_deployment_stops(&strategy_ids);
    let rehearsal = monitor::rehearsal::Rehearsal::new(store.clone(), cadence_ms, run_by)
        .map(|r| r.within(cadence_ms));
    let rehearsed = match rehearsal {
        Ok(r) => r
            .run(
                if scheduled {
                    monitor::drill::DrillTrigger::Scheduled
                } else {
                    monitor::drill::DrillTrigger::Manual
                },
                "t24",
            )
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    let (trip_passed, trip_failure) = match &rehearsed {
        Ok(outcome) => {
            measurements["detection_ms"] = json!(outcome.detection_ms);
            measurements["propagation_ms"] = json!(outcome.propagation_ms);
            measurements["notification_ms"] = json!(outcome.notification_ms);
            measurements["cadence_ms"] = json!(outcome.cadence_ms);
            (
                outcome.passed(),
                (!outcome.passed()).then(|| {
                    format!(
                        "the trip took {} ms to reach the gate, more than one {} ms cadence",
                        outcome.propagation_ms, outcome.cadence_ms
                    )
                }),
            )
        }
        Err(e) => (false, Some(format!("the trip could not be rehearsed: {e}"))),
    };
    let passed = trip_passed && all_refused && agent_refused;
    let failure = if passed {
        None
    } else {
        Some(
            [
                trip_failure,
                (!all_refused)
                    .then(|| "a running deployment was admitted through a global trip".to_owned()),
                (!agent_refused).then(|| "an agent reset was not refused".to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; "),
        )
    };
    let restore_clock = Instant::now();
    out.push(
        record(
            store,
            DrillKind::KillSwitchTripped,
            scheduled,
            run_by,
            started,
            Outcome {
                passed,
                measurements,
                failure,
            },
        )
        .await?,
    );

    // 4. Service is restored: the real gate is open again.
    let started = Utc::now();
    let lease_now = store.lease_status(Utc::now()).await;
    let residue: Vec<String> = store
        .halted_scopes()
        .await?
        .into_iter()
        .filter(|s| matches!(s, Scope::Drill(_)))
        .map(|s| s.to_string())
        .collect();
    let restored = lease_now.permits_orders() && residue.is_empty();
    let o = Outcome {
        passed: restored,
        measurements: json!({
            "lease": format!("{lease_now:?}"),
            "drill_scopes_left_halted": residue,
            "restore_ms": elapsed_ms(restore_clock),
        }),
        failure: (!restored).then(|| {
            if lease_now.permits_orders() {
                "a drill left a halted scope behind".to_owned()
            } else {
                format!("the order gate is not open after the drills: lease {lease_now:?} — is tbot-monitor running?")
            }
        }),
    };
    out.push(
        record(
            store,
            DrillKind::ServiceRestored,
            scheduled,
            run_by,
            started,
            o,
        )
        .await?,
    );
    Ok(out)
}

async fn record(
    store: &MonitorStore,
    drill: DrillKind,
    scheduled: bool,
    run_by: &str,
    started_at: DateTime<Utc>,
    o: Outcome,
) -> Result<RecordedDrill, DrillSuiteError> {
    let run = DrillRun {
        drill,
        scheduled,
        started_at,
        finished_at: Utc::now(),
        passed: o.passed,
        measurements: o.measurements,
        failure: o.failure.clone(),
        run_by: run_by.to_owned(),
    };
    let (run_id, defect_id) = store.record_drill_run(&run).await?;
    if o.passed {
        tracing::info!(drill = drill.as_str(), %run_id, "fail-closed drill passed");
    } else {
        tracing::error!(drill = drill.as_str(), %run_id, ?defect_id, failure = ?o.failure,
            "FAIL-CLOSED DRILL FAILED — a platform defect has been filed");
    }
    Ok(RecordedDrill {
        run_id,
        drill,
        passed: o.passed,
        duration_ms: (run.finished_at - run.started_at).num_milliseconds().max(0),
        failure: o.failure,
        defect_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(cadence_ms: i64, lifetime_ms: i64) -> monitor::lease::Lease {
        let renewed_at = Utc::now();
        monitor::lease::Lease {
            lease_id: monitor::LEASE_ID.to_owned(),
            holder: "m-1".to_owned(),
            cadence_ms,
            renewed_at,
            expires_at: renewed_at + Duration::milliseconds(lifetime_ms),
        }
    }

    #[test]
    fn a_lease_that_expires_within_a_cadence_passes_the_monitor_lost_drill() {
        let l = lease(60_000, 60_000);
        let o = monitor_lost(Some(&l), l.renewed_at);
        assert!(o.passed, "{:?}", o.failure);
        assert_eq!(o.measurements["refused_after_expiry"], true);
        assert_eq!(o.measurements["open_before_expiry"], true);
    }

    #[test]
    fn a_lease_that_outlives_its_cadence_fails_the_monitor_lost_drill() {
        // A monitor whose lease lasts three cadences leaves the gate open for
        // three cadences after it dies — exactly what §8.1 forbids.
        let l = lease(60_000, 180_000);
        let o = monitor_lost(Some(&l), l.renewed_at);
        assert!(!o.passed);
        assert!(o.failure.unwrap().contains("more than one"));
    }

    #[test]
    fn no_lease_at_all_is_a_closed_gate() {
        let o = monitor_lost(None, Utc::now());
        assert!(o.passed);
        assert_eq!(o.measurements["lease"], "missing");
    }

    #[test]
    fn a_planted_divergence_halts_that_strategy_before_its_next_order_and_no_other() {
        let o = divergence_planted();
        assert!(o.passed, "{:?} {}", o.failure, o.measurements);
        assert_eq!(o.measurements["next_order_refused"], true);
        assert_eq!(o.measurements["other_strategy_admitted"], true);
    }

    #[test]
    fn a_global_trip_refuses_every_deployment_and_no_agent_can_reset_it() {
        let ids = vec!["d-1".to_owned(), "d-2".to_owned()];
        let (all, agent, m) = every_deployment_stops(&ids);
        assert!(all && agent, "{m}");
        assert_eq!(m["deployments_refused"], 2);
    }

    #[test]
    fn the_suite_is_due_when_any_drill_has_never_run_or_is_a_week_old() {
        let now = Utc::now();
        assert!(suite_is_due(&[], now));
        let fresh: Vec<(String, DateTime<Utc>, bool)> = DrillKind::all()
            .iter()
            .map(|d| (d.as_str().to_owned(), now - Duration::days(1), true))
            .collect();
        assert!(!suite_is_due(&fresh, now));
        let mut stale = fresh.clone();
        stale[2].1 = now - Duration::days(DRILL_SUITE_INTERVAL_DAYS);
        assert!(suite_is_due(&stale, now));
    }

    #[test]
    fn the_venue_class_is_asserted_in_code_and_in_the_schema() {
        let src = include_str!("drills.rs");
        assert!(
            src.contains("d.environment != \"paper\""),
            "run_all must refuse a non-paper deployment"
        );
        let migration = include_str!("../../../migrations/0165_monitor_measurement.sql");
        assert!(
            migration.contains("venue_class   TEXT        NOT NULL CHECK (venue_class = 'paper')"),
            "monitor.drill_run must admit only paper"
        );
    }
}
