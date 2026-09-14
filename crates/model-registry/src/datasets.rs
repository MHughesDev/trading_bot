//! Dataset materialization: pulls `ClickHouse` bars via the leakage-safe
//! point-in-time [`DataView`], computes the feature columns, forward-labels, and
//! writes a pinned Parquet snapshot to the [`ArtifactStore`].
//!
//! **A dataset is a hash, not a path** (SPEC §3.1, INV-12). The `dataset_id` is
//! the content hash of a [`DatasetSpec`] that names the resolved surrogate
//! instrument ids, the point-in-time anchor, the quality exclusion mask, the
//! pinned calendar versions, the adjustment policy, the versioned feature-DAG
//! hash, the label and split specs, and the runtime image digest. Identical
//! `dataset_id` implies byte-identical data.
//!
//! The spec is built by [`DatasetManager::plan`] **before** anything is
//! dispatched, so the trial ledger records the real `dataset_id` rather than a
//! placeholder that is only knowable after the data has been read.

use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use backtest::store::{BarStore, LoadedBar};
use chrono::{DateTime, Utc};
use dataplane::calendar::CalendarVersion;
use dataplane::corporate::AdjustmentPolicy;
use dataplane::dataset::DatasetSpec;
use dataplane::identity::InstrumentKey;
use dataplane::label::LabelSpec;
use dataplane::quality::QualityFlags;
use dataplane::split::SplitSpec;
use domain::payloads::bar::Timeframe;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use storage::artifacts::{self, ArtifactStore};
use uuid::Uuid;

use crate::data_view::{AsOf, DataView};

/// Base (stored) timeframe the PIT view resamples *from*. Collectors persist 1m
/// candles; coarser request timeframes are assembled forming-bar-safely.
const BASE_TIMEFRAME: Timeframe = Timeframe::Minutes1;

/// The digest of the binary that materializes datasets. REQUIRED in every
/// dataset spec (§3.1) and therefore in every `dataset_id`: a code change that
/// alters how a frame is built must produce a different dataset, not silently
/// reuse a snapshot an older build wrote.
///
/// There is no default. A binary that cannot read its own image refuses to plan
/// a dataset rather than hashing a constant that means "some build, unknown".
///
/// # Errors
/// The running executable cannot be located or read.
pub fn runtime_image_digest() -> Result<&'static str> {
    static DIGEST: OnceLock<Option<String>> = OnceLock::new();
    DIGEST
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let bytes = std::fs::read(exe).ok()?;
            Some(format!("sha256:{}", hex_sha256(&bytes)))
        })
        .as_deref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "runtime_image_digest is required and could not be computed: the running executable could not be read"
            )
        })
}

/// What a caller asks for. The label spec is typed, not JSON: `LabelSpec` has no
/// serde default for `sample_weight_method` (§3.4), so a request that omits the
/// declaration cannot be constructed at all.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatasetRequest {
    /// The bitemporal membership query this universe came from.
    pub universe_spec_id: String,
    pub feature_set_ref: String,
    pub instruments: Vec<String>,
    /// Bar timeframe key, e.g. `"1m"`.
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub label_spec: LabelSpec,
    /// Fold geometry. A split is a rule with a deterministic expansion, never a
    /// slice, so it is fully known before the data is read (§3.5).
    pub split_spec: SplitSpec,
    /// Rows carrying any of these flags are excluded; the mask enters the hash.
    #[serde(default = "default_exclusion_mask")]
    pub quality_exclusion_mask: QualityFlags,
    #[serde(default = "default_adjustment_policy")]
    pub adjustment_policy: AdjustmentPolicy,
    pub output_prefix: String,
}

fn default_timeframe() -> String {
    "1m".to_string()
}

/// Crypto spot bars are unadjusted by construction; there are no corporate
/// actions to compose.
fn default_adjustment_policy() -> AdjustmentPolicy {
    AdjustmentPolicy::Unadjusted
}

/// Exclude rows the ingest layer already marked untrustworthy. Backfilled
/// knowledge time is deliberately **not** excluded: those rows are flagged on
/// the dataset (`uses_backfilled_knowledge`), not dropped (CLAUDE.md §6).
pub fn default_exclusion_mask() -> QualityFlags {
    QualityFlags::VENUE_OUTAGE
        .union(QualityFlags::INTERPOLATED)
        .union(QualityFlags::NON_REPRODUCIBLE)
}

/// A resolved, hashed dataset request. Produced before dispatch; the
/// `dataset_id` it carries is what the trial ledger records.
#[derive(Clone, Debug)]
pub struct DatasetPlan {
    pub spec: DatasetSpec,
    pub dataset_id: String,
    /// The principal the specs are written under (ADR-P0-16). Not part of the
    /// dataset's identity -- the same spec under two tenants is the same data.
    pub tenant: String,
    /// Requested symbols paired with the surrogate ids they resolved to, in
    /// request order. Empty only when no bar store is configured.
    pub resolved: Vec<(String, i64, i32)>,
    pub horizon_bars: u64,
    pub features: Vec<String>,
    req: DatasetRequest,
}

impl DatasetPlan {
    #[must_use]
    pub fn request(&self) -> &DatasetRequest {
        &self.req
    }

    /// Propagates to `TrialSubject.non_reproducible` (INV-08).
    #[must_use]
    pub fn non_reproducible(&self) -> bool {
        self.spec.non_reproducible()
    }

