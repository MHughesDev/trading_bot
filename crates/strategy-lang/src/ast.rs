//! The syntax tree: what the author wrote, before anything asks whether the
//! canonical form can hold it.
//!
//! # Why the tree keeps spans and raw text
//!
//! Two consumers need them. Diagnostics need spans, so the compiler can point
//! at the clause rather than at the strategy. And the canonical form stores
//! sizes, risk limits and exit fractions as **decimal strings** (ADR-0002), so
//! a number that went through `f64` on its way in would come out a different
//! number: [`Num`] keeps the digits the author typed and the compiler moves the
//! decimal point textually.
//!
//! # Why there is one expression type and not two
//!
//! FEAT-004 §3.1 has separate `cond` and `expr` productions. Parsing them
//! separately means a model that writes `entry long when ema(close, 7)` gets
//! "unexpected token" at the end of a line that is perfectly well formed. One
//! grammar, and the compiler saying "`ema_7` is a number; compare it with
//! something" at the point of use, is the error that teaches.

use serde::{Deserialize, Serialize};

use crate::diagnostic::Span;

/// A decimal literal exactly as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Num {
    pub raw: String,
    pub span: Span,
}

/// A bar field, the one name in the language that is not a feature.
///
/// Serialised because [`crate::form`]'s fills cross a JSON boundary, and a
/// closed enum there is what stops a constrained tier writing `bar('clsoe')`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarField {
    Open,
    High,
    Low,
    Close,
    Volume,
}

impl BarField {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::High => "high",
            Self::Low => "low",
            Self::Close => "close",
            Self::Volume => "volume",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "open" => Self::Open,
            "high" => Self::High,
            "low" => Self::Low,
            "close" => Self::Close,
            "volume" => Self::Volume,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    And,
    Or,
}

impl BinOp {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Gt => ">",
            Self::Lt => "<",
            Self::Ge => ">=",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::And => "and",
            Self::Or => "or",
        }
    }

    /// Whether both operands must be conditions and the result is one.
    #[must_use]
    pub const fn is_logical(self) -> bool {
        matches!(self, Self::And | Self::Or)
    }

    /// Whether the operands are values and the result is a condition.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Gt | Self::Lt | Self::Ge | Self::Le | Self::Eq | Self::Ne
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

/// An expression. Value or condition is decided at compile time, at the point
/// of use, where the message can say what the author meant to write.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(Num),
    /// `close`, `high` — the bar the decision is made on.
    Bar {
        field: BarField,
        span: Span,
    },
    /// A bare name: a declared `input`, a declared `param`, or a state
    /// predicate. Which one it is depends on the declarations, so the parser
    /// does not guess.
    Name {
        name: String,
        span: Span,
    },
    /// `ema(close, 7)`, `crossed_above(a, b)`, `feature('ema_7')`.
    Call {
        name: String,
        args: Vec<Expr>,
        /// The whole call, `name` through `)`.
        span: Span,
        /// The span of the callee alone, for an "unknown function" caret.
        name_span: Span,
    },
    /// A `'…'` literal. Only ever an argument to `feature`, `bar` or `param` —
    /// SLv2 has no string values.
    Text {
        value: String,
        span: Span,
    },
    Unary {
        op: UnOp,
        rhs: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Number(n) => n.span,
            Self::Bar { span, .. }
            | Self::Name { span, .. }
            | Self::Call { span, .. }
            | Self::Text { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. } => *span,
        }
    }

    /// Walk every node, outermost first.
    pub fn walk(&self, f: &mut impl FnMut(&Self)) {
        f(self);
        match self {
            Self::Unary { rhs, .. } => rhs.walk(f),
            Self::Binary { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            Self::Call { args, .. } => {
                for a in args {
                    a.walk(f);
                }
            }
            Self::Number(_) | Self::Bar { .. } | Self::Name { .. } | Self::Text { .. } => {}
        }
    }
}

/// How a parameter is declared and swept.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Int {
        min: Num,
        max: Num,
        step: Option<Num>,
        default: Num,
    },
    Float {
        min: Num,
        max: Num,
        log: bool,
        default: Num,
    },
    Enum {
        choices: Vec<String>,
        default: String,
    },
}

