//! Syntax tree → the canonical [`StrategyDefinition`].
//!
//! # The target is v1.3, not a new format
//!
//! ADR-0010 stands and AGENT-011 §9 repeats it: the canonical JSON is the
//! storage and runtime form, and SLv2 stops being what a model is *asked* to
//! produce without becoming what is *stored*. So this compiler targets
//! `domain::strategy_def::StrategyDefinition` exactly as the simulator and the
//! live runtime already read it, and it adds no field to it.
//!
//! # How a conjunction compiles, and why that took reading three evaluators
//!
//! The frozen v1 expression grammar (`domain::strategy_def::nodes`) has no
//! `and`, no `or` and no `not`: `expr = comparison`, one comparison, no boolean
//! combinators. FEAT-004 §3.3 lists `And, Or, Not` among the bytecode
//! *extensions* it would need, and they were never built. Taken at face value
//! that would make `entry long when crossed_above(…) and rsi(close,14) < 60` —
//! AGENT-011's own headline example — uncompilable, which would leave the
//! language able to say nothing beyond what JSON could already say.
//!
//! It is compilable, and the reason is a property all three evaluators share:
//!
//! - `strategy-validator::expressions::check_primary` accepts `( expr )` where
//!   `expr` is a full comparison, so a parenthesised comparison is a legal
//!   operand of `*` and `+`;
//! - `strategy-runtime::bytecode` and `strategy-runtime::interpreter` both push
//!   exactly `1.0` or `0.0` for every comparison operator;
//! - a condition node fires on `value != 0.0`.
//!
//! So booleans are already numbers in this platform, and Boolean algebra over
//! `{0, 1}` compiles into the frozen grammar untouched:
//!
//! | SLv2 | canonical |
//! |---|---|
//! | `a and b` | `(A * B)` |
//! | `a or b` | `((A + B) > 0)` |
//! | `not a` | `(1 - A)` |
//!
//! **The invariant that makes this sound is that every compiled condition
//! evaluates to exactly `0.0` or `1.0`, never to `2.0`.** `or` is therefore
//! `((A + B) > 0)` and not `(A + B)`: the shorter form is truthy at the top
//! level and wrong one level in, because `not (a or b)` with both true would be
//! `1 - 2 = -1`, which is truthy. `normalises_or_so_that_not_of_it_is_correct`
//! pins that, and it is the test to read first if this file is ever "simplified".
//!
//! # What is refused, and why refusing is the right answer
//!
//! `crossed_above`, `rising`, `flat`, `bars_in_trade`, `abs`, `clip`, a stop in
//! `atr` units, `vol_target` sizing, `cooldown`: each of these is in FEAT-004
//! §3.1 and none of them has anywhere to go in the canonical form. Each is
//! **parsed** and then refused *by name*, with the diagnostic saying which
//! mechanism is missing and where. That is deliberate. A model that writes
//! `crossed_above` and is told "unexpected token" learns that its syntax is
//! wrong; a model told "the expression grammar sees only the current bar, and
//! no feature lags a derived feature" learns the shape of the platform, and the
//! next agent to widen the platform has the list of what to widen.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    exits::{ExitPolicy, ExitRule},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    params::ParamSpec,
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use serde_json::{json, Value};

use crate::ast::{
    Anchor, BinOp, ExitKind, Expr, Num, ParamKind, RiskKind, Side, SizeKind, Strategy, UnOp, Unit,
};
use crate::catalogue::Catalogue;
use crate::diagnostic::{Code, Diagnostic, Span};

/// The definition version SLv2 writes: the one that has exits.
///
/// Not the newest version. 1.4 adds the `unit` on an exit distance, and a
/// document that does not use one is a 1.3 document — saying 1.4 anyway would
/// make every strategy unreadable to a runtime that knows 1.3 in exchange for a
/// field it does not set. [`SCALED_UNIT_VERSION`] is what a scaled distance
/// raises it to, and only then.
pub const TARGET_VERSION: &str = "1.3";
/// The version a volatility-scaled exit distance requires (ADR-D-09).
pub const SCALED_UNIT_VERSION: &str = domain::strategy_def::DEFINITION_VERSION_V1_4;
/// The asset class a strategy gets when it does not say. Matches the canonical
/// example in `domain::strategy_def`.
pub const DEFAULT_ASSET_CLASS: &str = "crypto_spot_cex";
/// The lane a definition subscribes to for bars.
pub const BARS_LANE_PREFIX: &str = "market.bars.";
/// The lane features arrive on.
pub const FEATURES_LANE: &str = "features.technical";

/// Functions SLv2 offers that need a bar other than the current one.
const NEEDS_HISTORY: &[&str] = &["crossed_above", "crossed_below", "rising", "falling"];
/// Functions SLv2 offers that need a bytecode operation that does not exist.
const NEEDS_OPERATOR: &[&str] = &["abs", "min", "max", "clip", "log", "exp", "sqrt", "sign"];
/// Names that read position state.
const POSITION_STATE: &[&str] = &[
    "flat",
    "long",
    "short",
    "position",
    "entry_price",
    "bars_in_trade",
    "unrealized_return",
    "equity",
    "last_exit_reason",
];

/// Compile against the engine's own feature registry (see [`crate::catalogue`]).
///
/// # Errors
///
/// Every reason the tree has no canonical form, in source order.
pub fn compile(ast: &Strategy) -> Result<StrategyDefinition, Vec<Diagnostic>> {
    compile_with(ast, &Catalogue::engine())
}

/// Compile against a caller-supplied feature catalogue.
///
/// # Errors
///
/// Every reason the tree has no canonical form, in source order.
pub fn compile_with(
    ast: &Strategy,
    catalogue: &Catalogue,
) -> Result<StrategyDefinition, Vec<Diagnostic>> {
    let mut c = Compiler {
        catalogue,
        params: BTreeSet::new(),
        inputs: HashMap::new(),
        used_inputs: BTreeSet::new(),
        features: BTreeSet::new(),
        scaled_unit: None,
        errors: Vec::new(),
    };
    let def = c.run(ast);
    if c.errors.is_empty() {
        Ok(def.expect("no errors implies a definition"))
    } else {
        c.errors.sort_by_key(|d| d.span.offset);
        Err(c.errors)
    }
}

struct Compiler<'a> {
    catalogue: &'a Catalogue,
    params: BTreeSet<String>,
    /// Input name → the feature name it stands for.
    inputs: HashMap<String, String>,
    used_inputs: BTreeSet<String>,
    features: BTreeSet<String>,
    /// Where the first volatility-scaled exit distance was written, if one was.
    ///
    /// A scaled distance is the one thing SLv2 can say that needs
    /// [`SCALED_UNIT_VERSION`], so the version a document gets is a
    /// *measurement* of what it contains rather than a constant. The span is
    /// kept so that a `version 1.3` header written above a scaled stop can be
    /// refused pointing at the stop.
    scaled_unit: Option<(Span, String)>,
    errors: Vec<Diagnostic>,
}

impl<'a> Compiler<'a> {
    fn err(&mut self, d: Diagnostic) {
        self.errors.push(d);
    }

