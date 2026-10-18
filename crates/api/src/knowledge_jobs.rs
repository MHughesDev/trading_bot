//! Writers for the knowledge plane (SPEC §5.5–§5.6, checklist 3.7/3.10,
//! ADR-P3-03/04).
//!
//! Two things the knowledge plane needs that nothing else produces: the outcome
//! tensor's facts, projected out of the ledger, and the insight store's
//! retrieval path.
//!
//! ## 3.7 — the tensor is a projection, not a second record
//!
//! `knowledge.outcome_tensor_fact` is derived entirely from `mlops.trial` and
//! its events. It is rebuilt rather than maintained, and rebuilding it is
//! idempotent, because a derived table that drifts from its source is worse than
//! no derived table — it is a second answer to the same question with no way to
//! tell which one is stale.
//!
//! **The propensity and the censoring travel with every fact.** Not as metadata:
//! they are the two inputs the eventual completion model needs. Missingness here
//! is informative — the platform did not try things it expected to fail — and a
//! factorization that ignores that estimates the value of what the policy
//! already liked. The censoring is the same point one level down: a stopped
//! trial is an observation with a bound, and dropping it biases every cell it
//! would have landed in.
//!
//! ## Scheduling and the insight store's life cycle (OPEN-TASKS C1.3, C1.5)
//!
//! [`TensorProjection::rebuild`] and the insight decay below were complete and
//! had **no caller**: nothing ran them, so the tensor was empty and no insight
//! ever decayed. [`spawn_timers`] is the caller. The insight store additionally
//! gains its missing halves: evidence-backed `confirm` and `contradict` (a
//! contradiction is a claim about the world and carries the same REQUIRED
//! evidence an insight does, in `knowledge.insight_event`, migration 0183); a
//! `decay_score` that is a pure function of age since last confirmation and the
//! contradiction count, so a sweep is idempotent; a local embedder
//! ([`OllamaEmbedder`], `nomic-embed-text`, 768 dimensions, over the claim text
//! only); and the retrieval/write endpoints ([`search_insights`] and friends).
//!
//! ## 3.10 — retrieval is capped server-side
//!
//! §15 gives the agent a 4 000-token budget for retrieved insights, and
//! [`InsightStore::search`] enforces it **here** rather than trusting the
//! caller to ask for few enough. A cap the client applies is a cap that is off
//! whenever a new client appears.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{scopes, BearerToken};
use crate::state::AppState;

// The `InstrumentProfile` worker feeds `knowledge.asset_embedding`, the other
// half of this module's subject, and is declared here so the knowledge plane's
// writers read as one unit. The file is `src/instrument_profile_worker.rs`.
#[path = "instrument_profile_worker.rs"]
pub mod instrument_profile_worker;

/// The §15 retrieval budget, in tokens.
pub const INSIGHT_TOKEN_CAP: usize = 4_000;

/// Rough tokens per character. Four is the usual English approximation; it is
/// deliberately a *floor* on the estimate — undercounting the cap's consumption
/// would let the budget be exceeded, which is the direction that matters.
const CHARS_PER_TOKEN: usize = 4;

/// How many trials one rebuild pass reads. Bounded so a tenant with a long
/// ledger cannot hold a transaction open for minutes.
const REBUILD_BATCH: i64 = 5_000;

// ───────────────────────────────────────────────────────────────────────────────
// 3.7 — the outcome tensor
// ───────────────────────────────────────────────────────────────────────────────

/// Projects ledger outcomes into `knowledge.outcome_tensor_fact`.
pub struct TensorProjection {
    pg: PgPool,
}

/// What one rebuild pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProjectionReport {
    pub facts_written: u64,
    /// Trials that produced no fact because they carry no metric — a failure, a
    /// deduplication, a trial still running. Counted rather than ignored: the
    /// ratio is how you notice the projection quietly covering less than it did.
    pub skipped_without_metric: u64,
    /// Facts whose censoring is not `none`. Reported because a tensor that is
    /// mostly censored observations is a tensor the completion model must be
    /// told about (AT-45).
    pub censored: u64,
}

impl TensorProjection {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Rebuild the tenant's facts from the ledger.
    ///
    /// Idempotent: the primary key is `(tenant, trial, metric)` and a re-run
    /// upserts. That is what lets this be a projection rather than a log — it
    /// can always be thrown away and rebuilt, and so it can never disagree with
    /// its source for long.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn rebuild(&self, tenant_id: &str) -> Result<ProjectionReport, sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

