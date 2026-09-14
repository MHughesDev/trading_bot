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
//! ## 3.10 — retrieval is capped server-side
//!
//! §15 gives the agent a 4 000-token budget for retrieved insights, and
//! [`InsightStore::search`] enforces it **here** rather than trusting the
//! caller to ask for few enough. A cap the client applies is a cap that is off
//! whenever a new client appears.

use serde::Serialize;
use sqlx::PgPool;

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

        // Everything the tensor's coordinates need, straight from the ledger.
        // The regime and cluster coordinates are 0 until 3.5's labels and 3.6's
        // clustering exist: a fact in the "unknown regime" cell is still a fact,
        // and refusing to write one until the clustering lands would leave the
        // completion model with nothing to be fitted on when it arrives.
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
                    (s.outcome_vector ->> 'sharpe_net')::double precision,
                    t.propensity,
                    s.censoring,
                    NULL::int,
                    now()
             FROM mlops.trial t
             JOIN mlops.trial_state s ON s.trial_id = t.trial_id
             WHERE t.tenant_id = $1
               AND s.outcome_vector ? 'sharpe_net'
               AND (s.outcome_vector ->> 'sharpe_net') IS NOT NULL
             ORDER BY t.registered_at
             LIMIT $2
             ON CONFLICT (tenant_id, trial_id, metric_name) DO UPDATE
               SET metric_value = EXCLUDED.metric_value,
                   propensity   = EXCLUDED.propensity,
                   censoring    = EXCLUDED.censoring,
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
               AND (s.outcome_vector IS NULL
                    OR NOT (s.outcome_vector ? 'sharpe_net')
                    OR (s.outcome_vector ->> 'sharpe_net') IS NULL)",
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
        return (true, format!("{trials} trials over {instruments} instruments"));
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
        let rows = sqlx::query_as::<_, (uuid::Uuid, i16, String, serde_json::Value, Vec<uuid::Uuid>, i32, i32, f32)>(
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
            .map(|(insight_id, tier, claim, scope, evidence_trial_ids, support_n, contradicted_n, decay_score)| {
                Insight { insight_id, tier, claim, scope, evidence_trial_ids, support_n, contradicted_n, decay_score }
            })
            .collect();
        Ok(apply_token_cap(all))
    }

    /// Record that a claim was contradicted, decaying it.
    ///
    /// Decay rather than deletion: "we believed this and stopped" is a fact
    /// worth keeping, and a claim that vanishes when contradicted can be
    /// rediscovered and re-believed indefinitely.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn contradict(
        &self,
        tenant_id: &str,
        insight_id: uuid::Uuid,
    ) -> Result<f32, sqlx::Error> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant_id)
            .await
            .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let (score,): (f32,) = sqlx::query_as(
            "UPDATE knowledge.insight
             SET contradicted_n = contradicted_n + 1,
                 decay_score = greatest(0.0, decay_score * $3)
             WHERE tenant_id = $1 AND insight_id = $2
             RETURNING decay_score",
        )
        .bind(tenant_id)
        .bind(insight_id)
        .bind(CONTRADICTION_DECAY)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(score)
    }
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
        assert!(kept.is_empty(), "one claim bigger than the whole budget returns nothing");
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
        assert_eq!(insight(1, 1.0).token_cost(), 1, "one character still costs a token");
        assert_eq!(insight(4, 1.0).token_cost(), 1);
        assert_eq!(insight(5, 1.0).token_cost(), 2);
    }
}
