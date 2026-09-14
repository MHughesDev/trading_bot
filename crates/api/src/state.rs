use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use uuid::Uuid;

use backtest::{BacktestManager, SuiteManager};
use demand_manager::{DemandRegistry, NoopPipelineFactory};
use execution::paper::PaperTradingEngine;
use execution::ExecutionEngine;
use model_registry::{
    quality_monitor::QualityMonitor, tags::TagRegistry, InferenceGateway, ModelManager,
};
use risk::{KillSwitch, RiskGate};
use strategy_runtime::{InstanceManager, WallClock};
use ui_gateway::SubscriptionRegistry;

use domain::strategy_def::StrategyDefinition;

/// Request to start a continuous live-data pipeline for an instrument.  Sent by
/// the asset-init handler after seeding so a newly initialized asset begins
/// 1-minute aggregation immediately, without a platform restart.  The platform
/// binary owns the receiver and the actual pipeline machinery.
#[derive(Clone, Debug)]
pub struct StreamRequest {
    pub instrument_id: String,
    pub asset_class: String,
}

/// Shared application state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub pg: PgPool,
    pub risk_gate: Arc<RiskGate>,
    pub kill_switch: Arc<KillSwitch>,
    pub execution: Arc<ExecutionEngine>,
    /// Internal paper trading engine — source of truth for paper-mode account
    /// data on the dashboard (balances, positions, P&L per asset class).
    pub paper_engine: Arc<PaperTradingEngine>,
    pub gateway: Arc<SubscriptionRegistry>,
    /// Live frame bus. Producers publish here; every `/ws/live` socket forwards
    /// the frames matching its own subscriptions. Without this the streaming
    /// surfaces (quotes, chart tail, order book, tape) have no data source.
    pub live: crate::live_bus::LiveSender,
    /// In-memory strategy definition store (keyed by Uuid).
    pub strategy_store: Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>,
    /// Active strategy instance manager.
    pub instance_manager: Arc<Mutex<InstanceManager>>,
    /// Wall clock used when initializing new strategy instances.
    pub clock: Arc<WallClock>,
    /// Backtest job orchestrator (connects to the market_simulator SDK).
    pub backtest: Arc<BacktestManager>,
    /// Backtest-suite orchestrator (Set J: experiments, studies, gates, vault,
    /// reconciliation) — the honest-evaluation core behind `/api/backtest/*`.
    pub suite: Arc<SuiteManager>,
    /// AI Model Studio orchestrator.
    pub models: Arc<ModelManager>,
    /// Rolling forecast quality monitor — drift detection, staleness, retrain triggers.
    pub quality_monitor: Arc<QualityMonitor>,
    /// Tags, annotations, and spec templates (I-6.4).
    pub tags: Arc<TagRegistry>,
    /// Inference gateway — alias resolution, prediction caching, circuit breaking.
    pub inference: Arc<InferenceGateway>,
    /// Email config for password-reset codes.
    pub email: cfg::model::EmailConfig,
    /// ClickHouse URL — used by asset init jobs and the chart bars endpoint.
    pub clickhouse_url: String,
    /// Channel to request a live 1-minute aggregation pipeline for a newly
    /// initialized instrument.  `None` in contexts with no platform pipeline
    /// host (e.g. tests).
    pub stream_tx: Option<tokio::sync::mpsc::UnboundedSender<StreamRequest>>,
    /// Envelope-encryption service for stored credentials (LLM API keys).
    /// `None` when the `CRED_KEK` env var is unset — credential routes then
    /// return 503 rather than storing anything unencrypted.
    pub cred_crypto: Option<Arc<crate::credentials::CredentialCrypto>>,
    /// Internal agent orchestrator (LLM-driven strategy design + backtests).
    pub agent: Arc<crate::agent::AgentManager>,
    /// Research orchestrator (FEAT-003): sweeps over the suite, diagnostics.
    pub research: Arc<crate::research::ResearchManager>,
    /// Durable job service (COMP-005, Set L Phase 1).
    ///
    /// Optional so that unit tests and tools can build an `AppState` without a
    /// database; the routes answer `503 jobs_unavailable` when it is absent rather
    /// than pretending to accept work they cannot durably record.
    pub jobs: Option<Arc<jobs::JobStore>>,
    /// Content-addressed artifact registry (COMP-005 §8).
    pub artifacts: Option<Arc<jobs::ArtifactRegistry>>,
    /// Capability profiles (ADR-0031, harness guide §1.2).
    ///
    /// Loaded at startup, never hardcoded. Every constraint in the active profile is
    /// enforced by harness code — the tool exposure budget, the context cap, the
    /// permission policy and the per-task budgets all read from here rather than from
    /// constants scattered through the request path.
    pub profiles: Arc<harness::ProfileSet>,
    /// The profile id a research session runs under unless its project pins another.
    pub default_profile: String,
}

