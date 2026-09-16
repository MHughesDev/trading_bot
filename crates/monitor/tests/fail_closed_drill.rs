//! The drills (COMP-007 §8.1, §8.2, §8.3, §8.5, §8.6, §8.7, §8.8).
//!
//! COMP-007 §4.2's point is that an untested kill switch is a decoration. These are
//! the tests that make the fail-closed design an observation rather than a claim:
//! they stop the monitor for real, let its lease lapse for real, and assert that a
//! `RiskGate` built exactly as the platform builds one refuses orders afterwards.
//!
//! ```bash
//! MONITOR_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p monitor --test fail_closed_drill -- --test-threads=1
//! ```
//!
//! Every test scopes itself to a unique venue or strategy id so the suite does not
//! fight the running platform — except the two that must use `global`, which
//! restore it on the way out.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use domain::{
    money::Size,
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use monitor::control::ResetRequest;
use monitor::events::{EventKind, MonitorEvent};
use monitor::family::Family;
use monitor::lease::LeaseTerms;
use monitor::service::{LeaseWatcher, MonitorService};
use monitor::store::MonitorStore;
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate, Scope};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("MONITOR_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("API_TEST_DATABASE_URL"))
        .ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

macro_rules! db {
    () => {
        match pool().await {
            Some(p) => p,
            None => {
                eprintln!("MONITOR_TEST_DATABASE_URL unset — skipping");
                return;
            }
        }
    };
}

fn ctx() -> GateContext {
    GateContext::for_manual_order(
        Decimal::ZERO,
        None,
        Decimal::from_str("0.01").unwrap(),
        Decimal::from_str("0.001").unwrap(),
        Decimal::ZERO,
        true,
        0,
        0,
    )
}

fn intent() -> OrderIntent {
    OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    )
}

/// Build the order gate exactly as `AppState` does: a fresh mirror, plus a watcher
/// that arms the lease before its first read. This is what "as the platform builds
/// one" means, and it is why these tests can claim anything about production.
fn runtime(store: &MonitorStore) -> (Arc<KillSwitch>, RiskGate, LeaseWatcher) {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    let watcher = LeaseWatcher::new(store.clone(), Arc::clone(&ks), Duration::from_millis(50));
    (ks, gate, watcher)
}

async fn clear_scope(pg: &PgPool, scope: &str) {
    sqlx::query("DELETE FROM monitor.events WHERE scope = $1")
        .bind(scope)
        .execute(pg)
        .await
        .ok();
    sqlx::query("DELETE FROM monitor.kill_switch WHERE scope = $1")
        .bind(scope)
        .execute(pg)
        .await
        .ok();
}

// ── §8.1 / §8.2 — killing the monitor stops order flow ──────────────────────

#[tokio::test]
async fn killing_the_monitor_stops_order_flow_within_one_cadence() {
    // The drill this whole track exists to pass. A real monitor renews a real
    // lease in a real database; the runtime trades. The monitor is then stopped —
    // not told to halt, just stopped — and within one cadence the runtime refuses.
    let pg = db!();
    let store = MonitorStore::new(pg.clone());

    // A 300 ms cadence so the drill runs in under a second. The arithmetic the
    // test asserts is the same at 60 s.
    let cadence_ms = 300;
    let terms = LeaseTerms::new(format!("drill-{}", Uuid::new_v4()), cadence_ms);
    let service = MonitorService::new(store.clone(), terms.clone());
    service.tick().await.expect("the monitor renews its lease");

    let (ks, gate, watcher) = runtime(&store);
    assert!(
        !ks.monitor_available(),
        "before the first refresh the gate is closed — there is no window in which \
         a starting process trades unwatched"
    );

    watcher.refresh().await;
    assert!(ks.monitor_available(), "a live lease opens the gate");
    assert!(gate.check(intent(), &ctx()).is_ok());

    // The monitor dies here. Nothing tells the runtime; the renewal simply stops.
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(cadence_ms).unwrap() + 50,
    ))
    .await;

    watcher.refresh().await;
    assert!(
        !ks.monitor_available(),
        "one cadence after the last renewal, the lease has lapsed"
    );
    let err = gate.check(intent(), &ctx());
    assert!(
        matches!(err, Err(RiskRejection::MonitorUnavailable)),
        "expected monitor_unavailable, got {err:?}"
    );

    // And restarting does not resume orders until the lease is valid again.
    service.tick().await.expect("the monitor comes back");
    watcher.refresh().await;
    assert!(gate.check(intent(), &ctx()).is_ok());
}

