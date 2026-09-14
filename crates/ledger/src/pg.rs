//! Durable ledger over Postgres (`mlops` schema, migration 0043).
//!
//! Hash chains are computed by triggers; [`TrialRow::recompute_hash`] and
//! [`EventRow::recompute_hash`] are the independent verifiers. Every statement runs
//! in a transaction that sets `app.tenant_id` with `set_config(..., true)` — the
//! transaction-local form, which is what keeps tenant context from bleeding across
//! pooled connections (S-2). A bare session `SET` never appears in this crate.
//!
//! The trait is synchronous because the dispatching Study engine is synchronous CPU
//! work run under `spawn_blocking`; registration must be durable before it returns.
//! Async callers use the `*_async` methods.

use sqlx::{PgPool, Postgres, Row, Transaction};
use tokio::runtime::Handle;
use uuid::Uuid;

use crate::anchor::{self, AnchorDocument, AnchorError, AnchorRow, AnchorSigner, DecisionRow, VerificationReport, WormStore};
use crate::holdout::{claimed_without_result, HoldoutClaim};
use crate::neff::{NEff, ReturnSeries};
use crate::{
    mint, Decision, DecisionLog, DispatchContext, EventRow, LedgerError, OutcomeVector, Registration, TrialEvent,
    TrialLedger, TrialRow, TrialState, TrialTicket,
};

pub struct PgTrialLedger {
    pool: PgPool,
    handle: Option<Handle>,
}

fn be(e: impl std::fmt::Display) -> LedgerError {
    LedgerError::Backend(e.to_string())
}

/// Begin a transaction scoped to one tenant.
///
/// # Errors
/// Backend failures.
pub async fn tenant_tx<'a>(pool: &'a PgPool, tenant_id: &str) -> Result<Transaction<'a, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)").bind(tenant_id).execute(&mut *tx).await?;
    Ok(tx)
}


/// Register a trial inside a caller's transaction, so the trial and whatever it
/// authorizes (a job row, say) commit or roll back together.
///
/// # Errors
/// Refused registrations and backend failures.
pub async fn register_in_tx(tx: &mut Transaction<'_, Postgres>, reg: &Registration<'_>) -> Result<TrialTicket, LedgerError> {
    reg.validate()?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)").bind(&reg.ctx.tenant_id).execute(&mut **tx).await.map_err(be)?;
        let trial_id: Uuid = sqlx::query_scalar(
            "INSERT INTO mlops.trial (
                 seq, prev_hash, row_hash, tenant_id, campaign_id, experiment_id, parent_trial_id,
                 config_hash, config, dataset_id, split_spec_id, code_hash, image_digest, seed_set,
                 actor_kind, actor_id, on_behalf_of, policy_id, policy_version, candidate_set_hash,
                 propensity, exploration_flag, hypothesis_id, prereg_hash, delta_practical,
                 non_reproducible, overlapping_labels_unweighted, split_overrides, planned_steps, supersedes
             ) VALUES (0, '\\x'::bytea, '\\x'::bytea, $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27)
             RETURNING trial_id",
        )
        .bind(&reg.ctx.tenant_id)
        .bind(reg.ctx.campaign_id)
        .bind(reg.ctx.experiment_id.as_deref())
        .bind(reg.parent_trial_id)
        .bind(&reg.subject.config_hash)
        .bind(&reg.subject.config)
        .bind(&reg.subject.dataset_id)
        .bind(reg.subject.split_spec_id.as_deref())
        .bind(&reg.subject.code_hash)
        .bind(&reg.subject.image_digest)
        .bind(&reg.subject.seed_set)
        .bind(reg.ctx.actor_kind.as_str())
        .bind(&reg.ctx.actor_id)
        .bind(reg.ctx.on_behalf_of.as_deref())
        .bind(&reg.ctx.policy_id)
        .bind(reg.ctx.policy_version)
        .bind(reg.candidate_set_hash.as_deref())
        .bind(reg.propensity)
        .bind(reg.exploration_flag)
        .bind(reg.hypothesis_id)
        .bind(reg.prereg_hash())
        .bind(reg.ctx.delta_practical)
        .bind(reg.subject.non_reproducible)
        .bind(reg.subject.overlapping_labels_unweighted)
        .bind(&reg.subject.split_overrides)
        .bind(reg.subject.planned_steps)
        .bind(reg.supersedes)
        .fetch_one(&mut **tx)
        .await
        .map_err(be)?;
    Ok(mint(trial_id, reg.ctx.tenant_id.clone(), reg.subject.config_hash.clone()))
}


/// Append a lifecycle event to an existing trial inside a caller's transaction,
/// validating against the trial's current state as stored. Used where the trial's
/// state changes together with another row (a job's state, say).
///
/// # Errors
/// Unknown trial, illegal transition, backend failure.
pub async fn append_event_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    trial_id: Uuid,
    event: &TrialEvent,
) -> Result<TrialState, LedgerError> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)").bind(tenant_id).execute(&mut **tx).await.map_err(be)?;
    let state: Option<String> = sqlx::query_scalar("SELECT state FROM mlops.trial_state WHERE trial_id = $1")
        .bind(trial_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(be)?;
    let from = state
        .as_deref()
        .and_then(TrialState::parse)
        .ok_or(LedgerError::UnknownTrial(trial_id))?;
    let to = event.validate(from)?;
    insert_event(tx, tenant_id, trial_id, to, event).await?;
    Ok(to)
}

