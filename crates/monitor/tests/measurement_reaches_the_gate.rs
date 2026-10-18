//! The 0165 measurements, against a real database: each one ends in a table
//! `LeaseWatcher::refresh` reads, or in a record a human acts on.
//!
//! ```text
//! MONITOR_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p monitor --test measurement_reaches_the_gate -- --test-threads=1
//! ```
//!
//! Without the variable every test here prints that it skipped and returns.
//! That is **not** a pass, and the phase notes say which database these ran on.
//!
//! Every test scopes itself to unique ids, and leaves the database as it found
//! it: rows it can delete it deletes, and an append-only verdict it had to write
//! ends on `agreed`, so no residue can refuse a later global reset.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use monitor::control::ResetRequest;
use monitor::defects::{DefectSource, Filed, NewDefect};
use monitor::feed::{FeedKey, Venue};
use monitor::lease::LeaseTerms;
use monitor::reconciled::VenueVerdict;
use monitor::service::{Cadence, LeaseWatcher, MonitorService};
use monitor::store::{MonitorStore, StoreError};
use risk::{KillSwitch, Scope};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("MONITOR_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("API_TEST_DATABASE_URL"))
        .ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

/// The same database, as the running platform reaches it: `app_role`.
///
/// Owner-connected tests cannot see a missing GRANT (ADR-P0-13; 0145). Every
/// write the platform process makes to a 0165 table is repeated through this.
async fn app_pool() -> Option<PgPool> {
    let url = std::env::var("MONITOR_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("API_TEST_DATABASE_URL"))
        .ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(2)
            .after_connect(|conn, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE app_role").execute(conn).await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .expect("connect as app_role"),
    )
}

macro_rules! db {
    () => {
        match pool().await {
            Some(p) => p,
            None => {
                eprintln!("MONITOR_TEST_DATABASE_URL unset — skipping (this is not a pass)");
                return;
            }
        }
    };
}

fn feed(symbol: &str, venue: &str) -> FeedKey {
    FeedKey::new(symbol, Venue::named(venue).expect("venue")).expect("feed")
}

async fn deploy(pg: &PgPool, instrument: &str) -> String {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json,
            instrument_id, environment, venue, starting_capital, started_by)
         VALUES ($1, 'monitor-test', $2, $3, '{}', $4, 'paper', 'platform_paper', 1000, 'human:test')",
    )
    .bind(id)
    .bind(format!("staleness-{id}"))
    .bind("a".repeat(64))
    .bind(instrument)
    .execute(pg)
    .await
    .expect("deployment row");
    id.to_string()
}

async fn cleanup_deployment(pg: &PgPool, id: &str) {
    let scope = format!("strategy:{id}");
    let _ = sqlx::query("DELETE FROM monitor.events WHERE scope = $1").bind(&scope).execute(pg).await;
    let _ = sqlx::query("DELETE FROM monitor.kill_switch WHERE scope = $1").bind(&scope).execute(pg).await;
    let _ = sqlx::query("DELETE FROM monitor.strategy_state WHERE strategy_id = $1").bind(id).execute(pg).await;
    let _ = sqlx::query("UPDATE strategy_deployment SET stopped_at = now(), stop_reason = 'test' WHERE deployment_id = $1::uuid AND stopped_at IS NULL")
        .bind(id)
        .execute(pg)
        .await;
}