/// Loads the capability profiles (ADR-0031). Read from disk at startup, never
/// hardcoded.
///
/// A missing or invalid directory leaves the set empty, and `AppState::profile` then
/// returns `None` — callers fall back to their own conservative behaviour and the
/// warning says why. Panicking here would take the whole platform down over the
/// agent's configuration, which is the wrong blast radius for a feature most of the
/// platform does not use.
///
/// Shared with the agent manager rather than loaded twice: two loaders would be two
/// answers to "which profile is this model?", and the disagreement would show up as
/// a run executing on a tier the UI says it is not.
#[must_use]
pub fn load_profiles() -> Arc<harness::ProfileSet> {
    let dir = std::env::var("TBOT_PROFILES_DIR").unwrap_or_else(|_| "config/profiles".to_string());
    let profiles = match harness::ProfileSet::load_dir(std::path::Path::new(&dir)) {
        Ok(set) => {
            tracing::info!(
                dir = %dir,
                profiles = set.len(),
                ids = ?set.ids(),
                "capability profiles loaded"
            );
            set
        }
        Err(e) => {
            tracing::warn!(
                dir = %dir,
                error = %e,
                "no capability profiles loaded; agent sessions will refuse to start rather than run unprofiled"
            );
            harness::ProfileSet::default()
        }
    };
    // The fallback for an unknown model id is named explicitly, never derived from
    // whichever profile happens to be lowest-tier: adding a dev-box profile must not
    // repoint the platform default as a side effect.
    let profiles = match std::env::var("TBOT_FALLBACK_PROFILE") {
        Ok(id) if !id.trim().is_empty() => profiles.with_default(id),
        _ => profiles,
    };
    Arc::new(profiles)
}

impl AppState {
    /// The active capability profile.
    ///
    /// An unknown model resolves to the most conservative tier with a warning rather
    /// than to frontier settings (guide §1.2) — so a typo in a project's pinned model
    /// produces a cautious session, not an unprofiled one.
    #[must_use]
    pub fn profile(&self, model_id: Option<&str>) -> Option<&harness::Profile> {
        // `accepting_substitution` rather than a silent unwrap: this is a read path
        // (the proxy's budget check, the tool exposure budget), where running under
        // the conservative default is better than running unbudgeted. An EXECUTION
        // path must call `resolve_for_execution` directly and decide for itself
        // whether a substitution is acceptable — that is the distinction
        // `Resolution` exists to force.
        self.profiles
            .resolve_for_execution(model_id.unwrap_or(&self.default_profile))
            .accepting_substitution()
    }
}

impl AppState {
    /// Fire-and-forget: send a request to start a live 1-minute aggregation
    /// pipeline for `instrument_id` with a known `asset_class`.  Idempotent —
    /// the pipeline manager ignores the request when a pipeline is already
    /// running for that instrument.
    pub fn ensure_pipeline(&self, instrument_id: &str, asset_class: &str) {
        if let Some(tx) = &self.stream_tx {
            let _ = tx.send(StreamRequest {
                instrument_id: instrument_id.to_owned(),
                asset_class: asset_class.to_owned(),
            });
        }
    }

    /// Like `ensure_pipeline` but resolves `asset_class` from the database,
    /// falling back to a symbol-name heuristic when the instrument is not yet
    /// in `asset_lifecycle` or `instruments`.
    pub async fn ensure_pipeline_for_instrument(&self, instrument_id: &str) {
        if self.stream_tx.is_none() {
            return;
        }
        let asset_class = self.resolve_asset_class(instrument_id).await;
        self.ensure_pipeline(instrument_id, &asset_class);
    }

