//! Nightly feature consistency diff (SPEC §3.3, INV-14).
//!
//! Once a day, every tenant's serves from the past week that are at least six hours
//! old and not yet diffed are recomputed and diffed. The six-hour floor gives late
//! and restated bars time to land, which is exactly what `late_arrival` measures.
//! A serve is diffed once; the pass is idempotent.

use std::time::Duration;

use tracing::{error, info, warn};

pub fn spawn(pg: sqlx::PgPool, clickhouse_url: String) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            run_once(&pg, &clickhouse_url).await;
        }
    });
}

pub async fn run_once(pg: &sqlx::PgPool, clickhouse_url: &str) {
    let now = chrono::Utc::now();
    let (since, until) = (now - chrono::Duration::days(7), now - chrono::Duration::hours(6));
    let tenants = match api::feature_consistency::serving_tenants(pg).await {
        Ok(t) => t,
        Err(e) => return warn!(error = %e, "feature consistency: could not list tenants"),
    };
    for tenant in tenants {
        match api::feature_consistency::diff_tenant(pg, clickhouse_url, &tenant, since, until).await {
            Ok(r) if r.code_drift > 0 => error!(report = ?r, "feature consistency: code drift detected"),
            Ok(r) if r.p99_relative_diff >= 1e-9 => warn!(report = ?r, "feature consistency: p99 relative diff breaches the 1e-9 SLO"),
            Ok(r) => info!(report = ?r, "feature consistency diff complete"),
            Err(e) => warn!(%tenant, error = %e, "feature consistency diff failed"),
        }
    }
}