#[tokio::test]
async fn a_database_the_runtime_cannot_read_closes_the_gate() {
    // §4's "fail closed" covers the case nobody writes a test for: not a monitor
    // that died, but a runtime that cannot find out either way. The pool is closed
    // under the watcher, which is as close to "the database went away" as a test
    // can get without unplugging something.
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let terms = LeaseTerms::new(format!("drill-{}", Uuid::new_v4()), 60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("renew");

    let (ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;
    assert!(gate.check(intent(), &ctx()).is_ok());

    pg.close().await;
    watcher.refresh().await;
    let err = gate.check(intent(), &ctx());
    assert!(
        matches!(err, Err(RiskRejection::MonitorUnavailable)),
        "an unreadable lease must refuse, not carry on; got {err:?}"
    );
}

// ── The restart drill the audit asked for ───────────────────────────────────

#[tokio::test]
async fn a_trip_survives_a_restart_and_the_boot_path_agrees() {
    // The bug this closes: `trip()` flipped an in-process AtomicBool and never
    // wrote the column boot reads, so a switch tripped on Monday was silently
    // cleared by the next restart — with nothing logged to say the platform had
    // decided to start trading again.
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    clear_scope(&pg, "global").await;

    let terms = LeaseTerms::new(format!("drill-{}", Uuid::new_v4()), 60_000);
    MonitorService::new(store.clone(), terms).tick().await.expect("renew");

    // Trip, as `POST /api/trading/kill` now does.
    let event = MonitorEvent::new(Scope::Global, Family::Platform, EventKind::Tripped, "human:test")
        .expect("human")
        .with_reason("restart drill");
    store.trip(&event, "restart drill").await.expect("trip");

    // "Restart": everything in-process is thrown away and rebuilt from scratch.
    let (ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;

    assert!(ks.is_active(), "the halt is still in force after a restart");
    assert!(matches!(
        gate.check(intent(), &ctx()),
        Err(RiskRejection::KillSwitchActive)
    ));

    // And the legacy boot column agrees, so `apps/platform` comes up halted too.
    let trading_enabled: bool =
        sqlx::query_scalar("SELECT trading_enabled FROM global_risk_config LIMIT 1")
            .fetch_one(&pg)
            .await
            .expect("global_risk_config");
    assert!(
        !trading_enabled,
        "the column apps/platform reads at boot must agree with the durable switch"
    );
    assert!(store.global_halted_at_boot().await);

    // Clean up: a typed, reasoned reset, which is the only way back.
    store
        .reset(&ResetRequest {
            scope: Scope::Global,
            typed_confirmation: "global".to_owned(),
            reason: "end of drill".to_owned(),
            actor: "human:test".to_owned(),
            all_venues_reconciled: true,
        })
        .await
        .expect("reset");
    clear_scope(&pg, "global").await;
}

// ── §8.3 — a venue halt is one venue ────────────────────────────────────────

#[tokio::test]
async fn an_injected_mismatch_halts_one_venue_and_blocks_resume_until_clean() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let bad = format!("kraken-{}", Uuid::new_v4());
    let good = format!("alpaca-{}", Uuid::new_v4());
    let bad_scope = Scope::venue(&bad);

    let terms = LeaseTerms::new(format!("drill-{}", Uuid::new_v4()), 60_000);
    MonitorService::new(store.clone(), terms).tick().await.expect("renew");

    let event = MonitorEvent::new(bad_scope.clone(), Family::Venue, EventKind::Halted, "monitor")
        .expect("monitor")
        .with_reason("reconciliation mismatch");
    store.trip(&event, "reconciliation mismatch").await.expect("trip");

    let (_ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;

    assert!(matches!(
        gate.check(intent(), &ctx().on_venue(&bad)),
        Err(RiskRejection::KillSwitchActive)
    ));
    assert!(
        gate.check(intent(), &ctx().on_venue(&good)).is_ok(),
        "the other venues keep trading"
    );

    // §8.3: resume is refused while a venue still fails reconciliation. A *global*
    // reset is the one that must wait for every venue to be clean, and this venue
    // is not.
    let refused = store
        .reset(&ResetRequest {
            scope: Scope::Global,
            typed_confirmation: "global".to_owned(),
            reason: "trying to resume everything".to_owned(),
            actor: "human:test".to_owned(),
            all_venues_reconciled: false,
        })
        .await;
    assert!(refused.is_err(), "a global resume must wait for reconciliation");

    clear_scope(&pg, &bad_scope.to_string()).await;
}

// ── §8.5 — the trip record exists before the first refused order ────────────

#[tokio::test]
async fn the_trip_record_is_present_before_the_first_refused_order() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let scope = Scope::strategy(&sid);

    let terms = LeaseTerms::new(format!("drill-{}", Uuid::new_v4()), 60_000);
    MonitorService::new(store.clone(), terms).tick().await.expect("renew");

    let event = MonitorEvent::new(scope.clone(), Family::Pnl, EventKind::Halted, "monitor")
        .expect("monitor")
        .with_metric("pnl_cusum", -5.4, Some(-5.0))
        .with_reason("CUSUM breach");
    store.trip(&event, "CUSUM breach").await.expect("trip");

    // The mirror has not been refreshed yet, so no order has been refused — and
    // the record is already there. That ordering is the assertion: the record is a
    // precondition of the halt, not a side effect of it.
    let recorded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM monitor.events WHERE scope = $1 AND kind = 'halted'",
    )
    .bind(scope.to_string())
    .fetch_one(&pg)
    .await
    .expect("count");
    assert_eq!(recorded, 1);

    let (_ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;
    assert!(matches!(
        gate.check(intent(), &ctx().from_strategy(&sid)),
        Err(RiskRejection::KillSwitchActive)
    ));

    // The event names the metric, the value and the limit (§8.4).
    let row: (Option<String>, Option<Decimal>, Option<Decimal>) = sqlx::query_as(
        "SELECT metric, observed_value, limit_value FROM monitor.events WHERE scope = $1",
    )
    .bind(scope.to_string())
    .fetch_one(&pg)
    .await
    .expect("event");
    assert_eq!(row.0.as_deref(), Some("pnl_cusum"));
    assert!(row.1.is_some() && row.2.is_some());

    clear_scope(&pg, &scope.to_string()).await;
}

// ── §8.6 — no agent, at any layer ───────────────────────────────────────────

#[tokio::test]
async fn the_database_refuses_an_agent_tripped_or_agent_reset_row() {
    // The layer under the API. Even a direct write — a bug in a handler, a script,
    // a future migration — cannot record an agent as having tripped or reset the
    // switch.
    let pg = db!();
    let scope = format!("strategy:agent-probe-{}", Uuid::new_v4());

    let tripped = sqlx::query(
        "INSERT INTO monitor.kill_switch (scope, state, tripped_by, reason, tripped_at)
         VALUES ($1, 'halted', 'agent:run-9', 'should be impossible', now())",
    )
    .bind(&scope)
    .execute(&pg)
    .await;
    assert!(tripped.is_err(), "tripped_by must never be agent:*");

    sqlx::query(
        "INSERT INTO monitor.kill_switch (scope, state, tripped_by, reason, tripped_at)
         VALUES ($1, 'halted', 'monitor', 'legitimate', now())",
    )
    .bind(&scope)
    .execute(&pg)
    .await
    .expect("a monitor trip is fine");

    let reset = sqlx::query(
        "UPDATE monitor.kill_switch
            SET state = 'armed', reset_by = 'agent:run-9', reset_reason = 'nope', reset_at = now()
          WHERE scope = $1",
    )
    .bind(&scope)
    .execute(&pg)
    .await;
    assert!(reset.is_err(), "reset_by must never be agent:*");

    let authored = sqlx::query(
        "INSERT INTO monitor.events (scope, family, kind, actor) VALUES ($1, 'pnl', 'tripped', 'agent:run-9')",
    )
    .bind(&scope)
    .execute(&pg)
    .await;
    assert!(authored.is_err(), "an agent must not author a monitor event");

    clear_scope(&pg, &scope).await;
}

#[tokio::test]
async fn no_agent_tripped_row_exists_anywhere() {
    // §8.6 as a standing assertion over the whole table rather than over one
    // inserted row: whatever else has happened to this database, nothing has ever
    // recorded an agent as tripping or resetting.
    let pg = db!();
    let bad: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM monitor.kill_switch
          WHERE tripped_by LIKE 'agent:%' OR reset_by LIKE 'agent:%'",
    )
    .fetch_one(&pg)
    .await
    .expect("count");
    assert_eq!(bad, 0);
}

// ── §8.7 — reset is typed, reasoned and rate-limited, against the real table ─

#[tokio::test]
async fn a_second_reset_within_five_minutes_is_refused_by_the_store() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let scope = Scope::strategy(&sid);

    let event = MonitorEvent::new(scope.clone(), Family::Pnl, EventKind::Halted, "monitor")
        .expect("monitor")
        .with_reason("first halt");
    store.trip(&event, "first halt").await.expect("trip");

    let request = ResetRequest {
        scope: scope.clone(),
        typed_confirmation: scope.to_string(),
        reason: "looked at it".to_owned(),
        actor: "human:test".to_owned(),
        all_venues_reconciled: true,
    };
    store.reset(&request).await.expect("the first reset is allowed");

    // Trip again — a flapping fault — and try to reset straight back into it.
    store.trip(&event, "flapped").await.expect("trip");
    let err = store.reset(&request).await;
    assert!(
        matches!(err, Err(monitor::store::StoreError::Reset(_))),
        "a second reset within five minutes must be refused; got {err:?}"
    );

    clear_scope(&pg, &scope.to_string()).await;
}