    fn run(&mut self, ast: &Strategy) -> Option<StrategyDefinition> {
        self.declare(ast);

        let mut nodes: Vec<Node> = Vec::new();
        let mut actions: Vec<Action> = Vec::new();

        let size_mode_and_value = self.size(ast);

        for (i, entry) in ast.entries.iter().enumerate() {
            if let Some((_, span)) = &entry.cooldown {
                self.err(
                    Diagnostic::semantic(
                        Code::ClauseNotRepresentable,
                        *span,
                        "`cooldown` has no field on the canonical definition: nothing between \
                         the signal node and the risk gate counts bars since the last exit",
                    )
                    .with_fix(
                        "drop the cooldown, or express the same idea as a filter on a feature \
                         the engine computes",
                    )
                    .with_see("guide#entries"),
                );
            }
            let Some(expr) = self.bool_expr(&entry.when) else {
                continue;
            };
            let cond_id = format!("e{i}_cond");
            nodes.push(Node {
                id: cond_id.clone(),
                kind: NodeKind::Condition { expr },
            });
            nodes.push(Node {
                id: format!("e{i}_sig"),
                kind: NodeKind::Signal {
                    when: cond_id,
                    emit: entry.side.as_str().to_string(),
                },
            });
        }

        // One action per distinct signal: two `entry long` clauses are an `or`
        // over one position, not two orders.
        if let Some((size_mode, size)) = &size_mode_and_value {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for entry in &ast.entries {
                let emit = entry.side.as_str();
                if !seen.insert(emit) {
                    continue;
                }
                actions.push(Action {
                    on_signal: emit.to_string(),
                    kind: ActionKind::PlaceOrder {
                        order: OrderSpec {
                            side: side_of(entry.side),
                            size_mode: size_mode.clone(),
                            size: size.clone(),
                        },
                    },
                });
            }
        }

        let exits = self.exits(ast, &mut nodes);
        let risk_overrides = self.risk(ast);
        let parameters = self.params(ast);
        // A constraint's text used to be stored verbatim and never read here, so
        // `constraint blah` compiled into a stored definition and failed much
        // later inside `domain::strategy_def::params::parse_constraint` — at
        // sweep time, on a strategy somebody had already committed to. That is
        // the free-form-string class this repository keeps finding: a string
        // accepted at the door and parsed at the far end, where the error names
        // a place the author is no longer looking.
        //
        // `check_constraint` is the same reader the sweep will use, so this
        // cannot disagree with it about what a constraint is.
        let constraints: Vec<String> = ast
            .constraints
            .iter()
            .filter(
                |c| match domain::strategy_def::params::check_constraint(&c.text) {
                    Ok(()) => true,
                    Err(e) => {
                        self.err(
                            Diagnostic::semantic(
                                Code::UnknownName,
                                c.span,
                                format!("`{}` is not a constraint: {e}", c.text),
                            )
                            .with_fix(
                                "a constraint compares two parameters, for example `fast < slow`",
                            ),
                        );
                        false
                    }
                },
            )
            .map(|c| c.text.clone())
            .collect();

        self.unused_inputs(ast);

        // After `self.exits`, which is what records a scaled distance.
        let version = self.version(ast);
        let asset_class = ast
            .asset_class
            .as_ref()
            .map_or_else(|| DEFAULT_ASSET_CLASS.to_string(), |(v, _)| v.clone());
        let min_trust_tier = match &ast.trust {
            None => domain::trust::TrustTier::CentralizedExchange,
            Some((word, span)) => match serde_json::from_value(Value::String(word.clone())) {
                Ok(t) => t,
                Err(_) => {
                    self.err(
                        Diagnostic::semantic(
                            Code::UnknownName,
                            *span,
                            format!("`{word}` is not a trust tier"),
                        )
                        .with_fix(
                            "one of `social_derived`, `onchain_tentative`, \
                                 `onchain_confirmed`, `centralized_exchange` or `regulated`",
                        )
                        .with_see("guide#header"),
                    );
                    domain::trust::TrustTier::CentralizedExchange
                }
            },
        };

        if !self.errors.is_empty() {
            return None;
        }

        let mut inputs = vec![InputDeclaration {
            lane: format!("{BARS_LANE_PREFIX}{}", ast.timeframe),
            instrument: ast.instrument.clone(),
            features: Vec::new(),
        }];
        if !self.features.is_empty() {
            inputs.push(InputDeclaration {
                lane: FEATURES_LANE.to_string(),
                instrument: ast.instrument.clone(),
                features: self.features.iter().cloned().collect(),
            });
        }

        Some(StrategyDefinition {
            strategy_id: ast.name.clone(),
            definition_version: version,
            asset_class,
            min_trust_tier,
            inputs,
            nodes,
            actions,
            risk_overrides,
            parameters,
            constraints,
            exits,
        })
    }

    // ── declarations ─────────────────────────────────────────────────────────

    fn declare(&mut self, ast: &Strategy) {
        for p in &ast.params {
            if !self.params.insert(p.name.clone()) {
                self.err(
                    Diagnostic::semantic(
                        Code::DuplicateName,
                        p.span,
                        format!("`{}` is declared as a parameter twice", p.name),
                    )
                    .with_fix("give one of them a different name"),
                );
            }
        }
        for b in &ast.inputs {
            if self.params.contains(&b.name) {
                self.err(
                    Diagnostic::semantic(
                        Code::DuplicateName,
                        b.name_span,
                        format!("`{}` is both a parameter and an input", b.name),
                    )
                    .with_fix("rename the input; a name means one thing"),
                );
                continue;
            }
            if crate::ast::BarField::parse(&b.name).is_some() {
                self.err(
                    Diagnostic::semantic(
                        Code::DuplicateName,
                        b.name_span,
                        format!("`{}` is a bar field and cannot be re-bound", b.name),
                    )
                    .with_fix(
                        "choose a name that is not `open`, `high`, `low`, `close` or `volume`",
                    ),
                );
                continue;
            }
            let Some(feature) = self.feature_of(&b.expr) else {
                continue;
            };
            self.features.insert(feature.clone());
            if self.inputs.insert(b.name.clone(), feature).is_some() {
                self.err(
                    Diagnostic::semantic(
                        Code::DuplicateName,
                        b.name_span,
                        format!("`{}` is bound twice", b.name),
                    )
                    .with_fix("delete one of the two `input` clauses"),
                );
            }
        }
    }

    /// An input must name exactly one feature.
    ///
    /// The canonical form has no place to put a named intermediate: `inputs` is
    /// a list of lane subscriptions, and the only named value a condition can
    /// read is `feature('…')`. So `input rv = rolling_std(close, 20)` is a name
    /// for a feature and compiles; `input spread = high - low` is a name for an
    /// expression and does not. Saying so is better than inventing a node type
    /// the runtime would not evaluate.
    fn feature_of(&mut self, e: &Expr) -> Option<String> {
        match e {
            Expr::Call { .. } => self.feature_name(e),
            other => {
                self.err(
                    Diagnostic::semantic(
                        Code::InputNotAFeature,
                        other.span(),
                        "an `input` names one feature; this is an expression",
                    )
                    .with_fix(
                        "write the arithmetic inline in the clause that uses it, or find a \
                         feature family that computes it — `list_features(query=\"…\")`",
                    )
                    .with_see("guide#inputs"),
                );
                None
            }
        }
    }

    fn unused_inputs(&mut self, ast: &Strategy) {
        for b in &ast.inputs {
            if self.inputs.contains_key(&b.name) && !self.used_inputs.contains(&b.name) {
                self.err(
                    Diagnostic::semantic(
                        Code::UnusedInput,
                        b.span,
                        format!("nothing reads the input `{}`", b.name),
                    )
                    .with_fix("use it in an entry or an exit, or delete the clause")
                    .with_see("guide#inputs"),
                );
            }
        }
    }

    // ── expressions ──────────────────────────────────────────────────────────

