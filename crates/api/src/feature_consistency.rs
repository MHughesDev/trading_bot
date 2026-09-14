//! The nightly backfill-and-diff job (SPEC §3.3, INV-14).
//!
//! Every logged serve not yet diffed is recomputed from the bars as they stand now,
//! through the single feature runtime, and each feature's diff is written with both
//! knowledge times and a diagnosis. `code_drift` is a P1 and is logged at error.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Serves diffed per pass, per tenant, so one pass stays bounded.
const MAX_SERVES_PER_PASS: i64 = 5_000;

#[derive(Debug, Default, Serialize)]
pub struct DiffReport {
    pub tenant: String,
    pub serves: usize,
    pub diffs: usize,
    /// Serves or features whose window no longer exists in the data.
    pub unrecomputable: usize,
    pub code_drift: usize,
    /// p99 absolute relative diff (SLO < 1e-9 for deterministic features).
    pub p99_relative_diff: f64,
}

/// Tenants with any serve on record.
///
/// # Errors
/// Backend failures.
pub async fn serving_tenants(pg: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT t FROM dataplane.serving_tenants() AS t ORDER BY t").fetch_all(pg).await?)
}

/// Diff one tenant's undiffed serves with `served_at` in `[since, until)`.
///
/// # Errors
/// Backend failures. A serve that cannot be recomputed is counted, not an error.
pub async fn diff_tenant(pg: &PgPool, clickhouse_url: &str, tenant: &str, since: DateTime<Utc>, until: DateTime<Utc>) -> anyhow::Result<DiffReport> {
    let mut report = DiffReport { tenant: tenant.to_string(), ..DiffReport::default() };
    let mut tx = ledger::pg::tenant_tx(pg, tenant).await?;
    let serves = sqlx::query(
        "SELECT l.serve_id, l.instrument_id, l.venue_id, l.period_secs, l.event_time, l.knowledge_time, l.feature_set_id,
                l.code_hashes, l.feature_values, l.served_at
           FROM dataplane.feature_serving_log l
          WHERE l.tenant_id = $1 AND l.served_at >= $2 AND l.served_at < $3
            AND NOT EXISTS (SELECT 1 FROM dataplane.feature_consistency_diff d WHERE d.serve_id = l.serve_id)
          ORDER BY l.served_at
          LIMIT $4",
    )
    .bind(tenant)
    .bind(since)
    .bind(until)
    .bind(MAX_SERVES_PER_PASS)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let store = backtest::BarStore::connect(clickhouse_url);
    let mut all = Vec::new();
    for s in serves {
        report.serves += 1;
        let record = features::ServeRecord {
            serve_id: s.get::<Uuid, _>("serve_id"),
            tenant: tenant.to_string(),
            instrument_id: s.get("instrument_id"),
            event_time: s.get("event_time"),
            knowledge_time: s.get("knowledge_time"),
            feature_set_id: s.get("feature_set_id"),
            code_hashes: serde_json::from_value::<BTreeMap<String, String>>(s.get("code_hashes"))?,
            values: serde_json::from_value::<BTreeMap<String, f64>>(s.get("feature_values"))?,
            served_at: s.get("served_at"),
        };
        let venue_id: i32 = s.get("venue_id");
        let period_secs = u32::try_from(s.get::<i32, _>("period_secs")).unwrap_or(60);
        let lookback = record.values.keys().filter_map(|k| features::runtime::lookback_bars(k)).max().unwrap_or(1);
        let period_ns = i64::from(period_secs) * 1_000_000_000;
        let event_open_ns = record.event_time.timestamp_nanos_opt().unwrap_or(0);
        let to_close = event_open_ns + period_ns + 1;
        let from_close = event_open_ns + period_ns - (i64::from(lookback) + 2) * period_ns;
        let bars = store.load_bars_by_key(record.instrument_id, venue_id, period_secs, from_close, to_close).await?;
        let Some(served_bar) = bars.iter().find(|b| b.open_ns == event_open_ns) else {
            report.unrecomputable += record.values.len();
            continue;
        };
        // Recompute through the same densification the serve used, or the diff
        // reports code drift every time a gap moves (SPEC 2, INV-14).
        let clock = features::densify_bars(&bars.iter().map(backtest::warmup::bar_obs).collect::<Vec<_>>(), period_ns);
        let Some(decision) = clock.decision_at(served_bar.knowledge_ns) else {
            report.unrecomputable += record.values.len();
            continue;
        };
        let knowledge = clock.knowledge_ns();
        let d = features::consistency::diff_serve(&record, &clock.rows, &knowledge, decision);
        report.unrecomputable += d.unrecomputable.len();
        if d.diffs.is_empty() {
            continue;
        }

        let mut tx = ledger::pg::tenant_tx(pg, tenant).await?;
        for diff in &d.diffs {
            let diagnosis = serde_json::to_value(diff.diagnosis)?;
            if diff.diagnosis == features::Diagnosis::CodeDrift {
                report.code_drift += 1;
                tracing::error!(%tenant, serve_id = %diff.serve_id, feature = %diff.feature_id, served = diff.served_value,
                    recomputed = diff.recomputed_value, "P1: feature code drift between serve and backfill");
            }
            sqlx::query(
                "INSERT INTO dataplane.feature_consistency_diff
                     (serve_id, feature_id, tenant_id, served_value, recomputed_value, abs_diff,
                      served_knowledge_time, recomputed_knowledge_time, diagnosis)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(diff.serve_id)
            .bind(&diff.feature_id)
            .bind(tenant)
            .bind(diff.served_value)
            .bind(diff.recomputed_value)
            .bind(diff.abs_diff)
            .bind(diff.served_knowledge_time)
            .bind(diff.recomputed_knowledge_time)
            .bind(diagnosis.as_str().unwrap_or("code_drift"))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        report.diffs += d.diffs.len();
        all.extend(d.diffs);
    }
    report.p99_relative_diff = features::p99_relative_diff(&all);
    Ok(report)
}