/// The trial's current state, inside a caller's transaction.
///
/// # Errors
/// Backend failure.
pub async fn state_in_tx(tx: &mut Transaction<'_, Postgres>, tenant_id: &str, trial_id: Uuid) -> Result<Option<TrialState>, LedgerError> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)").bind(tenant_id).execute(&mut **tx).await.map_err(be)?;
    let state: Option<String> = sqlx::query_scalar("SELECT state FROM mlops.trial_state WHERE trial_id = $1")
        .bind(trial_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(be)?;
    Ok(state.as_deref().and_then(TrialState::parse))
}

async fn insert_event(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    trial_id: Uuid,
    to: TrialState,
    event: &TrialEvent,
) -> Result<(), LedgerError> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)").bind(tenant_id).execute(&mut **tx).await.map_err(be)?;
    // A composite whose fields are all NULL *is* NULL in SQL; normalize so the
    // digest check never disagrees with the column.
    let o = event.outcome.filter(|o| *o != OutcomeVector::default());
        sqlx::query(
            "INSERT INTO mlops.trial_event (
                 trial_id, tenant_id, event_seq, state, censoring, terminal_reason, censor_at_step, run_id,
                 deduplicated_of, outcome, outcome_digest, gate_profile, gate_results, gpu_seconds, cpu_seconds,
                 peak_vram_bytes, usd_cost, artifacts_uri, metrics_uri, predictions_uri, returns_uri, detail,
                 prev_hash, row_hash
             ) VALUES (
                 $1, $2, 0, $3, $4, $5, $6, $7, $8,
                 CASE WHEN $9 THEN ROW($10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36)::mlops.outcome_vector END,
                 $37, $38, $39, $40, $41, $42, $43::numeric, $44, $45, $46, $47, $48, '\\x'::bytea, '\\x'::bytea
             )",
        )
        .bind(trial_id)
        .bind(tenant_id)
        .bind(to.as_str())
        .bind(event.effective_censoring().as_str())
        .bind(event.terminal_reason.map(crate::TerminalReason::as_str))
        .bind(event.censor_at_step)
        .bind(event.run_id.as_deref())
        .bind(event.deduplicated_of)
        .bind(o.is_some())
        .bind(o.and_then(|o| o.auc))
        .bind(o.and_then(|o| o.logloss))
        .bind(o.and_then(|o| o.brier))
        .bind(o.and_then(|o| o.ece))
        .bind(o.and_then(|o| o.ic))
        .bind(o.and_then(|o| o.ic_ir))
        .bind(o.and_then(|o| o.sharpe_net))
        .bind(o.and_then(|o| o.sortino_net))
        .bind(o.and_then(|o| o.calmar))
        .bind(o.and_then(|o| o.psr))
        .bind(o.and_then(|o| o.dsr))
        .bind(o.and_then(|o| o.pbo))
        .bind(o.and_then(|o| o.max_dd))
        .bind(o.and_then(|o| o.dd_duration_days))
        .bind(o.and_then(|o| o.turnover_annual))
        .bind(o.and_then(|o| o.capacity_usd))
        .bind(o.and_then(|o| o.breakeven_cost_multiple))
        .bind(o.and_then(|o| o.seed_sharpe_std))
        .bind(o.and_then(|o| o.regime_pnl_hhi))
        .bind(o.and_then(|o| o.cpcv_p05_sharpe))
        .bind(o.and_then(|o| o.bootstrap_p05_sharpe))
        .bind(o.and_then(|o| o.param_cliff_score))
        .bind(o.and_then(|o| o.alpha_t_stat))
        .bind(o.and_then(|o| o.factor_r2))
        .bind(o.and_then(|o| o.inference_latency_p99_ms))
        .bind(o.and_then(|o| o.model_bytes))
        .bind(o.and_then(|o| o.train_gpu_seconds))
        .bind(o.map(|o| o.digest()))
        .bind(event.gate_profile.as_deref())
        .bind(&event.gate_results)
        .bind(event.gpu_seconds)
        .bind(event.cpu_seconds)
        .bind(event.peak_vram_bytes)
        .bind(event.usd_cost.map(|c| c.to_string()))
        .bind(event.artifacts_uri.as_deref())
        .bind(event.metrics_uri.as_deref())
        .bind(event.predictions_uri.as_deref())
        .bind(event.returns_uri.as_deref())
        .bind(event.detail.clone().unwrap_or_else(|| serde_json::json!({})))
        .execute(&mut **tx)
        .await
        .map_err(be)?;
    Ok(())
}

