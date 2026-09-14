//! The Trial Ledger (SPEC §4): write-before-execute, propensity-logged,
//! hash-chained, append-only.
//!
//! INV-16 is enforced by a type: anything that dispatches compute demands a
//! [`TrialTicket`], and a ticket can only be minted inside this crate, by
//! [`TrialLedger::register`], after the registration row is written. There is no
//! public, test-only, or `From` constructor (ADR-P0-02).
//!
//! The ledger is not the artifact cache: a deduplicated dispatch still registers a
//! trial and settles as [`TrialState::Deduplicated`] naming the prior trial, because
//! trial accounting counts looks, not executions (ADR-P0-01, SPEC §12.4).

pub mod anchor;
pub mod audit;
pub mod campaign;
pub mod capital;
pub mod gates;
pub mod holdout;
pub mod internal;
pub mod neff;
pub mod outcome;
pub mod pg;
pub mod phase;
pub mod portfolio;
pub mod presets;
pub mod state;
pub mod tiers;
pub mod trajectory;

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use audit::{AuditRequest, AuditVerdict, Envelope, PreAudit, Principal};
pub use capital::AllowedFraction;
pub use internal::{Answer, Rung, SeedHoldout, TrainingRange, TrainingSet};
pub use outcome::OutcomeVector;
pub use phase::{fold as fold_campaign, CampaignEvent, CampaignPhase, CampaignState};
pub use state::{Censoring, TerminalReason, TrialState};

pub const GENESIS_HASH: [u8; 32] = [0u8; 32];

/// Reserved policy for dispatch paths that predate propensity logging and
/// pre-registration (CLAUDE.md §6). Counted in N_eff; excluded from every
/// off-policy estimator.
pub const LEGACY_POLICY: &str = "legacy_unlogged";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Agent,
    Scheduler,
}

impl ActorKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Scheduler => "scheduler",
        }
    }
}

/// Standing provenance of a dispatching session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DispatchContext {
    pub tenant_id: String,
    pub campaign_id: Option<Uuid>,
    pub experiment_id: Option<String>,
    pub actor_kind: ActorKind,
    pub actor_id: String,
    /// Principal attribution: "agent X acting for user Y".
    pub on_behalf_of: Option<String>,
    pub policy_id: String,
    pub policy_version: i32,
    /// Pre-declared practically-meaningful effect size. `None` only under
    /// [`LEGACY_POLICY`], where it means "never declared" rather than `0.0`.
    pub delta_practical: Option<f64>,
}

impl DispatchContext {
    #[must_use]
    pub fn human(tenant_id: impl Into<String>, actor_id: impl Into<String>, delta_practical: f64) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            campaign_id: None,
            experiment_id: None,
            actor_kind: ActorKind::Human,
            actor_id: actor_id.into(),
            on_behalf_of: None,
            policy_id: "human_direct".into(),
            policy_version: 1,
            delta_practical: Some(delta_practical),
        }
    }

    /// The only way to register without a propensity or declared effect size. The
    /// missing declaration is a defect of the calling path; this records it.
    #[must_use]
    pub fn legacy(tenant_id: impl Into<String>, actor_kind: ActorKind, actor_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            campaign_id: None,
            experiment_id: None,
            actor_kind,
            actor_id: actor_id.into(),
            on_behalf_of: None,
            policy_id: LEGACY_POLICY.into(),
            policy_version: 1,
            delta_practical: None,
        }
    }

    #[must_use]
    pub fn is_legacy(&self) -> bool {
        self.policy_id == LEGACY_POLICY
    }

    #[must_use]
    pub fn with_experiment(mut self, experiment_id: impl Into<String>) -> Self {
        self.experiment_id = Some(experiment_id.into());
        self
    }

    #[must_use]
    pub fn with_campaign(mut self, campaign_id: Uuid) -> Self {
        self.campaign_id = Some(campaign_id);
        self
    }

    #[must_use]
    pub fn with_policy(mut self, policy_id: impl Into<String>, version: i32) -> Self {
        self.policy_id = policy_id.into();
        self.policy_version = version;
        self
    }

    #[must_use]
    pub fn on_behalf_of(mut self, principal: impl Into<String>) -> Self {
        self.on_behalf_of = Some(principal.into());
        self
    }
}

/// What is being run. Built by the dispatching crate (backtest, training, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrialSubject {
    pub config_hash: String,
    pub config: serde_json::Value,
    pub dataset_id: String,
    pub split_spec_id: Option<String>,
    pub code_hash: String,
    pub image_digest: String,
    pub seed_set: Vec<i64>,
    pub non_reproducible: bool,
    pub overlapping_labels_unweighted: bool,
    pub split_overrides: serde_json::Value,
    pub planned_steps: Option<i32>,
}

/// One dispatch decision presented to the ledger before any compute runs.
#[derive(Clone, Debug)]
pub struct Registration<'a> {
    pub ctx: &'a DispatchContext,
    pub subject: &'a TrialSubject,
    pub propensity: Option<f64>,
    pub exploration_flag: bool,
    pub candidate_set_hash: Option<String>,
    pub parent_trial_id: Option<Uuid>,
    pub hypothesis_id: Option<Uuid>,
    pub supersedes: Option<Uuid>,
}

impl<'a> Registration<'a> {
    #[must_use]
    pub fn new(ctx: &'a DispatchContext, subject: &'a TrialSubject, propensity: f64) -> Self {
        Self {
            ctx,
            subject,
            propensity: Some(propensity),
            exploration_flag: false,
            candidate_set_hash: None,
            parent_trial_id: None,
            hypothesis_id: None,
            supersedes: None,
        }
    }

    /// A registration with no propensity. Accepted only under [`LEGACY_POLICY`].
    #[must_use]
    pub fn unlogged(ctx: &'a DispatchContext, subject: &'a TrialSubject) -> Self {
        Self { propensity: None, ..Self::new(ctx, subject, 1.0) }
    }

    #[must_use]
    pub fn exploration(mut self) -> Self {
        self.exploration_flag = true;
        self
    }

    #[must_use]
    pub fn candidate_set(mut self, hash: impl Into<String>) -> Self {
        self.candidate_set_hash = Some(hash.into());
        self
    }

    #[must_use]
    pub fn parent(mut self, trial_id: Uuid) -> Self {
        self.parent_trial_id = Some(trial_id);
        self
    }