    /// Propagates to `TrialSubject.overlapping_labels_unweighted` (§3.4).
    #[must_use]
    pub fn overlapping_labels_unweighted(&self) -> bool {
        self.req.label_spec.overlapping_labels_unweighted()
    }

    #[must_use]
    pub fn split_spec_id(&self) -> &str {
        &self.req.split_spec.split_spec_id
    }

    #[must_use]
    pub fn label_spec(&self) -> &LabelSpec {
        &self.req.label_spec
    }
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

    /// Resolve a request into a hashed [`DatasetSpec`] **before** any compute is
    /// dispatched (SPEC 3.1, INV-12).
    ///
    /// Resolution is the point of this step: the surrogate instrument ids are
    /// materialized into the spec rather than left as a universe query that would
    /// silently change as membership data is revised, and the feature set is
    /// reduced to its versioned DAG hash so a changed implementation yields a new
    /// dataset instead of reusing an older build's snapshot.
    ///
    /// # Errors
    /// An unknown feature set or timeframe, an invalid label spec, a symbol that
    /// resolves to no instrument, or a runtime image digest that cannot be
    /// computed.
    pub async fn plan(&self, tenant: &str, req: DatasetRequest) -> Result<DatasetPlan> {
        let fs = features::resolve_feature_set(&req.feature_set_ref)
            .ok_or_else(|| anyhow::anyhow!("unknown feature_set_ref: {}", req.feature_set_ref))?;
        <Timeframe as backtest::types::TimeframeExt>::from_key(&req.timeframe)
            .ok_or_else(|| anyhow::anyhow!("unknown timeframe: {}", req.timeframe))?;

        req.label_spec
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid label spec: {e}"))?;
        let horizon_bars = u64::from(req.label_spec.horizon_bars);

        // The versioned feature-DAG hash, not the human-facing set name.
        let feature_set_id = features::runtime::feature_set_version_hash(&fs.features)
            .map_err(|e| anyhow::anyhow!("feature set {}: {e}", req.feature_set_ref))?;

        // Surrogate ids, resolved now and stored explicitly (INV-04).
        let mut resolved: Vec<(String, i64, i32)> = Vec::new();
        if let Some(store) = &self.store {
            for symbol in &req.instruments {
                let (instrument_id, venue_id) = store
                    .resolve_instrument(symbol)
                    .await
                    .with_context(|| format!("resolving instrument {symbol}"))?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{symbol} resolves to no instrument: a dataset cannot name a symbol \
                             that has no surrogate id"
                        )
                    })?;
                resolved.push((symbol.clone(), instrument_id, venue_id));
            }
        }

        let spec = DatasetSpec {
            universe_spec_id: req.universe_spec_id.clone(),
            instrument_ids: resolved
                .iter()
                .map(|(_, id, _)| InstrumentKey(*id))
                .collect(),
            date_range: (req.start, req.end),
            frequency: req.timeframe.clone(),
            feature_set_id,
            label_spec_id: req.label_spec.label_spec_id.clone(),
            split_spec_id: req.split_spec.split_spec_id.clone(),
            // THE point-in-time anchor: nothing knowable after `end` may enter.
            as_of_knowledge_time: req.end,
            quality_exclusion_mask: req.quality_exclusion_mask,
            calendar_versions: calendar_versions_for(&req.instruments),
            adjustment_policy: req.adjustment_policy,
            continuous_method: None,
            finality_policy: None,
            runtime_image_digest: runtime_image_digest()?.to_string(),
            opted_in_non_reproducible_sources: Vec::new(),
        }
        .normalized();

        Ok(DatasetPlan {
            dataset_id: spec.dataset_id(),
            spec,
            tenant: tenant.to_string(),
            resolved,
            horizon_bars,
            features: fs.features.clone(),
            req,
        })
    }

    /// Build the frame described by `plan` and pin it.
    ///
    /// The snapshot is keyed by the plan's `dataset_id` -- the hash of the spec,
    /// not of the bytes -- so the identity a trial recorded before execution is
    /// the identity the stored data carries. Re-materializing an existing
    /// `dataset_id` returns the pinned row without rewriting it (I-0.6).
    ///
    /// # Errors
    /// A failed point-in-time read, Parquet encode, artifact write or DB write.
    #[allow(clippy::too_many_lines)]
    pub async fn materialize(&self, plan: &DatasetPlan) -> Result<DatasetVersionRecord> {
        let req = &plan.req;

        // Idempotency: the spec *is* the identity, so an existing row for this
        // dataset_id is the same data by construction (INV-12).
        if let Some(existing) = self.find_by_hash(&plan.dataset_id).await? {
            return Ok(existing);
        }

        let Built {
            acc,
            row_flags,
            parquet_bytes,
            frame_digest,
        } = self.build_frame(plan).await?;

        let row_count = i64::try_from(acc.row_count()).unwrap_or(i64::MAX);
        let (realized_start, realized_end) = acc.realized_span(req.start, req.end);
        let uses_backfilled = plan.spec.uses_backfilled_knowledge([row_flags]);

        // The object is addressed by the spec hash, so the pinned bytes and the
        // id the ledger recorded cannot drift apart.
        let key = format!(
            "{}/dataset_versions/{}.parquet",
            req.output_prefix.trim_end_matches('/'),
            plan.dataset_id.trim_start_matches("sha256:"),
        );
        let store = self.artifacts.clone();
        let key_for_put = key.clone();
        let artifact = tokio::task::spawn_blocking(move || {
            store.put_blocking(&key_for_put, &parquet_bytes)
        })
        .await
        .context("artifact put task panicked")??;
        let parquet_uri = artifact.uri;

        self.register_specs(plan, uses_backfilled).await?;
        self.record_frame_digest(plan, &frame_digest, row_count)
            .await?;

        let label_json = serde_json::to_value(&req.label_spec)?;
        sqlx::query(
            "INSERT INTO datasets (dataset_id, feature_set_ref, label_spec_json, created_at) \
             VALUES ($1, $2, $3, now()) ON CONFLICT (dataset_id) DO NOTHING",
        )
        .bind(&plan.dataset_id)
        .bind(&req.feature_set_ref)
        .bind(&label_json)
        .execute(&self.pg)
        .await?;

        let (version,): (i32,) = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM dataset_versions WHERE dataset_id = $1",
        )
        .bind(&plan.dataset_id)
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
        .bind(&plan.dataset_id)
        .bind(version)
        .bind(&req.feature_set_ref)
        .bind(&instruments_json)
        .bind(realized_start)
        .bind(realized_end)
        .bind(&label_json)
        .bind(row_count)
        .bind(&plan.dataset_id)
        .bind(&parquet_uri)
        .execute(&self.pg)
        .await?;

        Ok(DatasetVersionRecord {
            dataset_version_id,
            dataset_id: plan.dataset_id.clone(),
            version,
            feature_set_ref: req.feature_set_ref.clone(),
            instruments: req.instruments.clone(),
            start: realized_start,
            end: realized_end,
            label_spec: label_json,
            row_count,
            content_hash: plan.dataset_id.clone(),
            parquet_uri,
            created_at: Utc::now(),
        })
    }

    /// Read, densify, featurize and encode. The one place a dataset's bytes are
    /// produced, so `materialize` and the reproducibility rebuild cannot drift.
    async fn build_frame(&self, plan: &DatasetPlan) -> Result<Built> {
        let req = &plan.req;
        let target_tf = <Timeframe as backtest::types::TimeframeExt>::from_key(&req.timeframe)
            .ok_or_else(|| anyhow::anyhow!("unknown timeframe: {}", req.timeframe))?;

        // PIT pull, resample, features, label -- accumulated columnar across all
        // requested instruments. `as_of = end`: no bar past the window's right
        // edge can enter the snapshot (ADR-0008, leakage-structural).
        let as_of = AsOf::from_datetime(req.end);
        // The master clock the sparse bars are densified onto (SPEC 2): the UTC
        // grid at the target bar period, anchored to the epoch so every
        // instrument lands on the same ticks.
        let step_ns = i64::try_from(
            <Timeframe as backtest::types::TimeframeExt>::seconds(&target_tf) * 1_000_000_000,
        )
        .unwrap_or(60_000_000_000);
        let mut acc = FrameAccumulator::new(plan.features.clone());
        let mut row_flags = QualityFlags::NONE;

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
                let mut kept: Vec<LoadedBar> = Vec::with_capacity(bars.len());
                for b in bars {
                    let f = b.quality_flags;
                    row_flags = row_flags.union(f);
                    if !f.excluded_by(req.quality_exclusion_mask) {
                        kept.push(b);
                    }
                }
                let frame = features::build_aligned_training_frame(
                    &kept.iter().map(backtest::warmup::bar_obs).collect::<Vec<_>>(),
                    &plan.features,
                    plan.horizon_bars,
                    step_ns,
                );
                acc.push(instrument, &frame);
            }
        }

        let parquet_bytes = acc.encode_parquet()?;
        let frame_digest = format!("sha256:{}", hex_sha256(&parquet_bytes));
        Ok(Built {
            acc,
            row_flags,
            parquet_bytes,
            frame_digest,
        })
    }

    /// Reconstruct the plan that produced a recorded dataset, from the request
    /// stored beside its digest and the spec stored under its id.
    ///
    /// The spec is taken **verbatim** rather than re-planned: re-planning would
    /// stamp today's `runtime_image_digest` and today's instrument resolution
    /// into it and therefore produce a different `dataset_id`, which is a
    /// different dataset (ADR-P1-02), not a rebuild of this one.
    ///
    /// # Errors
    /// The stored request or spec no longer deserializes, or the dataset was
    /// built by a different binary than the one running -- which this binary
    /// cannot reproduce and must not pretend to.
    pub fn replay(
        &self,
        tenant: &str,
        dataset_id: &str,
        spec_json: &serde_json::Value,
        request_json: &serde_json::Value,
    ) -> Result<DatasetPlan> {
        let spec: DatasetSpec = serde_json::from_value(spec_json.clone())
            .with_context(|| format!("stored spec for {dataset_id} no longer deserializes"))?;
        let req: DatasetRequest = serde_json::from_value(request_json.clone())
            .with_context(|| format!("stored request for {dataset_id} no longer deserializes"))?;
        let running = runtime_image_digest()?;
        if spec.runtime_image_digest != running {
            anyhow::bail!(
                "{dataset_id} was built by image {} and this process is {running}: a different \
                 binary cannot reproduce it, and claiming otherwise would make the check meaningless",
                spec.runtime_image_digest
            );
        }
        Ok(DatasetPlan {
            dataset_id: dataset_id.to_string(),
            features: features::resolve_feature_set(&req.feature_set_ref)
                .map(|fs| fs.features.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!("feature set {} no longer exists", req.feature_set_ref)
                })?,
            horizon_bars: u64::from(req.label_spec.horizon_bars),
            resolved: spec
                .instrument_ids
                .iter()
                .zip(&req.instruments)
                .map(|(k, sym)| (sym.clone(), k.0, 0))
                .collect(),
            spec,
            tenant: tenant.to_string(),
            req,
        })
    }

    /// The materialized frame a plan describes, as columns rather than bytes —
    /// what the leakage suite runs its frame-level screens over.
    ///
    /// # Errors
    /// A failed point-in-time read or Parquet encode.
    pub async fn training_frame(&self, plan: &DatasetPlan) -> Result<features::TrainingFrame> {
        Ok(self.build_frame(plan).await?.acc.into_training_frame())
    }

    /// Rebuild a dataset from its spec and report the digest of the bytes,
    /// writing nothing.
    ///
    /// This is the live half of snapshot reproducibility (SPEC 12.5.3): a digest
    /// that differs from the one recorded when the `dataset_id` was first
    /// materialized means the data under a fixed spec changed -- a retroactive
    /// price adjustment, a revised calendar, a vendor restatement.
    ///
    /// # Errors
    /// A failed point-in-time read or Parquet encode.
    pub async fn rebuild_digest(&self, plan: &DatasetPlan) -> Result<(String, i64)> {
        let built = self.build_frame(plan).await?;
        let rows = i64::try_from(built.acc.row_count()).unwrap_or(i64::MAX);
        Ok((built.frame_digest, rows))
    }

    /// The digest of the bytes a `dataset_id` produced, as first recorded.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn recorded_frame_digest(
        &self,
        tenant: &str,
        dataset_id: &str,
    ) -> Result<Option<String>> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant).await?;
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT frame_digest FROM dataplane.dataset_frame_digest WHERE dataset_id = $1",
        )
        .bind(dataset_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|(d,)| d))
    }

    /// Record the first digest a `dataset_id` produced. The row is immutable and
    /// the insert does nothing if one already exists: a later rebuild that
    /// disagrees is a finding, never a correction.
    async fn record_frame_digest(
        &self,
        plan: &DatasetPlan,
        frame_digest: &str,
        row_count: i64,
    ) -> Result<()> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, &plan.tenant).await?;
        sqlx::query(
            "INSERT INTO dataplane.dataset_frame_digest \
             (dataset_id, tenant_id, frame_digest, row_count, request) \
             VALUES ($1,$2,$3,$4,$5) ON CONFLICT (dataset_id) DO NOTHING",
        )
        .bind(&plan.dataset_id)
        .bind(&plan.tenant)
        .bind(frame_digest)
        .bind(row_count)
        .bind(serde_json::to_value(&plan.req)?)
        .execute(&mut *tx)
        .await
        .context("recording dataset frame digest")?;
        tx.commit().await?;
        Ok(())
    }

    /// Persist the label, split and dataset specs the plan resolved. All three
    /// tables refuse mutation, so re-materializing is a no-op rather than a
    /// redefinition.
    async fn register_specs(&self, plan: &DatasetPlan, uses_backfilled: bool) -> Result<()> {
        let req = &plan.req;
        let mut tx = ledger::pg::tenant_tx(&self.pg, &plan.tenant).await?;
        let l = &req.label_spec;
        sqlx::query(
            "INSERT INTO dataplane.label_spec \
             (label_spec_id, kind, horizon_bars, pt_sl_multiples, vol_estimator, \
              min_return_threshold, sample_weight_method, code_hash) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (label_spec_id) DO NOTHING",
        )
        .bind(&l.label_spec_id)
        .bind(enum_tag(&l.kind)?)
        .bind(i32::try_from(l.horizon_bars).unwrap_or(i32::MAX))
        .bind(if l.pt_sl_multiples.is_empty() {
            None
        } else {
            Some(l.pt_sl_multiples.clone())
        })
        .bind(l.vol_estimator.clone())
        .bind(l.min_return_threshold)
        .bind(enum_tag(&l.sample_weight_method)?)
        .bind(&l.code_hash)
        .execute(&mut *tx)
        .await
        .context("registering label_spec")?;

        let sp = &req.split_spec;
        let overrides = sp.overrides();
        let embargo_reason = overrides
            .iter()
            .find(|o| o.field == "embargo_bars")
            .map(|o| o.reason.clone());
        let purge_reason = overrides
            .iter()
            .find(|o| o.field == "purge_on")
            .map(|o| o.reason.clone());
        sqlx::query(
            "INSERT INTO dataplane.split_spec \
             (split_spec_id, kind, n_folds, n_test_groups, train_window, computed_embargo_bars, \
              embargo_bars, embargo_override_reason, purge_on, purge_override_reason, \
              min_train_bars, regime_stratified) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (split_spec_id) DO NOTHING",
        )
        .bind(&sp.split_spec_id)
        .bind(enum_tag(&sp.kind)?)
        .bind(i32::try_from(sp.n_folds).unwrap_or(i32::MAX))
        .bind(i32::try_from(sp.n_test_groups).unwrap_or(0))
        .bind(match sp.train_window {
            dataplane::split::TrainWindow::Expanding => "expanding".to_string(),
            dataplane::split::TrainWindow::Rolling(n) => format!("rolling:{n}"),
        })
        .bind(i32::try_from(sp.computed_embargo_bars()).unwrap_or(i32::MAX))
        .bind(i32::try_from(sp.embargo_bars()).unwrap_or(i32::MAX))
        .bind(embargo_reason)
        .bind(match sp.purge_on() {
            dataplane::split::PurgeOn::T1 => "t1",
            dataplane::split::PurgeOn::T0 => "t0",
        })
        .bind(purge_reason)
        .bind(i32::try_from(sp.min_train_bars).unwrap_or(i32::MAX))
        .bind(sp.regime_stratified)
        .execute(&mut *tx)
        .await
        .context("registering split_spec")?;

        let spec = &plan.spec;
        sqlx::query(
            "INSERT INTO dataplane.dataset_spec \
             (dataset_id, tenant_id, spec, instrument_ids, date_from, date_to, frequency, \
              feature_set_id, label_spec_id, split_spec_id, as_of_knowledge_time, \
              quality_exclusion_mask, calendar_versions, adjustment_policy, finality_policy, \
              runtime_image_digest, non_reproducible, uses_backfilled_knowledge) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18) \
             ON CONFLICT (dataset_id) DO NOTHING",
        )
        .bind(&plan.dataset_id)
        .bind(&plan.tenant)
        .bind(serde_json::to_value(spec)?)
        .bind(
            spec.instrument_ids
                .iter()
                .map(|k| k.0)
                .collect::<Vec<i64>>(),
        )
        .bind(spec.date_range.0)
        .bind(spec.date_range.1)
        .bind(&spec.frequency)
        .bind(&spec.feature_set_id)
        .bind(&spec.label_spec_id)
        .bind(&spec.split_spec_id)
        .bind(spec.as_of_knowledge_time)
        .bind(i64::from(spec.quality_exclusion_mask.0))
        .bind(serde_json::to_value(&spec.calendar_versions)?)
        .bind(match spec.adjustment_policy {
            AdjustmentPolicy::Unadjusted => "unadjusted",
            AdjustmentPolicy::SplitsOnly => "splits_only",
            AdjustmentPolicy::SplitsAndDividends => "splits_and_dividends",
        })
        .bind(
            spec.finality_policy
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
        )
        .bind(&spec.runtime_image_digest)
        .bind(spec.non_reproducible())
        .bind(uses_backfilled)
        .execute(&mut *tx)
        .await
        .context("registering dataset_spec")?;

        tx.commit().await?;
        Ok(())
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

/// One materialization's product, before anything is written.
struct Built {
    acc: FrameAccumulator,
    /// Union of the quality flags across every row read, including excluded
    /// ones -- `uses_backfilled_knowledge` is about what the span contains, not
    /// about what survived the mask.
    row_flags: QualityFlags,
    parquet_bytes: Vec<u8>,
    frame_digest: String,
}

/// The pinned calendars a dataset's instruments trade under. Calendars are
/// revised retroactively, so the version is part of the dataset hash (SPEC 2).
///
/// Every instrument this platform trades today is crypto spot, which is
/// continuous; a venue-listed instrument must add its exchange calendar here
/// before it can be part of a dataset.
fn calendar_versions_for(instruments: &[String]) -> Vec<CalendarVersion> {
    if instruments.is_empty() {
        Vec::new()
    } else {
        vec![CalendarVersion::crypto_24_7()]
    }
}

/// The `snake_case` serde tag of a unit enum variant, for the CHECK-constrained
/// text columns the spec tables use.
fn enum_tag<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_value(v)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("enum variant is not a string tag"))
}

