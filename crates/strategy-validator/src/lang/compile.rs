//! SLv2 AST → the canonical [`StrategyDefinition`] (AGENT-011 §3, P1; ADR-0010).
//!
//! # The lowering, in one paragraph
//!
//! Each `entry` becomes a `Condition` node holding a v1 expression, a `Signal`
//! node watching it, and a `PlaceOrder` action on that signal. The boolean
//! operators the v1 grammar does not have are lowered arithmetically — `and` to
//! `*`, `or` to `+`, `not` to `1 - x` — which is exact because the bytecode's
//! comparison operators push 1.0 or 0.0 and a program is true when its result is
//! non-zero. `input` bindings are substituted at their use sites, because the v1
//! document has nowhere to put a named sub-expression; the *name* survives in the
//! comment the decompiler emits, so a strategy read back still shows what the
//! author called it.
//!
//! Exits become the typed `exits` block at `definition_version` 1.3, unchanged
//! and unextended: SLv2 says exits in the fields that already exist (ADR-P6-01).
//!
//! # Warm-up is computed here, not guessed
//!
//! `warmup_bars` is the longest declared lookback among the features the compiled
//! document reads, taken from `features::registry` — the same number
//! `backtest::requirements` derives, because it is the same table. A compile
//! result that reported a warm-up of its own would be a second answer to a
//! question that must have one (INV-13).

use std::collections::BTreeMap;

use domain::strategy_def::actions::{Action, ActionKind, OrderSpec};
use domain::strategy_def::exits::{ExitPolicy, ExitRule};
use domain::strategy_def::inputs::InputDeclaration;
use domain::strategy_def::nodes::{Node, NodeKind};
use domain::strategy_def::params::ParamSpec;
use domain::strategy_def::risk_overrides::RiskOverrides;
use domain::strategy_def::{StrategyDefinition, DEFINITION_VERSION_V1_3};
use rust_decimal::Decimal;

use super::parse::ParseError;
use super::{ArithOp, ExitKind, Expr, Program, Side};

/// What a compile produced.
#[derive(Clone, Debug)]
pub struct CompileOutput {
    pub definition: StrategyDefinition,
    /// Catalogued features the document reads, in first-seen order.
    pub features_used: Vec<String>,
    /// Bars of lead-in the longest feature needs — the engine's number.
    pub warmup_bars: u32,
    /// The most expensive cost class among the features used.
    pub cost_class: &'static str,
    /// Things that compiled but are worth saying out loud.
    pub warnings: Vec<String>,
}

/// The comment key under which a clause's `# …` lines are stored on a node.
///
/// A v1 `Node` has no comment field and the schema is frozen, so intent travels
/// in the node **id**'s companion map: `parameters` is the only open-ended block
/// on the document, and a reserved key prefix there is a place a comment can live
/// without changing the schema or the content hash's meaning for anything else.
pub const COMMENT_PARAM_PREFIX: &str = "__note_";