#[tokio::test]
async fn a_hard_stale_feed_halts_the_deployment_on_it_through_the_gate_and_no_other() {
    let pg = db!();
    let app = app_pool().await.expect("app pool");
    let symbol = format!("STALE-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let other = format!("FRESH-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let on_stale = deploy(&pg, &symbol).await;
    let on_fresh = deploy(&pg, &other).await;

    // The producer's writes, as the platform makes them: app_role.
    let producer = MonitorStore::new(app.clone());
    producer
        .observe_bar(&feed(&symbol, "kraken"), 60, Utc::now() - chrono::Duration::seconds(700), true)
        .await
        .expect("app_role can write the gauge");
    producer
        .observe_bar(&feed(&other, "kraken"), 60, Utc::now() - chrono::Duration::seconds(5), true)
        .await
        .expect("gauge");
    producer
        .affirm_feeds(&[(feed(&symbol, "kraken"), true)])
        .await
        .expect("app_role can affirm the gauge");

    let service = MonitorService::new(MonitorStore::new(pg.clone()), LeaseTerms::new("t", 60_000));
    let mut cadence = Cadence::default();
    let pass = service.assess_feeds(&mut cadence, Utc::now()).await.expect("assess");
    assert!(pass.halted.contains(&on_stale), "{pass:?}");
    assert!(!pass.halted.contains(&on_fresh), "{pass:?}");

    // It reached the gate's table, so the watched process's refresh carries it.
    let gate = Arc::new(KillSwitch::new(false));
    let watcher = LeaseWatcher::new(MonitorStore::new(app.clone()), Arc::clone(&gate), Duration::from_secs(5));
    watcher.refresh().await;
    assert!(gate.is_active_scope(&Scope::strategy(on_stale.as_str())), "the halt must survive the mirror refresh");
    assert!(!gate.is_active_scope(&Scope::strategy(on_fresh.as_str())));

    // The record names the instrument, the venue and the age.
    let reason: String = sqlx::query_scalar(
        "SELECT reason FROM monitor.events WHERE scope = $1 AND family = 'data' AND kind = 'halted'",
    )
    .bind(format!("strategy:{on_stale}"))
    .fetch_one(&pg)
    .await
    .expect("one halt event");
    assert!(reason.contains(&symbol) && reason.contains("kraken") && reason.contains("700"), "{reason}");

    // A second cadence over the same stale feed is the same incident.
    service.assess_feeds(&mut cadence, Utc::now()).await.expect("assess again");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM monitor.events WHERE scope = $1 AND kind = 'halted'",
    )
    .bind(format!("strategy:{on_stale}"))
    .fetch_one(&pg)
    .await
    .expect("count");
    assert_eq!(events, 1, "a stale feed persisting for two cadences wrote two trip records");

    for s in [&symbol, &other] {
        let _ = sqlx::query("DELETE FROM monitor.feed_freshness WHERE symbol = $1").bind(s).execute(&pg).await;
    }
    cleanup_deployment(&pg, &on_stale).await;
    cleanup_deployment(&pg, &on_fresh).await;
}

