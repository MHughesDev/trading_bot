//! ClickHouse migration runner against a real server (Set L, L-0.2).
//!
//! The unit tests in `storage::clickhouse::migrate` prove the SQL is *split*
//! correctly. They cannot prove ClickHouse *accepts* it, nor that replaying the DDL
//! on an existing database is a no-op — which is the entire point of the module,
//! since `/docker-entrypoint-initdb.d` only ever runs on a fresh volume.
//!
//! Gated on `STORAGE_MIGRATE_CLICKHOUSE_URL`; without it the test logs and returns,
//! keeping `cargo test` green where there is no service. Same discipline as
//! `backtest/tests/e2e.rs`.
//!
//! To run it:
//!
//! ```bash
//! STORAGE_MIGRATE_CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//!   cargo test -j 2 -p storage --test clickhouse_migrate -- --nocapture
//! ```
//!
//! It writes only to a throwaway database (`_migrate_test_<pid>`), which it drops
//! on the way out. It never touches `trading`.

use storage::clickhouse::{backfill, connect, migrate};

fn url_from_env() -> Option<String> {
    std::env::var("STORAGE_MIGRATE_CLICKHOUSE_URL").ok()
}

/// Rewrites the database component of a ClickHouse URL.
fn with_database(url: &str, database: &str) -> String {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let scheme = if url.starts_with("https") {
        "https"
    } else {
        "http"
    };
    let host_path = match after_scheme.rfind('@') {
        Some(at) => &after_scheme[at + 1..],
        None => after_scheme,
    };
    let creds = match after_scheme.rfind('@') {
        Some(at) => format!("{}@", &after_scheme[..at]),
        None => String::new(),
    };
    let host_port = match host_path.find('/') {
        Some(slash) => &host_path[..slash],
        None => host_path,
    };
    format!("{scheme}://{creds}{host_port}/{database}")
}

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };

    let db = format!("_migrate_test_{}", std::process::id());
    let admin = connect(&base_url);
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .expect("create scratch database");

    let scratch_url = with_database(&base_url, &db);

    // First apply: the "fresh volume" case.
    migrate::run_migrations(&scratch_url)
        .await
        .expect("first migration pass");

    let scratch = connect(&scratch_url);
    let tables: Vec<String> = scratch
        .query("SELECT name FROM system.tables WHERE database = ? ORDER BY name")
        .bind(&db)
        .fetch_all()
        .await
        .expect("list tables");

    for expected in [
        "market_trades",
        "market_bars",
        "market_bars_v2",
        "backtest_run_equity",
        "backtest_run_trades",
    ] {
        assert!(
            tables.iter().any(|t| t == expected),
            "{expected} missing after migration; got {tables:?}"
        );
    }

    // Second apply: the case that actually matters — an existing database, where
    // the initdb mount would never run. Must be a silent no-op, not an error.
    migrate::run_migrations(&scratch_url)
        .await
        .expect("second migration pass must be idempotent");

    let tables_after: Vec<String> = scratch
        .query("SELECT name FROM system.tables WHERE database = ? ORDER BY name")
        .bind(&db)
        .fetch_all()
        .await
        .expect("list tables again");
    assert_eq!(tables, tables_after, "replay changed the schema");

    // The v2 table must be append-only and keyed by timeframe. Read it back from
    // the server rather than from our own DDL string, so this asserts what
    // ClickHouse actually built.
    let create: Vec<String> = scratch
        .query("SELECT create_table_query FROM system.tables WHERE database = ? AND name = 'market_bars_v2'")
        .bind(&db)
        .fetch_all()
        .await
        .expect("fetch create_table_query");
    let ddl = create.first().expect("market_bars_v2 exists");
    // v1's bug was an incomplete sorting key, not the engine itself: it merged rows
    // that differed in `timeframe`, because `timeframe` was not in the key. v2 keeps
    // a collapsing engine — re-collection is routine and an append-only table grows
    // without bound — but its key names every dimension that makes two bars
    // different observations, so only genuine repeats can merge.
    assert!(
        ddl.contains("ENGINE = ReplacingMergeTree(ingested_time)"),
        "v2 collapses only exact repeats, most recent ingest winning: {ddl}"
    );
    let order_by = &ddl[ddl.find("ORDER BY").expect("a sorting key")..];
    for dimension in [
        "instrument_id",
        "timeframe",
        "venue_id",
        "source",
        "event_time",
        "revision",
    ] {
        assert!(
            order_by.contains(dimension),
            "{dimension} missing from the sorting key — bars differing only in it              would be destroyed on merge, which is exactly the v1 bug: {ddl}"
        );
    }

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch database");
}