    /// Compile a condition. The result evaluates to exactly `0.0` or `1.0`; see
    /// the module docs for why that is load-bearing.
    fn bool_expr(&mut self, e: &Expr) -> Option<String> {
        match e {
            Expr::Binary {
                op: BinOp::And,
                lhs,
                rhs,
                ..
            } => {
                let a = self.bool_expr(lhs);
                let b = self.bool_expr(rhs);
                Some(format!("({} * {})", a?, b?))
            }
            Expr::Binary {
                op: BinOp::Or,
                lhs,
                rhs,
                ..
            } => {
                let a = self.bool_expr(lhs);
                let b = self.bool_expr(rhs);
                Some(format!("(({} + {}) > 0)", a?, b?))
            }
            Expr::Unary {
                op: UnOp::Not, rhs, ..
            } => {
                let a = self.bool_expr(rhs)?;
                Some(format!("(1 - {a})"))
            }
            Expr::Binary { op, lhs, rhs, .. } if op.is_comparison() => {
                let a = self.num_expr(lhs);
                let b = self.num_expr(rhs);
                Some(format!("({} {} {})", a?, op.as_str(), b?))
            }
            Expr::Call {
                name, args, span, ..
            } if name == "between" => {
                if args.len() != 3 {
                    self.err(
                        Diagnostic::semantic(
                            Code::WrongArity,
                            *span,
                            format!("`between` takes 3 arguments, not {}", args.len()),
                        )
                        .with_fix("write `between(zscore(close, 20), -1, 1)`")
                        .with_see("guide#functions"),
                    );
                    return None;
                }
                let x = self.num_expr(&args[0]);
                let lo = self.num_expr(&args[1]);
                let hi = self.num_expr(&args[2]);
                let (x, lo, hi) = (x?, lo?, hi?);
                Some(format!("(({x} >= {lo}) * ({x} <= {hi}))"))
            }
            Expr::Call { name, span, .. } if NEEDS_HISTORY.contains(&name.as_str()) => {
                self.err(
                    Diagnostic::semantic(
                        Code::NeedsHistory,
                        *span,
                        format!(
                            "`{name}` compares this bar with an earlier one, and a condition \
                             expression sees only the current bar"
                        ),
                    )
                    .with_fix(
                        "compare levels instead — `macd(close, 7, 21) > 0` for what \
                         `crossed_above(ema(close,7), ema(close,21))` would have meant — or add \
                         the bytecode operation FEAT-004 §3.3 lists as an extension",
                    )
                    .with_see("guide#functions.history"),
                );
                None
            }
            Expr::Name { name, span } if POSITION_STATE.contains(&name.as_str()) => {
                self.position_state(name, *span);
                None
            }
            other => {
                self.err(
                    Diagnostic::semantic(
                        Code::TypeMismatch,
                        other.span(),
                        "this is a value; a `when` clause needs a condition",
                    )
                    .with_fix("compare it with something, as in `… > 0`")
                    .with_see("guide#grammar.expressions"),
                );
                None
            }
        }
    }

    /// Compile a value into the frozen v1 expression grammar.
    fn num_expr(&mut self, e: &Expr) -> Option<String> {
        match e {
            Expr::Number(n) => {
                if n.raw.starts_with('-') {
                    Some(format!("(-{})", &n.raw[1..]))
                } else {
                    Some(n.raw.clone())
                }
            }
            Expr::Bar { field, .. } => Some(format!("bar('{}')", field.as_str())),
            Expr::Name { name, span } => self.name_ref(name, *span),
            Expr::Text { span, .. } => {
                self.err(
                    Diagnostic::semantic(
                        Code::TypeMismatch,
                        *span,
                        "a text literal is only ever an argument to `feature`, `bar` or `param`",
                    )
                    .with_fix("did you mean `feature('…')`?"),
                );
                None
            }
            Expr::Unary {
                op: UnOp::Neg, rhs, ..
            } => {
                let a = self.num_expr(rhs)?;
                Some(format!("(-{a})"))
            }
            Expr::Unary {
                op: UnOp::Not,
                span,
                ..
            } => {
                self.err(
                    Diagnostic::semantic(
                        Code::TypeMismatch,
                        *span,
                        "`not` produces a condition; a value belongs here",
                    )
                    .with_see("guide#grammar.expressions"),
                );
                None
            }
            Expr::Binary { op, lhs, rhs, .. } if !op.is_logical() && !op.is_comparison() => {
                let a = self.num_expr(lhs);
                let b = self.num_expr(rhs);
                Some(format!("({} {} {})", a?, op.as_str(), b?))
            }
            Expr::Binary { span, .. } => {
                self.err(
                    Diagnostic::semantic(
                        Code::TypeMismatch,
                        *span,
                        "this is a condition; a value belongs here",
                    )
                    .with_fix("conditions combine with `and`, `or` and `not`, not with arithmetic")
                    .with_see("guide#grammar.expressions"),
                );
                None
            }
            call @ Expr::Call { .. } => self.call_value(call),
        }
    }

    fn name_ref(&mut self, name: &str, span: Span) -> Option<String> {
        if let Some(feature) = self.inputs.get(name) {
            let feature = feature.clone();
            self.used_inputs.insert(name.to_string());
            return Some(format!("feature('{feature}')"));
        }
        if self.params.contains(name) {
            return Some(format!("param('{name}')"));
        }
        if POSITION_STATE.contains(&name) {
            self.position_state(name, span);
            return None;
        }
        let mut d = Diagnostic::semantic(
            Code::UnknownName,
            span,
            format!("`{name}` is not a declared input, a declared parameter or a bar field"),
        )
        .with_see("guide#inputs");
        if self.catalogue.family(name).is_some() {
            d = d.with_fix(format!(
                "`{name}` is a feature family, so it needs its parameters: `{name}(close, 20)`"
            ));
        } else if let Some(near) = self.catalogue.nearest(name) {
            d = d.with_fix(format!(
                "did you mean `{near}(close, 20)`? `list_features(query=\"{near}\")`"
            ));
        } else {
            d = d.with_fix("declare it with an `input` clause, or name a feature family");
        }
        self.err(d);
        None
    }

    fn position_state(&mut self, name: &str, span: Span) {
        self.err(
            Diagnostic::semantic(
                Code::NeedsPositionState,
                span,
                format!(
                    "`{name}` reads position state, and a condition expression is a pure \
                     function of the current bar"
                ),
            )
            .with_fix(
                "say it in the exits instead — `exit time 48 bars`, `exit stop 2 % from entry` \
                 — which the runtime evaluates against the open position",
            )
            .with_see("guide#exits"),
        );
    }

    /// A call in value position: a feature, or one of the v1 forms.
    fn call_value(&mut self, e: &Expr) -> Option<String> {
        let Expr::Call {
            name, args, span, ..
        } = e
        else {
            unreachable!("call_value is only called on a call");
        };
        match name.as_str() {
            "bar" => {
                let field = self.one_text(name, args, *span)?;
                if crate::ast::BarField::parse(&field).is_none() {
                    self.err(
                        Diagnostic::semantic(
                            Code::UnknownName,
                            *span,
                            format!("`{field}` is not a bar field"),
                        )
                        .with_fix("one of `open`, `high`, `low`, `close` or `volume`"),
                    );
                    return None;
                }
                Some(format!("bar('{field}')"))
            }
            "param" => {
                let p = self.one_text(name, args, *span)?;
                if !self.params.contains(&p) {
                    self.err(
                        Diagnostic::semantic(
                            Code::UnknownName,
                            *span,
                            format!("`{p}` is used as a parameter and never declared"),
                        )
                        .with_fix(format!("add `param {p}: int[5..50] = 20`"))
                        .with_see("guide#params"),
                    );
                    return None;
                }
                Some(format!("param('{p}')"))
            }
            n if NEEDS_OPERATOR.contains(&n) => {
                self.err(
                    Diagnostic::semantic(
                        Code::NeedsOperator,
                        *span,
                        format!(
                            "`{n}` needs a bytecode operation the runtime does not have \
                             (FEAT-004 §3.3 lists it as an extension)"
                        ),
                    )
                    .with_fix(
                        "rewrite it with `+ - * /` and comparisons, or use a feature family \
                         that already computes what you want",
                    )
                    .with_see("guide#functions"),
                );
                None
            }
            n if NEEDS_HISTORY.contains(&n) => {
                // Reached when written in value position rather than as a
                // condition; the message is the same one and says the same
                // thing, so it is produced by the same path.
                self.bool_expr(e)
            }
            _ => {
                let feature = self.feature_name(e)?;
                self.features.insert(feature.clone());
                Some(format!("feature('{feature}')"))
            }
        }
    }

    fn one_text(&mut self, name: &str, args: &[Expr], span: Span) -> Option<String> {
        match args {
            [Expr::Text { value, .. }] => Some(value.clone()),
            _ => {
                self.err(
                    Diagnostic::semantic(
                        Code::WrongArity,
                        span,
                        format!("`{name}` takes one quoted name"),
                    )
                    .with_fix(format!("write `{name}('close')`")),
                );
                None
            }
        }
    }

