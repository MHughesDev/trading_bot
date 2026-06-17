use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use uuid::Uuid;

use backtest::BacktestManager;
use demand_manager::{DemandRegistry, NoopPipelineFactory};
use execution::paper::PaperTradingEngine;
use execution::ExecutionEngine;
use model_registry::{
    ensemble_manager::EnsembleManager,
    pipeline_manager::PipelineManager,
    quality_monitor::QualityMonitor,
    tags::TagRegistry,
    InferenceGateway, ModelManager,
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
    /// In-memory strategy definition store (keyed by Uuid).
    pub strategy_store: Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>,
    /// Active strategy instance manager.
    pub instance_manager: Arc<Mutex<InstanceManager>>,
    /// Wall clock used when initializing new strategy instances.
    pub clock: Arc<WallClock>,
    /// Backtest job orchestrator (connects to the market_simulator SDK).
    pub backtest: Arc<BacktestManager>,
    /// AI Model Studio orchestrator.
    pub models: Arc<ModelManager>,
    /// Ensemble orchestrator — mirrors ModelManager lifecycle for ensemble artifacts.
    pub ensembles: Arc<EnsembleManager>,
    /// Pipeline factory — declarative DAG pipelines with fan-out (Phase 5).
    pub pipelines: Arc<PipelineManager>,
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
    ) -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        let ensemble_sidecar = Arc::new(model_registry::sidecar::SidecarClient::from_env());
        let ensembles = EnsembleManager::new(pg.clone(), ensemble_sidecar);
        let pipelines = PipelineManager::new(pg.clone(), models.clone());
        let quality_monitor = QualityMonitor::new(
            pg.clone(),
            models.clone(),
            tokio::time::Duration::from_secs(3600),
        );
        let tags = Arc::new(TagRegistry::new(pg.clone()));
        Self {
            pg,
            risk_gate,
            kill_switch,
            execution,
            paper_engine,
            gateway,
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            clock: Arc::new(WallClock),
            backtest,
            models,
            ensembles,
            pipelines,
            quality_monitor,
            tags,
            inference,
            email,
            clickhouse_url,
            stream_tx,
        }
    }
}
