//! Strategy builder tools — incremental draft construction for AI agents.
//!
//! An agent calls `new_strategy_draft` to get a `draft_id`, then uses the
//! step-by-step tools to add nodes/inputs/actions, then calls `finalize_strategy`
//! which validates and persists the assembled definition.

use serde_json::{json, Value};
use uuid::Uuid;

use domain::strategy_def::actions::{Action, ActionKind, OrderSpec, SizeMode};
use domain::strategy_def::inputs::InputDeclaration;
use domain::strategy_def::nodes::{Node, NodeKind};
use domain::strategy_def::risk_overrides::RiskOverrides;
use domain::strategy_def::StrategyDefinition;
use domain::trust::TrustTier;

use crate::tools::authoring::{create_strategy_from_def, ValidationErrorItem};
use crate::McpContext;

/// In-memory draft accumulator.
pub struct StrategyDraft {
    pub strategy_id: Option<String>,
    pub definition_version: String,
    pub asset_class: Option<String>,
    pub min_trust_tier: Option<TrustTier>,
    pub inputs: Vec<InputDeclaration>,
    pub nodes: Vec<Node>,
    pub actions: Vec<Action>,
    pub risk_overrides: RiskOverrides,
}

impl StrategyDraft {
    pub fn new() -> Self {
        Self {
            strategy_id: None,
            definition_version: "1.0".into(),
            asset_class: None,
            min_trust_tier: None,
            inputs: vec![],
            nodes: vec![],
            actions: vec![],
            risk_overrides: RiskOverrides::default(),
        }
    }

    fn summary(&self, draft_id: Uuid) -> Value {
        json!({
            "draft_id": draft_id.to_string(),
            "strategy_id": self.strategy_id,
            "inputs_count": self.inputs.len(),
            "nodes_count": self.nodes.len(),
            "actions_count": self.actions.len(),
        })
    }
}

fn draft_not_found(draft_id: &str) -> Value {
    json!({ "error": "draft_not_found", "draft_id": draft_id })
}

/// `new_strategy_draft` — create a fresh empty draft, return its UUID.
pub fn new_strategy_draft(ctx: &McpContext) -> Value {
    let id = Uuid::new_v4();
    ctx.draft_store
        .lock()
        .expect("draft_store lock poisoned")
        .insert(id, StrategyDraft::new());
    json!({ "draft_id": id.to_string() })
}

/// `discard_draft` — remove a draft by ID.
pub fn discard_draft(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return json!({ "discarded": false });
    };
    let removed = ctx
        .draft_store
        .lock()
        .expect("draft_store lock poisoned")
        .remove(&draft_id)
        .is_some();
    json!({ "discarded": removed })
}

/// `set_strategy_meta` — set top-level strategy fields on a draft.
pub fn set_strategy_meta(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let mut store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get_mut(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    if let Some(sid) = params.get("strategy_id").and_then(|v| v.as_str()) {
        draft.strategy_id = Some(sid.to_owned());
    }
    if let Some(ac) = params.get("asset_class").and_then(|v| v.as_str()) {
        draft.asset_class = Some(ac.to_owned());
    }
    if let Some(tt) = params.get("min_trust_tier").and_then(|v| v.as_str()) {
        if let Ok(tier) = serde_json::from_value::<TrustTier>(Value::String(tt.to_owned())) {
            draft.min_trust_tier = Some(tier);
        }
    }
    draft.summary(draft_id)
}

/// `add_strategy_input` — append an `InputDeclaration` to a draft.
pub fn add_strategy_input(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let mut store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get_mut(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    let lane = params
        .get("lane")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let instrument = params
        .get("instrument")
        .and_then(|v| v.as_str())
        .unwrap_or("$bound_at_init")
        .to_owned();
    let features: Vec<String> = params
        .get("features")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_owned()))
                .collect()
        })
        .unwrap_or_default();
    draft.inputs.push(InputDeclaration {
        lane,
        instrument,
        features,
    });
    draft.summary(draft_id)
}

