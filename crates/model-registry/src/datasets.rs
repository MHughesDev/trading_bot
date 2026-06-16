//! Dataset materialization: pulls `ClickHouse` bars via the leakage-safe
//! point-in-time [`DataView`], computes the feature columns, forward-labels, and
//! writes a pinned Parquet snapshot to the [`ArtifactStore`].
//!
//! ADR-0008/0009: available-time correct, deterministic, no look-ahead. The
//! `content_hash` covers the materialization parameters *and* the encoded Parquet
//! bytes, so an identical request over identical data reproduces the same hash
//! and reuses the existing snapshot rather than rewriting it (I-0.5/I-0.6).

use std::sync::Arc;

use anyhow::{Context, Result};
use backtest::store::{BarStore, LoadedBar};
use chrono::{DateTime, Utc};
use domain::payloads::bar::Timeframe;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use storage::artifacts::{self, ArtifactStore};
use uuid::Uuid;

use crate::data_view::{AsOf, DataView};

/// Base (stored) timeframe the PIT view resamples *from*. Collectors persist 1m
/// candles; coarser request timeframes are assembled forming-bar-safely.
const BASE_TIMEFRAME: Timeframe = Timeframe::Minutes1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRequest {
    pub dataset_id: Option<String>,
    pub feature_set_ref: String,
    pub instruments: Vec<String>,
    /// Bar timeframe key, e.g. `"1m"`.
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// JSON label spec, e.g. {"type": "`forward_return`", "window": "1h", "clip": [-0.2, 0.2]}
    pub label_spec: serde_json::Value,
    pub output_prefix: String,
}