        // `mlops.trial_state.outcome` is a composite (`mlops.outcome_vector`), and
        // this read it as a JSONB column named `outcome_vector` that the view has
        // never had. The projection failed on its first row and, having no caller,
        // never failed anywhere anybody would see (found 2026-10-03 when the
        // timer was added and the query was first run).
        //
        // Everything the tensor's coordinates need, straight from the ledger.
        // The regime and cluster coordinates are 0 until 3.5's labels and 3.6's
        // clustering exist: a fact in the "unknown regime" cell is still a fact,
        // and refusing to write one until the clustering lands would leave the
        // completion model with nothing to be fitted on when it arrives.
        //
        // The outcome comes off the trial's latest event rather than the
        // `mlops.trial_state` view: the view is the same latest-event join, but it
        // drops `censor_at_step`, which is half of the censored-regression input
        // this table exists to carry (§5.5, INV-17). And `knowledge_time` is when
        // that event settled the trial, not when the projection happened to run
        // (INV-01): stamping `now()` also made every rebuild rewrite every row, so
        // a re-run was not the no-op a projection should be.
        let written = sqlx::query(
            "INSERT INTO knowledge.outcome_tensor_fact
                 (tenant_id, asset_cluster, venue_id, regime_id, strategy_family, config_hash,
                  trial_id, metric_name, metric_value, propensity, censoring, censor_at_step,
                  knowledge_time)
             SELECT t.tenant_id,
                    0 AS asset_cluster,
                    0 AS venue_id,
                    0 AS regime_id,
                    coalesce(t.code_hash, 'unknown') AS strategy_family,
                    t.config_hash,
                    t.trial_id,
                    'sharpe_net' AS metric_name,
                    (e.outcome).sharpe_net,
                    t.propensity,
                    coalesce(e.censoring, 'none'),
                    e.censor_at_step,
                    e.occurred_at
             FROM mlops.trial t
             JOIN LATERAL (
                 SELECT ev.outcome, ev.censoring, ev.censor_at_step, ev.occurred_at
                 FROM mlops.trial_event ev
                 WHERE ev.trial_id = t.trial_id
                 ORDER BY ev.event_seq DESC
                 LIMIT 1
             ) e ON true
             WHERE t.tenant_id = $1
               AND (e.outcome).sharpe_net IS NOT NULL
             ORDER BY t.registered_at
             LIMIT $2
             ON CONFLICT (tenant_id, trial_id, metric_name) DO UPDATE
               SET metric_value   = EXCLUDED.metric_value,
                   propensity     = EXCLUDED.propensity,
                   censoring      = EXCLUDED.censoring,
                   censor_at_step = EXCLUDED.censor_at_step,
                   knowledge_time = EXCLUDED.knowledge_time",
        )
        .bind(tenant_id)
        .bind(REBUILD_BATCH)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        let (skipped,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM mlops.trial t
             JOIN mlops.trial_state s ON s.trial_id = t.trial_id
             WHERE t.tenant_id = $1
               AND (s.outcome IS NULL OR (s.outcome).sharpe_net IS NULL)",
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        let (censored,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM knowledge.outcome_tensor_fact
             WHERE tenant_id = $1 AND censoring <> 'none'",
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(ProjectionReport {
            facts_written: written,
            skipped_without_metric: u64::try_from(skipped).unwrap_or(0),
            censored: u64::try_from(censored).unwrap_or(0),
        })
    }

    /// Whether there is enough in the tensor to fit the completion model
    /// (ADR-P3-03).
    ///
    /// Two conditions, both from the reference: ≥ 500 trials **and**
    /// ≥ 20 instruments. A factorization of a few hundred cells over three
    /// instruments is noise with a version number, and shipping one would make
    /// every downstream recommendation look model-backed.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn ready_to_fit(&self, tenant_id: &str) -> Result<(bool, String), sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let (trials, instruments): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(DISTINCT venue_id)
             FROM knowledge.outcome_tensor_fact WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(fit_readiness(trials, instruments))
    }
}

/// ADR-P3-03's thresholds.
pub const MIN_TRIALS_TO_FIT: i64 = 500;
pub const MIN_INSTRUMENTS_TO_FIT: i64 = 20;

/// Whether the tensor can be fitted, and what to say if not.
///
/// Split out so the arithmetic is testable without a database — the thresholds
/// are the load-bearing part, not the query.
#[must_use]
pub fn fit_readiness(trials: i64, instruments: i64) -> (bool, String) {
    if trials >= MIN_TRIALS_TO_FIT && instruments >= MIN_INSTRUMENTS_TO_FIT {
        return (
            true,
            format!("{trials} trials over {instruments} instruments"),
        );
    }
    (
        false,
        format!(
            "not fitted: {trials} of {MIN_TRIALS_TO_FIT} trials, {instruments} of \
             {MIN_INSTRUMENTS_TO_FIT} instruments. A factorization of this is noise with a \
             version number, so the recommender answers from its rule tier"
        ),
    )
}

// ───────────────────────────────────────────────────────────────────────────────
// 3.10 — the insight store
// ───────────────────────────────────────────────────────────────────────────────

/// One remembered claim.
#[derive(Debug, Clone, Serialize)]
pub struct Insight {
    pub insight_id: uuid::Uuid,
    pub tier: i16,
    pub claim: String,
    pub scope: serde_json::Value,
    pub evidence_trial_ids: Vec<uuid::Uuid>,
    pub support_n: i32,
    pub contradicted_n: i32,
    pub decay_score: f32,
}

impl Insight {
    /// Roughly how much of the agent's budget this claim costs to read.
    #[must_use]
    pub fn token_cost(&self) -> usize {
        self.claim.len().div_ceil(CHARS_PER_TOKEN)
    }
}

/// Reads and writes `knowledge.insight` — AGENT-003's durable memory and the
/// pack's insight table, one store under two names (ADR-P3-04).
pub struct InsightStore {
    pg: PgPool,
}