/// `add_condition_node` — append a Condition node to a draft.
pub fn add_condition_node(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let mut store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get_mut(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    let node_id = params
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if draft.nodes.iter().any(|n| n.id == node_id) {
        return json!({ "error": "duplicate_node_id", "node_id": node_id });
    }
    let expr = params
        .get("expr")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    draft.nodes.push(Node {
        id: node_id,
        kind: NodeKind::Condition { expr },
    });
    draft.summary(draft_id)
}

/// `add_signal_node` — append a Signal node to a draft.
pub fn add_signal_node(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let mut store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get_mut(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    let node_id = params
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if draft.nodes.iter().any(|n| n.id == node_id) {
        return json!({ "error": "duplicate_node_id", "node_id": node_id });
    }
    let when = params
        .get("when")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let emit = params
        .get("emit")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    draft.nodes.push(Node {
        id: node_id,
        kind: NodeKind::Signal { when, emit },
    });
    draft.summary(draft_id)
}

/// `add_strategy_action` — append a PlaceOrder action to a draft.
pub fn add_strategy_action(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let mut store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get_mut(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    let on_signal = params
        .get("on_signal")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let side_str = params.get("side").and_then(|v| v.as_str()).unwrap_or("buy");
    let side =
        match serde_json::from_value::<domain::order::Side>(Value::String(side_str.to_owned())) {
            Ok(s) => s,
            Err(_) => return json!({ "error": "invalid_side", "valid_values": ["buy", "sell"] }),
        };
    let size_mode_str = params
        .get("size_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("fixed");
    let size_mode = match size_mode_str {
        "fixed" => SizeMode::Fixed,
        "percent_of_balance" => SizeMode::PercentOfBalance,
        "risk_unit" => SizeMode::RiskUnit,
        _ => {
            return json!({ "error": "invalid_size_mode", "valid_values": ["fixed", "percent_of_balance", "risk_unit"] })
        }
    };
    let size = params
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_owned();
    draft.actions.push(Action {
        on_signal,
        kind: ActionKind::PlaceOrder {
            order: OrderSpec {
                side,
                size_mode,
                size,
            },
        },
    });
    draft.summary(draft_id)
}

/// `set_risk_overrides` — overwrite risk overrides on a draft.
pub fn set_risk_overrides(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let mut store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get_mut(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    if let Some(v) = params.get("max_position").and_then(|v| v.as_str()) {
        match v.parse::<rust_decimal::Decimal>() {
            Ok(d) => draft.risk_overrides.max_position = Some(d),
            Err(_) => return json!({ "error": "invalid_decimal", "field": "max_position" }),
        }
    }
    if let Some(v) = params
        .get("max_order_rate_per_minute")
        .and_then(|v| v.as_u64())
    {
        draft.risk_overrides.max_order_rate_per_minute = Some(v as u32);
    }
    if let Some(v) = params
        .get("max_order_rate_per_second")
        .and_then(|v| v.as_u64())
    {
        draft.risk_overrides.max_order_rate_per_second = Some(v as u32);
    }
    draft.summary(draft_id)
}

/// `get_draft_summary` — return the current draft as JSON without mutating.
pub fn get_draft_summary(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };
    let store = ctx.draft_store.lock().expect("draft_store lock poisoned");
    let Some(draft) = store.get(&draft_id) else {
        return draft_not_found(draft_id_str);
    };
    // Return the full typed definition so the agent can inspect it.
    let def = draft_to_definition(draft);
    match def {
        Ok(d) => {
            serde_json::to_value(&d).unwrap_or_else(|_| json!({"error": "serialization_error"}))
        }
        Err(msg) => json!({ "draft_id": draft_id.to_string(), "incomplete": true, "reason": msg }),
    }
}

fn draft_to_definition(draft: &StrategyDraft) -> Result<StrategyDefinition, String> {
    let strategy_id = draft.strategy_id.clone().ok_or("strategy_id not set")?;
    let asset_class = draft.asset_class.clone().ok_or("asset_class not set")?;
    Ok(StrategyDefinition {
        strategy_id,
        definition_version: draft.definition_version.clone(),
        asset_class,
        min_trust_tier: draft
            .min_trust_tier
            .unwrap_or(TrustTier::CentralizedExchange),
        inputs: draft.inputs.clone(),
        nodes: draft.nodes.clone(),
        actions: draft.actions.clone(),
        risk_overrides: draft.risk_overrides.clone(),
    })
}

/// `finalize_strategy` — assemble draft → validate → persist.
pub fn finalize_strategy(ctx: &McpContext, params: &Value) -> Value {
    let draft_id_str = params
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(draft_id) = Uuid::parse_str(draft_id_str) else {
        return draft_not_found(draft_id_str);
    };

    // Assemble the definition from the draft (while holding the lock).
    let def_result = {
        let store = ctx.draft_store.lock().expect("draft_store lock poisoned");
        let Some(draft) = store.get(&draft_id) else {
            return draft_not_found(draft_id_str);
        };
        draft_to_definition(draft)
    };

    let def = match def_result {
        Ok(d) => d,
        Err(reason) => {
            return json!({
                "valid": false,
                "errors": [{ "path": "<root>", "message": reason }]
            });
        }
    };

    // Validate using the shared validator.
    match strategy_validator::validate(&def) {
        Ok(_) => {}
        Err(errs) => {
            let errors: Vec<ValidationErrorItem> = errs
                .into_iter()
                .map(|e| ValidationErrorItem {
                    path: e.path,
                    message: e.message,
                })
                .collect();
            return json!({ "valid": false, "errors": errors });
        }
    }

    // Persist via authoring helper (which bypasses JSON-parse step).
    match create_strategy_from_def(ctx, def) {
        Ok(r) => {
            // Remove the draft on success.
            ctx.draft_store
                .lock()
                .expect("draft_store lock poisoned")
                .remove(&draft_id);
            json!({
                "store_id": r.store_id,
                "strategy_id": r.strategy_id,
                "valid": true,
            })
        }
        Err(e) => {
            json!({ "valid": false, "errors": e.errors })
        }
    }
}