fn default_timeframe() -> String {
    "1m".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetVersionRecord {
    pub dataset_version_id: Uuid,
    pub dataset_id: String,
    pub version: i32,
    pub feature_set_ref: String,
    pub instruments: Vec<String>,
    /// Realized span start (`min available_time` of surviving rows).
    pub start: DateTime<Utc>,
    /// Realized span end (`max available_time` of surviving rows).
    pub end: DateTime<Utc>,
    pub label_spec: serde_json::Value,
    pub row_count: i64,
    pub content_hash: String,
    pub parquet_uri: String,
    pub created_at: DateTime<Utc>,
}

pub struct DatasetManager {
    pg: PgPool,
    /// Point-in-time bar source. `None` in unit tests / when `CLICKHOUSE_URL` is
    /// unset; materialization then yields an empty (`row_count = 0`) snapshot
    /// rather than fabricating data.
    store: Option<BarStore>,
    artifacts: Arc<dyn ArtifactStore>,
}

impl DatasetManager {
    /// Build from environment: `CLICKHOUSE_URL` for the PIT store and
    /// `ARTIFACT_STORE` for the snapshot sink (defaults to local FS).
    pub fn new(pg: PgPool) -> Self {
        let store = std::env::var("CLICKHOUSE_URL")
            .ok()
            .filter(|u| !u.is_empty())
            .map(|url| BarStore::connect(&url));
        Self {
            pg,
            store,
            artifacts: Arc::from(artifacts::from_env()),
        }
    }

    /// Explicit constructor (tests / callers that already hold a store + sink).
    pub fn with_parts(
        pg: PgPool,
        store: Option<BarStore>,
        artifacts: Arc<dyn ArtifactStore>,
    ) -> Self {
        Self {
            pg,
            store,
            artifacts,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn materialize(&self, req: DatasetRequest) -> Result<DatasetVersionRecord> {
        // 1. Resolve the feature set (column names the snapshot must carry).
        let fs = features::resolve_feature_set(&req.feature_set_ref)
            .ok_or_else(|| anyhow::anyhow!("unknown feature_set_ref: {}", req.feature_set_ref))?;

        let target_tf = <Timeframe as backtest::types::TimeframeExt>::from_key(&req.timeframe)
            .ok_or_else(|| anyhow::anyhow!("unknown timeframe: {}", req.timeframe))?;

        // Label horizon (bars) from the spec's `window` token, e.g. "1h".
        let horizon_token = req
            .label_spec
            .get("window")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("1h");
        let horizon_bars = features::label_horizon_bars(horizon_token, &req.timeframe)
            .ok_or_else(|| anyhow::anyhow!("unparseable label window: {horizon_token}"))?;

        let dataset_id = req
            .dataset_id
            .clone()
            .unwrap_or_else(|| format!("ds_{}", Uuid::new_v4().as_simple()));

        // 2. PIT pull → resample → features → label, accumulated columnar across
        //    all requested instruments. `as_of = end`: no bar past the window's
        //    right edge can enter the snapshot (ADR-0008, leakage-structural).
        let as_of = AsOf::from_datetime(req.end);
        let mut acc = FrameAccumulator::new(fs.features.clone());

        if let Some(store) = &self.store {
            let view = DataView::new(store);
            for instrument in &req.instruments {
                let bars = view
                    .bars(
                        instrument,
                        BASE_TIMEFRAME,
                        target_tf,
                        req.start,
                        req.end,
                        as_of,
                    )
                    .await
                    .with_context(|| format!("PIT pull failed for {instrument}"))?;
                let ohlcv = to_ohlcv(&bars);
                let frame = features::build_training_frame(&ohlcv, &fs.features, horizon_bars);
                acc.push(instrument, &frame);
            }
        }

        let row_count = i64::try_from(acc.row_count()).unwrap_or(i64::MAX);
        let (realized_start, realized_end) = acc.realized_span(req.start, req.end);

        // 3. Encode Parquet bytes and hash over (params ‖ bytes) — deterministic.
        let parquet_bytes = acc.encode_parquet()?;
        let param_str = serde_json::to_string(&serde_json::json!({
            "feature_set_ref": req.feature_set_ref,
            "instruments": req.instruments,
            "timeframe": req.timeframe,
            "start": req.start,
            "end": req.end,
            "label_spec": req.label_spec,
            "features": fs.features,
            "horizon_bars": horizon_bars,
        }))?;
        let mut hasher_input = param_str.into_bytes();
        hasher_input.extend_from_slice(&parquet_bytes);
        let content_hash = format!("sha256:{}", hex_sha256(&hasher_input));

        // 4. Idempotency: identical params + data ⇒ identical hash ⇒ reuse the
        //    existing pinned version without rewriting its bytes (I-0.6).
        if let Some(existing) = self.find_by_hash(&content_hash).await? {
            return Ok(existing);
        }

        // 5. Write the immutable Parquet snapshot, keyed by content hash so the
        //    object is itself content-addressed.
        let key = format!(
            "{}/dataset_versions/{}.parquet",
            req.output_prefix.trim_end_matches('/'),
            content_hash.trim_start_matches("sha256:"),
        );
        let store = self.artifacts.clone();
        let bytes = parquet_bytes;
        let key_for_put = key.clone();
        let artifact =
            tokio::task::spawn_blocking(move || store.put_blocking(&key_for_put, &bytes))
                .await
                .context("artifact put task panicked")??;
        let parquet_uri = artifact.uri;

        // 6. Upsert dataset identity + version row.
        sqlx::query(
            "INSERT INTO datasets (dataset_id, feature_set_ref, label_spec_json, created_at) \
             VALUES ($1, $2, $3, now()) ON CONFLICT (dataset_id) DO NOTHING",
        )
        .bind(&dataset_id)
        .bind(&req.feature_set_ref)
        .bind(&req.label_spec)
        .execute(&self.pg)
        .await?;

        let (version,): (i32,) = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM dataset_versions WHERE dataset_id = $1",
        )
        .bind(&dataset_id)
        .fetch_one(&self.pg)
        .await?;

        let dataset_version_id = Uuid::new_v4();
        let instruments_json = serde_json::to_value(&req.instruments)?;
        sqlx::query(
            "INSERT INTO dataset_versions \
             (dataset_version_id, dataset_id, version, feature_set_ref, instruments_json, \
              start_time, end_time, label_spec_json, row_count, content_hash, parquet_uri, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,now())",
        )
        .bind(dataset_version_id)
        .bind(&dataset_id)
        .bind(version)
        .bind(&req.feature_set_ref)
        .bind(&instruments_json)
        .bind(realized_start)
        .bind(realized_end)
        .bind(&req.label_spec)
        .bind(row_count)
        .bind(&content_hash)
        .bind(&parquet_uri)
        .execute(&self.pg)
        .await?;

        Ok(DatasetVersionRecord {
            dataset_version_id,
            dataset_id,
            version,
            feature_set_ref: req.feature_set_ref,
            instruments: req.instruments,
            start: realized_start,
            end: realized_end,
            label_spec: req.label_spec,
            row_count,
            content_hash,
            parquet_uri,
            created_at: Utc::now(),
        })
    }

    /// Look up a fully-populated record by `content_hash` (idempotent reuse).
    async fn find_by_hash(&self, content_hash: &str) -> Result<Option<DatasetVersionRecord>> {
        let row = self
            .fetch_one_where("content_hash = $1", content_hash)
            .await?;
        Ok(row)
    }

    pub async fn get_version(
        &self,
        dataset_version_id: Uuid,
    ) -> Result<Option<DatasetVersionRecord>> {
        self.fetch_one_where(
            "dataset_version_id = $1::uuid",
            &dataset_version_id.to_string(),
        )
        .await
    }

    /// Shared row → record loader for the single-column lookups above.
    async fn fetch_one_where(
        &self,
        predicate: &str,
        bind: &str,
    ) -> Result<Option<DatasetVersionRecord>> {
        #[allow(clippy::type_complexity)]
        let row: Option<(
            Uuid,
            String,
            i32,
            String,
            serde_json::Value,
            DateTime<Utc>,
            DateTime<Utc>,
            serde_json::Value,
            i64,
            String,
            String,
            DateTime<Utc>,
        )> = sqlx::query_as(&format!(
            "SELECT dataset_version_id, dataset_id, version, feature_set_ref, instruments_json, \
             start_time, end_time, label_spec_json, row_count, content_hash, parquet_uri, created_at \
             FROM dataset_versions WHERE {predicate}"
        ))
        .bind(bind)
        .fetch_optional(&self.pg)
        .await?;

        Ok(row.map(
            |(
                vid,
                did,
                ver,
                fsr,
                instruments_json,
                start,
                end,
                label_spec,
                row_count,
                content_hash,
                parquet_uri,
                created_at,
            )| {
                let instruments: Vec<String> =
                    serde_json::from_value(instruments_json).unwrap_or_default();
                DatasetVersionRecord {
                    dataset_version_id: vid,
                    dataset_id: did,
                    version: ver,
                    feature_set_ref: fsr,
                    instruments,
                    start,
                    end,
                    label_spec,
                    row_count,
                    content_hash,
                    parquet_uri,
                    created_at,
                }
            },
        ))
    }
}

/// Convert PIT bars to the pure builder's float OHLCV rows.
fn to_ohlcv(bars: &[LoadedBar]) -> Vec<features::OhlcvRow> {
    bars.iter()
        .map(|b| features::OhlcvRow {
            ts_ns: b.ts_ns,
            open: b.open.to_f64().unwrap_or(f64::NAN),
            high: b.high.to_f64().unwrap_or(f64::NAN),
            low: b.low.to_f64().unwrap_or(f64::NAN),
            close: b.close.to_f64().unwrap_or(f64::NAN),
            volume: b.volume.to_f64().unwrap_or(f64::NAN),
        })
        .collect()
}

/// Columnar accumulator across instruments, plus the Parquet encoder. Holds the
/// `ts_ns`, `instrument`, per-feature, and `label` columns of the whole snapshot.
struct FrameAccumulator {
    feature_names: Vec<String>,
    ts_ns: Vec<i64>,
    instrument: Vec<String>,
    columns: Vec<Vec<f64>>,
    label: Vec<f64>,
}

impl FrameAccumulator {
    fn new(feature_names: Vec<String>) -> Self {
        let n = feature_names.len();
        Self {
            feature_names,
            ts_ns: Vec::new(),
            instrument: Vec::new(),
            columns: vec![Vec::new(); n],
            label: Vec::new(),
        }
    }

    /// Append one instrument's frame, tagging every row with `instrument`. Only
    /// the feature columns the accumulator was created with are kept (the frame
    /// names are a subset in the same order, since both derive from the feature
    /// set), so column alignment is by position within that shared order.
    fn push(&mut self, instrument: &str, frame: &features::TrainingFrame) {
        // Map accumulator column index → this frame's column index (frames skip
        // names they cannot compute, so indices can diverge).
        let mapping: Vec<Option<usize>> = self
            .feature_names
            .iter()
            .map(|name| frame.feature_names.iter().position(|n| n == name))
            .collect();

        for r in 0..frame.row_count() {
            self.ts_ns.push(frame.ts_ns[r]);
            self.instrument.push(instrument.to_string());
            self.label.push(frame.label[r]);
            for (acc_col, src) in self.columns.iter_mut().zip(mapping.iter()) {
                let v = match src {
                    Some(idx) => frame.columns[*idx][r],
                    None => f64::NAN,
                };
                acc_col.push(v);
            }
        }
    }

    fn row_count(&self) -> usize {
        self.ts_ns.len()
    }

    /// Realized `[min, max]` `available_time` of surviving rows, falling back to
    /// the requested span when the snapshot is empty.
    fn realized_span(
        &self,
        req_start: DateTime<Utc>,
        req_end: DateTime<Utc>,
    ) -> (DateTime<Utc>, DateTime<Utc>) {
        match (self.ts_ns.iter().min(), self.ts_ns.iter().max()) {
            (Some(&lo), Some(&hi)) => (ns_to_dt(lo, req_start), ns_to_dt(hi, req_end)),
            _ => (req_start, req_end),
        }
    }

    /// Encode the accumulated columns as a single Parquet buffer. Schema:
    /// `ts_ns: Int64, instrument: Utf8, <feature: Float64>…, label: Float64`.
    fn encode_parquet(&self) -> Result<Vec<u8>> {
        use arrow::{
            array::{ArrayRef, Float64Array, Int64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        };
        use parquet::arrow::ArrowWriter;

        let mut fields = vec![
            Field::new("ts_ns", DataType::Int64, false),
            Field::new("instrument", DataType::Utf8, false),
        ];
        for name in &self.feature_names {
            fields.push(Field::new(name, DataType::Float64, true));
        }
        fields.push(Field::new("label", DataType::Float64, false));
        let schema = Arc::new(Schema::new(fields));

        let mut arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(self.ts_ns.clone())),
            Arc::new(StringArray::from(
                self.instrument
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )),
        ];
        for col in &self.columns {
            arrays.push(Arc::new(Float64Array::from(col.clone())));
        }
        arrays.push(Arc::new(Float64Array::from(self.label.clone())));

        let batch =
            RecordBatch::try_new(schema.clone(), arrays).context("arrow record batch assembly")?;

        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer =
                ArrowWriter::try_new(&mut buf, schema, None).context("parquet writer init")?;
            writer.write(&batch).context("parquet write")?;
            writer.close().context("parquet finalize")?;
        }
        Ok(buf)
    }
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
fn ns_to_dt(ns: i64, fallback: DateTime<Utc>) -> DateTime<Utc> {
    let subsec = ns.rem_euclid(1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(ns.div_euclid(1_000_000_000), subsec).unwrap_or(fallback)
}

fn hex_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(names: &[&str]) -> features::TrainingFrame {
        let bars: Vec<features::OhlcvRow> = (0..30)
            .map(|i| features::OhlcvRow {
                ts_ns: i64::from(i) * 60_000_000_000,
                open: 100.0 + f64::from(i),
                high: 100.0 + f64::from(i),
                low: 100.0 + f64::from(i),
                close: 100.0 + f64::from(i),
                volume: 1.0,
            })
            .collect();
        let feats: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        features::build_training_frame(&bars, &feats, 1)
    }

    #[test]
    fn accumulator_concatenates_instruments_with_tag() {
        let names = vec!["close".to_string(), "ema_7".to_string()];
        let mut acc = FrameAccumulator::new(names.clone());
        let f = frame(&["close", "ema_7"]);
        let per_instrument = f.row_count();
        acc.push("BTC-USD", &f);
        acc.push("ETH-USD", &f);
        assert_eq!(acc.row_count(), per_instrument * 2);
        assert_eq!(acc.instrument[0], "BTC-USD");
        assert_eq!(acc.instrument[per_instrument], "ETH-USD");
        assert_eq!(acc.columns.len(), 2);
        assert_eq!(acc.columns[0].len(), acc.row_count());
    }

    #[test]
    fn parquet_roundtrips_row_count_and_schema() {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        let names = vec!["close".to_string(), "ema_7".to_string()];
        let mut acc = FrameAccumulator::new(names);
        let f = frame(&["close", "ema_7"]);
        acc.push("BTC-USD", &f);
        let expected_rows = acc.row_count();
        assert!(expected_rows > 0, "frame produced rows");

        let bytes = acc.encode_parquet().expect("encode");
        assert!(!bytes.is_empty(), "non-empty parquet");

        let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
            .expect("reader")
            .build()
            .expect("build");
        let mut total = 0usize;
        let mut cols = 0usize;
        for batch in reader {
            let batch = batch.expect("batch");
            total += batch.num_rows();
            cols = batch.num_columns();
        }
        assert_eq!(total, expected_rows, "row count round-trips");
        // ts_ns + instrument + 2 features + label
        assert_eq!(cols, 5);
    }

    #[test]
    fn encode_is_deterministic() {
        let names = vec!["close".to_string()];
        let build = || {
            let mut acc = FrameAccumulator::new(names.clone());
            acc.push("BTC-USD", &frame(&["close"]));
            acc.encode_parquet().expect("encode")
        };
        assert_eq!(build(), build(), "identical input ⇒ identical bytes");
    }

    #[test]
    fn empty_accumulator_encodes_zero_row_parquet() {
        let acc = FrameAccumulator::new(vec!["close".to_string()]);
        assert_eq!(acc.row_count(), 0);
        let bytes = acc.encode_parquet().expect("encode empty");
        assert!(!bytes.is_empty(), "still a valid parquet file");
    }

    #[test]
    fn realized_span_uses_row_bounds() {
        let names = vec!["close".to_string()];
        let mut acc = FrameAccumulator::new(names);
        acc.push("BTC-USD", &frame(&["close"]));
        let fallback_start = Utc::now();
        let fallback_end = Utc::now();
        let (lo, hi) = acc.realized_span(fallback_start, fallback_end);
        assert!(lo <= hi);
        assert_ne!(lo, fallback_start, "real bounds, not the fallback");
    }
}