impl InsightStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Retrieve insights for a scope, capped at [`INSIGHT_TOKEN_CAP`].
    ///
    /// The cap is applied **here**. A cap the caller applies is a cap that is off
    /// the moment a new caller appears, and the failure is silent: the agent's
    /// context fills with remembered claims and the actual task gets squeezed.
    ///
    /// Ordered by decay score, so a claim the platform has stopped confirming
    /// falls out of the budget before a live one does.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn search(
        &self,
        tenant_id: &str,
        tier: Option<i16>,
        limit: i64,
    ) -> Result<(Vec<Insight>, usize), sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let rows = sqlx::query_as::<
            _,
            (
                uuid::Uuid,
                i16,
                String,
                serde_json::Value,
                Vec<uuid::Uuid>,
                i32,
                i32,
                f32,
            ),
        >(
            "SELECT insight_id, tier, claim, scope, evidence_trial_ids, support_n,
                    contradicted_n, decay_score
             FROM knowledge.insight
             WHERE tenant_id = $1 AND ($2::smallint IS NULL OR tier = $2)
             ORDER BY decay_score DESC, last_confirmed_at DESC NULLS LAST
             LIMIT $3",
        )
        .bind(tenant_id)
        .bind(tier)
        .bind(limit.clamp(1, 500))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let all: Vec<Insight> = rows
            .into_iter()
            .map(
                |(
                    insight_id,
                    tier,
                    claim,
                    scope,
                    evidence_trial_ids,
                    support_n,
                    contradicted_n,
                    decay_score,
                )| {
                    Insight {
                        insight_id,
                        tier,
                        claim,
                        scope,
                        evidence_trial_ids,
                        support_n,
                        contradicted_n,
                        decay_score,
                    }
                },
            )
            .collect();
        Ok(apply_token_cap(all))
    }

    /// Mint an insight. Evidence is REQUIRED, and it has to be real: every id
    /// must be a trial this tenant ran.
    ///
    /// The CHECK on the table already refuses an empty list; this refuses a list
    /// of ids that are not trials, which the CHECK cannot see and which is the
    /// same rumour in a better disguise.
    ///
    /// # Errors
    /// [`InsightError::UnknownEvidence`] for an id the ledger does not hold, and
    /// backend failures.
    pub async fn create(
        &self,
        tenant_id: &str,
        tier: i16,
        scope: &serde_json::Value,
        claim: &str,
        evidence_trial_ids: &[Uuid],
    ) -> Result<Uuid, InsightError> {
        if claim.trim().is_empty() {
            return Err(InsightError::Invalid("a claim cannot be empty".into()));
        }
        if !(1..=3).contains(&tier) {
            return Err(InsightError::Invalid("tier is 1, 2 or 3".into()));
        }
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id).await?;
        require_trials(&mut tx, tenant_id, evidence_trial_ids).await?;
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO knowledge.insight (tenant_id, tier, scope, claim, evidence_trial_ids, support_n)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING insight_id",
        )
        .bind(tenant_id)
        .bind(tier)
        .bind(scope)
        .bind(claim)
        .bind(evidence_trial_ids)
        .bind(i32::try_from(evidence_trial_ids.len()).unwrap_or(i32::MAX))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Record that the platform saw the claim hold again, with the trials that
    /// showed it. Resets the staleness clock; contradictions still count.
    ///
    /// # Errors
    /// Unknown insight or evidence, and backend failures.
    pub async fn confirm(
        &self,
        tenant_id: &str,
        insight_id: Uuid,
        evidence_trial_ids: &[Uuid],
        note: Option<&str>,
    ) -> Result<f32, InsightError> {
        self.record(
            tenant_id,
            insight_id,
            InsightEventKind::Confirm,
            evidence_trial_ids,
            note,
        )
        .await
    }

    /// Record that the claim was contradicted, with the trials that contradict
    /// it, decaying it.
    ///
    /// Decay rather than deletion: "we believed this and stopped" is a fact
    /// worth keeping, and a claim that vanishes when contradicted can be
    /// rediscovered and re-believed indefinitely.
    ///
    /// # Errors
    /// Unknown insight or evidence, and backend failures.
    pub async fn contradict(
        &self,
        tenant_id: &str,
        insight_id: Uuid,
        evidence_trial_ids: &[Uuid],
        note: Option<&str>,
    ) -> Result<f32, InsightError> {
        self.record(
            tenant_id,
            insight_id,
            InsightEventKind::Contradict,
            evidence_trial_ids,
            note,
        )
        .await
    }

    async fn record(
        &self,
        tenant_id: &str,
        insight_id: Uuid,
        kind: InsightEventKind,
        evidence_trial_ids: &[Uuid],
        note: Option<&str>,
    ) -> Result<f32, InsightError> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id).await?;
        require_trials(&mut tx, tenant_id, evidence_trial_ids).await?;
        let row: Option<(
            chrono::DateTime<chrono::Utc>,
            Option<chrono::DateTime<chrono::Utc>>,
            i32,
            i32,
        )> = sqlx::query_as(
            "SELECT created_at, last_confirmed_at, contradicted_n, support_n
               FROM knowledge.insight WHERE tenant_id = $1 AND insight_id = $2 FOR UPDATE",
        )
        .bind(tenant_id)
        .bind(insight_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (created_at, last_confirmed, mut contradicted, mut support) =
            row.ok_or(InsightError::NotFound)?;
        let now = chrono::Utc::now();
        let mut confirmed_at = last_confirmed;
        match kind {
            InsightEventKind::Confirm => {
                support += 1;
                confirmed_at = Some(now);
            }
            InsightEventKind::Contradict => contradicted += 1,
        }
        let score = decayed_score(age_days(created_at, confirmed_at, now), contradicted);
        sqlx::query(
            "INSERT INTO knowledge.insight_event (tenant_id, insight_id, kind, evidence_trial_ids, note)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(tenant_id)
        .bind(insight_id)
        .bind(kind.as_str())
        .bind(evidence_trial_ids)
        .bind(note)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE knowledge.insight
                SET support_n = $3, contradicted_n = $4, last_confirmed_at = $5, decay_score = $6
              WHERE tenant_id = $1 AND insight_id = $2",
        )
        .bind(tenant_id)
        .bind(insight_id)
        .bind(support)
        .bind(contradicted)
        .bind(confirmed_at)
        .bind(score)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(score)
    }

    /// Recompute every insight's `decay_score` from its age and contradictions.
    ///
    /// Idempotent: the score is a pure function of its inputs ([`decayed_score`]),
    /// so running the sweep twice in a row changes nothing the second time. That
    /// is why it is a recomputation rather than "multiply by 0.99 each night",
    /// which would make the score depend on how often the timer fired.
    ///
    /// Returns how many rows changed.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn decay_sweep(&self, tenant_id: &str) -> Result<u64, sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let rows: Vec<(
            Uuid,
            chrono::DateTime<chrono::Utc>,
            Option<chrono::DateTime<chrono::Utc>>,
            i32,
            f32,
        )> = sqlx::query_as(
            "SELECT insight_id, created_at, last_confirmed_at, contradicted_n, decay_score
               FROM knowledge.insight WHERE tenant_id = $1 LIMIT 50000",
        )
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await?;
        let now = chrono::Utc::now();
        let mut changed = 0;
        for (id, created, confirmed, contradicted, current) in rows {
            let next = decayed_score(age_days(created, confirmed, now), contradicted);
            if (next - current).abs() > 1e-4 {
                sqlx::query(
                    "UPDATE knowledge.insight SET decay_score = $3
                      WHERE tenant_id = $1 AND insight_id = $2",
                )
                .bind(tenant_id)
                .bind(id)
                .bind(next)
                .execute(&mut *tx)
                .await?;
                changed += 1;
            }
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Embed the claims that have no embedding yet.
    ///
    /// **Over the claim text only** (SPEC §5.6): an embedding of a metric is a
    /// similarity measure nobody designed. A claim the embedder cannot be
    /// reached for stays unembedded and is retried on the next pass; retrieval
    /// still works without it, ordered by decay.
    ///
    /// Returns `(embedded, still_missing)`.
    ///
    /// # Errors
    /// Backend failures. An unreachable embedder is not one: it is reported as
    /// the claims left missing.
    pub async fn embed_missing(
        &self,
        tenant_id: &str,
        embedder: &dyn Embedder,
        limit: i64,
    ) -> Result<(u64, u64), sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let model = embedder.model_name();
        // Unembedded claims, and claims embedded by a different model: a model change
        // re-embeds, so a vector is only ever compared with vectors of its own model.
        let todo: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT insight_id, claim FROM knowledge.insight
              WHERE tenant_id = $1 AND (embedding IS NULL OR embedding_model IS DISTINCT FROM $3)
              ORDER BY created_at LIMIT $2",
        )
        .bind(tenant_id)
        .bind(limit.clamp(1, 500))
        .bind(&model)
        .fetch_all(&mut *tx)
        .await?;
        let mut done = 0_u64;
        for (id, claim) in &todo {
            match embedder.embed(claim).await {
                Ok(v) => match vector_literal(&v) {
                    Ok(lit) => {
                        sqlx::query(
                            "UPDATE knowledge.insight
                                SET embedding = $3::vector, embedding_model = $4, embedding_dim = $5
                              WHERE tenant_id = $1 AND insight_id = $2",
                        )
                        .bind(tenant_id)
                        .bind(id)
                        .bind(lit)
                        .bind(&model)
                        .bind(i32::try_from(v.len()).unwrap_or(i32::MAX))
                        .execute(&mut *tx)
                        .await?;
                        done += 1;
                    }
                    Err(why) => tracing::warn!(insight = %id, %why, "embedding refused"),
                },
                Err(why) => {
                    tracing::warn!(%why, "the embedder is unavailable; the remaining claims stay unembedded");
                    break;
                }
            }
        }
        tx.commit().await?;
        Ok((done, (todo.len() as u64).saturating_sub(done)))
    }

    /// Retrieve insights nearest a query, weighted by how alive each is, capped
    /// at [`INSIGHT_TOKEN_CAP`].
    ///
    /// Ranking is `decay_score x similarity`, so a stale claim loses to a live
    /// one even when it reads closer. Rows with no embedding yet rank by decay
    /// alone, after those that have one.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn search_semantic(
        &self,
        tenant_id: &str,
        query_embedding: &[f32],
        tier: Option<i16>,
        limit: i64,
    ) -> Result<(Vec<Insight>, usize), InsightError> {
        self.search_semantic_for_model(tenant_id, query_embedding, tier, limit, None)
            .await
    }

    /// [`Self::search_semantic`] against the vectors of one embedding model: a vector made
    /// by another model is not comparable with the query's and is ranked as unembedded
    /// (KM-06). `None` compares every vector, which is only right when one model has
    /// ever been used.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn search_semantic_for_model(
        &self,
        tenant_id: &str,
        query_embedding: &[f32],
        tier: Option<i16>,
        limit: i64,
        model: Option<&str>,
    ) -> Result<(Vec<Insight>, usize), InsightError> {
        let lit = vector_literal(query_embedding).map_err(InsightError::Invalid)?;
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id).await?;
        let rows: Vec<InsightRow> = sqlx::query_as(
            "SELECT insight_id, tier, claim, scope, evidence_trial_ids, support_n,
                    contradicted_n, decay_score
               FROM knowledge.insight
              WHERE tenant_id = $1 AND ($2::smallint IS NULL OR tier = $2)
              ORDER BY (CASE WHEN embedding IS NULL OR ($5::text IS NOT NULL AND embedding_model IS DISTINCT FROM $5) THEN 0.0
                             ELSE decay_score * (1.0 - (embedding <=> $4::vector)) END) DESC,
                       decay_score DESC
              LIMIT $3",
        )
        .bind(tenant_id)
        .bind(tier)
        .bind(limit.clamp(1, 500))
        .bind(lit)
        .bind(model)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(apply_token_cap(rows.into_iter().map(Into::into).collect()))
    }
}

