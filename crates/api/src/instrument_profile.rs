//! The `InstrumentProfile` job (SPEC §5.2–§5.3, checklist 3.1/3.3, ADR-P3-01).
//!
//! `apps/model-trainer/app/fingerprint.py` has known how to describe an
//! instrument since Phase 3 landed, and `knowledge.asset_embedding` has been
//! ready to hold the answer for just as long. Nothing joined the two, so the
//! knowledge plane had code, tests and no rows — and an empty embedding table
//! means every similarity question the platform can ask returns nothing.
//!
//! This is that join, and it is deliberately only a join: it reads bars through
//! the single PIT reader, hands them to the sidecar, and writes what comes
//! back. No statistic is computed here. A second fingerprint implementation in
//! Rust would be a second answer to "what is this instrument like", and the two
//! would disagree within a month.
//!
//! ## Three properties this file is responsible for
//!
//! **`knowledge_time` is the job's `as_of`, never `now()` (INV-01).** An
//! embedding computed from bars available at *t* is knowledge as of *t*. The
//! bars are bounded by `BarStore::as_of`, so the vector genuinely cannot carry
//! anything later; stamping the write with the wall clock would throw that away
//! and make `EmbeddingStore::neighbours` — which filters on exactly this column
//! (AT-43) — trust a vector it should have excluded. Backfilling a missed day
//! therefore writes the *old* knowledge time, which is the point.
//!
//! **Re-running writes the same row.** The primary key is
//! `(instrument_id, venue_id, embedding_version, valid_from)` and `valid_from`
//! is the `as_of`, so a job that ran twice — a lost lease, a restarted
//! scheduler, a manual re-submission — upserts rather than accumulating. Two
//! embeddings for one instrument at one knowledge time is the failure this
//! shape rules out.
//!
//! **An unwhitened population is named, not hidden.** `fit_whitener` refuses
//! below sixty instruments. When it does, the batch is stored under
//! [`EMBEDDING_VERSION_UNWHITENED`] with [`QUALITY_UNWHITENED`] set on every
//! row, rather than under the same version as a fitted one. Nothing in this
//! codebase picks an embedding version for a caller, so an unwhitened version
//! can only be read by someone who typed its name; 3.11's validation protocol
//! is what would promote one to a default, and an unwhitened batch cannot pass
//! it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

/// The version a whitened batch is stored under.
///
/// A name rather than a date, for the same reason `RULE_MODEL_VERSION` is: a
/// reader who sees it in a report knows which construction produced the vector.
pub const EMBEDDING_VERSION_WHITENED: &str = "tier1_whitened_v1";

/// The version an unwhitened batch is stored under.
///
/// Separate from the whitened one because they are different metrics. Sharing a
/// version would mean a neighbour list changed its meaning on the day the
/// sixtieth instrument was registered, with nothing in the data saying so.
pub const EMBEDDING_VERSION_UNWHITENED: &str = "tier1_unwhitened_v1";

// The quality-flag bits, mirroring `apps/model-trainer/app/profile.py`. Kept in
// sync by `the_quality_flags_match_the_sidecar`.
pub const QUALITY_UNWHITENED: i32 = 1 << 0;
pub const QUALITY_IMPUTED_DIMENSION: i32 = 1 << 1;
pub const QUALITY_SPARSE_FINGERPRINT: i32 = 1 << 2;

/// The width of `knowledge.asset_embedding.retrieval_vec`.
pub const RETRIEVAL_DIMS: usize = 48;

/// What the job was asked to do.
///
/// Every field is in the manifest, because the manifest is the job's identity
/// (COMP-005 §4): two runs with the same window over the same instruments as of
/// the same instant are the same job, and the scheduler relies on that for its
/// idempotency. A worker that read any of this from the environment instead
/// would make two jobs with the same hash do different things.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileManifest {
    /// The knowledge horizon. Bars are bounded by it and the written rows are
    /// stamped with it.
    pub as_of: DateTime<Utc>,
    /// Bar period in seconds.
    pub period_secs: u32,
    /// How much history the fingerprint is measured over.
    pub lookback_days: i64,
    /// Human-readable window, e.g. `90d@1h`. Stored on the row.
    pub window_spec: String,
    pub instruments: Vec<ProfileTarget>,
    #[serde(default = "default_true")]
    pub deflate: bool,
}

