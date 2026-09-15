//! `tbot-monitor` — the independent live monitor (COMP-007 §1 P1, §6).
//!
//! Its own process, its own Postgres connection, its own heartbeat, and no
//! dependency on the API. Kill this and the platform stops trading within one
//! cadence; kill the API and this keeps recording. That is the whole design, and
//! it is only true because this is a separate binary.

use std::time::Duration;

use monitor::lease::LeaseTerms;
use monitor::service::MonitorService;
use monitor::store::MonitorStore;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set for tbot-monitor".to_owned())?;

    let cadence_ms: i64 = std::env::var("MONITOR_CADENCE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| i64::try_from(monitor::DEFAULT_CADENCE_MS).unwrap_or(60_000));

    // The holder id names *this* instance. A second monitor taking the lease is a
    // takeover rather than a duplicate, and the holder is how the loser finds out.
    let holder = std::env::var("MONITOR_INSTANCE_ID").unwrap_or_else(|_| {
        format!(
            "tbot-monitor-{}",
            std::process::id()
        )
    });

    // A small pool on purpose: the monitor's job is a handful of short statements
    // per cadence, and a monitor that can exhaust the database's connections is a
    // monitor that can cause the outage it is watching for.
    let pg = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await?;

    let store = MonitorStore::new(pg);
    let terms = LeaseTerms::new(holder, cadence_ms);

    // Fail fast on a database that is present but does not have the monitor
    // schema: a monitor that starts, cannot write, and logs about it every minute
    // looks alive in `docker ps` and is not doing its job.
    store
        .renew_lease(&terms, chrono::Utc::now())
        .await
        .map_err(|e| format!("could not take the monitor lease: {e}"))?;

    MonitorService::new(store, terms).run().await;
    Ok(())
}