    /// `ema(close, 7)` → `ema_7`; `feature('ema_7')` → `ema_7`.
    fn feature_name(&mut self, e: &Expr) -> Option<String> {
        let Expr::Call {
            name,
            args,
            span,
            name_span,
        } = e
        else {
            unreachable!("feature_name is only called on a call");
        };

        if name == "feature" {
            let raw = self.one_text(name, args, *span)?;
            self.features.insert(raw.clone());
            return Some(raw);
        }

        let Some(family) = self.catalogue.family(name) else {
            let mut d = Diagnostic::semantic(
                Code::UnknownFeature,
                *name_span,
                format!("`{name}` is not a feature family"),
            )
            .with_see("guide#features");
            d = match self.catalogue.nearest(name) {
                Some(near) => d.with_fix(format!(
                    "did you mean `{near}`? `list_features(query=\"{near}\")`"
                )),
                None => d.with_fix("ask `list_features(query=\"…\")` for what exists"),
            };
            self.err(d);
            return None;
        };
        let family = family.clone();

        // A leading `close` is the documented default source of every windowed
        // family, so it is accepted and dropped. Any other bar field is not:
        // the registry gives a family that reads the high its own name
        // (`donchian_upper`, `atr`, `parkinson_vol`) rather than a source
        // argument, precisely so a name is its own canonical form.
        let mut rest: &[Expr] = args;
        if let Some(Expr::Bar { field, span }) = args.first() {
            if *field == crate::ast::BarField::Close {
                rest = &args[1..];
            } else {
                self.err(
                    Diagnostic::semantic(
                        Code::BadFeatureParam,
                        *span,
                        format!(
                            "a feature family reads the close; `{}` is not a source you can \
                             pass",
                            field.as_str()
                        ),
                    )
                    .with_fix(
                        "families that read another field have their own name — `atr`, \
                         `donchian_upper`, `parkinson_vol`; `list_features(query=\"…\")`",
                    )
                    .with_see("guide#features"),
                );
                return None;
            }
        }

        if rest.len() != family.params.len() {
            let shape = if family.params.is_empty() {
                format!("`{name}()`")
            } else {
                let names: Vec<&str> = family.params.iter().map(|p| p.name.as_str()).collect();
                format!("`{name}(close, {})`", names.join(", "))
            };
            self.err(
                Diagnostic::semantic(
                    Code::WrongArity,
                    *span,
                    format!(
                        "`{name}` takes {} parameter(s); {} given",
                        family.params.len(),
                        rest.len()
                    ),
                )
                .with_fix(format!("write {shape}"))
                .with_see("guide#features"),
            );
            return None;
        }

        let mut parts: Vec<String> = Vec::with_capacity(rest.len());
        for (arg, spec) in rest.iter().zip(&family.params) {
            match arg {
                Expr::Number(Num { raw, span }) => {
                    let Ok(v) = raw.parse::<i64>() else {
                        self.err(
                            Diagnostic::semantic(
                                Code::BadFeatureParam,
                                *span,
                                format!(
                                    "`{}` is `{raw}`; feature parameters are whole numbers of bars",
                                    spec.name
                                ),
                            )
                            .with_fix("round it to a whole number")
                            .with_see("guide#features"),
                        );
                        return None;
                    };
                    if v < spec.min || v > spec.max {
                        self.err(
                            Diagnostic::semantic(
                                Code::BadFeatureParam,
                                *span,
                                format!(
                                    "`{}` is {v}; `{name}` accepts {}..={}",
                                    spec.name, spec.min, spec.max
                                ),
                            )
                            .with_fix(format!(
                                "choose a value between {} and {}",
                                spec.min, spec.max
                            ))
                            .with_see("guide#features"),
                        );
                        return None;
                    }
                    parts.push(v.to_string());
                }
                Expr::Name { name: p, span } if self.params.contains(p) => {
                    // v1.2's `{{name}}` interpolation: `params::materialize`
                    // substitutes the literal before the validator or the
                    // runtime ever sees the name.
                    parts.push(format!("{{{{{p}}}}}"));
                    let _ = span;
                }
                other => {
                    self.err(
                        Diagnostic::semantic(
                            Code::BadFeatureParam,
                            other.span(),
                            format!(
                                "`{}` must be a whole number or a declared parameter",
                                spec.name
                            ),
                        )
                        .with_fix(format!(
                            "write `{name}(close, 20)`, or declare `param {}: int[…] = …`",
                            spec.name
                        ))
                        .with_see("guide#features"),
                    );
                    return None;
                }
            }
        }
        Some(family.name_for(&parts))
    }

    // ── clauses ──────────────────────────────────────────────────────────────

    fn size(&mut self, ast: &Strategy) -> Option<(SizeMode, String)> {
        let Some(clause) = &ast.size else {
            if !ast.entries.is_empty() {
                self.err(
                    Diagnostic::semantic(
                        Code::MissingSize,
                        ast.header_span,
                        "this strategy enters positions and never says how large",
                    )
                    .with_fix("add `size fixed 0.01`, or `size equity_fraction 0.25`")
                    .with_see("guide#size"),
                );
            }
            return None;
        };
        match &clause.kind {
            SizeKind::Fixed(n) => Some((SizeMode::Fixed, n.raw.clone())),
            SizeKind::EquityFraction(n) => Some((SizeMode::PercentOfBalance, n.raw.clone())),
            SizeKind::RiskPerTrade(n) => Some((SizeMode::RiskUnit, n.raw.clone())),
            SizeKind::Other { word, .. } => {
                self.err(
                    Diagnostic::semantic(
                        Code::SizeModeNotRepresentable,
                        clause.span,
                        format!(
                            "`{word}` sizing has no `SizeMode` on the canonical order \
                             specification"
                        ),
                    )
                    .with_fix(
                        "the three the runtime places orders with are `fixed`, \
                         `equity_fraction` and `risk_per_trade`",
                    )
                    .with_see("guide#size"),
                );
                None
            }
        }
    }