impl PgTrialLedger {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool, handle: Handle::try_current().ok() }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    fn block<F: std::future::Future>(&self, fut: F) -> F::Output {
        let handle = self.handle.as_ref().expect("PgTrialLedger: synchronous call outside a Tokio runtime; use the *_async methods");
        tokio::task::block_in_place(|| handle.block_on(fut))
    }

    ///
    /// # Errors
    /// Refused registrations and backend failures.
    pub async fn register_async(&self, reg: &Registration<'_>) -> Result<TrialTicket, LedgerError> {
        let mut tx = self.pool.begin().await.map_err(be)?;
        let ticket = register_in_tx(&mut tx, reg).await?;
        tx.commit().await.map_err(be)?;
        Ok(ticket)
    }

    /// Re-mint the ticket for a trial that is registered and not yet terminal — the
    /// path a worker takes when it claims queued work. The trial row already exists,
    /// so this opens no side door: a terminal or unknown trial is refused.
    ///
    /// # Errors
    /// Unknown or terminal trials, and backend failures.
    pub async fn resume_ticket_async(&self, tenant_id: &str, trial_id: Uuid) -> Result<TrialTicket, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: Option<(String, String, bool)> =
            sqlx::query_as("SELECT config_hash, state, terminal FROM mlops.trial_state WHERE trial_id = $1")
                .bind(trial_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(be)?;
        tx.commit().await.map_err(be)?;
        let (config_hash, state, terminal) = row.ok_or(LedgerError::UnknownTrial(trial_id))?;
        if terminal {
            return Err(LedgerError::Invalid(format!("trial {trial_id} is already terminal ({state})")));
        }
        let mut t = mint(trial_id, tenant_id.to_string(), config_hash);
        t.state = TrialState::parse(&state).ok_or_else(|| LedgerError::Invalid(format!("unknown state {state}")))?;
        Ok(t)
    }

    async fn append(&self, ticket: &TrialTicket, event: &TrialEvent) -> Result<TrialState, LedgerError> {
        let to = event.validate(ticket.state)?;
        let mut tx = self.pool.begin().await.map_err(be)?;
        insert_event(&mut tx, &ticket.tenant_id, ticket.trial_id, to, event).await?;
        tx.commit().await.map_err(be)?;
        Ok(to)
    }

    ///
    /// # Errors
    /// Illegal transitions and backend failures.
    pub async fn transition_async(&self, ticket: &mut TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        if event.state.is_some_and(TrialState::is_terminal) {
            return Err(LedgerError::Invalid("terminal transitions go through settle()".into()));
        }
        ticket.state = self.append(ticket, &event).await?;
        Ok(())
    }

    ///
    /// # Errors
    /// Illegal transitions and backend failures.
    pub async fn settle_async(&self, ticket: TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        if !event.state.is_some_and(TrialState::is_terminal) {
            return Err(LedgerError::Invalid("settle() requires a terminal state".into()));
        }
        self.append(&ticket, &event).await?;
        Ok(())
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn prior_trial_async(&self, tenant_id: &str, config_hash: &str) -> Result<Option<Uuid>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let id = sqlx::query_scalar(
            "SELECT trial_id FROM mlops.trial_state
              WHERE tenant_id = $1 AND config_hash = $2 AND terminal AND state <> 'deduplicated'
              ORDER BY registered_at DESC LIMIT 1",
        )
        .bind(tenant_id)
        .bind(config_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(id)
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn trial_rows_async(&self, tenant_id: &str) -> Result<Vec<TrialRow>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows = sqlx::query(
            "SELECT trial_id, tenant_id, campaign_id, experiment_id, seq, prev_hash, row_hash, config_hash, dataset_id,
                    code_hash, image_digest, seed_set, prereg_hash, delta_practical, actor_kind, actor_id, on_behalf_of,
                    policy_id, policy_version, candidate_set_hash, propensity, exploration_flag, supersedes
               FROM mlops.trial WHERE tenant_id = $1 ORDER BY seq",
        )
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(rows
            .into_iter()
            .map(|r| TrialRow {
                trial_id: r.get("trial_id"),
                tenant_id: r.get("tenant_id"),
                campaign_id: r.get("campaign_id"),
                experiment_id: r.get("experiment_id"),
                seq: r.get("seq"),
                prev_hash: r.get("prev_hash"),
                row_hash: r.get("row_hash"),
                config_hash: r.get("config_hash"),
                dataset_id: r.get("dataset_id"),
                code_hash: r.get("code_hash"),
                image_digest: r.get("image_digest"),
                seed_set: r.get("seed_set"),
                prereg_hash: r.get("prereg_hash"),
                delta_practical: r.get("delta_practical"),
                actor_kind: r.get("actor_kind"),
                actor_id: r.get("actor_id"),
                on_behalf_of: r.get("on_behalf_of"),
                policy_id: r.get("policy_id"),
                policy_version: r.get("policy_version"),
                candidate_set_hash: r.get("candidate_set_hash"),
                propensity: r.get("propensity"),
                exploration_flag: r.get("exploration_flag"),
                supersedes: r.get("supersedes"),
            })
            .collect())
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn event_rows_async(&self, tenant_id: &str, trial_id: Uuid) -> Result<Vec<EventRow>, LedgerError> {
        self.events_where(tenant_id, Some(trial_id)).await
    }

    /// Every event of a tenant, ordered by trial then sequence.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn all_event_rows_async(&self, tenant_id: &str) -> Result<Vec<EventRow>, LedgerError> {
        self.events_where(tenant_id, None).await
    }

    async fn events_where(&self, tenant_id: &str, trial_id: Option<Uuid>) -> Result<Vec<EventRow>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows = sqlx::query(
            "SELECT event_id, trial_id, event_seq, state, censoring, terminal_reason, censor_at_step, run_id,
                    deduplicated_of, outcome_digest, returns_uri, predictions_uri, prev_hash, row_hash,
                    (outcome).auc, (outcome).logloss, (outcome).brier, (outcome).ece, (outcome).ic, (outcome).ic_ir,
                    (outcome).sharpe_net, (outcome).sortino_net, (outcome).calmar, (outcome).psr, (outcome).dsr,
                    (outcome).pbo, (outcome).max_dd, (outcome).dd_duration_days, (outcome).turnover_annual,
                    (outcome).capacity_usd, (outcome).breakeven_cost_multiple, (outcome).seed_sharpe_std,
                    (outcome).regime_pnl_hhi, (outcome).cpcv_p05_sharpe, (outcome).bootstrap_p05_sharpe,
                    (outcome).param_cliff_score, (outcome).alpha_t_stat, (outcome).factor_r2,
                    (outcome).inference_latency_p99_ms, (outcome).model_bytes, (outcome).train_gpu_seconds,
                    outcome_digest IS NOT NULL AS has_outcome
               FROM mlops.trial_event WHERE tenant_id = $1 AND ($2::uuid IS NULL OR trial_id = $2)
              ORDER BY trial_id, event_seq",
        )
        .bind(tenant_id)
        .bind(trial_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let outcome = r.get::<bool, _>("has_outcome").then(|| OutcomeVector {
                    auc: r.get("auc"),
                    logloss: r.get("logloss"),
                    brier: r.get("brier"),
                    ece: r.get("ece"),
                    ic: r.get("ic"),
                    ic_ir: r.get("ic_ir"),
                    sharpe_net: r.get("sharpe_net"),
                    sortino_net: r.get("sortino_net"),
                    calmar: r.get("calmar"),
                    psr: r.get("psr"),
                    dsr: r.get("dsr"),
                    pbo: r.get("pbo"),
                    max_dd: r.get("max_dd"),
                    dd_duration_days: r.get("dd_duration_days"),
                    turnover_annual: r.get("turnover_annual"),
                    capacity_usd: r.get("capacity_usd"),
                    breakeven_cost_multiple: r.get("breakeven_cost_multiple"),
                    seed_sharpe_std: r.get("seed_sharpe_std"),
                    regime_pnl_hhi: r.get("regime_pnl_hhi"),
                    cpcv_p05_sharpe: r.get("cpcv_p05_sharpe"),
                    bootstrap_p05_sharpe: r.get("bootstrap_p05_sharpe"),
                    param_cliff_score: r.get("param_cliff_score"),
                    alpha_t_stat: r.get("alpha_t_stat"),
                    factor_r2: r.get("factor_r2"),
                    inference_latency_p99_ms: r.get("inference_latency_p99_ms"),
                    model_bytes: r.get("model_bytes"),
                    train_gpu_seconds: r.get("train_gpu_seconds"),
                });
                EventRow {
                    event_id: r.get("event_id"),
                    trial_id: r.get("trial_id"),
                    event_seq: r.get("event_seq"),
                    state: r.get("state"),
                    censoring: r.get("censoring"),
                    terminal_reason: r.get("terminal_reason"),
                    censor_at_step: r.get("censor_at_step"),
                    run_id: r.get("run_id"),
                    deduplicated_of: r.get("deduplicated_of"),
                    outcome,
                    outcome_digest: r.get("outcome_digest"),
                    returns_uri: r.get("returns_uri"),
                    predictions_uri: r.get("predictions_uri"),
                    prev_hash: r.get("prev_hash"),
                    row_hash: r.get("row_hash"),
                }
            })
            .collect())
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn trial_count_async(&self, tenant_id: &str) -> Result<i64, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(n)
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn exploration_fraction_async(&self, tenant_id: &str) -> Result<f64, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let f: Option<f64> = sqlx::query_scalar(
            "SELECT avg(CASE WHEN exploration_flag THEN 1.0 ELSE 0.0 END)::float8 FROM mlops.trial WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(f.unwrap_or(0.0))
    }

    ///
    /// # Errors
    /// Validation and backend failures.
    pub async fn log_decision_async(&self, ctx: &DispatchContext, d: &Decision) -> Result<Uuid, LedgerError> {
        d.validate(ctx)?;
        let mut tx = tenant_tx(&self.pool, &ctx.tenant_id).await.map_err(be)?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO mlops.decision (tenant_id, campaign_id, experiment_id, decision_kind, actor_kind, actor_id,
                 on_behalf_of, context_hash, context_uri, candidate_set, chosen, propensity, policy_id, policy_version,
                 exploration_flag, decision_tier, rationale, seq, prev_hash, row_hash)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,0,'\\x'::bytea,'\\x'::bytea)
             RETURNING decision_id",
        )
        .bind(&ctx.tenant_id)
        .bind(ctx.campaign_id)
        .bind(ctx.experiment_id.as_deref())
        .bind(d.kind.as_str())
        .bind(ctx.actor_kind.as_str())
        .bind(&ctx.actor_id)
        .bind(ctx.on_behalf_of.as_deref())
        .bind(&d.context_hash)
        .bind(d.context_uri.as_deref())
        .bind(serde_json::Value::Array(d.candidate_set.clone()))
        .bind(&d.chosen)
        .bind(d.propensity)
        .bind(&ctx.policy_id)
        .bind(ctx.policy_version)
        .bind(d.exploration_flag)
        .bind(d.decision_tier.as_str())
        .bind(d.rationale.as_deref())
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(id)
    }
}

/// Return series and platform N_eff (INV-18, INV-22).
impl PgTrialLedger {
    /// Persist a trial's OOS return series; returns the URI to chain into its
    /// terminal event.
    ///
    /// # Errors
    /// Malformed series, a second write for the trial, and backend failures.
    pub async fn persist_returns_async(&self, ticket: &TrialTicket, series: &ReturnSeries) -> Result<String, LedgerError> {
        series.validate()?;
        let mut tx = tenant_tx(&self.pool, &ticket.tenant_id).await.map_err(be)?;
        sqlx::query("INSERT INTO mlops.trial_return_series (trial_id, tenant_id, ts, returns, series_digest) VALUES ($1,$2,$3,$4,$5)")
            .bind(ticket.trial_id)
            .bind(&ticket.tenant_id)
            .bind(&series.timestamps)
            .bind(&series.returns)
            .bind(series.digest())
            .execute(&mut *tx)
            .await
            .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(format!("pg://mlops.trial_return_series/{}", ticket.trial_id))
    }

