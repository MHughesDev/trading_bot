//! The validation ladder (harness guide §3.2).
//!
//! Every model output passes through five rungs, in order, at every tier. Frontier
//! models fail these less often, not never — a hallucinated tool name from a
//! frontier model is the same bug as one from a 7B, and it is cheaper to catch than
//! to debug from a transcript.
//!
//! The rung that earns its place most often is **step 2, the name check**. A call to
//! a tool that exists but was not exposed this step is answered with the list of
//! tools that *were* — which turns a routing miss from a dead end into one wasted
//! turn.
//!
//! Failures return ONE short corrective message naming the exact violation. Not a
//! stack trace, not a schema dump: error text is model-facing UX, and a model given
//! three paragraphs of JSON Schema retries with a guess.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::registry::{ToolDef, ToolRegistry};

/// Which rung rejected the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    Parse,
    Name,
    Schema,
    Semantic,
}

#[derive(Debug, Clone, Serialize)]
pub struct Rejection {
    pub rung: Rung,
    /// Stable machine code, e.g. `"validation.unknown_tool"`.
    pub code: &'static str,
    /// The single corrective sentence handed back to the model.
    pub message: String,
}

impl Rejection {
    fn new(rung: Rung, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            rung,
            code,
            message: message.into(),
        }
    }
}

/// A tool call that survived the ladder.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Map<String, Value>,
    /// Arguments the harness fixed rather than rejected (`"5"` → `5`).
    pub coerced: Vec<String>,
}

/// Step 1 — parse. Extracts a tool call from whatever the model actually emitted.
///
/// Strips markdown fences and leading prose, because both are common and neither is
/// a real error. A model that wraps valid JSON in ```json has not failed; a harness
/// that rejects it has.
pub fn parse(raw: &str) -> Result<Value, Rejection> {
    let trimmed = raw.trim();
    let body = strip_fence(trimmed);

    if let Ok(v) = serde_json::from_str::<Value>(body) {
        return Ok(v);
    }
    // Prose before the object is the other common shape ("I'll read the file: {...}").
    if let Some(start) = body.find('{') {
        if let Some(end) = body.rfind('}') {
            if end > start {
                if let Ok(v) = serde_json::from_str::<Value>(&body[start..=end]) {
                    return Ok(v);
                }
            }
        }
    }
    Err(Rejection::new(
        Rung::Parse,
        "validation.unparseable",
        "your reply was not a tool call. Reply with a single JSON object: {\"name\": \"<tool>\", \"arguments\": {...}}",
    ))
}

fn strip_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        return rest.strip_suffix("```").unwrap_or(rest).trim();
    }
    s
}