    fn exits(&mut self, ast: &Strategy, nodes: &mut Vec<Node>) -> Option<ExitPolicy> {
        let requires_exits = version_at_least_1_3(ast);
        if ast.exits.is_empty() {
            if requires_exits {
                self.err(
                    Diagnostic::semantic(
                        Code::MissingExits,
                        ast.header_span,
                        "a strategy at version 1.3 declares what closes a position, even when \
                         the answer is nothing",
                    )
                    .with_fix(
                        "add an exit — `exit stop 2 % from entry`, `exit time 48 bars` — or say \
                         `exit none` to delegate to the risk layer on purpose",
                    )
                    .with_see("guide#exits"),
                );
            }
            return None;
        }

        if let Some(none_clause) = ast.exits.iter().find(|x| matches!(x.kind, ExitKind::None)) {
            if ast.exits.len() > 1 {
                self.err(
                    Diagnostic::semantic(
                        Code::MissingExits,
                        none_clause.span,
                        "`exit none` says there are no exit rules, and this strategy has some",
                    )
                    .with_fix("delete the `exit none` line")
                    .with_see("guide#exits"),
                );
                return None;
            }
            return Some(ExitPolicy::RiskLayerOnly);
        }

        let mut rules: Vec<ExitRule> = Vec::new();
        let mut when_index = 0usize;
        for clause in &ast.exits {
            match &clause.kind {
                ExitKind::None => unreachable!("handled above"),
                ExitKind::Stop(d) | ExitKind::Target(d) | ExitKind::Trailing(d) => {
                    let Some(distance) = self.distance(d.unit, d.unit_span, &d.value) else {
                        continue;
                    };
                    let wanted_anchor = match &clause.kind {
                        ExitKind::Trailing(_) => Anchor::HighWater,
                        _ => Anchor::Entry,
                    };
                    if d.anchor != wanted_anchor {
                        self.err(
                            Diagnostic::semantic(
                                Code::NotExpressible,
                                d.span,
                                format!(
                                    "this exit is measured from `{}`, and the canonical rule \
                                     measures it from `{}`",
                                    d.anchor.as_str(),
                                    wanted_anchor.as_str()
                                ),
                            )
                            .with_fix(format!("write `from {}`", wanted_anchor.as_str()))
                            .with_see("guide#exits"),
                        );
                        continue;
                    }
                    let mut rule = match &clause.kind {
                        ExitKind::Stop(_) => json!({"rule": "stop_loss"}),
                        ExitKind::Target(_) => json!({"rule": "take_profit"}),
                        _ => json!({"rule": "trailing_stop"}),
                    };
                    // `ExitDistance` is flattened into the rule, so its two
                    // keys are the rule's keys. Merged rather than spelled per
                    // variant so a change to the distance shape lands once.
                    for (k, v) in distance {
                        rule[k] = v;
                    }
                    match serde_json::from_value::<ExitRule>(rule) {
                        Ok(r) if r.is_well_formed() => rules.push(r),
                        // The bound depends on the unit, so the refusal has to
                        // as well: `1.5` as a fraction of entry is a stop that
                        // can never fire on a long, and `1.5 atr(14)` is the
                        // most ordinary stop in the domain. Reporting the
                        // fraction rule over both was what made the language
                        // unable to say the second.
                        _ => self.err(
                            Diagnostic::semantic(
                                Code::NotExpressible,
                                d.span,
                                match d.unit {
                                    Unit::Percent => format!(
                                        "a distance of `{}` is not a fraction of entry in (0, 1]",
                                        d.value.raw
                                    ),
                                    _ => format!(
                                        "a distance of `{}` is not a positive multiple of `{}`",
                                        d.value.raw,
                                        d.unit.render()
                                    ),
                                },
                            )
                            .with_fix(match d.unit {
                                Unit::Percent => {
                                    "a stop or target is between 0 % and 100 % of entry".to_string()
                                }
                                _ => format!(
                                    "write a positive multiple, as in `1.5 {}`; there is no \
                                     upper bound, because a 5-ATR stop is wide rather than wrong",
                                    d.unit.render()
                                ),
                            })
                            .with_see("guide#exits"),
                        ),
                    }
                }
                ExitKind::Time(n) => {
                    let Ok(bars) = n.raw.parse::<u32>() else {
                        self.err(
                            Diagnostic::semantic(
                                Code::NotExpressible,
                                n.span,
                                format!("`{}` is not a whole number of bars", n.raw),
                            )
                            .with_fix("write `exit time 48 bars`")
                            .with_see("guide#exits"),
                        );
                        continue;
                    };
                    if bars == 0 {
                        self.err(
                            Diagnostic::semantic(
                                Code::NotExpressible,
                                n.span,
                                "a time exit of 0 bars closes on the entry bar",
                            )
                            .with_fix("hold for at least one bar")
                            .with_see("guide#exits"),
                        );
                        continue;
                    }
                    rules.push(ExitRule::MaxHoldingBars { bars });
                }
                ExitKind::When(cond) => {
                    let Some(expr) = self.bool_expr(cond) else {
                        continue;
                    };
                    let emit = format!("x{when_index}");
                    let cond_id = format!("x{when_index}_cond");
                    nodes.push(Node {
                        id: cond_id.clone(),
                        kind: NodeKind::Condition { expr },
                    });
                    nodes.push(Node {
                        id: format!("x{when_index}_sig"),
                        kind: NodeKind::Signal {
                            when: cond_id,
                            emit: emit.clone(),
                        },
                    });
                    when_index += 1;
                    rules.push(ExitRule::OnSignal { emit });
                }
            }
        }
        if rules.is_empty() {
            return None;
        }
        Some(ExitPolicy::Rules { rules })
    }

    /// The `ExitDistance` fields for one SLv2 distance, as the keys they are
    /// flattened into the rule as.
    ///
    /// # Why the magnitude is not one number
    ///
    /// `%` converts exactly by moving the decimal point two places, because a
    /// percent-of-entry distance *is* a fraction: `2 %` is `0.02` of the
    /// reference price. A scaled distance is a **multiplier** and is stored as
    /// written: `1.5 atr(14)` is one and a half average true ranges, and
    /// shifting its point would turn it into `0.015` of something. That is the
    /// reason `ExitDistance`'s Rust field is called `magnitude` while its wire
    /// key stays `fraction` (ADR-0010 forbids rewriting the stored form), and
    /// the reason this function branches rather than normalising.
    ///
    /// Before ADR-D-09 this refused `atr` and `sigma` outright. The schema
    /// gained `DistanceUnit::Atr { period }` and the grammar gained the period,
    /// so there is nothing left to refuse — except a header that pins a version
    /// too low to hold the unit, which is recorded here and reported once in
    /// [`Self::version`].
    fn distance(
        &mut self,
        unit: Unit,
        unit_span: Span,
        value: &Num,
    ) -> Option<Vec<(&'static str, Value)>> {
        match unit {
            Unit::Percent => Some(vec![(
                "fraction",
                Value::String(shift_point(&value.raw, -2)),
            )]),
            Unit::Atr { period } | Unit::Sigma { period } => {
                if self.scaled_unit.is_none() {
                    self.scaled_unit = Some((unit_span, unit.render()));
                }
                Some(vec![
                    ("fraction", Value::String(value.raw.clone())),
                    (
                        "unit",
                        json!({ "measure": unit.as_str(), "period": period }),
                    ),
                ])
            }
        }
    }

    /// The `definition_version` this document gets, and the one refusal that
    /// comes with it.
    ///
    /// A header with no `version` gets the lowest version that can hold what was
    /// written: [`TARGET_VERSION`], or [`SCALED_UNIT_VERSION`] when a
    /// volatility-scaled distance is present. An *explicit* `version` below the
    /// one a scaled distance needs is refused rather than quietly raised,
    /// because a reader at the declared version has no `unit` field, would take
    /// `1.5` for a fraction of the entry price, and would place a 150 % stop
    /// where a 1.5-ATR stop was meant. Silently raising the header would be
    /// deciding on the author's behalf that a version claim is advisory.
    fn version(&mut self, ast: &Strategy) -> String {
        let Some((scaled_span, rendered)) = self.scaled_unit.clone() else {
            return ast
                .version
                .as_ref()
                .map_or_else(|| TARGET_VERSION.to_string(), |(v, _)| v.clone());
        };
        match &ast.version {
            None => SCALED_UNIT_VERSION.to_string(),
            Some((declared, _)) if declared.as_str() == SCALED_UNIT_VERSION => declared.clone(),
            Some((declared, _)) => {
                self.err(
                    Diagnostic::semantic(
                        Code::UnitNotRepresentable,
                        scaled_span,
                        format!(
                            "a distance in `{rendered}` needs definition_version \
                             {SCALED_UNIT_VERSION}, and this strategy declares `version \
                             {declared}`, which has no unit on an exit distance"
                        ),
                    )
                    .with_fix(format!(
                        "delete the `version {declared}` line — SLv2 writes \
                         {SCALED_UNIT_VERSION} on its own when a distance is scaled — or write \
                         the distance in `%` of entry. Do not hand-convert it: an ATR expressed \
                         as a fraction is right on the bar you computed it and wrong on the next"
                    ))
                    .with_see("guide#exits.units"),
                );
                declared.clone()
            }
        }
    }

    fn risk(&mut self, ast: &Strategy) -> RiskOverrides {
        let mut map = serde_json::Map::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for item in &ast.risk {
            if !seen.insert(item.kind.as_str()) {
                self.err(
                    Diagnostic::semantic(
                        Code::DuplicateName,
                        item.span,
                        format!("`{}` is set twice", item.kind.as_str()),
                    )
                    .with_fix("keep the tighter of the two"),
                );
                continue;
            }
            match item.kind {
                RiskKind::MaxPosition => {
                    map.insert("max_position".into(), Value::String(item.value.raw.clone()));
                }
                RiskKind::MaxOrdersPerMin | RiskKind::MaxOrdersPerSec => {
                    let Ok(v) = item.value.raw.parse::<u32>() else {
                        self.err(
                            Diagnostic::semantic(
                                Code::NotExpressible,
                                item.value.span,
                                format!("`{}` is not a whole number of orders", item.value.raw),
                            )
                            .with_fix("write a whole number, as in `risk max_orders_per_min 6`")
                            .with_see("guide#risk"),
                        );
                        continue;
                    };
                    let key = if item.kind == RiskKind::MaxOrdersPerMin {
                        "max_order_rate_per_minute"
                    } else {
                        "max_order_rate_per_second"
                    };
                    map.insert(key.into(), Value::from(v));
                }
                RiskKind::MaxDailyLoss => {
                    self.err(
                        Diagnostic::semantic(
                            Code::RiskItemNotRepresentable,
                            item.span,
                            "`max_daily_loss` has no field on `RiskOverrides`, which carries \
                             position size and order rate only",
                        )
                        .with_fix(
                            "set a daily loss limit in the platform risk configuration; a \
                             strategy override may only tighten what exists there",
                        )
                        .with_see("guide#risk"),
                    );
                }
            }
        }
        match serde_json::from_value::<RiskOverrides>(Value::Object(map)) {
            Ok(r) => r,
            Err(e) => {
                let span = ast.risk.first().map_or(ast.header_span, |r| r.span);
                self.err(
                    Diagnostic::semantic(
                        Code::NotExpressible,
                        span,
                        format!("a risk limit is not a number the platform can hold: {e}"),
                    )
                    .with_fix("write a decimal such as `0.5`")
                    .with_see("guide#risk"),
                );
                RiskOverrides::default()
            }
        }
    }

