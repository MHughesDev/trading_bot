//! Lifecycle tools: `apply_strategy`, `stop_strategy`, `list_strategies`.
//!
//! `apply_strategy` initializes a strategy instance in the runtime for a
//! specific instrument. All resulting order intents still pass through the
//! risk gate — there is no privileged execution path.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use strategy_runtime::{StrategyClock, WallClock};

use crate::McpContext;

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplyResult {
    pub store_id: String,
    pub user_id: String,
    pub instrument_id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplyError {
    pub error: String,
    pub detail: String,
}

/// `apply_strategy` — start a strategy instance on a specific instrument.
pub fn apply_strategy(
    ctx: &McpContext,
    store_id: &str,
    user_id: &str,
    instrument_id: &str,
) -> Result<ApplyResult, ApplyError> {
    let id: uuid::Uuid = store_id.parse().map_err(|_| ApplyError {
        error: "invalid_store_id".into(),
        detail: format!("'{store_id}' is not a valid UUID"),
    })?;

    let def = ctx
        .strategy_store
        .lock()
        .expect("strategy_store lock poisoned")
        .get(&id)
        .cloned()
        .ok_or_else(|| ApplyError {
            error: "not_found".into(),
            detail: format!("strategy '{store_id}' not found"),
        })?;

    let clock: Arc<dyn StrategyClock> = Arc::new(WallClock);

    ctx.instance_manager
        .lock()
        .expect("instance_manager lock poisoned")
        .initialize(user_id, instrument_id, def, &clock)
        .map_err(|e| ApplyError {
            error: "already_running".into(),
            detail: e.to_string(),
        })?;

    Ok(ApplyResult {
        store_id: store_id.to_owned(),
        user_id: user_id.to_owned(),
        instrument_id: instrument_id.to_owned(),
        status: "running".into(),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StopResult {
    pub user_id: String,
    pub instrument_id: String,
    pub stopped: bool,
}

/// `stop_strategy` — stop a running strategy instance.
pub fn stop_strategy(ctx: &McpContext, user_id: &str, instrument_id: &str) -> StopResult {
    ctx.instance_manager
        .lock()
        .expect("instance_manager lock poisoned")
        .stop(user_id, instrument_id);

    StopResult {
        user_id: user_id.to_owned(),
        instrument_id: instrument_id.to_owned(),
        stopped: true,
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StrategyListItem {
    pub store_id: String,
    pub strategy_id: String,
}

/// `list_strategies` — list all persisted strategy definitions.
pub fn list_strategies(ctx: &McpContext) -> Vec<StrategyListItem> {
    ctx.strategy_store
        .lock()
        .expect("strategy_store lock poisoned")
        .iter()
        .map(|(id, def)| StrategyListItem {
            store_id: id.to_string(),
            strategy_id: def.strategy_id.clone(),
        })
        .collect()
}