// ── §8.8 — no limits, no live ───────────────────────────────────────────────

#[tokio::test]
async fn a_strategy_with_no_declared_limits_cannot_be_promoted_to_live() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());

    let book = store.load_limits().await.expect("limits");
    assert!(
        book.may_promote_to_live(&sid).is_err(),
        "a strategy nobody declared limits for must not reach live"
    );

    for (metric, family, method) in [
        ("fill_rate", "execution", monitor::LimitMethod::PaperEwma3Sigma),
        ("orders_per_minute", "behaviour", monitor::LimitMethod::VenueRateCeiling),
        ("max_drawdown", "pnl", monitor::LimitMethod::McP95Drawdown),
    ] {
        store
            .declare_limit(&monitor::ControlLimit {
                strategy_id: sid.clone(),
                metric: metric.to_owned(),
                family: family.parse().expect("family"),
                severity: monitor::LimitSeverity::Breach,
                version: 1,
                lower: Some(0.5),
                upper: None,
                method,
                declared_from: "paper-run-7".to_owned(),
                declared_at: Utc::now(),
                declared_by: "human:test".to_owned(),
            })
            .await
            .expect("declare");
    }

    let book = store.load_limits().await.expect("limits");
    assert!(book.may_promote_to_live(&sid).is_ok());

    // And an agent cannot declare one, which is the same rule one layer down: a
    // limit the watched party can widen is not a control.
    let refused = store
        .declare_limit(&monitor::ControlLimit {
            strategy_id: sid.clone(),
            metric: "cancel_ratio".to_owned(),
            family: Family::Behaviour,
            severity: monitor::LimitSeverity::Breach,
            version: 1,
            lower: None,
            upper: Some(0.99),
            method: monitor::LimitMethod::DeclaredAbsolute,
            declared_from: "paper-run-7".to_owned(),
            declared_at: Utc::now(),
            declared_by: "agent:run-9".to_owned(),
        })
        .await;
    assert!(refused.is_err(), "an agent must not declare a control limit");

    sqlx::query("DELETE FROM monitor.limits WHERE strategy_id = $1")
        .bind(&sid)
        .execute(&pg)
        .await
        .ok();
}

