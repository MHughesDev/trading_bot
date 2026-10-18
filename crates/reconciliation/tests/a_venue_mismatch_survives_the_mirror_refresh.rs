//! S-7.6, the drill: a halt raised by reconciliation is still a halt after the
//! refresh that erases an in-memory one.
//!
//! The defect this exists to close: `check_position_divergence_at_venue` used to
//! halt a venue with `KillSwitch::trip_scope` and write nothing. The order gate's
//! scoped view is refilled by `KillSwitch::replace_halted`, which **replaces** it
//! with whatever `monitor.kill_switch` says — so a halt raised because our book
//! and the venue disagree about what is owned was erased on the next refresh,
//! about five seconds later in the platform, and the venue resumed trading with
//! nothing recorded anywhere.
//!
//! So the refresh here is real: a `LeaseWatcher` built exactly as `AppState`
//! builds one, reading the same database, called between the halt and the
//! assertion. Asserting only that the venue is halted *immediately* after the
//! divergence is what let the defect live — the broken version passes that.
//!
//! ```bash
//! RECONCILIATION_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p reconciliation --test a_venue_mismatch_survives_the_mirror_refresh \
//!   -- --test-threads=1
//! ```
//!
//! With no URL set the suite falls back to the platform's own configured
//! database (`config/default.toml`). If that will not answer either, the two
//! database-backed tests print a banner naming themselves as NOT RUN rather than
//! reporting a quiet pass — five of `crates/monitor`'s eleven drills spent
//! months green and vacuous for want of one. The third test needs no database
//! and always runs.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use monitor::lease::LeaseTerms;
use monitor::service::{LeaseWatcher, MonitorService};
use monitor::store::MonitorStore;
use reconciliation::divergence::check_position_divergence_at_venue;
use reconciliation::{reconcile_all, InternalPosition, ReconcileOutcome};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate, Scope};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

/// The database the platform itself is configured against (`config/default.toml`).
const CONFIGURED_URL: &str = "postgres://trading:trading@localhost:5432/trading";

/// Long enough that the whole drill runs inside one lease, short enough that a
/// killed process leaves the platform's order gate open for seconds rather than
/// minutes. [`Lease`] is deleted on the way out either way.
///
/// [`Lease`]: monitor::lease::Lease
const LEASE_CADENCE_MS: i64 = 10_000;

// ── The harness ──────────────────────────────────────────────────────────────

/// Connect, or say loudly that this test did not run.
///
/// Three outcomes, and only one of them is silent:
/// * a database answers and carries the monitor schema — the test runs;
/// * a URL was configured and will not answer, or answers without the schema —
///   a panic, because a database somebody pointed this at and that cannot serve
///   it is a failure and not a skip;
/// * nothing was configured and the default will not answer — a banner.
async fn database(test_name: &str) -> Option<(PgPool, String)> {
    let configured = [
        "RECONCILIATION_TEST_DATABASE_URL",
        "MONITOR_TEST_DATABASE_URL",
        "API_TEST_DATABASE_URL",
        "DATABASE_URL",
    ]
    .iter()
    .find_map(|key| {
        std::env::var(key)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| (*key, v))
    });

    let (source, url) = match &configured {
        Some((key, url)) => (*key, url.as_str()),
        None => ("config/default.toml", CONFIGURED_URL),
    };

    let pool = match PgPool::connect(url).await {
        Ok(pool) => pool,
        Err(e) => {
            assert!(
                configured.is_none(),
                "{source} names a database that will not answer: {e}. A configured \
                 database that cannot be reached is a failure, not a skip."
            );
            eprintln!(
                "\n\
                 ================================================================\n\
                 NOT RUN: {test_name}\n\
                 ----------------------------------------------------------------\n\
                 No database. Tried {url} ({source}): {e}\n\
                 \n\
                 This test is the only check that a reconciliation halt survives\n\
                 the mirror refresh. Nothing below it ran. A green suite without\n\
                 this line does not mean the halt is durable.\n\
                 Set RECONCILIATION_TEST_DATABASE_URL to run it.\n\
                 ================================================================\n"
            );
            return None;
        }
    };

    let has_schema: bool =
        sqlx::query_scalar("SELECT to_regclass('monitor.kill_switch') IS NOT NULL")
            .fetch_one(&pool)
            .await
            .expect("the database answers");
    assert!(
        has_schema,
        "{source} points at a database with no `monitor.kill_switch`. Passing \
         against it would mean nothing; run the migrations first."
    );

    Some((pool, url.to_owned()))
}