type InsightRow = (Uuid, i16, String, serde_json::Value, Vec<Uuid>, i32, i32, f32);

impl From<InsightRow> for Insight {
    fn from(r: InsightRow) -> Self {
        Insight {
            insight_id: r.0,
            tier: r.1,
            claim: r.2,
            scope: r.3,
            evidence_trial_ids: r.4,
            support_n: r.5,
            contradicted_n: r.6,
            decay_score: r.7,
        }
    }
}

/// What can go wrong with an insight write.
#[derive(Debug, thiserror::Error)]
pub enum InsightError {
    #[error("{0}")]
    Invalid(String),
    #[error("no such insight")]
    NotFound,
    #[error("evidence names {0} trial(s) this tenant did not run; a remembered claim with no trial behind it is a rumour")]
    UnknownEvidence(usize),
    #[error("{0}")]
    Backend(#[from] sqlx::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsightEventKind {
    Confirm,
    Contradict,
}

impl InsightEventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Contradict => "contradict",
        }
    }
}

/// Every id must be a trial the tenant ran.
async fn require_trials(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    ids: &[Uuid],
) -> Result<(), InsightError> {
    if ids.is_empty() {
        return Err(InsightError::Invalid(
            "evidence is required: name at least one trial".into(),
        ));
    }
    let (found,): (i64,) = sqlx::query_as(
        "SELECT count(DISTINCT trial_id) FROM mlops.trial
          WHERE tenant_id = $1 AND trial_id = ANY($2)",
    )
    .bind(tenant_id)
    .bind(ids)
    .fetch_one(&mut **tx)
    .await?;
    let distinct = ids.iter().collect::<std::collections::BTreeSet<_>>().len();
    let missing = distinct.saturating_sub(usize::try_from(found).unwrap_or(0));
    if missing > 0 {
        return Err(InsightError::UnknownEvidence(missing));
    }
    Ok(())
}

/// Half-life of an unconfirmed claim, in days.
///
/// SPEC §5.6: a claim whose support is not reconfirmed decays out of retrieval.
/// The spec says "within its scope's regime"; a regime is not a duration, so
/// this build approximates it with one (ADR-C-05) and states so. 90 days is a
/// quarter, the horizon over which a volatility regime has usually turned.
pub const HALF_LIFE_DAYS: f64 = 90.0;

/// Days since a claim was last supported: its last confirmation, else its birth.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn age_days(
    created_at: chrono::DateTime<chrono::Utc>,
    last_confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> f64 {
    let since = last_confirmed_at.unwrap_or(created_at).max(created_at);
    ((now - since).num_seconds().max(0)) as f64 / 86_400.0
}

/// A claim's score: `0.5^(age / half_life) x 0.5^contradicted`, in `[0, 1]`.
///
/// A pure function of its two inputs, which is what makes the sweep idempotent
/// and the contradiction path agree with it. Never reaches exactly zero by
/// decay alone, and a fresh uncontradicted claim scores 1.0.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn decayed_score(age_days: f64, contradicted_n: i32) -> f32 {
    let time = 0.5_f64.powf(age_days.max(0.0) / HALF_LIFE_DAYS);
    let contradiction = f64::from(CONTRADICTION_DECAY).powi(contradicted_n.max(0));
    (time * contradiction).clamp(0.0, 1.0) as f32
}

