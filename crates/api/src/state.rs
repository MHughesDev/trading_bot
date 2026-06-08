use std::sync::Arc;

use sqlx::PgPool;

use execution::ExecutionEngine;
use risk::{KillSwitch, RiskGate};

/// Shared application state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub pg: PgPool,
    pub risk_gate: Arc<RiskGate>,
    pub kill_switch: Arc<KillSwitch>,
    pub execution: Arc<ExecutionEngine>,
}

impl AppState {
    pub fn new(
        pg: PgPool,
        risk_gate: Arc<RiskGate>,
        kill_switch: Arc<KillSwitch>,
        execution: Arc<ExecutionEngine>,
    ) -> Self {
        Self {
            pg,
            risk_gate,
            kill_switch,
            execution,
        }
    }
}