    fn params(&mut self, ast: &Strategy) -> BTreeMap<String, ParamSpec> {
        let mut out = BTreeMap::new();
        for p in &ast.params {
            let value = match &p.kind {
                ParamKind::Int {
                    min,
                    max,
                    step,
                    default,
                } => {
                    let Some(min) = self.int(min) else { continue };
                    let Some(max) = self.int(max) else { continue };
                    let Some(default) = self.int(default) else {
                        continue;
                    };
                    let step = match step {
                        None => 1,
                        Some(s) => match self.int(s) {
                            Some(v) => v,
                            None => continue,
                        },
                    };
                    json!({"type": "int", "min": min, "max": max, "step": step, "default": default})
                }
                ParamKind::Float {
                    min,
                    max,
                    log,
                    default,
                } => {
                    let Some(min) = self.float(min) else { continue };
                    let Some(max) = self.float(max) else { continue };
                    let Some(default) = self.float(default) else {
                        continue;
                    };
                    json!({
                        "type": "float",
                        "min": min,
                        "max": max,
                        "default": default,
                        "scale": if *log { "log" } else { "linear" },
                    })
                }
                ParamKind::Enum { choices, default } => {
                    json!({"type": "enum", "choices": choices, "default": default})
                }
            };
            let spec: ParamSpec = match serde_json::from_value(value) {
                Ok(s) => s,
                Err(e) => {
                    self.err(
                        Diagnostic::semantic(
                            Code::BadParamSpec,
                            p.span,
                            format!("`{}` is not a parameter the sweeper can read: {e}", p.name),
                        )
                        .with_see("guide#params"),
                    );
                    continue;
                }
            };
            if let Err(e) = spec.validate_spec(&p.name) {
                self.err(
                    Diagnostic::semantic(Code::BadParamSpec, p.span, e.to_string())
                        .with_fix("the default must lie inside the range, and min ≤ max")
                        .with_see("guide#params"),
                );
                continue;
            }
            out.insert(p.name.clone(), spec);
        }
        out
    }

    fn int(&mut self, n: &Num) -> Option<i64> {
        match n.raw.parse::<i64>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.err(
                    Diagnostic::semantic(
                        Code::BadParamSpec,
                        n.span,
                        format!("`{}` is not a whole number", n.raw),
                    )
                    .with_fix("an `int` parameter's range and default are whole numbers")
                    .with_see("guide#params"),
                );
                None
            }
        }
    }

    fn float(&mut self, n: &Num) -> Option<f64> {
        match n.raw.parse::<f64>() {
            Ok(v) if v.is_finite() => Some(v),
            _ => {
                self.err(
                    Diagnostic::semantic(
                        Code::BadParamSpec,
                        n.span,
                        format!("`{}` is not a finite number", n.raw),
                    )
                    .with_see("guide#params"),
                );
                None
            }
        }
    }
}

fn side_of(side: Side) -> domain::order::Side {
    match side {
        Side::Long => domain::order::Side::Buy,
        Side::Short => domain::order::Side::Sell,
    }
}

/// Whether this document is at the version that requires an exit policy.
fn version_at_least_1_3(ast: &Strategy) -> bool {
    let Some((v, _)) = &ast.version else {
        return true;
    };
    let mut parts = v.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor) >= (1, 3)
}

/// Move a decimal literal's point `k` places, exactly.
///
/// `2` → `0.02` at `k = -2`, and back at `k = 2`. Done on the digits rather
/// than through `f64` because the canonical form stores these as decimal strings
/// (ADR-0002) and `0.1_f64 * 0.01` is not `0.001`. The result is normalised —
/// no leading zeros beyond one, no trailing zeros after the point — so that
/// moving the point there and back is the identity on the text, which is what
/// the round trip needs.
pub(crate) fn shift_point(raw: &str, k: i32) -> String {
    let (sign, body) = match raw.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", raw),
    };
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    let digits: String = format!("{int_part}{frac_part}");
    let point = int_part.len() as i32 + k;

    let (digits, point) = if point <= 0 {
        (
            format!("{}{digits}", "0".repeat((1 - point) as usize)),
            1i32,
        )
    } else if point as usize > digits.len() {
        let pad = point as usize - digits.len();
        (format!("{digits}{}", "0".repeat(pad)), point)
    } else {
        (digits, point)
    };

    let (mut lhs, rhs) = digits.split_at(point as usize);
    lhs = lhs.trim_start_matches('0');
    let lhs = if lhs.is_empty() { "0" } else { lhs };
    let rhs = rhs.trim_end_matches('0');
    let out = if rhs.is_empty() {
        lhs.to_string()
    } else {
        format!("{lhs}.{rhs}")
    };
    if out == "0" {
        out
    } else {
        format!("{sign}{out}")
    }
}

#[cfg(test)]
mod tests {
    /// A constraint that is not a constraint is refused where it is written.
    ///
    /// It used to be stored verbatim and parsed at sweep time — the free-form
    /// string class: accepted at the door, rejected at the far end, with the
    /// error naming a place the author is no longer looking.
    #[test]
    fn a_constraint_that_is_not_one_is_refused_at_compile_time() {
        let d = code(
            "strategy s on $bound_at_init timeframe 1h
param fast: int[5..50 step 1] = 7
param slow: int[20..200 step 5] = 21
constraint blah
entry long when ema(close, 7) > ema(close, 21)
size fixed 1
exit none
",
            Code::UnknownName,
        );
        assert!(
            d.message.contains("is not a constraint"),
            "refused, but not for the reason the author needs to read: {d:?}"
        );

        // And the one that is a constraint still compiles, so the check is not
        // simply refusing everything.
        let good = super::super::text_to_definition(
            "strategy s on $bound_at_init timeframe 1h
param fast: int[5..50 step 1] = 7
param slow: int[20..200 step 5] = 21
constraint param('fast') < param('slow')
entry long when ema(close, 7) > ema(close, 21)
size fixed 1
exit none
",
        )
        .expect("a real constraint compiles");
        assert_eq!(
            good.constraints,
            vec!["param('fast') < param('slow')".to_string()]
        );
    }

    use super::*;
    use crate::parse::parse;
    use domain::strategy_def::exits::DistanceUnit;

    fn def(src: &str) -> StrategyDefinition {
        match parse(src).and_then(|ast| compile(&ast)) {
            Ok(d) => d,
            Err(e) => panic!("expected a definition, got {e:#?}"),
        }
    }

    fn refuse(src: &str) -> Vec<Diagnostic> {
        match parse(src).and_then(|ast| compile(&ast)) {
            Ok(d) => panic!("expected a refusal, compiled to {d:#?}"),
            Err(e) => e,
        }
    }

    fn code(src: &str, want: Code) -> Diagnostic {
        let errs = refuse(src);
        errs.iter()
            .find(|d| d.code == want)
            .unwrap_or_else(|| panic!("expected {want}, got {errs:#?}"))
            .clone()
    }