/// Deletes what the test wrote, whether or not the test panics.
///
/// A leaked `venue:*` halt is not inert: `crates/api/src/routes/monitor.rs`
/// derives `all_venues_reconciled` from "no venue is halted", so one abandoned
/// row refuses every global reset until somebody finds it by hand. And a leaked
/// `monitor.lease` row would leave the platform's order gate open on a box where
/// no monitor is running.
///
/// `Drop` cannot await, so the deletes run on a borrowed thread with a runtime of
/// their own — which works because this thread is not the one blocking on them.
struct Cleanup {
    url: String,
    scopes: Vec<String>,
    /// Whether a lease row existed before the test. Only a row this test created
    /// is removed.
    lease_was_absent: bool,
    /// The `tbot-monitor` instance this test beat as, for
    /// `monitor.heartbeat`.
    ///
    /// The lease was always deleted here and the heartbeat never was, which is
    /// not a small asymmetry: `MonitorService::tick` writes both, so every green
    /// run of this file left one more permanently stale `tbot-monitor` instance
    /// behind. Nine of them had accumulated in the live database by 2026-09-21,
    /// beside sixteen from `crates/monitor`'s drills — and a stale
    /// `tbot-monitor` beat is the input `monitor::platform::assess` turns into a
    /// global halt.
    holder: String,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let url = self.url.clone();
        let scopes = std::mem::take(&mut self.scopes);
        let drop_lease = self.lease_was_absent;
        let holder = std::mem::take(&mut self.holder);
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("cleanup runtime");
            rt.block_on(async move {
                let Ok(pg) = PgPool::connect(&url).await else {
                    eprintln!("CLEANUP FAILED: cannot reconnect to delete {scopes:?}");
                    return;
                };
                for scope in &scopes {
                    for sql in [
                        "DELETE FROM monitor.events WHERE scope = $1",
                        "DELETE FROM monitor.kill_switch WHERE scope = $1",
                    ] {
                        if let Err(e) = sqlx::query(sql).bind(scope).execute(&pg).await {
                            eprintln!("CLEANUP FAILED for {scope}: {e}");
                        }
                    }
                }
                if let Err(e) = sqlx::query(
                    "DELETE FROM monitor.heartbeat WHERE component = $1 AND instance = $2",
                )
                .bind(monitor::LEASE_ID)
                .bind(&holder)
                .execute(&pg)
                .await
                {
                    eprintln!("CLEANUP FAILED for the heartbeat row: {e}");
                }
                if drop_lease {
                    if let Err(e) = sqlx::query("DELETE FROM monitor.lease WHERE lease_id = $1")
                        .bind(monitor::LEASE_ID)
                        .execute(&pg)
                        .await
                    {
                        eprintln!("CLEANUP FAILED for the lease row: {e}");
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

/// A context an order can actually pass on.
///
/// The mark price is load-bearing: `GateContext::for_manual_order` turns a `None`
/// into `Measured::No` and the gate then refuses with `MarketDataUnavailable`
/// *before* it reaches the kill switch, which would make every assertion below
/// true for the wrong reason.
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

/// The order gate as `api::AppState` builds it: a fresh mirror, and a watcher
/// that refills it from the database.
fn runtime(store: &MonitorStore) -> (Arc<KillSwitch>, RiskGate, LeaseWatcher) {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    let watcher = LeaseWatcher::new(store.clone(), Arc::clone(&ks), Duration::from_millis(50));
    (ks, gate, watcher)
}

/// Take the lease so the gate is not refusing everything with
/// `monitor_unavailable`; report whether this test is the one that created the
/// row, and under which holder it beat.
///
/// The holder is returned rather than discarded because `tick` writes two rows,
/// not one, and the caller has to be able to name both to the guard.
async fn hold_the_lease(store: &MonitorStore, pg: &PgPool) -> (bool, String) {
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM monitor.lease WHERE lease_id = $1")
        .bind(monitor::LEASE_ID)
        .fetch_one(pg)
        .await
        .expect("count the lease");
    let holder = format!("reconciliation-drill-{}", Uuid::new_v4());
    let terms = LeaseTerms::new(holder.clone(), LEASE_CADENCE_MS);
    MonitorService::new(store.clone(), terms)
        .tick()
        .await
        .expect("the drill renews the lease");
    (before == 0, holder)
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

// ── The drill ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_reconciliation_halt_is_still_a_halt_after_the_mirror_refresh() {
    let Some((pg, url)) =
        database("a_reconciliation_halt_is_still_a_halt_after_the_mirror_refresh").await
    else {
        return;
    };
    let store = MonitorStore::new(pg.clone());

    // Unique venue ids, so the drill does not fight the running platform and so
    // a leftover row from an earlier run cannot make it pass.
    let diverging = format!("kraken-{}", Uuid::new_v4());
    let quiet = format!("alpaca-{}", Uuid::new_v4());

    let (lease_was_absent, holder) = hold_the_lease(&store, &pg).await;
    let _cleanup = Cleanup {
        url: url.clone(),
        scopes: vec![
            Scope::venue(&diverging).to_string(),
            Scope::venue(&quiet).to_string(),
        ],
        lease_was_absent,
        holder,
    };

    let (ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;
    gate.check(intent(), &ctx().on_venue(&diverging))
        .expect("before the divergence this venue trades");

    // Our book says we are long 1; the venue says 2. That is a disagreement
    // about what is owned, which is the only fact this raises.
    let outcome = check_position_divergence_at_venue(
        "BTC-USD",
        &diverging,
        dec("1.0"),
        dec("2.0"),
        &store,
        &ks,
    )
    .await
    .expect("the mismatch is recorded and the venue halted");
    assert!(matches!(outcome, ReconcileOutcome::Diverged { .. }));

    // Immediately: halted. The broken version passes this line too, which is
    // exactly why the line below it exists.
    assert!(
        matches!(
            gate.check(intent(), &ctx().on_venue(&diverging)),
            Err(RiskRejection::KillSwitchActive)
        ),
        "the venue that disagrees with our book must stop taking orders"
    );

    // The refresh that used to erase it. Not "could happen" — it happens here,
    // reading the same database the platform's watcher reads.
    watcher.refresh().await;

    assert!(
        matches!(
            gate.check(intent(), &ctx().on_venue(&diverging)),
            Err(RiskRejection::KillSwitchActive)
        ),
        "THE DEFECT: the halt was in memory only, so the refresh that refills the \
         mirror from monitor.kill_switch dropped it and the venue resumed trading"
    );
    gate.check(intent(), &ctx().on_venue(&quiet))
        .expect("a mismatch at one venue says nothing about the others");

    // And the record is there to read afterwards, naming the venue.
    let scope = Scope::venue(&diverging).to_string();
    let reason: String = sqlx::query_scalar(
        "SELECT reason FROM monitor.kill_switch WHERE scope = $1 AND state = 'halted'",
    )
    .bind(&scope)
    .fetch_one(&pg)
    .await
    .expect("the durable row is there");
    assert!(reason.contains(&diverging), "{reason}");
}

#[tokio::test]
async fn a_sweep_halts_the_venue_once_however_many_instruments_disagree() {
    let Some((pg, url)) =
        database("a_sweep_halts_the_venue_once_however_many_instruments_disagree").await
    else {
        return;
    };
    let store = MonitorStore::new(pg.clone());
    let venue = format!("coinbase-{}", Uuid::new_v4());

    let (lease_was_absent, holder) = hold_the_lease(&store, &pg).await;
    let _cleanup = Cleanup {
        url: url.clone(),
        scopes: vec![Scope::venue(&venue).to_string()],
        lease_was_absent,
        holder,
    };

    let (ks, gate, watcher) = runtime(&store);
    watcher.refresh().await;

    // Three instruments, two of them wrong. The venue reports nothing at all,
    // which is a venue saying "flat in everything" — and we are not.
    let internal = vec![
        InternalPosition {
            instrument_id: "BTC-USD".to_owned(),
            quantity: dec("1.0"),
        },
        InternalPosition {
            instrument_id: "ETH-USD".to_owned(),
            quantity: dec("4.0"),
        },
        InternalPosition {
            instrument_id: "SOL-USD".to_owned(),
            quantity: Decimal::ZERO,
        },
    ];
    let outcomes = reconcile_all(&internal, &venue, &[], &store, &ks)
        .await
        .expect("the sweep reaches a verdict and records it");
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, ReconcileOutcome::Diverged { .. }))
            .count(),
        2,
        "a position we hold and the venue does not list is a disagreement; a \
         flat one is not"
    );

    watcher.refresh().await;
    assert!(
        matches!(
            gate.check(intent(), &ctx().on_venue(&venue)),
            Err(RiskRejection::KillSwitchActive)
        ),
        "the sweep's halt must survive the refresh too"
    );

    // One sweep is one incident. `monitor.events` is the timeline an operator
    // reads to reconstruct what happened, and two instruments disagreeing in the
    // same sweep is not two incidents.
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM monitor.events WHERE scope = $1 AND kind = 'halted'",
    )
    .bind(Scope::venue(&venue).to_string())
    .fetch_one(&pg)
    .await
    .expect("count");
    assert_eq!(events, 1, "one sweep, one trip record");
}

// ── The invariant, without a database ────────────────────────────────────────

/// No halt leaves this crate except through the durable receiving half.
///
/// The rule is not a paragraph in a module doc: a reviewer adding a second raise
/// site would have to delete this test to reintroduce the defect, and deleting a
/// test is a thing a diff shows.
#[test]
fn no_halt_in_this_crate_is_raised_by_a_bare_scope_trip() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0usize;
    for file in ["src/divergence.rs", "src/positions.rs", "src/recorder.rs"] {
        let path: PathBuf = root.join(file);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is unreadable: {e}", path.display()));
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("trip_scope"),
            "{file} raises a scoped halt directly. KillSwitch::replace_halted \
             replaces the scoped view from monitor.kill_switch on every refresh, \
             so a trip with no row behind it is erased within one cadence. Raise \
             it through MonitorStore::carry_out_mismatch instead."
        );
        checked += 1;
    }
    assert_eq!(checked, 3, "every raise site in this crate must be checked");

    let divergence = std::fs::read_to_string(root.join("src/divergence.rs")).expect("readable");
    assert!(
        divergence.contains("carry_out_mismatch"),
        "the venue-scoped halt must go through the durable receiving half"
    );
}