#[tokio::test]
async fn a_shut_session_halts_nothing_however_old_the_bar() {
    let pg = db!();
    let symbol = format!("SHUT-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let d = deploy(&pg, &symbol).await;
    let store = MonitorStore::new(pg.clone());
    store
        .observe_bar(&feed(&symbol, "alpaca"), 60, Utc::now() - chrono::Duration::hours(15), false)
        .await
        .expect("gauge");
    let service = MonitorService::new(store, LeaseTerms::new("t", 60_000));
    let pass = service.assess_feeds(&mut Cadence::default(), Utc::now()).await.expect("assess");
    assert!(!pass.halted.contains(&d));
    assert!(pass.session_closed >= 1);
    let _ = sqlx::query("DELETE FROM monitor.feed_freshness WHERE symbol = $1").bind(&symbol).execute(&pg).await;
    cleanup_deployment(&pg, &d).await;
}

#[tokio::test]
async fn a_venue_halted_for_a_mismatch_resumes_only_after_a_reconciliation_agrees() {
    let pg = db!();
    let app = app_pool().await.expect("app pool");
    let venue = format!("recvenue{}", &Uuid::new_v4().simple().to_string()[..8]);
    let store = MonitorStore::new(app.clone());
    let gate = KillSwitch::new(false);

    // Raised as the platform raises it: through carry_out_mismatch, as app_role.
    let halted = store.carry_out_mismatch(&venue, &gate).await.expect("mismatch recorded");
    assert_eq!(halted, Scope::venue(venue.as_str()));
    assert!(!store.venue_reconciled(&venue).await.expect("read"));

    let request = ResetRequest {
        scope: Scope::venue(venue.as_str()),
        typed_confirmation: format!("venue:{venue}"),
        reason: "I looked and it seems fine".to_owned(),
        actor: "human:test".to_owned(),
        // The caller's claim, which the store discards.
        all_venues_reconciled: true,
    };
    match store.reset(&request).await {
        Err(StoreError::Reset(monitor::control::ResetRefusal::VenueNotReconciled { venue: v })) => {
            assert_eq!(v, venue);
        }
        other => panic!("a venue whose last reconciliation disagreed was resumable: {other:?}"),
    }

    // While it disagrees, a global reset is refused too — by the verdict, not by
    // the "no venue halted" proxy.
    let global = ResetRequest {
        scope: Scope::Global,
        typed_confirmation: "global".to_owned(),
        reason: "x".to_owned(),
        actor: "human:test".to_owned(),
        all_venues_reconciled: true,
    };
    assert!(matches!(
        store.reset(&global).await,
        Err(StoreError::Reset(monitor::control::ResetRefusal::VenuesNotReconciled))
    ));

    // The sweep agrees; now a human may resume it.
    store
        .record_venue_check(&venue, VenueVerdict::Agreed, 3, None, "reconciliation")
        .await
        .expect("app_role can record a verdict");
    store.reset(&request).await.expect("an agreed venue resumes");
    let state: String = sqlx::query_scalar("SELECT state FROM monitor.kill_switch WHERE scope = $1")
        .bind(format!("venue:{venue}"))
        .fetch_one(&pg)
        .await
        .expect("row");
    assert_eq!(state, "armed");

    let _ = sqlx::query("DELETE FROM monitor.events WHERE scope = $1").bind(format!("venue:{venue}")).execute(&pg).await;
    let _ = sqlx::query("DELETE FROM monitor.kill_switch WHERE scope = $1").bind(format!("venue:{venue}")).execute(&pg).await;
}

#[tokio::test]
async fn verdicts_are_append_only() {
    let pg = db!();
    let venue = format!("appendonly{}", &Uuid::new_v4().simple().to_string()[..8]);
    let store = MonitorStore::new(pg.clone());
    store.record_venue_check(&venue, VenueVerdict::Agreed, 1, None, "reconciliation").await.expect("insert");
    let err = sqlx::query("UPDATE monitor.venue_reconciliation SET verdict = 'disagreed' WHERE venue = $1")
        .bind(&venue)
        .execute(&pg)
        .await;
    assert!(err.is_err(), "a verdict could be edited after the fact");
    // And a non-clean verdict must say why.
    let mut tx = pg.begin().await.expect("tx");
    let r = sqlx::query(
        "INSERT INTO monitor.venue_reconciliation (venue, verdict, instruments_checked, checked_by)
         VALUES ($1, 'disagreed', 1, 'reconciliation')",
    )
    .bind(&venue)
    .execute(&mut *tx)
    .await;
    assert!(r.is_err(), "a disagreement with no detail was stored");
}

#[tokio::test]
async fn a_defect_is_filed_once_per_condition_and_resolved_only_by_a_person() {
    let pg = db!();
    let app = app_pool().await.expect("app pool");
    let store = MonitorStore::new(app);
    let key = format!("test:{}", Uuid::new_v4());
    let d = NewDefect {
        source: DefectSource::FleetCommonMode,
        dedupe_key: key.clone(),
        summary: "four deployments missed the same way".to_owned(),
        evidence: serde_json::json!({ "n": 4 }),
        filed_by: "reconciliation".to_owned(),
    };
    let first = store.file_defect(&d).await.expect("app_role files");
    let Filed::New(id) = first else { panic!("first filing must be new: {first:?}") };
    assert_eq!(store.file_defect(&d).await.expect("again"), Filed::AlreadyOpen(id));
    assert!(store.resolve_defect(id, "monitor", "auto").await.is_err(), "a machine resolved a defect");
    assert!(store.resolve_defect(id, "human:test", "replayed; cost model fixed in 0170").await.expect("resolve"));
    // Resolved once; and now the same condition files a new defect.
    assert!(matches!(store.file_defect(&d).await.expect("refile"), Filed::New(_)));
    let _ = sqlx::query("ALTER TABLE monitor.platform_defect DISABLE TRIGGER platform_defect_resolve_once")
        .execute(&pg)
        .await;
    let _ = sqlx::query("DELETE FROM monitor.platform_defect WHERE dedupe_key = $1").bind(&key).execute(&pg).await;
    let _ = sqlx::query("ALTER TABLE monitor.platform_defect ENABLE TRIGGER platform_defect_resolve_once")
        .execute(&pg)
        .await;
}

#[tokio::test]
async fn a_failed_drill_run_must_name_its_defect_and_only_paper_can_be_drilled() {
    let pg = db!();
    // Inside a transaction that is rolled back: drill runs are append-only, and a
    // test row would tell the real schedule the suite ran this week.
    let mut tx = pg.begin().await.expect("tx");
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.expect("role");
    let insert = |venue_class: &'static str, passed: bool| {
        format!(
            "INSERT INTO monitor.drill_run (drill, trigger, venue_class, started_at, finished_at,
               duration_ms, passed, measurements, failure, run_by)
             VALUES ('monitor_lost', 'on_demand', '{venue_class}', now(), now(), 0, {passed},
               '{{}}', {}, 'human:test')",
            if passed { "NULL" } else { "'it failed'" }
        )
    };
    sqlx::query("SAVEPOINT a").execute(&mut *tx).await.expect("sp");
    assert!(sqlx::query(&insert("paper", false)).execute(&mut *tx).await.is_err(),
        "a failed drill was recorded without a defect");
    sqlx::query("ROLLBACK TO SAVEPOINT a").execute(&mut *tx).await.expect("rb");
    assert!(sqlx::query(&insert("live", true)).execute(&mut *tx).await.is_err(),
        "a drill against live capital was recorded");
    sqlx::query("ROLLBACK TO SAVEPOINT a").execute(&mut *tx).await.expect("rb");
    sqlx::query(&insert("paper", true)).execute(&mut *tx).await.expect("app_role records a passed paper drill");
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn no_deployment_leaves_paper_without_limits_in_three_families() {
    let pg = db!();
    let mut tx = pg.begin().await.expect("tx");
    // Simulate the day the environment CHECK is widened: the trigger must still hold.
    sqlx::query("ALTER TABLE strategy_deployment DROP CONSTRAINT IF EXISTS strategy_deployment_environment_check")
        .execute(&mut *tx)
        .await
        .expect("drop check (rolled back)");
    let id = Uuid::new_v4();
    let insert = "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json,
            instrument_id, environment, venue, starting_capital, started_by)
         VALUES ($1, 'monitor-test', 'promote', repeat('a', 64), '{}', 'BTC-USD', 'live',
                 'platform_paper', 1000, 'human:test')";
    sqlx::query("SAVEPOINT a").execute(&mut *tx).await.expect("sp");
    let refused = sqlx::query(insert).bind(id).execute(&mut *tx).await;
    assert!(refused.is_err(), "a live deployment with no declared limits was written");
    sqlx::query("ROLLBACK TO SAVEPOINT a").execute(&mut *tx).await.expect("rb");
    for (metric, family, method) in [
        ("fill_rate", "execution", "declared_absolute"),
        ("signals_per_hour", "behaviour", "declared_absolute"),
        ("cusum_low", "pnl", "cusum"),
    ] {
        sqlx::query(
            "INSERT INTO monitor.limits (strategy_id, metric, version, family, lower_limit, method, declared_from, declared_by)
             VALUES ($1, $2, 1, $3, 0.1, $4, 'paper-run-1', 'human:test')",
        )
        .bind(id.to_string())
        .bind(metric)
        .bind(family)
        .bind(method)
        .execute(&mut *tx)
        .await
        .expect("limit");
    }
    sqlx::query(insert).bind(id).execute(&mut *tx).await.expect("with limits declared it may leave paper");
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn a_sustained_cusum_breach_on_measured_daily_returns_halts_the_deployment_at_the_gate() {
    let pg = db!();
    let id = Uuid::new_v4();
    let strategy = id.to_string();
    let start = Utc::now() - chrono::Duration::days(8);
    sqlx::query(
        "INSERT INTO strategy_deployment
           (deployment_id, user_id, strategy_name, revision_hash, definition_json,
            instrument_id, environment, venue, starting_capital, started_by, started_at)
         VALUES ($1, 'monitor-test', $2, repeat('a', 64), '{}', 'PNLTEST', 'paper',
                 'platform_paper', 1000, 'human:test', $3)",
    )
    .bind(id)
    .bind(format!("pnl-{id}"))
    .bind(start)
    .execute(&pg)
    .await
    .expect("deployment");

    // One buy of 10 at 100 on the first day, then a mark that falls 2% a day:
    // a strategy that has quietly stopped working.
    let key = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO orders (idempotency_key, strategy_id, instrument_id, side, order_type, size, state, created_at)
         VALUES ($1, $2, 'PNLTEST', 'buy', 'market', 10, 'filled', $3)",
    )
    .bind(key)
    .bind(&strategy)
    .bind(start + chrono::Duration::hours(1))
    .execute(&pg)
    .await
    .expect("order");
    sqlx::query(
        "INSERT INTO fills (idempotency_key, broker_order_id, instrument_id, side, filled_size,
                            cumulative_filled, fill_price, commission, filled_at)
         VALUES ($1, 'b-1', 'PNLTEST', 'buy', 10, 10, 100, 0, $2)",
    )
    .bind(key)
    .bind(start + chrono::Duration::hours(1))
    .execute(&pg)
    .await
    .expect("fill");
    for day in 0..8_i64 {
        let close = 100.0 * 0.98_f64.powi(i32::try_from(day).expect("day"));
        sqlx::query(
            "INSERT INTO strategy_decision_journal
               (deployment_id, seq, tick, decided_at, cause, digest, input_digest, inputs, trace)
             VALUES ($1, $2, $2, $3, 'bar_close', repeat('b', 64), repeat('c', 64), $4, '{}')",
        )
        .bind(id)
        .bind(day + 1)
        .bind(start + chrono::Duration::days(day) + chrono::Duration::hours(2))
        .bind(serde_json::json!({ "event": { "event": "bar", "input": 0,
            "bar": { "close_ns": 0, "open": close, "high": close, "low": close, "close": close, "volume": 1.0 } } }))
        .execute(&pg)
        .await
        .expect("journal row");
    }

    let store = MonitorStore::new(pg.clone());
    store
        .declare_return_baseline(&strategy, 0.001, 0.005, 365.0, "paper-run-pnl-test", "human:test")
        .await
        .expect("baseline");
    store
        .declare_limit(&monitor::limits::ControlLimit {
            strategy_id: strategy.clone(),
            metric: "cusum_low".to_owned(),
            family: monitor::family::Family::Pnl,
            severity: monitor::limits::LimitSeverity::Breach,
            version: 1,
            lower: Some(-5.0),
            upper: None,
            method: monitor::limits::LimitMethod::Cusum,
            declared_from: "paper-run-pnl-test".to_owned(),
            declared_at: Utc::now(),
            declared_by: "human:test".to_owned(),
        })
        .await
        .expect("limit");

    let service = MonitorService::new(store.clone(), LeaseTerms::new("t", 60_000));
    let mut cadence = Cadence::default();
    let mut last = None;
    // Dwell: a P&L breach is an estimate, so it must persist three cadences.
    for _ in 0..3 {
        let decisions = service.collect_cadence(&mut cadence, Utc::now()).await.expect("cadence");
        last = decisions.into_iter().find(|d| d.strategy_id == strategy);
    }
    let decision = last.expect("the deployment was judged");
    assert_eq!(decision.transition.to, monitor::state::StrategyState::Halted, "{decision:?}");

    // Through the gate's table: the halt reaches the order gate.
    let halted = store.halted_scopes().await.expect("scopes");
    assert!(halted.contains(&Scope::strategy(strategy.as_str())), "{halted:?}");
    let metric: Option<String> = sqlx::query_scalar(
        "SELECT metric FROM monitor.events WHERE scope = $1 AND kind = 'halted'",
    )
    .bind(format!("strategy:{strategy}"))
    .fetch_one(&pg)
    .await
    .expect("the halt record");
    assert_eq!(metric.as_deref(), Some("cusum_low"));

    let _ = sqlx::query("DELETE FROM monitor.limits WHERE strategy_id = $1").bind(&strategy).execute(&pg).await;
    let _ = sqlx::query("DELETE FROM monitor.dwell WHERE strategy_id = $1").bind(&strategy).execute(&pg).await;
    cleanup_deployment(&pg, &strategy).await;
}

