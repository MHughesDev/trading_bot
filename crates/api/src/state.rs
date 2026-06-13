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
        }
    }
}