/// `connect` must honour the credentials and database in the URL. If it does not,
/// migrations silently land in `default` — the schema looks applied while every
/// read goes somewhere else.
#[tokio::test]
async fn connect_honours_database_in_url() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };

    let db = format!("_migrate_conn_{}", std::process::id());
    let admin = connect(&base_url);
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .expect("create scratch database");

    let scoped = connect(&with_database(&base_url, &db));
    scoped
        .query("CREATE TABLE IF NOT EXISTS probe (x Int64) ENGINE = MergeTree ORDER BY x")
        .execute()
        .await
        .expect("create probe table");

    let found: Vec<String> = admin
        .query("SELECT database FROM system.tables WHERE name = 'probe' AND database = ?")
        .bind(&db)
        .fetch_all()
        .await
        .expect("locate probe table");

    assert_eq!(
        found,
        vec![db.clone()],
        "table landed outside the URL's database"
    );

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch database");
}

/// The backfill must move every row, derive the two new time columns correctly, and
/// above all **preserve the colliding bars that v1 destroys**. This seeds the exact
/// pair measured on the live table on 2026-09-11: a 1m and a 1h BTC-USD bar sharing
/// a close of 2026-05-31 06:00.
#[tokio::test]
async fn backfill_preserves_colliding_bars_and_derives_times() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };

    let db = format!("_backfill_test_{}", std::process::id());
    let admin = connect(&base_url);
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .expect("create scratch database");
    let scratch_url = with_database(&base_url, &db);
    migrate::run_migrations(&scratch_url)
        .await
        .expect("migrations");
    let scratch = connect(&scratch_url);

    // Two bars that v1 treats as one: same instrument, same close, different
    // timeframe, both revision 0. Inserted in ONE batch.
    scratch
        .query(
            "INSERT INTO market_bars VALUES (generateUUIDv4(),'market.bars.1h','BTC-USD','coinbase','coinbase_rest','t','2026-05-31 06:00:00','2026-05-31 06:00:01',1,'1h',74054.0,74100.72,73939.99,73947.01,75.0697,0,0,'k1'), (generateUUIDv4(),'market.bars.1m','BTC-USD','coinbase','coinbase_rest','t','2026-05-31 06:00:00','2026-05-31 06:00:01',2,'1m',73961.99,73961.99,73947.01,73947.01,1.4108,0,0,'k2')",
        )
        .execute()
        .await
        .expect("seed v1");

    // v1 has already eaten one of them — both rows arrived in a single insert block,
    // so ReplacingMergeTree collapsed them as the part was written. This is the bug,
    // asserted rather than described.
    let v1_rows: u64 = scratch
        .query("SELECT count() FROM market_bars")
        .fetch_one()
        .await
        .expect("count v1");
    assert_eq!(
        v1_rows, 1,
        "v1 is expected to destroy one of the two bars on insert; if this ever reports 2, the v1 schema changed and this test's premise is stale"
    );

    // So seed the backfill source from rows that survive: insert the pair again in
    // two separate batches, which v1 keeps until a merge.
    scratch
        .query("TRUNCATE TABLE market_bars")
        .execute()
        .await
        .expect("truncate");
    for (tf, seq, o, h, l, c, v) in [
        (
            "1h", 1u64, "74054.0", "74100.72", "73939.99", "73947.01", "75.0697",
        ),
        (
            "1m", 2u64, "73961.99", "73961.99", "73947.01", "73947.01", "1.4108",
        ),
    ] {
        scratch
            .query(&format!(
                "INSERT INTO market_bars VALUES (generateUUIDv4(),'market.bars.{tf}','BTC-USD','coinbase','coinbase_rest','t','2026-05-31 06:00:00','2026-05-31 06:00:01',{seq},'{tf}',{o},{h},{l},{c},{v},0,0,'k{seq}')"
            ))
            .execute()
            .await
            .expect("seed v1 row");
    }

    let report = backfill::backfill_market_bars_v2(&scratch_url, "market_bars")
        .await
        .expect("backfill");
    assert_eq!(
        report.rows_copied, 2,
        "both bars must be copied: {report:?}"
    );

    // Both survive in v2, and stay distinct through a forced merge — the property
    // v1 cannot provide.
    scratch
        .query("OPTIMIZE TABLE market_bars_v2 FINAL")
        .execute()
        .await
        .expect("optimize v2");

    let survivors: Vec<String> = scratch
        .query("SELECT timeframe FROM market_bars_v2 ORDER BY timeframe")
        .fetch_all()
        .await
        .expect("read v2");
    assert_eq!(
        survivors,
        vec!["1h".to_string(), "1m".to_string()],
        "v2 must keep both timeframes through a merge"
    );

    // The derived columns: event_time is the close, bar_open_time is close - period.
    let opens: Vec<(String, String, String)> = scratch
        .query(
            "SELECT timeframe, toString(bar_open_time), toString(event_time) FROM market_bars_v2 ORDER BY timeframe",
        )
        .fetch_all()
        .await
        .expect("read derived times");

    let hourly = &opens[0];
    assert_eq!(hourly.0, "1h");
    assert!(
        hourly.1.starts_with("2026-05-31 05:00:00"),
        "1h bar_open_time must be close minus 1h, got {}",
        hourly.1
    );
    assert!(
        hourly.2.starts_with("2026-05-31 06:00:00"),
        "event_time must be the close, got {}",
        hourly.2
    );

    let minute = &opens[1];
    assert_eq!(minute.0, "1m");
    assert!(
        minute.1.starts_with("2026-05-31 05:59:00"),
        "1m bar_open_time must be close minus 1m, got {}",
        minute.1
    );

    // Re-running is a no-op, not a duplication.
    let again = backfill::backfill_market_bars_v2(&scratch_url, "market_bars")
        .await
        .expect("second backfill");
    assert_eq!(again.rows_copied, 0, "re-run must copy nothing: {again:?}");
    assert!(!again.did_work());
    let total: u64 = scratch
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .expect("count v2");
    assert_eq!(total, 2, "re-running the backfill duplicated rows");

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch database");
}