#[tokio::test]
async fn a_measured_fill_rate_below_its_floor_scales_the_deployment_to_a_quarter_in_the_gate() {
    let pg = db!();
    let strategy = deploy(&pg, "FILLTEST").await;
    // Ten orders in the last hour, four filled: a measured fill rate of 0.4.
    for i in 0..10 {
        let key = Uuid::new_v4();
        let state = if i < 4 { "filled" } else { "cancelled" };
        sqlx::query(
            "INSERT INTO orders (idempotency_key, strategy_id, instrument_id, side, order_type, size, state, created_at)
             VALUES ($1, $2, 'FILLTEST', 'buy', 'limit', 1, $3, now() - interval '10 minutes')",
        )
        .bind(key)
        .bind(&strategy)
        .bind(state)
        .execute(&pg)
        .await
        .expect("order");
        if i < 4 {
            sqlx::query(
                "INSERT INTO fills (idempotency_key, broker_order_id, instrument_id, side, filled_size,
                                    cumulative_filled, fill_price, commission, filled_at)
                 VALUES ($1, $2, 'FILLTEST', 'buy', 1, 1, 100, 0, now() - interval '9 minutes')",
            )
            .bind(key)
            .bind(format!("b-{i}"))
            .execute(&pg)
            .await
            .expect("fill");
        }
    }
    let store = MonitorStore::new(pg.clone());
    store
        .declare_limit(&monitor::limits::ControlLimit {
            strategy_id: strategy.clone(),
            metric: "fill_rate".to_owned(),
            family: monitor::family::Family::Execution,
            severity: monitor::limits::LimitSeverity::Breach,
            version: 1,
            lower: Some(0.8),
            upper: None,
            method: monitor::limits::LimitMethod::PaperEwma3Sigma,
            declared_from: "paper-run-fill-test".to_owned(),
            declared_at: Utc::now(),
            declared_by: "human:test".to_owned(),
        })
        .await
        .expect("limit");
    let service = MonitorService::new(store.clone(), LeaseTerms::new("t", 60_000));
    let mut cadence = Cadence::default();
    for _ in 0..3 {
        service.collect_cadence(&mut cadence, Utc::now()).await.expect("cadence");
    }
    let (state, scale): (String, i16) =
        sqlx::query_as("SELECT state, scale_pct FROM monitor.strategy_state WHERE strategy_id = $1")
            .bind(&strategy)
            .fetch_one(&pg)
            .await
            .expect("state row");
    assert_eq!((state.as_str(), scale), ("scaled_down", 25));

    // The process boundary: the watched process's refresh carries the size.
    let gate = Arc::new(KillSwitch::new(false));
    LeaseWatcher::new(store.clone(), Arc::clone(&gate), Duration::from_secs(5))
        .refresh()
        .await;
    assert_eq!(gate.scale_pct(&strategy), 25);

    let (metric, value, limit): (Option<String>, Option<rust_decimal::Decimal>, Option<rust_decimal::Decimal>) =
        sqlx::query_as(
            "SELECT metric, observed_value, limit_value FROM monitor.events
              WHERE scope = $1 AND kind = 'scaled_down'",
        )
        .bind(format!("strategy:{strategy}"))
        .fetch_one(&pg)
        .await
        .expect("the scale-down record");
    assert_eq!(metric.as_deref(), Some("fill_rate"));
    let f = |d: Option<rust_decimal::Decimal>| {
        use rust_decimal::prelude::ToPrimitive;
        d.and_then(|d| d.to_f64()).expect("a number")
    };
    assert!((f(value) - 0.4).abs() < 1e-9, "the value is not the measured 0.4");
    assert!((f(limit) - 0.8).abs() < 1e-9, "the limit is not the declared 0.8");

    let _ = sqlx::query("DELETE FROM monitor.limits WHERE strategy_id = $1").bind(&strategy).execute(&pg).await;
    let _ = sqlx::query("DELETE FROM monitor.dwell WHERE strategy_id = $1").bind(&strategy).execute(&pg).await;
    cleanup_deployment(&pg, &strategy).await;
}

