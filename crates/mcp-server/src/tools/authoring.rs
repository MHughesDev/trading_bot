//! Authoring tools: `validate_strategy`, `create_strategy`, `get_strategy`,
//! `list_strategies`.
//!
//! Validation runs locally (the validator is pure and identical to the one the
//! platform uses); persistence goes through the platform API per ADR-0010, so
//! strategies created here are real: visible in the UI, usable by slug in
//! backtests, and durable across restarts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use domain::strategy_def::StrategyDefinition;
use strategy_validator::validate;

use crate::ApiClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationErrorItem>,
}

/// One rejection, as the model receives it.
///
/// `code`, `fix` and `see` are AGENT-011 §5 (Set S task S-5.3). The validator
/// has carried a fix for every code since that task; this struct used to drop it
/// on the floor, which meant the teaching existed and no caller ever saw it — a
/// rejection that says what is wrong and not what to change leaves a small model
/// guessing, and every guess costs a turn.
///
/// `fix` and `see` are skipped when absent rather than serialised as null: a
/// null `fix` reads as "there is no fix", which is a different and more
/// discouraging claim than "this error has no canned edit".
#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationErrorItem {
    /// The stable identifier, so a retry policy can branch without parsing prose.
    pub code: String,
    pub path: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub see: Option<String>,
    /// A correct instance of this clause, attached when the same `path` has now
    /// been rejected repeatedly. The cheapest understanding gain on a small tier
    /// is seeing one, not reading another rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

impl From<strategy_validator::ValidationError> for ValidationErrorItem {
    /// The one mapping. There were two identical copies of it — here and in
    /// `builder.rs` — which is how the builder would have kept returning bare
    /// rejections after this one learned to teach.
    ///
    /// The example is attached on **every** rejection rather than only the
    /// third. AGENT-011 §5 counts rejections within a run, and that is state a
    /// pure function of one document does not have; withholding the example
    /// until a counter this crate cannot keep would mean never attaching it,
    /// and an example the model did not need costs it one clause of context.
    fn from(e: strategy_validator::ValidationError) -> Self {
        Self {
            code: e.code.to_string(),
            fix: e.fix().map(str::to_string),
            see: e.see().map(str::to_string),
            example: e.example().map(str::to_string),
            path: e.path,
            message: e.message,
        }
    }
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
                    code: "invalid_json".into(),
                    path: "<root>".into(),
                    message: format!("JSON parse error: {e}"),
                    fix: Some(
                        "the definition must be a JSON object. Check for a trailing comma, an                          unquoted key, or a single quote where a double quote belongs."
                            .into(),
                    ),
                    see: Some("guide#definition".into()),
                    example: None,
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
                .map(ValidationErrorItem::from)
                .collect(),
        },
    }
}

/// `create_strategy` — validate locally, then persist via the platform API.
///
/// `parent_strategy_id` is the optional explicit lineage described on the tool
/// spec below: the slug this definition was derived from, when it came from a
/// *different* lineage than its own.
pub async fn create_strategy(
    api: &ApiClient,
    definition_json: &str,
    parent_strategy_id: Option<&str>,
) -> Value {
    let vr = validate_strategy(definition_json);
    if !vr.valid {
        return json!({ "error": "validation_failed", "errors": vr.errors });
    }
    let def: StrategyDefinition = serde_json::from_str(definition_json).expect("already validated");
    create_strategy_from_def(api, def, parent_strategy_id).await
}

/// Persist a typed, already-validated `StrategyDefinition` via the API.
///
/// Also used by `finalize_strategy` in the builder flow.
///
/// # The note this returns used to be wrong
///
/// It said "upserted by strategy_id — reusing this slug overwrites the
/// strategy", and the tool description drew the obvious conclusion: mint
/// `my_strat_v2` when iterating. Nothing is overwritten. `strategy_revision` is
/// append-only and `strategy_definitions.strategy_id` is a pointer into it, so
/// re-saving a slug **appends** and moves the head; the previous definition is
/// still there and still reconstructable, which is the only reason
/// `parent_hash` can be derived at all. The advice was therefore the cause of
/// the thing it warned about: a new slug has no head, so every "v2" started a
/// lineage of one.
pub async fn create_strategy_from_def(
    api: &ApiClient,
    def: StrategyDefinition,
    parent_strategy_id: Option<&str>,
) -> Value {
    let body = match serde_json::to_value(&def) {
        Ok(v) => v,
        Err(e) => return json!({ "error": "serialization_error", "detail": e.to_string() }),
    };
    let path = match parent_strategy_id.filter(|p| !p.is_empty()) {
        Some(parent) => format!(
            "/api/strategies?parent_strategy_id={}",
            crate::tools::market::urlencode(parent)
        ),
        None => "/api/strategies".to_string(),
    };
    match api.post(&path, body).await {
        Ok(resp) => json!({
            "strategy_id": resp.get("strategy_id").cloned().unwrap_or(json!(def.strategy_id)),
            "created": true,
            // Reported, not assumed. `parent_revision` is null on a genuine
            // first version and a hash on every later one; a caller that sees
            // null where it expected a chain has reused a slug it thought was
            // new, and that is worth seeing at the call rather than in an audit.
            "revision_hash": resp.get("revision_hash").cloned().unwrap_or(Value::Null),
            "parent_revision": resp.get("parent_revision").cloned().unwrap_or(Value::Null),
            "actor_kind": resp.get("actor_kind").cloned().unwrap_or(Value::Null),
            "note": "saved as a new revision of this slug. Re-saving the same slug appends to \
                     its history and moves the head; nothing is overwritten and the previous \
                     definition stays readable.",
        }),
        Err(e) => e.to_tool_error(),
    }
}