    /// `N_eff` over every stored series on the tenant's ledger.
    ///
    /// # Errors
    /// Backend failures.
    /// `N_eff` as it stood at `cutoff` — the same computation over the trials and
    /// series that existed then.
    ///
    /// This is what makes "N_eff growth vs trial growth" (§16.2) answerable: the
    /// marginal independence of a window is `(N_eff_now − N_eff_then) / (trials
    /// now − trials then)`, and without a historical value there is nothing to
    /// take a difference against.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn n_eff_before_async(
        &self,
        tenant_id: &str,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<NEff, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let trials: i64 =
            sqlx::query_scalar("SELECT count(*) FROM mlops.trial WHERE tenant_id = $1 AND created_at < $2")
                .bind(tenant_id)
                .bind(cutoff)
                .fetch_one(&mut *tx)
                .await
                .map_err(be)?;
        let rows: Vec<(Vec<chrono::DateTime<chrono::Utc>>, Vec<f64>)> = sqlx::query_as(
            "SELECT s.ts, s.returns FROM mlops.trial_return_series s
               JOIN mlops.trial t ON t.trial_id = s.trial_id
              WHERE s.tenant_id = $1 AND t.created_at < $2
              ORDER BY s.trial_id",
        )
        .bind(tenant_id)
        .bind(cutoff)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        let series: Vec<ReturnSeries> = rows.into_iter().map(|(timestamps, returns)| ReturnSeries { timestamps, returns }).collect();
        Ok(NEff::compute(usize::try_from(trials).unwrap_or(0), &series))
    }

    pub async fn n_eff_async(&self, tenant_id: &str) -> Result<NEff, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let trials: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.trial WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(be)?;
        let rows: Vec<(Vec<chrono::DateTime<chrono::Utc>>, Vec<f64>)> =
            sqlx::query_as("SELECT ts, returns FROM mlops.trial_return_series WHERE tenant_id = $1 ORDER BY trial_id")
                .bind(tenant_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(be)?;
        tx.commit().await.map_err(be)?;
        let series: Vec<ReturnSeries> = rows.into_iter().map(|(timestamps, returns)| ReturnSeries { timestamps, returns }).collect();
        Ok(NEff::compute(usize::try_from(trials).unwrap_or(0), &series))
    }
}

/// The sealed holdout: one evaluation per strategy lineage, ever (SPEC §12.7).
impl PgTrialLedger {
    /// # Errors
    /// A lineage claimed without a recorded result, and backend failures.
    pub async fn claim_sealed_holdout_async(&self, tenant_id: &str, lineage: &str, requested_by: &str) -> Result<HoldoutClaim, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let prior: Option<(Uuid, chrono::DateTime<chrono::Utc>, serde_json::Value)> = sqlx::query_as(
            "SELECT trial_id, called_at, result FROM mlops.sealed_holdout_call WHERE tenant_id = $1 AND strategy_lineage_id = $2",
        )
        .bind(tenant_id)
        .bind(lineage)
        .fetch_optional(&mut *tx)
        .await
        .map_err(be)?;
        // The attempt row IS the claim: the partial unique index admits one first
        // attempt per lineage, so a racing request fails here, before any data.
        let inserted = sqlx::query(
            "INSERT INTO mlops.sealed_holdout_attempt (tenant_id, strategy_lineage_id, requested_by, served_first_result) VALUES ($1,$2,$3,$4)",
        )
        .bind(tenant_id)
        .bind(lineage)
        .bind(requested_by)
        .bind(prior.is_some())
        .execute(&mut *tx)
        .await;
        if let Err(e) = inserted {
            return Err(if e.to_string().contains("uq_holdout_first_attempt") {
                LedgerError::Invalid(claimed_without_result(lineage))
            } else {
                be(e)
            });
        }
        tx.commit().await.map_err(be)?;
        Ok(match prior {
            Some((first_trial, called_at, result)) => HoldoutClaim::Repeat { first_trial, called_at, result },
            None => HoldoutClaim::First,
        })
    }

    /// # Errors
    /// A second result for the lineage, and backend failures.
    pub async fn record_sealed_holdout_async(&self, tenant_id: &str, lineage: &str, trial_id: Uuid, result: &serde_json::Value) -> Result<(), LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        sqlx::query("INSERT INTO mlops.sealed_holdout_call (tenant_id, strategy_lineage_id, trial_id, result) VALUES ($1,$2,$3,$4)")
            .bind(tenant_id)
            .bind(lineage)
            .bind(trial_id)
            .bind(result)
            .execute(&mut *tx)
            .await
            .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(())
    }
}

