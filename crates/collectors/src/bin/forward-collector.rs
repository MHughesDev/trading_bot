//! Run the forward derivatives collector as its own process (Set P P-0.3).
//!
//! The platform spawns the same collector through
//! `collectors::derivs::runner::spawn`; this binary exists so the clock can run —
//! and be proven to run — without starting the platform, which also starts every
//! live venue collector.
//!
//! ```text
//! DATABASE_URL=postgres://…@127.0.0.1:5432/trading \
//! CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//! FORWARD_POLL_SECS=900 FORWARD_RUN_SECS=3900 \
//!   cargo run -p collectors --bin forward-collector
//! ```
//!
//! `FORWARD_RUN_SECS` bounds the run (unset: until Ctrl-C). `FORWARD_POLL_SECS`
//! may be faster than the declared hourly cadence and never slower.

use std::time::Duration;

use collectors::derivs::runner;
use collectors::derivs::SourceIds;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pg_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let ch_url = std::env::var("CLICKHOUSE_URL").map_err(|_| "CLICKHOUSE_URL is required")?;
    let poll: u64 = std::env::var("FORWARD_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(runner::DEFAULT_POLL_SECS);
    let run_for: Option<u64> = std::env::var("FORWARD_RUN_SECS")
        .ok()
        .and_then(|s| s.parse().ok());

    // The catalogue comes from Postgres, where the ids are defined.
    let pg = storage::postgres::connect(&pg_url).await?;
    let ids = storage::clickhouse::forward::source_ids(&pg).await?;
    pg.close().await;

    let shutdown = async move {
        match run_for {
            Some(s) => tokio::time::sleep(Duration::from_secs(s)).await,
            None => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    };
    let report = runner::run(
        &ch_url,
        SourceIds::new(ids),
        Duration::from_secs(poll),
        shutdown,
    )
    .await?;
    println!(
        "polls={} funding_rows={} open_interest_rows={} rate_limited={} failures={}",
        report.polls,
        report.funding_rows,
        report.open_interest_rows,
        report.rate_limited,
        report.failures
    );
    Ok(())
}