/// Columnar accumulator across instruments, plus the Parquet encoder. Holds the
/// `ts_ns`, `instrument`, per-feature (with its two staleness companions), and
/// `label` columns of the whole snapshot.
struct FrameAccumulator {
    feature_names: Vec<String>,
    ts_ns: Vec<i64>,
    instrument: Vec<String>,
    columns: Vec<Vec<f64>>,
    /// `{feature}_age_minutes`, parallel to `columns` (INV-11).
    age_minutes: Vec<Vec<i64>>,
    /// `{feature}_quality`, parallel to `columns` (INV-11).
    quality: Vec<Vec<i64>>,
    label: Vec<f64>,
    /// Average-uniqueness weight per row (SPEC §3.4, ADR-P2-22). The trainer
    /// consumes this, which is what makes the label spec's declared
    /// `sample_weight_method = uniqueness` a fact rather than a claim.
    sample_weight: Vec<f64>,
}

impl FrameAccumulator {
    fn new(feature_names: Vec<String>) -> Self {
        let n = feature_names.len();
        Self {
            feature_names,
            ts_ns: Vec::new(),
            instrument: Vec::new(),
            columns: vec![Vec::new(); n],
            age_minutes: vec![Vec::new(); n],
            quality: vec![Vec::new(); n],
            label: Vec::new(),
            sample_weight: Vec::new(),
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
            // A frame built without alignment carries no weights; an unweighted
            // row is weight 1, which is what "no weighting" means.
            self.sample_weight.push(frame.sample_weight.get(r).copied().unwrap_or(1.0));
            for (i, src) in mapping.iter().enumerate() {
                let (v, age, q) = match src {
                    Some(idx) => (
                        frame.columns[*idx][r],
                        frame.age_minutes.get(*idx).map_or(0, |c| c[r]),
                        frame.quality.get(*idx).map_or(0, |c| i64::from(c[r])),
                    ),
                    None => (f64::NAN, 0, 0),
                };
                self.columns[i].push(v);
                self.age_minutes[i].push(age);
                self.quality[i].push(q);
            }
        }
    }