fn default_true() -> bool {
    true
}

/// One instrument to profile, by the surrogate keys the knowledge plane stores.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProfileTarget {
    pub instrument_id: i64,
    pub venue_id: i32,
    /// Whether the venue ever closes. A 24/7 venue has no overnight gap, and
    /// the fingerprint returns `overnight_share: None` rather than `0.0`.
    pub continuous_venue: bool,
}

impl ProfileManifest {
    /// The canonical manifest value. Instruments are sorted, so a population
    /// listed in a different order is still the same job.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut sorted = self.clone();
        sorted.instruments.sort();
        sorted.instruments.dedup();
        serde_json::to_value(sorted).unwrap_or(Value::Null)
    }
}

/// One instrument's row as the sidecar returned it.
#[derive(Debug, Clone, Deserialize)]
struct ProfiledRow {
    instrument_id: i64,
    venue_id: i32,
    tier1: Vec<Option<f32>>,
    retrieval_vec: Vec<f32>,
    #[serde(default)]
    absent: BTreeMap<String, String>,
    quality_flags: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct ProfileBatch {
    instruments: Vec<ProfiledRow>,
    whitened: bool,
    #[serde(default)]
    intrinsic_dim: Option<f32>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    dimensions: Vec<String>,
}

/// Computes tier-1 fingerprints and writes `knowledge.asset_embedding`.
pub struct InstrumentProfileWorker {
    pg: PgPool,
    clickhouse_url: String,
    trainer_url: String,
    http: reqwest::Client,
}

impl InstrumentProfileWorker {
    /// # Panics
    /// If the HTTP client cannot be built, which means the process has no TLS
    /// backend and nothing else would work either.
    #[must_use]
    pub fn new(pg: PgPool, clickhouse_url: String) -> Self {
        let trainer_url = std::env::var("MODEL_TRAINER_URL")
            .unwrap_or_else(|_| "http://localhost:8001".to_string());
        Self::with_trainer(pg, clickhouse_url, trainer_url)
    }