    /// The pre-registration hash: effect size, policy and campaign locked before
    /// the first execution.
    #[must_use]
    pub fn prereg_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.subject.config_hash.as_bytes());
        if let Some(d) = self.ctx.delta_practical {
            h.update(d.to_be_bytes());
        }
        h.update(self.ctx.policy_id.as_bytes());
        h.update(self.ctx.policy_version.to_be_bytes());
        h.update(self.ctx.campaign_id.map(|c| c.to_string()).unwrap_or_default().as_bytes());
        h.update(self.ctx.experiment_id.as_deref().unwrap_or_default().as_bytes());
        format!("sha256:{}", hex::encode(h.finalize()))
    }

    pub(crate) fn validate(&self) -> Result<(), LedgerError> {
        let legacy = self.ctx.is_legacy();
        if self.propensity.is_none() && !legacy {
            return Err(LedgerError::MissingPropensity);
        }
        if let Some(p) = self.propensity {
            if !(p > 0.0 && p <= 1.0 && p.is_finite()) {
                return Err(LedgerError::BadPropensity(p));
            }
        }
        match self.ctx.delta_practical {
            None if !legacy => return Err(LedgerError::MissingDeltaPractical),
            Some(d) if !d.is_finite() => return Err(LedgerError::MissingDeltaPractical),
            _ => {}
        }
        if self.ctx.campaign_id.is_none() && self.ctx.experiment_id.is_none() && !legacy {
            return Err(LedgerError::NoCampaign);
        }
        Ok(())
    }
}

/// Proof that a trial was registered before compute was dispatched. Mintable only
/// inside this crate. Do not add a constructor.
#[derive(Debug)]
pub struct TrialTicket {
    trial_id: Uuid,
    tenant_id: String,
    config_hash: String,
    state: TrialState,
}

impl TrialTicket {
    #[must_use]
    pub fn trial_id(&self) -> Uuid {
        self.trial_id
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    #[must_use]
    pub fn config_hash(&self) -> &str {
        &self.config_hash
    }

    #[must_use]
    pub fn state(&self) -> TrialState {
        self.state
    }
}

pub(crate) fn mint(trial_id: Uuid, tenant_id: String, config_hash: String) -> TrialTicket {
    TrialTicket { trial_id, tenant_id, config_hash, state: TrialState::Registered }
}

/// One lifecycle transition.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrialEvent {
    pub state: Option<TrialState>,
    pub terminal_reason: Option<TerminalReason>,
    pub censoring: Option<Censoring>,
    pub censor_at_step: Option<i32>,
    pub run_id: Option<String>,
    pub deduplicated_of: Option<Uuid>,
    pub outcome: Option<OutcomeVector>,
    pub gate_profile: Option<String>,
    pub gate_results: Option<serde_json::Value>,
    pub gpu_seconds: Option<f64>,
    pub cpu_seconds: Option<f64>,
    pub peak_vram_bytes: Option<i64>,
    pub usd_cost: Option<f64>,
    pub artifacts_uri: Option<String>,
    pub metrics_uri: Option<String>,
    pub predictions_uri: Option<String>,
    pub returns_uri: Option<String>,
    pub detail: Option<serde_json::Value>,
}

impl TrialEvent {
    #[must_use]
    pub fn to(state: TrialState) -> Self {
        Self { state: Some(state), ..Self::default() }
    }

    #[must_use]
    pub fn completed(run_id: impl Into<String>, outcome: Option<OutcomeVector>) -> Self {
        Self { run_id: Some(run_id.into()), outcome, ..Self::to(TrialState::Completed) }
    }

    #[must_use]
    pub fn deduplicated(prior: Uuid, run_id: impl Into<String>) -> Self {
        Self { deduplicated_of: Some(prior), run_id: Some(run_id.into()), ..Self::to(TrialState::Deduplicated) }
    }

    /// A terminal failure. Censoring follows from the reason.
    #[must_use]
    pub fn failed(reason: TerminalReason, detail: impl Into<String>) -> Self {
        Self {
            terminal_reason: Some(reason),
            censoring: Some(reason.censoring()),
            detail: Some(serde_json::json!({ "message": detail.into() })),
            ..Self::to(TrialState::Failed)
        }
    }

    #[must_use]
    pub fn with_returns(mut self, uri: impl Into<String>) -> Self {
        self.returns_uri = Some(uri.into());
        self
    }

    #[must_use]
    pub fn with_predictions(mut self, uri: impl Into<String>) -> Self {
        self.predictions_uri = Some(uri.into());
        self
    }

    #[must_use]
    pub fn at_step(mut self, step: i32) -> Self {
        self.censor_at_step = Some(step);
        self
    }

    fn effective_censoring(&self) -> Censoring {
        self.censoring.or_else(|| self.terminal_reason.map(TerminalReason::censoring)).unwrap_or(Censoring::None)
    }

    pub(crate) fn validate(&self, from: TrialState) -> Result<TrialState, LedgerError> {
        let to = self.state.ok_or(LedgerError::Invalid("event has no target state".into()))?;
        if !from.can_transition_to(to) {
            return Err(LedgerError::IllegalTransition { from, to });
        }
        let censoring = self.effective_censoring();
        if let Some(r) = self.terminal_reason {
            if r.censoring() != censoring && !(r == TerminalReason::IntegrityRejected && censoring == Censoring::None) {
                return Err(LedgerError::Invalid(format!("reason {} implies censoring {}", r.as_str(), r.censoring().as_str())));
            }
        }
        match to {
            TrialState::Failed if censoring == Censoring::None || self.terminal_reason.is_none() => {
                return Err(LedgerError::Invalid("a failed trial must carry a terminal_reason and censoring (INV-17)".into()));
            }
            TrialState::Completed | TrialState::CompletedPass | TrialState::CompletedFail if censoring != Censoring::None => {
                return Err(LedgerError::Invalid("a completed trial is not censored".into()));
            }
            TrialState::Deduplicated if self.deduplicated_of.is_none() => {
                return Err(LedgerError::Invalid("deduplicated must name the prior trial (SPEC §9)".into()));
            }
            _ => {}
        }
        Ok(to)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("propensity is required: a deterministic logging policy makes off-policy evaluation formally impossible (SPEC §4.2·6); use policy_id='legacy_unlogged' only for pre-existing paths")]
    MissingPropensity,
    #[error("propensity {0} is outside (0, 1]")]
    BadPropensity(f64),
    #[error("delta_practical is REQUIRED and has no default (SPEC §4.1)")]
    MissingDeltaPractical,
    #[error("a trial must belong to a campaign or experiment unless it is legacy_unlogged")]
    NoCampaign,
    #[error("illegal transition {from:?} -> {to:?} (SPEC §9)")]
    IllegalTransition { from: TrialState, to: TrialState },
    #[error("invalid event: {0}")]
    Invalid(String),
    #[error("unknown trial {0}")]
    UnknownTrial(Uuid),
    #[error("ledger backend: {0}")]
    Backend(String),
}