    fn row_count(&self) -> usize {
        self.ts_ns.len()
    }

    /// The accumulated snapshot as a frame, for callers that need the columns
    /// rather than the encoded bytes (the leakage suite). Rows from every
    /// instrument are stacked, which is what the screens want: a leak planted in
    /// one instrument's columns is still a leak.
    fn into_training_frame(self) -> features::TrainingFrame {
        features::TrainingFrame {
            feature_names: self.feature_names,
            ts_ns: self.ts_ns,
            columns: self.columns,
            sample_weight: self.sample_weight,
            age_minutes: self.age_minutes,
            quality: self
                .quality
                .into_iter()
                .map(|c| c.into_iter().map(|q| u32::try_from(q).unwrap_or(0)).collect())
                .collect(),
            label: self.label,
        }
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
            // A value is never emitted without its two companions (INV-11).
            let [value, age, quality] = features::align::companion_columns(name);
            fields.push(Field::new(value, DataType::Float64, true));
            fields.push(Field::new(age, DataType::Int64, false));
            fields.push(Field::new(quality, DataType::Int64, false));
        }
        fields.push(Field::new("label", DataType::Float64, false));
        fields.push(Field::new("sample_weight", DataType::Float64, false));
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
        for i in 0..self.feature_names.len() {
            arrays.push(Arc::new(Float64Array::from(self.columns[i].clone())));
            arrays.push(Arc::new(Int64Array::from(self.age_minutes[i].clone())));
            arrays.push(Arc::new(Int64Array::from(self.quality[i].clone())));
        }
        arrays.push(Arc::new(Float64Array::from(self.label.clone())));
        arrays.push(Arc::new(Float64Array::from(self.sample_weight.clone())));

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
    use dataplane::label::{LabelKind, SampleWeightMethod};
    use dataplane::split::{EmbargoInputs, SplitKind};