/// `get_strategy` — fetch a stored definition by slug.
pub async fn get_strategy(api: &ApiClient, params: &Value) -> Value {
    let slug = params
        .get("strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if slug.is_empty() {
        return json!({ "error": "missing_field", "field": "strategy_id" });
    }
    match api.get(&format!("/api/strategies/{slug}/config")).await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_strategies` — list stored strategy slugs.
pub async fn list_strategies(api: &ApiClient) -> Value {
    match api.get("/api/strategies").await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_compatible_strategies` — strategies whose data requirements the given
/// instrument/asset-class can satisfy (incompatible ones are omitted).
pub async fn list_compatible_strategies(api: &ApiClient, params: &Value) -> Value {
    let mut query = Vec::new();
    if let Some(i) = params.get("instrument_id").and_then(|v| v.as_str()) {
        query.push(format!("instrument={}", crate::tools::market::urlencode(i)));
    }
    let asset_class = params
        .get("asset_class")
        .and_then(|v| v.as_str())
        .unwrap_or("crypto_spot_cex");
    query.push(format!(
        "asset_class={}",
        crate::tools::market::urlencode(asset_class)
    ));
    match api
        .get(&format!("/api/strategies/apply-list?{}", query.join("&")))
        .await
    {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// Declarations for the five `strategy` tools (AGENT-006 §2.3).
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    let definition = json!({
        "type": "object",
        "required": ["definition_json"],
        "properties": {
            "definition_json": { "type": "object", "description": "Strategy definition OBJECT (format: get_authoring_guide). Send the object, not a string containing JSON." }
        }
    });
    // `create_strategy` takes one field the read-only tools do not: the explicit
    // lineage (ADR-D-21). Optional, and deliberately not defaulted — a first
    // version genuinely has no parent, and a required lineage field is a field
    // that gets filled with whatever is nearest.
    let create_definition = json!({
        "type": "object",
        "required": ["definition_json"],
        "properties": {
            "definition_json": { "type": "object", "description": "Strategy definition OBJECT (format: get_authoring_guide). Send the object, not a string containing JSON." },
            "parent_strategy_id": {
                "type": "string",
                "description": "OPTIONAL. The slug this was forked from, when it came from a DIFFERENT slug's lineage. Omit it when iterating on the same slug, and for a first version."
            }
        }
    });
    vec![
        ToolSpec::new(
            "validate_strategy",
            "strategy",
            Risk::Read,
            "Validate a strategy definition without persisting it; returns structured errors \
             (path plus message) you can fix and retry. Free and instant.",
            definition.clone(),
        )
        .internal(),
        ToolSpec::new(
            "create_strategy",
            "strategy",
            Risk::Write,
            "Validate and persist a strategy definition to the platform's library. Saving is \
             append-only: re-using the same strategy_id records a NEW revision whose parent is \
             the previous one and moves the slug's head to it. Nothing is overwritten and every \
             earlier version stays readable, so KEEP THE SAME SLUG while you iterate — that is \
             what links the versions into a lineage. Minting my_strat_v2 starts a separate \
             history with no parent. Use parent_strategy_id only when this definition came from \
             a different slug's lineage.",
            create_definition,
        )
        .internal()
        .evidence(&["strategy_id"])
        .notes("create_strategy"),
        ToolSpec::new(
            "get_strategy",
            "strategy",
            Risk::Read,
            "Fetch a stored strategy definition by its strategy_id slug.",
            json!({
                "type": "object",
                "required": ["strategy_id"],
                "properties": {
                    "strategy_id": { "type": "string", "description": "Slug returned by create_strategy / list_strategies" }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "list_strategies",
            "strategy",
            Risk::Read,
            "List all stored strategy slugs in the platform library.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "list_compatible_strategies",
            "strategy",
            Risk::Read,
            "List stored strategies whose data requirements the given instrument or asset class \
             can satisfy; incompatible ones are omitted, with each strategy's kind, trigger and \
             required lanes.",
            json!({
                "type": "object",
                "properties": {
                    "instrument_id": { "type": "string", "description": "Optional instrument filter" },
                    "asset_class": { "type": "string", "description": "Default crypto_spot_cex" }
                }
            }),
        )
        .internal(),
    ]
}