    /// # Panics
    /// As [`InstrumentProfileWorker::new`].
    #[must_use]
    pub fn with_trainer(pg: PgPool, clickhouse_url: String, trainer_url: String) -> Self {
        Self {
            pg,
            clickhouse_url,
            trainer_url,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("http client"),
        }
    }
}

/// The bars for one instrument, in the shape the sidecar's `ProfileBars` wants.
#[derive(Debug, Clone, Serialize)]
struct BarPayload {
    ts_ns: Vec<i64>,
    open: Vec<f64>,
    high: Vec<f64>,
    low: Vec<f64>,
    close: Vec<f64>,
    volume: Vec<f64>,
    continuous_venue: bool,
}

/// The fewest bars worth fingerprinting.
///
/// `compute` measures what it can from any length, but below a couple of
/// hundred bars almost every horizon comes back absent and the row is a list of
/// reasons rather than a description. Sixty-four is where the Hurst exponent
/// becomes measurable, which is the first statistic that needs real history.
const MIN_BARS: usize = 64;

#[async_trait::async_trait]
impl Worker for InstrumentProfileWorker {
    fn kind(&self) -> JobKind {
        JobKind::InstrumentProfile
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        Cost {
            compute_s: Some(30.0),
            gpu_s: None,
            cost_usd: Some(0.0),
        }
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let req: ProfileManifest = serde_json::from_value(manifest.clone()).map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "invalid_manifest",
                format!("instrument_profile needs as_of, period_secs, lookback_days, window_spec and instruments: {e}"),
            )
        })?;
        if req.instruments.is_empty() {
            return Err(JobError::logic(
                ledger::TerminalReason::DataError,
                "empty_population",
                "an InstrumentProfile job with no instruments would fit a whitening transform on \
                 nothing; the scheduler should not have submitted it",
            ));
        }

        // ── read, through the single PIT reader, bounded by the job's horizon ──
        let store = backtest::store::BarStore::connect(&self.clickhouse_url).as_of(req.as_of);
        let to_ns = req.as_of.timestamp_nanos_opt().unwrap_or(i64::MAX);
        let from_ns = (req.as_of - chrono::Duration::days(req.lookback_days))
            .timestamp_nanos_opt()
            .unwrap_or(0);

        let mut payload = Vec::new();
        let mut too_short: Vec<i64> = Vec::new();
        for (i, target) in req.instruments.iter().enumerate() {
            if ctx.is_cancelled() {
                return Err(JobError::terminal(
                    ledger::TerminalReason::Cancelled,
                    "cancelled",
                    "cancelled while reading bars",
                ));
            }
            ctx.progress(Progress {
                pct: Some(i as f32 / req.instruments.len() as f32 * 0.5),
                stage: Some("bars".into()),
                message: None,
            })
            .await;

            let bars = store
                .load_bars_by_key(
                    target.instrument_id,
                    target.venue_id,
                    req.period_secs,
                    from_ns,
                    to_ns,
                )
                .await
                .map_err(|e| JobError::infrastructure(format!("bar_read_failed: {e}")))?;

            if bars.len() < MIN_BARS {
                too_short.push(target.instrument_id);
                continue;
            }

            payload.push(json!({
                "instrument_id": target.instrument_id,
                "venue_id": target.venue_id,
                "bars": BarPayload {
                    ts_ns: bars.iter().map(|b| b.ts_ns).collect(),
                    open: bars.iter().map(|b| to_f64(b.open)).collect(),
                    high: bars.iter().map(|b| to_f64(b.high)).collect(),
                    low: bars.iter().map(|b| to_f64(b.low)).collect(),
                    close: bars.iter().map(|b| to_f64(b.close)).collect(),
                    volume: bars.iter().map(|b| to_f64(b.volume)).collect(),
                    continuous_venue: target.continuous_venue,
                },
            }));
        }

        if payload.is_empty() {
            // Not a failure of the job: the data is not there yet. Reported as a
            // successful pass that wrote nothing, because a failed job would be
            // deduplicated onto by the next tick and never retried.
            return Ok(JobOutput {
                summary: Some(format!(
                    "no instrument had at least {MIN_BARS} bars of {}s at {}; nothing written",
                    req.period_secs, req.as_of
                )),
                result: json!({
                    "rows_written": 0,
                    "skipped_too_short": too_short,
                    "as_of": req.as_of,
                }),
                ..Default::default()
            }
            .clamped());
        }

        // ── the fingerprint itself, in the one place it is implemented ────────
        ctx.progress(Progress {
            pct: Some(0.6),
            stage: Some("fingerprint".into()),
            message: None,
        })
        .await;

        let response = self
            .http
            .post(format!("{}/fingerprint", self.trainer_url))
            .json(&json!({
                "as_of": req.as_of,
                "window_spec": req.window_spec,
                "deflate": req.deflate,
                "instruments": payload,
            }))
            .send()
            .await
            .map_err(|e| JobError::infrastructure(format!("trainer_unreachable: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(JobError::infrastructure(format!(
                "fingerprint_rejected: {status} {}",
                body.chars().take(160).collect::<String>()
            )));
        }
        let batch: ProfileBatch = response
            .json()
            .await
            .map_err(|e| JobError::infrastructure(format!("fingerprint_unreadable: {e}")))?;

        let version = embedding_version(batch.whitened);

        // ── write ─────────────────────────────────────────────────────────────
        ctx.progress(Progress {
            pct: Some(0.9),
            stage: Some("write".into()),
            message: None,
        })
        .await;

        let mut written = 0_u64;
        for row in &batch.instruments {
            if row.retrieval_vec.len() != RETRIEVAL_DIMS {
                return Err(JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "wrong_retrieval_width",
                    format!(
                        "the sidecar returned {} dimensions for instrument {}; the column is \
                         vector({RETRIEVAL_DIMS}) and a short vector would be padded by whoever \
                         stored it, which is a different embedding under the same version",
                        row.retrieval_vec.len(),
                        row.instrument_id
                    ),
                ));
            }
            write_embedding(&self.pg, &req, version, batch.intrinsic_dim, row)
                .await
                .map_err(|e| JobError::infrastructure(format!("embedding_write_failed: {e}")))?;
            written += 1;
        }

        Ok(JobOutput {
            summary: Some(format!(
                "{written} embeddings at {} under {version}{}",
                req.as_of,
                batch
                    .reason
                    .as_deref()
                    .map(|r| format!(" — not whitened: {r}"))
                    .unwrap_or_default()
            )),
            result: json!({
                "rows_written": written,
                "embedding_version": version,
                "whitened": batch.whitened,
                "intrinsic_dim": batch.intrinsic_dim,
                "reason": batch.reason,
                "dimensions": batch.dimensions,
                "skipped_too_short": too_short,
                "absent": batch
                    .instruments
                    .iter()
                    .map(|r| (r.instrument_id.to_string(), r.absent.clone()))
                    .collect::<BTreeMap<_, _>>(),
                "as_of": req.as_of,
            }),
            ..Default::default()
        }
        .clamped())
    }
}

