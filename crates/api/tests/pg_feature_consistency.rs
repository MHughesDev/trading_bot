//! The nightly consistency diff, end to end (SPEC §3.3, INV-14; checklist 0.21).
//!
//! The diff logic has unit tests. This is the thing they cannot prove: that a
//! **live serve** writes a log row a **later recomputation** can find, read back
//! through the real point-in-time reader, and diagnose. Every piece in between —
//! the serving log's schema, RLS, the tenant transaction, the instrument
//! resolution, the bar-window arithmetic the recompute uses to re-find the
//! decision row — only exists on this path.
//!
//! Ignored by default; needs live Postgres **and** ClickHouse:
//!
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//! CONSISTENCY_E2E_CLICKHOUSE_URL=http://trading:trading@localhost:8123 \
//!   cargo test -p api --test pg_feature_consistency -- --ignored --test-threads=1
//! ```

use chrono::{Duration, TimeZone, Utc};
use sqlx::{Connection, Executor, PgConnection, PgPool, Row};
use uuid::Uuid;

use backtest::store::{BarStore, CollectedBar};
use domain::payloads::bar::Timeframe;

fn pg_base() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

// 0046 backfills surrogate identity from the engine's symbol-keyed catalogue, so
// that catalogue has to exist first.
const M0002: &str = include_str!("../../../migrations/0002_instruments.sql");
const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0044: &str = include_str!("../../../migrations/0044_roles_grants_rls.sql");
const M0046: &str = include_str!("../../../migrations/0046_dataplane.sql");
const M0047: &str = include_str!("../../../migrations/0047_ledger_fixation.sql");
const M0048: &str = include_str!("../../../migrations/0048_feature_consistency.sql");

async fn scratch_pg(name: &str) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&pg_base(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&pg_base(), name)).await.unwrap();
    pool.execute("CREATE TABLE IF NOT EXISTS public._sqlx_migrations (version BIGINT PRIMARY KEY)")
        .await
        .unwrap();
    for m in [M0002, M0043, M0044, M0046, M0047, M0048] {
        pool.execute(m).await.expect("migration applies");
    }
    pool
}

async fn scratch_clickhouse(base: &str, db: &str) -> String {
    let admin = storage::clickhouse::connect(base);
    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    let (scheme, rest) = base.split_once("://").expect("scheme");
    let url = format!("{scheme}://{}/{db}", rest.split('/').next().unwrap());
    storage::clickhouse::migrate::run_migrations(&url)
        .await
        .expect("apply clickhouse schema");
    url
}