    const TREND: &str = "\
strategy ema_cross on $bound_at_init timeframe 1h
input fast = ema(close, 7)
input slow = ema(close, 21)
entry long when fast > slow and rsi(close, 14) < 60
size fixed 0.01
exit stop 2 % from entry
exit target 5 % from entry
exit time 48 bars
risk max_position 0.5
";

    /// S-5.6 acceptance: the compiled definition is the hand-written JSON.
    #[test]
    fn the_trend_skeleton_compiles_to_the_definition_a_human_would_have_written() {
        let got = def(TREND);
        let want: StrategyDefinition = serde_json::from_str(
            r#"{
              "strategy_id": "ema_cross",
              "definition_version": "1.3",
              "asset_class": "crypto_spot_cex",
              "min_trust_tier": "centralized_exchange",
              "inputs": [
                { "lane": "market.bars.1h", "instrument": "$bound_at_init" },
                { "lane": "features.technical", "instrument": "$bound_at_init",
                  "features": ["ema_21", "ema_7", "rsi_14"] }
              ],
              "nodes": [
                { "id": "e0_cond", "type": "condition",
                  "expr": "((feature('ema_7') > feature('ema_21')) * (feature('rsi_14') < 60))" },
                { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }
              ],
              "actions": [
                { "on_signal": "long", "type": "place_order",
                  "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
              ],
              "risk_overrides": { "max_position": "0.5" },
              "exits": { "mode": "rules", "rules": [
                { "rule": "stop_loss", "fraction": "0.02" },
                { "rule": "take_profit", "fraction": "0.05" },
                { "rule": "max_holding_bars", "bars": 48 }
              ]}
            }"#,
        )
        .expect("the hand-written JSON parses");
        assert_eq!(got, want);
    }

    /// The invariant the whole boolean encoding rests on: every compiled
    /// condition is `0` or `1`, so `not` of it is its complement. `or` written
    /// as a bare sum would be truthy at the top level and wrong inside a `not`,
    /// which is a bug no top-level test would find.
    #[test]
    fn normalises_or_so_that_not_of_it_is_correct() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
entry long when not (close > 1 or close < 0)
size fixed 1
exit none
");
        let NodeKind::Condition { expr } = &d.nodes[0].kind else {
            panic!("expected a condition node");
        };
        assert_eq!(
            expr,
            "(1 - (((bar('close') > 1) + (bar('close') < 0)) > 0))"
        );
        // Evaluated over {0,1}: both true → inner sum 2 → `> 0` → 1 → `1-1` = 0.
        // A bare `(A + B)` would have given `1 - 2 = -1`, which the runtime
        // reads as true.
    }

    #[test]
    fn and_is_a_product_and_the_operands_are_parenthesised() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1 and close < 5
size fixed 1
exit none
");
        let NodeKind::Condition { expr } = &d.nodes[0].kind else {
            panic!("expected a condition node");
        };
        assert_eq!(expr, "((bar('close') > 1) * (bar('close') < 5))");
    }

    #[test]
    fn between_expands_to_two_comparisons() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
entry long when between(zscore(close, 20), 0, 2)
size fixed 1
exit none
");
        let NodeKind::Condition { expr } = &d.nodes[0].kind else {
            panic!("expected a condition node");
        };
        assert_eq!(
            expr,
            "((feature('zscore_20') >= 0) * (feature('zscore_20') <= 2))"
        );
    }

    #[test]
    fn a_parameterised_feature_uses_the_v1_2_interpolation() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
param fast: int[5..50] = 7
input trend = ema(close, fast)
entry long when trend > 0
size fixed 1
exit none
");
        assert_eq!(d.inputs[1].features, vec!["ema_{{fast}}"]);
        assert!(d.parameters.contains_key("fast"));
    }

    // ── the refusals, each naming what is missing ────────────────────────────

    #[test]
    fn crossed_above_says_the_grammar_sees_one_bar() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
input fast = ema(close, 7)
input slow = ema(close, 21)
entry long when crossed_above(fast, slow)
size fixed 1
exit none
",
            Code::NeedsHistory,
        );
        assert!(d.message.contains("only the current bar"), "{}", d.message);
        assert!(d.fix.as_ref().unwrap().contains("macd"));
    }

    #[test]
    fn a_state_predicate_is_sent_to_the_exits() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when bars_in_trade > 3
size fixed 1
exit none
",
            Code::NeedsPositionState,
        );
        assert!(d.fix.as_ref().unwrap().contains("exit time 48 bars"));
    }

    #[test]
    fn a_scaled_stop_stores_the_unit_and_the_lookback_that_was_written() {
        let d = def(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit stop 1.5 atr(14) from entry
exit trailing 3 sigma(20) from high_water
",
        );
        let Some(ExitPolicy::Rules { rules }) = &d.exits else {
            panic!("expected rules, got {:?}", d.exits);
        };
        assert_eq!(rules[0].unit(), DistanceUnit::Atr { period: 14 });
        assert_eq!(rules[1].unit(), DistanceUnit::Sigma { period: 20 });
        // The magnitude is a multiplier and is stored as written. Shifting the
        // point, as `%` does, would turn one and a half ATRs into 0.015 of
        // something — and `as_fraction_of_entry` is the accessor that stops a
        // pre-1.4 reader from taking it for one.
        let d0 = rules[0].distance().expect("a stop measures a distance");
        assert_eq!(d0.magnitude.to_string(), "1.5");
        assert!(d0.as_fraction_of_entry().is_none());
    }

    /// A scaled unit is the one thing SLv2 can write that needs 1.4, so the
    /// version is measured from the document rather than pinned to a constant.
    /// A `%`-only strategy claiming 1.4 would be unreadable to a 1.3 runtime in
    /// exchange for a field it does not set.
    #[test]
    fn the_version_is_raised_by_a_scaled_unit_and_by_nothing_else() {
        let scaled = def(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit stop 1.5 atr(14) from entry
",
        );
        assert_eq!(scaled.definition_version, SCALED_UNIT_VERSION);

        let percent = def(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit stop 2 % from entry
",
        );
        assert_eq!(percent.definition_version, TARGET_VERSION);
    }

    /// The header is a claim, not a hint. Raising it silently would mean a
    /// reader at the declared version taking `1.5` for a fraction of entry and
    /// placing a 150 % stop where a 1.5-ATR stop was meant.
    #[test]
    fn a_version_pinned_below_the_one_a_scaled_unit_needs_is_refused() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
