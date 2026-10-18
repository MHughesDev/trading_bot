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

async fn pool() -> Option<(PgPool, String)> {
    let url = std::env::var("MONITOR_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("API_TEST_DATABASE_URL"))
        .ok()?;
    let pg = PgPool::connect(&url).await.expect("connect");
    Some((pg, url))
}

/// A pool and the [`Drill`] guard that owns everything the test is about to write.
///
/// The two come together on purpose. Every test in this file writes to a database
/// that outlives it, and the guard is the only thing that takes those rows back
/// out; a test that could get a pool without one would be a test that leaks, and
/// six of them did.
macro_rules! db {
    () => {
        match pool().await {
            Some((p, url)) => {
                let guard = Drill::begin(&p, url).await;
                (p, guard)
            }
            None => {
                eprintln!("MONITOR_TEST_DATABASE_URL unset — skipping");
                return;
            }
        }
    };
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

// ── The guard ───────────────────────────────────────────────────────────────

/// The `monitor.lease` row as it was found, so it can be put back exactly.
#[derive(Clone, Debug)]
struct LeaseRow {
    holder: String,
    cadence_ms: i64,
    renewed_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

/// Everything one drill wrote, deleted whether or not the drill panicked.
///
/// # Why this exists rather than six careful call sites
///
/// The cleanup in this file used to be a `clear_scope` at the end of each test.
/// That covered `monitor.events` and `monitor.kill_switch`, it ran only on the
/// success path, and it covered nothing at all of the two tables that actually
/// hurt:
///
/// * **`monitor.heartbeat`.** `MonitorService::tick` beats as well as renewing,
///   under `component = 'tbot-monitor'` and `instance = <holder>`. The key is
///   `(component, instance)` and nothing evicts, so every drill left a permanent
///   row behind — measured at six per run, 24 of them on this box before this
///   guard existed, every one of them written by a test.
/// * **`monitor.lease`.** There is one row, id [`monitor::LEASE_ID`], and the
///   drill has to be the holder of it (see [`Drill::terms`]). A drill that just
///   walked away left the platform's own lease expired under a `drill-<uuid>`
///   holder, and the order gate then refused every order with
///   `monitor_unavailable` until somebody went looking.
///
/// So the drill is given a single object that mints its lease holders and is told
/// about every row it creates, and the deletes live in `Drop`. A holder that is
/// not minted here is a holder nothing reaps, which is why [`Drill::terms`] is the
/// only `LeaseTerms` constructor this file uses.
///
/// `Drop` cannot await, so the deletes run on a borrowed thread with a runtime of
/// their own — which works because this thread is not the one blocking on them.
/// The same shape, and for the same reason, as the `Cleanup` guard in
/// `crates/reconciliation/tests/a_venue_mismatch_survives_the_mirror_refresh.rs`.
struct Drill {
    url: String,
    /// The lease as it was found. `None` means there was no row — which is a
    /// state to restore, not an absence to ignore.
    lease_before: Option<LeaseRow>,
    /// When this drill took the lease, and therefore evicted whatever held it.
    /// `None` means it never asked for terms, and the row is left alone.
    ///
    /// The instant is load-bearing as well as the fact: making the lease lapse
    /// makes `LeaseWatcher::note_lease` write `global`/`lease` events, and those
    /// rows sit in the operator's timeline looking exactly like a real monitor
    /// outage. They are deleted by time window rather than by id, because the
    /// watcher writes them and the test never learns their ids — and bounding the
    /// delete to the window is safe precisely because [`Drill::terms`] refuses to
    /// run while a live monitor holds the lease, so there is no genuine lease
    /// event in it to lose.
    took_lease_at: Option<DateTime<Utc>>,
    /// Lease holders minted here → `monitor.heartbeat` rows to reap.
    holders: Vec<String>,
    /// Whole heartbeat components seeded by the test.
    components: Vec<String>,
    /// Scopes whose `monitor.events` and `monitor.kill_switch` rows are the
    /// test's own.
    scopes: Vec<String>,
    /// Individual events, for the scopes that are *not* the test's own — nothing
    /// may delete every `global` event just because a test wrote one.
    events: Vec<Uuid>,
    drills: Vec<Uuid>,
    windows: Vec<Uuid>,
    /// Strategy ids whose `monitor.limits` rows the test declared.
    limits: Vec<String>,
    /// Scope prefixes whose rows are the test's own, for scopes it cannot name in
    /// advance: `Rehearsal::run` mints `drill:<label>-<uuid>` inside the call, so
    /// a failure inside `run` would otherwise leave rows nothing registered.
    scope_prefixes: Vec<String>,
    /// `global_risk_config.trading_enabled` as it stood before the drill, for a
    /// drill that trips the global switch. A failure between the trip and the
    /// typed reset leaves the column false, and the next boot comes up halted.
    trading_enabled_before: Option<bool>,
}

impl Drill {
    /// Read what is there before the test writes anything.
    async fn begin(pg: &PgPool, url: String) -> Self {
        let found: Option<(String, i64, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT holder, cadence_ms, renewed_at, expires_at
               FROM monitor.lease WHERE lease_id = $1",
        )
        .bind(monitor::LEASE_ID)
        .fetch_optional(pg)
        .await
        .expect("the lease row is readable — a drill that cannot read it cannot restore it");
        Self {
            url,
            lease_before: found.map(|(holder, cadence_ms, renewed_at, expires_at)| LeaseRow {
                holder,
                cadence_ms,
                renewed_at,
                expires_at,
            }),
            took_lease_at: None,
            holders: Vec::new(),
            components: Vec::new(),
            scopes: Vec::new(),
            events: Vec::new(),
            drills: Vec::new(),
            windows: Vec::new(),
            limits: Vec::new(),
            scope_prefixes: Vec::new(),
            trading_enabled_before: None,
        }
    }

    /// Lease terms under a holder this guard will reap.
    ///
    /// # Why the drill takes the real lease
    ///
    /// [`MonitorStore::read_lease`] hard-codes [`monitor::LEASE_ID`], so the row
    /// the order gate consults is the only row that exists. A drill holding some
    /// private lease id would renew a row nothing reads, `LeaseWatcher::refresh`
    /// would report `Missing`, and every `is_ok()` assertion in this file would
    /// fail — or, worse, a `read_lease` taught to accept an id from its caller
    /// would be a second read path into the gate's one question, which is exactly
    /// the shape this crate exists to not have. So the drill evicts the holder,
    /// and puts it back.
    ///
    /// # And refuses to race a live one
    ///
    /// Eviction is survivable; racing is not. A monitor renewing on its own
    /// cadence while `killing_the_monitor_stops_order_flow_within_one_cadence`
    /// waits for the lease to lapse would keep the lease alive, and the drill
    /// would report on a lease it did not control. An unreliable measurement is
    /// not a green test, so this refuses out loud rather than measuring anyway.
    fn terms(&mut self, cadence_ms: i64) -> LeaseTerms {
        if self.took_lease_at.is_none() {
            if let Some(before) = &self.lease_before {
                assert!(
                    before.expires_at <= Utc::now() || before.holder.contains("drill-"),
                    "a live monitor holds the lease (holder {:?}, valid until {}). These \
                     drills take that row over and time its expiry; running against a \
                     monitor that is renewing it would measure a lease this test does not \
                     control. Stop tbot-monitor and run them again.",
                    before.holder,
                    before.expires_at
                );
            }
            self.took_lease_at = Some(Utc::now());
        }
        let holder = format!("drill-{}", Uuid::new_v4());
        self.holders.push(holder.clone());
        LeaseTerms::new(holder, cadence_ms)
    }

    /// This scope's events and kill-switch rows belong to the test.
    fn owns_scope(&mut self, scope: impl Into<String>) {
        self.scopes.push(scope.into());
    }

    fn owns_event(&mut self, event_id: Uuid) {
        self.events.push(event_id);
    }

    fn owns_drill_record(&mut self, drill_id: Uuid) {
        self.drills.push(drill_id);
    }

    fn owns_window(&mut self, window_id: Uuid) {
        self.windows.push(window_id);
    }

    fn owns_component(&mut self, component: impl Into<String>) {
        self.components.push(component.into());
    }

    fn owns_limits_for(&mut self, strategy_id: impl Into<String>) {
        self.limits.push(strategy_id.into());
    }

    /// Every scope starting with `prefix` belongs to the test. Register it
    /// *before* the call that mints the scope, so a failure inside that call is
    /// still cleaned up.
    fn owns_scope_prefix(&mut self, prefix: impl Into<String>) {
        self.scope_prefixes.push(prefix.into());
    }

    /// Remember `global_risk_config.trading_enabled`, so a drill that trips the
    /// global switch puts the boot column back however it ends.
    async fn remembers_trading_enabled(&mut self, pg: &PgPool) {
        self.trading_enabled_before =
            sqlx::query_scalar("SELECT trading_enabled FROM global_risk_config LIMIT 1")
                .fetch_optional(pg)
                .await
                .expect("global_risk_config is readable");
    }
}

impl Drop for Drill {
    fn drop(&mut self) {
        let url = std::mem::take(&mut self.url);
        let holders = std::mem::take(&mut self.holders);
        let components = std::mem::take(&mut self.components);
        let scopes = std::mem::take(&mut self.scopes);
        let events = std::mem::take(&mut self.events);
        let drills = std::mem::take(&mut self.drills);
        let windows = std::mem::take(&mut self.windows);
        let limits = std::mem::take(&mut self.limits);
        let scope_prefixes = std::mem::take(&mut self.scope_prefixes);
        let trading_enabled_before = self.trading_enabled_before.take();
        let took_lease_at = self.took_lease_at;
        let lease = took_lease_at.map(|_| self.lease_before.clone());

        if lease.is_none()
            && holders.is_empty()
            && components.is_empty()
            && scopes.is_empty()
            && events.is_empty()
            && drills.is_empty()
            && windows.is_empty()
            && limits.is_empty()
            && scope_prefixes.is_empty()
            && trading_enabled_before.is_none()
        {
            return;
        }

        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("cleanup runtime");
            rt.block_on(async move {
                // A fresh pool: one test closes the pool it was given, and the
                // rows it wrote still have to go.
                let Ok(pg) = PgPool::connect(&url).await else {
                    eprintln!("CLEANUP FAILED: cannot reconnect to {url}; rows are leaking");
                    return;
                };

                async fn purge(pg: &PgPool, sql: &str, key: &str) {
                    if let Err(e) = sqlx::query(sql).bind(key).execute(pg).await {
                        eprintln!("CLEANUP FAILED ({sql}) for {key}: {e}");
                    }
                }

                for holder in &holders {
                    purge(
                        &pg,
                        "DELETE FROM monitor.heartbeat WHERE component = 'tbot-monitor' \
                         AND instance = $1",
                        holder,
                    )
                    .await;
                }
                for component in &components {
                    purge(
                        &pg,
                        "DELETE FROM monitor.heartbeat WHERE component = $1",
                        component,
                    )
                    .await;
                }
                for scope in &scopes {
                    purge(&pg, "DELETE FROM monitor.events WHERE scope = $1", scope).await;
                    purge(
                        &pg,
                        "DELETE FROM monitor.kill_switch WHERE scope = $1",
                        scope,
                    )
                    .await;
                    // 0165: a venue this test made disagree keeps that as its
                    // last word for ever (verdicts are append-only), and one
                    // left disagreeing refuses every global reset on this
                    // database. So it ends on an agreement, said to be cleanup.
                    if let Some(v) = scope.strip_prefix("venue:") {
                        if let Err(e) = sqlx::query(
                            "INSERT INTO monitor.venue_reconciliation                                (venue, verdict, instruments_checked, detail, checked_by)                              SELECT $1, 'agreed', 0, 'test cleanup', 'test'                               WHERE NOT monitor.venue_reconciled($1)",
                        )
                        .bind(v)
                        .execute(&pg)
                        .await
                        {
                            eprintln!("CLEANUP FAILED: venue {v} left unreconciled: {e}");
                        }
                    }
                }
                for event_id in &events {
                    purge(
                        &pg,
                        "DELETE FROM monitor.events WHERE event_id = $1::uuid",
                        &event_id.to_string(),
                    )
                    .await;
                }
                for drill_id in &drills {
                    purge(
                        &pg,
                        "DELETE FROM monitor.drill WHERE drill_id = $1::uuid",
                        &drill_id.to_string(),
                    )
                    .await;
                }
                for window_id in &windows {
                    purge(
                        &pg,
                        "DELETE FROM monitor.maintenance_window WHERE window_id = $1::uuid",
                        &window_id.to_string(),
                    )
                    .await;
                }
                for strategy_id in &limits {
                    purge(
                        &pg,
                        "DELETE FROM monitor.limits WHERE strategy_id = $1",
                        strategy_id,
                    )
                    .await;
                }
                for prefix in &scope_prefixes {
                    let like = format!("{prefix}%");
                    for sql in [
                        "DELETE FROM monitor.events WHERE scope LIKE $1",
                        "DELETE FROM monitor.kill_switch WHERE scope LIKE $1",
                        "DELETE FROM monitor.drill WHERE scope LIKE $1",
                    ] {
                        purge(&pg, sql, &like).await;
                    }
                }
                // The boot column a global trip flips. The test's own reset puts it
                // back on success; this covers the failure between the two.
                if let Some(enabled) = trading_enabled_before {
                    if let Err(e) = sqlx::query("UPDATE global_risk_config SET trading_enabled = $1")
                        .bind(enabled)
                        .execute(&pg)
                        .await
                    {
                        eprintln!("CLEANUP FAILED: global_risk_config.trading_enabled not restored: {e}");
                    }
                }

                // The `global`/`lease` rows the *watcher* wrote while the drill
                // held the lease. See `Drill::took_lease_at`: a drill that makes
                // the lease lapse makes `LeaseWatcher::note_lease` record a lost
                // lease and then a regained one, and left behind those two rows
                // are a monitor outage in the operator's timeline that never
                // happened.
                if let Some(since) = took_lease_at {
                    if let Err(e) = sqlx::query(
                        "DELETE FROM monitor.events
                          WHERE scope = 'global' AND family = 'lease' AND occurred_at >= $1",
                    )
                    .bind(since)
                    .execute(&pg)
                    .await
                    {
                        eprintln!("CLEANUP FAILED: drill lease events remain: {e}");
                    }
                }

                // The lease last, and only if this drill took it.
                //
                // And only while one of this drill's holders still has it. A guard
                // that drops late must not clobber a monitor — or a later drill —
                // that took the lease over in the meantime: restoring the row
                // under a real monitor would hand the gate a stale expiry and
                // close it on a healthy platform.
                //
                // Restoring a row that had already expired restores an expired
                // row, which closes the gate until the real holder renews. That is
                // the right direction: the alternative — leaving the drill's own
                // live row in place — is a gate held *open* by a monitor that is
                // not running. And a lease that was absent must go back to absent,
                // because "no monitor has ever run here" is a state the gate reads
                // and not a blank to be filled in.
                match lease {
                    None => {}
                    Some(Some(before)) => {
                        if let Err(e) = sqlx::query(
                            "INSERT INTO monitor.lease
                               (lease_id, holder, cadence_ms, renewed_at, expires_at)
                             VALUES ($1, $2, $3, $4, $5)
                             ON CONFLICT (lease_id) DO UPDATE
                               SET holder = EXCLUDED.holder,
                                   cadence_ms = EXCLUDED.cadence_ms,
                                   renewed_at = EXCLUDED.renewed_at,
                                   expires_at = EXCLUDED.expires_at
                               WHERE monitor.lease.holder = ANY($6)",
                        )
                        .bind(monitor::LEASE_ID)
                        .bind(&before.holder)
                        .bind(before.cadence_ms)
                        .bind(before.renewed_at)
                        .bind(before.expires_at)
                        .bind(&holders)
                        .execute(&pg)
                        .await
                        {
                            eprintln!("CLEANUP FAILED: the lease is still held by a drill: {e}");
                        }
                    }
                    Some(None) => {
                        if let Err(e) = sqlx::query(
                            "DELETE FROM monitor.lease WHERE lease_id = $1 AND holder = ANY($2)",
                        )
                        .bind(monitor::LEASE_ID)
                        .bind(&holders)
                        .execute(&pg)
                            .await
                        {
                            eprintln!("CLEANUP FAILED: the lease is still held by a drill: {e}");
                        }
                    }
                }
            });
        });
        // Joined, so the rows are gone before the next test starts — and so a
        // failure to clean up is visible in this test's output rather than in
        // whichever unrelated test trips over the leftovers.
        if handle.join().is_err() {
            eprintln!("CLEANUP FAILED: the cleanup thread panicked");
        }
    }
}

// ── The guard's own property ────────────────────────────────────────────────

#[tokio::test]
async fn a_drill_that_panics_still_hands_back_the_lease_and_the_heartbeat() {
    // The property [`Drill`] exists to have, asserted rather than described.
    //
    // Cleanup that ran only on the success path is exactly what produced the leak
    // this guard replaced — six `monitor.heartbeat` rows per run that nothing
    // evicts, and a `monitor.lease` row left expired under a `drill-<uuid>`
    // holder, which is why the order gate on this box refused every order with
    // `monitor_unavailable`. A failing drill is the case that leaks, so a failing
    // drill is the case this pins.
    let (pg, _outer) = db!();
    let url = std::env::var("MONITOR_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("API_TEST_DATABASE_URL"))
        .expect("db! did not return early, so one of these is set");

    async fn state(pg: &PgPool) -> (i64, Option<(String, DateTime<Utc>)>) {
        let beats: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM monitor.heartbeat WHERE instance LIKE 'drill-%'",
        )
        .fetch_one(pg)
        .await
        .expect("count");
        let lease: Option<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT holder, expires_at FROM monitor.lease WHERE lease_id = $1")
                .bind(monitor::LEASE_ID)
                .fetch_optional(pg)
                .await
                .expect("lease");
        (beats, lease)
    }

    let before = state(&pg).await;

    // A drill that renews, beats, and then fails. Its guard is a local of the
    // task, so it is dropped by the unwind and by nothing else.
    let inner_url = url.clone();
    let task = tokio::spawn(async move {
        let inner = PgPool::connect(&inner_url).await.expect("connect");
        let mut drill = Drill::begin(&inner, inner_url.clone()).await;
        let terms = drill.terms(60_000);
        MonitorService::new(MonitorStore::new(inner.clone()), terms)
            .tick()
            .await
            .expect("renew");
        panic!("the drill fails in the middle, which is the case that leaked");
    });
    assert!(
        task.await.is_err(),
        "this test needs the drill to have panicked; if it returned, it is asserting nothing"
    );

    let after = state(&pg).await;
    assert_eq!(
        after, before,
        "a drill that panicked left a heartbeat row behind, or left the lease under \
         its own holder. Either one is the defect this guard was written to close."
    );
}

// ── §8.1 / §8.2 — killing the monitor stops order flow ──────────────────────

#[tokio::test]
async fn killing_the_monitor_stops_order_flow_within_one_cadence() {
    // The drill this whole track exists to pass. A real monitor renews a real
    // lease in a real database; the runtime trades. The monitor is then stopped —
    // not told to halt, just stopped — and within one cadence the runtime refuses.
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());

    // The cadence is a real wall-clock budget, not a knob for making the test
    // quick. A lease is good for exactly one cadence, so everything between
    // `tick()` and the refresh that follows it — two round trips to a Postgres on
    // a box that is also running three other cargo builds — has to fit inside it,
    // or the lease this test needs alive has already lapsed and the assertion
    // fails for a reason that has nothing to do with the monitor.
    //
    // At 300 ms it did: `killing_the_monitor_...` failed roughly one run in three
    // on 2026-09-21, at "a live lease opens the gate" and at the resume assertion
    // — both of them the *valid* side. A drill that is green two times in three is
    // a drill that gets re-run until it is green, which is the same habit COMP-007
    // §4.3 exists to prevent one layer up.
    //
    // Three seconds is slack, not slowness: the property under test is that a dead
    // monitor loses its lease within one cadence, and it is asserted the same way
    // at 3 s as at 60 s.
    let cadence_ms = 3_000;
    let terms = drill.terms(cadence_ms);
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
    let first = gate.check(intent(), &ctx());
    assert!(first.is_ok(), "{first:?}");

    // The monitor dies here. Nothing tells the runtime; the renewal simply stops.
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(cadence_ms).unwrap() + 100,
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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    let terms = drill.terms(60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("renew");

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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    clear_scope(&pg, "global").await;
    drill.owns_scope("global");
    drill.remembers_trading_enabled(&pg).await;

    let terms = drill.terms(60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("renew");

    // Trip, as `POST /api/trading/kill` now does.
    let event = MonitorEvent::new(
        Scope::Global,
        Family::Platform,
        EventKind::Tripped,
        "human:test",
    )
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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    let bad = format!("kraken-{}", Uuid::new_v4());
    let good = format!("alpaca-{}", Uuid::new_v4());
    let bad_scope = Scope::venue(&bad);
    drill.owns_scope(bad_scope.to_string());

    let terms = drill.terms(60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("renew");

    // Injected the way `crates/reconciliation` raises it (0165): a `disagreed`
    // verdict, then the durable trip. A bare `trip` with no verdict is a venue
    // halted for some other reason, which §8.3 does not hold to reconciliation.
    let (ks, gate, watcher) = runtime(&store);
    store
        .carry_out_mismatch(&bad, &ks)
        .await
        .expect("mismatch recorded");
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
    assert!(
        refused.is_err(),
        "a global resume must wait for reconciliation"
    );
    // And the venue itself stays halted until a reconciliation of it agrees.
    let venue_reset = ResetRequest {
        scope: bad_scope.clone(),
        typed_confirmation: bad_scope.to_string(),
        reason: "trying to resume the venue".to_owned(),
        actor: "human:test".to_owned(),
        all_venues_reconciled: true,
    };
    assert!(
        matches!(
            store.reset(&venue_reset).await,
            Err(monitor::store::StoreError::Reset(
                monitor::control::ResetRefusal::VenueNotReconciled { .. }
            ))
        ),
        "a venue that disagrees with our book was resumable"
    );
    store
        .record_venue_check(
            &bad,
            monitor::reconciled::VenueVerdict::Agreed,
            1,
            None,
            "reconciliation",
        )
        .await
        .expect("agreed");
    store
        .reset(&venue_reset)
        .await
        .expect("clean now, so a human may resume it");
}

// ── §8.5 — the trip record exists before the first refused order ────────────

#[tokio::test]
async fn the_trip_record_is_present_before_the_first_refused_order() {
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let scope = Scope::strategy(&sid);
    drill.owns_scope(scope.to_string());

    let terms = drill.terms(60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("renew");

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
    let (pg, mut drill) = db!();
    let scope = format!("strategy:agent-probe-{}", Uuid::new_v4());
    drill.owns_scope(scope.clone());

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
    assert!(
        authored.is_err(),
        "an agent must not author a monitor event"
    );
}

#[tokio::test]
async fn no_agent_tripped_row_exists_anywhere() {
    // §8.6 as a standing assertion over the whole table rather than over one
    // inserted row: whatever else has happened to this database, nothing has ever
    // recorded an agent as tripping or resetting.
    //
    // It writes nothing, so its guard has nothing to reap — but it still holds
    // one, because a test that can reach a pool without a guard is the shape the
    // guard exists to remove.
    let (pg, _drill) = db!();
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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    let scope = Scope::strategy(&sid);
    drill.owns_scope(scope.to_string());

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
    store
        .reset(&request)
        .await
        .expect("the first reset is allowed");

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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    let sid = format!("s-{}", Uuid::new_v4());
    drill.owns_limits_for(sid.clone());

    let book = store.load_limits().await.expect("limits");
    assert!(
        book.may_promote_to_live(&sid).is_err(),
        "a strategy nobody declared limits for must not reach live"
    );

    for (metric, family, method) in [
        (
            "fill_rate",
            "execution",
            monitor::LimitMethod::PaperEwma3Sigma,
        ),
        (
            "orders_per_minute",
            "behaviour",
            monitor::LimitMethod::VenueRateCeiling,
        ),
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
    assert!(
        refused.is_err(),
        "an agent must not declare a control limit"
    );
}

// ── §4.3 — a window suppresses a platform trip and nothing else ─────────────

#[tokio::test]
async fn an_open_window_is_visible_attributed_and_bounded() {
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());
    let window = monitor::MaintenanceWindow::open("human:test", "rolling deploy", 15, Utc::now())
        .expect("window");
    drill.owns_window(window.window_id);
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
    let (pg, mut drill) = db!();
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
    drill.owns_drill_record(slow.drill_id);
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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());

    let recorded = format!("kraken-{}", Uuid::new_v4());
    let in_memory_only = format!("alpaca-{}", Uuid::new_v4());
    let untouched = format!("coinbase-{}", Uuid::new_v4());
    drill.owns_scope(Scope::venue(&recorded).to_string());

    let terms = drill.terms(60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("renew");

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
        gate.check(intent(), &ctx().on_venue(&in_memory_only))
            .is_ok(),
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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());

    // By prefix, and before `run`: it mints `drill:track-d4-<uuid>` inside the
    // call, so a failure in there would otherwise leave rows nobody registered.
    drill.owns_scope_prefix("drill:track-d4-");
    let rehearsal =
        monitor::Rehearsal::new(store.clone(), 60_000, "human:test").expect("a human may rehearse");
    let outcome = rehearsal
        .run(monitor::DrillTrigger::Manual, "track-d4")
        .await
        .expect("the rehearsal completes");
    drill.owns_drill_record(outcome.drill_id);
    drill.owns_scope(outcome.scope.to_string());

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
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());

    let component = format!("api-{}", Uuid::new_v4());
    drill.owns_component(component.clone());
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
    drill.owns_window(window.window_id);
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
    let event = MonitorEvent::new(
        Scope::Global,
        Family::Platform,
        EventKind::Warning,
        "monitor",
    )
    .expect("monitor")
    .with_reason(verdict.reason())
    .suppressed_by(window.window_id);
    drill.owns_event(event.event_id);
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

// ── The last exit criterion — "within one event of a trip" ──────────────────

#[tokio::test]
async fn a_trip_reaches_the_order_gate_without_waiting_a_cadence() {
    // Phase 7's last exit criterion is that the kill-switch state is visible
    // "within one event of a trip", and migrations 0074 and 0075 were written to
    // make that possible: each installs an `AFTER INSERT OR UPDATE` trigger that
    // calls `pg_notify`. `0074`'s comment states the consequence outright — the
    // mirror "is refreshed on every lease renewal and on this NOTIFY, so a trip
    // propagates without waiting a cadence".
    //
    // Until 2026-09-23 nothing in the workspace issued a `LISTEN`. Both triggers
    // published into an empty room and `LeaseWatcher::run` was a
    // `sleep(cadence)` loop, so a halt reached the order gate when the timer next
    // came round.
    //
    // The cadence here is deliberately absurd. A minute is far longer than any
    // real `MONITOR_MIRROR_REFRESH_MS`, and that is the point: with the poll as
    // the only mechanism this test cannot pass inside its five-second budget, so
    // it is measuring the notification and nothing else. Reverted to the
    // pre-change `run`, it fails on the timeout.
    let (pg, mut drill) = db!();
    let store = MonitorStore::new(pg.clone());

    let venue = format!("notify-{}", Uuid::new_v4());
    let scope = Scope::venue(&venue);
    drill.owns_scope(scope.to_string());

    // A valid lease, so the watcher's first refresh sees a healthy monitor and
    // records no lease gap. Without it this test would write `global`/`lease`
    // events into the operator's timeline for an outage that never happened.
    let terms = drill.terms(60_000);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("the monitor renews its lease");

    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    // A minute, so the cadence cannot be what refreshes the mirror inside this
    // test: if the assertion below passes, the notification is the only thing
    // that could have carried the trip.
    let watcher = LeaseWatcher::new(store.clone(), Arc::clone(&ks), Duration::from_secs(60));

    // The watcher runs as `AppState` spawns it: the whole loop, not a hand-driven
    // `refresh`. A test that called `refresh` itself would prove nothing about
    // when production refreshes, which is the entire question.
    let running = tokio::spawn(watcher.run());

    // Wait for the first pass to have happened, so the assertion below is about a
    // trip that arrived *after* the loop settled into its wait.
    for _ in 0..100 {
        if ks.monitor_available() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        ks.monitor_available(),
        "the watcher's first refresh never ran; this test is broken, not passing"
    );
    assert!(
        gate.check(intent(), &ctx().on_venue(&venue)).is_ok(),
        "the venue trades before the trip"
    );

    // The monitor process trips the venue. `MonitorStore::trip` writes the
    // durable row and the event and touches no in-memory mirror — which is what
    // makes this a test of propagation rather than of a shared `Arc`. The
    // trigger fires inside the same transaction's commit.
    let event = MonitorEvent::new(scope.clone(), Family::Venue, EventKind::Tripped, "monitor")
        .expect("monitor is not an agent actor")
        .with_reason("the notification drill halts one venue");
    drill.owns_event(event.event_id);
    store
        .trip(&event, "the notification drill halts one venue")
        .await
        .expect("the trip is recorded and the row written");

    // Five seconds against a sixty-second cadence. Anything that passes here
    // passed because of the NOTIFY.
    let mut propagated_in_ms = None;
    let started = std::time::Instant::now();
    for _ in 0..100 {
        if ks.is_active_scope(&scope) {
            propagated_in_ms = Some(started.elapsed().as_millis());
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    running.abort();

    let ms = propagated_in_ms.unwrap_or_else(|| {
        panic!(
            "five seconds after the trip committed, the order gate's mirror still \
             says {venue} may trade. The cadence is 60 s, so the only thing that \
             could have carried this is the pg_notify from migration 0075 — and \
             nothing was listening for it."
        )
    });
    assert!(
        ms < 5_000,
        "propagation took {ms} ms, which is not 'within one event'"
    );
    assert!(
        matches!(
            gate.check(intent(), &ctx().on_venue(&venue)),
            Err(RiskRejection::KillSwitchActive)
        ),
        "the mirror knows the venue is halted and the gate does not refuse it — the \
         assertion above is reading something the order path does not"
    );
}