/// A timeframe with no known duration must stop the backfill, not receive a guessed
/// `bar_open_time`.
#[tokio::test]
async fn backfill_refuses_unknown_timeframes() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };

    let db = format!("_backfill_unknown_{}", std::process::id());
    let admin = connect(&base_url);
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .expect("create scratch database");
    let scratch_url = with_database(&base_url, &db);
    migrate::run_migrations(&scratch_url)
        .await
        .expect("migrations");
    let scratch = connect(&scratch_url);

    scratch
        .query(
            "INSERT INTO market_bars VALUES (generateUUIDv4(),'l','BTC-USD','coinbase','coinbase_rest','t','2026-05-31 06:00:00','2026-05-31 06:00:01',1,'2m',1.0,1.0,1.0,1.0,1.0,0,0,'k1')",
        )
        .execute()
        .await
        .expect("seed odd timeframe");

    let err = backfill::backfill_market_bars_v2(&scratch_url, "market_bars")
        .await
        .expect_err("unknown timeframe must abort the backfill");
    assert!(
        format!("{err}").contains("2m"),
        "the error must name the offending timeframe, got: {err}"
    );

    let copied: u64 = scratch
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .expect("count v2");
    assert_eq!(copied, 0, "nothing may be written before validation passes");

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch database");
}

/// Sets up a scratch database with the schema applied, and returns its name plus URL.
async fn scratch_db(base_url: &str, prefix: &str) -> (String, String) {
    let db = format!("_{}_{}", prefix, std::process::id());
    let admin = connect(base_url);
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .expect("create scratch database");
    let url = with_database(base_url, &db);
    migrate::run_migrations(&url).await.expect("migrations");
    (db, url)
}