// ───────────────────────────────────────────────────────────────────────────────
// the embedder
// ───────────────────────────────────────────────────────────────────────────────

/// The column's width. `vector(768)` in migration 0055.
pub const EMBEDDING_DIMS: usize = 768;

/// Something that turns a claim into a vector.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// # Errors
    /// The reason the embedder could not answer.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String>;

    /// The model that produced the vectors, recorded with each one (KM-06). `"unknown"`
    /// for an embedder that does not say, which retrieval treats as its own version.
    fn model_name(&self) -> String {
        "unknown".to_string()
    }
}

/// Whether an embedding endpoint is on this host: loopback or the Docker host alias.
///
/// KM-06: claims are the platform's own research and an embedding request carries their
/// text, so no embedding request may leave the machine. A remote `OLLAMA_URL` is refused
/// before any connection is made.
#[must_use]
pub fn is_local_endpoint(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host == "localhost"
        || host == "::1"
        || host == "host.docker.internal"
        || host.strip_prefix("127.").is_some_and(|r| r.split('.').count() == 3 && r.split('.').all(|o| o.parse::<u8>().is_ok()))
}

/// A local Ollama server running `nomic-embed-text` (768 dimensions).
///
/// Local on purpose: the claims are the platform's own research, and §5.6 puts
/// the embedding in the database beside them rather than at a third party.
pub struct OllamaEmbedder {
    url: String,
    model: String,
    http: reqwest::Client,
}

impl OllamaEmbedder {
    /// An embedder for an explicit endpoint and model.
    #[must_use]
    pub fn new(url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            model: model.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    /// `OLLAMA_URL` (default `http://127.0.0.1:11434`) and `INSIGHT_EMBED_MODEL`
    /// (default `nomic-embed-text`).
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
            model: std::env::var("INSIGHT_EMBED_MODEL")
                .unwrap_or_else(|_| "nomic-embed-text".into()),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait::async_trait]
impl Embedder for OllamaEmbedder {
    fn model_name(&self) -> String {
        self.model.clone()
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        // Before any connection: the text of a claim does not leave this host (KM-06).
        if !is_local_endpoint(&self.url) {
            return Err(format!(
                "embedding endpoint {:?} is not on this host; claims are embedded locally and never sent elsewhere",
                self.url
            ));
        }
        let resp = self
            .http
            .post(format!("{}/api/embeddings", self.url.trim_end_matches('/')))
            .json(&json!({ "model": self.model, "prompt": text }))
            .send()
            .await
            .map_err(|e| format!("embedder unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("embedder answered {}", resp.status()));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("embedder body: {e}"))?;
        #[allow(clippy::cast_possible_truncation)]
        let v: Vec<f32> = body
            .get("embedding")
            .and_then(serde_json::Value::as_array)
            .ok_or("embedder body has no `embedding`")?
            .iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect();
        if v.len() != EMBEDDING_DIMS {
            return Err(format!(
                "{} returned {} dimensions and the column is vector({EMBEDDING_DIMS}); a model \
                 of another width needs a migration, not a truncation",
                self.model,
                v.len()
            ));
        }
        Ok(v)
    }
}

/// pgvector's text form for the 768-wide insight column.
///
/// # Errors
/// A reason naming what was wrong.
pub fn vector_literal(v: &[f32]) -> Result<String, String> {
    vector_literal_n(v, EMBEDDING_DIMS)
}

/// pgvector's text form at an explicit width, refusing the wrong width and
/// non-finite values. The asset embeddings are `vector(48)`.
///
/// # Errors
/// A reason naming what was wrong.
pub fn vector_literal_n(v: &[f32], dims: usize) -> Result<String, String> {
    if v.len() != dims {
        return Err(format!("expected {dims} dimensions, got {}", v.len()));
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err("an embedding with a non-finite component is not an embedding".into());
    }
    let parts: Vec<String> = v.iter().map(|x| format!("{x}")).collect();
    Ok(format!("[{}]", parts.join(",")))
}

/// What one contradiction does to a claim's score.
///
/// A half, so three contradictions take a claim from 1.0 to 0.125 — below the
/// retrieval budget's reach in any crowded scope, without ever deleting it.
pub const CONTRADICTION_DECAY: f32 = 0.5;

/// Truncate a result set to the token cap, returning what fitted and the cost.
///
/// Truncation is by whole insights: half a claim is not a claim, and an agent
/// reading a truncated one will act on it anyway.
#[must_use]
pub fn apply_token_cap(insights: Vec<Insight>) -> (Vec<Insight>, usize) {
    let mut kept = Vec::new();
    let mut used = 0;
    for i in insights {
        let cost = i.token_cost();
        if used + cost > INSIGHT_TOKEN_CAP {
            break;
        }
        used += cost;
        kept.push(i);
    }
    (kept, used)
}

// ───────────────────────────────────────────────────────────────────────────────
// the endpoints
// ───────────────────────────────────────────────────────────────────────────────

fn refuse(code: &str, message: String, fix: &str) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": code, "message": message, "fix": fix })),
    )
        .into_response()
}

fn forbidden() -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "forbidden", "message": format!("this needs the `{}` scope", scopes::MEMORY) })),
    )
        .into_response()
}

