//! P-0.6 clause 2 against a real ClickHouse: **a table driven past its budget has
//! its older rows downsampled, not deleted.**
//!
//! `dataplane::retention::over_budget_never_deletes` proves the *decision*. It
//! cannot prove a table was coarsened, because the decision is a pure function and
//! nothing called it. This drives the whole path — measure the table's bytes,
//! declare a budget below them, ask `retention_action`, carry it out — and then
//! checks the table: every `(contract, day)` that held an observation still holds
//! exactly one, the one kept is the day's last, recent rows and other venues are
//! untouched.
//!
//! Gated on `STORAGE_RETENTION_CLICKHOUSE_URL` like `clickhouse_migrate.rs`, and
//! for the same reason: CI has no ClickHouse service. **A skipped run is not
//! evidence** — the phase file records the run that was made, and against which
//! server. It writes only to a throwaway database, which it drops.
//!
//! ```bash
//! STORAGE_RETENTION_CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//!   cargo test -j 2 -p storage --test clickhouse_retention -- --nocapture
//! ```

use chrono::{Duration, DurationRound, Utc};
use dataplane::retention::{ForwardSeriesSpec, RetentionPolicy};
use storage::clickhouse::{connect, migrate, retention};

fn with_database(url: &str, database: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let (creds, host_path) = match rest.rfind('@') {
        Some(at) => (format!("{}@", &rest[..at]), &rest[at + 1..]),
        None => (String::new(), rest),
    };
    let host = host_path.split('/').next().unwrap_or(host_path);
    format!("{scheme}://{creds}{host}/{database}")
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct N {
    n: u64,
}

async fn n(c: &clickhouse::Client, sql: &str) -> u64 {
    c.query(sql).fetch_one::<N>().await.expect(sql).n
}

#[tokio::test]
async fn a_chain_table_past_its_budget_is_downsampled_not_deleted() {
    let Ok(base) = std::env::var("STORAGE_RETENTION_CLICKHOUSE_URL") else {
        eprintln!("STORAGE_RETENTION_CLICKHOUSE_URL unset — skipping (not evidence)");
        return;
    };
    let db = format!("_retention_test_{}", std::process::id());
    let admin = connect(&base);
    admin
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    let url = with_database(&base, &db);
    migrate::run_migrations(&url).await.expect("schema");
    let c = connect(&url);

    // Two contracts, three days of hourly snapshots 100 days ago (in scope for a
    // 90-day rule), two days 10 days ago (out of scope), and one other venue.
    let now = Utc::now();
    let old_day0 = (now - Duration::days(100)).duration_trunc(Duration::days(1)).unwrap();
    let new_day0 = (now - Duration::days(10)).duration_trunc(Duration::days(1)).unwrap();
    let mut values = Vec::new();
    let mut push = |venue: &str, strike: u32, t: chrono::DateTime<Utc>, iv: f64| {
        values.push(format!(
            "('{venue}','SPY','SPY{strike}','2027-01-15',{strike},'C',\
             fromUnixTimestamp64Nano(toInt64({s})*1000000000),NULL,now64(9),\
             fromUnixTimestamp64Nano(toInt64({s}+900)*1000000000),\
             1,1.1,1.05,500,10,100,{iv},'bs',0.05,0.0,1.0,30,1,'hourly',0,19)",
            s = t.timestamp()
        ));
    };
    for strike in [500_u32, 510] {
        for d in 0..3 {
            for h in 0..24 {
                let t = old_day0 + Duration::days(d) + Duration::hours(h);
                push("tradier", strike, t, f64::from(h as u32) / 100.0);
            }
        }
        for d in 0..2 {
            for h in 0..24 {
                push("tradier", strike, new_day0 + Duration::days(d) + Duration::hours(h), 0.2);
            }
        }
    }
    for h in 0..24 {
        push("other_venue", 500, old_day0 + Duration::hours(h), 0.3);
    }
    c.query(&format!(
        "INSERT INTO option_chain_snapshots (venue, underlying, instrument, expiry, strike, option_right,
            event_time, venue_ts, ingest_time, knowledge_time, bid, ask, mark, underlying_price,
            volume, open_interest, iv, iv_model, iv_rate, iv_div, moneyness, dte,
            in_liquid_universe, snapshot_cadence, quality_flags, source_id) VALUES {}",
        values.join(",")
    ))
    .execute()
    .await
    .expect("seed");
    c.query("OPTIMIZE TABLE option_chain_snapshots FINAL")
        .execute()
        .await
        .unwrap();

    // Drive it past its budget: the budget is set below what is measured.
    let used = retention::venue_bytes(&url, "option_chain_snapshots", "tradier")
        .await
        .unwrap();
    assert!(used > 0, "the table must measure non-empty");
    let spec = ForwardSeriesSpec {
        series_id: "equity.option_chains".into(),
        store_table: "option_chain_snapshots".into(),
        venue: "tradier".into(),
        cadence_secs: 3600,
        alarm_after_cadences: 3,
        disk_budget_bytes: used / 2,
        retention: RetentionPolicy::Downsample {
            after_days: 90,
            to_secs: 86_400,
        },
        enabled: true,
    };
    let out = retention::apply(&url, &spec, used, now).await.expect("pass");
    let retention::SeriesOutcome::Downsampled { report, .. } = out else {
        panic!("over budget with a downsample rule must downsample: {out:?}");
    };

    let scope = format!(
        "venue = 'tradier' AND event_time < fromUnixTimestamp64Nano(toInt64({})*1000000000)",
        (now - Duration::days(90)).timestamp()
    );
    // Downsampled: 2 contracts × 3 days = 6 cells, each held by one coarse row.
    assert_eq!(report.fine_rows_before, 144);
    assert_eq!(report.buckets_before, 6);
    assert!(report.continuity_held(), "{report:?}");
    assert_eq!(report.fine_rows_after, 0);
    assert_eq!(n(&c, &format!("SELECT count() AS n FROM option_chain_snapshots FINAL WHERE {scope}")).await, 6);
    assert_eq!(
        n(&c, &format!("SELECT count() AS n FROM option_chain_snapshots FINAL WHERE {scope} AND snapshot_cadence = 'daily'")).await,
        6
    );
    // Not deleted: the kept row is each day's last real observation (hour 23).
    assert_eq!(
        n(&c, &format!("SELECT count() AS n FROM option_chain_snapshots FINAL WHERE {scope} AND toHour(event_time) = 23")).await,
        6
    );
    // Untouched: recent rows, and another venue's old rows.
    assert_eq!(
        n(&c, "SELECT count() AS n FROM option_chain_snapshots FINAL WHERE venue = 'tradier' AND snapshot_cadence = 'hourly'").await,
        96
    );
    assert_eq!(
        n(&c, "SELECT count() AS n FROM option_chain_snapshots FINAL WHERE venue = 'other_venue'").await,
        24
    );

    // Idempotent: a second pass over the coarsened table changes nothing.
    let again = retention::downsample_chain_snapshots(&url, "tradier", now - Duration::days(90), 86_400)
        .await
        .unwrap();
    assert_eq!(again.fine_rows_before, 0);
    assert_eq!(again.buckets_after, 6);

    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
}