async fn seed_bar(client: &clickhouse::Client, tf: &str, seq: u64, close: &str) {
    client
        .query(&format!(
            "INSERT INTO market_bars VALUES (generateUUIDv4(),'market.bars.{tf}','BTC-USD','coinbase','coinbase_rest','t','{close}','2026-05-31 06:00:01',{seq},'{tf}',1.0,2.0,0.5,1.5,10.0,0,0,'k{seq}')"
        ))
        .execute()
        .await
        .expect("seed bar");
}

/// Verification must pass on a faithful copy, and the numbers it reports must be the
/// real ones.
#[tokio::test]
async fn verification_passes_on_a_faithful_copy() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };
    let (db, url) = scratch_db(&base_url, "verify_ok").await;
    let scratch = connect(&url);

    seed_bar(&scratch, "1h", 1, "2026-05-31 06:00:00").await;
    seed_bar(&scratch, "1m", 2, "2026-05-31 07:00:00").await;

    backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("backfill");

    let report = backfill::verify_market_bars_v2(&url, "market_bars")
        .await
        .expect("verification must pass");
    assert!(report.passed());
    assert_eq!(report.source_rows, 2);
    assert_eq!(report.v2_rows, 2);
    assert_eq!(report.missing_rows, 0);

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}

/// The test that gives the check its value: a row missing from v2 must fail
/// verification. A verifier that has never rejected anything is decoration.
#[tokio::test]
async fn verification_fails_when_a_row_is_missing_from_v2() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };
    let (db, url) = scratch_db(&base_url, "verify_bad").await;
    let scratch = connect(&url);

    seed_bar(&scratch, "1h", 1, "2026-05-31 06:00:00").await;
    seed_bar(&scratch, "1m", 2, "2026-05-31 07:00:00").await;

    backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("backfill");
    backfill::verify_market_bars_v2(&url, "market_bars")
        .await
        .expect("baseline verification");

    // Remove one row from v2, simulating a copy that dropped data.
    scratch
        .query(
            "ALTER TABLE market_bars_v2 DELETE WHERE timeframe = '1h' SETTINGS mutations_sync = 2",
        )
        .execute()
        .await
        .expect("delete a row from v2");

    let err = backfill::verify_market_bars_v2(&url, "market_bars")
        .await
        .expect_err("a missing row must fail verification");
    let message = format!("{err}");
    assert!(
        message.contains("verification FAILED"),
        "unhelpful error: {message}"
    );
    assert!(
        message.contains("do not start the bar backfill"),
        "the error must say what not to do next: {message}"
    );

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}

/// v1 destroys rows on merge — that is the bug v2 exists to fix — so over time v2
/// legitimately holds *more* rows than the source. Verification checks containment,
/// so this must still pass. An equality check would start failing here on a
/// perfectly healthy system.
#[tokio::test]
async fn verification_tolerates_v2_being_a_superset() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };
    let (db, url) = scratch_db(&base_url, "verify_superset").await;
    let scratch = connect(&url);

    seed_bar(&scratch, "1h", 1, "2026-05-31 06:00:00").await;
    seed_bar(&scratch, "1m", 2, "2026-05-31 07:00:00").await;
    backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("backfill");

    // The source loses a row, exactly as a ReplacingMergeTree merge would do.
    scratch
        .query("ALTER TABLE market_bars DELETE WHERE timeframe = '1h' SETTINGS mutations_sync = 2")
        .execute()
        .await
        .expect("shrink the source");

    let report = backfill::verify_market_bars_v2(&url, "market_bars")
        .await
        .expect("v2 being a superset of v1 is correct, not a failure");
    assert!(report.passed());
    assert_eq!(report.source_rows, 1);
    assert_eq!(report.v2_rows, 2, "v2 keeps the bar that v1 lost");
    assert_eq!(report.missing_rows, 0);

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}