/// Which version a batch is stored under.
#[must_use]
pub fn embedding_version(whitened: bool) -> &'static str {
    if whitened {
        EMBEDDING_VERSION_WHITENED
    } else {
        EMBEDDING_VERSION_UNWHITENED
    }
}

/// Write one embedding, closing the previous validity interval for its key.
///
/// The upsert is what makes a re-run harmless; the interval close is what makes
/// `valid_from`/`valid_to` mean what checklist 3.3 says they mean. Both happen
/// in one transaction, so a crash between them cannot leave two open intervals
/// for the same instrument — which would make "the embedding as of *t*"
/// ambiguous, and an ambiguous as-of read is the failure `knowledge_time` exists
/// to prevent.
async fn write_embedding(
    pg: &PgPool,
    req: &ProfileManifest,
    version: &str,
    intrinsic_dim: Option<f32>,
    row: &ProfiledRow,
) -> Result<(), sqlx::Error> {
    let mut tx = pg.begin().await?;

    sqlx::query(
        "UPDATE knowledge.asset_embedding
            SET valid_to = $4
          WHERE instrument_id = $1 AND venue_id = $2 AND embedding_version = $3
            AND valid_from < $4 AND valid_to IS NULL",
    )
    .bind(row.instrument_id)
    .bind(row.venue_id)
    .bind(version)
    .bind(req.as_of)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO knowledge.asset_embedding
             (instrument_id, venue_id, embedding_version, window_spec, valid_from, valid_to,
              knowledge_time, tier1, tier2, tier3, retrieval_vec, intrinsic_dim,
              hubness_corrected, quality_flags, info_class)
         VALUES ($1, $2, $3, $4, $5, NULL, $5, $6, NULL, NULL, $7::real[]::vector, $8,
                 -- Mutual proximity is a query-time rescaling and nothing applies
                 -- it yet: `EmbeddingStore::neighbours` takes a plain cosine
                 -- distance. The column's default is TRUE, so writing this
                 -- explicitly is the difference between a row that says what was
                 -- done and one that inherits a claim nobody made.
                 false, $9, 'market_public')
         ON CONFLICT (instrument_id, venue_id, embedding_version, valid_from) DO UPDATE
           SET window_spec       = EXCLUDED.window_spec,
               knowledge_time    = EXCLUDED.knowledge_time,
               tier1             = EXCLUDED.tier1,
               retrieval_vec     = EXCLUDED.retrieval_vec,
               intrinsic_dim     = EXCLUDED.intrinsic_dim,
               hubness_corrected = EXCLUDED.hubness_corrected,
               quality_flags     = EXCLUDED.quality_flags,
               valid_to          = NULL",
    )
    .bind(row.instrument_id)
    .bind(row.venue_id)
    .bind(version)
    .bind(&req.window_spec)
    .bind(req.as_of)
    .bind(&row.tier1)
    .bind(&row.retrieval_vec)
    .bind(intrinsic_dim)
    .bind(row.quality_flags)
    .execute(&mut *tx)
    .await?;

    tx.commit().await
}