fn insight_error(e: InsightError) -> axum::response::Response {
    match e {
        InsightError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": e.to_string() })),
        )
            .into_response(),
        InsightError::Invalid(_) | InsightError::UnknownEvidence(_) => refuse(
            "invalid_insight",
            e.to_string(),
            "name the trials the claim rests on; evidence is required (SPEC §5.6)",
        ),
        InsightError::Backend(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/knowledge/insights` query.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub tier: Option<i16>,
    pub limit: Option<i64>,
    /// Natural-language query. Ranked semantically when the embedder answers,
    /// by decay alone when it does not — and the response says which.
    pub q: Option<String>,
    /// Comma-separated capabilities the caller has (`short_selling,options_data`).
    /// An insight whose `scope.requires` names one the caller lacks is flagged
    /// `capability_mismatch`; with none given the flag is `null`, not `false` (KM-02).
    #[serde(default)]
    pub capabilities: Option<String>,
}

/// `GET /api/knowledge/insights` — the retrieval path (SPEC §5.6).
///
/// The 4 000-token cap is applied in [`InsightStore`], not here, so a client
/// that asks for `limit=500` still gets at most the budget.
pub async fn search_insights(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<SearchParams>,
) -> impl IntoResponse {
    if !token.permits(scopes::MEMORY) {
        return forbidden();
    }
    let tenant = token.user_id().to_string();
    let store = InsightStore::new(state.pg.clone());
    let limit = params.limit.unwrap_or(50);

    let mut semantic = false;
    let mut why: Option<String> = None;
    let mut result = None;
    // KM-02: the structured filter runs first. When it matches nothing there is
    // nothing to rank, so no semantic query is issued: an embedding call that cannot
    // change the answer is cost, latency and an egress the caller did not need.
    let by_metadata = match store.search(&tenant, params.tier, limit).await {
        Ok(r) => r,
        Err(e) => return insight_error(InsightError::Backend(e)),
    };
    let semantic_skipped = by_metadata.0.is_empty();
    if let Some(q) = params.q.as_deref().filter(|q| {
        crate::knowledge_routes::should_issue_semantic_query(by_metadata.0.len(), Some(q))
    }) {
        let embedder = OllamaEmbedder::from_env();
        let model = embedder.model_name();
        match embedder.embed(q).await {
            Ok(v) => match store
                .search_semantic_for_model(&tenant, &v, params.tier, limit, Some(&model))
                .await
            {
                Ok(r) => {
                    semantic = true;
                    result = Some(r);
                }
                Err(e) => why = Some(e.to_string()),
            },
            Err(e) => why = Some(e),
        }
    }
    let (insights, used) = match result {
        Some(r) => r,
        None => by_metadata,
    };
    let have: Option<Vec<String>> = params.capabilities.as_deref().map(|c| {
        c.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    });
    let annotated = crate::knowledge_routes::annotate_insights(&state.pg, &tenant, &insights, have.as_deref()).await;
    Json(json!({
        "insights": annotated,
        "tokens_used": used,
        "token_cap": INSIGHT_TOKEN_CAP,
        "semantic": semantic,
        "semantic_skipped": semantic_skipped,
        "semantic_unavailable": why,
    }))
    .into_response()
}

/// `POST /api/knowledge/insights` body.
#[derive(Debug, Deserialize)]
pub struct NewInsight {
    pub tier: i16,
    #[serde(default)]
    pub scope: serde_json::Value,
    pub claim: String,
    pub evidence_trial_ids: Vec<Uuid>,
}

/// `POST /api/knowledge/insights` — mint a claim with its evidence.
pub async fn create_insight(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<NewInsight>,
) -> impl IntoResponse {
    if !token.permits(scopes::MEMORY) {
        return forbidden();
    }
    let scope = if body.scope.is_null() {
        json!({})
    } else {
        body.scope
    };
    match InsightStore::new(state.pg.clone())
        .create(
            &token.user_id().to_string(),
            body.tier,
            &scope,
            &body.claim,
            &body.evidence_trial_ids,
        )
        .await
    {
        Ok(id) => (StatusCode::CREATED, Json(json!({ "insight_id": id }))).into_response(),
        Err(e) => insight_error(e),
    }
}

/// `POST /api/knowledge/insights/{id}/confirm|contradict` body.
#[derive(Debug, Deserialize)]
pub struct InsightEventBody {
    pub evidence_trial_ids: Vec<Uuid>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /api/knowledge/insights/{id}/contradict`.
pub async fn contradict_insight(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<InsightEventBody>,
) -> impl IntoResponse {
    if !token.permits(scopes::MEMORY) {
        return forbidden();
    }
    match InsightStore::new(state.pg.clone())
        .contradict(
            &token.user_id().to_string(),
            id,
            &body.evidence_trial_ids,
            body.note.as_deref(),
        )
        .await
    {
        Ok(score) => Json(json!({ "insight_id": id, "decay_score": score })).into_response(),
        Err(e) => insight_error(e),
    }
}

/// `POST /api/knowledge/insights/{id}/confirm`.
pub async fn confirm_insight(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<InsightEventBody>,
) -> impl IntoResponse {
    if !token.permits(scopes::MEMORY) {
        return forbidden();
    }
    match InsightStore::new(state.pg.clone())
        .confirm(
            &token.user_id().to_string(),
            id,
            &body.evidence_trial_ids,
            body.note.as_deref(),
        )
        .await
    {
        Ok(score) => Json(json!({ "insight_id": id, "decay_score": score })).into_response(),
        Err(e) => insight_error(e),
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// the timer
// ───────────────────────────────────────────────────────────────────────────────

/// How often the knowledge plane's derived tables are refreshed.
///
/// Hourly: the tensor is a projection of an append-only ledger and rebuilding it
/// is an idempotent upsert, and the insight decay is a pure function of time, so
/// neither is sensitive to the period. It only has to be short against how long
/// the facts are expected to be read stale.
pub const TIMER_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// The first pass waits this long, so the migrations and the pools have settled.
const FIRST_PASS_DELAY: Duration = Duration::from_secs(120);

/// The tenants the plane serves: every user, because a tenant id here is a user
/// id (the job store and the suite both register trials under
/// `user_id.to_string()`).
///
/// The separation job used to run for the tenant `"default"`, which no trial
/// was ever registered under, so it measured an empty ledger every day.
///
/// # Errors
/// Backend failures.
pub async fn tenants(pg: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT user_id::text FROM users WHERE active ORDER BY created_at")
        .fetch_all(pg)
        .await
}

/// What one timer pass did, for the log and for tests.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct TimerReport {
    pub tenants: usize,
    pub facts_written: u64,
    pub insights_decayed: u64,
    pub insights_embedded: u64,
    pub errors: usize,
}

/// One pass: rebuild every tenant's outcome tensor, decay its insights, embed
/// the claims that have no embedding.
///
/// A failure for one tenant or one step is logged and counted, not propagated:
/// the passes are independent and idempotent, and one tenant's bad row must not
/// stop another's tensor being rebuilt.
pub async fn run_once(pg: &PgPool, embedder: Option<&dyn Embedder>) -> TimerReport {
    let mut report = TimerReport::default();
    let tenants = match tenants(pg).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "knowledge timer: could not list tenants");
            report.errors += 1;
            return report;
        }
    };
    report.tenants = tenants.len();
    let projection = TensorProjection::new(pg.clone());
    let insights = InsightStore::new(pg.clone());
    for tenant in &tenants {
        match projection.rebuild(tenant).await {
            Ok(r) => {
                report.facts_written += r.facts_written;
                if let Ok((ready, why)) = projection.ready_to_fit(tenant).await {
                    tracing::info!(%tenant, facts = r.facts_written, censored = r.censored, ready, %why, "outcome tensor rebuilt");
                }
            }
            Err(e) => {
                tracing::error!(%tenant, error = %e, "outcome tensor rebuild failed");
                report.errors += 1;
            }
        }
        match insights.decay_sweep(tenant).await {
            Ok(n) => report.insights_decayed += n,
            // The insight table needs pgvector; a database without it has no
            // table and this is the honest report of that.
            Err(e) => {
                tracing::warn!(%tenant, error = %e, "insight decay sweep failed");
                report.errors += 1;
            }
        }
        if let Some(embedder) = embedder {
            match insights.embed_missing(tenant, embedder, 100).await {
                Ok((done, _)) => report.insights_embedded += done,
                Err(e) => {
                    tracing::warn!(%tenant, error = %e, "insight embedding pass failed");
                    report.errors += 1;
                }
            }
        }
    }
    report
}