/// Steps 2–4 — name, schema and semantic checks.
///
/// `exposed` is the set the registry actually put in front of the model this step.
pub fn validate(
    registry: &ToolRegistry,
    exposed: &BTreeSet<String>,
    call: &Value,
) -> Result<ToolCall, Rejection> {
    // Step 2 — name.
    let name = call
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Rejection::new(
                Rung::Parse,
                "validation.no_name",
                "the tool call has no `name` field",
            )
        })?
        .to_string();

    let Some(def) = registry.get(&name) else {
        return Err(Rejection::new(
            Rung::Name,
            "validation.unknown_tool",
            format!(
                "there is no tool called {name:?}. Available this step: {}",
                list(exposed)
            ),
        ));
    };
    if !exposed.contains(&name) {
        // Exists but was not exposed: a routing miss, and the escape hatch is the
        // documented way out of it.
        return Err(Rejection::new(
            Rung::Name,
            "validation.tool_not_exposed",
            format!(
                "{name:?} is not available this step. Available: {}. Call search_tools to request another.",
                list(exposed)
            ),
        ));
    }

    // Step 3 — schema.
    let args = call
        .get("arguments")
        .or_else(|| call.get("input"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let Value::Object(args) = args else {
        return Err(Rejection::new(
            Rung::Schema,
            "validation.arguments_not_object",
            "`arguments` must be a JSON object",
        ));
    };

    let (args, coerced) = check_schema(def, args)?;

    Ok(ToolCall {
        name,
        arguments: args,
        coerced,
    })
}

fn list(names: &BTreeSet<String>) -> String {
    names.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// Step 3, with the coercions of §3.2.
///
/// Trivially fixable issues are fixed in code: `"5"` → `5`, `"true"` → `true`.
/// **Ambiguous ones are never coerced** — turning `"maybe"` into `false` produces a
/// call that runs and does the wrong thing, which is strictly worse than a rejection
/// the model can correct.
fn check_schema(
    def: &ToolDef,
    mut args: Map<String, Value>,
) -> Result<(Map<String, Value>, Vec<String>), Rejection> {
    let schema = &def.input_schema;
    let props = schema.get("properties").and_then(Value::as_object);
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    for key in &required {
        if !args.contains_key(*key) {
            return Err(Rejection::new(
                Rung::Schema,
                "validation.missing_required",
                format!("{:?} needs the `{key}` argument", def.name),
            ));
        }
    }

    let Some(props) = props else {
        return Ok((args, Vec::new()));
    };

    let mut coerced = Vec::new();
    let keys: Vec<String> = args.keys().cloned().collect();
    for key in keys {
        let Some(spec) = props.get(&key) else {
            // An unknown argument is dropped rather than rejected: models add
            // plausible extras, and failing the call for one costs a turn to learn
            // something the harness can simply ignore.
            args.remove(&key);
            continue;
        };
        let want = spec.get("type").and_then(Value::as_str).unwrap_or("string");
        let got = args.get(&key).cloned().unwrap_or(Value::Null);

        if let Some(en) = spec.get("enum").and_then(Value::as_array) {
            if !en.iter().any(|v| v == &got) {
                return Err(Rejection::new(
                    Rung::Schema,
                    "validation.bad_enum",
                    format!(
                        "`{key}` must be one of {}; got {got}",
                        en.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            continue;
        }

        match coerce(&got, want) {
            Coerced::Ok => {}
            Coerced::Fixed(v) => {
                args.insert(key.clone(), v);
                coerced.push(key.clone());
            }
            Coerced::No => {
                return Err(Rejection::new(
                    Rung::Schema,
                    "validation.bad_type",
                    format!("`{key}` must be a {want}; got {got}"),
                ));
            }
        }
    }

    Ok((args, coerced))
}

enum Coerced {
    Ok,
    Fixed(Value),
    No,
}

fn coerce(got: &Value, want: &str) -> Coerced {
    match (want, got) {
        ("string", Value::String(_))
        | ("integer", Value::Number(_))
        | ("number", Value::Number(_))
        | ("boolean", Value::Bool(_))
        | ("array", Value::Array(_))
        | ("object", Value::Object(_)) => {
            if want == "integer" && !got.as_i64().is_some_and(|_| true) {
                return Coerced::No;
            }
            Coerced::Ok
        }
        // The unambiguous fixes.
        ("integer", Value::String(s)) => s
            .trim()
            .parse::<i64>()
            .map_or(Coerced::No, |n| Coerced::Fixed(Value::from(n))),
        ("number", Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .map_or(Coerced::No, |n| Coerced::Fixed(Value::from(n))),
        ("boolean", Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Coerced::Fixed(Value::Bool(true)),
            "false" => Coerced::Fixed(Value::Bool(false)),
            // "yes", "1", "maybe" are guesses. A wrong boolean runs the tool and
            // does the opposite of what was meant.
            _ => Coerced::No,
        },
        ("string", Value::Number(n)) => Coerced::Fixed(Value::String(n.to_string())),
        _ => Coerced::No,
    }
}

/// Strict conformance of a value to a JSON Schema subset — **with no coercion**.
///
/// This is the fence check (§3.1, ADR-0032), and it is deliberately *not*
/// `check_schema`. Coercion is the right answer when a model guessed a type; it is
/// exactly the wrong answer when a **backend** returned a shape the grammar it was
/// handed forbade. Repairing that would destroy the only evidence that constrained
/// decoding is not actually running.
///
/// Supports the subset a tool-call envelope uses: `type`, `properties`, `required`,
/// `enum` and `items`. Anything it does not understand it **accepts** — a checker
/// that invented failures would fence healthy sessions, which is a worse bug than
/// the one it is looking for.
pub fn conforms(schema: &Value, value: &Value) -> Result<(), String> {
    conforms_at("", schema, value)
}

fn conforms_at(path: &str, schema: &Value, value: &Value) -> Result<(), String> {
    let at = if path.is_empty() {
        "the reply".to_string()
    } else {
        format!("`{path}`")
    };

    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Err(format!(
                "{at} is {} but the schema allows only {}",
                compact(value),
                allowed.iter().map(compact).collect::<Vec<_>>().join(", ")
            ));
        }
    }

    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        let ok = match ty {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            // An unknown type keyword is not evidence of anything.
            _ => true,
        };
        if !ok {
            return Err(format!("{at} should be {ty} but is {}", type_of(value)));
        }
    }

    if let Some(req) = schema.get("required").and_then(Value::as_array) {
        if let Some(obj) = value.as_object() {
            for k in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(k) {
                    return Err(format!("{at} is missing the required field `{k}`"));
                }
            }
        }
    }

    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        if let Some(obj) = value.as_object() {
            for (k, sub) in props {
                if let Some(v) = obj.get(k) {
                    let child = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    conforms_at(&child, sub, v)?;
                }
            }
        }
    }

    if let (Some(items), Some(arr)) = (schema.get("items"), value.as_array()) {
        for (i, v) in arr.iter().enumerate() {
            conforms_at(&format!("{path}[{i}]"), items, v)?;
        }
    }

    Ok(())
}

fn compact(v: &Value) -> String {
    match v {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Step 5 — the retry budget.
///
/// After `max_retries` the step fails to the orchestrator rather than looping. A
/// silent loop is the failure mode this exists to prevent: it burns the whole budget
/// and produces nothing anyone can act on.
#[derive(Debug, Clone)]
pub struct RetryBudget {
    max: u32,
    used: u32,
    history: Vec<String>,
}

impl RetryBudget {
    #[must_use]
    pub fn new(max: u32) -> Self {
        Self {
            max,
            used: 0,
            history: Vec::new(),
        }
    }

    /// Records a rejection. Returns the corrective message to append, or `None` when
    /// the budget is spent and the step must fail upward.
    pub fn record(&mut self, r: &Rejection) -> Option<String> {
        self.used += 1;
        self.history.push(r.code.to_string());
        if self.used >= self.max {
            return None;
        }
        Some(r.message.clone())
    }

    #[must_use]
    pub fn exhausted(&self) -> bool {
        self.used >= self.max
    }

    /// How many rejections have been recorded. For the timeline's attempt number —
    /// the loop should not have to keep a second counter that can disagree with this
    /// one.
    #[must_use]
    pub fn used(&self) -> u32 {
        self.used
    }

    /// Whether the same rejection keeps repeating — stuck detection for the ladder
    /// (§5.3). Three identical failures mean corrective messages are not landing,
    /// and more of them will not help.
    #[must_use]
    pub fn is_stuck(&self) -> bool {
        self.history.len() >= 3
            && self.history[self.history.len() - 3..]
                .iter()
                .all(|c| c == &self.history[self.history.len() - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Risk;
    use serde_json::json;

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(ToolDef {
            name: "read_bars".into(),
            namespace: "data".into(),
            description: "Read bars. Use when the step needs price history.".into(),
            risk: Risk::Read,
            core: false,
            idempotent: true,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "instrument": {"type": "string"},
                    "limit": {"type": "integer"},
                    "tf": {"type": "string", "enum": ["1m", "1h"]},
                    "adjusted": {"type": "boolean"}
                },
                "required": ["instrument"]
            }),
        })
        .unwrap();
        r.register(ToolDef {
            name: "hidden_tool".into(),
            namespace: "admin".into(),
            description: "Not exposed in these tests. Use never.".into(),
            risk: Risk::Destructive,
            core: false,
            idempotent: false,
            input_schema: json!({"type": "object", "properties": {}}),
        })
        .unwrap();
        r
    }

    fn exposed() -> BTreeSet<String> {
        ["read_bars".to_string()].into_iter().collect()
    }

    // ── Step 1: parse ───────────────────────────────────────────────────────

    #[test]
    fn a_fenced_tool_call_parses() {
        let v = parse("```json\n{\"name\":\"read_bars\",\"arguments\":{}}\n```").unwrap();
        assert_eq!(v["name"], "read_bars");
    }

    #[test]
    fn prose_before_the_object_is_stripped() {
        let v =
            parse("Sure, I'll read those bars: {\"name\":\"read_bars\",\"arguments\":{}}").unwrap();
        assert_eq!(v["name"], "read_bars");
    }

    #[test]
    fn unparseable_output_names_the_expected_shape() {
        let e = parse("I think we should look at BTC.").unwrap_err();
        assert_eq!(e.rung, Rung::Parse);
        assert!(e.message.contains("{\"name\""), "the fix shows the shape");
    }

    // ── Step 2: name ────────────────────────────────────────────────────────

    #[test]
    fn a_hallucinated_tool_gets_the_available_list() {
        let e = validate(&registry(), &exposed(), &json!({"name": "read_barz"})).unwrap_err();
        assert_eq!(e.rung, Rung::Name);
        assert_eq!(e.code, "validation.unknown_tool");
        assert!(e.message.contains("read_bars"));
    }

    /// The rung that turns a routing miss from a dead end into one wasted turn.
    #[test]
    fn a_real_but_unexposed_tool_is_refused_and_points_at_the_escape_hatch() {
        let e = validate(&registry(), &exposed(), &json!({"name": "hidden_tool"})).unwrap_err();
        assert_eq!(e.code, "validation.tool_not_exposed");
        assert!(e.message.contains("search_tools"));
    }

    // ── Step 3: schema ──────────────────────────────────────────────────────

    #[test]
    fn a_missing_required_argument_names_the_argument() {
        let e = validate(
            &registry(),
            &exposed(),
            &json!({"name": "read_bars", "arguments": {"limit": 10}}),
        )
        .unwrap_err();
        assert!(e.message.contains("instrument"));
    }

    #[test]
    fn trivially_fixable_types_are_coerced_in_code() {
        let c = validate(
            &registry(),
            &exposed(),
            &json!({"name": "read_bars", "arguments": {"instrument": "BTC", "limit": "10", "adjusted": "true"}}),
        )
        .unwrap();
        assert_eq!(c.arguments["limit"], json!(10));
        assert_eq!(c.arguments["adjusted"], json!(true));
        assert_eq!(c.coerced.len(), 2);
    }

    /// The line the coercion rule draws. A wrong boolean runs the tool and does the
    /// opposite of what was meant, which is worse than a rejection.
    #[test]
    fn ambiguous_values_are_never_coerced() {
        for bad in ["maybe", "yes", "1"] {
            let e = validate(
                &registry(),
                &exposed(),
                &json!({"name": "read_bars", "arguments": {"instrument": "BTC", "adjusted": bad}}),
            )
            .unwrap_err();
            assert_eq!(e.code, "validation.bad_type", "{bad} should not coerce");
        }
    }

    #[test]
    fn an_enum_violation_lists_the_legal_values() {
        let e = validate(
            &registry(),
            &exposed(),
            &json!({"name": "read_bars", "arguments": {"instrument": "BTC", "tf": "5m"}}),
        )
        .unwrap_err();
        assert!(e.message.contains("1m") && e.message.contains("1h"));
    }

    #[test]
    fn an_unknown_argument_is_dropped_rather_than_failing_the_call() {
        let c = validate(
            &registry(),
            &exposed(),
            &json!({"name": "read_bars", "arguments": {"instrument": "BTC", "colour": "red"}}),
        )
        .unwrap();
        assert!(!c.arguments.contains_key("colour"));
        assert_eq!(c.arguments["instrument"], "BTC");
    }

    // ── Step 5: the retry budget ────────────────────────────────────────────

    #[test]
    fn the_retry_budget_hands_back_corrections_then_gives_up() {
        let mut b = RetryBudget::new(3);
        let r = Rejection::new(Rung::Schema, "validation.bad_type", "fix it");
        assert_eq!(b.record(&r).as_deref(), Some("fix it"));
        assert_eq!(b.record(&r).as_deref(), Some("fix it"));
        assert_eq!(
            b.record(&r),
            None,
            "the step fails upward instead of looping"
        );
        assert!(b.exhausted());
    }

    #[test]
    fn three_identical_failures_count_as_stuck() {
        let mut b = RetryBudget::new(10);
        let r = Rejection::new(Rung::Schema, "validation.bad_type", "fix it");
        b.record(&r);
        b.record(&r);
        assert!(!b.is_stuck());
        b.record(&r);
        assert!(
            b.is_stuck(),
            "corrective messages are not landing; more will not help"
        );
    }

    #[test]
    fn every_rejection_is_one_short_actionable_sentence() {
        let cases = vec![
            parse("nonsense").unwrap_err(),
            validate(&registry(), &exposed(), &json!({"name": "nope"})).unwrap_err(),
            validate(
                &registry(),
                &exposed(),
                &json!({"name": "read_bars", "arguments": {}}),
            )
            .unwrap_err(),
        ];
        for e in cases {
            assert!(!e.message.is_empty());
            assert!(
                e.message.len() < 300,
                "a model given three paragraphs of schema retries with a guess: {}",
                e.message
            );
            assert!(e.code.starts_with("validation."));
        }
    }
}