/// The Trial Ledger. `register` must be durable before it returns.
pub trait TrialLedger: Send + Sync {
    ///
    /// # Errors
    /// Refused registrations dispatch nothing.
    fn register(&self, reg: &Registration<'_>) -> Result<TrialTicket, LedgerError>;

    /// A non-terminal transition.
    ///
    /// # Errors
    /// Illegal transitions and backend failures.
    fn transition(&self, ticket: &mut TrialTicket, event: TrialEvent) -> Result<(), LedgerError>;

    /// A terminal transition; consumes the ticket so it cannot authorize more work.
    ///
    /// # Errors
    /// Illegal transitions and backend failures.
    fn settle(&self, ticket: TrialTicket, event: TrialEvent) -> Result<(), LedgerError>;

    /// The most recent terminal, non-deduplicated trial with this config hash —
    /// the target a deduplicated dispatch points at (AT-25).
    fn prior_trial(&self, tenant_id: &str, config_hash: &str) -> Option<Uuid>;

    /// Every look on this stream: failures, cache hits and exploration included.
    fn trial_count(&self, tenant_id: &str) -> usize;

    fn exploration_fraction(&self, tenant_id: &str) -> f64;

    /// Persist a trial's out-of-sample return series (INV-18) and return the URI its
    /// terminal event chains. A series is written once per trial.
    ///
    /// # Errors
    /// Malformed series, a second write, and backend failures.
    fn persist_returns(&self, ticket: &TrialTicket, series: &neff::ReturnSeries) -> Result<String, LedgerError>;

    /// The platform-computed `N_eff` over the tenant's whole ledger (INV-22). The
    /// only way to obtain an [`neff::NEff`].
    ///
    /// # Errors
    /// Backend failures.
    fn n_eff(&self, tenant_id: &str) -> Result<neff::NEff, LedgerError>;

    /// Record a statistic the platform computed about a trial (ADR-P2-31).
    ///
    /// This is how a number the funnel computes reaches a gate that runs later,
    /// in a different process. The alternative — passing it through the gate
    /// job's manifest — would make the statistic something the submitter
    /// supplies, and a gate stack whose inputs the candidate chooses is not a
    /// gate stack.
    ///
    /// `name` is drawn from the closed set `mlops.trial_statistic` CHECKs.
    ///
    /// # Errors
    /// A non-finite value, an unknown name, or a backend failure.
    fn record_statistic(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        name: &str,
        value: f64,
        produced_by: &str,
    ) -> Result<(), LedgerError>;

    /// Claim a strategy lineage's one sealed-holdout evaluation, or receive the
    /// evaluation it already had (SPEC §12.7). Every request is logged.
    ///
    /// # Errors
    /// A lineage claimed without a recorded result, and backend failures.
    fn claim_sealed_holdout(&self, tenant_id: &str, lineage: &str, requested_by: &str) -> Result<holdout::HoldoutClaim, LedgerError>;

    /// Record the result of a claimed evaluation. Written once per lineage.
    ///
    /// # Errors
    /// A second result, a result without a claim, and backend failures.
    fn record_sealed_holdout(&self, tenant_id: &str, lineage: &str, trial_id: Uuid, result: &serde_json::Value) -> Result<(), LedgerError>;

    /// DEFINE a campaign (SPEC §10). The only way to obtain a
    /// [`campaign::CampaignHandle`], so a dispatch cannot name a campaign that
    /// was never defined — the same sealing as [`TrialTicket`].
    ///
    /// # Errors
    /// A definition that fails [`campaign::CampaignDefinition::validate`] (a
    /// missing `delta_practical` above all), a duplicate slug, and backend
    /// failures.
    fn define_campaign(&self, tenant_id: &str, def: &campaign::CampaignDefinition) -> Result<campaign::CampaignHandle, LedgerError>;

    /// The campaign's achieved exploration fraction, for the floor check.
    /// Defaults to the tenant-wide fraction where a backend cannot scope it.
    fn campaign_exploration_fraction(&self, tenant_id: &str, _campaign_id: Uuid) -> f64 {
        self.exploration_fraction(tenant_id)
    }
}

/// The statistic names `mlops.trial_statistic` accepts.
///
/// A closed set, stated here and CHECKed by the database, because a misspelled
/// name is a statistic the gate silently never finds — and a gate that never
/// finds its input is inconclusive forever without anybody noticing.
pub const TRIAL_STATISTICS: &[&str] = &[
    "cpcv_p05_sharpe",
    "walk_forward_sharpe",
    "walk_forward_regimes",
    "pbo",
    "deflated_sharpe",
    "permutation_p_value",
    "breakeven_cost_multiple",
    "prereg_hash_present",
];

impl<T: TrialLedger + ?Sized> TrialLedger for std::sync::Arc<T> {
    fn register(&self, reg: &Registration<'_>) -> Result<TrialTicket, LedgerError> {
        (**self).register(reg)
    }
    fn transition(&self, ticket: &mut TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        (**self).transition(ticket, event)
    }
    fn settle(&self, ticket: TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        (**self).settle(ticket, event)
    }
    fn prior_trial(&self, tenant_id: &str, config_hash: &str) -> Option<Uuid> {
        (**self).prior_trial(tenant_id, config_hash)
    }
    fn trial_count(&self, tenant_id: &str) -> usize {
        (**self).trial_count(tenant_id)
    }
    fn exploration_fraction(&self, tenant_id: &str) -> f64 {
        (**self).exploration_fraction(tenant_id)
    }
    fn persist_returns(&self, ticket: &TrialTicket, series: &neff::ReturnSeries) -> Result<String, LedgerError> {
        (**self).persist_returns(ticket, series)
    }
    fn n_eff(&self, tenant_id: &str) -> Result<neff::NEff, LedgerError> {
        (**self).n_eff(tenant_id)
    }
    fn claim_sealed_holdout(&self, tenant_id: &str, lineage: &str, requested_by: &str) -> Result<holdout::HoldoutClaim, LedgerError> {
        (**self).claim_sealed_holdout(tenant_id, lineage, requested_by)
    }
    fn record_sealed_holdout(&self, tenant_id: &str, lineage: &str, trial_id: Uuid, result: &serde_json::Value) -> Result<(), LedgerError> {
        (**self).record_sealed_holdout(tenant_id, lineage, trial_id, result)
    }
    fn define_campaign(&self, tenant_id: &str, def: &campaign::CampaignDefinition) -> Result<campaign::CampaignHandle, LedgerError> {
        (**self).define_campaign(tenant_id, def)
    }
    fn campaign_exploration_fraction(&self, tenant_id: &str, campaign_id: Uuid) -> f64 {
        (**self).campaign_exploration_fraction(tenant_id, campaign_id)
    }