/// The `InstrumentProfile` job for a calendar day.
///
/// The day is *in the manifest*, and that is the scheduling: top-level jobs are
/// deduplicated on their manifest hash (JB-02), so submitting this every hour
/// creates the day's job once and returns it for the other twenty-three. A
/// manifest with no date would be deduplicated against yesterday's finished job
/// for ever, and the profile would run exactly once.
#[must_use]
pub fn daily_profile_manifest(day: chrono::NaiveDate) -> serde_json::Value {
    json!({
        "window_days": instrument_profile_worker::DEFAULT_WINDOW_DAYS,
        "as_of_day": day.to_string(),
    })
}

/// Submit today's profile job. The first caller of the kind's worker.
///
/// A market-public job has no owner; it is submitted by the system under the nil
/// user, in the system queue.
///
/// # Errors
/// The job store's refusal.
pub async fn submit_daily_profile(
    store: &jobs::store::JobStore,
    today: chrono::NaiveDate,
) -> Result<jobs::store::Submitted, jobs::store::JobStoreError> {
    store
        .submit(jobs::store::Submission::new(
            jobs::JobKind::InstrumentProfile,
            Uuid::nil(),
            daily_profile_manifest(today),
        ))
        .await
}

/// Start the hourly pass. The one production caller of [`TensorProjection`], of
/// the insight decay, and (through the job store) of `InstrumentProfile`: all
/// three were complete and unscheduled.
pub fn spawn_timers(pg: PgPool, jobs: Option<std::sync::Arc<jobs::store::JobStore>>) {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_PASS_DELAY).await;
        let embedder = OllamaEmbedder::from_env();
        loop {
            let report = run_once(&pg, Some(&embedder)).await;
            tracing::info!(?report, "knowledge timer pass");
            if let Some(store) = &jobs {
                match submit_daily_profile(store, chrono::Utc::now().date_naive()).await {
                    Ok(s) => tracing::info!(
                        job = %s.job_id,
                        deduplicated = s.deduplicated,
                        "instrument profile job"
                    ),
                    Err(e) => tracing::warn!(error = %e, "instrument profile job not submitted"),
                }
            }
            tokio::time::sleep(TIMER_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insight(claim_len: usize, decay: f32) -> Insight {
        Insight {
            insight_id: uuid::Uuid::new_v4(),
            tier: 3,
            claim: "x".repeat(claim_len),
            scope: serde_json::json!({}),
            evidence_trial_ids: vec![uuid::Uuid::new_v4()],
            support_n: 1,
            contradicted_n: 0,
            decay_score: decay,
        }
    }

    /// Below either threshold the answer is "not fitted", spelled out. A
    /// factorization of three hundred cells over three instruments is noise with
    /// a version number, and shipping it would make every recommendation look
    /// model-backed.
    #[test]
    fn the_tensor_is_not_fitted_until_both_thresholds_are_met() {
        assert!(!fit_readiness(499, 50).0);
        assert!(!fit_readiness(5_000, 19).0);
        assert!(fit_readiness(500, 20).0);

        let (_, why) = fit_readiness(120, 4);
        assert!(why.contains("not fitted"), "{why}");
        assert!(why.contains("120 of 500"), "{why}");
        assert!(why.contains("4 of 20"), "{why}");
        assert!(why.contains("rule tier"), "{why}");
    }

    /// The cap is server-side and counted in whole insights: half a claim is not
    /// a claim, and an agent reading a truncated one acts on it anyway.
    #[test]
    fn the_retrieval_budget_is_enforced_by_whole_insights() {
        // Each of these costs 1 000 tokens; five would be 5 000.
        let many: Vec<Insight> = (0..5).map(|_| insight(4_000, 1.0)).collect();
        let (kept, used) = apply_token_cap(many);
        assert_eq!(kept.len(), 4, "the fifth does not fit");
        assert_eq!(used, 4_000);
        assert!(used <= INSIGHT_TOKEN_CAP);
    }

    #[test]
    fn a_single_oversized_claim_does_not_blow_the_budget() {
        let (kept, used) = apply_token_cap(vec![insight(INSIGHT_TOKEN_CAP * 8, 1.0)]);
        assert!(
            kept.is_empty(),
            "one claim bigger than the whole budget returns nothing"
        );
        assert_eq!(used, 0);
    }

    #[test]
    fn an_empty_scope_costs_nothing() {
        let (kept, used) = apply_token_cap(Vec::new());
        assert!(kept.is_empty());
        assert_eq!(used, 0);
    }

    /// Three contradictions take a claim well below the reach of a crowded
    /// scope's budget — without ever deleting it. "We believed this and stopped"
    /// is a fact worth keeping, and a claim that vanishes can be rediscovered
    /// and re-believed indefinitely.
    #[test]
    fn contradiction_decays_rather_than_deletes() {
        let mut score = 1.0_f32;
        for _ in 0..3 {
            score *= CONTRADICTION_DECAY;
        }
        assert!((score - 0.125).abs() < 1e-6);
        assert!(score > 0.0, "it never reaches zero by decay alone");
    }

    #[test]
    fn a_claims_token_cost_rounds_up() {
        assert_eq!(
            insight(1, 1.0).token_cost(),
            1,
            "one character still costs a token"
        );
        assert_eq!(insight(4, 1.0).token_cost(), 1);
        assert_eq!(insight(5, 1.0).token_cost(), 2);
    }

    // ---- the insight life cycle ----------------------------------------------

    /// A fresh, uncontradicted claim scores 1.0; each half-life halves it; each
    /// contradiction halves it again. Pure, so the sweep and the write agree.
    #[test]
    fn the_decay_score_is_a_pure_function_of_age_and_contradictions() {
        assert!((decayed_score(0.0, 0) - 1.0).abs() < 1e-6);
        assert!((decayed_score(HALF_LIFE_DAYS, 0) - 0.5).abs() < 1e-6);
        assert!((decayed_score(0.0, 1) - 0.5).abs() < 1e-6);
        assert!((decayed_score(HALF_LIFE_DAYS, 2) - 0.125).abs() < 1e-6);
        // Idempotent by construction: same inputs, same output.
        assert_eq!(decayed_score(31.7, 2), decayed_score(31.7, 2));
        // Never negative, never above one, never exactly zero by decay alone.
        assert!(decayed_score(10_000.0, 0) > 0.0);
        assert!(decayed_score(-5.0, 0) <= 1.0);
    }

    /// A contradiction takes a live claim below an uncontradicted stale one
    /// within a quarter, which is the point of §5.6.
    #[test]
    fn a_contradicted_claim_loses_to_an_unconfirmed_one_of_the_same_age() {
        assert!(decayed_score(10.0, 1) < decayed_score(10.0, 0));
        assert!(decayed_score(0.0, 3) < decayed_score(HALF_LIFE_DAYS, 0));
    }

    #[test]
    fn age_runs_from_the_last_confirmation_and_never_from_before_birth() {
        use chrono::TimeZone;
        let born = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let later = born + chrono::Duration::days(40);
        let now = born + chrono::Duration::days(100);
        assert!((age_days(born, None, now) - 100.0).abs() < 1e-9);
        assert!((age_days(born, Some(later), now) - 60.0).abs() < 1e-9);
        // A confirmation stamped before the claim existed is clock skew, not
        // extra age.
        let skew = born - chrono::Duration::days(10);
        assert!((age_days(born, Some(skew), now) - 100.0).abs() < 1e-9);
        // A future timestamp is not negative age.
        assert_eq!(age_days(born, None, born - chrono::Duration::days(1)), 0.0);
    }

    // ---- the embedder --------------------------------------------------------

    #[test]
    fn a_vector_literal_is_pgvectors_text_form_at_exactly_768_dimensions() {
        let v = vec![0.5_f32; EMBEDDING_DIMS];
        let lit = vector_literal(&v).unwrap();
        assert!(lit.starts_with("[0.5,0.5"));
        assert!(lit.ends_with("0.5]"));
        assert_eq!(lit.matches(',').count(), EMBEDDING_DIMS - 1);
    }

    /// A model of another width is a migration, not a truncation; a NaN is not a
    /// coordinate.
    #[test]
    fn the_wrong_width_and_non_finite_values_are_refused() {
        assert!(vector_literal(&vec![0.0; 384]).unwrap_err().contains("768"));
        let mut v = vec![0.0_f32; EMBEDDING_DIMS];
        v[7] = f32::NAN;
        assert!(vector_literal(&v).unwrap_err().contains("non-finite"));
        v[7] = f32::INFINITY;
        assert!(vector_literal(&v).is_err());
    }

    /// A fake embedder drives the trait without a model.
    struct Fixed(Vec<f32>);

    #[async_trait::async_trait]
    impl Embedder for Fixed {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, String> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn an_embedder_returns_through_the_trait() {
        let e = Fixed(vec![0.25; EMBEDDING_DIMS]);
        let v = e.embed("a claim").await.unwrap();
        assert_eq!(v.len(), EMBEDDING_DIMS);
        assert!(vector_literal(&v).is_ok());
    }

    /// The day is in the manifest, so the hourly timer creates one job per day.
    #[test]
    fn the_daily_profile_manifest_differs_by_day_and_not_within_one() {
        let d1 = chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let d2 = chrono::NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        let hash = |d| {
            jobs::store::Submission::new(
                jobs::JobKind::InstrumentProfile,
                Uuid::nil(),
                daily_profile_manifest(d),
            )
            .manifest_hash()
        };
        assert_eq!(hash(d1), hash(d1), "hourly resubmission must dedupe");
        assert_ne!(hash(d1), hash(d2), "a new day must be a new job");
        assert!(!jobs::JobKind::InstrumentProfile.counts_trial());
    }

    /// The route is scope-gated: a token without the memory scope is refused
    /// before any query runs.
    #[test]
    fn the_insight_endpoints_are_behind_the_memory_scope() {
        assert_eq!(scopes::MEMORY, "research:memory");
        assert!(!scopes::permits(&["research:data.read".to_string()], scopes::MEMORY));
        assert!(scopes::permits(&[scopes::MEMORY.to_string()], scopes::MEMORY));
        assert!(scopes::permits(&[scopes::WEB_FULL.to_string()], scopes::MEMORY));
    }

    #[tokio::test]
    async fn an_embedding_request_never_leaves_the_host() {
        for local in [
            "http://127.0.0.1:11434",
            "http://localhost:11434/",
            "http://[::1]:11434",
            "http://host.docker.internal:11434",
            "http://127.1.2.3",
        ] {
            assert!(is_local_endpoint(local), "{local}");
        }
        for remote in [
            "http://example.com:11434",
            "https://api.openai.com/v1",
            "http://10.0.0.5:11434",
            "http://127.0.0.1.evil.example",
            "http://user@evil.example:80",
            "http://localhost.evil.example",
        ] {
            assert!(!is_local_endpoint(remote), "{remote}");
        }
        // The refusal happens before a connection is attempted: this URL cannot resolve, and
        // the error names the policy, not a network failure.
        let e = OllamaEmbedder::new("http://embed.invalid:11434", "nomic-embed-text")
            .embed("a claim")
            .await
            .unwrap_err();
        assert!(e.contains("not on this host"), "{e}");
        assert_eq!(OllamaEmbedder::new("http://127.0.0.1:11434", "m1").model_name(), "m1");
    }
}
