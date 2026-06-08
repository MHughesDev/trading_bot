use std::sync::Arc;

use sqlx::PgPool;

use execution::ExecutionEngine;
use risk::{KillSwitch, RiskGate};
use ui_gateway::SubscriptionRegistry;

/// Shared application state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub pg: PgPool,
    pub risk_gate: Arc<RiskGate>,
    pub kill_switch: Arc<KillSwitch>,
    pub execution: Arc<ExecutionEngine>,
    pub gateway: Arc<SubscriptionRegistry>,
}

impl AppState {
    pub fn new(
        pg: PgPool,
        risk_gate: Arc<RiskGate>,
        kill_switch: Arc<KillSwitch>,
        execution: Arc<ExecutionEngine>,
        gateway: Arc<SubscriptionRegistry>,
    ) -> Self {
        Self {
            pg,
            risk_gate,
            kill_switch,
            execution,
            gateway,
        }
    }
}
