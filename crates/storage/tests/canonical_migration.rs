//! v2 → canonical migration against a copy of real bars. Ignored by default:
//!
//! ```bash
//! CANON_TEST_CH_URL=http://trading:trading@localhost:8123 \
//! CANON_TEST_PG_URL=postgres://trading:trading@localhost:5432/jobs_test \
//!   cargo test -j 2 -p storage --test canonical_migration -- --ignored --nocapture
//! ```
//! Copies `trading.market_bars_v2` into a throwaway ClickHouse database, migrates it,
//! and checks every guarantee the migration makes.

use clickhouse::Row;
use serde::Deserialize;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn with_db(base: &str, db: &str) -> String {
    let (scheme, rest) = base.split_once("://").unwrap();
    let host = rest.split('/').next().unwrap();
    format!("{scheme}://{host}/{db}")
}

#[derive(Row, Deserialize)]
struct N {
    n: u64,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live ClickHouse and a migrated Postgres"]
async fn migrates_v2_honestly() {
    let (Some(ch_base), Some(pg_url)) = (env("CANON_TEST_CH_URL"), env("CANON_TEST_PG_URL")) else { return };
    let admin = storage::clickhouse::connect(&ch_base);
    admin.query("DROP DATABASE IF EXISTS canon_mig").execute().await.unwrap();
    admin.query("CREATE DATABASE canon_mig").execute().await.unwrap();
    admin.query("CREATE TABLE canon_mig.market_bars_v2 AS trading.market_bars_v2").execute().await.unwrap();
    admin.query("INSERT INTO canon_mig.market_bars_v2 SELECT * EXCEPT snapshot_id FROM trading.market_bars_v2").execute().await.unwrap();
    let url = with_db(&ch_base, "canon_mig");
    storage::clickhouse::migrate::run_migrations(&url).await.unwrap();

    let pg = sqlx::PgPool::connect(&pg_url).await.unwrap();
    storage::postgres::run_migrations(&pg).await.unwrap();
    let report = storage::clickhouse::canonical::migrate_to_canonical(&url, &pg).await.expect("migration");
    println!("{report:?}");
    assert!(report.chunks_copied > 0);

    let ch = storage::clickhouse::connect(&url);
    let q = |sql: &'static str| {
        let ch = ch.clone();
        async move { ch.query(sql).fetch_one::<N>().await.unwrap().n }
    };

    // Every distinct v2 cell has a canonical cell.
    let v2 = q("SELECT uniqExact(instrument_id, venue_id, event_time, revision) AS n FROM market_bars_v2").await;
    let canon = q("SELECT uniqExact(instrument_id, venue_id, event_time, revision_seq) AS n FROM market_bar").await;
    assert_eq!(v2, canon, "cell counts must match");

    // Bar timestamps are the OPEN: aligned to their period.
    let misaligned = q("SELECT count() AS n FROM market_bar WHERE toUnixTimestamp64Nano(event_time) % (toInt64(period_secs) * 1000000000) != 0").await;
    assert_eq!(misaligned, 0);

    // Nothing is knowable before its bar closed.
    let early = q("SELECT count() AS n FROM market_bar WHERE knowledge_time < event_time + toIntervalSecond(period_secs)").await;
    assert_eq!(early, 0);

    // REST backfills are flagged; nothing that arrived late is marked observed.
    let late_unflagged = q(
        "SELECT count() AS n FROM market_bar WHERE ingest_time > event_time + toIntervalSecond(period_secs) + INTERVAL 5 MINUTE
            AND bitAnd(quality_flags, 32768) = 0",
    )
    .await;
    assert_eq!(late_unflagged, 0, "every late row carries BACKFILLED_KNOWLEDGE_TIME");
    let flagged = q("SELECT countIf(bitAnd(quality_flags, 32768) != 0) AS n FROM market_bar").await;
    assert!(flagged > 0, "the REST history is backfilled");

    // Live websocket bars kept their observed knowledge time.
    let live_observed = q(
        "SELECT count() AS n FROM market_bar WHERE source_id = 9 AND bitAnd(quality_flags, 32768) = 0",
    )
    .await;
    assert!(live_observed > 0);

    // Venues are not blended: BTC-USD exists on two venues as two instruments.
    let btc_venues = q(
        "SELECT uniqExact(venue_id) AS n FROM market_bar WHERE instrument_id IN
           (SELECT instrument_id FROM instrument_symbol_dim WHERE symbol = 'BTC-USD')",
    )
    .await;
    assert!(btc_venues >= 2);

    // Idempotent: a second run copies nothing.
    let again = storage::clickhouse::canonical::migrate_to_canonical(&url, &pg).await.unwrap();
    assert_eq!(again.chunks_copied, 0);
    assert_eq!(q("SELECT uniqExact(instrument_id, venue_id, event_time, revision_seq) AS n FROM market_bar").await, canon);

    admin.query("DROP DATABASE canon_mig").execute().await.unwrap();
}
