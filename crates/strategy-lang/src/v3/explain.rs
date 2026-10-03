//! A strategy in plain words (Set U U-7.4's `explain`; FEAT-007 §10).
//!
//! Generated from the graph and the operator registry, never written by hand:
//! each node is its SLv3 expression and the registry's one line on what that
//! operator is for, each rule its condition and its intent. So an explanation
//! can only describe what the strategy actually does.

use domain::strategy_graph::graph::Sink;
use domain::strategy_graph::{ops, StrategyGraph, StrategyKind};
use serde_json::Value;

use super::print::{data_value, node_expr, node_value};

/// The strategy, one line per thing it does, in evaluation order.
#[must_use]
pub fn explain(g: &StrategyGraph) -> Vec<String> {
    let mut out = Vec::new();
    let what = match g.kind {
        StrategyKind::Trading => "decides what to do with one instrument",
        StrategyKind::Selection => "ranks a universe and picks from it",
    };
    let when = match (
        &g.triggers.schedule,
        g.triggers.bar_close,
        g.triggers.on_fill,
    ) {
        (Some(s), _, _) => format!("on the schedule `{s}` (UTC)"),
        (None, true, true) => format!(
            "at every {} bar close and on its own fills",
            g.timeframe.key()
        ),
        (None, true, false) => format!("at every {} bar close", g.timeframe.key()),
        (None, false, true) => "on its own fills".to_string(),
        (None, false, false) => "never — no trigger is set".to_string(),
    };
    out.push(format!("`{}` {what}, {when} ({}).", g.name, g.asset_class));
    for (name, spec) in &g.params {
        let v = serde_json::to_value(spec).unwrap_or_default();
        out.push(format!(
            "Parameter `{name}`: {}.",
            data_value(&v, Some("type"))
        ));
    }
    for c in &g.constraints {
        out.push(format!("Constraint: {c}."));
    }
    for i in &g.inputs {
        let v = serde_json::to_value(&i.instrument).unwrap_or_default();
        out.push(format!(
            "Input `{}`: {} at {}.",
            i.id,
            data_value(&v, Some("kind")),
            i.timeframe.key()
        ));
    }
    for n in &g.nodes {
        let args = serde_json::to_value(&n.args).unwrap_or_default();
        let expr = node_expr(&n.op, args.as_object());
        let why = ops::op(&n.op).map_or("not an operator the registry knows", |o| o.use_when);
        out.push(format!("`{}` = {expr} — {why}", n.id));
    }
    match &g.sink {
        Sink::Decide(d) => {
            if d.rules.is_empty() {
                out.push("It has no rules, so it never acts.".into());
            }
            for (i, r) in d.rules.iter().enumerate() {
                let when = node_value(&serde_json::to_value(&r.when).unwrap_or_default());
                let then = data_value(
                    &serde_json::to_value(&r.then).unwrap_or_default(),
                    Some("kind"),
                );
                let label = r
                    .name
                    .as_deref()
                    .map_or_else(|| format!("Rule {}", i + 1), |n| format!("Rule `{n}`"));
                let group = if r.group == "main" {
                    String::new()
                } else {
                    format!(" (group `{}`; first match wins within it)", r.group)
                };
                out.push(format!("{label}{group}: when {when}, {then}."));
            }
            out.push("Otherwise it holds: working orders stay working.".into());
        }
        Sink::Select { select, universe } => {
            let u = serde_json::to_value(universe).unwrap_or_default();
            out.push(format!("Universe: {}.", data_value(&u, Some("kind"))));
            let s = serde_json::to_value(select).unwrap_or_default();
            let mut line = format!("Ranks members by {}", node_value(&s["score"]));
            line.push_str(if s["order"] == "asc" {
                ", lowest first"
            } else {
                ", highest first"
            });
            if let Some(w) = s.get("where") {
                line.push_str(&format!(", keeping those where {}", node_value(w)));
            }
            if let Some(n) = s.get("top_n").and_then(Value::as_u64) {
                line.push_str(&format!(", and selects the top {n}"));
            }
            line.push('.');
            out.push(line);
            if let Some(Value::Object(a)) = s.get("attributes") {
                for (k, v) in a {
                    out.push(format!(
                        "Each selected member carries `{k}` = {}.",
                        node_value(v)
                    ));
                }
            }
        }
    }
    let risk = serde_json::to_value(&g.risk).unwrap_or_default();
    if risk.as_object().is_some_and(|o| !o.is_empty()) {
        out.push(format!(
            "Its own limits, tighter than the platform's: {}.",
            data_value(&risk, None)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explanation_names_every_node_rule_and_what_each_operator_is_for() {
        let g = super::super::parse(
            "strategy x kind trading asset_class crypto_spot_cex timeframe 1h\n\
             let f = ema(close, 5)\nlet s = ema(close, 20)\n\
             rule entry when crossed_above(f, s) and state.flat -> target(size: units(x: 1))\n\
             when crossed_below(f, s) -> close\n",
        )
        .unwrap();
        let lines = explain(&g);
        assert!(lines[0].contains("every 1h bar close"), "{lines:#?}");
        let ema = ops::op("ema").unwrap().use_when;
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("`f` = ema(close, 5.0)") && l.ends_with(ema)),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l
                    .starts_with("Rule `entry`: when crossed_above(f, s) and state.flat, target(")),
            "{lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l == "Rule 2: when crossed_below(f, s), close."),
            "{lines:#?}"
        );
    }
}