    fn record_statistic(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        name: &str,
        value: f64,
        produced_by: &str,
    ) -> Result<(), LedgerError> {
        (**self).record_statistic(tenant_id, trial_id, name, value, produced_by)
    }

}

/// A trial registration row as read back.
#[derive(Clone, Debug, PartialEq)]
pub struct TrialRow {
    pub trial_id: Uuid,
    pub tenant_id: String,
    pub campaign_id: Option<Uuid>,
    pub experiment_id: Option<String>,
    pub seq: i64,
    pub prev_hash: Vec<u8>,
    pub row_hash: Vec<u8>,
    pub config_hash: String,
    pub dataset_id: String,
    pub code_hash: String,
    pub image_digest: String,
    pub seed_set: Vec<i64>,
    pub prereg_hash: String,
    pub delta_practical: Option<f64>,
    pub actor_kind: String,
    pub actor_id: String,
    pub on_behalf_of: Option<String>,
    pub policy_id: String,
    pub policy_version: i32,
    pub candidate_set_hash: Option<String>,
    pub propensity: Option<f64>,
    pub exploration_flag: bool,
    pub supersedes: Option<Uuid>,
}

impl TrialRow {
    /// Mirrors `mlops.trial_chain()` byte for byte.
    #[must_use]
    pub fn recompute_hash(&self) -> Vec<u8> {
        let seeds = self.seed_set.iter().map(ToString::to_string).collect::<Vec<_>>().join(",");
        let mut h = Sha256::new();
        h.update(&self.prev_hash);
        h.update(self.trial_id.to_string());
        h.update(self.seq.to_string());
        h.update(&self.tenant_id);
        h.update(self.campaign_id.map(|c| c.to_string()).unwrap_or_default());
        h.update(self.experiment_id.as_deref().unwrap_or_default());
        h.update(&self.config_hash);
        h.update(&self.dataset_id);
        h.update(&self.code_hash);
        h.update(&self.image_digest);
        h.update(seeds);
        h.update(&self.prereg_hash);
        if let Some(d) = self.delta_practical {
            h.update(d.to_be_bytes());
        }
        h.update(&self.actor_kind);
        h.update(&self.actor_id);
        h.update(self.on_behalf_of.as_deref().unwrap_or_default());
        h.update(&self.policy_id);
        h.update(self.policy_version.to_string());
        h.update(self.candidate_set_hash.as_deref().unwrap_or_default());
        if let Some(p) = self.propensity {
            h.update(p.to_be_bytes());
        }
        h.update(if self.exploration_flag { "true" } else { "false" });
        h.update(self.supersedes.map(|s| s.to_string()).unwrap_or_default());
        h.finalize().to_vec()
    }
}

/// A lifecycle event row as read back.
#[derive(Clone, Debug, PartialEq)]
pub struct EventRow {
    pub event_id: Uuid,
    pub trial_id: Uuid,
    pub event_seq: i32,
    pub state: String,
    pub censoring: String,
    pub terminal_reason: Option<String>,
    pub censor_at_step: Option<i32>,
    pub run_id: Option<String>,
    pub deduplicated_of: Option<Uuid>,
    pub outcome: Option<OutcomeVector>,
    pub outcome_digest: Option<String>,
    pub returns_uri: Option<String>,
    pub predictions_uri: Option<String>,
    pub prev_hash: Vec<u8>,
    pub row_hash: Vec<u8>,
}

impl EventRow {
    /// Mirrors `mlops.trial_event_chain()` byte for byte.
    #[must_use]
    pub fn recompute_hash(&self) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(&self.prev_hash);
        h.update(self.event_id.to_string());
        h.update(self.trial_id.to_string());
        h.update(self.event_seq.to_string());
        h.update(&self.state);
        h.update(&self.censoring);
        h.update(self.terminal_reason.as_deref().unwrap_or_default());
        h.update(self.censor_at_step.map(|s| s.to_string()).unwrap_or_default());
        h.update(self.run_id.as_deref().unwrap_or_default());
        h.update(self.deduplicated_of.map(|d| d.to_string()).unwrap_or_default());
        h.update(self.outcome_digest.as_deref().unwrap_or_default());
        h.update(self.returns_uri.as_deref().unwrap_or_default());
        h.update(self.predictions_uri.as_deref().unwrap_or_default());
        h.finalize().to_vec()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainBreak {
    pub id: Uuid,
    pub seq: i64,
    pub reason: String,
}

impl std::fmt::Display for ChainBreak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ledger chain broken at seq {} ({}): {}", self.seq, self.id, self.reason)
    }
}

impl std::error::Error for ChainBreak {}

/// Verify a tenant's trial stream (ascending `seq`).
///
/// # Errors
/// The first row whose sequence, linkage or content hash disagrees.
pub fn verify_trials(rows: &[TrialRow]) -> Result<usize, ChainBreak> {
    let mut prev = GENESIS_HASH.to_vec();
    for (i, r) in rows.iter().enumerate() {
        let expected = i as i64;
        let brk = |reason: String| ChainBreak { id: r.trial_id, seq: r.seq, reason };
        if r.seq != expected {
            return Err(brk(format!("expected seq {expected}, found {}", r.seq)));
        }
        if r.prev_hash != prev {
            return Err(brk("prev_hash does not match the previous row".into()));
        }
        if r.recompute_hash() != r.row_hash {
            return Err(brk("row_hash does not match the row's contents".into()));
        }
        prev.clone_from(&r.row_hash);
    }
    Ok(rows.len())
}

/// Verify one trial's events (ascending `event_seq`) against its registration row.
///
/// # Errors
/// The first inconsistent event, including an outcome whose digest no longer
/// matches the stored values.
pub fn verify_events(trial: &TrialRow, events: &[EventRow]) -> Result<usize, ChainBreak> {
    let mut prev = trial.row_hash.clone();
    let mut state = TrialState::Registered;
    for (i, e) in events.iter().enumerate() {
        let brk = |reason: String| ChainBreak { id: e.event_id, seq: i64::from(e.event_seq), reason };
        if e.trial_id != trial.trial_id || e.event_seq != i as i32 {
            return Err(brk("event out of sequence or for another trial".into()));
        }
        if e.prev_hash != prev {
            return Err(brk("prev_hash does not match the previous event".into()));
        }
        if let (Some(o), Some(d)) = (&e.outcome, &e.outcome_digest) {
            if &o.digest() != d {
                return Err(brk("outcome values no longer match their chained digest".into()));
            }
        }
        if e.outcome.is_some() != e.outcome_digest.is_some() {
            return Err(brk("outcome and digest must be present together".into()));
        }
        if e.recompute_hash() != e.row_hash {
            return Err(brk("row_hash does not match the event's contents".into()));
        }
        let to = TrialState::parse(&e.state).ok_or_else(|| brk(format!("unknown state {}", e.state)))?;
        if !state.can_transition_to(to) {
            return Err(brk(format!("illegal transition {state:?} -> {to:?}")));
        }
        state = to;
        prev.clone_from(&e.row_hash);
    }
    Ok(events.len())
}

/// Canonical hash over a candidate set (SPEC §4.4).
///
/// # Errors
/// If the candidates cannot be represented as JSON.
pub fn candidate_set_hash<T: Serialize>(candidates: &[T]) -> serde_json::Result<String> {
    dataplane::content_hash(&candidates)
}

// ── decisions (§4.4) ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Propose,
    Select,
    Prune,
    Reallocate,
    Stop,
    Promote,
    Reject,
}

impl DecisionKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Propose => "propose",
            Self::Select => "select",
            Self::Prune => "prune",
            Self::Reallocate => "reallocate",
            Self::Stop => "stop",
            Self::Promote => "promote",
            Self::Reject => "reject",
        }
    }
}

/// Which cold-start tier produced a decision (§13.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTier {
    Rule,
    Shrunk,
    Learned,
}

impl DecisionTier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Shrunk => "shrunk",
            Self::Learned => "learned",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub kind: DecisionKind,
    pub context_hash: String,
    pub context_uri: Option<String>,
    /// ALL options considered, including the rejected ones.
    pub candidate_set: Vec<serde_json::Value>,
    /// JSON `null` means "decided not to act".
    pub chosen: serde_json::Value,
    pub propensity: Option<f64>,
    pub exploration_flag: bool,
    pub decision_tier: DecisionTier,
    pub rationale: Option<String>,
}

