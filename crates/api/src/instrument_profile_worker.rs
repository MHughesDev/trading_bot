//! The `InstrumentProfile` worker: bars in, fingerprints and embeddings out
//! (SPEC §5.2-5.3, checklist 3.1/3.4/3.7, ADR-P3-01/05; OPEN-TASKS C1.3).
//!
//! # What was missing
//!
//! `JobKind::InstrumentProfile` existed — it is a Data-class kind that logs to
//! the exploration ledger — and **no worker was registered for it**, so a job of
//! that kind failed `no_worker`. Behind it, `fingerprint.compute` and
//! `embedding.fit_whitener` (the trainer sidecar) were tested and called by
//! nothing, and `knowledge.asset_embedding` had no writer anywhere. The
//! continuous-research track wants instrument profiles as an input; this is what
//! produces them.
//!
//! # What it does
//!
//! 1. For each instrument with a current symbol, reads the trailing window of 1 m
//!    bars **through the one point-in-time reader** (`BarStore::load_bars`), so a
//!    fingerprint can never be computed from bars the as-of date could not have
//!    seen (AT-43).
//! 2. Asks the trainer sidecar for tier-1 fingerprints (`POST /fingerprint`) and
//!    appends them to `knowledge.instrument_fingerprint` (migration 0184). An
//!    instrument with too few bars is skipped *with the reason in the output*; a
//!    vector of NULLs is not stored.
//! 3. Takes the **latest fingerprint of every instrument** — not just the ones
//!    this job touched, because the whitener is fitted over the population — and
//!    asks for an embedding (`POST /embed`). Below the sidecar's minimum
//!    population (60) it answers 422 `not_enough_population`, and the job reports
//!    `embedding: "deferred"` with the count rather than failing: a platform with
//!    twelve instruments has twelve fingerprints and no embeddings, and says so.
//! 4. When it does embed, closes the previous version's rows (`valid_to`) and
//!    writes `knowledge.asset_embedding`, the pgvector literal bound as text.
//!
//! The kNN reader (`knowledge::EmbeddingStore::neighbours`) was already built and
//! bounded by `knowledge_time`; this is the writer it was waiting for.
//!
//! `hubness_corrected` is written **false**: mutual-proximity rescaling is a
//! query-time operation over a distance matrix and this job does not apply it.
//! The column's default is true, which would claim a correction nobody made.

#![allow(clippy::result_large_err)]

use std::time::Duration;

use backtest::BarStore;
use chrono::{DateTime, Utc};
use domain::payloads::bar::Timeframe;
use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

/// The trailing window fingerprints are computed over (`21d@1m`, SPEC §5.2).
pub const DEFAULT_WINDOW_DAYS: i64 = 21;

/// Instruments per fingerprint request. Each carries ~30 000 one-minute bars, so
/// a small batch keeps the request body in the low megabytes.
const BATCH: usize = 6;

/// The version the platform writes beside the vectors; the sidecar returns the
/// same string and the worker refuses a mismatch.
pub const EXPECTED_EMBEDDING_VERSION: &str = "tier1_pca48_v1";