/// A live serve is logged, and the next day's recomputation finds it, agrees
/// with it, and records both knowledge times.
#[tokio::test]
#[ignore = "requires live postgres and clickhouse"]
async fn a_live_serve_is_logged_and_the_nightly_diff_recomputes_it() {
    let Ok(ch_base) = std::env::var("CONSISTENCY_E2E_CLICKHOUSE_URL") else {
        eprintln!("CONSISTENCY_E2E_CLICKHOUSE_URL unset — skipping");
        return;
    };
    let pg = scratch_pg("consistency_e2e").await;
    let ch = scratch_clickhouse(&ch_base, "consistency_e2e").await;

    // A live source: bars written within the receipt tolerance are *observed*,
    // so their knowledge time is real rather than a backfill sentinel.
    let identity = std::sync::Arc::new(storage::identity::MemoryIdentity::new());
    identity.add_source(
        "consistency_e2e",
        storage::identity::SourceInfo {
            source_id: 17,
            declared_vendor_lag: std::time::Duration::ZERO,
            live: false,
        },
    );
    storage::identity::install(identity.clone());

    let store = BarStore::connect(&ch);
    let symbol = format!("CONS-{}", Uuid::new_v4().simple());
    let tf = Timeframe::Minutes1;
    let start = Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap();

    // 300 bars: enough for `ema_7` (5·7) and `rsi_14` (5·14+1) to have complete
    // windows, so the serve returns real values rather than nulls.
    let bars: Vec<CollectedBar> = (0..300i64)
        .map(|i| {
            let close = 100.0 + (i as f64 * 0.21).sin() * 6.0;
            CollectedBar {
                available_time: start + Duration::minutes(i),
                sequence: u64::try_from(i).unwrap(),
                open: format!("{close:.8}"),
                high: format!("{:.8}", close + 0.5),
                low: format!("{:.8}", close - 0.5),
                close: format!("{close:.8}"),
                volume: "3".to_string(),
                trade_count: 3,
            }
        })
        .collect();
    store
        .insert_collected(&symbol, "binance", "consistency_e2e", "test", tf, &bars)
        .await
        .expect("seed market_bar");
    identity.sync_dims(&ch).await.expect("sync identity dims");

    let (instrument_id, venue_id) = store
        .resolve_instrument(&symbol)
        .await
        .expect("resolve")
        .expect("the seeded symbol has a surrogate id");

    let loaded = store
        .load_bars(&symbol, tf, start - Duration::minutes(1), start + Duration::minutes(301))
        .await
        .expect("load_bars");
    assert_eq!(loaded.len(), 300, "every seeded bar reads back");

    // ── the live serve ──────────────────────────────────────────────────────
    assert!(
        api::feature_consistency::serving_tenants(&pg)
            .await
            .expect("list tenants")
            .is_empty(),
        "nothing has served yet"
    );
    let tenant = format!("t-{}", Uuid::new_v4().simple());
    let names: Vec<String> = vec!["ema_7".into(), "rsi_14".into(), "zscore_20".into()];
    let ctx = api::features_compute::ServeContext {
        pg: &pg,
        tenant: &tenant,
        instrument_id,
        venue_id,
        period_secs: 60,
        feature_set_id: "fs_consistency_e2e",
    };
    let served = api::features_compute::serve_vector(&ctx, &loaded, &names)
        .await
        .expect("serve");
    for n in &names {
        assert!(
            served.get(n).is_some_and(|v| v.is_number()),
            "{n} served a real value"
        );
        // INV-11: never a bare value.
        assert!(served.contains_key(&format!("{n}_age_minutes")));
        assert!(served.contains_key(&format!("{n}_quality")));
    }

    let logged: i64 = sqlx::query_scalar("SELECT count(*) FROM dataplane.feature_serving_log")
        .fetch_one(&pg)
        .await
        .unwrap();
    assert_eq!(logged, 1, "the serve wrote exactly one log row");

    // The discovery the scheduled job walks. A job that lists no tenants does
    // nothing and reports success, so this is the difference between the job
    // running and the job appearing to run.
    assert_eq!(
        api::feature_consistency::serving_tenants(&pg)
            .await
            .expect("list tenants"),
        vec![tenant.clone()],
        "the scheduled job discovers the tenant that served"
    );

    // ── the nightly diff ────────────────────────────────────────────────────
    let now = Utc::now();
    let report = api::feature_consistency::diff_tenant(
        &pg,
        &ch,
        &tenant,
        now - Duration::days(1),
        now + Duration::days(1),
    )
    .await
    .expect("diff runs");

    assert_eq!(report.serves, 1, "the diff found the serve");
    assert_eq!(
        report.unrecomputable, 0,
        "every served feature could be recomputed"
    );
    assert_eq!(report.diffs, names.len(), "one diff row per served feature");
    assert_eq!(report.code_drift, 0, "the same code produced the same values");
    assert!(
        report.p99_relative_diff < 1e-9,
        "p99 relative diff {} breaches the SLO",
        report.p99_relative_diff
    );

    // Both knowledge times are on the row. That column is the whole point: it is
    // what turns "the numbers differ" into "the backfill assumed data that
    // arrived late".
    let rows = sqlx::query(
        "SELECT feature_id, diagnosis, served_knowledge_time, recomputed_knowledge_time, abs_diff \
           FROM dataplane.feature_consistency_diff ORDER BY feature_id",
    )
    .fetch_all(&pg)
    .await
    .unwrap();
    assert_eq!(rows.len(), names.len());
    for r in &rows {
        assert_eq!(
            r.get::<String, _>("diagnosis"),
            "match",
            "{} did not match",
            r.get::<String, _>("feature_id")
        );
        assert!(r.get::<f64, _>("abs_diff") < 1e-9);
        assert!(
            r.try_get::<chrono::DateTime<Utc>, _>("served_knowledge_time").is_ok()
                && r.try_get::<chrono::DateTime<Utc>, _>("recomputed_knowledge_time").is_ok(),
            "both knowledge times are recorded"
        );
    }

    // A second pass is idempotent: a serve is diffed once.
    let again = api::feature_consistency::diff_tenant(
        &pg,
        &ch,
        &tenant,
        now - Duration::days(1),
        now + Duration::days(1),
    )
    .await
    .expect("second pass");
    assert_eq!(again.serves, 0, "an already-diffed serve is not re-diffed");
}
