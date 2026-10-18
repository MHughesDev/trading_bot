//! The nightly coverage census (Set P P-0.1).
//!
//! `backtest::census::take_census` and `persist` had no caller anywhere, so
//! `dataplane.data_coverage_census` held 0 rows until an ops run wrote the first
//! nine. The census is a time series — watching depth accrue is the only way a dead
//! collector ever announces itself — so it has to run on its own, not when someone
//! remembers.
//!
//! The first tick fires at boot, then every 24 h. A failed pass is logged and the
//! next one is attempted; it never aborts the process, because a census that cannot
//! be taken is a monitoring gap, not a reason to stop trading.
//!
//! One calendar for every venue: continuous 24/7. For a session venue that
//! over-states the expected bar count and so under-states coverage, the safe
//! direction for a number whose job is to stop people believing the platform has
//! more history than it does (`take_census` documents the same fallback). Per-venue
//! session calendars belong with the first session venue's onboarding.

use std::time::Duration;

use chrono::Utc;
use dataplane::calendar::{Calendar, CalendarVersion};
use tracing::{info, warn};

pub fn spawn(pg: sqlx::PgPool, ch_url: String) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            match run_once(&pg, &ch_url).await {
                Ok((series, written)) => info!(series, written, "coverage census taken"),
                Err(e) => warn!(error = %e, "coverage census failed"),
            }
        }
    });
}

/// One pass: returns `(series censused, rows written)`.
///
/// After the census is persisted, every historical series is graded with the
/// `dataplane::qc` battery and the report persisted to `data_qc_reports` (P-2.2).
/// The census supplies `expected_bars`, so coverage is measured against the
/// calendar rather than against the bars' own ends. This is what keeps
/// `REFUSE_UNGRADED` (P-2.3) honest: a series with bars gets a report within a
/// day, so "ungraded" cannot mean "nobody has run the job yet". A series that
/// fails to grade is logged and the pass goes on.
///
/// # Errors
/// The reader or the database, for the census itself.
pub async fn run_once(pg: &sqlx::PgPool, ch_url: &str) -> anyhow::Result<(usize, u64)> {
    let store = backtest::store::BarStore::connect(ch_url);
    let records =
        backtest::census::take_census(&store, Utc::now(), |_venue| Calendar::Continuous {
            version: CalendarVersion::crypto_24_7(),
        })
        .await?;
    let written = backtest::census::persist(pg, &records).await?;
    let (graded, failed) = grade_series(pg, ch_url, &records).await;
    info!(graded, failed, "series graded with the data_qc battery");
    Ok((records.len(), written))
}

/// Grade every historical series in `records`. Returns `(graded, failed)`.
pub async fn grade_series(
    pg: &sqlx::PgPool,
    ch_url: &str,
    records: &[backtest::census::CensusRecord],
) -> (usize, usize) {
    let cfg = dataplane::qc::QcConfig::default();
    let (mut graded, mut failed) = (0usize, 0usize);
    for r in records {
        if r.kind != backtest::census::SeriesKind::Historical {
            continue;
        }
        let c = &r.census;
        let (Some(first), Some(last)) = (c.first_bar_ns, c.last_bar_ns) else {
            continue;
        };
        let from = chrono::DateTime::from_timestamp_nanos(first);
        let to = chrono::DateTime::from_timestamp_nanos(last)
            + chrono::Duration::seconds(i64::from(c.period_secs));
        match storage::clickhouse::qc_reports::grade_and_persist(
            pg,
            ch_url,
            &c.instrument,
            &c.venue,
            &c.timeframe,
            c.period_secs,
            from,
            to,
            c.expected_bars,
            &cfg,
        )
        .await
        {
            Ok((id, report)) => {
                graded += 1;
                info!(
                    instrument = %c.instrument, venue = %c.venue, timeframe = %c.timeframe,
                    grade = report.graded.grade.as_str(), report = %id, "series graded"
                );
            }
            Err(e) => {
                failed += 1;
                warn!(
                    instrument = %c.instrument, venue = %c.venue, timeframe = %c.timeframe,
                    error = %e, "series could not be graded"
                );
            }
        }
    }
    (graded, failed)
}

#[cfg(test)]
mod tests {
    /// One real pass against the stores in `DATABASE_URL` and `CLICKHOUSE_URL`
    /// (the operator's, or CI's): census, persist, grade, persist. Not run by
    /// default - it reads the whole bar table - but it is how a human takes the
    /// first census after a rule change without starting the platform:
    /// `cargo test -p platform census_jobs::tests::live_pass -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "reads and writes the operator's live stores"]
    async fn live_pass() {
        let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
            .await
            .expect("postgres");
        // The platform's own `cfg.clickhouse.url`: it names the database.
        let ch = std::env::var("CLICKHOUSE_URL")
            .unwrap_or_else(|_| "http://trading:trading@localhost:8123/trading".into());
        let (series, written) = super::run_once(&pg, &ch).await.expect("pass");
        println!("censused {series} series, wrote {written} census rows");
    }

    /// The module is only worth anything if `main` calls it. This is the guard the
    /// other job modules carry: a census job nobody spawns is the defect P-0.1 was
    /// downgraded for.
    #[test]
    fn main_spawns_the_census() {
        let main = include_str!("main.rs");
        assert!(
            main.contains("census_jobs::spawn("),
            "apps/platform/src/main.rs must call census_jobs::spawn"
        );
    }
}