    // ------------------------------------------------------------------ //
    // INV-12: a dataset is a hash over its full spec
    // ------------------------------------------------------------------ //

    fn manager() -> DatasetManager {
        DatasetManager::with_parts(
            sqlx::PgPool::connect_lazy("postgres://invalid/invalid").expect("lazy pool"),
            None,
            Arc::from(storage::artifacts::from_env()),
        )
    }

    fn label(horizon_bars: u32, weighting: SampleWeightMethod) -> LabelSpec {
        LabelSpec {
            label_spec_id: String::new(),
            kind: LabelKind::HorizonReturn,
            horizon_bars,
            pt_sl_multiples: Vec::new(),
            vol_estimator: None,
            min_return_threshold: None,
            sample_weight_method: weighting,
            code_hash: "code".into(),
        }
        .content_keyed()
    }

    fn request(label_spec: LabelSpec) -> DatasetRequest {
        let horizon = label_spec.horizon_bars;
        DatasetRequest {
            universe_spec_id: "explicit:BTC-USD".into(),
            feature_set_ref: "fs_core_ohlcv_v3".into(),
            instruments: vec!["BTC-USD".into()],
            timeframe: "1m".into(),
            start: Utc::now() - chrono::Duration::days(30),
            end: Utc::now(),
            label_spec,
            split_spec: SplitSpec::new(
                String::new(),
                SplitKind::WalkForward,
                1,
                0,
                &EmbargoInputs {
                    horizon_bars: horizon,
                    max_lookback_bars: 0,
                    max_knowledge_lag_ms: 0,
                    settlement_lag_bars: 0,
                },
            )
            .content_keyed(),
            quality_exclusion_mask: default_exclusion_mask(),
            adjustment_policy: AdjustmentPolicy::Unadjusted,
            output_prefix: "./artifacts".into(),
        }
    }

