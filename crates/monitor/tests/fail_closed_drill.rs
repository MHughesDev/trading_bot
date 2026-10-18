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
//!
//! The lease is the exception that cannot be scoped: `monitor.lease` holds one
//! row, keyed by the constant [`LEASE_ID`], and every drill that needs an open
//! gate takes *that* row. So `--test-threads=1` is not a preference, and two
//! copies of this suite against one database will fight over it — a second
//! runner renewing at 60 s makes the lapse this file asserts on never happen.
//! Run it once at a time.
//!
//! Whatever a drill writes is registered with [`Cleanup`] and deleted on the way
//! out, on the unwind as well as on the happy path. Nothing here may leave a row
//! in a live database; see that type for what each leaked row would go on to do.
//!
//! [`LEASE_ID`]: monitor::LEASE_ID

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use domain::{
    money::{Price, Size},
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

/// The pool **and the url**. The url, because [`Cleanup`] has to be able to
/// reconnect: one of these drills closes the pool it was handed on purpose, and
/// a guard that cleaned up through that pool would be silently unable to clean
/// up after exactly the drill that needs it most.
async fn pool() -> Option<(PgPool, String)> {
    let url = std::env::var("MONITOR_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("API_TEST_DATABASE_URL"))
        .ok()?;
    let pg = PgPool::connect(&url).await.expect("connect");
    Some((pg, url))
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

// ── The guard ───────────────────────────────────────────────────────────────

/// Everything a drill wrote, deleted on the way out — whether the drill returns
/// or unwinds.
///
/// These drills used to tidy up on their last few lines, which means they tidied
/// up only when they passed. A failed assertion unwinds straight past those
/// lines, and what it leaves behind in a live database is not inert:
///
/// * a leaked `venue:*` halt is read by `all_venues_reconciled`, so one
///   abandoned row refuses **every global reset** until somebody finds it by
///   hand;
/// * a leaked `monitor.lease` row names a holder that is not running. Expired,
///   it refuses every order with `monitor_unavailable` — and it refuses them
///   from a table that is *not empty*, so the obvious diagnosis ("nothing has
///   taken the lease") is looking at the wrong thing;
/// * a leaked `tbot-monitor` heartbeat is a permanently stale component, which
///   is the input [`assess`][a] turns into a global halt;
/// * a leaked `global` trip leaves `global_risk_config.trading_enabled` false,
///   so the next boot of the platform comes up halted for a reason that is
///   nowhere in the timeline.
///
/// None of that needed a panic, either: `MonitorService::tick` writes a lease
/// row *and* a heartbeat row, and no drill here ever deleted either. Six
/// heartbeat rows per **green** run is what the live database had accumulated
/// by the time this guard was written.
///
/// `Drop` cannot await, so the deletes run on a borrowed thread with a runtime
/// of their own — which works because this thread is not the one blocking on
/// them. The thread is joined, so the rows are gone before the next drill
/// starts, and a cleanup that fails is visible in the output of the drill that
/// leaked rather than in whichever unrelated drill later trips over it.
///
/// [a]: monitor::platform::assess
struct Cleanup {
    url: String,
    /// `monitor.events` and `monitor.kill_switch` rows for these exact scopes.
    scopes: Vec<String>,
    /// The same, for scopes whose full name the drill cannot know in advance —
    /// `Rehearsal::run` mints its own `drill:<label>-<uuid>` inside the call
    /// that can panic, so the label is all there is to register beforehand.
    scope_prefixes: Vec<String>,
    /// `monitor.heartbeat` rows for these `tbot-monitor` instances.
    holders: Vec<String>,
    /// `monitor.heartbeat` rows for these components, whatever the instance.
    components: Vec<String>,
    strategies: Vec<String>,
    drills: Vec<Uuid>,
    windows: Vec<Uuid>,
    /// Events named by id, for scopes shared with the running platform.
    events: Vec<Uuid>,
    /// The `global`/`lease` gap rows a drill leaves behind when it lapses the
    /// lease deliberately. Bounded by time because `LeaseWatcher` writes them
    /// itself and hands back no ids — and bounded rather than "every lease
    /// event", because a real gap recorded before this drill began is not this
    /// drill's to erase.
    lease_events_since: Option<DateTime<Utc>>,
    /// Whether a lease row existed before the drill. Only a row the drill
    /// created is removed; a real monitor's lease is left where it is.
    lease_was_absent: bool,
    /// `global_risk_config.trading_enabled` as it stood before the drill, for
    /// the one drill that trips `global`.
    trading_enabled_before: Option<bool>,
}

impl Cleanup {
    /// Record what the database looked like before the drill touched it.
    async fn new(url: &str, pg: &PgPool) -> Self {
        let leases: i64 =
            sqlx::query_scalar("SELECT count(*) FROM monitor.lease WHERE lease_id = $1")
                .bind(monitor::LEASE_ID)
                .fetch_one(pg)
                .await
                .expect("count the lease");
        // Spelled out rather than `..Self::default()`: a type with a `Drop` impl
        // cannot be built by functional update, because that would move the
        // default's fields out of a value that still owes a drop.
        Self {
            url: url.to_owned(),
            scopes: Vec::new(),
            scope_prefixes: Vec::new(),
            holders: Vec::new(),
            components: Vec::new(),
            strategies: Vec::new(),
            drills: Vec::new(),
            windows: Vec::new(),
            events: Vec::new(),
            lease_events_since: None,
            lease_was_absent: leases == 0,
            trading_enabled_before: None,
        }
    }

    /// Remember `global_risk_config.trading_enabled`, so a drill that trips
    /// `global` and then fails cannot leave the platform booting halted.
    async fn restoring_global_trading(mut self, pg: &PgPool) -> Self {
        self.trading_enabled_before =
            sqlx::query_scalar("SELECT trading_enabled FROM global_risk_config LIMIT 1")
                .fetch_optional(pg)
                .await
                .expect("global_risk_config");
        self
    }

    fn scope(&mut self, scope: &str) -> &mut Self {
        self.scopes.push(scope.to_owned());
        self
    }

    fn scope_prefix(&mut self, prefix: &str) -> &mut Self {
        self.scope_prefixes.push(prefix.to_owned());
        self
    }

    fn holder(&mut self, holder: &str) -> &mut Self {
        self.holders.push(holder.to_owned());
        self
    }

    fn component(&mut self, component: &str) -> &mut Self {
        self.components.push(component.to_owned());
        self
    }

    fn strategy(&mut self, strategy_id: &str) -> &mut Self {
        self.strategies.push(strategy_id.to_owned());
        self
    }

    fn drill(&mut self, drill_id: Uuid) -> &mut Self {
        self.drills.push(drill_id);
        self
    }

    fn window(&mut self, window_id: Uuid) -> &mut Self {
        self.windows.push(window_id);
        self
    }

    fn event(&mut self, event_id: Uuid) -> &mut Self {
        self.events.push(event_id);
        self
    }

    /// Called before the refreshes, by the drills that lapse the lease on
    /// purpose.
    fn lease_gap_events(&mut self) -> &mut Self {
        self.lease_events_since = Some(Utc::now());
        self
    }
}

/// A cleanup that failed is reported and does not abort the rest of the cleanup:
/// one undeletable row must not strand the other nine.
fn report<T>(what: &str, result: Result<T, sqlx::Error>) {
    if let Err(e) = result {
        eprintln!("CLEANUP FAILED for {what}: {e}");
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let url = std::mem::take(&mut self.url);
        if url.is_empty() {
            return;
        }
        let scopes = std::mem::take(&mut self.scopes);
        let prefixes = std::mem::take(&mut self.scope_prefixes);
        let holders = std::mem::take(&mut self.holders);
        let components = std::mem::take(&mut self.components);
        let strategies = std::mem::take(&mut self.strategies);
        let drills = std::mem::take(&mut self.drills);
        let windows = std::mem::take(&mut self.windows);
        let events = std::mem::take(&mut self.events);
        let lease_events_since = self.lease_events_since;
        let drop_lease = self.lease_was_absent;
        let trading = self.trading_enabled_before;

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("cleanup runtime");
            rt.block_on(async move {
                let Ok(pg) = PgPool::connect(&url).await else {
                    eprintln!(
                        "CLEANUP FAILED: cannot reconnect to the database — rows left behind"
                    );
                    return;
                };

                // Events before the windows they may point at: the foreign key
                // runs that way and so must the deletes.
                for scope in &scopes {
                    report(
                        scope,
                        sqlx::query("DELETE FROM monitor.events WHERE scope = $1")
                            .bind(scope)
                            .execute(&pg)
                            .await,
                    );
                }
                for id in &events {
                    report(
                        "an event",
                        sqlx::query("DELETE FROM monitor.events WHERE event_id = $1")
                            .bind(id)
                            .execute(&pg)
                            .await,
                    );
                }
                if let Some(since) = lease_events_since {
                    report(
                        "the lease gap events",
                        sqlx::query(
                            "DELETE FROM monitor.events
                              WHERE scope = 'global' AND family = 'lease'
                                AND occurred_at >= $1",
                        )
                        .bind(since)
                        .execute(&pg)
                        .await,
                    );
                }
                for prefix in &prefixes {
                    let like = format!("{prefix}%");
                    for sql in [
                        "DELETE FROM monitor.events WHERE scope LIKE $1",
                        "DELETE FROM monitor.kill_switch WHERE scope LIKE $1",
                        "DELETE FROM monitor.drill WHERE scope LIKE $1",
                    ] {
                        report(prefix, sqlx::query(sql).bind(&like).execute(&pg).await);
                    }
                }
                for scope in &scopes {
                    report(
                        scope,
                        sqlx::query("DELETE FROM monitor.kill_switch WHERE scope = $1")
                            .bind(scope)
                            .execute(&pg)
                            .await,
                    );
                }
                for id in &drills {
                    report(
                        "a drill record",
                        sqlx::query("DELETE FROM monitor.drill WHERE drill_id = $1")
                            .bind(id)
                            .execute(&pg)
                            .await,
                    );
                }
                for holder in &holders {
                    report(
                        holder,
                        sqlx::query(
                            "DELETE FROM monitor.heartbeat
                              WHERE component = $1 AND instance = $2",
                        )
                        .bind(monitor::LEASE_ID)
                        .bind(holder)
                        .execute(&pg)
                        .await,
                    );
                }
                for component in &components {
                    report(
                        component,
                        sqlx::query("DELETE FROM monitor.heartbeat WHERE component = $1")
                            .bind(component)
                            .execute(&pg)
                            .await,
                    );
                }
                for strategy_id in &strategies {
                    report(
                        strategy_id,
                        sqlx::query("DELETE FROM monitor.limits WHERE strategy_id = $1")
                            .bind(strategy_id)
                            .execute(&pg)
                            .await,
                    );
                }
                for id in &windows {
                    report(
                        "a maintenance window",
                        sqlx::query("DELETE FROM monitor.maintenance_window WHERE window_id = $1")
                            .bind(id)
                            .execute(&pg)
                            .await,
                    );
                }
                if drop_lease {
                    // `holder LIKE 'drill-%'` because a real monitor may have
                    // started during the drill and taken the row over; its lease
                    // is not ours to delete.
                    report(
                        "the lease row",
                        sqlx::query(
                            "DELETE FROM monitor.lease
                              WHERE lease_id = $1 AND holder LIKE 'drill-%'",
                        )
                        .bind(monitor::LEASE_ID)
                        .execute(&pg)
                        .await,
                    );
                }
                if let Some(before) = trading {
                    report(
                        "global_risk_config.trading_enabled",
                        sqlx::query("UPDATE global_risk_config SET trading_enabled = $1")
                            .bind(before)
                            .execute(&pg)
                            .await,
                    );
                }
            });
        });
        if handle.join().is_err() {
            eprintln!("CLEANUP FAILED: the cleanup thread panicked — rows may be left behind");
        }
    }
}

/// A context an order can actually pass on.
///
/// The mark price is not decoration. `GateContext::for_manual_order` turns a
/// `None` here into `Measured::No`, and `RiskGate::check` refuses such an order
/// with `MarketDataUnavailable` **before** it ever reaches the kill switch — so
/// with `None` every `is_ok()` assertion in this file was false and every
/// `Err(..)` assertion was true for the wrong reason.
///
/// That went unnoticed because these drills return early without
/// `MONITOR_TEST_DATABASE_URL`: they were green and vacuous, and COMP-007 §8.1,
/// §8.2 and §8.3 named them as their verification. Four of them failed the first
/// time they met a database, on 2026-09-21. The number is arbitrary; that it
/// exists is not.
fn ctx() -> GateContext {
    GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_str("50000").unwrap()),
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

/// Take the lease the way the monitor does, and register **both** rows
/// `MonitorService::tick` writes — the lease row and the `tbot-monitor`
/// heartbeat — for deletion.
///
/// Every drill used to open with three lines that took the lease and registered
/// nothing, which is how the live `monitor.heartbeat` table came to hold sixteen
/// `drill-*` instances at three different cadences.
async fn take_the_lease(
    store: &MonitorStore,
    cleanup: &mut Cleanup,
    cadence_ms: i64,
) -> MonitorService {
    let holder = format!("drill-{}", Uuid::new_v4());
    cleanup.holder(&holder);
    let service = MonitorService::new(store.clone(), LeaseTerms::new(holder, cadence_ms));
    service.tick().await.expect("the monitor renews its lease");
    service
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
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let mut cleanup = Cleanup::new(&url, &pg).await;
    // This drill lapses the lease on purpose, so the `global`/`lease` gap rows
    // the watcher writes below are its residue and not an incident.
    cleanup.lease_gap_events();

    // The cadence is a real-time budget, not a knob for making the drill quick.
    //
    // It was 300 ms, and that is not a margin — it is the whole time the runtime
    // has to notice a lease that was renewed one round trip ago. `tick` writes
    // the lease *and* the heartbeat before `refresh` reads anything, so at 300 ms
    // three database round trips had to complete inside the lease they were
    // establishing. Against Postgres in Docker on Windows, under a platform that
    // is committing on its own, they do not reliably do so: this drill failed
    // about one run in five, at whichever of the three assertions lost the race,
    // and each failure wrote a `global` lease-gap row on its way out.
    //
    // Two seconds is the same arithmetic with room for the round trips to be
    // slow. The assertion below is cadence-independent — it is the same at 60 s,
    // which is what the monitor actually runs at.
    let cadence_ms = 2_000;
    let service = take_the_lease(&store, &mut cleanup, cadence_ms).await;

    let (ks, gate, watcher) = runtime(&store);
    assert!(
        !ks.monitor_available(),
        "before the first refresh the gate is closed — there is no window in which \
         a starting process trades unwatched"
    );

    watcher.refresh().await;
    assert!(ks.monitor_available(), "a live lease opens the gate");
    let first = gate.check(intent(), &ctx());
    assert!(first.is_ok(), "{first:?}");

    // The monitor dies here. Nothing tells the runtime; the renewal simply stops.
    // The overshoot is a quarter of a cadence rather than a flat 50 ms, so that
    // the drill is past the expiry by a margin that scales with the budget above.
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(cadence_ms).unwrap() + u64::try_from(cadence_ms / 4).unwrap(),
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
    let first = gate.check(intent(), &ctx());
    assert!(first.is_ok(), "{first:?}");
}

#[tokio::test]
async fn a_database_the_runtime_cannot_read_closes_the_gate() {
    // §4's "fail closed" covers the case nobody writes a test for: not a monitor
    // that died, but a runtime that cannot find out either way. The pool is closed
    // under the watcher, which is as close to "the database went away" as a test
    // can get without unplugging something.
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let mut cleanup = Cleanup::new(&url, &pg).await;
    cleanup.lease_gap_events();
    take_the_lease(&store, &mut cleanup, 60_000).await;

    let (_ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;
    let first = gate.check(intent(), &ctx());
    assert!(first.is_ok(), "{first:?}");

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
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    // This is the one drill that trips `global`, so it is the one that has to
    // put `global_risk_config.trading_enabled` back — a failed assertion between
    // the trip and the reset below would otherwise leave the *next boot of the
    // platform* halted, with nothing in the timeline saying why.
    let mut cleanup = Cleanup::new(&url, &pg)
        .await
        .restoring_global_trading(&pg)
        .await;
    cleanup.scope("global");
    clear_scope(&pg, "global").await;

    take_the_lease(&store, &mut cleanup, 60_000).await;

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
}

// ── §8.3 — a venue halt is one venue ────────────────────────────────────────

#[tokio::test]
async fn an_injected_mismatch_halts_one_venue_and_blocks_resume_until_clean() {
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let bad = format!("kraken-{}", Uuid::new_v4());
    let good = format!("alpaca-{}", Uuid::new_v4());
    let bad_scope = Scope::venue(&bad);

    let mut cleanup = Cleanup::new(&url, &pg).await;
    // Registered before the trip, not after the assertions: a `venue:*` halt
    // that outlives this drill makes `all_venues_reconciled` false and refuses
    // every global reset from then on.
    cleanup.scope(&bad_scope.to_string());

    take_the_lease(&store, &mut cleanup, 60_000).await;

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
}

// ── §8.5 — the trip record exists before the first refused order ────────────

#[tokio::test]
async fn the_trip_record_is_present_before_the_first_refused_order() {
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let scope = Scope::strategy(&sid);

    let mut cleanup = Cleanup::new(&url, &pg).await;
    cleanup.scope(&scope.to_string());

    take_the_lease(&store, &mut cleanup, 60_000).await;

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
}

// ── §8.6 — no agent, at any layer ───────────────────────────────────────────

#[tokio::test]
async fn the_database_refuses_an_agent_tripped_or_agent_reset_row() {
    // The layer under the API. Even a direct write — a bug in a handler, a script,
    // a future migration — cannot record an agent as having tripped or reset the
    // switch.
    let (pg, url) = db!();
    let scope = format!("strategy:agent-probe-{}", Uuid::new_v4());
    let mut cleanup = Cleanup::new(&url, &pg).await;
    cleanup.scope(&scope);

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
}

#[tokio::test]
async fn no_agent_tripped_row_exists_anywhere() {
    // §8.6 as a standing assertion over the whole table rather than over one
    // inserted row: whatever else has happened to this database, nothing has ever
    // recorded an agent as tripping or resetting.
    let (pg, _url) = db!();
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
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let scope = Scope::strategy(&sid);
    let mut cleanup = Cleanup::new(&url, &pg).await;
    cleanup.scope(&scope.to_string());

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
    //
    // A *new* event, because that is what a second incident is. Re-tripping with
    // the same `MonitorEvent` reuses its `event_id` and the insert fails on
    // `events_pkey`, which is the primary key doing its job: one row per thing
    // that happened. The original of this test re-used it and never found out,
    // because it returned early without a database.
    let again = MonitorEvent::new(scope.clone(), Family::Pnl, EventKind::Halted, "monitor")
        .expect("monitor")
        .with_reason("flapped");
    store.trip(&again, "flapped").await.expect("trip");
    let err = store.reset(&request).await;
    assert!(
        matches!(err, Err(monitor::store::StoreError::Reset(_))),
        "a second reset within five minutes must be refused; got {err:?}"
    );
}

// ── §8.8 — no limits, no live ───────────────────────────────────────────────

#[tokio::test]
async fn a_strategy_with_no_declared_limits_cannot_be_promoted_to_live() {
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let mut cleanup = Cleanup::new(&url, &pg).await;
    cleanup.strategy(&sid);

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
}

// ── §4.3 — a window suppresses a platform trip and nothing else ─────────────

#[tokio::test]
async fn an_open_window_is_visible_attributed_and_bounded() {
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let mut cleanup = Cleanup::new(&url, &pg).await;
    let window = monitor::MaintenanceWindow::open(
        "human:test",
        "rolling deploy",
        15,
        Utc::now(),
    )
    .expect("window");
    cleanup.window(window.window_id);
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
}

// ── §4.2 — a slow drill is recorded as a failure, and cannot be recorded as a pass

#[tokio::test]
async fn a_slow_drill_cannot_be_recorded_as_having_passed() {
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let mut cleanup = Cleanup::new(&url, &pg).await;

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
    cleanup.drill(slow.drill_id);
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
}

// ── S-7.6 — the receiving half of a reconciliation mismatch ──────────────────

#[tokio::test]
async fn a_mismatch_halt_survives_the_refresh_that_erases_an_in_memory_one() {
    // The defect, demonstrated side by side rather than described.
    //
    // `reconciliation::divergence::check_divergence` halts a venue by calling
    // `KillSwitch::trip_scope` and writing nothing. `LeaseWatcher::refresh` calls
    // `KillSwitch::replace_halted`, which **replaces** the scoped view with what
    // `monitor.kill_switch` says — so that halt is not merely unrecorded, it is
    // erased on the next refresh and the venue that disagrees with our book about
    // what is owned resumes trading about five seconds later.
    //
    // Two venues, halted one way each, one refresh between them.
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());

    let recorded = format!("kraken-{}", Uuid::new_v4());
    let in_memory_only = format!("alpaca-{}", Uuid::new_v4());
    let untouched = format!("coinbase-{}", Uuid::new_v4());

    let mut cleanup = Cleanup::new(&url, &pg).await;
    cleanup.scope(&Scope::venue(&recorded).to_string());

    take_the_lease(&store, &mut cleanup, 60_000).await;

    let (ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;

    // The way `crates/reconciliation` does it today.
    ks.trip_scope(&Scope::venue(&in_memory_only));
    // The way the receiving half does it: durable row first, mirror second.
    let halted = store
        .carry_out_mismatch(&recorded, &ks)
        .await
        .expect("the mismatch is recorded and the venue halted");
    assert_eq!(halted, Scope::venue(&recorded));

    // Before the refresh the two are indistinguishable from the gate's side,
    // which is why this defect passes every test that stops here.
    assert!(matches!(
        gate.check(intent(), &ctx().on_venue(&recorded)),
        Err(RiskRejection::KillSwitchActive)
    ));
    assert!(matches!(
        gate.check(intent(), &ctx().on_venue(&in_memory_only)),
        Err(RiskRejection::KillSwitchActive)
    ));

    // One refresh — the thing that happens every few seconds in production.
    watcher.refresh().await;

    assert!(
        matches!(
            gate.check(intent(), &ctx().on_venue(&recorded)),
            Err(RiskRejection::KillSwitchActive)
        ),
        "a recorded mismatch must still be halted after the refresh that reads the table"
    );
    assert!(
        gate.check(intent(), &ctx().on_venue(&in_memory_only)).is_ok(),
        "this is the defect: an in-memory trip with no durable row is erased by \
         replace_halted, and the venue silently resumes"
    );
    assert!(
        gate.check(intent(), &ctx().on_venue(&untouched)).is_ok(),
        "a mismatch at one venue says nothing about the others"
    );

    // §8.5: the record exists, it names the venue, and there is one of it.
    let reason: String = sqlx::query_scalar(
        "SELECT reason FROM monitor.kill_switch WHERE scope = $1 AND state = 'halted'",
    )
    .bind(halted.to_string())
    .fetch_one(&pg)
    .await
    .expect("the durable row is there");
    assert!(reason.contains(&recorded), "{reason}");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM monitor.events WHERE scope = $1 AND kind = 'halted'",
    )
    .bind(halted.to_string())
    .fetch_one(&pg)
    .await
    .expect("count");
    assert_eq!(events, 1, "one incident, one trip record");
}

// ── S-7.13 — a drill that measures, and a window that suppresses ─────────────

#[tokio::test]
async fn a_drill_measures_its_own_propagation_and_leaves_nothing_behind() {
    // COMP-007 §4.2. Until now the three latencies came from the request body of
    // `POST /api/monitor/drill`, so "a drill whose propagation exceeds one cadence
    // fails the deploy" was arithmetic over numbers the caller typed. These are
    // measured: a real trip, a real read of `monitor.kill_switch`, a real
    // `replace_halted` and a real `is_active_scope`.
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let mut cleanup = Cleanup::new(&url, &pg).await;
    // By prefix, because `run` mints `drill:track-d4-<uuid>` inside the call
    // that can fail: the scope this has to clean up does not exist yet, and if
    // the rehearsal panics it never comes back to be named.
    cleanup.scope_prefix("drill:track-d4-");

    let rehearsal = monitor::Rehearsal::new(store.clone(), 60_000, "human:test")
        .expect("a human may rehearse");
    let outcome = rehearsal
        .run(monitor::DrillTrigger::Manual, "track-d4")
        .await
        .expect("the rehearsal completes");

    assert!(outcome.passed(), "propagation {}ms", outcome.propagation_ms);
    assert!(!outcome.refused_real);
    assert!(
        matches!(outcome.scope, Scope::Drill(_)),
        "a rehearsal runs against a drill scope and nothing else"
    );
    assert!(
        outcome
            .notes
            .as_deref()
            .is_some_and(|n| n.contains("monitor.events")),
        "the notification number must say which channel it measured"
    );

    // It is a measurement, so it is in the table with its verdict.
    let stored: (i64, bool) =
        sqlx::query_as("SELECT propagation_ms, passed FROM monitor.drill WHERE drill_id = $1")
            .bind(outcome.drill_id)
            .fetch_one(&pg)
            .await
            .expect("the drill was recorded");
    assert_eq!(stored.0, outcome.propagation_ms);
    assert!(stored.1);

    // And nothing is left halted: a drill row still sitting in
    // `monitor.kill_switch` is indistinguishable from a real trip on every page
    // that lists them, and the first thing anyone would do is reset it.
    let still_halted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM monitor.kill_switch WHERE scope = $1 AND state = 'halted'",
    )
    .bind(outcome.scope.to_string())
    .fetch_one(&pg)
    .await
    .expect("count");
    assert_eq!(still_halted, 0, "the rehearsed scope is armed again");
}