/// Fixation: daily anchors and whole-ledger verification (SPEC §4.6).
impl PgTrialLedger {
    /// Every tenant with ledger history.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn tenants_async(&self) -> Result<Vec<String>, LedgerError> {
        sqlx::query_scalar("SELECT t FROM mlops.ledger_tenants() AS t ORDER BY t").fetch_all(&self.pool).await.map_err(be)
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn decision_rows_async(&self, tenant_id: &str) -> Result<Vec<DecisionRow>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows = sqlx::query(
            "SELECT decision_id, tenant_id, seq, decision_kind, actor_kind, actor_id, context_hash,
                    candidate_set::text AS candidate_set_text, chosen::text AS chosen_text, policy_id, policy_version,
                    propensity, exploration_flag, decision_tier, prev_hash, row_hash
               FROM mlops.decision WHERE tenant_id = $1 ORDER BY seq",
        )
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(rows
            .into_iter()
            .map(|r| DecisionRow {
                decision_id: r.get("decision_id"),
                tenant_id: r.get("tenant_id"),
                seq: r.get("seq"),
                decision_kind: r.get("decision_kind"),
                actor_kind: r.get("actor_kind"),
                actor_id: r.get("actor_id"),
                context_hash: r.get("context_hash"),
                candidate_set_text: r.get("candidate_set_text"),
                chosen_text: r.get("chosen_text"),
                policy_id: r.get("policy_id"),
                policy_version: r.get("policy_version"),
                propensity: r.get("propensity"),
                exploration_flag: r.get("exploration_flag"),
                decision_tier: r.get("decision_tier"),
                prev_hash: r.get("prev_hash"),
                row_hash: r.get("row_hash"),
            })
            .collect())
    }

    ///
    /// # Errors
    /// Backend failures.
    pub async fn anchor_rows_async(&self, tenant_id: &str) -> Result<Vec<AnchorRow>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows = sqlx::query(
            "SELECT anchor_id, tenant_id, anchor_date, trial_max_seq, trial_head, decision_max_seq, decision_head,
                    signature, key_id, worm_uri
               FROM mlops.ledger_anchor WHERE tenant_id = $1 ORDER BY anchor_date",
        )
        .bind(tenant_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(rows
            .into_iter()
            .map(|r| AnchorRow {
                anchor_id: r.get("anchor_id"),
                body: anchor::AnchorBody {
                    tenant_id: r.get("tenant_id"),
                    anchor_date: r.get("anchor_date"),
                    trial_max_seq: r.get("trial_max_seq"),
                    trial_head: r.get("trial_head"),
                    decision_max_seq: r.get("decision_max_seq"),
                    decision_head: r.get("decision_head"),
                },
                signature: r.get("signature"),
                key_id: r.get("key_id"),
                worm_uri: r.get("worm_uri"),
            })
            .collect())
    }

    /// Anchor a tenant's chains for `date`. Verifies the chains first — a broken
    /// chain is never anchored — then writes the signed document to WORM, then the
    /// row. Returns `None` when the day is already anchored.
    ///
    /// # Errors
    /// A broken chain, a WORM refusal, or a backend failure.
    pub async fn write_anchor_async(
        &self,
        tenant_id: &str,
        date: chrono::NaiveDate,
        signer: &AnchorSigner,
        worm: &dyn WormStore,
    ) -> Result<Option<Uuid>, AnchorError> {
        let abe = |e: LedgerError| AnchorError::Backend(e.to_string());
        let existing = self.anchor_rows_async(tenant_id).await.map_err(abe)?;
        if existing.iter().any(|a| a.body.anchor_date == date) {
            return Ok(None);
        }
        let trials = self.trial_rows_async(tenant_id).await.map_err(abe)?;
        let decisions = self.decision_rows_async(tenant_id).await.map_err(abe)?;
        crate::verify_trials(&trials).map_err(AnchorError::Broken)?;
        anchor::verify_decisions(&decisions).map_err(AnchorError::Broken)?;
        let body = anchor::body_for(tenant_id, date, &trials, &decisions);
        let doc = AnchorDocument { signature: signer.sign(&body), key_id: signer.key_id().to_string(), body };
        let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| AnchorError::Worm(e.to_string()))?;
        let key = format!("ledger-anchors/{}/{}.json", hex::encode(tenant_id.as_bytes()), date);
        let uri = worm.put_once(&key, &bytes)?;
        let sbe = |e: sqlx::Error| AnchorError::Backend(e.to_string());
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(sbe)?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO mlops.ledger_anchor (tenant_id, anchor_date, trial_max_seq, trial_head, decision_max_seq,
                 decision_head, signature, key_id, worm_uri)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING anchor_id",
        )
        .bind(tenant_id)
        .bind(date)
        .bind(doc.body.trial_max_seq)
        .bind(&doc.body.trial_head)
        .bind(doc.body.decision_max_seq)
        .bind(&doc.body.decision_head)
        .bind(&doc.signature)
        .bind(&doc.key_id)
        .bind(&uri)
        .fetch_one(&mut *tx)
        .await
        .map_err(sbe)?;
        tx.commit().await.map_err(sbe)?;
        Ok(Some(id))
    }

    /// Verify everything for one tenant and record the result, pass or fail.
    ///
    /// # Errors
    /// Backend failures (a chain break is a report, not an error).
    pub async fn verify_and_record_async(
        &self,
        tenant_id: &str,
        keys: &[ed25519_dalek::VerifyingKey],
        worm: Option<&dyn WormStore>,
    ) -> Result<VerificationReport, LedgerError> {
        let trials = self.trial_rows_async(tenant_id).await?;
        let mut events: Vec<(Uuid, Vec<EventRow>)> = Vec::new();
        for e in self.all_event_rows_async(tenant_id).await? {
            match events.last_mut() {
                Some((id, v)) if *id == e.trial_id => v.push(e),
                _ => events.push((e.trial_id, vec![e])),
            }
        }
        let decisions = self.decision_rows_async(tenant_id).await?;
        let anchors = self.anchor_rows_async(tenant_id).await?;
        let report = anchor::verify_all(tenant_id, &trials, &events, &decisions, &anchors, keys, worm);
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        sqlx::query(
            "INSERT INTO mlops.ledger_verification (tenant_id, ok, trials, events, decisions, anchors, failures)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(tenant_id)
        .bind(report.ok())
        .bind(i64::try_from(report.trials).unwrap_or(i64::MAX))
        .bind(i64::try_from(report.events).unwrap_or(i64::MAX))
        .bind(i64::try_from(report.decisions).unwrap_or(i64::MAX))
        .bind(i64::try_from(report.anchors).unwrap_or(i64::MAX))
        .bind(serde_json::json!(report.failures))
        .execute(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(report)
    }
}

impl TrialLedger for PgTrialLedger {
    fn register(&self, reg: &Registration<'_>) -> Result<TrialTicket, LedgerError> {
        self.block(self.register_async(reg))
    }

    fn transition(&self, ticket: &mut TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        self.block(self.transition_async(ticket, event))
    }

    fn settle(&self, ticket: TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        self.block(self.settle_async(ticket, event))
    }

    fn prior_trial(&self, tenant_id: &str, config_hash: &str) -> Option<Uuid> {
        self.block(self.prior_trial_async(tenant_id, config_hash)).ok().flatten()
    }

    fn trial_count(&self, tenant_id: &str) -> usize {
        self.block(self.trial_count_async(tenant_id)).map_or(0, |n| n.max(0) as usize)
    }

    fn exploration_fraction(&self, tenant_id: &str) -> f64 {
        self.block(self.exploration_fraction_async(tenant_id)).unwrap_or(0.0)
    }

    fn persist_returns(&self, ticket: &TrialTicket, series: &ReturnSeries) -> Result<String, LedgerError> {
        self.block(self.persist_returns_async(ticket, series))
    }

    fn n_eff(&self, tenant_id: &str) -> Result<NEff, LedgerError> {
        self.block(self.n_eff_async(tenant_id))
    }

    fn claim_sealed_holdout(&self, tenant_id: &str, lineage: &str, requested_by: &str) -> Result<HoldoutClaim, LedgerError> {
        self.block(self.claim_sealed_holdout_async(tenant_id, lineage, requested_by))
    }

    fn record_sealed_holdout(&self, tenant_id: &str, lineage: &str, trial_id: Uuid, result: &serde_json::Value) -> Result<(), LedgerError> {
        self.block(self.record_sealed_holdout_async(tenant_id, lineage, trial_id, result))
    }

    fn define_campaign(
        &self,
        tenant_id: &str,
        def: &crate::campaign::CampaignDefinition,
    ) -> Result<crate::campaign::CampaignHandle, LedgerError> {
        self.block(self.define_campaign_async(tenant_id, def))
    }

    fn campaign_exploration_fraction(&self, tenant_id: &str, campaign_id: Uuid) -> f64 {
        self.block(self.campaign_exploration_fraction_async(tenant_id, campaign_id))
            .unwrap_or(0.0)
    }
    fn record_statistic(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        name: &str,
        value: f64,
        produced_by: &str,
    ) -> Result<(), LedgerError> {
        self.block(self.record_statistic_async(tenant_id, trial_id, name, value, produced_by))
    }

}

impl PgTrialLedger {
    /// DEFINE a campaign (SPEC §10). Every fact is immutable once written: the
    /// table's trigger refuses UPDATE and DELETE, and the unique `(tenant, slug)`
    /// makes a redefinition a conflict rather than a silent overwrite.
    ///
    /// # Errors
    /// An invalid definition, a duplicate slug, and backend failures.
    pub async fn define_campaign_async(
        &self,
        tenant_id: &str,
        def: &crate::campaign::CampaignDefinition,
    ) -> Result<crate::campaign::CampaignHandle, LedgerError> {
        def.validate()?;
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: Option<(Uuid,)> = sqlx::query_as(
            "INSERT INTO mlops.campaign
                 (slug, tenant_id, hypothesis, objective, benchmark, delta_practical, budget,
                  exploration_floor, gates_profile, preference_vector, search_space, created_by,
                  approval_spend_usd)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13::float8::numeric)
             ON CONFLICT (tenant_id, slug) DO NOTHING
             RETURNING campaign_id",
        )
        .bind(&def.slug)
        .bind(tenant_id)
        .bind(&def.hypothesis)
        .bind(&def.objective)
        .bind(&def.benchmark)
        .bind(def.delta_practical)
        .bind(serde_json::to_value(def.budget).map_err(be)?)
        .bind(def.exploration_floor.value())
        .bind(&def.gates_profile)
        .bind(&def.preference_vector)
        .bind(&def.search_space)
        .bind(tenant_id)
        .bind(def.approval_spend_usd)
        .fetch_optional(&mut *tx)
        .await
        .map_err(be)?;
        // DEFINE and the first phase event are one transaction. A campaign row
        // with no `define` event would fold to nothing, and a `define` event
        // with no campaign row has nowhere to point: neither half is allowed to
        // exist alone (SPEC §10, ADR-P2-04).
        if let Some((campaign_id,)) = row {
            sqlx::query(
                "INSERT INTO mlops.campaign_event (campaign_id, tenant_id, state, detail)
                 VALUES ($1, $2, 'define', $3)",
            )
            .bind(campaign_id)
            .bind(tenant_id)
            .bind(serde_json::json!({ "define_hash": def.define_hash() }))
            .execute(&mut *tx)
            .await
            .map_err(be)?;
        }
        tx.commit().await.map_err(be)?;

        let Some((campaign_id,)) = row else {
            return Err(LedgerError::Invalid(format!(
                "campaign '{}' is already defined for this tenant; DEFINE facts are immutable, so a \
                 change is a new campaign",
                def.slug
            )));
        };
        Ok(crate::campaign::CampaignHandle::seal(
            campaign_id,
            tenant_id.to_string(),
            def.clone(),
        ))
    }

    /// Write the `pre` half of an audit record — before the policy check
    /// (SPEC §15, ADR-P2-21, AT-64).
    ///
    /// The row's `seq` and hash chain are computed by `mlops.audit_chain`, so
    /// the ordering and the tamper-evidence are the database's, not this
    /// function's. What this owes the caller is the returned [`PreAudit`], which
    /// is the only thing that can produce a `post` row: `chk_audit_post_has_pre`
    /// refuses a `post` without one.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn record_audit_pre_async(
        &self,
        req: &crate::audit::AuditRequest,
    ) -> Result<crate::audit::PreAudit, LedgerError> {
        let p = &req.principal;
        let mut tx = tenant_tx(&self.pool, &p.tenant_id).await.map_err(be)?;
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO mlops.audit_event
                 (tenant_id, seq, record_phase, action, envelope, actor_kind, actor_id,
                  on_behalf_of, request, verdict, prev_hash, row_hash)
             VALUES ($1, 0, 'pre', $2, $3, $4, $5, $6, $7, 'pending_approval', ''::bytea, ''::bytea)
             RETURNING audit_id",
        )
        .bind(&p.tenant_id)
        .bind(&req.action)
        .bind(req.envelope.as_str())
        .bind(&p.actor_kind)
        .bind(&p.actor_id)
        .bind(&p.on_behalf_of)
        .bind(&req.request)
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(crate::audit::PreAudit::seal(id))
    }

    /// Write the `post` half: what the platform did about the request.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn record_audit_post_async(
        &self,
        req: &crate::audit::AuditRequest,
        pre: crate::audit::PreAudit,
        verdict: crate::audit::AuditVerdict,
        approval_id: Option<Uuid>,
    ) -> Result<(), LedgerError> {
        let p = &req.principal;
        let mut tx = tenant_tx(&self.pool, &p.tenant_id).await.map_err(be)?;
        sqlx::query(
            "INSERT INTO mlops.audit_event
                 (tenant_id, seq, record_phase, action, envelope, actor_kind, actor_id,
                  on_behalf_of, request, verdict, approval_id, pre_audit_id, prev_hash, row_hash)
             VALUES ($1, 0, 'post', $2, $3, $4, $5, $6, $7, $8, $9, $10, ''::bytea, ''::bytea)",
        )
        .bind(&p.tenant_id)
        .bind(&req.action)
        .bind(req.envelope.as_str())
        .bind(&p.actor_kind)
        .bind(&p.actor_id)
        .bind(&p.on_behalf_of)
        .bind(&req.request)
        .bind(verdict.as_str())
        .bind(approval_id)
        .bind(pre.id())
        .execute(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(())
    }

    /// Whether internal-model training is frozen, and why (§14.3, checklist 4.16).
    ///
    /// The latest row wins and the table is append-only, so a freeze cannot be
    /// quietly lifted — only superseded by a row that says who lifted it.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn internal_models_frozen_async(&self) -> Result<Option<String>, LedgerError> {
        let row: Option<(bool, String)> = sqlx::query_as(
            "SELECT frozen, reason FROM mlops.internal_model_freeze ORDER BY set_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(be)?;
        Ok(row.and_then(|(frozen, reason)| frozen.then_some(reason)))
    }

    /// Read the tenant's frozen seed holdout (ADR-P4-02).
    ///
    /// An unfrozen tenant returns an empty holdout, which
    /// [`crate::internal::TrainingSet::build`] treats as "nothing is held out
    /// yet" — correct, because before the first internal model is trained there
    /// is nothing to hold out *from*.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn seed_holdout_async(
        &self,
        tenant_id: &str,
    ) -> Result<crate::internal::SeedHoldout, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: Option<(Vec<Uuid>,)> =
            sqlx::query_as("SELECT trial_ids FROM mlops.seed_holdout WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(row.map_or_else(crate::internal::SeedHoldout::unfrozen, |(ids,)| {
            crate::internal::SeedHoldout::from_stored(ids)
        }))
    }

    /// Freeze the tenant's seed holdout: the first `SEED_HOLDOUT_SIZE` trials in
    /// registration order.
    ///
    /// Idempotent in the only sense that matters — a second call returns the
    /// holdout that is already frozen rather than replacing it. The primary key
    /// and the append-only trigger make that the database's answer as well as
    /// this function's.
    ///
    /// # Errors
    /// Backend failures, or a tenant with no trials to freeze.
    pub async fn freeze_seed_holdout_async(
        &self,
        tenant_id: &str,
        frozen_for: &str,
    ) -> Result<crate::internal::SeedHoldout, LedgerError> {
        let existing = self.seed_holdout_async(tenant_id).await?;
        if existing.is_frozen() {
            return Ok(existing);
        }

        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT trial_id FROM mlops.trial WHERE tenant_id = $1
             ORDER BY registered_at, trial_id
             LIMIT $2",
        )
        .bind(tenant_id)
        .bind(i64::try_from(crate::internal::SEED_HOLDOUT_SIZE).unwrap_or(i64::MAX))
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;

        let ids: Vec<Uuid> = rows.into_iter().map(|(id,)| id).collect();
        if ids.is_empty() {
            tx.commit().await.map_err(be)?;
            return Err(LedgerError::Invalid(
                "this tenant has no trials; there is nothing to hold out and nothing to learn from"
                    .into(),
            ));
        }
        let seq: (i64,) = sqlx::query_as("SELECT count(*) FROM mlops.trial WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(be)?;

        sqlx::query(
            "INSERT INTO mlops.seed_holdout (tenant_id, trial_ids, frozen_for, ledger_seq_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (tenant_id) DO NOTHING",
        )
        .bind(tenant_id)
        .bind(&ids)
        .bind(frozen_for)
        .bind(seq.0)
        .execute(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;

        // Read back rather than trusting the insert: another writer may have won
        // the race, and the holdout that exists is the holdout, not the one this
        // call computed.
        self.seed_holdout_async(tenant_id).await
    }

    /// Record a statistic the platform computed about a trial (ADR-P2-31).
    ///
    /// `name` must be one of the closed set `mlops.trial_statistic` CHECKs — a
    /// misspelled name is a statistic the gate silently never finds, and a gate
    /// that never finds its input is inconclusive forever without anybody
    /// noticing.
    ///
    /// # Errors
    /// A non-finite value, an unknown name (refused by the CHECK), or a backend
    /// failure.
    pub async fn record_statistic_async(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        name: &str,
        value: f64,
        produced_by: &str,
    ) -> Result<(), LedgerError> {
        if !value.is_finite() {
            return Err(LedgerError::Invalid(format!(
                "`{name}` is {value}, which is not an observation"
            )));
        }
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        sqlx::query(
            "INSERT INTO mlops.trial_statistic (tenant_id, trial_id, name, value, produced_by)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(tenant_id)
        .bind(trial_id)
        .bind(name)
        .bind(value)
        .bind(produced_by)
        .execute(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(())
    }

    /// The newest value of each statistic recorded for a trial.
    ///
    /// Newest rather than all: a re-computation appends, and the gate judges the
    /// current belief. The history stays readable in the table for anyone asking
    /// how the belief moved.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn statistics_async(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
    ) -> Result<std::collections::BTreeMap<String, f64>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows: Vec<(String, f64)> = sqlx::query_as(
            "SELECT DISTINCT ON (name) name, value
             FROM mlops.trial_statistic
             WHERE tenant_id = $1 AND trial_id = $2
             ORDER BY name, recorded_at DESC",
        )
        .bind(tenant_id)
        .bind(trial_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(rows.into_iter().collect())
    }

    /// The hash a campaign's DEFINE facts are locked by (§10, Gate 1).
    ///
    /// This is the platform's pre-registration: the hypothesis, the objective,
    /// the benchmark and the declared effect size, hashed before anything ran and
    /// immutable afterwards. Gate 1 asks whether one exists, which is the only
    /// question worth asking — a claim written down after the result is not a
    /// pre-registration whatever it hashes to.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn campaign_define_hash_async(
        &self,
        tenant_id: &str,
        campaign_id: Uuid,
    ) -> Result<Option<String>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        // The hash is recorded on the campaign's own `define` event, written in
        // the same transaction as the campaign row.
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT detail FROM mlops.campaign_event
             WHERE campaign_id = $1 AND state = 'define'
             ORDER BY seq LIMIT 1",
        )
        .bind(campaign_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(row.and_then(|(d,)| {
            d.get("define_hash")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
        }))
    }

    /// One trial's stored return series (INV-18).
    ///
    /// This is what Gates 13 and 14 read. No new runs are executed for either —
    /// they are ledger statistics over series the platform already has, which is
    /// the whole reason they are cheap enough to run on every candidate
    /// (ADR-P2-17).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn return_series_async(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
    ) -> Result<Option<crate::neff::ReturnSeries>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: Option<(Vec<chrono::DateTime<chrono::Utc>>, Vec<f64>)> = sqlx::query_as(
            "SELECT ts, returns FROM mlops.trial_return_series WHERE trial_id = $1",
        )
        .bind(trial_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(row.map(|(timestamps, returns)| crate::neff::ReturnSeries { timestamps, returns }))
    }

    /// Every candidate's return series for one experiment, oldest first.
    ///
    /// Gate 14's family. It is deliberately **every** trial the experiment
    /// registered rather than the ones that looked promising: a stepdown over a
    /// curated family controls nothing, because the curation is the multiple
    /// comparison.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn experiment_family_async(
        &self,
        tenant_id: &str,
        experiment_id: &str,
    ) -> Result<Vec<(Uuid, Vec<f64>)>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows: Vec<(Uuid, Vec<f64>)> = sqlx::query_as(
            "SELECT s.trial_id, s.returns
             FROM mlops.trial_return_series s
             JOIN mlops.trial t ON t.trial_id = s.trial_id
             WHERE t.tenant_id = $1 AND t.experiment_id = $2
             ORDER BY t.registered_at, s.trial_id",
        )
        .bind(tenant_id)
        .bind(experiment_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(rows)
    }

    /// The newest leakage run for a subject (§12.3 Gate 2).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn latest_leakage_run_async(
        &self,
        tenant_id: &str,
        subject: &str,
    ) -> Result<Option<(String, Vec<String>, i64, i64, chrono::DateTime<chrono::Utc>)>, LedgerError>
    {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: Option<(String, Vec<String>, i32, i32, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as(
                "SELECT subject, checks_run, blocking_count, flag_count, finished_at
                 FROM mlops.leakage_run
                 WHERE tenant_id = $1 AND subject = $2
                 ORDER BY finished_at DESC
                 LIMIT 1",
            )
            .bind(tenant_id)
            .bind(subject)
            .fetch_optional(&mut *tx)
            .await
            .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(row.map(|(s, c, b, f, t)| (s, c, i64::from(b), i64::from(f), t)))
    }

    /// How many trials an experiment has registered. Gate 8 and Gate 14's
    /// significance context (INV-3).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn experiment_trial_count_async(
        &self,
        tenant_id: &str,
        experiment_id: &str,
    ) -> Result<i64, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let (n,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM mlops.trial WHERE tenant_id = $1 AND experiment_id = $2",
        )
        .bind(tenant_id)
        .bind(experiment_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(n)
    }

    /// The campaign's platform-held seed (SPEC §12.7, ADR-P2-18).
    ///
    /// This is the one read in the codebase that touches `campaign.platform_seed`,
    /// and it runs as the platform role. `agent_role` holds no grant on the
    /// column, so an agent that reached this code path through some future
    /// accessor would still be refused by the database (AT-63).
    ///
    /// What the seed is *for*: §12.7's countermeasure against gate-hacking. Some
    /// of the randomness a result depends on — the stochastic fills a comparison
    /// replicate uses, the re-evaluation of ASHA's surviving candidates — is
    /// drawn from a number the thing being evaluated cannot see, so it cannot be
    /// searched over.
    ///
    /// # Errors
    /// An unknown campaign, or a backend failure.
    pub async fn platform_seed_async(
        &self,
        tenant_id: &str,
        campaign_id: Uuid,
    ) -> Result<i64, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT platform_seed FROM mlops.campaign WHERE campaign_id = $1")
                .bind(campaign_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(be)?;
        tx.commit().await.map_err(be)?;
        row.map(|r| r.0).ok_or_else(|| {
            LedgerError::Invalid(format!("campaign {campaign_id} is not defined for this tenant"))
        })
    }

    /// Append one phase event and return the `seq` the database assigned it.
    ///
    /// The position is not a parameter. `mlops.check_campaign_transition`
    /// computes it from the current tail and refuses a transition SPEC §10 does
    /// not allow, so two drivers racing on the same campaign cannot interleave
    /// into a log that folds differently for the next reader — one of them loses
    /// the unique index on `(campaign_id, seq)` and retries against the new tail.
    ///
    /// # Errors
    /// An illegal transition (refused by the trigger), a lost race on `seq`, or
    /// a backend failure.
    pub async fn append_campaign_event_async(
        &self,
        tenant_id: &str,
        campaign_id: Uuid,
        event: &crate::phase::CampaignEvent,
    ) -> Result<i64, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let (seq,): (i64,) = sqlx::query_as(
            "INSERT INTO mlops.campaign_event (campaign_id, tenant_id, state, detail)
             VALUES ($1, $2, $3, $4) RETURNING seq",
        )
        .bind(campaign_id)
        .bind(tenant_id)
        .bind(event.phase.as_str())
        .bind(&event.detail)
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(seq)
    }

    /// Every phase event for a campaign, in log order.
    ///
    /// Ordered by `seq`, not `occurred_at`: two events written in the same
    /// millisecond must still fold in the order they were appended.
    ///
    /// # Errors
    /// Backend failures, or a stored `state` this build does not know — which is
    /// a database written by a newer version, and is reported rather than folded
    /// into whatever phase it resembles.
    pub async fn campaign_events_async(
        &self,
        tenant_id: &str,
        campaign_id: Uuid,
    ) -> Result<Vec<crate::phase::CampaignEvent>, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let rows: Vec<(String, serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT state, detail, occurred_at FROM mlops.campaign_event
             WHERE campaign_id = $1 ORDER BY seq",
        )
        .bind(campaign_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;

        rows.into_iter()
            .map(|(state, detail, occurred_at)| {
                let phase = crate::phase::CampaignPhase::from_code(&state).ok_or_else(|| {
                    LedgerError::Invalid(format!(
                        "campaign event state `{state}` is not a phase this build knows"
                    ))
                })?;
                Ok(crate::phase::CampaignEvent { phase, detail, occurred_at })
            })
            .collect()
    }

    /// The campaign's state: the fold of its log, computed fresh every time.
    ///
    /// # Errors
    /// Backend failures, or a log the fold refuses (see [`crate::phase::fold`]).
    pub async fn campaign_state_async(
        &self,
        tenant_id: &str,
        campaign_id: Uuid,
    ) -> Result<crate::phase::CampaignState, LedgerError> {
        let events = self.campaign_events_async(tenant_id, campaign_id).await?;
        crate::phase::fold(&events)
    }

    /// The campaign's achieved exploration fraction, counted over every trial it
    /// dispatched — failures, cache hits and gate-failures included, because the
    /// floor is a floor on looks (§4.5).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn campaign_exploration_fraction_async(
        &self,
        tenant_id: &str,
        campaign_id: Uuid,
    ) -> Result<f64, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE exploration_flag)
               FROM mlops.trial WHERE tenant_id = $1 AND campaign_id = $2",
        )
        .bind(tenant_id)
        .bind(campaign_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        let (total, explored) = row;
        Ok(if total == 0 {
            0.0
        } else {
            explored as f64 / total as f64
        })
    }
}