#[tokio::test]
async fn an_events_observed_and_limit_values_are_served_and_absent_ones_stay_null() {
    let pg = db!();
    let store = MonitorStore::new(pg.clone());
    let strategy = format!("values-{}", Uuid::new_v4());
    let scope = format!("strategy:{strategy}");
    let with = monitor::events::MonitorEvent::new(
        Scope::strategy(strategy.as_str()),
        monitor::family::Family::Execution,
        monitor::events::EventKind::ScaledDown,
        "monitor",
    )
    .expect("event")
    .with_metric("fill_rate", 0.4, Some(0.8))
    .with_reason("fill_rate 0.4 outside its declared limit of 0.8");
    let without = monitor::events::MonitorEvent::new(
        Scope::strategy(strategy.as_str()),
        monitor::family::Family::Lease,
        monitor::events::EventKind::Warning,
        "monitor",
    )
    .expect("event")
    .with_reason("no metric on this one");
    store.record_event(&with).await.expect("record");
    store.record_event(&without).await.expect("record");

    let rows = store
        .recent_events(&monitor::store::EventsQuery::recent(10).scoped(scope.clone()))
        .await
        .expect("read");
    assert_eq!(rows.len(), 2);
    let metric_row = rows.iter().find(|r| r.metric.as_deref() == Some("fill_rate")).expect("metric row");
    assert!((metric_row.observed_value.expect("observed") - 0.4).abs() < 1e-9);
    assert!((metric_row.limit_value.expect("limit") - 0.8).abs() < 1e-9);
    let bare = rows.iter().find(|r| r.metric.is_none()).expect("bare row");
    assert_eq!(bare.observed_value, None, "a missing number became a value");
    assert_eq!(bare.limit_value, None);

    let _ = sqlx::query("DELETE FROM monitor.events WHERE scope = $1").bind(&scope).execute(&pg).await;
}
