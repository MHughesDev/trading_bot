//! Serve a model's feature vector from recent bars (SPEC §3.3, INV-14).
//!
//! Values come from the single feature runtime — the same windowed implementations
//! the dataset materializer and the backtest use — and every serve is written to
//! `dataplane.feature_serving_log` before the vector is returned. A serve whose log
//! row cannot be written is a failed serve, not an unlogged one.

use std::sync::Arc;

use backtest::store::LoadedBar;
use serde_json::{json, Map, Value};
use sqlx::PgPool;

/// Who and what a serve is for.
pub struct ServeContext<'a> {
    pub pg: &'a PgPool,
    pub tenant: &'a str,
    /// Surrogate instrument id of the bars.
    pub instrument_id: i64,
    /// Venue the bars were read from.
    pub venue_id: i32,
    /// Bar period of the rows read.
    pub period_secs: u32,
    pub feature_set_id: &'a str,
}

/// The latest value of each named feature over `bars` (ascending). Features whose
/// declared window is not yet complete, or whose value is not finite, are `null` —
/// never a fabricated zero. Everything served is logged first.
///
/// The bars are densified onto the UTC master clock by the one densifier
/// (`features::align`) before anything is computed, and every value is returned
/// with its `_age_minutes` and `_quality` companions (SPEC §2, INV-11) — the
/// same contract, and the same code, the dataset builder uses.
///
/// # Errors
/// An unknown feature name, or a failure writing the serving log.
pub async fn serve_vector(ctx: &ServeContext<'_>, bars: &[LoadedBar], names: &[String]) -> anyhow::Result<Map<String, Value>> {
    let step_ns = i64::from(ctx.period_secs) * 1_000_000_000;
    let clock = features::densify_bars(&bars.iter().map(backtest::warmup::bar_obs).collect::<Vec<_>>(), step_ns);
    let rows = clock.rows.clone();
    let mut out: Map<String, Value> = Map::new();
    for n in names {
        let [value, age, quality] = features::align::companion_columns(n);
        out.insert(value, Value::Null);
        out.insert(age, Value::Null);
        out.insert(quality, Value::Null);
    }
    let Some(last) = bars.last() else { return Ok(out) };
    if rows.is_empty() {
        return Ok(out);
    }
    let decision = rows.len() - 1;

    let mut available: Vec<Arc<dyn features::Feature>> = Vec::new();
    for name in names {
        let f = features::runtime::feature(name)?;
        // The companions describe the inputs, so they are reported whether or not
        // the value itself is available: "no value yet, from data 40 minutes old"
        // is a different fact from "no value yet".
        let (age, q) = clock.provenance_for(f.as_ref(), decision);
        let [_, age_col, quality_col] = features::align::companion_columns(name);
        out.insert(age_col, json!(age));
        out.insert(quality_col, json!(q.0));
        if features::runtime::value_at(f.as_ref(), &rows, decision).is_some() {
            available.push(f);
        }
    }
    if available.is_empty() {
        return Ok(out);
    }

    let log = Arc::new(features::MemoryServeLog::default());
    let runtime = features::FeatureRuntime::new(ctx.feature_set_id, available, log.clone())?;
    let event_time = chrono::DateTime::from_timestamp_nanos(last.open_ns);
    // The serve can only know what every row it read was knowable by.
    let knowledge_time = chrono::DateTime::from_timestamp_nanos(bars.iter().map(|b| b.knowledge_ns).max().unwrap_or(last.knowledge_ns));
    let values = runtime.serve_live(ctx.tenant, ctx.instrument_id, &rows, event_time, knowledge_time)?;

    let records = std::mem::take(&mut *log.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    let mut tx = ledger::pg::tenant_tx(ctx.pg, ctx.tenant).await?;
    // Every served feature is registered (§3.2): its definition row exists before
    // any value it produced is logged.
    for def in runtime.defs() {
        sqlx::query(
            "INSERT INTO dataplane.feature_def
                 (feature_id, version, code_hash, lookback_bars, knowledge_lag_ms, output_dtype, asset_classes, deflators, info_class)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (feature_id, version, code_hash) DO NOTHING",
        )
        .bind(&def.feature_id)
        .bind(i32::try_from(def.version).unwrap_or(i32::MAX))
        .bind(&def.code_hash)
        .bind(i32::try_from(def.lookback_bars).unwrap_or(i32::MAX))
        .bind(i64::try_from(def.knowledge_lag_ms).unwrap_or(i64::MAX))
        .bind(&def.output_dtype)
        .bind(&def.asset_classes)
        .bind(&def.deflators)
        .bind(serde_json::to_value(def.info_class)?.as_str().unwrap_or("market_public"))
        .execute(&mut *tx)
        .await?;
    }
    for r in &records {
        sqlx::query(
            "INSERT INTO dataplane.feature_serving_log
                 (serve_id, tenant_id, instrument_id, venue_id, period_secs, event_time, knowledge_time, feature_set_id,
                  code_hashes, feature_values, served_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        )
        .bind(r.serve_id)
        .bind(&r.tenant)
        .bind(r.instrument_id)
        .bind(ctx.venue_id)
        .bind(i32::try_from(ctx.period_secs).unwrap_or(i32::MAX))
        .bind(r.event_time)
        .bind(r.knowledge_time)
        .bind(&r.feature_set_id)
        .bind(json!(r.code_hashes))
        .bind(json!(r.values))
        .bind(r.served_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    for (k, v) in values {
        out.insert(k, json!(v));
    }
    Ok(out)
}