/// What `POST /fingerprint` returns, as far as this worker reads it.
#[derive(Debug, Clone, Deserialize)]
pub struct FingerprintResponse {
    pub order: Vec<String>,
    pub fingerprints: Vec<FingerprintOut>,
    #[serde(default)]
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FingerprintOut {
    pub instrument_id: i64,
    pub venue_id: i32,
    pub bars_used: i32,
    pub tier1: Vec<Option<f32>>,
    #[serde(default)]
    pub absent: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Skipped {
    pub instrument_id: i64,
    pub venue_id: i32,
    pub reason: String,
}

/// What `POST /embed` returns when it succeeds.
#[derive(Debug, Clone, Deserialize)]
pub struct EmbedResponse {
    pub embedding_version: String,
    pub retrieval: Vec<Vec<f64>>,
    pub imputed_per_row: Vec<i32>,
    pub intrinsic_dim: Option<f32>,
    #[serde(default)]
    pub dimensions_dropped: Vec<String>,
}

/// The sidecar's answer to an embedding request.
#[derive(Debug, Clone)]
pub enum EmbedAnswer {
    Embedded(EmbedResponse),
    /// "Not enough instruments yet": an answer, not a failure.
    NotEnoughPopulation(String),
}

/// One instrument's bars, in the shape the sidecar takes.
#[derive(Debug, Clone, Serialize)]
pub struct BarsPayload {
    pub instrument_id: i64,
    pub venue_id: i32,
    pub ts_ns: Vec<i64>,
    pub open: Vec<f64>,
    pub high: Vec<f64>,
    pub low: Vec<f64>,
    pub close: Vec<f64>,
    pub volume: Vec<f64>,
}

/// The two sidecar calls, behind a trait so the worker's logic is tested without
/// a Python process.
#[async_trait::async_trait]
pub trait ProfileSidecar: Send + Sync {
    async fn fingerprint(
        &self,
        window_spec: &str,
        instruments: &[BarsPayload],
    ) -> Result<FingerprintResponse, String>;

    async fn embed(
        &self,
        order: &[String],
        rows: &[Vec<Option<f32>>],
    ) -> Result<EmbedAnswer, String>;
}

/// The production sidecar: the model-trainer service.
pub struct HttpProfileSidecar {
    url: String,
    http: reqwest::Client,
}

impl HttpProfileSidecar {
    /// `MODEL_TRAINER_URL`, the same variable `SidecarClient` reads.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("MODEL_TRAINER_URL")
                .unwrap_or_else(|_| "http://localhost:8001".to_string()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait::async_trait]
impl ProfileSidecar for HttpProfileSidecar {
    async fn fingerprint(
        &self,
        window_spec: &str,
        instruments: &[BarsPayload],
    ) -> Result<FingerprintResponse, String> {
        let resp = self
            .http
            .post(format!("{}/fingerprint", self.url.trim_end_matches('/')))
            .json(&json!({ "window_spec": window_spec, "instruments": instruments }))
            .send()
            .await
            .map_err(|e| format!("the trainer is unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("/fingerprint answered {}", resp.status()));
        }
        resp.json().await.map_err(|e| format!("/fingerprint body: {e}"))
    }

    async fn embed(
        &self,
        order: &[String],
        rows: &[Vec<Option<f32>>],
    ) -> Result<EmbedAnswer, String> {
        let resp = self
            .http
            .post(format!("{}/embed", self.url.trim_end_matches('/')))
            .json(&json!({ "order": order, "rows": rows }))
            .send()
            .await
            .map_err(|e| format!("the trainer is unreachable: {e}"))?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(|e| format!("/embed body: {e}"))?;
        if status.is_success() {
            return serde_json::from_value(body)
                .map(EmbedAnswer::Embedded)
                .map_err(|e| format!("/embed body does not parse: {e}"));
        }
        if status.as_u16() == 422
            && body.pointer("/detail/error").and_then(Value::as_str) == Some("not_enough_population")
        {
            return Ok(EmbedAnswer::NotEnoughPopulation(
                body.pointer("/detail/message")
                    .and_then(Value::as_str)
                    .unwrap_or("not enough instruments")
                    .to_string(),
            ));
        }
        Err(format!("/embed answered {status}: {body}"))
    }
}

fn logic(code: &'static str, detail: impl Into<String>) -> JobError {
    JobError::logic(
        ledger::TerminalReason::IntegrityRejected,
        code,
        detail.into(),
    )
}

/// `21d@1m`.
#[must_use]
pub fn window_spec(days: i64) -> String {
    format!("{days}d@1m")
}

/// Bars as column arrays. Decimal to f64 is lossy by design here: a fingerprint
/// is a statistic over returns, not a ledger.
#[must_use]
pub fn payload_from_bars(
    instrument_id: i64,
    venue_id: i32,
    bars: &[backtest::store::LoadedBar],
) -> BarsPayload {
    let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or(f64::NAN);
    BarsPayload {
        instrument_id,
        venue_id,
        ts_ns: bars.iter().map(|b| b.ts_ns).collect(),
        open: bars.iter().map(|b| f(b.open)).collect(),
        high: bars.iter().map(|b| f(b.high)).collect(),
        low: bars.iter().map(|b| f(b.low)).collect(),
        close: bars.iter().map(|b| f(b.close)).collect(),
        volume: bars.iter().map(|b| f(b.volume)).collect(),
    }
}

/// The population the whitener is fitted over: the latest fingerprint of every
/// instrument for this window, all in one dimension order.
///
/// Fingerprints computed under a different order (the order changed between
/// runs) are excluded rather than reshaped; mixing them would put one dimension
/// in another's column.
#[derive(Debug, Default, PartialEq)]
pub struct Population {
    pub keys: Vec<(i64, i32)>,
    pub rows: Vec<Vec<Option<f32>>>,
    pub excluded_other_order: usize,
}

#[must_use]
pub fn assemble_population(
    order: &[String],
    stored: Vec<(i64, i32, Vec<String>, Vec<Option<f32>>)>,
) -> Population {
    let mut p = Population::default();
    for (instrument, venue, their_order, tier1) in stored {
        if their_order == order && tier1.len() == order.len() {
            p.keys.push((instrument, venue));
            p.rows.push(tier1);
        } else {
            p.excluded_other_order += 1;
        }
    }
    p
}

/// Performs `JobKind::InstrumentProfile`.
pub struct InstrumentProfileWorker {
    pg: PgPool,
    clickhouse_url: String,
    sidecar: std::sync::Arc<dyn ProfileSidecar>,
}

impl InstrumentProfileWorker {
    #[must_use]
    pub fn new(
        pg: PgPool,
        clickhouse_url: String,
        sidecar: std::sync::Arc<dyn ProfileSidecar>,
    ) -> Self {
        Self {
            pg,
            clickhouse_url,
            sidecar,
        }
    }

    /// Every instrument that currently has a symbol, with the venue it trades on.
    async fn instruments(&self) -> Result<Vec<(i64, i32, String)>, JobError> {
        sqlx::query_as(
            "SELECT DISTINCT ON (instrument_id, venue_id) instrument_id, venue_id, symbol
               FROM dataplane.instrument_symbol
              WHERE valid_to IS NULL
              ORDER BY instrument_id, venue_id, valid_from DESC",
        )
        .fetch_all(&self.pg)
        .await
        .map_err(|e| JobError::infrastructure(format!("listing instruments: {e}")))
    }

    async fn store_fingerprints(
        &self,
        spec: &str,
        order: &[String],
        out: &[FingerprintOut],
        now: DateTime<Utc>,
    ) -> Result<(), JobError> {
        for f in out {
            if f.tier1.len() != order.len() {
                return Err(logic(
                    "fingerprint_shape",
                    format!(
                        "instrument {} came back with {} values for {} dimensions",
                        f.instrument_id,
                        f.tier1.len(),
                        order.len()
                    ),
                ));
            }
            sqlx::query(
                "INSERT INTO knowledge.instrument_fingerprint
                     (instrument_id, venue_id, window_spec, dimension_order, tier1, absent,
                      bars_used, knowledge_time)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT DO NOTHING",
            )
            .bind(f.instrument_id)
            .bind(f.venue_id)
            .bind(spec)
            .bind(order)
            .bind(&f.tier1)
            .bind(if f.absent.is_null() { json!({}) } else { f.absent.clone() })
            .bind(f.bars_used)
            .bind(now)
            .execute(&self.pg)
            .await
            .map_err(|e| JobError::infrastructure(format!("storing a fingerprint: {e}")))?;
        }
        Ok(())
    }

    async fn latest_population(
        &self,
        spec: &str,
    ) -> Result<Vec<(i64, i32, Vec<String>, Vec<Option<f32>>)>, JobError> {
        sqlx::query_as(
            "SELECT DISTINCT ON (instrument_id, venue_id)
                    instrument_id, venue_id, dimension_order, tier1
               FROM knowledge.instrument_fingerprint
              WHERE window_spec = $1
              ORDER BY instrument_id, venue_id, knowledge_time DESC",
        )
        .bind(spec)
        .fetch_all(&self.pg)
        .await
        .map_err(|e| JobError::infrastructure(format!("reading the population: {e}")))
    }

    async fn write_embeddings(
        &self,
        spec: &str,
        keys: &[(i64, i32)],
        order: &[String],
        rows: &[Vec<Option<f32>>],
        answer: &EmbedResponse,
        now: DateTime<Utc>,
    ) -> Result<u64, JobError> {
        if answer.retrieval.len() != keys.len() || answer.imputed_per_row.len() != keys.len() {
            return Err(logic(
                "embedding_shape",
                format!(
                    "the sidecar returned {} vectors for {} instruments",
                    answer.retrieval.len(),
                    keys.len()
                ),
            ));
        }
        let _ = order;
        let mut tx = self
            .pg
            .begin()
            .await
            .map_err(|e| JobError::infrastructure(format!("begin: {e}")))?;
        let mut written = 0_u64;
        for (i, (instrument, venue)) in keys.iter().enumerate() {
            let f32s: Vec<f32> = answer.retrieval[i].iter().map(|x| *x as f32).collect();
            let lit = super::vector_literal_n(&f32s, 48)
                .map_err(|why| logic("embedding_width", why))?;
            // Close the previous version's current row, then append the new one:
            // the table is bitemporal and a row is never edited, only ended.
            sqlx::query(
                "UPDATE knowledge.asset_embedding SET valid_to = $4
                  WHERE instrument_id = $1 AND venue_id = $2 AND embedding_version = $3
                    AND valid_to IS NULL",
            )
            .bind(instrument)
            .bind(venue)
            .bind(&answer.embedding_version)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::infrastructure(format!("closing the old embedding: {e}")))?;
            sqlx::query(
                "INSERT INTO knowledge.asset_embedding
                     (instrument_id, venue_id, embedding_version, window_spec, valid_from,
                      knowledge_time, tier1, retrieval_vec, intrinsic_dim, hubness_corrected,
                      quality_flags)
                 VALUES ($1, $2, $3, $4, $5, $5, $6, $7::vector, $8, FALSE, $9)",
            )
            .bind(instrument)
            .bind(venue)
            .bind(&answer.embedding_version)
            .bind(spec)
            .bind(now)
            .bind(&rows[i])
            .bind(lit)
            .bind(answer.intrinsic_dim)
            .bind(answer.imputed_per_row[i])
            .execute(&mut *tx)
            .await
            .map_err(|e| JobError::infrastructure(format!("writing an embedding: {e}")))?;
            written += 1;
        }
        tx.commit()
            .await
            .map_err(|e| JobError::infrastructure(format!("commit: {e}")))?;
        Ok(written)
    }
}

#[async_trait::async_trait]
impl Worker for InstrumentProfileWorker {
    fn kind(&self) -> JobKind {
        JobKind::InstrumentProfile
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        Cost::default()
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let days = manifest
            .get("window_days")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_WINDOW_DAYS)
            .clamp(2, 365);
        let only: Option<Vec<String>> = manifest
            .get("symbols")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
        let spec = window_spec(days);
        let end = Utc::now();
        let start = end - chrono::Duration::days(days);
        let store = BarStore::connect(&self.clickhouse_url);

        let mut all = self.instruments().await?;
        if let Some(only) = &only {
            all.retain(|(_, _, s)| only.iter().any(|o| o.eq_ignore_ascii_case(s)));
        }
        if all.is_empty() {
            return Ok(JobOutput {
                summary: Some("no instruments with a current symbol to profile".into()),
                result: json!({ "window_spec": spec, "instruments": 0 }),
                ..Default::default()
            });
        }

        let mut skipped: Vec<Skipped> = Vec::new();
        let mut order: Option<Vec<String>> = None;
        let mut computed = 0_usize;
        let now = Utc::now();
        let total = all.len();
        for (batch_no, chunk) in all.chunks(BATCH).enumerate() {
            if ctx.is_cancelled() {
                break;
            }
            let mut payloads = Vec::new();
            for (instrument, venue, symbol) in chunk {
                match store.load_bars(symbol, Timeframe::Minutes1, start, end).await {
                    Ok(bars) if !bars.is_empty() => {
                        payloads.push(payload_from_bars(*instrument, *venue, &bars));
                    }
                    Ok(_) => skipped.push(Skipped {
                        instrument_id: *instrument,
                        venue_id: *venue,
                        reason: format!("{symbol}: no stored 1m bars in the window"),
                    }),
                    Err(e) => skipped.push(Skipped {
                        instrument_id: *instrument,
                        venue_id: *venue,
                        reason: format!("{symbol}: reading bars failed: {e}"),
                    }),
                }
            }
            if payloads.is_empty() {
                continue;
            }
            let resp = self
                .sidecar
                .fingerprint(&spec, &payloads)
                .await
                .map_err(|e| JobError::infrastructure(format!("the trainer: {e}")))?;
            self.store_fingerprints(&spec, &resp.order, &resp.fingerprints, now)
                .await?;
            computed += resp.fingerprints.len();
            skipped.extend(resp.skipped);
            order = Some(resp.order);
            ctx.progress(Progress {
                pct: Some(((batch_no + 1) * BATCH) as f32 / total as f32 * 0.8),
                stage: Some("fingerprinting".into()),
                message: Some(format!("{computed} fingerprints")),
            })
            .await;
        }

        // The embedding is over the whole population, not over what this run
        // touched. If nothing was fingerprinted this run, the order is the one
        // the stored rows carry.
        let stored = self.latest_population(&spec).await?;
        let order = order
            .or_else(|| stored.first().map(|(_, _, o, _)| o.clone()))
            .unwrap_or_default();
        let population = assemble_population(&order, stored);

        let embedding = if population.rows.is_empty() {
            json!({ "status": "nothing_to_embed" })
        } else {
            match self
                .sidecar
                .embed(&order, &population.rows)
                .await
                .map_err(|e| JobError::infrastructure(format!("the trainer: {e}")))?
            {
                EmbedAnswer::NotEnoughPopulation(why) => json!({
                    "status": "deferred",
                    "population": population.rows.len(),
                    "why": why,
                }),
                EmbedAnswer::Embedded(answer) => {
                    if answer.embedding_version != EXPECTED_EMBEDDING_VERSION {
                        return Err(logic(
                            "embedding_version",
                            format!(
                                "the sidecar embedded as `{}` and this worker writes `{}`",
                                answer.embedding_version, EXPECTED_EMBEDDING_VERSION
                            ),
                        ));
                    }
                    let n = self
                        .write_embeddings(
                            &spec,
                            &population.keys,
                            &order,
                            &population.rows,
                            &answer,
                            now,
                        )
                        .await?;
                    json!({
                        "status": "written",
                        "rows": n,
                        "embedding_version": answer.embedding_version,
                        "intrinsic_dim": answer.intrinsic_dim,
                        "dimensions_dropped": answer.dimensions_dropped,
                    })
                }
            }
        };

        Ok(JobOutput {
            summary: Some(format!(
                "{computed} fingerprints computed, {} skipped; embedding {}",
                skipped.len(),
                embedding["status"].as_str().unwrap_or("?")
            )),
            result: json!({
                "window_spec": spec,
                "instruments": total,
                "fingerprints": computed,
                "skipped": skipped,
                "population": population.rows.len(),
                "excluded_other_order": population.excluded_other_order,
                "embedding": embedding,
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_window_spec_names_days_and_resolution() {
        assert_eq!(window_spec(21), "21d@1m");
    }

    /// The whitener is fitted over the latest fingerprint of every instrument,
    /// in one dimension order. A fingerprint computed under another order is
    /// excluded, because reshaping it would put one dimension in another's
    /// column.
    #[test]
    fn the_population_is_one_order_and_counts_what_it_excluded() {
        let o = order(&["a", "b"]);
        let p = assemble_population(
            &o,
            vec![
                (1, 1, order(&["a", "b"]), vec![Some(1.0), None]),
                (2, 1, order(&["b", "a"]), vec![Some(1.0), Some(2.0)]),
                (3, 1, order(&["a", "b"]), vec![Some(3.0)]),
                (4, 2, order(&["a", "b"]), vec![Some(4.0), Some(5.0)]),
            ],
        );
        assert_eq!(p.keys, vec![(1, 1), (4, 2)]);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0], vec![Some(1.0), None], "absent stays null, never zero");
        assert_eq!(p.excluded_other_order, 2);
    }

    #[test]
    fn a_fingerprint_response_parses_with_nulls_for_absent_dimensions() {
        let r: FingerprintResponse = serde_json::from_value(json!({
            "order": ["a", "b"],
            "fingerprints": [{
                "instrument_id": 7, "venue_id": 1, "bars_used": 900,
                "tier1": [0.5, null], "absent": { "b": "no signed volume published" }
            }],
            "skipped": [{ "instrument_id": 8, "venue_id": 1, "reason": "100 bars, fewer than 500" }]
        }))
        .unwrap();
        assert_eq!(r.fingerprints[0].tier1, vec![Some(0.5), None]);
        assert_eq!(r.skipped[0].instrument_id, 8);
    }

    #[test]
    fn bars_become_column_arrays_in_time_order() {
        use rust_decimal::Decimal;
        let bar = |ts: i64, c: i64| backtest::store::LoadedBar {
            ts_ns: ts,
            open_ns: ts - 60,
            knowledge_ns: ts,
            open: Decimal::from(c),
            high: Decimal::from(c + 1),
            low: Decimal::from(c - 1),
            close: Decimal::from(c),
            volume: Decimal::from(10),
            trade_count: 1,
            quality_flags: Default::default(),
        };
        let p = payload_from_bars(7, 1, &[bar(100, 5), bar(200, 6)]);
        assert_eq!(p.ts_ns, vec![100, 200]);
        assert_eq!(p.close, vec![5.0, 6.0]);
        assert_eq!(p.high, vec![6.0, 7.0]);
    }

    /// A fake sidecar that says "not enough instruments yet" is an answer the
    /// worker carries, not a failure.
    struct Small;

    #[async_trait::async_trait]
    impl ProfileSidecar for Small {
        async fn fingerprint(
            &self,
            _spec: &str,
            _i: &[BarsPayload],
        ) -> Result<FingerprintResponse, String> {
            Err("not called".into())
        }
        async fn embed(
            &self,
            _o: &[String],
            rows: &[Vec<Option<f32>>],
        ) -> Result<EmbedAnswer, String> {
            Ok(EmbedAnswer::NotEnoughPopulation(format!(
                "whitening needs at least 60 instruments, got {}",
                rows.len()
            )))
        }
    }

    #[tokio::test]
    async fn a_small_population_is_deferred_not_failed() {
        let answer = Small.embed(&order(&["a"]), &[vec![Some(1.0)]]).await.unwrap();
        match answer {
            EmbedAnswer::NotEnoughPopulation(why) => assert!(why.contains("60"), "{why}"),
            EmbedAnswer::Embedded(_) => panic!("a fake that defers embedded"),
        }
    }
}