    #[tokio::test]
    async fn identical_requests_produce_the_same_dataset_id() {
        let m = manager();
        let a = m
            .plan("t", request(label(60, SampleWeightMethod::None)))
            .await
            .expect("plan");
        let b = m
            .plan("t", request(label(60, SampleWeightMethod::None)))
            .await
            .expect("plan");
        // The requested window is `now`-relative, so only the parts that are
        // genuinely identical are compared here; the spec's own hash test covers
        // the date range.
        assert_eq!(a.spec.feature_set_id, b.spec.feature_set_id);
        assert_eq!(a.spec.label_spec_id, b.spec.label_spec_id);
        assert_eq!(a.spec.split_spec_id, b.spec.split_spec_id);
        assert_eq!(a.spec.runtime_image_digest, b.spec.runtime_image_digest);
    }

    /// A different label horizon is a different dataset, not a re-use.
    #[tokio::test]
    async fn label_spec_enters_the_dataset_id() {
        let m = manager();
        let mut req_a = request(label(60, SampleWeightMethod::None));
        let mut req_b = request(label(120, SampleWeightMethod::None));
        // Pin the window so only the label differs.
        req_b.start = req_a.start;
        req_b.end = req_a.end;
        req_a.split_spec = req_b.split_spec.clone();
        let a = m.plan("t", req_a).await.expect("plan");
        let b = m.plan("t", req_b).await.expect("plan");
        assert_ne!(a.dataset_id, b.dataset_id);
    }

