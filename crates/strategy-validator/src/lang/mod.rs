//! **SLv2** — the language a model writes strategies in (AGENT-011 §3, FEAT-004 §3.1).
//!
//! # Why a text language at all
//!
//! The canonical storage form is JSON and stays JSON (ADR-0010). What changes is
//! what a *model* is asked to produce. Nested JSON is the form a 7B mis-nests and
//! a frontier model pads, and neither can read back what it wrote without
//! re-parsing it. `entry long when crossed_above(ema(close, 7), ema(close, 21))`
//! is one line a quant would recognise, and it compiles to the same document the
//! visual builder produces.
//!
//! # The shape of the language, and why it has that shape
//!
//! **One clause per line, keyword first.** Every line begins with `strategy`,
//! `asset_class`, `timeframe`, `param`, `input`, `entry`, `size`, `exit` or
//! `risk`. That is what makes a clause-level JSON grammar possible (§3, P7): a
//! constrained tier fills one clause per generation and the compiler assembles
//! the text, rather than being asked to close braces correctly.
//!
//! **Features are written as calls.** `ema(close, 7)` rather than
//! `feature('ema_7')`. The first is how the idea is said; the second is how it is
//! stored. The compiler maps one to the other through
//! [`features::registry`] — the same table the backtest and the live runtime
//! resolve against — so a feature that compiles is a feature that runs.
//!
//! **Comments are part of the language.** `# …` binds to the clause that
//! follows, travels onto the canonical document and comes back out of the
//! decompiler. A threshold of 0.6 with no record of why is a number nobody can
//! audit six months later, and compile → JSON → decompile is exactly where that
//! record used to be lost (AGENT-011 §3).
//!
//! # What this compiles to, and the one surprise in it
//!
//! The v1 expression grammar (`domain::strategy_def::nodes`) is frozen and has no
//! `and`, no `or` and no `not`. It does not need them: its comparison operators
//! **push 1.0 or 0.0 onto a numeric stack** (`strategy_runtime::bytecode`), a
//! parenthesised expression is a legal primary, and a program is true when its
//! result is non-zero. So
//!
//! - `a and b`  →  `(a) * (b)`
//! - `a or b`   →  `(a) + (b)`
//! - `not a`    →  `1 - (a)`
//!
//! are exact lowerings into the frozen grammar, evaluated by the existing
//! bytecode with no new opcode and no runtime change. `boolean_lowering_matches_
//! the_truth_table` proves it on all combinations rather than asserting it here.
//! This is why SLv2 could be built without touching `crates/strategy-runtime`,
//! which another track owns.
//!
//! # What SLv2 refuses, and why refusing is the right answer
//!
//! A language that silently compiles something into a different meaning is worse
//! than one that says no. These are refused with a [`crate::teach`] fix:
//!
//! - **`abs`, `min`, `max`, `clip`** (AGENT-011 §3) — the frozen grammar has no
//!   function but `feature` and `bar`, and there is no arithmetic identity for
//!   them. Compiling `abs(x)` to `x` would be a lie.
//! - **State predicates** (`flat`, `long`, `bars_in_trade`, `unrealized_return`,
//!   `last_exit_reason`) — deliberately not in the grammar; ADR-P6-01 put
//!   position state in the typed `exits` block precisely so the comparison
//!   operator would not have to grow state. The error points there.
//! - **`atr` and `sigma` exit units** — `ExitRule::StopLoss` carries a
//!   `fraction` of entry and nothing else at `definition_version` 1.3, which is
//!   frozen. `1.5 atr` is not `1.5 %` and must not be written as it.
//!
//! Each refusal names what to write instead. The list is a report of what the
//! canonical form can hold today, not a wish list.

pub mod compile;
pub mod decompile;
pub mod moves;
pub mod parse;
pub mod skeleton;

use std::fmt::Write as _;

pub use compile::{compile, CompileOutput};
pub use decompile::decompile;
pub use parse::{parse, ParseError};

/// A parsed SLv2 program.
///
/// The AST is the whole meaning: [`render`](Program::render) is a pure function
/// of it, so two programs that render identically are the same program and a
/// program that round-trips is proved to have lost nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub strategy_id: String,
    pub asset_class: String,
    pub timeframe: String,
    pub params: Vec<ParamDecl>,
    pub inputs: Vec<Binding>,
    pub entries: Vec<Entry>,
    pub size: Option<Size>,
    pub exits: Vec<ExitClause>,
    pub risk: Vec<RiskItem>,
    /// Comments attached to the `strategy` line — the strategy's own note.
    pub comments: Vec<String>,
}