/// The backfill must stay a no-op once v2 has moved ahead of v1 (Set L, L-0.3).
///
/// This is a regression test for a failure that took the platform down on boot. The
/// first version skipped a chunk only when the two row counts were equal and aborted
/// otherwise — but after the cutover, live collection writes only to v2, so v2
/// legitimately holds more rows than v1 for the current month. The backfill runs at
/// startup, so it read that healthy state as corruption and refused to boot.
#[tokio::test]
async fn backfill_is_a_noop_once_v2_has_moved_ahead_of_v1() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };
    let (db, url) = scratch_db(&base_url, "backfill_ahead").await;
    let scratch = connect(&url);

    seed_bar(&scratch, "1m", 1, "2026-05-31 06:00:00").await;
    seed_bar(&scratch, "1m", 2, "2026-05-31 06:01:00").await;

    backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("initial backfill");

    // Live collection continues, writing only to v2 — the steady state after L-0.7.
    scratch
        .query(
            "INSERT INTO market_bars_v2 VALUES (generateUUIDv4(),'market.bars.1m','BTC-USD','coinbase','coinbase_rest','t','2026-05-31 06:01:00','2026-05-31 06:02:00','2026-05-31 06:02:00','2026-05-31 06:02:01',3,'1m',1.0,2.0,0.5,1.5,10.0,0,0,'k3')",
        )
        .execute()
        .await
        .expect("live write to v2");

    let report = backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("a v2 ahead of v1 is the normal state after cutover, not an error");
    assert_eq!(report.rows_copied, 0, "nothing was missing: {report:?}");

    let total: u64 = scratch
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(total, 3, "the backfill must not have duplicated anything");

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}

/// A half-finished chunk must resume, not duplicate.
#[tokio::test]
async fn backfill_resumes_a_partial_chunk_without_duplicating() {
    let Some(base_url) = url_from_env() else {
        return;
    };
    let (db, url) = scratch_db(&base_url, "backfill_partial").await;
    let scratch = connect(&url);

    for seq in 1..=3u64 {
        seed_bar(&scratch, "1m", seq, &format!("2026-05-31 06:0{seq}:00")).await;
    }

    // Copy only part of the chunk, as an interrupted run would leave it.
    scratch
        .query(
            "INSERT INTO market_bars_v2 (bar_open_time, event_time, available_time, event_id, lane, instrument_id, venue_id, source, trust_tier, ingested_time, sequence, timeframe, open, high, low, close, volume, trade_count, revision, dedup_key) \
             SELECT available_time - toIntervalSecond(60), available_time, available_time, event_id, lane, instrument_id, venue_id, source, trust_tier, ingested_time, sequence, timeframe, open, high, low, close, volume, trade_count, revision, dedup_key \
             FROM market_bars WHERE sequence = 1",
        )
        .execute()
        .await
        .expect("partial copy");

    let report = backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("a partial chunk must resume");
    assert_eq!(report.rows_copied, 2, "only the missing rows: {report:?}");

    let total: u64 = scratch
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(
        total, 3,
        "resuming must not duplicate the row already copied"
    );

    backfill::verify_market_bars_v2(&url, "market_bars")
        .await
        .expect("verification after resume");

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}