/// Compile a parsed program.
///
/// # Errors
/// A [`ParseError`] per clause that cannot be represented.
pub fn compile(prog: &Program) -> Result<CompileOutput, Vec<ParseError>> {
    let mut errors = Vec::new();
    let mut nodes = Vec::new();
    let mut actions = Vec::new();
    let mut warnings = Vec::new();

    let bindings: BTreeMap<&str, &Expr> = prog.inputs.iter().map(|b| (b.name.as_str(), &b.expr)).collect();
    let defaults: BTreeMap<&str, &str> = prog.params.iter().map(|p| (p.name.as_str(), p.default.as_str())).collect();

    for (i, entry) in prog.entries.iter().enumerate() {
        let expr = match lower(&entry.when, &bindings, &defaults, 0) {
            Ok(e) => e,
            Err(mut e) => {
                e.message = format!("entry[{i}]: {}", e.message);
                errors.push(e);
                continue;
            }
        };
        let signal = format!("{}_{i}", entry.side.as_str());
        nodes.push(Node { id: format!("c{i}"), kind: NodeKind::Condition { expr } });
        nodes.push(Node { id: format!("s{i}"), kind: NodeKind::Signal { when: format!("c{i}"), emit: signal.clone() } });
        actions.push(Action {
            on_signal: signal,
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: if entry.side == Side::Long { "buy".into() } else { "sell".into() },
                    size_mode: "fixed".into(),
                    size: Some(prog.size.as_ref().map_or_else(|| "0.01".to_string(), |s| s.value.clone())),
                    node_ref: None,
                },
            },
        });
        if contains_crossing(&entry.when) && matches!(entry.when, Expr::And(..)) {
            warnings.push(format!(
                "entry[{i}]: the crossing fires on the rising edge of the whole condition, so it enters on the bar the conjunction first becomes true — which is the bar the cross happens only if the other clauses already held"
            ));
        }
    }

    let exits = compile_exits(prog, &mut errors);

    let mut parameters: BTreeMap<String, ParamSpec> = BTreeMap::new();
    for p in &prog.params {
        match param_spec(p) {
            Ok(spec) => {
                parameters.insert(p.name.clone(), spec);
            }
            Err(e) => errors.push(e),
        }
    }
    store_comments(prog, &mut parameters);

    let mut risk_overrides = RiskOverrides::default();
    for r in &prog.risk {
        let Ok(v) = r.value.parse::<Decimal>() else {
            errors.push(ParseError { line: 1, code: "malformed_risk", message: format!("`{}` is not a decimal", r.value), fix: None, see: None });
            continue;
        };
        match r.name.as_str() {
            "max_position" => risk_overrides.max_position = Some(v),
            "max_orders_per_min" => risk_overrides.max_orders_per_min = v.try_into().ok(),
            "max_daily_loss" => risk_overrides.max_daily_loss = Some(v),
            _ => {}
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let mut features_used = Vec::new();
    for e in &prog.entries {
        collect_expanded(&e.when, &bindings, &mut features_used);
    }

    let mut warmup_bars = 0u32;
    let mut cost = features::registry::CostClass::Cheap;
    for name in &features_used {
        if let Some(r) = features::registry::resolve(name) {
            warmup_bars = warmup_bars.max(r.lookback_bars().unwrap_or(0));
            cost = cost.max(r.family.cost);
        }
    }

    let definition = StrategyDefinition {
        strategy_id: prog.strategy_id.clone(),
        definition_version: DEFINITION_VERSION_V1_3.to_string(),
        asset_class: prog.asset_class.clone(),
        min_trust_tier: domain::trust::TrustTier::CentralizedExchange,
        inputs: vec![
            InputDeclaration { lane: format!("market.bars.{}", prog.timeframe), instrument: "$bound_at_init".into(), features: vec![] },
            InputDeclaration { lane: "features.technical".into(), instrument: "$bound_at_init".into(), features: features_used.clone() },
        ],
        nodes,
        actions,
        risk_overrides,
        parameters,
        constraints: vec![],
        exits,
    };

    Ok(CompileOutput { definition, features_used, warmup_bars, cost_class: cost.as_str(), warnings })
}

fn param_spec(p: &super::ParamDecl) -> Result<ParamSpec, ParseError> {
    let bad = |what: &str| ParseError { line: 1, code: "malformed_param", message: format!("param `{}`: {what}", p.name), fix: None, see: None };
    let parse = |s: &str| s.parse::<Decimal>().map_err(|_| bad("not a decimal"));
    Ok(ParamSpec {
        kind: if p.ptype == "int" { "int".into() } else { "number".into() },
        default: parse(&p.default)?,
        min: Some(parse(&p.min)?),
        max: Some(parse(&p.max)?),
        step: None,
        values: vec![],
        description: None,
    })
}

/// Comments ride in reserved `parameters` keys so that intent is part of the
/// content hash: a revision that only changed *why* a threshold is 0.6 is a
/// different artifact from one that did not say, which is the point.
fn store_comments(prog: &Program, parameters: &mut BTreeMap<String, ParamSpec>) {
    let mut put = |key: String, lines: &[String]| {
        if lines.is_empty() {
            return;
        }
        parameters.insert(
            key,
            ParamSpec {
                kind: "note".into(),
                default: Decimal::ZERO,
                min: None,
                max: None,
                step: None,
                values: vec![],
                description: Some(lines.join("\n")),
            },
        );
    };
    put(format!("{COMMENT_PARAM_PREFIX}strategy"), &prog.comments);
    for (i, e) in prog.entries.iter().enumerate() {
        put(format!("{COMMENT_PARAM_PREFIX}entry_{i}"), &e.comments);
    }
    for (i, x) in prog.exits.iter().enumerate() {
        put(format!("{COMMENT_PARAM_PREFIX}exit_{i}"), &x.comments);
    }
    for b in &prog.inputs {
        put(format!("{COMMENT_PARAM_PREFIX}input_{}", b.name), &b.comments);
    }
    for p in &prog.params {
        put(format!("{COMMENT_PARAM_PREFIX}param_{}", p.name), &p.comments);
    }
    for (i, r) in prog.risk.iter().enumerate() {
        put(format!("{COMMENT_PARAM_PREFIX}risk_{i}"), &r.comments);
    }
    if let Some(s) = &prog.size {
        put(format!("{COMMENT_PARAM_PREFIX}size"), &s.comments);
    }
}

fn compile_exits(prog: &Program, errors: &mut Vec<ParseError>) -> Option<ExitPolicy> {
    if prog.exits.iter().any(|x| x.kind == ExitKind::RiskLayerOnly) {
        return Some(ExitPolicy::RiskLayerOnly);
    }
    let mut rules = Vec::new();
    for x in &prog.exits {
        let pct = |v: &str| -> Option<Decimal> { v.parse::<Decimal>().ok().map(|d| d / Decimal::from(100)) };
        let rule = match &x.kind {
            ExitKind::Stop(v) => pct(v).map(|fraction| ExitRule::StopLoss { fraction }),
            ExitKind::Target(v) => pct(v).map(|fraction| ExitRule::TakeProfit { fraction }),
            ExitKind::Trail(v) => pct(v).map(|fraction| ExitRule::TrailingStop { fraction }),
            ExitKind::Time(n) => Some(ExitRule::MaxHoldingBars { bars: *n }),
            ExitKind::OnSignal(s) => Some(ExitRule::OnSignal { emit: s.clone() }),
            ExitKind::RiskLayerOnly => None,
        };
        match rule {
            Some(r) => rules.push(r),
            None => errors.push(ParseError { line: 1, code: "malformed_exit", message: "an exit distance is not a decimal".into(), fix: None, see: None }),
        }
    }
    Some(ExitPolicy::Rules { rules })
}

fn contains_crossing(e: &Expr) -> bool {
    match e {
        Expr::CrossedAbove(..) | Expr::CrossedBelow(..) => true,
        Expr::And(a, b) | Expr::Or(a, b) => contains_crossing(a) || contains_crossing(b),
        Expr::Not(a) => contains_crossing(a),
        _ => false,
    }
}

fn collect_expanded(e: &Expr, bindings: &BTreeMap<&str, &Expr>, out: &mut Vec<String>) {
    match e {
        Expr::Input(n) => {
            if let Some(b) = bindings.get(n.as_str()) {
                collect_expanded(b, bindings, out);
            }
        }
        _ => {
            let mut own = Vec::new();
            e.feature_names(&mut own);
            for n in own {
                if !out.contains(&n) {
                    out.push(n);
                }
            }
            let mut inputs = Vec::new();
            e.input_names(&mut inputs);
            for n in inputs {
                if let Some(b) = bindings.get(n.as_str()) {
                    collect_expanded(b, bindings, out);
                }
            }
        }
    }
}

/// How deep an `input` chain may go before it is a cycle. Three is more nesting
/// than a readable strategy has; the point is to terminate, not to be generous.
const MAX_BINDING_DEPTH: usize = 8;

/// Lower one SLv2 expression to a v1 expression string.
fn lower(e: &Expr, bindings: &BTreeMap<&str, &Expr>, defaults: &BTreeMap<&str, &str>, depth: usize) -> Result<String, ParseError> {
    if depth > MAX_BINDING_DEPTH {
        return Err(ParseError { line: 1, code: "malformed_input", message: "an input refers to itself".into(), fix: Some("break the cycle".into()), see: None });
    }
    let go = |x: &Expr| lower(x, bindings, defaults, depth + 1);
    Ok(match e {
        Expr::Num(n) => n.clone(),
        Expr::Bar(f) => format!("bar('{f}')"),
        Expr::Feature { family, params } => {
            let mut name = family.clone();
            for p in params {
                name.push('_');
                name.push_str(&p.to_string());
            }
            format!("feature('{name}')")
        }
        Expr::Param(p) => (*defaults.get(p.as_str()).ok_or_else(|| ParseError {
            line: 1,
            code: "unknown_param",
            message: format!("`{p}` is not a declared parameter"),
            fix: Some(format!("declare it: `param {p} number 1 range 0 10`")),
            see: None,
        })?)
        .to_string(),
        Expr::Input(n) => {
            let b = bindings.get(n.as_str()).ok_or_else(|| ParseError {
                line: 1,
                code: "malformed_input",
                message: format!("`{n}` is not a bound input"),
                fix: Some(format!("add `input {n} = …`")),
                see: None,
            })?;
            go(b)?
        }
        Expr::Neg(a) => format!("-({})", go(a)?),
        Expr::Arith { op, lhs, rhs } => format!("({}) {} ({})", go(lhs)?, op.as_str(), go(rhs)?),
        Expr::Cmp { op, lhs, rhs } => format!("({}) {} ({})", go(lhs)?, op.as_str(), go(rhs)?),
        // `and` is a product and `or` a sum: the comparison operators push 1.0 or
        // 0.0, and a program is true when its result is non-zero. Exact, and it
        // needs no opcode the runtime does not already have.
        Expr::And(a, b) => format!("({}) * ({})", go(a)?, go(b)?),
        Expr::Or(a, b) => format!("({}) + ({})", go(a)?, go(b)?),
        Expr::Not(a) => format!("1 - ({})", go(a)?),
        // A crossing is a comparison; the crossing itself is the signal node's
        // rising edge (see the module docs on `Signal`).
        Expr::CrossedAbove(a, b) => format!("({}) > ({})", go(a)?, go(b)?),
        Expr::CrossedBelow(a, b) => format!("({}) < ({})", go(a)?, go(b)?),
        Expr::Between(x, lo, hi) => {
            let x = go(x)?;
            format!("(({x}) >= ({})) * (({x}) <= ({}))", go(lo)?, go(hi)?)
        }
        Expr::Rising(_, n) => format!("(bar('close')) > (feature('lag_{n}'))"),
        Expr::Falling(_, n) => format!("(bar('close')) < (feature('lag_{n}'))"),
    })
}

/// The lowering of an arithmetic operator, exposed so the decompiler and the
/// property test speak about the same mapping rather than two copies of it.
#[must_use]
pub fn arith_symbol(op: ArithOp) -> &'static str {
    op.as_str()
}