/// A clause's attached comments, in the order they were written.
///
/// AGENT-011 §3: a threshold of 0.6 does not explain itself, and nobody reading
/// the dossier in six months can recover why it is 0.6 from the number. The
/// parser attaches each `# …` to the clause below it.
pub type Comments = Vec<String>;

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub kind: ParamKind,
    pub span: Span,
    pub comments: Comments,
}

/// `input trend = ema(close, 7)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub name: String,
    pub name_span: Span,
    pub expr: Expr,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Long,
    Short,
}

impl Side {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Long => "long",
            Self::Short => "short",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub side: Side,
    pub when: Expr,
    /// `cooldown 5 bars`. Parsed because FEAT-004 §3.1 has it; refused by the
    /// compiler, which is a better answer than a parse error for something the
    /// language really does contain.
    pub cooldown: Option<(Num, Span)>,
    pub span: Span,
    pub comments: Comments,
}

/// The unit a distance is measured in. AGENT-011 §3: "a bare number in a
/// distance is a compile error with the three legal units named".
///
/// # Why the two scaled units carry a period and `%` does not
///
/// `atr` on its own is not a measurement: the engine serves `atr_14` and
/// `atr_50`, and they are different numbers. `domain`'s
/// [`DistanceUnit::Atr { period }`] says the same thing in the schema, so a
/// grammar that parsed a bare `atr` would have nothing to put in the field and
/// would have to invent a lookback the author never wrote. The period is
/// therefore part of the unit and not an optional decoration —
/// `1.5 atr(14) from entry` — which is also why two stops at `atr(14)` and
/// `atr(50)` compare unequal here and in the differ.
///
/// `%` has no period because a fraction of the entry price is a fraction of one
/// number, measured on no window at all.
///
/// [`DistanceUnit::Atr { period }`]: domain::strategy_def::exits::DistanceUnit::Atr
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Atr { period: u32 },
    Sigma { period: u32 },
}

impl Unit {
    /// The keyword, without the period. What a diagnostic's `expected` list
    /// holds, because the keyword is what the author types first.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Percent => "%",
            Self::Atr { .. } => "atr",
            Self::Sigma { .. } => "sigma",
        }
    }

    /// The unit as it is written in SLv2, period and all: `%`, `atr(14)`.
    ///
    /// This — not [`Self::as_str`] — is what the decompiler emits and what the
    /// differ compares, because `atr(14)` and `atr(50)` are two units and
    /// `as_str` cannot tell them apart.
    #[must_use]
    pub fn render(self) -> String {
        match self {
            Self::Percent => "%".to_string(),
            Self::Atr { period } => format!("atr({period})"),
            Self::Sigma { period } => format!("sigma({period})"),
        }
    }

    /// The lookback, for the two units that have one.
    #[must_use]
    pub const fn period(self) -> Option<u32> {
        match self {
            Self::Percent => None,
            Self::Atr { period } | Self::Sigma { period } => Some(period),
        }
    }

    /// The three keywords, in the order the guide lists them.
    ///
    /// Words rather than values: a `Unit` value now needs a period, and a
    /// placeholder period inside the list a *missing unit* diagnostic prints
    /// would be a number the author never wrote appearing in the advice.
    pub const WORDS: [&'static str; 3] = ["%", "atr", "sigma"];
}

/// What a distance is measured from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Entry,
    HighWater,
}

