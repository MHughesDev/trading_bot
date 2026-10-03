//! The canonical graph → SLv3 text (Set U U-7.1).
//!
//! One layout, so a graph prints the same way every time: the header, then
//! declarations (params, constraints, inputs, universe), then every node in
//! order, then the sink, then triggers and risk when they differ from the
//! defaults. `parse(print(g)) == g` for every graph `domain` accepts.

use domain::strategy_graph::ops;
use domain::strategy_graph::StrategyGraph;
use serde_json::{Map, Value};

use super::parse::{tag_of, INFIX, KEYWORDS};

/// Precedence, loosest first.
const OR: u8 = 1;
const AND: u8 = 2;
const NOT: u8 = 3;
const CMP: u8 = 4;
const ADD: u8 = 5;
const MUL: u8 = 6;
const NEG: u8 = 7;
const ATOM: u8 = 8;

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|h| h.is_ascii_alphabetic() || h == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// A name the parser would read back as itself (not a keyword, not a literal).
fn bare_ok(s: &str) -> bool {
    is_ident(s) && !KEYWORDS.contains(&s) && !matches!(s, "not" | "true" | "false")
}

/// A reference the parser reads back as the same string.
fn ref_ok(s: &str) -> bool {
    if let Some(p) = s.strip_prefix('$') {
        return is_ident(p);
    }
    match s.split_once('.') {
        Some((a, b)) => bare_ok(a) && is_ident(b),
        None => bare_ok(s),
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn name_tok(s: &str) -> String {
    if bare_ok(s) {
        s.to_string()
    } else {
        quote(s)
    }
}

fn num(n: &serde_json::Number) -> String {
    n.to_string()
}

fn infix_word(op: &str) -> Option<&'static str> {
    INFIX.iter().find(|(_, o)| *o == op).map(|(w, _)| *w)
}

fn level(op: &str) -> u8 {
    match op {
        "or" => OR,
        "and" => AND,
        "lt" | "gt" | "le" | "ge" | "eq" | "ne" => CMP,
        "add" | "sub" => ADD,
        "mul" | "div" => MUL,
        _ => ATOM,
    }
}

/// An inline node or a node's own `{op, args}`.
fn as_node(v: &Value) -> Option<(&str, Option<&Map<String, Value>>)> {
    let o = v.as_object()?;
    let op = o.get("op")?.as_str()?;
    let extra = o.keys().any(|k| k != "op" && k != "args" && k != "id");
    if extra {
        return None;
    }
    Some((op, o.get("args").and_then(Value::as_object)))
}

/// A node value at precedence `min`: parenthesised when looser.
fn node_at(v: &Value, min: u8) -> String {
    let (s, prec) = node_prec(v);
    if prec < min {
        format!("({s})")
    } else {
        s
    }
}

fn node_prec(v: &Value) -> (String, u8) {
    match v {
        Value::String(s) => (s.clone(), ATOM),
        Value::Number(n) => (num(n), ATOM),
        Value::Bool(b) => (b.to_string(), ATOM),
        Value::Array(xs) => (
            format!(
                "[{}]",
                xs.iter()
                    .map(|x| node_at(x, OR))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ATOM,
        ),
        Value::Object(o) if o.len() == 1 && o.get("str").is_some_and(Value::is_string) => {
            (quote(o["str"].as_str().unwrap_or_default()), ATOM)
        }
        Value::Null => ("null".into(), ATOM),
        Value::Object(_) => match as_node(v) {
            Some((op, args)) => node_op(op, args),
            None => (data(v, None), ATOM),
        },
    }
}

fn node_op(op: &str, args: Option<&Map<String, Value>>) -> (String, u8) {
    let empty = Map::new();
    let a = args.unwrap_or(&empty);
    let keys: Vec<&str> = a.keys().map(String::as_str).collect();
    let binary = keys == ["a", "b"];
    let unary = keys == ["x"];
    match op {
        "or" | "and" if binary => {
            let l = level(op);
            let word = op;
            (
                format!("{} {word} {}", node_at(&a["a"], l), node_at(&a["b"], l + 1)),
                l,
            )
        }
        "not" if unary => (format!("not {}", node_at(&a["x"], NOT)), NOT),
        "neg" if unary && !a["x"].is_number() => (format!("-{}", node_at(&a["x"], NEG)), NEG),
        _ if binary && infix_word(op).is_some() => {
            let l = level(op);
            let w = infix_word(op).unwrap_or("?");
            // Comparisons do not chain: both sides must bind tighter.
            let left_min = if l == CMP { CMP + 1 } else { l };
            (
                format!(
                    "{} {w} {}",
                    node_at(&a["a"], left_min),
                    node_at(&a["b"], l + 1)
                ),
                l,
            )
        }
        _ => (call(op, a), ATOM),
    }
}

/// `op(positional…, port: value…)`: the declared ports present from the first,
/// positionally; everything else by name.
fn call(op: &str, args: &Map<String, Value>) -> String {
    let mut parts = Vec::new();
    let mut used = Vec::new();
    if let Some(spec) = ops::op(op) {
        for p in spec.inputs {
            match args.get(p.name) {
                Some(v) => {
                    parts.push(node_at(v, OR));
                    used.push(p.name);
                }
                None => break,
            }
        }
    }
    for (k, v) in args {
        if !used.contains(&k.as_str()) {
            parts.push(format!("{k}: {}", node_at(v, OR)));
        }
    }
    format!("{op}({})", parts.join(", "))
}

/// A data value; `tag` is the tag key of the field it sits in.
fn data(v: &Value, tag: Option<&str>) -> String {
    match v {
        Value::Object(o) => {
            if let Some(k) = tag {
                if let Some(Value::String(t)) = o.get(k) {
                    let rest: Vec<(&String, &Value)> =
                        o.iter().filter(|(f, _)| f.as_str() != k).collect();
                    if rest.is_empty() && is_ident(t) {
                        return t.clone();
                    }
                    let fields: Vec<String> = rest
                        .iter()
                        .map(|(f, x)| format!("{f}: {}", data(x, tag_of(f))))
                        .collect();
                    return format!("{t}({})", fields.join(", "));
                }
            }
            if as_node(v).is_some() {
                return node_at(v, OR);
            }
            let fields: Vec<String> = o
                .iter()
                .map(|(f, x)| format!("{f}: {}", data(x, tag_of(f))))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Value::String(s) => {
            if ref_ok(s) {
                s.clone()
            } else {
                quote(s)
            }
        }
        Value::Number(n) => num(n),
        Value::Bool(b) => b.to_string(),
        Value::Array(xs) => format!(
            "[{}]",
            xs.iter()
                .map(|x| data(x, None))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Null => "null".into(),
    }
}

/// A node value as SLv3 (a reference, literal, list or expression).
pub(crate) fn node_value(v: &Value) -> String {
    node_at(v, OR)
}

/// A node's own expression, `ema(close, $fast)` or `a < b`.
pub(crate) fn node_expr(op: &str, args: Option<&Map<String, Value>>) -> String {
    node_op(op, args).0
}

/// A data value as SLv3; `tag` is the tag key of the field it sits in.
pub(crate) fn data_value(v: &Value, tag: Option<&str>) -> String {
    data(v, tag)
}

/// The graph as SLv3 text.
///
/// # Panics
/// Never for a graph `domain` built: its wire form always serialises.
#[must_use]
pub fn print(g: &StrategyGraph) -> String {
    let v = serde_json::to_value(g).expect("a graph serialises");
    let o = v.as_object().expect("a graph is an object");
    let s = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut out = Vec::new();
    out.push(format!(
        "strategy {} kind {} asset_class {} timeframe {}",
        name_tok(&s("name")),
        s("kind"),
        name_tok(&s("asset_class")),
        s("timeframe")
    ));
    if let Some(d) = o.get("description").and_then(Value::as_str) {
        out.push(format!("description {}", quote(d)));
    }
    if let Some(Value::Object(params)) = o.get("params") {
        for (k, spec) in params {
            out.push(format!("param {k}: {}", data(spec, Some("type"))));
        }
    }
    if let Some(Value::Array(cs)) = o.get("constraints") {
        for c in cs {
            out.push(format!(
                "constraint {}",
                quote(c.as_str().unwrap_or_default())
            ));
        }
    }
    if let Some(Value::Array(inputs)) = o.get("inputs") {
        for i in inputs {
            out.push(format!(
                "input {}: {} {}",
                i["id"].as_str().unwrap_or_default(),
                data(&i["instrument"], Some("kind")),
                i["timeframe"].as_str().unwrap_or_default()
            ));
        }
    }
    if let Some(u) = o.get("universe") {
        out.push(format!("universe {}", data(u, Some("kind"))));
    }
    out.push(String::new());
    if let Some(Value::Array(nodes)) = o.get("nodes") {
        for n in nodes {
            let id = n["id"].as_str().unwrap_or_default();
            let (op, args) = as_node(n).unwrap_or(("?", None));
            out.push(format!("let {id} = {}", node_op(op, args).0));
        }
    }
    if let Some(d) = o.get("decide") {
        out.push(String::new());
        for r in d["rules"].as_array().into_iter().flatten() {
            let mut head = String::new();
            let name = r.get("name").and_then(Value::as_str);
            let group = r.get("group").and_then(Value::as_str);
            if name.is_some() || group.is_some() {
                head.push_str("rule ");
                if let Some(n) = name {
                    // A name that reads as `group` or `when` must be quoted.
                    if bare_ok(n) && n != "group" && n != "when" {
                        head.push_str(n);
                    } else {
                        head.push_str(&quote(n));
                    }
                    head.push(' ');
                }
                if let Some(g) = group {
                    head.push_str(&format!("group {g} "));
                }
            }
            out.push(format!(
                "{head}when {} -> {}",
                node_at(&r["when"], OR),
                data(&r["then"], Some("kind"))
            ));
        }
        out.push(format!(
            "otherwise {}",
            d["otherwise"].as_str().unwrap_or("hold")
        ));
    }
    if let Some(sel) = o.get("select") {
        out.push(String::new());
        let mut line = format!("select {}", node_at(&sel["score"], OR));
        if let Some(w) = sel.get("where") {
            line.push_str(&format!(" where {}", node_at(w, OR)));
        }
        if let Some(n) = sel.get("top_n").and_then(Value::as_u64) {
            line.push_str(&format!(" top {n}"));
        }
        if let Some(ord) = sel.get("order").and_then(Value::as_str) {
            if ord != "desc" {
                line.push_str(&format!(" order {ord}"));
            }
        }
        if let Some(Value::Object(attrs)) = sel.get("attributes") {
            for (k, x) in attrs {
                line.push_str(&format!(" attribute {k} = {}", node_at(x, OR)));
            }
        }
        out.push(line);
    }
    let default_triggers = serde_json::json!({"bar_close": true, "on_fill": false});
    if let Some(t) = o.get("triggers") {
        if *t != default_triggers {
            out.push(String::new());
            out.push(format!("triggers {}", data(t, None)));
        }
    }
    if let Some(Value::Object(r)) = o.get("risk") {
        if !r.is_empty() {
            out.push(format!("risk {}", data(&Value::Object(r.clone()), None)));
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}