    pub(crate) async fn resolve_asset_class(&self, instrument_id: &str) -> String {
        if let Ok(Some((ac,))) = sqlx::query_as::<_, (String,)>(
            "SELECT asset_class FROM asset_lifecycle WHERE symbol = $1",
        )
        .bind(instrument_id)
        .fetch_optional(&self.pg)
        .await
        {
            return ac;
        }
        if let Ok(Some((ac,))) = sqlx::query_as::<_, (String,)>(
            "SELECT asset_class FROM instruments WHERE instrument_id = $1",
        )
        .bind(instrument_id)
        .fetch_optional(&self.pg)
        .await
        {
            return ac;
        }
        // Heuristic: crypto pairs typically end with -USD/-USDT/-USDC/-BTC/-ETH.
        let u = instrument_id.to_uppercase();
        if u.ends_with("-USD")
            || u.ends_with("-USDT")
            || u.ends_with("-USDC")
            || u.ends_with("-BTC")
            || u.ends_with("-ETH")
            || u.ends_with("USDT")
            || u.ends_with("USDC")
        {
            "crypto_spot_cex".to_string()
        } else {
            "equity".to_string()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pg: PgPool,
        risk_gate: Arc<RiskGate>,
        kill_switch: Arc<KillSwitch>,
        execution: Arc<ExecutionEngine>,
        paper_engine: Arc<PaperTradingEngine>,
        gateway: Arc<SubscriptionRegistry>,
        backtest: Arc<BacktestManager>,
        models: Arc<ModelManager>,
        inference: Arc<InferenceGateway>,
        email: cfg::model::EmailConfig,
        clickhouse_url: String,
        stream_tx: Option<tokio::sync::mpsc::UnboundedSender<StreamRequest>>,
        agent: Arc<crate::agent::AgentManager>,
        // Shared live frame bus. `None` builds a private one, which is what
        // tests and tools want; the platform passes its own so the producers it
        // spawns reach the same sockets.
        live: Option<crate::live_bus::LiveSender>,
    ) -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        let quality_monitor = QualityMonitor::new(
            pg.clone(),
            models.clone(),
            tokio::time::Duration::from_secs(3600),
        );
        let tags = Arc::new(TagRegistry::new(pg.clone()));
        let cred_crypto = match crate::credentials::CredentialCrypto::from_env() {
            Ok(c) => Some(Arc::new(c)),
            Err(_) => {
                tracing::warn!(
                    "CRED_KEK not set — LLM credential storage disabled (routes return 503)"
                );
                None
            }
        };
        // The suite runs real backtests when a runtime is present (the platform);
        // outside one (unit tests) it falls back to the synthetic executor.
        let suite = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let executor = backtest::sim_executor::SimRunExecutor::new(
                    handle,
                    pg.clone(),
                    clickhouse_url.clone(),
                );
                // The durable, hash-chained trial ledger. Inside a runtime this
                // is the Postgres one: every dispatch writes a REGISTERED row
                // before any compute runs, and that row survives a restart.
                let ledger = Arc::new(backtest::ledger::pg::PgTrialLedger::new(pg.clone()));
                Arc::new(SuiteManager::with_executor(Box::new(executor), ledger))
            }
            Err(_) => Arc::new(SuiteManager::new()),
        };
        let research = crate::research::ResearchManager::new(pg.clone(), Arc::clone(&suite), 2);

        // The job service counts trials against Set J's experiment counter inside
        // the submission transaction (INV-1, JB-03).
        let job_store = Arc::new(jobs::JobStore::new(
            pg.clone(),
            Arc::new(jobs::store::PgTrialCounter),
        ));
        let artifact_registry = Arc::new(jobs::ArtifactRegistry::new(
            pg.clone(),
            Arc::from(storage::artifacts::from_env()),
        ));

        let profiles = load_profiles();
        Self {
            pg,
            risk_gate,
            kill_switch,
            execution,
            paper_engine,
            gateway,
            live: live.unwrap_or_else(crate::live_bus::channel),
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            clock: Arc::new(WallClock),
            backtest,
            suite,
            models,
            quality_monitor,
            tags,
            inference,
            email,
            clickhouse_url,
            stream_tx,
            cred_crypto,
            agent,
            research,
            jobs: Some(job_store),
            artifacts: Some(artifact_registry),
            profiles,
            default_profile: std::env::var("TBOT_DEFAULT_PROFILE")
                .unwrap_or_else(|_| "claude-opus-5".to_string()),
        }
    }
}