fn to_f64(d: rust_decimal::Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(instruments: Vec<ProfileTarget>) -> ProfileManifest {
        ProfileManifest {
            as_of: DateTime::from_timestamp(1_700_000_000, 0).expect("an instant"),
            period_secs: 3600,
            lookback_days: 90,
            window_spec: "90d@1h".into(),
            instruments,
            deflate: true,
        }
    }

    fn target(id: i64) -> ProfileTarget {
        ProfileTarget {
            instrument_id: id,
            venue_id: 4,
            continuous_venue: false,
        }
    }

    /// The manifest *is* the job's identity (COMP-005 §4), so a population the
    /// scheduler happened to list in a different order must hash the same. Two
    /// jobs for one knowledge day is two embeddings at one knowledge time, which
    /// is exactly what the schedule must never produce.
    #[test]
    fn a_reordered_population_is_the_same_job() {
        let a = manifest(vec![target(3), target(1), target(2)]);
        let b = manifest(vec![target(1), target(2), target(3)]);
        assert_eq!(a.to_value(), b.to_value());
        assert_eq!(
            jobs::manifest::hash(&a.to_value(), &[], "latest", "instrument_profile"),
            jobs::manifest::hash(&b.to_value(), &[], "latest", "instrument_profile"),
        );
    }

    /// A duplicated instrument is the same population, not a bigger one. Left
    /// in, it would be fingerprinted twice, counted twice in the whitening fit,
    /// and written twice to the same primary key.
    #[test]
    fn a_duplicated_instrument_does_not_change_the_job() {
        let a = manifest(vec![target(1), target(2)]);
        let b = manifest(vec![target(1), target(2), target(1)]);
        assert_eq!(a.to_value(), b.to_value());
    }

    /// Different horizons are different jobs. Without this the scheduler's daily
    /// submission would deduplicate onto yesterday's and the plane would hold
    /// exactly one day of knowledge forever.
    #[test]
    fn a_different_as_of_is_a_different_job() {
        let a = manifest(vec![target(1)]);
        let mut b = a.clone();
        b.as_of += chrono::Duration::days(1);
        assert_ne!(
            jobs::manifest::hash(&a.to_value(), &[], "latest", "instrument_profile"),
            jobs::manifest::hash(&b.to_value(), &[], "latest", "instrument_profile"),
        );
    }

    /// A whitened and an unwhitened batch are different metrics and must not
    /// share a version: a neighbour list that silently changed meaning on the
    /// day the sixtieth instrument was registered is worse than one that
    /// visibly did.
    #[test]
    fn whitened_and_unwhitened_batches_are_stored_under_different_versions() {
        assert_ne!(embedding_version(true), embedding_version(false));
        assert!(embedding_version(false).contains("unwhitened"));
    }

    /// The bitmask is shared with the sidecar by value, not by import. This is
    /// the test that notices when one side moves.
    #[test]
    fn the_quality_flags_match_the_sidecar() {
        let src = include_str!("../../../apps/model-trainer/app/profile.py");
        for (name, value) in [
            ("QUALITY_UNWHITENED", QUALITY_UNWHITENED),
            ("QUALITY_IMPUTED_DIMENSION", QUALITY_IMPUTED_DIMENSION),
            ("QUALITY_SPARSE_FINGERPRINT", QUALITY_SPARSE_FINGERPRINT),
        ] {
            let shift = value.trailing_zeros();
            assert!(
                src.contains(&format!("{name} = 1 << {shift}")),
                "{name} is {value} here; the sidecar disagrees"
            );
        }
        assert!(
            src.contains(&format!("RETRIEVAL_DIMS = {RETRIEVAL_DIMS}"))
                || src.contains("embedding.RETRIEVAL_DIMS"),
            "the retrieval width must be the same on both sides"
        );
    }

    /// `deflate` defaults on, because the failure it prevents is silent: crypto's
    /// hour-of-week volatility shape otherwise makes the fingerprint encode what
    /// time the window happened to start (checklist 3.2).
    #[test]
    fn seasonal_deflation_is_on_unless_a_manifest_turns_it_off() {
        let m: ProfileManifest = serde_json::from_value(json!({
            "as_of": "2026-09-15T00:00:00Z",
            "period_secs": 3600,
            "lookback_days": 90,
            "window_spec": "90d@1h",
            "instruments": [{"instrument_id": 1, "venue_id": 4, "continuous_venue": true}],
        }))
        .expect("a manifest without deflate");
        assert!(m.deflate);
    }
}