impl Anchor {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::HighWater => "high_water",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Distance {
    pub value: Num,
    pub unit: Unit,
    pub unit_span: Span,
    pub anchor: Anchor,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExitKind {
    /// `exit none` — the risk layer is the whole exit logic, said out loud.
    None,
    Stop(Distance),
    Target(Distance),
    Trailing(Distance),
    Time(Num),
    When(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exit {
    pub kind: ExitKind,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SizeKind {
    Fixed(Num),
    EquityFraction(Num),
    RiskPerTrade(Num),
    /// `vol_target`, `kelly` and the rest of FEAT-004 §3.1's list, kept as the
    /// word the author wrote so the refusal can quote it back.
    Other {
        word: String,
        rest: Option<Num>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SizeClause {
    pub kind: SizeKind,
    pub span: Span,
    pub comments: Comments,
}

/// The order type an `order` clause names (FEAT-004 §3.1).
///
/// The canonical [`domain::strategy_def::actions::OrderSpec`] carries a side, a
/// size mode and a size — **there is no order-type field at all** — so the only
/// one of these the platform can store is [`OrderType::Market`], which FEAT-004
/// also names as the default. The other three are parsed so the compiler can
/// refuse them *where they are written*, which is the same treatment `cooldown`
/// and `max_daily_loss` get: a clause the spec lists and the schema cannot hold
/// is worth a named refusal rather than an `unknown_clause`, because the author
/// of `order limit offset 0.1 %` has not made a spelling mistake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
}

impl OrderType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Limit => "limit",
            Self::Stop => "stop",
            Self::StopLimit => "stop_limit",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "market" => Self::Market,
            "limit" => Self::Limit,
            "stop" => Self::Stop,
            "stop_limit" => Self::StopLimit,
            _ => return None,
        })
    }

    pub const ALL: [Self; 4] = [Self::Market, Self::Limit, Self::Stop, Self::StopLimit];
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `order limit offset 0.1 % tif gtc` — FEAT-004 §3.1's optional order clause.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderClause {
    pub order_type: OrderType,
    /// The type word itself, so a refusal points at `limit` rather than at the
    /// whole line.
    pub type_span: Span,
    /// `offset 0.1 %`, the distance a limit or stop price sits from the
    /// reference. Kept with its own span for the same reason.
    pub offset: Option<(Num, Span)>,
    /// `tif gtc` — the word as written, so the refusal can quote it back.
    pub tif: Option<(String, Span)>,
    pub span: Span,
    pub comments: Comments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskKind {
    MaxPosition,
    MaxOrdersPerMin,
    MaxOrdersPerSec,
    /// FEAT-004 §3.1 lists it; `RiskOverrides` has no field for it.
    MaxDailyLoss,
}

impl RiskKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxPosition => "max_position",
            Self::MaxOrdersPerMin => "max_orders_per_min",
            Self::MaxOrdersPerSec => "max_orders_per_sec",
            Self::MaxDailyLoss => "max_daily_loss",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "max_position" => Self::MaxPosition,
            "max_orders_per_min" => Self::MaxOrdersPerMin,
            "max_orders_per_sec" => Self::MaxOrdersPerSec,
            "max_daily_loss" => Self::MaxDailyLoss,
            _ => return None,
        })
    }

    pub const ALL: [Self; 4] = [
        Self::MaxPosition,
        Self::MaxOrdersPerMin,
        Self::MaxOrdersPerSec,
        Self::MaxDailyLoss,
    ];
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiskItem {
    pub kind: RiskKind,
    pub value: Num,
    pub span: Span,
    pub comments: Comments,
}

/// A cross-parameter constraint, kept as the text between `constraint` and the
/// end of the line. `domain::strategy_def` stores it as a string and parses it
/// itself, so re-rendering it here would be a second grammar for the same thing.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstraintClause {
    pub text: String,
    pub span: Span,
    pub comments: Comments,
}

/// One parsed strategy.
#[derive(Clone, Debug, PartialEq)]
pub struct Strategy {
    pub name: String,
    pub name_span: Span,
    pub instrument: String,
    pub timeframe: String,
    pub header_span: Span,
    pub header_comments: Comments,
    /// `version 1.2`. Absent means the current version, which is the one that
    /// requires exits.
    pub version: Option<(String, Span)>,
    pub asset_class: Option<(String, Span)>,
    pub trust: Option<(String, Span)>,
    pub params: Vec<ParamDecl>,
    pub inputs: Vec<Binding>,
    pub entries: Vec<Entry>,
    pub size: Option<SizeClause>,
    pub exits: Vec<Exit>,
    /// `order …`. Parsed so the compiler can refuse a type the canonical
    /// `OrderSpec` has no field for by name; see [`OrderType`].
    pub orders: Vec<OrderClause>,
    pub risk: Vec<RiskItem>,
    pub constraints: Vec<ConstraintClause>,
}