// ── §4.3 — a window suppresses a platform trip and nothing else ─────────────

#[tokio::test]
async fn an_open_window_is_visible_attributed_and_bounded() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let window = monitor::MaintenanceWindow::open(
        "human:test",
        "rolling deploy",
        15,
        Utc::now(),
    )
    .expect("window");
    store.open_window(&window).await.expect("open");

    let open = store.open_windows().await.expect("read");
    let mine = open
        .iter()
        .find(|w| w.window_id == window.window_id)
        .expect("an open window must be visible — a window nobody can see is a hole");
    assert_eq!(mine.opened_by, "human:test");
    assert!(!mine.reason.is_empty());
    assert!(mine.suppresses(Family::Platform, Utc::now()));
    for family in Family::all() {
        if family != Family::Platform {
            assert!(
                !mine.suppresses(family, Utc::now()),
                "{family} is a fact about the market or the book, not about the deploy"
            );
        }
    }

    store.close_window(window.window_id).await.expect("close");
    sqlx::query("DELETE FROM monitor.maintenance_window WHERE window_id = $1")
        .bind(window.window_id)
        .execute(&pg)
        .await
        .ok();
}

// ── §4.2 — a slow drill is recorded as a failure, and cannot be recorded as a pass

#[tokio::test]
async fn a_slow_drill_cannot_be_recorded_as_having_passed() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());

    let slow = monitor::DrillOutcome::record(
        Scope::drill("global"),
        monitor::DrillTrigger::Deploy,
        Utc::now(),
        60_000,
        1_000,
        61_000,
        500,
        false,
        "human:test",
    )
    .expect("a drill-scoped rehearsal");
    assert!(!slow.passed());
    assert!(slow.blocks_deploy());
    store.record_drill(&slow).await.expect("record");

    // The database agrees, because `passed` is a CHECK over the arithmetic rather
    // than a boolean the writer chose.
    let lie = sqlx::query(
        "INSERT INTO monitor.drill
           (scope, trigger_kind, cadence_ms, detection_ms, propagation_ms,
            notification_ms, passed, operator)
         VALUES ('drill:global', 'deploy', 60000, 1, 61000, 1, true, 'human:test')",
    )
    .execute(&pg)
    .await;
    assert!(
        lie.is_err(),
        "a slow drill must not be recordable as a pass, however it is written"
    );

    sqlx::query("DELETE FROM monitor.drill WHERE drill_id = $1")
        .bind(slow.drill_id)
        .execute(&pg)
        .await
        .ok();
}
