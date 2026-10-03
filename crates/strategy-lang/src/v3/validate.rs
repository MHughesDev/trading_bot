//! The one validator (Set U U-7.2; ADR-0045 decision 2).
//!
//! A strategy arrives as SLv3 text, as raw graph JSON, or as a graph the
//! builder assembled — and whichever door it came through, the same mistake
//! gets the same finding: the same code, the same place, the same fix. That is
//! the whole point of having one validator, and the reason it is the compiler's
//! diagnostics (`strategy-engine`, the only typechecker) plus the few rules that
//! are about a strategy rather than about a program, plus one teaching table.
//!
//! The order is the order a person would want to hear it in: the text's syntax,
//! then the graph's shape, then the program's meaning. A later stage is not run
//! once an earlier one refused, because its findings would be about a strategy
//! that does not exist.

use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::graph::{GraphParseError, Sink};
use domain::strategy_graph::{Arg, StrategyGraph};
use serde::Serialize;
use strategy_engine::{Plan, Registries};

/// One thing wrong, and what to do about it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Stable, machine-readable.
    pub code: String,
    /// Where: `line 3, column 9` for text; a node id, `rule[2]`, `select.score`
    /// or `params` for a graph.
    pub at: String,
    pub message: String,
    /// The edit that resolves it, in the author's terms.
    pub fix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// The guide section with the rule behind it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub see: Option<&'static str>,
}

/// How a strategy arrived.
// A graph is passed by value once, into `validate`; boxing it would cost every
// caller an allocation to save a copy nobody makes.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Source<'a> {
    Text(&'a str),
    Json(&'a serde_json::Value),
    Graph(StrategyGraph),
}

/// A strategy that validated: its graph, revision, canonical text and plan.
#[derive(Debug)]
pub struct Valid {
    pub graph: StrategyGraph,
    pub revision: String,
    pub text: String,
    pub plan: Plan,
}

/// Every code this validator emits that is not the compiler's, and the guide
/// section behind every code.
pub const TEACH: &[(&str, &str)] = &[
    ("syntax", "slv3#syntax"),
    ("malformed", "slv3#structure"),
    ("wrong_format", "slv3#structure"),
    ("sink_mismatch", "slv3#structure"),
    ("selection_without_universe", "slv3#selection"),
    ("duplicate_node_id", "slv3#nodes"),
    ("bad_node_id", "slv3#nodes"),
    ("bad_param_name", "slv3#params"),
    ("bad_ref", "slv3#values"),
    ("unknown_timeframe", "slv3#structure"),
    ("empty_name", "slv3#structure"),
    ("distance_without_unit", "slv3#intents"),
    ("distance_not_positive", "slv3#intents"),
    ("close_fraction_out_of_range", "slv3#intents"),
    ("atr_period_out_of_range", "slv3#intents"),
    ("no_rules", "slv3#rules"),
    ("rule_shadowed", "slv3#rules"),
    ("never_fires", "slv3#rules"),
    ("warmup_exceeds_window", "slv3#warmup"),
    ("params", "slv3#params"),
    ("unknown_feature", "slv3#features"),
    ("unknown_op", "slv3#operators"),
    ("unknown_ref", "slv3#values"),
    ("type_mismatch", "slv3#operators"),
    ("unknown_model", "slv3#models"),
    ("model_without_contract", "slv3#models"),
];

fn see(code: &str) -> Option<&'static str> {
    TEACH.iter().find(|(c, _)| *c == code).map(|(_, s)| *s)
}

fn finding(
    code: &str,
    at: impl Into<String>,
    message: impl Into<String>,
    fix: impl Into<String>,
) -> Finding {
    Finding {
        code: code.into(),
        at: at.into(),
        message: message.into(),
        fix: fix.into(),
        example: None,
        see: see(code),
    }
}

