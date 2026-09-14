//! The nightly leakage pass (SPEC §12.5, checklist 1.7–1.11).
//!
//! §12.5 puts the random-label test "in platform CI, running nightly against the
//! platform itself, not just per strategy", and that is what this is: once a day
//! every tenant's recent datasets are rebuilt and compared against the digest
//! they first produced, and the harness itself is audited through the production
//! fold geometry.
//!
//! A blocking finding is a P1 and is logged at error. It is deliberately not a
//! process abort: the finding is already durable in `mlops.leakage_finding`, and
//! killing the trading platform because a nightly audit found a research-side
//! defect would trade a real risk for a theoretical one.

use std::time::Duration;

use tracing::{error, info, warn};

pub fn spawn(pg: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            run_once(&pg).await;
        }
    });
}

pub async fn run_once(pg: &sqlx::PgPool) {
    let datasets = model_registry::datasets::DatasetManager::new(pg.clone());
    let tenants = match api::leakage_audit::dataset_tenants(pg).await {
        Ok(t) => t,
        Err(e) => return warn!(error = %e, "leakage audit: could not list tenants"),
    };
    for tenant in tenants {
        match api::leakage_audit::audit_tenant(pg, &datasets, &tenant).await {
            Ok(r) if r.blocking > 0 => {
                error!(report = ?r, "P1: leakage suite found a blocking defect");
            }
            Ok(r) if r.flags > 0 => warn!(report = ?r, "leakage suite raised flags"),
            Ok(r) => info!(report = ?r, "leakage suite clean"),
            Err(e) => warn!(%tenant, error = %e, "leakage audit failed"),
        }
    }
}