    /// The weighting declaration is what makes an unweighted overlap visible in
    /// every comparison (SPEC 3.4).
    #[tokio::test]
    async fn unweighted_overlapping_labels_are_flagged_on_the_plan() {
        let m = manager();
        let unweighted = m
            .plan("t", request(label(60, SampleWeightMethod::None)))
            .await
            .expect("plan");
        assert!(unweighted.overlapping_labels_unweighted());

        let weighted = m
            .plan("t", request(label(60, SampleWeightMethod::Uniqueness)))
            .await
            .expect("plan");
        assert!(!weighted.overlapping_labels_unweighted());
    }

    /// A one-bar horizon has no overlap to weight, so `none` is not a flag.
    #[tokio::test]
    async fn non_overlapping_labels_are_not_flagged() {
        let m = manager();
        let p = m
            .plan("t", request(label(1, SampleWeightMethod::None)))
            .await
            .expect("plan");
        assert!(!p.overlapping_labels_unweighted());
    }

    /// The image digest is REQUIRED and real: it is the hash of this test
    /// binary, not a constant.
    #[tokio::test]
    async fn the_runtime_image_digest_is_required_and_real() {
        let d = runtime_image_digest().expect("digest");
        assert!(d.starts_with("sha256:"));
        assert_eq!(d.len(), "sha256:".len() + 64);
        let p = m_digest().await;
        assert_eq!(p, d, "the plan carries the running image, not a placeholder");
    }

    async fn m_digest() -> String {
        manager()
            .plan("t", request(label(60, SampleWeightMethod::None)))
            .await
            .expect("plan")
            .spec
            .runtime_image_digest
    }

    /// A spec that names no calendar version cannot be reproduced across a
    /// calendar revision, so a non-empty universe always pins one.
    #[tokio::test]
    async fn every_instrument_universe_pins_a_calendar_version() {
        let p = manager()
            .plan("t", request(label(60, SampleWeightMethod::None)))
            .await
            .expect("plan");
        assert!(!p.spec.calendar_versions.is_empty());
    }

    /// The feature set enters the hash as its versioned DAG hash, so a changed
    /// implementation cannot silently reuse an older snapshot.
    #[tokio::test]
    async fn the_feature_set_enters_the_hash_as_a_version_hash_not_a_name() {
        let p = manager()
            .plan("t", request(label(60, SampleWeightMethod::None)))
            .await
            .expect("plan");
        assert!(p.spec.feature_set_id.starts_with("sha256:"));
        assert_ne!(p.spec.feature_set_id, "fs_core_ohlcv_v3");
    }


    /// 400 bars, not 30. Features are windowed (ADR-P0-19) and produce no value
    /// until their full declared window exists — `ema_7` needs 5·7 bars, `rsi_N`
    /// needs 5·N+1 — so a short fixture yields an empty frame. Lengthen the
    /// fixture; never reintroduce partial-window values.
    fn frame(names: &[&str]) -> features::TrainingFrame {
        frame_named(&contiguous_bars(400), names)
    }

    fn contiguous_bars(n: i32) -> Vec<features::BarObs> {
        (0..n).map(|i| obs(i, 100.0 + f64::from(i))).collect()
    }

    fn obs(minute: i32, close: f64) -> features::BarObs {
        let ts = i64::from(minute) * 60_000_000_000;
        features::BarObs {
            ts_ns: ts,
            knowledge_ns: ts,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            quality: QualityFlags::NONE,
        }
    }

    fn frame_named(bars: &[features::BarObs], names: &[&str]) -> features::TrainingFrame {
        let feats: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        features::build_aligned_training_frame(bars, &feats, 1, 60_000_000_000)
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
        // ts_ns + instrument + 2 features x (value, age, quality) + label + weight
        assert_eq!(cols, 10);
    }

    // ------------------------------------------------------------------ //
    // AT-11 / INV-11: every aligned feature emits its staleness companions
    // ------------------------------------------------------------------ //