/// A typed, sweepable parameter (AGENT-011 §3; v1.2 `parameters`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDecl {
    pub name: String,
    /// `int` or `number`.
    pub ptype: String,
    pub default: String,
    pub min: String,
    pub max: String,
    pub comments: Vec<String>,
}

/// A named expression, mandatory for anything used twice (AGENT-011 §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: String,
    pub expr: Expr,
    pub comments: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub side: Side,
    pub when: Expr,
    pub comments: Vec<String>,
}

/// How much to trade. `fixed` only in v2.0; everything else in FEAT-004 §3.1's
/// `size_expr` needs equity or a model at decision time, which the v1 action
/// schema has no field for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Size {
    pub kind: String,
    pub value: String,
    pub comments: Vec<String>,
}

/// One exit clause. Mirrors `domain::strategy_def::exits::ExitRule`, which is
/// frozen: this enum gains a variant only when that one does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitKind {
    /// `exit stop <n> %`
    Stop(String),
    /// `exit target <n> %`
    Target(String),
    /// `exit trail <n> %`
    Trail(String),
    /// `exit time <n> bars`
    Time(u32),
    /// `exit on signal <name>`
    OnSignal(String),
    /// `exit risk_layer_only` — the stated choice to delegate, never an absence.
    RiskLayerOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitClause {
    pub kind: ExitKind,
    pub comments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskItem {
    pub name: String,
    pub value: String,
    pub comments: Vec<String>,
}

// ── expressions ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl ArithOp {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
        }
    }
    fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Sub => 4,
            Self::Mul | Self::Div => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

