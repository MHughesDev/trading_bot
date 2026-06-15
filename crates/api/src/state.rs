use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use uuid::Uuid;

use backtest::BacktestManager;
use demand_manager::{DemandRegistry, NoopPipelineFactory};
use execution::paper::PaperTradingEngine;
use execution::ExecutionEngine;
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pg: PgPool,
        risk_gate: Arc<RiskGate>,
        kill_switch: Arc<KillSwitch>,
        execution: Arc<ExecutionEngine>,
        paper_engine: Arc<PaperTradingEngine>,
        gateway: Arc<SubscriptionRegistry>,
        backtest: Arc<BacktestManager>,
        email: cfg::model::EmailConfig,
        clickhouse_url: String,
        stream_tx: Option<tokio::sync::mpsc::UnboundedSender<StreamRequest>>,
    ) -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
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
            email,
            clickhouse_url,
            stream_tx,
        }
    }
}
