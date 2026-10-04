//! v1 definitions → `tbot.strategy/3` graphs (Set U U-7.3; FEAT-007 §12).
//!
//! Every stored v1.0–1.5 definition either translates or is refused naming the
//! construct that has no translation yet — never translated approximately. A
//! translation carries the list of behaviours that change, because a
//! translated strategy is **not** expected to backtest like its old results:
//! those came from an evaluator that filled at the bar close, never called a
//! model, and skipped the risk gate.
//!
//! | v1 | v3 |
//! |---|---|
//! | `condition` (an expression) | a node: the same expression |
//! | `signal` + `place_order` action | a rule on the signal's rising edge: `when c and not prev(c, 1) -> order(…)` |
//! | `exits` stop / take-profit / trailing | a `protect` rule while in a position |
//! | `exits` max holding bars | `when state.bars_in_position >= n -> close` |
//! | `exits` on signal | `when <condition> and not state.flat -> close` |
//! | `model_forecast`, `inference`, `sizing` | `model` nodes, outputs read as fields |
//! | `combine` | `weighted_mean`, `max`, `min` or a sum |
//! | `parameters`, `constraints` | the same, unchanged |
//! | `risk_overrides.max_order_rate_per_minute` | `risk.max_orders_per_min` |

use std::collections::BTreeMap;

use domain::strategy_def::actions::{ActionKind, SizeMode};
use domain::strategy_def::exits::{DistanceUnit, ExitDistance, ExitPolicy, ExitRule};
use domain::strategy_def::nodes::{CombineOp, NodeKind};
use domain::strategy_def::StrategyDefinition;
use domain::strategy_graph::ops::{self, Ty};
use domain::strategy_graph::StrategyGraph;
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;
use serde_json::{json, Map, Value};

/// A behaviour the translated strategy has that the v1 one did not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BehaviourChange {
    pub code: &'static str,
    pub detail: String,
}

/// A translated strategy.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Translation {
    pub graph: StrategyGraph,
    pub text: String,
    pub changes: Vec<BehaviourChange>,
}

/// Why a definition was not translated: the construct, where it is, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("`{construct}` at {at} has no v3 translation yet: {why}")]
pub struct Refusal {
    pub construct: String,
    pub at: String,
    pub why: String,
}

fn refuse(construct: &str, at: impl Into<String>, why: impl Into<String>) -> Refusal {
    Refusal {
        construct: construct.into(),
        at: at.into(),
        why: why.into(),
    }
}

