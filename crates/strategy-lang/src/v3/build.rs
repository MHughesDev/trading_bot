//! Builder operations — the third view of a strategy (Set U U-7.4; FEAT-007
//! §10; ADR-0045 decision 3).
//!
//! A small model that cannot hold a grammar can still build a strategy one
//! constrained step at a time: start it, add a node, connect an output to a
//! port, add a rule. Each step is a JSON object with a fixed shape, and the
//! result is the canonical graph — the same graph the equivalent SLv3 text
//! parses to, checked by the same validator. There is no builder-only
//! artefact for anything to drift from.

use std::collections::BTreeMap;

use domain::strategy_graph::StrategyGraph;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::validate::Finding;

/// One builder step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum BuildOp {
    /// Start a strategy. Must come first when there is no graph to edit.
    New {
        name: String,
        kind: String,
        asset_class: String,
        timeframe: String,
    },
    /// A node: an operator and its arguments, each a number, a `{"str": …}`
    /// literal, a reference string, or a list.
    AddNode {
        id: String,
        #[serde(rename = "operator")]
        operator: String,
        #[serde(default)]
        args: BTreeMap<String, Value>,
    },
    /// Wire `from` (a node, `node.field`, a source like `close`, `$param` or
    /// `state.x`) into `port` of node `to`.
    Connect {
        from: String,
        to: String,
        port: String,
    },
    /// Set a port to a literal: a number, `true`/`false`, or a string (which is
    /// stored as a literal, never as a reference).
    SetArg {
        node: String,
        port: String,
        value: Value,
    },
    RemoveNode {
        id: String,
    },
    /// A rule: `when` is a reference (or an inline node), `then` an intent.
    AddRule {
        when: Value,
        then: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        group: Option<String>,
    },
    SetParam {
        name: String,
        spec: Value,
    },
    SetUniverse {
        universe: Value,
    },
    SetSelect {
        score: Value,
        #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
        filter: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        top_n: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<String>,
        #[serde(default)]
        attributes: BTreeMap<String, Value>,
    },
}

fn refuse(i: usize, code: &str, message: impl Into<String>, fix: impl Into<String>) -> Finding {
    Finding {
        code: code.into(),
        at: format!("ops[{i}]"),
        message: message.into(),
        fix: fix.into(),
        example: None,
        see: Some("slv3#builder"),
    }
}

fn node_mut<'a>(g: &'a mut Map<String, Value>, id: &str) -> Option<&'a mut Map<String, Value>> {
    g.get_mut("nodes")?
        .as_array_mut()?
        .iter_mut()
        .filter_map(Value::as_object_mut)
        .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
}