impl CmpOp {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Lt => "<",
            Self::Ge => ">=",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// A numeric literal, kept exactly as written so `2` does not become `2.0`.
    Num(String),
    /// `ema(close, 7)` — a catalogued family with its integer parameters.
    Feature { family: String, params: Vec<usize> },
    /// `close`, `open`, `high`, `low`, `volume` used as a bar field.
    Bar(String),
    /// `param(fast)`.
    Param(String),
    /// A name bound by an `input` clause.
    Input(String),
    Neg(Box<Expr>),
    Arith { op: ArithOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Cmp { op: CmpOp, lhs: Box<Expr>, rhs: Box<Expr> },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// `crossed_above(a, b)` — see the module docs on rising-edge semantics.
    CrossedAbove(Box<Expr>, Box<Expr>),
    CrossedBelow(Box<Expr>, Box<Expr>),
    /// `between(x, lo, hi)`, inclusive.
    Between(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `rising(close, n)` — the close is above its own value `n` bars ago.
    Rising(Box<Expr>, usize),
    Falling(Box<Expr>, usize),
}

impl Expr {
    /// Binding tightness, for deciding where the renderer must put parentheses.
    fn precedence(&self) -> u8 {
        match self {
            Self::Or(..) => 1,
            Self::And(..) => 2,
            Self::Not(..) => 3,
            Self::Cmp { .. } => 3,
            Self::Arith { op, .. } => op.precedence(),
            _ => 9,
        }
    }

    /// Every feature name this expression resolves against the catalogue, in
    /// first-seen order, with `inputs` already substituted by the caller.
    pub fn feature_names(&self, out: &mut Vec<String>) {
        match self {
            Self::Feature { family, params } => {
                let mut name = family.clone();
                for p in params {
                    let _ = write!(name, "_{p}");
                }
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            Self::Neg(a) | Self::Not(a) | Self::Rising(a, _) | Self::Falling(a, _) => a.feature_names(out),
            Self::Arith { lhs, rhs, .. } | Self::Cmp { lhs, rhs, .. } | Self::And(lhs, rhs) | Self::Or(lhs, rhs) | Self::CrossedAbove(lhs, rhs) | Self::CrossedBelow(lhs, rhs) => {
                lhs.feature_names(out);
                rhs.feature_names(out);
            }
            Self::Between(x, a, b) => {
                x.feature_names(out);
                a.feature_names(out);
                b.feature_names(out);
            }
            Self::Num(_) | Self::Bar(_) | Self::Param(_) | Self::Input(_) => {}
        }
    }

    /// Every `input` name this expression references.
    pub fn input_names(&self, out: &mut Vec<String>) {
        match self {
            Self::Input(n) => {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
            Self::Neg(a) | Self::Not(a) | Self::Rising(a, _) | Self::Falling(a, _) => a.input_names(out),
            Self::Arith { lhs, rhs, .. } | Self::Cmp { lhs, rhs, .. } | Self::And(lhs, rhs) | Self::Or(lhs, rhs) | Self::CrossedAbove(lhs, rhs) | Self::CrossedBelow(lhs, rhs) => {
                lhs.input_names(out);
                rhs.input_names(out);
            }
            Self::Between(x, a, b) => {
                x.input_names(out);
                a.input_names(out);
                b.input_names(out);
            }
            _ => {}
        }
    }

    /// The canonical text of this expression, parenthesised by precedence only —
    /// never by where the author happened to type a bracket. Two expressions
    /// that mean the same thing therefore render the same way, which is what
    /// makes `diff_strategies` able to say a rewrite changed nothing.
    #[must_use]
    pub fn render(&self) -> String {
        let wrap = |e: &Expr, min: u8| {
            let s = e.render();
            if e.precedence() < min {
                format!("({s})")
            } else {
                s
            }
        };
        match self {
            Self::Num(n) => n.clone(),
            Self::Feature { family, params } => {
                if params.is_empty() {
                    family.clone()
                } else {
                    let args: Vec<String> = params.iter().map(usize::to_string).collect();
                    format!("{family}(close, {})", args.join(", "))
                }
            }
            Self::Bar(f) => f.clone(),
            Self::Param(p) => format!("param({p})"),
            Self::Input(n) => n.clone(),
            Self::Neg(a) => format!("-{}", wrap(a, 9)),
            Self::Arith { op, lhs, rhs } => {
                let p = op.precedence();
                // Right operand needs a tighter binding to survive `a - (b - c)`.
                format!("{} {} {}", wrap(lhs, p), op.as_str(), wrap(rhs, p + 1))
            }
            Self::Cmp { op, lhs, rhs } => format!("{} {} {}", wrap(lhs, 4), op.as_str(), wrap(rhs, 4)),
            Self::And(a, b) => format!("{} and {}", wrap(a, 2), wrap(b, 3)),
            Self::Or(a, b) => format!("{} or {}", wrap(a, 1), wrap(b, 2)),
            Self::Not(a) => format!("not {}", wrap(a, 4)),
            Self::CrossedAbove(a, b) => format!("crossed_above({}, {})", a.render(), b.render()),
            Self::CrossedBelow(a, b) => format!("crossed_below({}, {})", a.render(), b.render()),
            Self::Between(x, a, b) => format!("between({}, {}, {})", x.render(), a.render(), b.render()),
            Self::Rising(a, n) => format!("rising({}, {n})", a.render()),
            Self::Falling(a, n) => format!("falling({}, {n})", a.render()),
        }
    }
}

fn push_comments(out: &mut String, comments: &[String]) {
    for c in comments {
        let _ = writeln!(out, "# {c}");
    }
}

impl Program {
    /// The canonical text. Parsing this and rendering again returns the same
    /// bytes — that is the language's only definition of "the same strategy",
    /// and `render_parse_render_is_byte_identical` is what holds it to it.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn render(&self) -> String {
        let mut out = String::new();
        push_comments(&mut out, &self.comments);
        let _ = writeln!(out, "strategy {}", self.strategy_id);
        let _ = writeln!(out, "asset_class {}", self.asset_class);
        let _ = writeln!(out, "timeframe {}", self.timeframe);
        for p in &self.params {
            push_comments(&mut out, &p.comments);
            let _ = writeln!(out, "param {} {} {} range {} {}", p.name, p.ptype, p.default, p.min, p.max);
        }
        for b in &self.inputs {
            push_comments(&mut out, &b.comments);
            let _ = writeln!(out, "input {} = {}", b.name, b.expr.render());
        }
        for e in &self.entries {
            push_comments(&mut out, &e.comments);
            let _ = writeln!(out, "entry {} when {}", e.side.as_str(), e.when.render());
        }
        if let Some(s) = &self.size {
            push_comments(&mut out, &s.comments);
            let _ = writeln!(out, "size {} {}", s.kind, s.value);
        }
        for x in &self.exits {
            push_comments(&mut out, &x.comments);
            let line = match &x.kind {
                ExitKind::Stop(v) => format!("exit stop {v} %"),
                ExitKind::Target(v) => format!("exit target {v} %"),
                ExitKind::Trail(v) => format!("exit trail {v} %"),
                ExitKind::Time(n) => format!("exit time {n} bars"),
                ExitKind::OnSignal(s) => format!("exit on signal {s}"),
                ExitKind::RiskLayerOnly => "exit risk_layer_only".to_string(),
            };
            let _ = writeln!(out, "{line}");
        }
        for r in &self.risk {
            push_comments(&mut out, &r.comments);
            let _ = writeln!(out, "risk {} {}", r.name, r.value);
        }
        out
    }

    /// Every catalogued feature the program reads, with `input` bindings
    /// expanded, in first-seen order.
    #[must_use]
    pub fn features_used(&self) -> Vec<String> {
        let mut out = Vec::new();
        for b in &self.inputs {
            b.expr.feature_names(&mut out);
        }
        for e in &self.entries {
            e.when.feature_names(&mut out);
        }
        out
    }
}