/// A v1 expression as SLv3: `feature('x')` reads a feature — or, when `x` is
/// a model output a node bound under that name, that node's field;
/// `bar('close')` is the source `close`; `param('p')` and `{{p}}` are `$p`.
fn expression(expr: &str, outputs: &BTreeMap<String, String>, at: &str) -> Result<Value, Refusal> {
    let mut out = String::new();
    let mut rest = expr;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("{{") {
            let Some(end) = r.find("}}") else {
                return Err(refuse("expression", at, "an unterminated `{{`"));
            };
            out.push('$');
            out.push_str(r[..end].trim());
            rest = &r[end + 2..];
            continue;
        }
        let call = ["feature", "bar", "param"].into_iter().find_map(|f| {
            let r = rest
                .strip_prefix(f)?
                .trim_start()
                .strip_prefix('(')?
                .trim_start();
            let q = r.chars().next().filter(|c| *c == '\'' || *c == '"')?;
            let body = &r[1..];
            let end = body.find(q)?;
            let after = body[end + 1..].trim_start().strip_prefix(')')?;
            Some((f, body[..end].to_string(), after))
        });
        if let Some((f, arg, after)) = call {
            if arg.contains("{{") {
                // `ema_{{p}}` is the operator with a parameter port — the
                // compiler lowers `ema(close, $p)` to the very `ema_N` column the
                // v1 name resolved to, so nothing about the value changes.
                if let Some(op) = parameterised_family(f, &arg) {
                    out.push_str(&op);
                    rest = after;
                    continue;
                }
                return Err(refuse(
                    "parameterised feature name",
                    at,
                    format!("`{f}('{arg}')` names a different feature for each parameter value; a v3 feature name is fixed — use the operator with a parameter port instead, e.g. `ema(close, $fast)`"),
                ));
            }
            match f {
                "feature" => match outputs.get(&arg) {
                    Some(field) => out.push_str(field),
                    None => out.push_str(&format!("feature(\"{arg}\")")),
                },
                "bar" => out.push_str(&arg),
                _ => {
                    out.push('$');
                    out.push_str(&arg);
                }
            }
            rest = after;
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    super::parse::parse_expr(&out)
        .map(|v| numeric_truth(v, None))
        .map_err(|e| {
            refuse(
                "expression",
                at,
                format!("`{expr}` does not read as an expression: {}", e.message),
            )
        })
}

/// v1 evaluates a comparison to 1.0 or 0.0, so stored strategies multiply
/// conditions — `(a > b) * (c > d) > 0` means "both". v3 does not let a truth
/// value stand for a number. Wherever a truth-valued expression sits in a port
/// that wants a number, it becomes `if(x, 1, 0)`: exactly v1's meaning, written
/// out. `want` is the type the enclosing port asks for.
fn numeric_truth(v: Value, want: Option<Ty>) -> Value {
    let Value::Object(mut node) = v else {
        return v;
    };
    let Some(name) = node.get("op").and_then(Value::as_str).map(str::to_owned) else {
        return Value::Object(node);
    };
    let spec = ops::op(&name);
    if let Some(Value::Object(args)) = node.remove("args") {
        let args = args
            .into_iter()
            .map(|(port, a)| {
                let ty = spec
                    .and_then(|s| s.inputs.iter().find(|p| p.name == port))
                    .map(|p| p.ty);
                (port, numeric_truth(a, ty))
            })
            .collect();
        node.insert("args".into(), Value::Object(args));
    }
    let node = Value::Object(node);
    if want == Some(Ty::Num) && spec.is_some_and(|s| s.output == Ty::Bool) {
        return json!({"op": "if", "args": {"cond": node, "then": 1.0, "else": 0.0}});
    }
    node
}

/// `feature('ema_{{p}}')` → `ema(close, $p)`, for the families whose v3
/// operator takes the window as a parameter port. `None` for anything else.
fn parameterised_family(f: &str, arg: &str) -> Option<String> {
    if f != "feature" {
        return None;
    }
    let (family, rest) = arg.split_once('_')?;
    let param = rest.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    if param.is_empty() || !param.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let op = match family {
        "ema" => "ema",
        "sma" => "rolling_mean",
        _ => return None,
    };
    Some(format!("{op}(close, ${param})"))
}

fn decimal(d: rust_decimal::Decimal) -> f64 {
    d.to_f64().unwrap_or(f64::NAN)
}

/// A v1 exit distance as a v3 distance.
fn distance(d: &ExitDistance) -> Value {
    let m = decimal(d.magnitude);
    match d.unit {
        // `0.02` of entry is 2%.
        DistanceUnit::FractionOfEntry => json!({"amount": m * 100.0, "unit": {"unit": "pct"}}),
        DistanceUnit::Atr { period } => json!({"amount": m, "unit": {"unit": "atr", "n": period}}),
        // A multiple of the standard deviation of close — a price distance.
        DistanceUnit::Sigma { period } => json!({
            "amount": {"op": "mul", "args": {"a": m, "b": {"op": "feature", "args": {"name": {"str": format!("rolling_std_{period}")}}}}},
            "unit": {"unit": "abs"}
        }),
    }
}

fn model_ref(model: &str, alias: &str) -> Value {
    json!({"str": format!("{model}@{alias}")})
}

/// Translate one v1 definition. `timeframe` is the decision timeframe it ran
/// at — v1 kept it outside the definition.
///
/// # Errors
/// The first construct with no translation, named.
#[allow(clippy::too_many_lines)]
pub fn translate(def: &StrategyDefinition, timeframe: &str) -> Result<Translation, Refusal> {
    let mut changes = vec![
        BehaviourChange {
            code: "fills_next_bar_open",
            detail: "orders fill at the next bar's open, not at the close of the bar that decided".into(),
        },
        BehaviourChange {
            code: "risk_gate_applied",
            detail: "every order passes the production risk gate, in backtests too".into(),
        },
        BehaviourChange {
            code: "missing_holds",
            detail: "a value that cannot be computed (an indicator warming up, a model that does not answer) holds the rule, with the reason journaled, rather than comparing false".into(),
        },
    ];
    for i in &def.inputs {
        if i.instrument != "$bound_at_init" {
            return Err(refuse(
                "inputs",
                format!("inputs[lane={}]", i.lane),
                format!(
                    "it reads `{}`, another instrument; declare it as a v3 `input` by hand",
                    i.instrument
                ),
            ));
        }
    }
    if def.risk_overrides.max_position.is_some() {
        return Err(refuse(
            "risk_overrides.max_position",
            "risk_overrides",
            "a limit in units has no v3 equivalent (v3 limits are a fraction of equity or a notional), and dropping it would loosen risk",
        ));
    }
    if def.risk_overrides.max_order_rate_per_second.is_some() {
        return Err(refuse(
            "risk_overrides.max_order_rate_per_second",
            "risk_overrides",
            "v3 limits orders per minute; dropping a per-second limit would loosen risk",
        ));
    }

    let mut nodes: Vec<Value> = Vec::new();
    // `as` name → `node.field`, for expressions that read a model output as a
    // feature.
    let mut outputs: BTreeMap<String, String> = BTreeMap::new();
    // emit → the condition nodes that emit it.
    let mut signals: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut models = false;
    for n in &def.nodes {
        let at = format!("nodes[{}]", n.id);
        match &n.kind {
            NodeKind::Condition { expr } => {
                let v = expression(expr, &outputs, &at)?;
                let Value::Object(mut o) = v else {
                    return Err(refuse("condition", &at, format!("`{expr}` is a bare value, not a comparison")));
                };
                o.insert("id".into(), json!(n.id));
                nodes.push(Value::Object(o));
            }
            NodeKind::Signal { when, emit } => {
                signals.entry(emit.clone()).or_default().push(when.clone());
            }
            NodeKind::ModelForecast { model_ref: m, alias, direction, min_confidence, .. } => {
                models = true;
                let model = format!("{}__model", n.id);
                nodes.push(json!({"id": model, "op": "model", "args": {"ref": model_ref(m, alias)}}));
                nodes.push(json!({"id": n.id, "op": "and", "args": {
                    "a": {"op": "is", "args": {"x": format!("{model}.direction"), "label": {"str": direction}}},
                    "b": {"op": "ge", "args": {"a": format!("{model}.confidence"), "b": min_confidence}}
                }}));
            }
            NodeKind::Inference { model_ref: m, alias, outputs: binds, condition, .. } => {
                models = true;
                let model = if condition.is_some() { format!("{}__model", n.id) } else { n.id.clone() };
                nodes.push(json!({"id": model, "op": "model", "args": {"ref": model_ref(m, alias)}}));
                for b in binds {
                    outputs.insert(b.as_.clone(), format!("{model}.{}", b.field));
                }
                if let Some(c) = condition {
                    nodes.push(json!({"id": n.id, "op": "and", "args": {
                        "a": {"op": "is", "args": {"x": format!("{model}.direction"), "label": {"str": c.direction}}},
                        "b": {"op": "ge", "args": {"a": format!("{model}.confidence"), "b": c.min_confidence}}
                    }}));
                }
            }
            NodeKind::Sizing { model_ref: m, alias, clamp, fallback, .. } => {
                models = true;
                let model = format!("{}__model", n.id);
                nodes.push(json!({"id": model, "op": "model", "args": {"ref": model_ref(m, alias)}}));
                let mut x = json!(format!("{model}.size_fraction"));
                if let Some([lo, hi]) = clamp {
                    let (lo, hi) = (lo.parse::<f64>(), hi.parse::<f64>());
                    let (Ok(lo), Ok(hi)) = (lo, hi) else {
                        return Err(refuse("sizing.clamp", &at, "the clamp bounds are not numbers"));
                    };
                    x = json!({"op": "clip", "args": {"x": x, "lo": lo, "hi": hi}});
                }
                if let Some(f) = fallback {
                    let Ok(f) = f.parse::<f64>() else {
                        return Err(refuse("sizing.fallback", &at, "the fallback is not a number"));
                    };
                    x = json!({"op": "coalesce", "args": {"x": x, "default": f}});
                }
                let Value::Object(mut o) = (if x.is_object() { x } else { json!({"op": "add", "args": {"a": x, "b": 0}}) }) else {
                    unreachable!("built as an object");
                };
                o.insert("id".into(), json!(n.id));
                nodes.push(Value::Object(o));
            }
            NodeKind::Combine { inputs, op, as_ } => {
                let xs: Vec<Value> = inputs
                    .iter()
                    .map(|i| json!(outputs.get(i).cloned().unwrap_or_else(|| i.clone())))
                    .collect();
                let body = match op {
                    CombineOp::WeightedAverage { weights } => json!({"op": "weighted_mean", "args": {"xs": xs, "weights": weights}}),
                    CombineOp::Max | CombineOp::Min | CombineOp::Sum => {
                        let name = match op {
                            CombineOp::Max => "max",
                            CombineOp::Min => "min",
                            _ => "add",
                        };
                        let mut it = xs.into_iter();
                        let first = it.next().ok_or_else(|| refuse("combine", &at, "it combines nothing"))?;
                        it.fold(first, |acc, x| json!({"op": name, "args": {"a": acc, "b": x}}))
                    }
                };
                let Value::Object(mut o) = body else {
                    return Err(refuse("combine", &at, "a combine of one input is that input; remove the node"));
                };
                o.insert("id".into(), json!(n.id));
                nodes.push(Value::Object(o));
                outputs.insert(as_.clone(), n.id.clone());
            }
            NodeKind::Decision { .. } => return Err(refuse("decision", &at, "a decision model maps classes to signals; translate it to `is(model.action, …)` conditions by hand")),
            NodeKind::LlmInference { .. } => return Err(refuse("llm_inference", &at, "its prompt and parse mode must become an LLM model version's contract first")),
            NodeKind::ModelRank { .. } => return Err(refuse("model_rank", &at, "ranking belongs in a selection strategy")),
            NodeKind::DataSource { .. } | NodeKind::Rank { .. } | NodeKind::Filter { .. } | NodeKind::TakeTopN { .. } | NodeKind::SurfaceAction { .. } => {
                return Err(refuse("universe pipeline", &at, "a v1.5 universe pipeline becomes a selection strategy; write it as one"));
            }
        }
    }
    if models {
        changes.push(BehaviourChange {
            code: "models_called",
            detail: "model nodes are called — point-in-time in backtests — where the v1 evaluator dropped them; their inputs are the model's contract, not the v1 `input` block".into(),
        });
    }

    let condition_of = |emit: &str, at: &str| -> Result<Value, Refusal> {
        let ids = signals
            .get(emit)
            .ok_or_else(|| refuse("signal", at, format!("no signal emits `{emit}`")))?;
        let mut it = ids.iter().map(|i| json!(i));
        let first = it
            .next()
            .ok_or_else(|| refuse("signal", at, format!("no signal emits `{emit}`")))?;
        Ok(it.fold(
            first,
            |acc, x| json!({"op": "or", "args": {"a": acc, "b": x}}),
        ))
    };
    let in_position = json!({"op": "not", "args": {"x": "state.flat"}});

    let mut rules = Vec::new();
    for (i, a) in def.actions.iter().enumerate() {
        let at = format!("actions[{i}]");
        let ActionKind::PlaceOrder { order } = &a.kind;
        let size = match &order.size_mode {
            SizeMode::Fixed => json!({"mode": "units", "x": order.size.parse::<f64>().map_err(|_| refuse("order.size", &at, "not a number"))?}),
            SizeMode::PercentOfBalance => json!({"mode": "equity_fraction", "x": order.size.parse::<f64>().map_err(|_| refuse("order.size", &at, "not a number"))?}),
            SizeMode::Model { node_ref } => json!({"mode": "equity_fraction", "x": node_ref}),
            SizeMode::RiskUnit => return Err(refuse("size_mode risk_unit", &at, "an R-multiple needs the stop it is measured against; write `risk(fraction: …, stop: …)`")),
        };
        // v1 places an order on a signal's rising edge, not on every bar it
        // holds (`backtest::sim`, pinned by `ema_cross_over_rising_bars_places_one_order`).
        // Found by the stored-strategy re-run (U-8.5): a level-triggered
        // translation placed 10 079 orders where v1 placed 333.
        let signal = condition_of(&a.on_signal, &at)?;
        let rising = json!({"op": "and", "args": {
            "a": signal.clone(),
            "b": {"op": "not", "args": {"x": {"op": "prev", "args": {"x": signal, "n": 1}}}}
        }});
        rules.push(json!({
            "when": rising,
            "then": {"kind": "order", "side": order.side, "size": size}
        }));
    }
    if def.actions.is_empty() {
        changes.push(BehaviourChange {
            code: "no_orders",
            detail: "the v1 definition places no orders; the translation has no entry rule".into(),
        });
    }
    match &def.exits {
        None => {}
        Some(ExitPolicy::RiskLayerOnly) => changes.push(BehaviourChange {
            code: "risk_layer_only",
            detail: "the v1 definition delegated its exits to the risk layer; the translation has no exit rule either".into(),
        }),
        Some(ExitPolicy::Rules { rules: exits }) => {
            let mut protect = Map::new();
            for (i, r) in exits.iter().enumerate() {
                let at = format!("exits.rules[{i}]");
                match r {
                    ExitRule::StopLoss { distance: d } => {
                        protect.insert("stop".into(), distance(d));
                    }
                    ExitRule::TakeProfit { distance: d } => {
                        protect.insert("take_profit".into(), distance(d));
                    }
                    ExitRule::TrailingStop { distance: d } => {
                        protect.insert("trailing".into(), distance(d));
                    }
                    ExitRule::MaxHoldingBars { bars } => rules.push(json!({
                        "group": "exits",
                        "when": {"op": "ge", "args": {"a": "state.bars_in_position", "b": bars}},
                        "then": {"kind": "close"}
                    })),
                    ExitRule::OnSignal { emit } => rules.push(json!({
                        "group": "exits",
                        "when": {"op": "and", "args": {"a": condition_of(emit, &at)?, "b": in_position}},
                        "then": {"kind": "close"}
                    })),
                }
            }
            if !protect.is_empty() {
                protect.insert("kind".into(), json!("protect"));
                rules.push(json!({"group": "protect", "when": in_position, "then": Value::Object(protect)}));
                changes.push(BehaviourChange {
                    code: "exits_are_orders",
                    detail: "stops, targets and trailing stops are placed as venue orders when a position opens and fill inside the bar, where v1 checked them at each bar's close".into(),
                });
            }
        }
    }

    let mut g = Map::new();
    g.insert("format".into(), json!(domain::strategy_graph::FORMAT));
    g.insert("kind".into(), json!("trading"));
    g.insert("name".into(), json!(def.strategy_id));
    g.insert(
        "description".into(),
        json!(format!("translated from v{}", def.definition_version)),
    );
    g.insert("asset_class".into(), json!(def.asset_class));
    g.insert("timeframe".into(), json!(timeframe));
    if !def.parameters.is_empty() {
        g.insert(
            "params".into(),
            serde_json::to_value(&def.parameters).unwrap_or_default(),
        );
    }
    if !def.constraints.is_empty() {
        g.insert("constraints".into(), json!(def.constraints));
    }
    g.insert("nodes".into(), Value::Array(nodes));
    g.insert(
        "decide".into(),
        json!({"rules": rules, "otherwise": "hold"}),
    );
    if let Some(n) = def.risk_overrides.max_order_rate_per_minute {
        g.insert("risk".into(), json!({"max_orders_per_min": n}));
    }
    let graph = StrategyGraph::from_json(&Value::Object(g))
        .map_err(|e| refuse("definition", "graph", e.to_string()))?;
    Ok(Translation {
        text: super::print::print(&graph),
        graph,
        changes,
    })
}