impl Decision {
    pub(crate) fn validate(&self, ctx: &DispatchContext) -> Result<(), LedgerError> {
        if self.propensity.is_none() && !ctx.is_legacy() {
            return Err(LedgerError::MissingPropensity);
        }
        if let Some(p) = self.propensity {
            if !(p > 0.0 && p <= 1.0 && p.is_finite()) {
                return Err(LedgerError::BadPropensity(p));
            }
        }
        if self.candidate_set.is_empty() {
            return Err(LedgerError::Invalid("candidate_set must list every option considered".into()));
        }
        if ctx.campaign_id.is_none() && ctx.experiment_id.is_none() {
            return Err(LedgerError::NoCampaign);
        }
        Ok(())
    }
}

pub trait DecisionLog: Send + Sync {
    ///
    /// # Errors
    /// Missing propensity, empty candidate set, or backend failure.
    fn log_decision(&self, ctx: &DispatchContext, decision: &Decision) -> Result<Uuid, LedgerError>;
}

// ── in-memory implementation ──────────────────────────────────────────────────

struct MemTrial {
    row: TrialRow,
    events: Vec<EventRow>,
}

/// A real in-process ledger: it chains, validates the state machine and refuses
/// unlogged propensities. Used by tests and offline runs.
#[derive(Default)]
pub struct InMemoryLedger {
    trials: Mutex<Vec<MemTrial>>,
    decisions: Mutex<Vec<(DispatchContext, Decision)>>,
    returns: Mutex<Vec<(Uuid, String, neff::ReturnSeries)>>,
    holdouts: Mutex<std::collections::HashMap<(String, String), holdout::MemHoldout>>,
    /// `(tenant_id, slug)` is unique, mirroring `mlops.campaign`.
    campaigns: Mutex<Vec<((String, String), campaign::CampaignDefinition)>>,
    gate_verdicts: Mutex<Vec<(String, chrono::DateTime<chrono::Utc>, gates::GateRecord)>>,
    /// `(tenant, trial, name) -> value`, newest write wins, mirroring the
    /// `DISTINCT ON … ORDER BY recorded_at DESC` the Postgres reader uses.
    statistics: Mutex<std::collections::BTreeMap<(String, Uuid, String), f64>>,
}

impl InMemoryLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn rows(&self, tenant_id: &str) -> Vec<TrialRow> {
        self.trials.lock().expect("ledger poisoned").iter().filter(|t| t.row.tenant_id == tenant_id).map(|t| t.row.clone()).collect()
    }

    #[must_use]
    pub fn events(&self, trial_id: Uuid) -> Vec<EventRow> {
        self.trials.lock().expect("ledger poisoned").iter().find(|t| t.row.trial_id == trial_id).map(|t| t.events.clone()).unwrap_or_default()
    }

    #[must_use]
    pub fn state_of(&self, trial_id: Uuid) -> Option<(TrialState, Censoring)> {
        let trials = self.trials.lock().expect("ledger poisoned");
        let t = trials.iter().find(|t| t.row.trial_id == trial_id)?;
        Some(t.events.last().map_or((TrialState::Registered, Censoring::None), |e| {
            (TrialState::parse(&e.state).unwrap_or(TrialState::Registered), Censoring::parse(&e.censoring).unwrap_or(Censoring::None))
        }))
    }

    /// Logged sealed-holdout requests for a lineage (the claim and every repeat).
    #[must_use]
    pub fn holdout_attempts(&self, tenant_id: &str, lineage: &str) -> usize {
        self.holdouts
            .lock()
            .expect("ledger poisoned")
            .get(&(tenant_id.to_string(), lineage.to_string()))
            .map_or(0, |h| h.attempts)
    }

    #[must_use]
    pub fn decisions(&self) -> Vec<Decision> {
        self.decisions.lock().expect("ledger poisoned").iter().map(|(_, d)| d.clone()).collect()
    }

    fn append(&self, ticket_id: Uuid, event: &TrialEvent, from: TrialState) -> Result<TrialState, LedgerError> {
        let to = event.validate(from)?;
        // Mirrors chk_event_returns_persisted (migration 0043).
        if matches!(to, TrialState::Completed | TrialState::CompletedPass | TrialState::CompletedFail)
            && event.outcome.is_some_and(|o| o.sharpe_net.is_some())
            && event.returns_uri.is_none()
        {
            return Err(LedgerError::Invalid("a completion reporting a Sharpe must carry its persisted return series (INV-18)".into()));
        }
        let mut trials = self.trials.lock().expect("ledger poisoned");
        let t = trials.iter_mut().find(|t| t.row.trial_id == ticket_id).ok_or(LedgerError::UnknownTrial(ticket_id))?;
        let prev = t.events.last().map_or_else(|| t.row.row_hash.clone(), |e| e.row_hash.clone());
        let mut row = EventRow {
            event_id: Uuid::new_v4(),
            trial_id: ticket_id,
            event_seq: t.events.len() as i32,
            state: to.as_str().into(),
            censoring: event.effective_censoring().as_str().into(),
            terminal_reason: event.terminal_reason.map(|r| r.as_str().to_string()),
            censor_at_step: event.censor_at_step,
            run_id: event.run_id.clone(),
            deduplicated_of: event.deduplicated_of,
            outcome: event.outcome.filter(|o| *o != OutcomeVector::default()),
            outcome_digest: event.outcome.filter(|o| *o != OutcomeVector::default()).map(|o| o.digest()),
            returns_uri: event.returns_uri.clone(),
            predictions_uri: event.predictions_uri.clone(),
            prev_hash: prev,
            row_hash: Vec::new(),
        };
        row.row_hash = row.recompute_hash();
        t.events.push(row);
        Ok(to)
    }
}

impl TrialLedger for InMemoryLedger {
    fn register(&self, reg: &Registration<'_>) -> Result<TrialTicket, LedgerError> {
        reg.validate()?;
        let mut trials = self.trials.lock().expect("ledger poisoned");
        let tenant = &reg.ctx.tenant_id;
        let (prev_hash, seq) = trials
            .iter()
            .filter(|t| &t.row.tenant_id == tenant)
            .max_by_key(|t| t.row.seq)
            .map_or((GENESIS_HASH.to_vec(), 0), |t| (t.row.row_hash.clone(), t.row.seq + 1));
        let trial_id = Uuid::new_v4();
        let mut row = TrialRow {
            trial_id,
            tenant_id: tenant.clone(),
            campaign_id: reg.ctx.campaign_id,
            experiment_id: reg.ctx.experiment_id.clone(),
            seq,
            prev_hash,
            row_hash: Vec::new(),
            config_hash: reg.subject.config_hash.clone(),
            dataset_id: reg.subject.dataset_id.clone(),
            code_hash: reg.subject.code_hash.clone(),
            image_digest: reg.subject.image_digest.clone(),
            seed_set: reg.subject.seed_set.clone(),
            prereg_hash: reg.prereg_hash(),
            delta_practical: reg.ctx.delta_practical,
            actor_kind: reg.ctx.actor_kind.as_str().into(),
            actor_id: reg.ctx.actor_id.clone(),
            on_behalf_of: reg.ctx.on_behalf_of.clone(),
            policy_id: reg.ctx.policy_id.clone(),
            policy_version: reg.ctx.policy_version,
            candidate_set_hash: reg.candidate_set_hash.clone(),
            propensity: reg.propensity,
            exploration_flag: reg.exploration_flag,
            supersedes: reg.supersedes,
        };
        row.row_hash = row.recompute_hash();
        trials.push(MemTrial { row, events: Vec::new() });
        Ok(mint(trial_id, tenant.clone(), reg.subject.config_hash.clone()))
    }