/// Every code the retired `strategy-validator` emitted, and what answers the
/// same mistake now (ADR-0045: "every error code `strategy-validator` emits
/// today has an equivalent here with the same teachable fix").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Equivalent {
    /// This code, emitted by this validator.
    Code(&'static str),
    /// The mistake cannot be written in a `tbot.strategy/3` graph, and why.
    Unrepresentable(&'static str),
}

pub const LEGACY: &[(&str, Equivalent)] = &[
    ("unknown_feature", Equivalent::Code("unknown_feature")),
    ("distance_without_unit", Equivalent::Code("distance_without_unit")),
    ("when_names_expression", Equivalent::Unrepresentable(
        "a rule's `when` is any condition — a node, `node.field` or an expression — so there is no expression it cannot name")),
    ("loosening", Equivalent::Unrepresentable(
        "`risk` holds only tighten-only limits: the gate applies the stricter of a strategy's and the platform's, so no value widens anything")),
    ("warmup_exceeds_window", Equivalent::Code("warmup_exceeds_window")),
    ("never_fires", Equivalent::Code("never_fires")),
    ("signal_has_no_action", Equivalent::Unrepresentable(
        "there are no signals: a rule's condition and its intent are one statement, `when … -> …`")),
    ("unknown_definition_version", Equivalent::Code("wrong_format")),
    ("empty_strategy_id", Equivalent::Code("empty_name")),
    ("duplicate_node_id", Equivalent::Code("duplicate_node_id")),
    ("unknown_signal", Equivalent::Code("unknown_ref")),
    ("exits_required", Equivalent::Code("no_rules")),
    ("exits_empty_rules", Equivalent::Code("no_rules")),
    ("exit_fraction_out_of_range", Equivalent::Code("close_fraction_out_of_range")),
    ("exit_distance_not_positive", Equivalent::Code("distance_not_positive")),
    ("exit_unit_period_out_of_range", Equivalent::Code("atr_period_out_of_range")),
    ("exit_unit_version_too_low", Equivalent::Code("wrong_format")),
    ("exit_holding_bars_zero", Equivalent::Unrepresentable(
        "there is no holding-bars exit: a time exit is a rule, `when state.bars_in_position >= 48 -> close`, and any count is a valid comparison")),
    ("exits_version_too_low", Equivalent::Code("wrong_format")),
    ("exit_rule_shadowed", Equivalent::Code("rule_shadowed")),
    ("exit_signal_name_empty", Equivalent::Unrepresentable(
        "rules name no signal; a rule's condition is written in the rule")),
    ("parameter_out_of_range", Equivalent::Code("params")),
    ("constraint_violated", Equivalent::Code("params")),
    ("bad_expression", Equivalent::Code("syntax")),
];

/// The graph a source describes, or why there is none.
fn to_graph(src: Source<'_>) -> Result<StrategyGraph, Vec<Finding>> {
    match src {
        Source::Graph(g) => Ok(g),
        // Text that is SLv3 goes through the same structural check as JSON, so
        // a duplicate node is the same finding whichever way it was written.
        Source::Text(t) => {
            let v = super::parse::parse_to_json(t).map_err(|e| {
                vec![finding(
                    "syntax",
                    format!("line {}, column {}", e.line, e.col),
                    e.message,
                    e.fix,
                )]
            })?;
            StrategyGraph::from_json(&v).map_err(|e| vec![json_finding(&e)])
        }
        Source::Json(v) => {
            // A v1 definition is not a malformed v3 graph; it is another format,
            // and the fix is the translator, not an edit.
            let format = v.get("format").and_then(serde_json::Value::as_str);
            if format != Some(domain::strategy_graph::FORMAT) {
                let named = format.map_or_else(
                    || {
                        if v.get("definition_version").is_some() {
                            "a v1 definition".to_string()
                        } else {
                            "no `format`".to_string()
                        }
                    },
                    |f| format!("format `{f}`"),
                );
                return Err(vec![finding(
                    "wrong_format",
                    "format",
                    format!("this is {named}, not `{}`", domain::strategy_graph::FORMAT),
                    "a v1 definition is translated, not edited: run it through the v1 translator, then edit the graph",
                )]);
            }
            StrategyGraph::from_json(v).map_err(|e| vec![json_finding(&e)])
        }
    }
}

fn json_finding(e: &GraphParseError) -> Finding {
    match e {
        GraphParseError::Structural(g) => {
            let fix = match g.code() {
                "duplicate_node_id" => "node ids are unique within a strategy; rename one of the pair — references resolve by id, so a duplicate makes them ambiguous",
                "wrong_format" => "a `tbot.strategy/1.x` definition is translated, not edited: run it through the v1 translator, then edit the graph",
                "sink_mismatch" => "a trading strategy ends in `decide` (rules); a selection strategy in `select` and `universe`",
                "selection_without_universe" => "add `universe`: a list, an asset class, or a named universe",
                "bad_node_id" => "a node id is letters, digits and `_`, not starting with a digit",
                "bad_param_name" => "a parameter name is letters, digits and `_`",
                "bad_ref" => "write a reference as `node`, `node.field`, `$param` or `state.field`",
                _ => "use one of 1s 1m 5m 15m 1h 4h 1d",
            };
            finding(g.code(), "graph", g.to_string(), fix)
        }
        GraphParseError::Malformed(m) if m.contains("missing field `unit`") => finding(
            "distance_without_unit",
            "graph",
            m.clone(),
            "give the distance a unit: `{amount: 1.5, unit: atr(n: 14)}`, `unit: pct` or `unit: sigma(of: …)`. A bare number is ambiguous across instruments at different price scales",
        ),
        GraphParseError::Malformed(m) => finding(
            "malformed",
            "graph",
            m.clone(),
            "match the `tbot.strategy/3` shape; printing a working strategy as SLv3 text shows every field in place",
        ),
    }
}

/// Rules about a strategy as a strategy, beyond whether it is a valid program.
fn strategy_rules(
    g: &StrategyGraph,
    window_bars: Option<u32>,
    plan: Option<&Plan>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    if g.name.trim().is_empty() {
        out.push(finding(
            "empty_name",
            "name",
            "the strategy has no name",
            "give it a slug: lowercase, underscores and a version suffix — `ema_cross_v1`. It is what evidence cites it by",
        ));
    }
    for (name, spec) in &g.params {
        use domain::strategy_def::params::ParamSpec;
        let bad = match spec {
            ParamSpec::Int {
                default, min, max, ..
            } => (min > max || default < min || default > max)
                .then(|| format!("default {default} is outside {min}..{max}")),
            ParamSpec::Float {
                default, min, max, ..
            } => (min > max || default < min || default > max)
                .then(|| format!("default {default} is outside {min}..{max}")),
            ParamSpec::Enum { default, choices } => (!choices.contains(default))
                .then(|| format!("default `{default}` is not one of its choices")),
        };
        if let Some(why) = bad {
            out.push(finding(
                "params",
                format!("params.{name}"),
                format!("parameter `{name}`: {why}"),
                "make the default one of the values the parameter allows; the range is what a sweep is checked against, so widening it is a decision rather than a fix",
            ));
        }
    }
    if let Sink::Decide(d) = &g.sink {
        if d.rules.is_empty() {
            out.push(finding(
                "no_rules",
                "decide",
                "a trading strategy with no rules never acts",
                "add a rule — at least one entry and, unless the risk gate is the whole exit, one exit: `when … -> close`",
            ));
        }
        for (i, r) in d.rules.iter().enumerate() {
            if let Some(j) = d.rules[..i]
                .iter()
                .position(|e| e.group == r.group && e.when == r.when)
            {
                out.push(finding(
                    "rule_shadowed",
                    format!("rule[{i}]"),
                    format!("rule[{i}] has the same condition as rule[{j}] in group `{}`; the first match wins, so it can never fire", r.group),
                    "delete one of the pair, or put it in its own `group` if both should act",
                ));
            }
            let then = serde_json::to_value(&r.then).unwrap_or_default();
            if let Some(n) = atr_periods(&then)
                .into_iter()
                .find(|n| *n == 0 || *n > 1_000)
            {
                out.push(finding(
                    "atr_period_out_of_range",
                    format!("rule[{i}]"),
                    format!("`atr(n: {n})` is not a period the feature engine computes"),
                    "give the unit a lookback the feature engine computes: `atr(n: 14)`. The period selects the `atr_14` feature, so `atr(n: 0)` names a feature nothing serves",
                ));
            }
            if let Some(why) = never_fires(g, &r.when) {
                out.push(finding(
                    "never_fires",
                    format!("rule[{i}]"),
                    why,
                    "loosen the threshold, or check the sign of the comparison; a preview names which value held it",
                ));
            }
        }
    }
    if let (Some(window), Some(plan)) = (window_bars, plan) {
        if plan.warmup_ticks >= window {
            out.push(finding(
                "warmup_exceeds_window",
                "params",
                format!("the strategy needs {} bars of warm-up and the window holds {window}", plan.warmup_ticks),
                "start the window earlier, or shorten the longest lookback — `ema` over 200 bars to 100 halves the warm-up",
            ));
        }
    }
    out
}

fn atr_periods(v: &serde_json::Value) -> Vec<u64> {
    let mut out = Vec::new();
    match v {
        serde_json::Value::Object(o) => {
            if o.get("unit").and_then(serde_json::Value::as_str) == Some("atr") {
                if let Some(n) = o.get("n").and_then(serde_json::Value::as_u64) {
                    out.push(n);
                }
            }
            for x in o.values() {
                out.extend(atr_periods(x));
            }
        }
        serde_json::Value::Array(xs) => xs.iter().for_each(|x| out.extend(atr_periods(x))),
        _ => {}
    }
    out
}

/// A condition that is false on every bar by construction: a bounded
/// oscillator compared past its bound, or a value compared with itself.
fn never_fires(g: &StrategyGraph, when: &Arg) -> Option<String> {
    let resolve = |a: &Arg| -> Option<(String, std::collections::BTreeMap<String, Arg>)> {
        match a {
            Arg::Ref(r) => g.node(r).map(|n| (n.op.clone(), n.args.clone())),
            Arg::Inline(n) => Some((n.op.clone(), n.args.clone())),
            _ => None,
        }
    };
    let (op, args) = resolve(when)?;
    let (a, b) = (args.get("a")?, args.get("b")?);
    if matches!(op.as_str(), "gt" | "lt") && a == b {
        return Some(format!("`{op}` of a value with itself is never true"));
    }
    let bounded = |x: &Arg| -> bool {
        let feature = match x {
            Arg::Ref(r) => g
                .node(r)
                .filter(|n| n.op == "feature")
                .and_then(|n| n.args.get("name").cloned()),
            Arg::Inline(n) if n.op == "feature" => n.args.get("name").cloned(),
            _ => None,
        };
        matches!(feature, Some(Arg::Str(s)) if s.str.starts_with("rsi_"))
    };
    let lit = b.literal_num()?;
    if !bounded(a) {
        return None;
    }
    match op.as_str() {
        "gt" if lit >= 100.0 => Some(format!("an RSI is at most 100, so `> {lit}` is never true")),
        "ge" if lit > 100.0 => Some(format!(
            "an RSI is at most 100, so `>= {lit}` is never true"
        )),
        "lt" if lit <= 0.0 => Some(format!("an RSI is at least 0, so `< {lit}` is never true")),
        "le" if lit < 0.0 => Some(format!("an RSI is at least 0, so `<= {lit}` is never true")),
        _ => None,
    }
}

/// Validate a strategy however it arrived.
///
/// `window_bars`, when given, is the evaluation window the strategy is about
/// to run over, in bars of its timeframe.
///
/// # Errors
/// Every finding, in the order a reader should take them.
pub fn validate(
    src: Source<'_>,
    reg: &dyn Registries,
    params: &ParamValues,
    window_bars: Option<u32>,
) -> Result<Valid, Vec<Finding>> {
    let graph = to_graph(src)?;
    let compiled = strategy_engine::compile(&graph, reg, params);
    let mut findings: Vec<Finding> = match &compiled {
        Ok(_) => Vec::new(),
        Err(diags) => diags
            .iter()
            .map(|d| Finding {
                code: d.code.into(),
                at: d.at.clone(),
                message: d.message.clone(),
                fix: d.fix.clone(),
                example: d.example.clone(),
                see: see(d.code),
            })
            .collect(),
    };
    findings.extend(strategy_rules(&graph, window_bars, compiled.as_ref().ok()));
    if !findings.is_empty() {
        return Err(findings);
    }
    let plan = compiled.map_err(|_| Vec::<Finding>::new())?;
    Ok(Valid {
        revision: plan.revision.clone(),
        text: super::print::print(&graph),
        graph,
        plan,
    })
}
