use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use uuid::Uuid;

use demand_manager::{DemandRegistry, NoopPipelineFactory};
use execution::ExecutionEngine;
use market_simulator_adapter::BacktestReport;
use risk::{KillSwitch, RiskGate};
use strategy_runtime::{InstanceManager, WallClock};
use ui_gateway::SubscriptionRegistry;

use domain::strategy_def::StrategyDefinition;

/// Status of an async backtest job.
#[derive(Debug, Clone)]
pub enum BacktestJobState {
    Pending,
    Completed(BacktestReport),
    Failed(String),
}

/// Shared application state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub pg: PgPool,
    pub risk_gate: Arc<RiskGate>,
    pub kill_switch: Arc<KillSwitch>,
    pub execution: Arc<ExecutionEngine>,
    pub gateway: Arc<SubscriptionRegistry>,
    /// In-memory backtest job store for Phase 4.
    pub backtest_jobs: Arc<Mutex<HashMap<Uuid, BacktestJobState>>>,
    /// In-memory strategy definition store (keyed by Uuid).
    pub strategy_store: Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>,
    /// Active strategy instance manager.
    pub instance_manager: Arc<Mutex<InstanceManager>>,
    /// Wall clock used when initializing new strategy instances.
    pub clock: Arc<WallClock>,
}

impl AppState {
    pub fn new(
        pg: PgPool,
        risk_gate: Arc<RiskGate>,
        kill_switch: Arc<KillSwitch>,
        execution: Arc<ExecutionEngine>,
        gateway: Arc<SubscriptionRegistry>,
    ) -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        Self {
            pg,
            risk_gate,
            kill_switch,
            execution,
            gateway,
            backtest_jobs: Arc::new(Mutex::new(HashMap::new())),
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            clock: Arc::new(WallClock),
        }
    }
}