impl PgTrialLedger {
    /// Record one gate verdict (SPEC §12.3). Append-only; the database repeats
    /// every structural check the record makes.
    ///
    /// # Errors
    /// An invalid record, or a backend failure.
    pub async fn record_gate_async(
        &self,
        tenant_id: &str,
        record: &crate::gates::GateRecord,
    ) -> Result<Uuid, LedgerError> {
        record.validate()?;
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO mlops.gate_verdict
                 (tenant_id, experiment_id, trial_id, campaign_id, profile_id, gate_no, gate_name,
                  passed, statistic, threshold, detail, evidence, n_eff, trial_count_at_eval)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
             RETURNING verdict_id",
        )
        .bind(tenant_id)
        .bind(&record.experiment_id)
        .bind(record.trial_id)
        .bind(record.campaign_id)
        .bind(&record.profile_id)
        .bind(record.gate_no)
        .bind(&record.gate_name)
        .bind(record.passed)
        .bind(record.statistic)
        .bind(record.threshold)
        .bind(&record.detail)
        .bind(serde_json::to_value(&record.evidence).map_err(be)?)
        .bind(record.n_eff)
        .bind(record.trial_count_at_eval)
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(id)
    }

    /// Candidate-level pass rate under one profile: recent window against
    /// everything before it (§16.2).
    ///
    /// A candidate counts as passed when no gate failed it, which is why this
    /// groups by subject rather than averaging over verdicts — a candidate that
    /// cleared fifteen gates and failed one did not pass, and counting verdicts
    /// would score it 15/16.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn pass_rate_async(
        &self,
        tenant_id: &str,
        profile_id: &str,
        window_days: i64,
    ) -> Result<crate::gates::PassRate, LedgerError> {
        let mut tx = tenant_tx(&self.pool, tenant_id).await.map_err(be)?;
        let row: (i64, i64, i64, i64) = sqlx::query_as(
            "WITH subject AS (
               SELECT coalesce(experiment_id, trial_id::text) AS subject_id,
                      max(decided_at)                          AS last_at,
                      bool_and(passed)                         AS passed
                 FROM mlops.gate_verdict
                WHERE tenant_id = $1 AND profile_id = $2
                GROUP BY 1
             )
             SELECT
               count(*) FILTER (WHERE last_at >= now() - make_interval(days => $3::int)),
               count(*) FILTER (WHERE last_at >= now() - make_interval(days => $3::int) AND passed),
               count(*) FILTER (WHERE last_at <  now() - make_interval(days => $3::int)),
               count(*) FILTER (WHERE last_at <  now() - make_interval(days => $3::int) AND passed)
             FROM subject",
        )
        .bind(tenant_id)
        .bind(profile_id)
        .bind(i32::try_from(window_days).unwrap_or(30))
        .fetch_one(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        let (recent_decided, recent_passed, baseline_decided, baseline_passed) = row;
        Ok(crate::gates::PassRate { recent_decided, recent_passed, baseline_decided, baseline_passed })
    }
}

impl crate::gates::GateLog for PgTrialLedger {
    fn record_gate(&self, tenant_id: &str, record: &crate::gates::GateRecord) -> Result<Uuid, LedgerError> {
        self.block(self.record_gate_async(tenant_id, record))
    }

    fn pass_rate(&self, tenant_id: &str, profile_id: &str, window_days: i64) -> Result<crate::gates::PassRate, LedgerError> {
        self.block(self.pass_rate_async(tenant_id, profile_id, window_days))
    }
}

impl DecisionLog for PgTrialLedger {
    fn log_decision(&self, ctx: &DispatchContext, decision: &Decision) -> Result<Uuid, LedgerError> {
        self.block(self.log_decision_async(ctx, decision))
    }
}