    fn transition(&self, ticket: &mut TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        if event.state.is_some_and(TrialState::is_terminal) {
            return Err(LedgerError::Invalid("terminal transitions go through settle(), which consumes the ticket".into()));
        }
        ticket.state = self.append(ticket.trial_id, &event, ticket.state)?;
        Ok(())
    }

    fn settle(&self, ticket: TrialTicket, event: TrialEvent) -> Result<(), LedgerError> {
        if !event.state.is_some_and(TrialState::is_terminal) {
            return Err(LedgerError::Invalid("settle() requires a terminal state".into()));
        }
        self.append(ticket.trial_id, &event, ticket.state)?;
        Ok(())
    }

    fn prior_trial(&self, tenant_id: &str, config_hash: &str) -> Option<Uuid> {
        let trials = self.trials.lock().expect("ledger poisoned");
        trials
            .iter()
            .filter(|t| t.row.tenant_id == tenant_id && t.row.config_hash == config_hash)
            .filter(|t| t.events.last().is_some_and(|e| e.state != "deduplicated" && TrialState::parse(&e.state).is_some_and(TrialState::is_terminal)))
            .max_by_key(|t| t.row.seq)
            .map(|t| t.row.trial_id)
    }

    fn trial_count(&self, tenant_id: &str) -> usize {
        self.trials.lock().expect("ledger poisoned").iter().filter(|t| t.row.tenant_id == tenant_id).count()
    }

    fn exploration_fraction(&self, tenant_id: &str) -> f64 {
        let trials = self.trials.lock().expect("ledger poisoned");
        let rows: Vec<_> = trials.iter().filter(|t| t.row.tenant_id == tenant_id).collect();
        if rows.is_empty() {
            return 0.0;
        }
        rows.iter().filter(|t| t.row.exploration_flag).count() as f64 / rows.len() as f64
    }

    fn persist_returns(&self, ticket: &TrialTicket, series: &neff::ReturnSeries) -> Result<String, LedgerError> {
        series.validate()?;
        if !self.trials.lock().expect("ledger poisoned").iter().any(|t| t.row.trial_id == ticket.trial_id && t.row.tenant_id == ticket.tenant_id) {
            return Err(LedgerError::UnknownTrial(ticket.trial_id));
        }
        let mut returns = self.returns.lock().expect("ledger poisoned");
        if returns.iter().any(|(id, _, _)| *id == ticket.trial_id) {
            return Err(LedgerError::Invalid(format!("trial {} already has a return series", ticket.trial_id)));
        }
        returns.push((ticket.trial_id, ticket.tenant_id.clone(), series.clone()));
        Ok(format!("mem://mlops.trial_return_series/{}", ticket.trial_id))
    }

    fn n_eff(&self, tenant_id: &str) -> Result<neff::NEff, LedgerError> {
        let trials = self.trial_count(tenant_id);
        let series: Vec<neff::ReturnSeries> =
            self.returns.lock().expect("ledger poisoned").iter().filter(|(_, t, _)| t == tenant_id).map(|(_, _, s)| s.clone()).collect();
        Ok(neff::NEff::compute(trials, &series))
    }

    fn claim_sealed_holdout(&self, tenant_id: &str, lineage: &str, _requested_by: &str) -> Result<holdout::HoldoutClaim, LedgerError> {
        let mut holdouts = self.holdouts.lock().expect("ledger poisoned");
        let h = holdouts.entry((tenant_id.to_string(), lineage.to_string())).or_default();
        if let Some((first_trial, called_at, result)) = &h.call {
            h.attempts += 1;
            return Ok(holdout::HoldoutClaim::Repeat { first_trial: *first_trial, called_at: *called_at, result: result.clone() });
        }
        if h.claimed {
            return Err(LedgerError::Invalid(holdout::claimed_without_result(lineage)));
        }
        h.claimed = true;
        h.attempts += 1;
        Ok(holdout::HoldoutClaim::First)
    }

    fn record_sealed_holdout(&self, tenant_id: &str, lineage: &str, trial_id: Uuid, result: &serde_json::Value) -> Result<(), LedgerError> {
        let mut holdouts = self.holdouts.lock().expect("ledger poisoned");
        let h = holdouts.entry((tenant_id.to_string(), lineage.to_string())).or_default();
        if !h.claimed {
            return Err(LedgerError::Invalid(format!("no claim exists for the sealed holdout of lineage '{lineage}'")));
        }
        if h.call.is_some() {
            return Err(LedgerError::Invalid(format!("the sealed holdout of lineage '{lineage}' already has its result")));
        }
        h.call = Some((trial_id, chrono::Utc::now(), result.clone()));
        Ok(())
    }

    fn define_campaign(&self, tenant_id: &str, def: &campaign::CampaignDefinition) -> Result<campaign::CampaignHandle, LedgerError> {
        def.validate()?;
        let mut campaigns = self.campaigns.lock().expect("ledger poisoned");
        let key = (tenant_id.to_string(), def.slug.clone());
        if campaigns.iter().any(|(k, _)| *k == key) {
            return Err(LedgerError::Invalid(format!(
                "campaign '{}' is already defined for this tenant; DEFINE facts are immutable, so a                  change is a new campaign",
                def.slug
            )));
        }
        campaigns.push((key, def.clone()));
        Ok(campaign::CampaignHandle::seal(Uuid::new_v4(), tenant_id.to_string(), def.clone()))
    }

    fn campaign_exploration_fraction(&self, tenant_id: &str, campaign_id: Uuid) -> f64 {
        let trials = self.trials.lock().expect("ledger poisoned");
        let mine: Vec<&MemTrial> = trials
            .iter()
            .filter(|t| t.row.tenant_id == tenant_id && t.row.campaign_id == Some(campaign_id))
            .collect();
        if mine.is_empty() {
            return 0.0;
        }
        let explored = mine.iter().filter(|t| t.row.exploration_flag).count();
        explored as f64 / mine.len() as f64
    }
    fn record_statistic(
        &self,
        tenant_id: &str,
        trial_id: Uuid,
        name: &str,
        value: f64,
        _produced_by: &str,
    ) -> Result<(), LedgerError> {
        if !TRIAL_STATISTICS.contains(&name) {
            return Err(LedgerError::Invalid(format!(
                "`{name}` is not a statistic this platform records; the set is fixed so a                  misspelling cannot become a gate input nobody ever finds"
            )));
        }
        if !value.is_finite() {
            return Err(LedgerError::Invalid(format!(
                "`{name}` is {value}, which is not an observation"
            )));
        }
        self.statistics
            .lock()
            .expect("ledger poisoned")
            .insert((tenant_id.to_string(), trial_id, name.to_string()), value);
        Ok(())
    }

}