/// Re-collecting a bar must not make the backfill copy it forever (Set L, L-0.3).
///
/// Regression test for a loop that ran on every platform boot. The backfill
/// fingerprinted every shared column, `ingested_time` included, so a bar that had
/// been collected twice looked like two different rows. v2's engine keeps only the
/// most recently ingested copy, so after a merge the older row's exact content was
/// gone, the anti-join reported it missing, and the backfill re-inserted it — which
/// the next merge collapsed again, and so on.
///
/// The fix is to compare on what makes two rows the *same observation* — v2's
/// sorting key — rather than on their full content.
#[tokio::test]
async fn a_recollected_bar_is_not_copied_on_every_run() {
    let Some(base_url) = url_from_env() else {
        eprintln!("STORAGE_MIGRATE_CLICKHOUSE_URL unset — skipping");
        return;
    };
    let (db, url) = scratch_db(&base_url, "backfill_loop").await;
    let scratch = connect(&url);

    // The same bar, collected twice: identical in every dimension that identifies an
    // observation, differing only in when it was ingested.
    for ingested in ["2026-05-31 06:00:01", "2026-05-31 09:30:00"] {
        scratch
            .query(&format!(
                "INSERT INTO market_bars VALUES (generateUUIDv4(),'market.bars.1m','BTC-USD','coinbase','coinbase_rest','t','2026-05-31 06:00:00','{ingested}',1,'1m',1.0,2.0,0.5,1.5,10.0,0,0,'k1')"
            ))
            .execute()
            .await
            .expect("seed v1");
    }

    let first = backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("first backfill");
    assert!(first.rows_copied >= 1);

    // Merge, as ClickHouse does on its own schedule. The older ingest disappears.
    scratch
        .query("OPTIMIZE TABLE market_bars_v2 FINAL")
        .execute()
        .await
        .expect("merge");

    let second = backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("second backfill");
    assert_eq!(
        second.rows_copied, 0,
        "a merged-away duplicate ingest must not read as a missing row: {second:?}"
    );

    // And again, to be sure it is stable rather than merely alternating.
    scratch
        .query("OPTIMIZE TABLE market_bars_v2 FINAL")
        .execute()
        .await
        .expect("merge again");
    let third = backfill::backfill_market_bars_v2(&url, "market_bars")
        .await
        .expect("third backfill");
    assert_eq!(third.rows_copied, 0, "still stable: {third:?}");

    let rows: u64 = scratch
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(
        rows, 1,
        "one observation is one row, however often it was collected"
    );

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}

/// The engine must collapse exact repeats while keeping everything that differs.
#[tokio::test]
async fn v2_collapses_repeats_but_keeps_every_real_difference() {
    let Some(base_url) = url_from_env() else {
        return;
    };
    let (db, url) = scratch_db(&base_url, "v2_engine").await;
    let scratch = connect(&url);

    // Five rows: one bar collected twice, plus variants differing in exactly one
    // identity dimension each. Four distinct observations must survive.
    let rows = [
        // (timeframe, venue, source, close, revision, ingested)
        (
            "1m",
            "coinbase",
            "rest",
            "2026-05-31 06:00:00",
            0,
            "06:00:01",
        ),
        (
            "1m",
            "coinbase",
            "rest",
            "2026-05-31 06:00:00",
            0,
            "09:30:00",
        ), // repeat
        (
            "1h",
            "coinbase",
            "rest",
            "2026-05-31 06:00:00",
            0,
            "06:00:01",
        ), // timeframe
        ("1m", "kraken", "rest", "2026-05-31 06:00:00", 0, "06:00:01"), // venue
        (
            "1m",
            "coinbase",
            "rest",
            "2026-05-31 06:00:00",
            1,
            "06:00:01",
        ), // revision
    ];
    for (tf, venue, src, close, revision, ingest) in rows {
        scratch
            .query(&format!(
                "INSERT INTO market_bars_v2 VALUES (generateUUIDv4(),'market.bars.{tf}','BTC-USD','{venue}','{src}','t','{close}' - INTERVAL 1 MINUTE,'{close}','{close}','2026-05-31 {ingest}',1,'{tf}',1.0,2.0,0.5,1.5,10.0,0,{revision},'k')"
            ))
            .execute()
            .await
            .expect("seed v2");
    }

    scratch
        .query("OPTIMIZE TABLE market_bars_v2 FINAL")
        .execute()
        .await
        .expect("merge");

    let surviving: u64 = scratch
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(
        surviving, 4,
        "the repeat collapses; different timeframe, venue and revision all survive"
    );

    // Specifically: the 1h bar sharing a close with the 1m bar is still there. That
    // is the original bug, still fixed under a collapsing engine.
    let timeframes: Vec<String> = scratch
        .query("SELECT DISTINCT timeframe FROM market_bars_v2 ORDER BY timeframe")
        .fetch_all()
        .await
        .expect("timeframes");
    assert_eq!(timeframes, vec!["1h".to_string(), "1m".to_string()]);

    connect(&base_url)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop");
}
