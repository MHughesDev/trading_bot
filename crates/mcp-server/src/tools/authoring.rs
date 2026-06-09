//! Authoring tools: `validate_strategy` and `create_strategy`.
//!
//! Mandatory validation before create: a definition is rejected with
//! structured errors if it is malformed or loosens risk limits.

use serde::{Deserialize, Serialize};

use domain::strategy_def::StrategyDefinition;
use strategy_validator::validate;

use crate::McpContext;

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationErrorItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationErrorItem {
    pub path: String,
    pub message: String,
}

/// `validate_strategy` — validate a definition JSON without persisting.
///
/// Returns structured errors the agent can parse and act on.
pub fn validate_strategy(definition_json: &str) -> ValidationResult {
    let def: StrategyDefinition = match serde_json::from_str(definition_json) {
        Ok(d) => d,
        Err(e) => {
            return ValidationResult {
                valid: false,
                errors: vec![ValidationErrorItem {
                    path: "<root>".into(),
                    message: format!("JSON parse error: {e}"),
                }],
            }
        }
    };

    match validate(&def) {
        Ok(_) => ValidationResult {
            valid: true,
            errors: vec![],
        },
        Err(errs) => ValidationResult {
            valid: false,
            errors: errs
                .into_iter()
                .map(|e| ValidationErrorItem {
                    path: e.path,
                    message: e.message,
                })
                .collect(),
        },
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateResult {
    pub strategy_id: String,
    pub store_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateError {
    pub error: String,
    pub errors: Vec<ValidationErrorItem>,
}

/// `create_strategy` — validate and persist a strategy definition.
///
/// Returns the store ID on success, or structured errors on validation failure.
pub fn create_strategy(
    ctx: &McpContext,
    definition_json: &str,
) -> Result<CreateResult, CreateError> {
    let vr = validate_strategy(definition_json);
    if !vr.valid {
        return Err(CreateError {
            error: "validation_failed".into(),
            errors: vr.errors,
        });
    }

    let def: StrategyDefinition = serde_json::from_str(definition_json).expect("already validated");
    let store_id = uuid::Uuid::new_v4();
    let strategy_id = def.strategy_id.clone();

    ctx.strategy_store
        .lock()
        .expect("strategy_store lock poisoned")
        .insert(store_id, def);

    Ok(CreateResult {
        strategy_id,
        store_id: store_id.to_string(),
    })
}
