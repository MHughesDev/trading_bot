//! The hourly retention pass for the forward-only series (Set P P-0.6).
//!
//! `dataplane::retention::retention_action` decided what to do when a series
//! approached its disk budget and nothing ever asked it. `storage::clickhouse::
//! retention::retention_pass` measures every registered series, asks, and acts;
//! this is its caller.
//!
//! Outcomes are logged three ways, like the silence alarm: a series over budget with
//! no rule that helps is an **error** (a human must decide — the pass never deletes),
//! a downsample is info, and a series that could not be processed is a warning that
//! names it, so one bad registry row cannot hide the others.

use std::time::Duration;

use chrono::Utc;
use storage::clickhouse::retention::{retention_pass, SeriesOutcome};
use tracing::{error, info, warn};

pub fn spawn(pg: sqlx::PgPool, ch_url: String) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            run_once(&pg, &ch_url).await;
        }
    });
}

pub async fn run_once(pg: &sqlx::PgPool, ch_url: &str) {
    let results = match retention_pass(pg, ch_url, Utc::now()).await {
        Ok(r) => r,
        Err(e) => {
            return warn!(error = %e, "retention pass: could not read the forward-series registry")
        }
    };
    for r in results {
        match r {
            Ok(SeriesOutcome::NeedsDecision {
                series_id,
                used_bytes,
                budget_bytes,
            }) => error!(
                %series_id, used_bytes, budget_bytes,
                "P1: forward series is over its disk budget with no downsampling rule; nothing was deleted, a human must decide"
            ),
            Ok(SeriesOutcome::Downsampled {
                series_id, report, ..
            }) => {
                info!(%series_id, ?report, "forward series downsampled (older rows coarsened, none deleted)")
            }
            Ok(SeriesOutcome::Unsupported { series_id, table }) => {
                warn!(%series_id, %table, "retention rule names a table the pass cannot coarsen")
            }
            Ok(SeriesOutcome::Kept { .. } | SeriesOutcome::Disabled { .. }) => {}
            Err(e) => warn!(error = %e, "retention pass: one series failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn main_spawns_the_retention_pass() {
        let main = include_str!("main.rs");
        assert!(
            main.contains("retention_jobs::spawn("),
            "apps/platform/src/main.rs must call retention_jobs::spawn"
        );
    }
}