impl InMemoryLedger {
    /// Campaigns defined on this ledger, in definition order.
    #[must_use]
    pub fn campaigns(&self) -> Vec<campaign::CampaignDefinition> {
        self.campaigns.lock().expect("ledger poisoned").iter().map(|(_, d)| d.clone()).collect()
    }

    /// Gate verdicts recorded on this ledger, in decision order.
    #[must_use]
    pub fn gate_verdicts(&self) -> Vec<gates::GateRecord> {
        self.gate_verdicts.lock().expect("ledger poisoned").iter().map(|(_, _, r)| r.clone()).collect()
    }
}

impl gates::GateLog for InMemoryLedger {
    fn record_gate(&self, tenant_id: &str, record: &gates::GateRecord) -> Result<Uuid, LedgerError> {
        record.validate()?;
        self.gate_verdicts
            .lock()
            .expect("ledger poisoned")
            .push((tenant_id.to_string(), chrono::Utc::now(), record.clone()));
        Ok(Uuid::new_v4())
    }

    fn pass_rate(&self, tenant_id: &str, profile_id: &str, window_days: i64) -> Result<gates::PassRate, LedgerError> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days.max(0));
        let verdicts = self.gate_verdicts.lock().expect("ledger poisoned");
        // A candidate passed when no gate failed it: group by subject, then count.
        let mut recent: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        let mut baseline: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        for (t, at, r) in verdicts.iter() {
            if t != tenant_id || r.profile_id != profile_id {
                continue;
            }
            let subject = r
                .experiment_id
                .clone()
                .or_else(|| r.trial_id.map(|id| id.to_string()))
                .unwrap_or_default();
            let bucket = if *at >= cutoff { &mut recent } else { &mut baseline };
            let e = bucket.entry(subject).or_insert(true);
            *e = *e && r.passed;
        }
        let count = |m: &std::collections::HashMap<String, bool>| {
            (m.len() as i64, m.values().filter(|p| **p).count() as i64)
        };
        let (recent_decided, recent_passed) = count(&recent);
        let (baseline_decided, baseline_passed) = count(&baseline);
        Ok(gates::PassRate { recent_decided, recent_passed, baseline_decided, baseline_passed })
    }
}

impl DecisionLog for InMemoryLedger {
    fn log_decision(&self, ctx: &DispatchContext, decision: &Decision) -> Result<Uuid, LedgerError> {
        decision.validate(ctx)?;
        self.decisions.lock().expect("ledger poisoned").push((ctx.clone(), decision.clone()));
        Ok(Uuid::new_v4())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(n: u64) -> TrialSubject {
        TrialSubject {
            config_hash: format!("sha256:cfg{n}"),
            config: serde_json::json!({ "n": n }),
            dataset_id: "sha256:ds".into(),
            split_spec_id: None,
            code_hash: "sha256:code".into(),
            image_digest: "engine@1".into(),
            seed_set: vec![n as i64],
            non_reproducible: false,
            overlapping_labels_unweighted: false,
            split_overrides: serde_json::json!([]),
            planned_steps: None,
        }
    }

    fn ctx() -> DispatchContext {
        DispatchContext::human("tenant-a", "mason", 0.15).with_experiment("exp-1")
    }

    #[test]
    fn registration_needs_propensity_delta_and_scope() {
        let l = InMemoryLedger::new();
        let s = subject(1);
        let c = ctx();
        assert!(matches!(l.register(&Registration::unlogged(&c, &s)), Err(LedgerError::MissingPropensity)));
        let mut nodelta = ctx();
        nodelta.delta_practical = None;
        assert!(matches!(l.register(&Registration::new(&nodelta, &s, 0.5)), Err(LedgerError::MissingDeltaPractical)));
        let unscoped = DispatchContext::human("tenant-a", "m", 0.1);
        assert!(matches!(l.register(&Registration::new(&unscoped, &s, 0.5)), Err(LedgerError::NoCampaign)));
        for bad in [0.0, 1.5, f64::NAN] {
            assert!(matches!(l.register(&Registration::new(&c, &s, bad)), Err(LedgerError::BadPropensity(_))));
        }
        assert_eq!(l.trial_count("tenant-a"), 0);
    }

    #[test]
    fn legacy_marker_is_the_only_unlogged_path() {
        let l = InMemoryLedger::new();
        let s = subject(1);
        let legacy = DispatchContext::legacy("tenant-a", ActorKind::Human, "ui");
        let t = l.register(&Registration::unlogged(&legacy, &s)).unwrap();
        assert_eq!(l.rows("tenant-a")[0].delta_practical, None);
        l.settle(t, TrialEvent::failed(TerminalReason::Cancelled, "user stop")).unwrap();
        assert_eq!(l.trial_count("tenant-a"), 1);
    }

    #[test]
    fn full_lifecycle_chains_and_verifies() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let s = subject(1);
        let mut t = l.register(&Registration::new(&c, &s, 0.25)).unwrap();
        let id = t.trial_id();
        for st in [TrialState::Queued, TrialState::Provision, TrialState::Running, TrialState::Preempted, TrialState::Recovering, TrialState::Running, TrialState::Evaluate, TrialState::Gated] {
            l.transition(&mut t, TrialEvent::to(st)).unwrap();
        }
        let outcome = OutcomeVector { sharpe_net: Some(1.1), ..Default::default() };
        let t0 = chrono::Utc::now();
        let uri = l.persist_returns(&t, &neff::ReturnSeries { timestamps: vec![t0, t0 + chrono::Duration::days(1)], returns: vec![0.01, -0.002] }).unwrap();
        l.settle(t, TrialEvent { outcome: Some(outcome), gate_profile: Some("strict_v1".into()), ..TrialEvent::to(TrialState::CompletedPass) }.with_returns(uri)).unwrap();
        let rows = l.rows("tenant-a");
        assert_eq!(verify_trials(&rows).unwrap(), 1);
        let evs = l.events(id);
        assert_eq!(verify_events(&rows[0], &evs).unwrap(), 9);
    }