version 1.3
entry long when close > 1
size fixed 1
exit stop 1.5 atr(14) from entry
",
            Code::UnitNotRepresentable,
        );
        assert!(d.message.contains("atr(14)"), "{}", d.message);
        assert!(d.message.contains("1.4"), "{}", d.message);
        assert!(
            d.fix.as_ref().unwrap().contains("Do not hand-convert"),
            "{:?}",
            d.fix
        );
    }

    /// `1.5 atr` is not a stop with a default lookback. A 14-bar ATR and a
    /// 50-bar ATR are different numbers and the author wrote neither.
    #[test]
    fn a_scaled_unit_without_a_lookback_is_refused_rather_than_defaulted() {
        for unit in ["atr", "sigma"] {
            let src = format!(
                "strategy s on $bound_at_init timeframe 1h\nentry long when close > 1\nsize fixed 1\nexit stop 1.5 {unit} from entry\n"
            );
            let ds = refuse(&src);
            let d = ds
                .iter()
                .find(|d| d.code == Code::UnitNeedsPeriod)
                .unwrap_or_else(|| panic!("expected unit_needs_period for `{unit}`: {ds:#?}"));
            assert!(d.fix.as_ref().unwrap().contains("(14)"), "{:?}", d.fix);
        }
    }

    /// `atr(0)` is not "no lookback": it is the feature name `atr_0`, which
    /// nothing serves. Refused where it is written rather than at the first bar
    /// of a run somebody paid for.
    #[test]
    fn a_lookback_the_engine_will_not_compute_is_refused_at_the_distance() {
        for bad in ["atr(0)", "atr(1)", "sigma(1.5)", "atr(200000)"] {
            let src = format!(
                "strategy s on $bound_at_init timeframe 1h\nentry long when close > 1\nsize fixed 1\nexit stop 1.5 {bad} from entry\n"
            );
            let ds = refuse(&src);
            assert!(
                ds.iter().any(|d| d.code == Code::BadFeatureParam),
                "`{bad}` was not refused as a bad lookback: {ds:#?}"
            );
        }
    }

    /// `domain::strategy_def::exits` spells `MIN_UNIT_PERIOD`/`MAX_UNIT_PERIOD`
    /// out because it sits below `features` in the dependency order, and its own
    /// doc comment names this test as the thing holding that mirror straight.
    /// Read from the registry, so the day `atr` changes its range the two stop
    /// agreeing here rather than inside a stored strategy.
    #[test]
    fn the_unit_periods_match_the_engine_registry() {
        let cat = Catalogue::engine();
        for (unit, family) in [
            (DistanceUnit::Atr { period: 14 }, "atr"),
            (DistanceUnit::Sigma { period: 20 }, "rolling_std"),
        ] {
            let f = cat
                .family(family)
                .unwrap_or_else(|| panic!("the engine serves `{family}`"));
            assert_eq!(f.params.len(), 1, "`{family}` takes one lookback");
            assert_eq!(
                (f.params[0].min, f.params[0].max),
                (
                    i64::from(domain::strategy_def::exits::MIN_UNIT_PERIOD),
                    i64::from(domain::strategy_def::exits::MAX_UNIT_PERIOD)
                ),
                "`{family}`'s range and the domain's unit-period bounds disagree"
            );
            // And the name the unit asks the engine for is one the engine
            // resolves. Without this the bounds could agree while the *name* was
            // wrong: `sigma` is served by `rolling_std`, not by `sigma`.
            let name = unit.feature_name().expect("a scaled unit names a feature");
            assert!(
                cat.resolve(&name).is_some(),
                "`{name}` does not resolve in the engine registry"
            );
        }
    }

    #[test]
    fn an_unknown_family_suggests_the_nearest_one() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when rsl(close, 14) < 30
size fixed 1
exit none
",
            Code::UnknownFeature,
        );
        assert_eq!(
            d.fix.as_deref(),
            Some("did you mean `rsi`? `list_features(query=\"rsi\")`")
        );
    }

    #[test]
    fn a_bare_family_name_says_it_needs_its_parameters() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when rsi < 30
size fixed 1
exit none
",
            Code::UnknownName,
        );
        assert!(
            d.fix.as_ref().unwrap().contains("rsi(close, 20)"),
            "{:?}",
            d.fix
        );
    }

    #[test]
    fn the_wrong_arity_shows_the_shape_with_the_parameter_names() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when bb_upper(close, 20) > 1
size fixed 1
exit none
",
            Code::WrongArity,
        );
        assert!(
            d.fix.as_ref().unwrap().contains("bb_upper(close, n, k)"),
            "{:?}",
            d.fix
        );
    }

    #[test]
    fn a_feature_parameter_outside_its_range_names_the_range() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when rsi(close, 1) < 30
size fixed 1
exit none
",
            Code::BadFeatureParam,
        );
        assert!(d.message.contains("2..=100000"), "{}", d.message);
    }

    #[test]
    fn a_source_other_than_close_points_at_the_family_that_reads_it() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when rolling_max(high, 20) > 1
size fixed 1
exit none
",
            Code::BadFeatureParam,
        );
        assert!(d.fix.as_ref().unwrap().contains("donchian_upper"));
    }

    #[test]
    fn an_input_that_is_not_a_feature_is_refused_by_name() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
input spread = high - low
entry long when spread > 1
size fixed 1
exit none
",
            Code::InputNotAFeature,
        );
        assert!(d.fix.as_ref().unwrap().contains("list_features"));
    }

    #[test]
    fn an_input_nothing_reads_is_refused() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
input trend = ema(close, 7)
entry long when close > 1
size fixed 1
exit none
",
            Code::UnusedInput,
        );
        assert!(d.message.contains("trend"));
    }

    #[test]
    fn a_strategy_that_enters_and_never_sizes_is_refused() {
        code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
exit none
",
            Code::MissingSize,
        );
    }

    /// `exits.rs`: "no exits must be said out loud". SLv2 makes forgetting them
    /// unrepresentable rather than defaulting to `RiskLayerOnly`, because the
    /// two look identical at runtime and are different claims.
    #[test]
    fn forgetting_exits_is_refused_and_delegating_them_is_a_clause() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
",
            Code::MissingExits,
        );
        assert!(d.fix.as_ref().unwrap().contains("exit none"));

        let delegated = def("\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit none
");
        assert_eq!(delegated.exits, Some(ExitPolicy::RiskLayerOnly));
    }

    #[test]
    fn a_pre_1_3_document_may_have_no_exits_at_all() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
version 1.0
entry long when close > 1
size fixed 1
");
        assert_eq!(d.definition_version, "1.0");
        assert_eq!(d.exits, None, "absence is not RiskLayerOnly");
    }

    #[test]
    fn max_daily_loss_says_where_a_daily_loss_limit_lives() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit none
risk max_daily_loss 100
",
            Code::RiskItemNotRepresentable,
        );
        assert!(d.fix.as_ref().unwrap().contains("tighten"));
    }

    #[test]
    fn vol_target_sizing_names_the_three_modes_that_exist() {
        let d = code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size vol_target 0.15
exit none
",
            Code::SizeModeNotRepresentable,
        );
        assert!(d.fix.as_ref().unwrap().contains("equity_fraction"));
    }

    #[test]
    fn a_cooldown_is_refused_after_parsing_not_during_it() {
        code(
            "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1 cooldown 5 bars
size fixed 1
exit none
",
            Code::ClauseNotRepresentable,
        );
    }

    #[test]
    fn an_exit_when_clause_becomes_a_signal_the_exit_policy_watches() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit when rsi(close, 14) > 70
");
        assert_eq!(
            d.exits,
            Some(ExitPolicy::Rules {
                rules: vec![ExitRule::OnSignal { emit: "x0".into() }]
            })
        );
        assert!(d
            .nodes
            .iter()
            .any(|n| matches!(&n.kind, NodeKind::Signal { emit, .. } if emit == "x0")));
    }

    #[test]
    fn two_entries_on_the_same_side_share_one_action() {
        let d = def("\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
entry long when close < 0
size fixed 1
exit none
");
        assert_eq!(d.actions.len(), 1);
        assert_eq!(
            d.nodes
                .iter()
                .filter(|n| matches!(&n.kind, NodeKind::Signal { emit, .. } if emit == "long"))
                .count(),
            2
        );
    }

    #[test]
    fn every_refusal_carries_a_fix_or_says_why_it_cannot() {
        let sources = [
            "strategy s on $bound_at_init timeframe 1h\nentry long when rsl(close,14) < 1\nsize fixed 1\nexit none\n",
            "strategy s on $bound_at_init timeframe 1h\nentry long when flat\nsize fixed 1\nexit none\n",
            "strategy s on $bound_at_init timeframe 1h\nentry long when close > 1\nsize fixed 1\nexit stop 1 sigma from entry\n",
        ];
        let mut seen = 0;
        for src in sources {
            for d in refuse(src) {
                assert!(d.fix.is_some(), "{:?} has no fix", d.code);
                assert!(d.see.is_some(), "{:?} has no guide anchor", d.code);
                seen += 1;
            }
        }
        assert!(seen >= sources.len(), "the scan found {seen} refusals");
    }

    // ── the decimal helper ───────────────────────────────────────────────────

    #[test]
    fn moving_the_point_there_and_back_is_the_identity() {
        for raw in ["2", "1.5", "0.5", "100", "0.05", "-3.25", "12.5"] {
            let there = shift_point(raw, -2);
            let back = shift_point(&there, 2);
            assert_eq!(back, raw, "{raw} → {there} → {back}");
        }
    }

    #[test]
    fn percent_becomes_a_fraction_exactly() {
        assert_eq!(shift_point("2", -2), "0.02");
        assert_eq!(shift_point("1.5", -2), "0.015");
        assert_eq!(shift_point("100", -2), "1");
        assert_eq!(shift_point("0.05", -2), "0.0005");
        assert_eq!(shift_point("0", -2), "0");
    }
}