/// Apply `ops` to `start` (or to nothing, when the first op is `new`).
///
/// # Errors
/// The first step that cannot be applied, or a result that is not a
/// structurally valid graph — named by the step that broke it.
pub fn build(start: Option<&StrategyGraph>, ops: &[BuildOp]) -> Result<StrategyGraph, Finding> {
    let mut g: Option<Map<String, Value>> = match start {
        Some(s) => match serde_json::to_value(s) {
            Ok(Value::Object(o)) => Some(o),
            _ => None,
        },
        None => None,
    };
    for (i, op) in ops.iter().enumerate() {
        if let BuildOp::New {
            name,
            kind,
            asset_class,
            timeframe,
        } = op
        {
            let mut o = Map::new();
            o.insert("format".into(), json!(domain::strategy_graph::FORMAT));
            o.insert("kind".into(), json!(kind));
            o.insert("name".into(), json!(name));
            o.insert("asset_class".into(), json!(asset_class));
            o.insert("timeframe".into(), json!(timeframe));
            o.insert("nodes".into(), json!([]));
            if kind == "trading" {
                o.insert("decide".into(), json!({"rules": [], "otherwise": "hold"}));
            }
            g = Some(o);
            continue;
        }
        let Some(graph) = g.as_mut() else {
            return Err(refuse(
                i,
                "builder_no_strategy",
                "there is no strategy to edit yet",
                "start with `{\"op\": \"new\", …}`",
            ));
        };
        match op {
            BuildOp::New { .. } => unreachable!("handled above"),
            BuildOp::AddNode { id, operator, args } => {
                if node_mut(graph, id).is_some() {
                    return Err(refuse(
                        i,
                        "duplicate_node_id",
                        format!("there is already a node `{id}`"),
                        "pick another id, or `remove_node` first",
                    ));
                }
                let mut n = Map::new();
                n.insert("id".into(), json!(id));
                n.insert("op".into(), json!(operator));
                if !args.is_empty() {
                    n.insert("args".into(), json!(args));
                }
                if let Some(a) = graph
                    .entry("nodes")
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                {
                    a.push(Value::Object(n));
                }
            }
            BuildOp::Connect { from, to, port } => {
                let Some(n) = node_mut(graph, to) else {
                    return Err(refuse(
                        i,
                        "unknown_ref",
                        format!("there is no node `{to}` to connect into"),
                        "add the node first",
                    ));
                };
                if let Some(a) = n.entry("args").or_insert_with(|| json!({})).as_object_mut() {
                    a.insert(port.clone(), json!(from));
                }
            }
            BuildOp::SetArg { node, port, value } => {
                let v = match value {
                    // A builder string is a literal; references go through
                    // `connect`, so the two can never be confused.
                    Value::String(s) => json!({"str": s}),
                    other => other.clone(),
                };
                let Some(n) = node_mut(graph, node) else {
                    return Err(refuse(
                        i,
                        "unknown_ref",
                        format!("there is no node `{node}`"),
                        "add the node first",
                    ));
                };
                if let Some(a) = n.entry("args").or_insert_with(|| json!({})).as_object_mut() {
                    a.insert(port.clone(), v);
                }
            }
            BuildOp::RemoveNode { id } => {
                let Some(nodes) = graph.get_mut("nodes").and_then(Value::as_array_mut) else {
                    continue;
                };
                let before = nodes.len();
                nodes.retain(|n| n.get("id").and_then(Value::as_str) != Some(id));
                if nodes.len() == before {
                    return Err(refuse(
                        i,
                        "unknown_ref",
                        format!("there is no node `{id}`"),
                        "check the id",
                    ));
                }
            }
            BuildOp::AddRule {
                when,
                then,
                name,
                group,
            } => {
                let Some(rules) = graph
                    .get_mut("decide")
                    .and_then(|d| d.get_mut("rules"))
                    .and_then(Value::as_array_mut)
                else {
                    return Err(refuse(
                        i,
                        "sink_mismatch",
                        "rules belong to a trading strategy",
                        "a selection strategy ends in `set_select`",
                    ));
                };
                let mut r = Map::new();
                r.insert("when".into(), when.clone());
                r.insert("then".into(), then.clone());
                if let Some(n) = name {
                    r.insert("name".into(), json!(n));
                }
                if let Some(gr) = group {
                    r.insert("group".into(), json!(gr));
                }
                rules.push(Value::Object(r));
            }
            BuildOp::SetParam { name, spec } => {
                if let Some(p) = graph
                    .entry("params")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                {
                    p.insert(name.clone(), spec.clone());
                }
            }
            BuildOp::SetUniverse { universe } => {
                graph.insert("universe".into(), universe.clone());
            }
            BuildOp::SetSelect {
                score,
                filter,
                top_n,
                order,
                attributes,
            } => {
                let mut s = Map::new();
                s.insert("score".into(), score.clone());
                if let Some(f) = filter {
                    s.insert("where".into(), f.clone());
                }
                if let Some(n) = top_n {
                    s.insert("top_n".into(), json!(n));
                }
                if let Some(o) = order {
                    s.insert("order".into(), json!(o));
                }
                if !attributes.is_empty() {
                    s.insert("attributes".into(), json!(attributes));
                }
                graph.insert("select".into(), Value::Object(s));
            }
        }
    }
    let Some(g) = g else {
        return Err(refuse(
            0,
            "builder_no_strategy",
            "no operations, and nothing to edit",
            "start with `{\"op\": \"new\", …}`",
        ));
    };
    StrategyGraph::from_json(&Value::Object(g)).map_err(|e| {
        let code = match &e {
            domain::strategy_graph::GraphParseError::Structural(s) => s.code(),
            domain::strategy_graph::GraphParseError::Malformed(_) => "malformed",
        };
        refuse(ops.len().saturating_sub(1), code, e.to_string(), "undo the last step, or finish the strategy (a trading strategy needs rules; a selection a universe and `set_select`)")
    })
}