    /// A cross-asset snapshot spanning a market holiday (one asset silent) and a
    /// crypto weekend (the other trading straight through). Every feature column
    /// carries `_age_minutes` and `_quality`, and the gap is visible in the age
    /// rather than closed silently.
    #[test]
    fn a_cross_asset_frame_exposes_staleness_across_a_holiday_and_a_weekend() {
        // "Equity": trades for 200 minutes, then a 600-minute holiday, then
        // resumes. "Crypto": continuous across the whole span.
        let equity: Vec<features::BarObs> = (0..200)
            .map(|i| obs(i, 100.0 + f64::from(i)))
            .chain((800..1000).map(|i| obs(i, 100.0 + f64::from(i))))
            .collect();
        let crypto = contiguous_bars(1000);

        let names = vec!["close".to_string(), "ema_7".to_string()];
        let mut acc = FrameAccumulator::new(names.clone());
        acc.push("EQ-USD", &frame_named(&equity, &["close", "ema_7"]));
        let eq_rows = acc.row_count();
        acc.push("BTC-USD", &frame_named(&crypto, &["close", "ema_7"]));

        assert!(eq_rows > 0 && acc.row_count() > eq_rows);

        // Companions exist for every feature, one value per row.
        assert_eq!(acc.age_minutes.len(), names.len());
        assert_eq!(acc.quality.len(), names.len());
        for i in 0..names.len() {
            assert_eq!(acc.age_minutes[i].len(), acc.row_count());
            assert_eq!(acc.quality[i].len(), acc.row_count());
        }

        // The holiday is carried forward with its age exposed, and the carried
        // rows are flagged INTERPOLATED rather than passing as observations.
        let eq_ages = &acc.age_minutes[0][..eq_rows];
        assert!(
            eq_ages.iter().any(|a| *a > 60),
            "the holiday shows up as a stale value, not a closed gap"
        );
        let interpolated = QualityFlags::INTERPOLATED.0;
        assert!(
            acc.quality[0][..eq_rows]
                .iter()
                .any(|q| u32::try_from(*q).unwrap_or(0) & interpolated != 0),
            "a carried value says it was carried"
        );

        // The continuously-trading asset is never stale.
        assert!(
            acc.age_minutes[0][eq_rows..].iter().all(|a| *a == 0),
            "a 24/7 series has no staleness to report"
        );
    }

    /// A feature whose window spans the gap inherits the gap's quality, because
    /// it averaged over it.
    #[test]
    fn a_window_spanning_a_gap_inherits_its_quality() {
        let gappy: Vec<features::BarObs> = (0..200)
            .map(|i| obs(i, 100.0))
            .chain((260..400).map(|i| obs(i, 100.0)))
            .collect();
        let f = frame_named(&gappy, &["ema_7"]);
        let interpolated = QualityFlags::INTERPOLATED.0;
        assert!(
            f.quality[0].iter().any(|q| q & interpolated != 0),
            "ema_7's window covered carried rows"
        );
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

    // ------------------------------------------------------------------ //
    // I-0.6: Pinned snapshot immutability
    // ------------------------------------------------------------------ //

    /// Same parameters + same data ⇒ identical hash (idempotency key).
    #[test]
    fn same_data_produces_same_hash() {
        let names = vec!["close".to_string()];
        let build_hash = || {
            let mut acc = FrameAccumulator::new(names.clone());
            acc.push("BTC-USD", &frame(&["close"]));
            let bytes = acc.encode_parquet().expect("encode");
            let param = b"params";
            let mut input = param.to_vec();
            input.extend_from_slice(&bytes);
            hex_sha256(&input)
        };
        assert_eq!(
            build_hash(),
            build_hash(),
            "identical params+data ⇒ identical hash"
        );
    }

    /// Different data (a different instrument or bar range) produces a
    /// different hash, ensuring late-data revisions yield a new snapshot
    /// version rather than silently mutating the existing one (I-0.6).
    #[test]
    fn different_data_produces_different_hash() {
        let names = vec!["close".to_string()];
        let hash_for = |instrument: &str| {
            let mut acc = FrameAccumulator::new(names.clone());
            // Use the same feature frame but tag it to a different instrument
            // so the `instrument` column differs → different Parquet bytes →
            // different hash → a separate dataset_version row would be inserted.
            acc.push(instrument, &frame(&["close"]));
            let bytes = acc.encode_parquet().expect("encode");
            let param = b"params";
            let mut input = param.to_vec();
            input.extend_from_slice(&bytes);
            hex_sha256(&input)
        };
        assert_ne!(
            hash_for("BTC-USD"),
            hash_for("ETH-USD"),
            "different data (instrument tag changes Parquet bytes) ⇒ different hash ⇒ new snapshot version"
        );
    }

    /// Snapshots are immutable by construction: the INSERT has no ON CONFLICT
    /// UPDATE clause, so re-running with the same hash returns the existing
    /// row untouched (the `find_by_hash` early-return path in `materialize`).
    /// This test verifies the hash logic that drives that idempotency key.
    #[test]
    fn hash_covers_both_params_and_bytes() {
        let names = vec!["close".to_string()];
        let mut acc = FrameAccumulator::new(names.clone());
        acc.push("BTC-USD", &frame(&["close"]));
        let bytes = acc.encode_parquet().expect("encode");

        // Same bytes, different param string → different hash.
        let h1 = {
            let mut input = b"params_v1".to_vec();
            input.extend_from_slice(&bytes);
            hex_sha256(&input)
        };
        let h2 = {
            let mut input = b"params_v2".to_vec();
            input.extend_from_slice(&bytes);
            hex_sha256(&input)
        };
        assert_ne!(h1, h2, "param change alone must change the hash");

        // Same param, different bytes (different accumulator content) → different hash.
        let mut acc2 = FrameAccumulator::new(names);
        acc2.push("ETH-USD", &frame(&["close"]));
        let bytes2 = acc2.encode_parquet().expect("encode");
        let h3 = {
            let mut input = b"params_v1".to_vec();
            input.extend_from_slice(&bytes2);
            hex_sha256(&input)
        };
        assert_ne!(h1, h3, "data change alone must change the hash");
    }
}