#[tokio::test]
async fn a_platform_fault_inside_a_window_is_recorded_and_halts_nothing() {
    // COMP-007 §4.3, against the database. A rolling deploy looks exactly like a
    // heartbeat loss; without the window the deploy halts trading and the operator
    // learns to pre-emptively reset the switch.
    let (pg, url) = db!();
    let store = MonitorStore::new(pg.clone());
    let mut cleanup = Cleanup::new(&url, &pg).await;

    let component = format!("api-{}", Uuid::new_v4());
    cleanup.component(&component);
    sqlx::query(
        "INSERT INTO monitor.heartbeat (component, instance, beat_at, cadence_ms)
         VALUES ($1, 'only', now() - interval '1 hour', 1000)",
    )
    .bind(&component)
    .execute(&pg)
    .await
    .expect("seed a lost component");

    let window = monitor::MaintenanceWindow::open("human:test", "rolling deploy", 15, Utc::now())
        .expect("a bounded, reasoned window");
    cleanup.window(window.window_id);
    store.open_window(&window).await.expect("open");

    let beats = store
        .component_beats(monitor::platform::MISSED_BEATS_BEFORE_HALT)
        .await
        .expect("read the heartbeats");
    let mine: Vec<monitor::ComponentBeat> = beats
        .into_iter()
        .filter(|b| b.component == component)
        .collect();
    assert_eq!(mine.len(), 1, "the seeded component is there to be judged");
    assert!(mine[0].stale, "an hour-old beat at a 1s cadence is stale");

    let open = store.open_windows().await.expect("read the windows");
    let verdict = monitor::platform::assess(&mine, &open, Utc::now());
    match &verdict {
        monitor::PlatformVerdict::Suppressed { lost, window_id } => {
            assert_eq!(lost, std::slice::from_ref(&component));
            assert_eq!(*window_id, window.window_id);
        }
        other => panic!("a lost component inside an open window must be suppressed: {other:?}"),
    }

    // The row §4.3 asks for: a window nobody can see is a hole nobody can audit.
    let event = MonitorEvent::new(Scope::Global, Family::Platform, EventKind::Warning, "monitor")
        .expect("monitor")
        .with_reason(verdict.reason())
        .suppressed_by(window.window_id);
    cleanup.event(event.event_id);
    store.record_event(&event).await.expect("record");
    let recorded: Option<Uuid> =
        sqlx::query_scalar("SELECT suppressed_by_window FROM monitor.events WHERE event_id = $1")
            .bind(event.event_id)
            .fetch_one(&pg)
            .await
            .expect("the suppressed event is there");
    assert_eq!(recorded, Some(window.window_id));

    // The adversarial half: the same fault with the window closed halts. Without
    // this the test would pass just as well if `assess` suppressed everything.
    store.close_window(window.window_id).await.expect("close");
    let open = store.open_windows().await.expect("read the windows");
    assert!(
        !open.iter().any(|w| w.window_id == window.window_id),
        "the closed window is gone"
    );
    assert_eq!(
        monitor::platform::assess(&mine, &open, Utc::now()),
        monitor::PlatformVerdict::Halt {
            lost: vec![component.clone()]
        },
        "with no window open, a lost component is a global halt (§4)"
    );
}