    #[test]
    fn illegal_transitions_are_refused() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let s = subject(1);
        let mut t = l.register(&Registration::new(&c, &s, 0.5)).unwrap();
        assert!(matches!(l.transition(&mut t, TrialEvent::to(TrialState::Gated)), Err(LedgerError::IllegalTransition { .. })));
        l.transition(&mut t, TrialEvent::to(TrialState::Running)).unwrap();
        l.transition(&mut t, TrialEvent::to(TrialState::Evaluate)).unwrap();
        assert!(l.settle(t, TrialEvent::to(TrialState::CompletedPass)).is_err(), "cannot pass without gating");
    }

    /// AT-22: every terminal kind writes a row with the right censoring.
    #[test]
    fn all_terminal_states_write_with_correct_censoring() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let cases: Vec<(Vec<TrialState>, TrialEvent, Censoring)> = vec![
            (vec![TrialState::Running], TrialEvent::completed("r1", None), Censoring::None),
            (vec![TrialState::Running, TrialState::Evaluate, TrialState::Gated], TrialEvent { terminal_reason: Some(TerminalReason::GateFailed), ..TrialEvent::to(TrialState::CompletedFail) }, Censoring::None),
            (vec![TrialState::Running], TrialEvent::failed(TerminalReason::Oom, "oom"), Censoring::Failed),
            (vec![TrialState::Running], TrialEvent::failed(TerminalReason::NanDivergence, "nan"), Censoring::Failed),
            (vec![TrialState::Running], TrialEvent::failed(TerminalReason::Timeout, "t"), Censoring::Failed),
            (vec![TrialState::Queued], TrialEvent::failed(TerminalReason::Cancelled, "c"), Censoring::RightCancel),
            (vec![TrialState::Running, TrialState::Preempted], TrialEvent::failed(TerminalReason::PreemptedAbandoned, "p"), Censoring::RightPreempt),
            (vec![TrialState::Running], TrialEvent::failed(TerminalReason::AshaStopped, "a").at_step(40), Censoring::RightAsha),
        ];
        for (n, (path, terminal, want)) in cases.into_iter().enumerate() {
            let s = subject(n as u64);
            let mut t = l.register(&Registration::new(&c, &s, 0.5)).unwrap();
            let id = t.trial_id();
            for st in path {
                l.transition(&mut t, TrialEvent::to(st)).unwrap();
            }
            l.settle(t, terminal).unwrap();
            let (state, cens) = l.state_of(id).unwrap();
            assert!(state.is_terminal());
            assert_eq!(cens, want, "case {n}");
        }
        assert_eq!(l.trial_count("tenant-a"), 8);
    }

    #[test]
    fn a_failed_trial_without_reason_is_refused() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let s = subject(1);
        let t = l.register(&Registration::new(&c, &s, 0.5)).unwrap();
        assert!(l.settle(t, TrialEvent::to(TrialState::Failed)).is_err());
    }

    /// AT-25: the same config twice → the second is DEDUPLICATED naming the first.
    #[test]
    fn dedup_names_the_prior_trial() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let s = subject(7);
        let mut first = l.register(&Registration::new(&c, &s, 0.5)).unwrap();
        let first_id = first.trial_id();
        l.transition(&mut first, TrialEvent::to(TrialState::Running)).unwrap();
        l.settle(first, TrialEvent::completed("run7", None)).unwrap();
        let second = l.register(&Registration::new(&c, &s, 0.5)).unwrap();
        let prior = l.prior_trial("tenant-a", &s.config_hash).unwrap();
        assert_eq!(prior, first_id);
        let second_id = second.trial_id();
        l.settle(second, TrialEvent::deduplicated(prior, "run7")).unwrap();
        assert_eq!(l.events(second_id)[0].deduplicated_of, Some(first_id));
        assert_eq!(l.trial_count("tenant-a"), 2, "the dedup is still a look");
    }

    #[test]
    fn tampering_is_detected() {
        let l = InMemoryLedger::new();
        let c = ctx();
        for n in 0..4 {
            let s = subject(n);
            l.register(&Registration::new(&c, &s, 0.3)).unwrap();
        }
        let mut rows = l.rows("tenant-a");
        rows[1].propensity = Some(0.99);
        assert_eq!(verify_trials(&rows).unwrap_err().seq, 1);
        let mut rows = l.rows("tenant-a");
        rows.remove(2);
        assert!(verify_trials(&rows).is_err());
    }

    #[test]
    fn tampered_outcome_is_detected() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let s = subject(1);
        let mut t = l.register(&Registration::new(&c, &s, 0.5)).unwrap();
        let id = t.trial_id();
        l.transition(&mut t, TrialEvent::to(TrialState::Running)).unwrap();
        let uri = l.persist_returns(&t, &neff::ReturnSeries { timestamps: vec![chrono::Utc::now()], returns: vec![0.004] }).unwrap();
        l.settle(t, TrialEvent::completed("r", Some(OutcomeVector { sharpe_net: Some(0.4), ..Default::default() })).with_returns(uri)).unwrap();
        let row = l.rows("tenant-a").remove(0);
        let mut evs = l.events(id);
        evs[1].outcome.as_mut().unwrap().sharpe_net = Some(2.4);
        assert!(verify_events(&row, &evs).is_err());
    }

    #[test]
    fn decisions_require_candidates_and_propensity() {
        let l = InMemoryLedger::new();
        let c = ctx();
        let d = Decision {
            kind: DecisionKind::Prune,
            context_hash: "sha256:ctx".into(),
            context_uri: None,
            candidate_set: vec![serde_json::json!("a"), serde_json::json!("b")],
            chosen: serde_json::json!("a"),
            propensity: Some(0.5),
            exploration_flag: false,
            decision_tier: DecisionTier::Rule,
            rationale: None,
        };
        assert!(l.log_decision(&c, &d).is_ok());
        let mut nop = d.clone();
        nop.propensity = None;
        assert!(matches!(l.log_decision(&c, &nop), Err(LedgerError::MissingPropensity)));
        let mut empty = d;
        empty.candidate_set.clear();
        assert!(l.log_decision(&c, &empty).is_err());
    }
}

#[cfg(test)]
mod statistic_tests {
    use super::*;

    /// A misspelled statistic name is a gate input nobody ever finds, and a gate
    /// that never finds its input is inconclusive forever without anybody
    /// noticing. The set is closed here and CHECKed by migration 0057.
    #[test]
    fn an_unknown_statistic_name_is_refused() {
        let led = InMemoryLedger::new();
        let trial = Uuid::new_v4();
        assert!(led.record_statistic("t", trial, "pbo", 0.1, "funnel").is_ok());
        let err = led
            .record_statistic("t", trial, "PBO", 0.1, "funnel")
            .expect_err("case matters");
        assert!(format!("{err}").contains("not a statistic"), "{err}");
        assert!(led.record_statistic("t", trial, "sharpe", 1.0, "funnel").is_err());
    }

    #[test]
    fn a_non_finite_value_is_not_an_observation() {
        let led = InMemoryLedger::new();
        let trial = Uuid::new_v4();
        assert!(led.record_statistic("t", trial, "pbo", f64::NAN, "funnel").is_err());
        assert!(led.record_statistic("t", trial, "pbo", f64::INFINITY, "funnel").is_err());
    }

    /// The names are exactly the ones the gate worker reads and the database
    /// accepts. Drift in either direction breaks a gate silently, so it breaks
    /// here loudly instead.
    #[test]
    fn the_vocabulary_is_the_one_the_gates_read() {
        assert_eq!(
            TRIAL_STATISTICS,
            &[
                "cpcv_p05_sharpe",
                "walk_forward_sharpe",
                "walk_forward_regimes",
                "pbo",
                "deflated_sharpe",
                "permutation_p_value",
                "breakeven_cost_multiple",
                "prereg_hash_present",
            ]
        );
    }
}
