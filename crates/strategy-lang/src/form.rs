//! The clause-level grammar a `select_fill` tier writes SLv2 through (S-5.9;
//! AGENT-011 §3, P7).
//!
//! # One clause per fill, and one compiler
//!
//! AGENT-011's argument is that a small model cannot author freely but can fill
//! in a form: "a `select_fill` tier is given a JSON-Schema-shaped grammar over
//! these clauses (one clause per fill), not the EBNF, and the compiler
//! assembles the text". So this module is a set of typed clause fills, a JSON
//! Schema over them for a constrained decoder, and one function that renders
//! them as SLv2.
//!
//! **What it is not is a second compiler.** [`FormStrategy::compile`] is
//! [`FormStrategy::to_text`] followed by [`crate::text_to_definition`] — the
//! same lexer, the same parser, the same compiler, the same diagnostics that a
//! frontier tier's free-form text goes through. Two compilers would mean a
//! strategy that validates in the form and not in the language, and the tier
//! that could least afford to debug it would be the one that hit it.
//! `the_form_compiles_through_the_language_and_not_beside_it` pins the
//! equality.
//!
//! # Why the schema enumerates families one at a time
//!
//! §8.7 asks a 7B to produce a compiled strategy "in ≤ 6 fills with no
//! `unknown_feature` error". A schema whose feature name is a free string
//! satisfies the letter of "grammar-constrained" and leaves that error exactly
//! as reachable as it was in free text — the free-form-string shape this
//! repository keeps finding. [`document_schema`] instead emits one branch per
//! family, read from [`crate::Catalogue`] (which reads
//! `features::registry`), each carrying its own arity and each parameter's
//! range. A decoder constrained by it cannot name a feature the engine does not
//! serve, cannot pass the wrong number of parameters, and cannot pass one out of
//! range. `every_family_in_the_schema_compiles_when_the_form_uses_it` walks
//! every branch and compiles a strategy through it.
//!
//! # Where the form is narrower than the language, it says so by name
//!
//! The form deliberately cannot say everything SLv2 can. A form that quietly
//! dropped the clause it could not hold would hand back a strategy that is not
//! the one it was given, and nothing downstream would know. So
//! [`FormStrategy::from_strategy`] — the projection of free-form SLv2 into the
//! form — returns a [`Refusal`] carrying a typed [`Limitation`], and
//! [`Limitation::all`] is a closed list, each entry carrying an SLv2 document
//! that exercises it and a [`Reach`] saying what the *language* does with that
//! document: either the free form compiles it (the form is strictly narrower)
//! or the compiler refuses it too, by the named code (the form refuses earlier
//! and for the same reason). `every_limitation_is_real_and_reachable` checks
//! both halves against the live compiler, so an entry that stopped being true
//! fails the build rather than misleading a reader.

use std::collections::{BTreeMap, BTreeSet};

use domain::strategy_def::StrategyDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ast::{self, BarField, Side, Strategy};
use crate::catalogue::Catalogue;
use crate::diagnostic::{Code, Diagnostic, Span};

// ── the non-empty list ───────────────────────────────────────────────────────

/// A list that cannot be empty, by construction rather than by a check.
///
/// The same shape as [`crate::diagnostic::Expected`], for the same reason: an
/// entry with no filters, or a strategy with no exits, would render as text the
/// parser refuses, and the fill that produced it would have looked valid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    into = "Vec<T>",
    try_from = "Vec<T>",
    bound = "T: Clone + Serialize + serde::de::DeserializeOwned"
)]
pub struct NonEmpty<T> {
    first: T,
    rest: Vec<T>,
}

impl<T> NonEmpty<T> {
    pub fn new(first: T, rest: Vec<T>) -> Self {
        Self { first, rest }
    }

    /// `None` when `items` is empty.
    pub fn any(items: impl IntoIterator<Item = T>) -> Option<Self> {
        let mut it = items.into_iter();
        let first = it.next()?;
        Some(Self {
            first,
            rest: it.collect(),
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.first).chain(self.rest.iter())
    }

    /// Edit in place. There is deliberately no `push`, `remove` or `retain`:
    /// this type's whole job is that the collection cannot become empty, and a
    /// `remove` would hand that guarantee back to the caller's arithmetic.
    /// Adding or dropping a member goes through [`Self::any`], which refuses
    /// the empty case where it happens (`crate::apply` is the caller that
    /// does).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        std::iter::once(&mut self.first).chain(self.rest.iter_mut())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        1 + self.rest.len()
    }

    /// Never true. Present because clippy asks for it beside `len`, and it
    /// documents the type's whole point.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }
}

impl<T> From<NonEmpty<T>> for Vec<T> {
    fn from(n: NonEmpty<T>) -> Self {
        std::iter::once(n.first).chain(n.rest).collect()
    }
}

impl<T> TryFrom<Vec<T>> for NonEmpty<T> {
    type Error = &'static str;

    fn try_from(v: Vec<T>) -> Result<Self, Self::Error> {
        Self::any(v).ok_or("this list must name at least one item")
    }
}

// ── the fills ────────────────────────────────────────────────────────────────

/// `strategy <name> on <instrument> timeframe <tf>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    /// `$bound_at_init`, or an instrument such as `BTC-USD`.
    pub instrument: String,
    pub timeframe: String,
}

/// An argument to a feature family: a whole number of bars, or the name of a
/// declared parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Bars(i64),
    Param(String),
}

/// A value a filter can compare. Plain values only — see [`Limitation::ComposedOperand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operand", rename_all = "snake_case")]
pub enum Operand {
    Feature {
        family: String,
        #[serde(default)]
        args: Vec<Arg>,
    },
    Bar {
        field: BarField,
    },
    Input {
        name: String,
    },
    Param {
        name: String,
    },
    /// Kept as the digits the fill wrote: the canonical form stores sizes and
    /// fractions as decimal strings (ADR-0002), and a round trip through `f64`
    /// is a different number.
    Number {
        value: String,
    },
}

/// The six comparisons. A closed set, so an arithmetic operator cannot be
/// written into a filter at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

impl Compare {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Lt => "<",
            Self::Ge => ">=",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }

    pub const ALL: [Self; 6] = [Self::Gt, Self::Lt, Self::Ge, Self::Le, Self::Eq, Self::Ne];

    #[must_use]
    pub fn from_bin_op(op: ast::BinOp) -> Option<Self> {
        Some(match op {
            ast::BinOp::Gt => Self::Gt,
            ast::BinOp::Lt => Self::Lt,
            ast::BinOp::Ge => Self::Ge,
            ast::BinOp::Le => Self::Le,
            ast::BinOp::Eq => Self::Eq,
            ast::BinOp::Ne => Self::Ne,
            _ => return None,
        })
    }
}

/// One clause of a condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "filter", rename_all = "snake_case")]
pub enum Filter {
    Compare {
        lhs: Operand,
        op: Compare,
        rhs: Operand,
    },
    /// `between(x, low, high)`, inclusive at both ends.
    Between {
        value: Operand,
        low: Operand,
        high: Operand,
    },
}

/// How a condition's filters join. One level, no nesting — see
/// [`Limitation::NestedCondition`].
///
/// With one filter the two are the same condition and render as the same text,
/// so the projection back from SLv2 reports [`Combine::All`] whatever the fill
/// said. `a_single_filter_normalises_its_combine` pins that, because it is the
/// one place a form does not come back exactly as it went in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    /// Every filter must hold (`and`).
    All,
    /// Any filter may hold (`or`).
    Any,
}

impl Combine {
    const fn joiner(self) -> &'static str {
        match self {
            Self::All => " and ",
            Self::Any => " or ",
        }
    }
}

/// `entry long when …`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub side: Side,
    pub combine: Combine,
    pub filters: NonEmpty<Filter>,
}

/// `param name: int[…] = …`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Param {
    Int {
        name: String,
        min: i64,
        max: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<i64>,
        default: i64,
    },
    Float {
        name: String,
        min: f64,
        max: f64,
        #[serde(default)]
        log: bool,
        default: f64,
    },
    Enum {
        name: String,
        choices: NonEmpty<String>,
        default: String,
    },
}

impl Param {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Int { name, .. } | Self::Float { name, .. } | Self::Enum { name, .. } => name,
        }
    }
}

/// `constraint param('fast') < param('slow')`.
///
/// Structured rather than a line of text. `domain::strategy_def::params`
/// parses a constraint as `operand OP operand` where an operand is
/// `param('name')` or a number, and this pair of types covers that grammar
/// exactly — so a fill cannot write a constraint the sweeper will later reject,
/// which is what a free-form string here would have allowed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "side", rename_all = "snake_case")]
pub enum ConstraintOperand {
    Param { name: String },
    Number { value: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub lhs: ConstraintOperand,
    pub op: Compare,
    pub rhs: ConstraintOperand,
}

/// `input name = family(close, …)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub name: String,
    pub family: String,
    #[serde(default)]
    pub args: Vec<Arg>,
}

/// `size …`. The three modes the canonical order specification has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "size", rename_all = "snake_case")]
pub enum Size {
    Fixed { quantity: String },
    EquityFraction { fraction: String },
    RiskPerTrade { fraction: String },
}

/// `exit …`. Distances are in percent of entry only.
///
/// Not because the platform cannot hold another unit — since ADR-D-09 it can,
/// and `exit stop 1.5 atr(14) from entry` compiles as text — but because a fill
/// carrying a unit and a lookback is three decisions in one clause for the tier
/// this grammar exists to serve. [`Limitation::NonPercentDistance`] says so, and
/// says the language is the way out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "exit", rename_all = "snake_case")]
pub enum Exit {
    /// Delegate to the risk layer, on purpose and out loud.
    None,
    Stop {
        percent: String,
    },
    Target {
        percent: String,
    },
    Trailing {
        percent: String,
    },
    Time {
        bars: u32,
    },
    When {
        combine: Combine,
        filters: NonEmpty<Filter>,
    },
}

/// `risk …`. The limits `RiskOverrides` has fields for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "risk", rename_all = "snake_case")]
pub enum Risk {
    MaxPosition { value: String },
    MaxOrdersPerMin { orders: u32 },
    MaxOrdersPerSec { orders: u32 },
}

/// A whole strategy, one clause per fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormStrategy {
    pub header: Header,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Constraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Input>,
    pub entries: NonEmpty<Entry>,
    pub size: Size,
    pub exits: NonEmpty<Exit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk: Vec<Risk>,
}

// ── rendering ────────────────────────────────────────────────────────────────

fn render_feature(family: &str, args: &[Arg]) -> String {
    if args.is_empty() {
        return format!("{family}()");
    }
    let mut out = format!("{family}(close");
    for a in args {
        match a {
            Arg::Bars(n) => out.push_str(&format!(", {n}")),
            Arg::Param(p) => out.push_str(&format!(", {p}")),
        }
    }
    out.push(')');
    out
}

/// Feature call as written → the input name standing for it.
///
/// AGENT-011 §3 makes `inputs` mandatory for anything used twice, and
/// `parse::used_twice` enforces it on the text. That rule is about the
/// *document*, and a clause grammar fills one clause at a time with no view of
/// the others — so two independently reasonable fills can name the same
/// feature, and the assembled text would be refused after both were spent. The
/// same happens when one fill declares `input trend = ema(close, 7)` and
/// another writes `ema(close, 7)` inline.
///
/// So the assembler binds instead of failing: a call that matches a declared
/// input is written as that input's name, and a call written twice is hoisted
/// into a generated one. This changes nothing about what the strategy means —
/// it is the substitution `decompile` already performs in the other direction —
/// and it is what stops the form emitting text its own parser rejects.
type Bindings = BTreeMap<String, String>;

fn render_operand(o: &Operand, bound: &Bindings) -> String {
    match o {
        Operand::Feature { family, args } => {
            let call = render_feature(family, args);
            bound.get(&call).cloned().unwrap_or(call)
        }
        Operand::Bar { field } => field.as_str().to_string(),
        // A parameter reads as its bare name, which is what a feature argument
        // must be and what an operand may be — one spelling, so the decompiled
        // form of a form-written strategy looks like the form wrote it.
        Operand::Input { name } | Operand::Param { name } => name.clone(),
        Operand::Number { value } => value.clone(),
    }
}

fn render_filter(f: &Filter, bound: &Bindings) -> String {
    match f {
        Filter::Compare { lhs, op, rhs } => format!(
            "{} {} {}",
            render_operand(lhs, bound),
            op.as_str(),
            render_operand(rhs, bound)
        ),
        Filter::Between { value, low, high } => format!(
            "between({}, {}, {})",
            render_operand(value, bound),
            render_operand(low, bound),
            render_operand(high, bound)
        ),
    }
}

fn render_condition(combine: Combine, filters: &NonEmpty<Filter>, bound: &Bindings) -> String {
    filters
        .iter()
        .map(|f| render_filter(f, bound))
        .collect::<Vec<_>>()
        .join(combine.joiner())
}

/// An identifier for a generated binding: the feature name with its template
/// braces dropped, which is already unique per call.
fn binding_name(family: &str, args: &[Arg]) -> String {
    let mut s = family.to_string();
    for a in args {
        s.push('_');
        match a {
            Arg::Bars(n) => s.push_str(&n.to_string()),
            Arg::Param(p) => s.push_str(p),
        }
    }
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn render_constraint_operand(o: &ConstraintOperand) -> String {
    match o {
        ConstraintOperand::Param { name } => format!("param('{name}')"),
        ConstraintOperand::Number { value } => value.clone(),
    }
}

impl FormStrategy {
    /// Every filter in the document, entries and signal exits alike.
    fn filters(&self) -> impl Iterator<Item = &Filter> {
        self.entries
            .iter()
            .flat_map(|e| e.filters.iter())
            .chain(self.exits.iter().flat_map(|x| match x {
                Exit::When { filters, .. } => filters.iter().collect::<Vec<_>>(),
                _ => Vec::new(),
            }))
    }

    /// The substitutions [`Bindings`] describes, plus the bindings that have to
    /// be written out because nothing declared them.
    fn bindings(&self) -> (Bindings, Vec<(String, String)>) {
        let mut bound: Bindings = BTreeMap::new();
        for i in &self.inputs {
            bound
                .entry(render_feature(&i.family, &i.args))
                .or_insert_with(|| i.name.clone());
        }
        let mut counts: BTreeMap<String, (usize, &str, &[Arg])> = BTreeMap::new();
        for f in self.filters() {
            let operands: Vec<&Operand> = match f {
                Filter::Compare { lhs, rhs, .. } => vec![lhs, rhs],
                Filter::Between { value, low, high } => vec![value, low, high],
            };
            for o in operands {
                if let Operand::Feature { family, args } = o {
                    let e = counts
                        .entry(render_feature(family, args))
                        .or_insert((0, family, args));
                    e.0 += 1;
                }
            }
        }
        let mut taken: BTreeSet<String> = self
            .params
            .iter()
            .map(|p| p.name().to_string())
            .chain(self.inputs.iter().map(|i| i.name.clone()))
            .collect();
        let mut generated = Vec::new();
        for (call, (n, family, args)) in counts {
            if n < 2 || bound.contains_key(&call) {
                continue;
            }
            let base = binding_name(family, args);
            let mut name = base.clone();
            let mut suffix = 2;
            while !taken.insert(name.clone()) {
                name = format!("{base}_{suffix}");
                suffix += 1;
            }
            generated.push((name.clone(), call.clone()));
            bound.insert(call, name);
        }
        (bound, generated)
    }

    /// The SLv2 text these fills describe.
    #[must_use]
    pub fn to_text(&self) -> String {
        let (bound, generated) = self.bindings();
        let mut s = String::new();
        s.push_str(&format!(
            "strategy {} on {} timeframe {}\n",
            self.header.name,
            crate::decompile::render_instrument(&self.header.instrument),
            self.header.timeframe
        ));
        for p in &self.params {
            s.push_str(&render_param(p));
        }
        for c in &self.constraints {
            s.push_str(&format!(
                "constraint {} {} {}\n",
                render_constraint_operand(&c.lhs),
                c.op.as_str(),
                render_constraint_operand(&c.rhs)
            ));
        }
        for i in &self.inputs {
            s.push_str(&format!(
                "input {} = {}\n",
                i.name,
                render_feature(&i.family, &i.args)
            ));
        }
        for (name, call) in &generated {
            s.push_str(&format!("input {name} = {call}\n"));
        }
        for e in self.entries.iter() {
            s.push_str(&format!(
                "entry {} when {}\n",
                e.side.as_str(),
                render_condition(e.combine, &e.filters, &bound)
            ));
        }
        s.push_str(&match &self.size {
            Size::Fixed { quantity } => format!("size fixed {quantity}\n"),
            Size::EquityFraction { fraction } => format!("size equity_fraction {fraction}\n"),
            Size::RiskPerTrade { fraction } => format!("size risk_per_trade {fraction}\n"),
        });
        for x in self.exits.iter() {
            s.push_str(&match x {
                Exit::None => "exit none\n".to_string(),
                Exit::Stop { percent } => format!("exit stop {percent} % from entry\n"),
                Exit::Target { percent } => format!("exit target {percent} % from entry\n"),
                Exit::Trailing { percent } => {
                    format!("exit trailing {percent} % from high_water\n")
                }
                Exit::Time { bars } => format!("exit time {bars} bars\n"),
                Exit::When { combine, filters } => {
                    format!(
                        "exit when {}\n",
                        render_condition(*combine, filters, &bound)
                    )
                }
            });
        }
        for r in &self.risk {
            s.push_str(&match r {
                Risk::MaxPosition { value } => format!("risk max_position {value}\n"),
                Risk::MaxOrdersPerMin { orders } => {
                    format!("risk max_orders_per_min {orders}\n")
                }
                Risk::MaxOrdersPerSec { orders } => {
                    format!("risk max_orders_per_sec {orders}\n")
                }
            });
        }
        s
    }

    /// The canonical definition, **through the language**.
    ///
    /// # Errors
    ///
    /// Whatever [`crate::text_to_definition`] says about the assembled text —
    /// the same diagnostics a frontier tier's own text would have produced.
    pub fn compile(&self) -> Result<StrategyDefinition, Vec<Diagnostic>> {
        crate::text_to_definition(&self.to_text())
    }

    /// How many fills this strategy took: one per clause (AGENT-011 §3).
    #[must_use]
    pub fn fills(&self) -> usize {
        1 + self.params.len()
            + self.constraints.len()
            + self.inputs.len()
            + self.entries.len()
            + 1
            + self.exits.len()
            + self.risk.len()
    }

    /// Project free-form SLv2 into the form.
    ///
    /// # Errors
    ///
    /// One [`Refusal`] per thing the form cannot hold, each naming a
    /// [`Limitation`]. Never a silent drop: a clause the form has no fill for
    /// is refused, because a form that returned a strategy missing a clause
    /// would be returning a different strategy.
    pub fn from_strategy(ast: &Strategy) -> Result<Self, Vec<Refusal>> {
        from_strategy_with(ast, &Catalogue::engine())
    }
}

fn render_param(p: &Param) -> String {
    match p {
        Param::Int {
            name,
            min,
            max,
            step,
            default,
        } => {
            let step = match step {
                Some(s) if *s != 1 => format!(" step {s}"),
                _ => String::new(),
            };
            format!("param {name}: int[{min}..{max}{step}] = {default}\n")
        }
        Param::Float {
            name,
            min,
            max,
            log,
            default,
        } => {
            let log = if *log { " log" } else { "" };
            format!("param {name}: float[{min}..{max}{log}] = {default}\n")
        }
        Param::Enum {
            name,
            choices,
            default,
        } => format!(
            "param {name}: enum[{}] = {default}\n",
            choices.iter().cloned().collect::<Vec<_>>().join(", ")
        ),
    }
}

// ── what the form cannot say ─────────────────────────────────────────────────

/// What the *language* does with a document the form cannot hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The free form compiles it: the form is strictly narrower here, and a
    /// model that needs it must write text.
    Compiles,
    /// The compiler refuses it too, by this code. The form refuses earlier and
    /// for the same reason, so writing it as text will not help.
    RefusedByTheCompiler(Code),
}

/// A thing SLv2 can say and the clause grammar cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Limitation {
    ComposedOperand,
    Negation,
    NestedCondition,
    DefinitionVersion,
    AssetClass,
    TrustTier,
    RawFeatureName,
    IncompleteDocument,
    Cooldown,
    NonPercentDistance,
    ExitAnchor,
    SizeModeBeyondTheThree,
    MaxDailyLoss,
    OrderType,
}

impl Limitation {
    pub const ALL: [Self; 14] = [
        Self::ComposedOperand,
        Self::Negation,
        Self::NestedCondition,
        Self::DefinitionVersion,
        Self::AssetClass,
        Self::TrustTier,
        Self::RawFeatureName,
        Self::IncompleteDocument,
        Self::Cooldown,
        Self::NonPercentDistance,
        Self::ExitAnchor,
        Self::SizeModeBeyondTheThree,
        Self::MaxDailyLoss,
        Self::OrderType,
    ];

    /// Every limitation, for a tool that lists what the form cannot do.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &Self::ALL
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ComposedOperand => "composed_operand",
            Self::Negation => "negation",
            Self::NestedCondition => "nested_condition",
            Self::DefinitionVersion => "definition_version",
            Self::AssetClass => "asset_class",
            Self::TrustTier => "trust_tier",
            Self::RawFeatureName => "raw_feature_name",
            Self::IncompleteDocument => "incomplete_document",
            Self::Cooldown => "cooldown",
            Self::NonPercentDistance => "non_percent_distance",
            Self::ExitAnchor => "exit_anchor",
            Self::SizeModeBeyondTheThree => "size_mode_beyond_the_three",
            Self::MaxDailyLoss => "max_daily_loss",
            Self::OrderType => "order_type",
        }
    }

    /// Why the form has no fill for it.
    #[must_use]
    pub const fn why(self) -> &'static str {
        match self {
            Self::ComposedOperand => {
                "a filter compares plain values — a feature, a bar field, an input, a \
                 parameter or a number. An expression built out of them has no fill"
            }
            Self::Negation => {
                "the form has no `not`: a condition is a list of filters joined by one \
                 `and` or one `or`"
            }
            Self::NestedCondition => {
                "the form joins filters at one level, so a condition mixing `and` and `or` \
                 has no fill"
            }
            Self::DefinitionVersion => {
                "the form writes documents at the current definition version, which is the \
                 one that requires exits"
            }
            Self::AssetClass => {
                "the form does not set the asset class, so a strategy takes the platform \
                 default"
            }
            Self::TrustTier => {
                "the form does not set the minimum trust tier, so a strategy takes the \
                 platform default"
            }
            Self::RawFeatureName => {
                "the form names a feature by its family and parameters, which the schema \
                 enumerates from the engine registry. A raw `feature('…')` name the engine \
                 does not serve cannot be written"
            }
            Self::IncompleteDocument => {
                "the form fills a whole strategy: an entry, a size and an exit are each \
                 required, and a document missing one of them has no form"
            }
            Self::Cooldown => {
                "nothing between the signal node and the risk gate counts bars since the \
                 last exit"
            }
            Self::NonPercentDistance => {
                "the form's exit fills carry a `percent` and nothing else. The *language* \
                 writes a per-bar unit — `1.5 atr(14)` — since ADR-D-09 gave the schema a \
                 `unit` and the grammar a period; this narrowing is the form's, not the \
                 platform's"
            }
            Self::ExitAnchor => {
                "a stop and a target are measured from the entry and a trailing stop from \
                 the high water mark; the canonical rule has no other anchor"
            }
            Self::SizeModeBeyondTheThree => {
                "the canonical order specification has three size modes — `fixed`, \
                 `equity_fraction` and `risk_per_trade`"
            }
            Self::MaxDailyLoss => "`RiskOverrides` carries position size and order rate only",
            Self::OrderType => {
                "the canonical order specification carries a side, a size mode and a size, and nothing that names an order type, an offset or a time-in-force"
            }
        }
    }

    /// What to do instead.
    #[must_use]
    pub const fn instead(self) -> &'static str {
        match self {
            Self::ComposedOperand => {
                "find a feature family that computes it — `list_features(query=\"…\")` — or \
                 write the strategy as SLv2 text"
            }
            Self::Negation => "invert the comparison: `not (x > 70)` is `x <= 70`",
            Self::NestedCondition => {
                "split it into two `entry` clauses on the same side, which the compiler \
                 joins as an `or` over one position"
            }
            Self::DefinitionVersion => "write the strategy as SLv2 text",
            Self::AssetClass => "write the strategy as SLv2 text",
            Self::TrustTier => "write the strategy as SLv2 text",
            Self::RawFeatureName => {
                "name the family and its parameters; `list_features(query=\"…\")` for what \
                 exists"
            }
            Self::IncompleteDocument => {
                "fill the missing clause; `exit none` delegates the exits on purpose"
            }
            Self::Cooldown => {
                "drop it, or express the same idea as a filter on a feature the engine \
                 computes"
            }
            Self::NonPercentDistance => {
                "write the distance in `%` of entry, or write the strategy as SLv2 text, \
                 where `exit stop 1.5 atr(14) from entry` compiles"
            }
            Self::ExitAnchor => {
                "write `from entry` on a stop or a target, `from high_water` on a trailing \
                 stop"
            }
            Self::SizeModeBeyondTheThree => "choose one of the three",
            Self::MaxDailyLoss => {
                "set a daily loss limit in the platform risk configuration; a strategy \
                 override may only tighten what exists there"
            }
            Self::OrderType => {
                "drop the clause — every entry is a market order at the close of the bar that emitted it"
            }
        }
    }

    /// What the language does with [`Limitation::example`].
    #[must_use]
    pub const fn reach(self) -> Reach {
        match self {
            Self::ComposedOperand
            | Self::Negation
            | Self::NestedCondition
            | Self::DefinitionVersion
            | Self::AssetClass
            | Self::TrustTier
            | Self::RawFeatureName
            | Self::IncompleteDocument
            // Was `RefusedByTheCompiler(UnitNotRepresentable)`. ADR-D-09 gave
            // `ExitDistance` a `unit` and the grammar gained the period it
            // needs, so a scaled distance now compiles and this is a narrowing
            // of the *form* alone. Writing it as text is the answer, which is
            // what `Reach::Compiles` tells the caller.
            | Self::NonPercentDistance => Reach::Compiles,
            Self::Cooldown => Reach::RefusedByTheCompiler(Code::ClauseNotRepresentable),
            Self::ExitAnchor => Reach::RefusedByTheCompiler(Code::NotExpressible),
            Self::SizeModeBeyondTheThree => {
                Reach::RefusedByTheCompiler(Code::SizeModeNotRepresentable)
            }
            Self::MaxDailyLoss => Reach::RefusedByTheCompiler(Code::RiskItemNotRepresentable),
            // Was unreachable: all four order types hit `unknown_clause` in the
            // parser, so the form had nothing to be narrower *than*. The
            // language now parses the clause and refuses the three types the
            // schema cannot hold, which is what makes this a real entry.
            Self::OrderType => Reach::RefusedByTheCompiler(Code::ClauseNotRepresentable),
        }
    }

    /// A whole SLv2 document that exercises this limitation and nothing else.
    #[must_use]
    pub const fn example(self) -> &'static str {
        match self {
            Self::ComposedOperand => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when high - low > close * 0.01
size fixed 1
exit time 3 bars
"
            }
            Self::Negation => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when not (rsi(close, 14) > 70)
size fixed 1
exit time 3 bars
"
            }
            Self::NestedCondition => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when (close > 1 and volume > 2) or close < 0
size fixed 1
exit time 3 bars
"
            }
            Self::DefinitionVersion => {
                "\
strategy s on $bound_at_init timeframe 1h
version 1.0
entry long when close > 1
size fixed 1
exit time 3 bars
"
            }
            Self::AssetClass => {
                "\
strategy s on $bound_at_init timeframe 1h
asset_class equity
entry long when close > 1
size fixed 1
exit time 3 bars
"
            }
            Self::TrustTier => {
                "\
strategy s on $bound_at_init timeframe 1h
trust regulated
entry long when close > 1
size fixed 1
exit time 3 bars
"
            }
            Self::RawFeatureName => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when feature('frobnicate_12') > 0
size fixed 1
exit time 3 bars
"
            }
            Self::IncompleteDocument => {
                "\
strategy s on $bound_at_init timeframe 1h
size fixed 1
exit none
"
            }
            Self::Cooldown => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1 cooldown 5 bars
size fixed 1
exit time 3 bars
"
            }
            Self::NonPercentDistance => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit stop 1.5 atr(14) from entry
"
            }
            Self::ExitAnchor => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit stop 2 % from high_water
"
            }
            Self::SizeModeBeyondTheThree => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size vol_target 0.15
exit time 3 bars
"
            }
            Self::MaxDailyLoss => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
exit time 3 bars
risk max_daily_loss 100
"
            }
            Self::OrderType => {
                "\
strategy s on $bound_at_init timeframe 1h
entry long when close > 1
size fixed 1
order limit offset 0.1 %
exit time 3 bars
"
            }
        }
    }
}

impl std::fmt::Display for Limitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing the form could not hold, and the diagnostic that says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub limitation: Limitation,
    pub diagnostic: Diagnostic,
}

impl Refusal {
    fn new(limitation: Limitation, span: Span, what: impl std::fmt::Display) -> Self {
        Self {
            limitation,
            diagnostic: Diagnostic::semantic(
                Code::NotInTheForm,
                span,
                format!(
                    "the clause grammar has no fill for {what}: {}",
                    limitation.why()
                ),
            )
            .with_fix(limitation.instead())
            .with_see("guide#form"),
        }
    }
}

// ── free-form SLv2 → the form ────────────────────────────────────────────────

/// As [`FormStrategy::from_strategy`], against a caller-supplied catalogue.
///
/// # Errors
///
/// As [`FormStrategy::from_strategy`].
pub fn from_strategy_with(ast: &Strategy, cat: &Catalogue) -> Result<FormStrategy, Vec<Refusal>> {
    let mut p = Projector {
        cat,
        inputs: ast.inputs.iter().map(|b| b.name.clone()).collect(),
        params: ast.params.iter().map(|d| d.name.clone()).collect(),
        refusals: Vec::new(),
    };
    let out = p.run(ast);
    if p.refusals.is_empty() {
        Ok(out.expect("no refusals implies a form"))
    } else {
        Err(p.refusals)
    }
}

struct Projector<'a> {
    cat: &'a Catalogue,
    inputs: BTreeSet<String>,
    params: BTreeSet<String>,
    refusals: Vec<Refusal>,
}

impl Projector<'_> {
    fn refuse(&mut self, limitation: Limitation, span: Span, what: impl std::fmt::Display) {
        self.refusals.push(Refusal::new(limitation, span, what));
    }

    fn run(&mut self, ast: &Strategy) -> Option<FormStrategy> {
        if let Some((_, span)) = &ast.version {
            self.refuse(Limitation::DefinitionVersion, *span, "a `version` clause");
        }
        if let Some((_, span)) = &ast.asset_class {
            self.refuse(Limitation::AssetClass, *span, "an `asset_class` clause");
        }
        if let Some((_, span)) = &ast.trust {
            self.refuse(Limitation::TrustTier, *span, "a `trust` clause");
        }
        // Refused rather than dropped, including `order market`. A projection
        // that silently discarded the clause would hand back a form whose
        // `to_text()` is a document the author did not write, which is the
        // "decompiler that loses a clause" failure in the other direction.
        if let Some(order) = ast.orders.first() {
            self.refuse(Limitation::OrderType, order.span, "an `order` clause");
        }

        let params: Vec<Param> = ast.params.iter().filter_map(|d| self.param(d)).collect();
        let constraints: Vec<Constraint> = ast
            .constraints
            .iter()
            .filter_map(|c| self.constraint(c))
            .collect();
        let inputs: Vec<Input> = ast.inputs.iter().filter_map(|b| self.input(b)).collect();
        let entries: Vec<Entry> = ast.entries.iter().filter_map(|e| self.entry(e)).collect();
        let size = self.size(ast);
        let exits: Vec<Exit> = ast.exits.iter().filter_map(|x| self.exit(x)).collect();
        let risk: Vec<Risk> = ast.risk.iter().filter_map(|r| self.risk(r)).collect();

        let (Some(entries), Some(size), Some(exits)) =
            (NonEmpty::any(entries), size, NonEmpty::any(exits))
        else {
            if self.refusals.is_empty() {
                self.refuse(
                    Limitation::IncompleteDocument,
                    ast.header_span,
                    "a strategy without an entry, a size or an exit",
                );
            }
            return None;
        };

        Some(FormStrategy {
            header: Header {
                name: ast.name.clone(),
                instrument: ast.instrument.clone(),
                timeframe: ast.timeframe.clone(),
            },
            params,
            constraints,
            inputs,
            entries,
            size,
            exits,
            risk,
        })
    }

    fn param(&mut self, d: &ast::ParamDecl) -> Option<Param> {
        Some(match &d.kind {
            ast::ParamKind::Int {
                min,
                max,
                step,
                default,
            } => Param::Int {
                name: d.name.clone(),
                min: self.whole(min, d.span)?,
                max: self.whole(max, d.span)?,
                step: match step {
                    None => None,
                    Some(s) => Some(self.whole(s, d.span)?),
                },
                default: self.whole(default, d.span)?,
            },
            ast::ParamKind::Float {
                min,
                max,
                log,
                default,
            } => Param::Float {
                name: d.name.clone(),
                min: self.real(min, d.span)?,
                max: self.real(max, d.span)?,
                log: *log,
                default: self.real(default, d.span)?,
            },
            ast::ParamKind::Enum { choices, default } => Param::Enum {
                name: d.name.clone(),
                choices: NonEmpty::any(choices.iter().cloned())?,
                default: default.clone(),
            },
        })
    }

    fn whole(&mut self, n: &ast::Num, span: Span) -> Option<i64> {
        match n.raw.parse::<i64>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.refuse(
                    Limitation::ComposedOperand,
                    span,
                    format!("`{}` where an `int` parameter wants a whole number", n.raw),
                );
                None
            }
        }
    }

    fn real(&mut self, n: &ast::Num, span: Span) -> Option<f64> {
        match n.raw.parse::<f64>() {
            Ok(v) if v.is_finite() => Some(v),
            _ => {
                self.refuse(
                    Limitation::ComposedOperand,
                    span,
                    format!("`{}` where a `float` parameter wants a number", n.raw),
                );
                None
            }
        }
    }

    fn constraint(&mut self, c: &ast::ConstraintClause) -> Option<Constraint> {
        // The same grammar `domain::strategy_def::params` parses, read back off
        // the text this crate is the only writer of. A constraint it cannot
        // read is one the sweeper cannot read either.
        for op in Compare::ALL {
            let Some((l, r)) = c.text.split_once(op.as_str()) else {
                continue;
            };
            // `<` would split `<=`; take the longest match by checking that the
            // remainder does not begin with the rest of a two-character
            // operator.
            if r.starts_with('=') {
                continue;
            }
            let (Some(lhs), Some(rhs)) = (constraint_operand(l), constraint_operand(r)) else {
                break;
            };
            return Some(Constraint { lhs, op, rhs });
        }
        self.refuse(
            Limitation::ComposedOperand,
            c.span,
            format!("the constraint `{}`", c.text),
        );
        None
    }

    fn input(&mut self, b: &ast::Binding) -> Option<Input> {
        match self.operand(&b.expr) {
            Some(Operand::Feature { family, args }) => Some(Input {
                name: b.name.clone(),
                family,
                args,
            }),
            Some(_) => {
                self.refuse(
                    Limitation::ComposedOperand,
                    b.span,
                    format!("the input `{}`, which does not name a feature", b.name),
                );
                None
            }
            None => None,
        }
    }

    fn entry(&mut self, e: &ast::Entry) -> Option<Entry> {
        if let Some((_, span)) = &e.cooldown {
            self.refuse(Limitation::Cooldown, *span, "a `cooldown`");
        }
        let (combine, filters) = self.condition(&e.when)?;
        Some(Entry {
            side: e.side,
            combine,
            filters,
        })
    }

    fn size(&mut self, ast: &Strategy) -> Option<Size> {
        let clause = ast.size.as_ref()?;
        Some(match &clause.kind {
            ast::SizeKind::Fixed(n) => Size::Fixed {
                quantity: n.raw.clone(),
            },
            ast::SizeKind::EquityFraction(n) => Size::EquityFraction {
                fraction: n.raw.clone(),
            },
            ast::SizeKind::RiskPerTrade(n) => Size::RiskPerTrade {
                fraction: n.raw.clone(),
            },
            ast::SizeKind::Other { word, .. } => {
                self.refuse(
                    Limitation::SizeModeBeyondTheThree,
                    clause.span,
                    format!("`size {word}`"),
                );
                return None;
            }
        })
    }

    fn exit(&mut self, x: &ast::Exit) -> Option<Exit> {
        let distance = |p: &mut Self, d: &ast::Distance, want: ast::Anchor| -> Option<String> {
            if d.unit != ast::Unit::Percent {
                p.refuse(
                    Limitation::NonPercentDistance,
                    d.unit_span,
                    format!("a distance in `{}`", d.unit.render()),
                );
                return None;
            }
            if d.anchor != want {
                p.refuse(
                    Limitation::ExitAnchor,
                    d.span,
                    format!("an exit measured from `{}`", d.anchor.as_str()),
                );
                return None;
            }
            Some(d.value.raw.clone())
        };
        Some(match &x.kind {
            ast::ExitKind::None => Exit::None,
            ast::ExitKind::Stop(d) => Exit::Stop {
                percent: distance(self, d, ast::Anchor::Entry)?,
            },
            ast::ExitKind::Target(d) => Exit::Target {
                percent: distance(self, d, ast::Anchor::Entry)?,
            },
            ast::ExitKind::Trailing(d) => Exit::Trailing {
                percent: distance(self, d, ast::Anchor::HighWater)?,
            },
            ast::ExitKind::Time(n) => match n.raw.parse::<u32>() {
                Ok(bars) => Exit::Time { bars },
                Err(_) => {
                    self.refuse(
                        Limitation::ComposedOperand,
                        n.span,
                        format!("`exit time {}`, which is not a whole number of bars", n.raw),
                    );
                    return None;
                }
            },
            ast::ExitKind::When(cond) => {
                let (combine, filters) = self.condition(cond)?;
                Exit::When { combine, filters }
            }
        })
    }

    fn risk(&mut self, r: &ast::RiskItem) -> Option<Risk> {
        let orders = |p: &mut Self, n: &ast::Num| -> Option<u32> {
            match n.raw.parse::<u32>() {
                Ok(v) => Some(v),
                Err(_) => {
                    p.refuse(
                        Limitation::ComposedOperand,
                        n.span,
                        format!("`{}`, which is not a whole number of orders", n.raw),
                    );
                    None
                }
            }
        };
        Some(match r.kind {
            ast::RiskKind::MaxPosition => Risk::MaxPosition {
                value: r.value.raw.clone(),
            },
            ast::RiskKind::MaxOrdersPerMin => Risk::MaxOrdersPerMin {
                orders: orders(self, &r.value)?,
            },
            ast::RiskKind::MaxOrdersPerSec => Risk::MaxOrdersPerSec {
                orders: orders(self, &r.value)?,
            },
            ast::RiskKind::MaxDailyLoss => {
                self.refuse(
                    Limitation::MaxDailyLoss,
                    r.span,
                    "a `risk max_daily_loss` clause",
                );
                return None;
            }
        })
    }

    /// A condition: filters at one level, joined by one operator.
    fn condition(&mut self, e: &ast::Expr) -> Option<(Combine, NonEmpty<Filter>)> {
        let combine = match e {
            ast::Expr::Binary {
                op: ast::BinOp::And,
                ..
            } => Combine::All,
            ast::Expr::Binary {
                op: ast::BinOp::Or, ..
            } => Combine::Any,
            _ => Combine::All,
        };
        let want = match combine {
            Combine::All => ast::BinOp::And,
            Combine::Any => ast::BinOp::Or,
        };
        let mut leaves: Vec<&ast::Expr> = Vec::new();
        flatten(e, want, &mut leaves);
        let mut filters = Vec::with_capacity(leaves.len());
        for leaf in leaves {
            filters.push(self.filter(leaf)?);
        }
        NonEmpty::any(filters).map(|f| (combine, f))
    }

    fn filter(&mut self, e: &ast::Expr) -> Option<Filter> {
        match e {
            ast::Expr::Binary { op, lhs, rhs, span } if op.is_comparison() => {
                let cmp = Compare::from_bin_op(*op).expect("is_comparison implies a Compare");
                let l = self.operand(lhs);
                let r = self.operand(rhs);
                let _ = span;
                Some(Filter::Compare {
                    lhs: l?,
                    op: cmp,
                    rhs: r?,
                })
            }
            ast::Expr::Call { name, args, .. } if name == "between" && args.len() == 3 => {
                let v = self.operand(&args[0]);
                let lo = self.operand(&args[1]);
                let hi = self.operand(&args[2]);
                Some(Filter::Between {
                    value: v?,
                    low: lo?,
                    high: hi?,
                })
            }
            ast::Expr::Unary {
                op: ast::UnOp::Not,
                span,
                ..
            } => {
                self.refuse(Limitation::Negation, *span, "a `not`");
                None
            }
            ast::Expr::Binary { op, span, .. } if op.is_logical() => {
                self.refuse(
                    Limitation::NestedCondition,
                    *span,
                    format!("an `{}` inside another condition", op.as_str()),
                );
                None
            }
            other => {
                self.refuse(
                    Limitation::ComposedOperand,
                    other.span(),
                    "a condition that is not a comparison or a `between`",
                );
                None
            }
        }
    }

    fn operand(&mut self, e: &ast::Expr) -> Option<Operand> {
        match e {
            ast::Expr::Number(n) => Some(Operand::Number {
                value: n.raw.clone(),
            }),
            ast::Expr::Bar { field, .. } => Some(Operand::Bar { field: *field }),
            ast::Expr::Name { name, span } => {
                if self.inputs.contains(name) {
                    Some(Operand::Input { name: name.clone() })
                } else if self.params.contains(name) {
                    Some(Operand::Param { name: name.clone() })
                } else {
                    self.refuse(
                        Limitation::ComposedOperand,
                        *span,
                        format!("the name `{name}`, which is not a declared input or parameter"),
                    );
                    None
                }
            }
            ast::Expr::Call {
                name,
                args,
                span,
                name_span,
            } => self.call_operand(name, args, *span, *name_span),
            // A negative literal reaches the tree as a negation of a positive
            // one — `unary()` runs before `primary()` — and it is still a
            // number. Without this a form carrying `-5` writes text it cannot
            // read back, which is the drift the round-trip test exists to catch.
            ast::Expr::Unary {
                op: ast::UnOp::Neg,
                rhs,
                ..
            } if matches!(**rhs, ast::Expr::Number(_)) => {
                let ast::Expr::Number(n) = &**rhs else {
                    unreachable!("guarded by the match")
                };
                Some(Operand::Number {
                    value: format!("-{}", n.raw),
                })
            }
            other => {
                self.refuse(
                    Limitation::ComposedOperand,
                    other.span(),
                    "an operand built out of other values",
                );
                None
            }
        }
    }

    fn call_operand(
        &mut self,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        name_span: Span,
    ) -> Option<Operand> {
        match name {
            "bar" => match args {
                [ast::Expr::Text { value, .. }] => BarField::parse(value)
                    .map(|field| Operand::Bar { field })
                    .or_else(|| {
                        self.refuse(
                            Limitation::ComposedOperand,
                            span,
                            format!("`bar('{value}')`, which is not a bar field"),
                        );
                        None
                    }),
                _ => {
                    self.refuse(Limitation::ComposedOperand, span, "a malformed `bar(…)`");
                    None
                }
            },
            "param" => match args {
                [ast::Expr::Text { value, .. }] if self.params.contains(value) => {
                    Some(Operand::Param {
                        name: value.clone(),
                    })
                }
                _ => {
                    self.refuse(
                        Limitation::ComposedOperand,
                        span,
                        "a `param(…)` naming no declared parameter",
                    );
                    None
                }
            },
            "feature" => match args {
                [ast::Expr::Text { value, .. }] => match self.cat.resolve(value) {
                    Some((family, parts)) => {
                        let family = family.name.clone();
                        let args = parts
                            .iter()
                            .map(|p| match p.parse::<i64>() {
                                Ok(n) => Arg::Bars(n),
                                Err(_) => Arg::Param(
                                    p.trim_start_matches("{{")
                                        .trim_end_matches("}}")
                                        .to_string(),
                                ),
                            })
                            .collect();
                        Some(Operand::Feature { family, args })
                    }
                    None => {
                        self.refuse(
                            Limitation::RawFeatureName,
                            span,
                            format!("`feature('{value}')`"),
                        );
                        None
                    }
                },
                _ => {
                    self.refuse(
                        Limitation::ComposedOperand,
                        span,
                        "a malformed `feature(…)`",
                    );
                    None
                }
            },
            family if self.cat.family(family).is_some() => {
                let mut rest = args;
                if let Some(ast::Expr::Bar {
                    field: BarField::Close,
                    ..
                }) = args.first()
                {
                    rest = &args[1..];
                }
                let mut out = Vec::with_capacity(rest.len());
                for a in rest {
                    match a {
                        ast::Expr::Number(n) => match n.raw.parse::<i64>() {
                            Ok(v) => out.push(Arg::Bars(v)),
                            Err(_) => {
                                self.refuse(
                                    Limitation::ComposedOperand,
                                    n.span,
                                    format!("`{}` as a feature parameter", n.raw),
                                );
                                return None;
                            }
                        },
                        ast::Expr::Name { name, .. } if self.params.contains(name) => {
                            out.push(Arg::Param(name.clone()));
                        }
                        other => {
                            self.refuse(
                                Limitation::ComposedOperand,
                                other.span(),
                                "a feature parameter that is not a number or a declared \
                                 parameter",
                            );
                            return None;
                        }
                    }
                }
                Some(Operand::Feature {
                    family: family.to_string(),
                    args: out,
                })
            }
            other => {
                self.refuse(
                    Limitation::RawFeatureName,
                    name_span,
                    format!("`{other}(…)`"),
                );
                None
            }
        }
    }
}

fn constraint_operand(t: &str) -> Option<ConstraintOperand> {
    let t = t.trim();
    if let Some(inner) = t.strip_prefix("param(").and_then(|r| r.strip_suffix(')')) {
        let name = inner.trim().trim_matches(|c| c == '\'' || c == '"');
        if name.is_empty() {
            return None;
        }
        return Some(ConstraintOperand::Param {
            name: name.to_string(),
        });
    }
    t.parse::<f64>().ok().map(|_| ConstraintOperand::Number {
        value: t.to_string(),
    })
}

/// Collect the leaves of a left-associated chain of one logical operator.
fn flatten<'a>(e: &'a ast::Expr, op: ast::BinOp, out: &mut Vec<&'a ast::Expr>) {
    if let ast::Expr::Binary {
        op: found,
        lhs,
        rhs,
        ..
    } = e
    {
        if *found == op {
            flatten(lhs, op, out);
            flatten(rhs, op, out);
            return;
        }
    }
    out.push(e);
}

// ── the schema ───────────────────────────────────────────────────────────────

/// The JSON Schema for one whole strategy, over the engine's own registry.
#[must_use]
pub fn document_schema() -> Value {
    document_schema_with(&Catalogue::engine())
}

/// As [`document_schema`], against a caller-supplied catalogue.
#[must_use]
pub fn document_schema_with(cat: &Catalogue) -> Value {
    let mut defs = serde_json::Map::new();
    for (name, schema) in clause_schemas_with(cat) {
        defs.insert(name.to_string(), schema);
    }
    defs.insert("operand".into(), operand_schema(cat));
    defs.insert("filter".into(), filter_schema());
    defs.insert("combine".into(), json!({"enum": ["all", "any"]}));
    defs.insert(
        "compare".into(),
        json!({"enum": Compare::ALL.iter().map(|c| serde_json::to_value(c).expect("a compare serialises")).collect::<Vec<_>>()}),
    );
    defs.insert("feature".into(), feature_schema(cat));
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "SLv2 strategy, one clause per fill",
        "type": "object",
        "additionalProperties": false,
        "required": ["header", "entries", "size", "exits"],
        "properties": {
            "header": {"$ref": "#/$defs/header"},
            "params": {"type": "array", "items": {"$ref": "#/$defs/param"}},
            "constraints": {"type": "array", "items": {"$ref": "#/$defs/constraint"}},
            "inputs": {"type": "array", "items": {"$ref": "#/$defs/input"}},
            "entries": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/entry"}},
            "size": {"$ref": "#/$defs/size"},
            "exits": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/exit"}},
            "risk": {"type": "array", "items": {"$ref": "#/$defs/risk"}},
        },
        "$defs": Value::Object(defs),
    })
}

/// Replace every `{"$ref": "#/$defs/x"}` with the definition it names.
///
/// A sampler's grammar converter should not have to resolve references to be
/// right, and a tool's `inputSchema` is read by more than one consumer. The
/// schema is not recursive; the depth bound turns a loop introduced by a future
/// edit into a panic here instead of an unbounded expansion somewhere a model is
/// waiting.
fn inline_refs(v: &Value, defs: &serde_json::Map<String, Value>, depth: usize) -> Value {
    assert!(depth < 16, "a schema reference loops");
    match v {
        Value::Object(o) => {
            if let Some(Value::String(r)) = o.get("$ref") {
                let name = r.trim_start_matches("#/$defs/");
                let target = defs
                    .get(name)
                    .unwrap_or_else(|| panic!("`{name}` is not defined in the schema"));
                return inline_refs(target, defs, depth + 1);
            }
            Value::Object(
                o.iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "$schema" | "$comment" | "$defs"))
                    .map(|(k, x)| (k.clone(), inline_refs(x, defs, depth)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| inline_refs(x, defs, depth)).collect()),
        other => other.clone(),
    }
}

/// [`document_schema`], self-contained: no `$ref`, no `$defs`.
///
/// This is the form a tool declares as its `inputSchema`, so that a
/// `select_fill` tier's arguments are decoded under it and the strategy it
/// writes cannot name a feature the engine does not serve.
#[must_use]
pub fn document_schema_inlined() -> Value {
    let doc = document_schema();
    let defs = doc["$defs"].as_object().cloned().unwrap_or_default();
    inline_refs(&doc, &defs, 0)
}

/// One clause's schema, self-contained. `None` for a name that is not a clause.
#[must_use]
pub fn clause_schema_inlined(name: &str) -> Option<Value> {
    let doc = document_schema();
    let defs = doc["$defs"].as_object().cloned().unwrap_or_default();
    defs.get(name).map(|s| inline_refs(s, &defs, 0))
}

/// The feature, as a decoder-sized schema: the family is an enumeration of the
/// engine's names, and the arguments are up to [`features::registry::MAX_PARAMS`]
/// whole numbers or declared parameter names.
///
/// What it gives up against [`feature_schema`]'s one branch per family is the
/// arity and the per-parameter range. What it keeps is the property S-5.9's
/// acceptance names: **an unknown feature cannot be written**. A wrong number of
/// arguments, or one out of range, reaches the compiler and comes back as
/// `wrong_arity` / `bad_feature_param` with a fix, which is the same diagnostic
/// free text gets.
///
/// The reason is size. The per-family form inlines to about 270 KB, because an
/// operand carries every family and a filter has up to five operands; a tool's
/// `inputSchema` is listed to every client, and a sampler's grammar of that size
/// is slow to build. This one inlines to a few KB.
fn compact_feature_schema(cat: &Catalogue) -> Value {
    let mut names: Vec<&str> = cat.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["family"],
        "properties": {
            "family": {"enum": names},
            "args": {
                "type": "array",
                "maxItems": features::registry::MAX_PARAMS,
                "items": {"oneOf": [
                    {"type": "integer", "minimum": 1},
                    {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"},
                ]},
            },
        },
    })
}

/// [`operand_schema`] over [`compact_feature_schema`].
fn compact_operand_schema(cat: &Catalogue) -> Value {
    let identifier = json!({"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"});
    let decimal = json!({"type": "string", "pattern": "^-?[0-9]+(\\.[0-9]+)?$"});
    let mut feature = compact_feature_schema(cat);
    if let Some(props) = feature.get_mut("properties").and_then(Value::as_object_mut) {
        props.insert("operand".into(), json!({"const": "feature"}));
    }
    if let Some(req) = feature.get_mut("required").and_then(Value::as_array_mut) {
        req.insert(0, json!("operand"));
    }
    json!({"oneOf": [
        feature,
        {"type": "object", "additionalProperties": false, "required": ["operand", "field"],
         "properties": {"operand": {"const": "bar"},
                        "field": {"enum": ["open", "high", "low", "close", "volume"]}}},
        {"type": "object", "additionalProperties": false, "required": ["operand", "name"],
         "properties": {"operand": {"const": "input"}, "name": identifier.clone()}},
        {"type": "object", "additionalProperties": false, "required": ["operand", "name"],
         "properties": {"operand": {"const": "param"}, "name": identifier}},
        {"type": "object", "additionalProperties": false, "required": ["operand", "value"],
         "properties": {"operand": {"const": "number"}, "value": decimal}},
    ]})
}

/// The document schema with the compact feature, and the definitions it needs.
fn compact_document() -> (Value, serde_json::Map<String, Value>) {
    let cat = Catalogue::engine();
    let mut doc = document_schema_with(&cat);
    if let Some(defs) = doc.get_mut("$defs").and_then(Value::as_object_mut) {
        defs.insert("feature".into(), compact_feature_schema(&cat));
        let operand = compact_operand_schema(&cat);
        // What is compared *to* may be a number; what is being tested may not.
        // A filter of two numbers (`-1 < 1`) is well-formed and says nothing, and
        // a small model reaches for the shortest branch of a `oneOf` unless the
        // grammar removes it — measured, in `small_tier_eval`.
        let mut lhs = operand.clone();
        if let Some(a) = lhs.get_mut("oneOf").and_then(Value::as_array_mut) {
            a.pop();
        }
        defs.insert("operand_lhs".into(), lhs);
        defs.insert("operand".into(), operand);
        let mut filter = filter_schema();
        if let Some(branches) = filter.get_mut("oneOf").and_then(Value::as_array_mut) {
            for (b, key) in branches.iter_mut().zip(["lhs", "value"]) {
                if let Some(p) = b.get_mut("properties").and_then(Value::as_object_mut) {
                    p.insert(key.into(), json!({"$ref": "#/$defs/operand_lhs"}));
                }
            }
        }
        defs.insert("filter".into(), filter);
        // A distance is a positive percentage; the sign is the side, not the number.
        if let Some(exit) = defs.get("exit").cloned() {
            let text = exit.to_string().replace(
                "^-?[0-9]+(\\\\.[0-9]+)?$",
                "^[0-9]+(\\\\.[0-9]+)?$",
            );
            if let Ok(v) = serde_json::from_str(&text) {
                defs.insert("exit".into(), v);
            }
        }
    }
    let defs = doc["$defs"].as_object().cloned().unwrap_or_default();
    (doc, defs)
}

/// [`document_schema_inlined`] over the compact feature — the schema a tool
/// declares and a small tier decodes under. See [`compact_feature_schema`] for
/// what it keeps and what it gives up.
#[must_use]
pub fn document_schema_compact() -> Value {
    let (doc, defs) = compact_document();
    inline_refs(&doc, &defs, 0)
}

/// One clause's compact schema, self-contained. `None` for a name that is not a
/// clause.
#[must_use]
pub fn clause_schema_compact(name: &str) -> Option<Value> {
    let (_, defs) = compact_document();
    defs.get(name).map(|s| inline_refs(s, &defs, 0))
}

/// One schema per clause kind: what a `select_fill` tier is shown, one fill at
/// a time (AGENT-011 §3).
#[must_use]
pub fn clause_schemas() -> Vec<(&'static str, Value)> {
    clause_schemas_with(&Catalogue::engine())
}

/// As [`clause_schemas`], against a caller-supplied catalogue.
#[must_use]
pub fn clause_schemas_with(cat: &Catalogue) -> Vec<(&'static str, Value)> {
    let identifier = json!({"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"});
    let decimal = json!({"type": "string", "pattern": "^-?[0-9]+(\\.[0-9]+)?$"});
    let _ = cat;
    vec![
        (
            "header",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "instrument", "timeframe"],
                "properties": {
                    "name": identifier,
                    "instrument": {"type": "string"},
                    "timeframe": {"type": "string", "pattern": "^[0-9]+[smhd]$"},
                },
            }),
        ),
        (
            "param",
            json!({"oneOf": [
                {"type": "object", "additionalProperties": false,
                 "required": ["type", "name", "min", "max", "default"],
                 "properties": {"type": {"const": "int"}, "name": identifier,
                                "min": {"type": "integer"}, "max": {"type": "integer"},
                                "step": {"type": "integer", "minimum": 1},
                                "default": {"type": "integer"}}},
                {"type": "object", "additionalProperties": false,
                 "required": ["type", "name", "min", "max", "default"],
                 "properties": {"type": {"const": "float"}, "name": identifier,
                                "min": {"type": "number"}, "max": {"type": "number"},
                                "log": {"type": "boolean"}, "default": {"type": "number"}}},
                {"type": "object", "additionalProperties": false,
                 "required": ["type", "name", "choices", "default"],
                 "properties": {"type": {"const": "enum"}, "name": identifier,
                                "choices": {"type": "array", "minItems": 1, "items": identifier},
                                "default": identifier}},
            ]}),
        ),
        (
            "constraint",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["lhs", "op", "rhs"],
                "properties": {
                    "lhs": {"$ref": "#/$defs/constraint_operand"},
                    "op": {"$ref": "#/$defs/compare"},
                    "rhs": {"$ref": "#/$defs/constraint_operand"},
                },
            }),
        ),
        (
            "constraint_operand",
            json!({"oneOf": [
                {"type": "object", "additionalProperties": false,
                 "required": ["side", "name"],
                 "properties": {"side": {"const": "param"}, "name": identifier}},
                {"type": "object", "additionalProperties": false,
                 "required": ["side", "value"],
                 "properties": {"side": {"const": "number"}, "value": decimal}},
            ]}),
        ),
        (
            "input",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "family"],
                "properties": {
                    "name": identifier,
                    "family": {"$ref": "#/$defs/feature"},
                },
                "$comment": "`family` and `args` are constrained together by #/$defs/feature",
            }),
        ),
        (
            "entry",
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["side", "combine", "filters"],
                "properties": {
                    "side": {"enum": ["long", "short"]},
                    "combine": {"$ref": "#/$defs/combine"},
                    "filters": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/filter"}},
                },
            }),
        ),
        (
            "size",
            json!({"oneOf": [
                {"type": "object", "additionalProperties": false,
                 "required": ["size", "quantity"],
                 "properties": {"size": {"const": "fixed"}, "quantity": decimal}},
                {"type": "object", "additionalProperties": false,
                 "required": ["size", "fraction"],
                 "properties": {"size": {"const": "equity_fraction"}, "fraction": decimal}},
                {"type": "object", "additionalProperties": false,
                 "required": ["size", "fraction"],
                 "properties": {"size": {"const": "risk_per_trade"}, "fraction": decimal}},
            ]}),
        ),
        (
            "exit",
            json!({"oneOf": [
                {"type": "object", "additionalProperties": false, "required": ["exit"],
                 "properties": {"exit": {"const": "none"}}},
                {"type": "object", "additionalProperties": false, "required": ["exit", "percent"],
                 "properties": {"exit": {"const": "stop"}, "percent": decimal}},
                {"type": "object", "additionalProperties": false, "required": ["exit", "percent"],
                 "properties": {"exit": {"const": "target"}, "percent": decimal}},
                {"type": "object", "additionalProperties": false, "required": ["exit", "percent"],
                 "properties": {"exit": {"const": "trailing"}, "percent": decimal}},
                {"type": "object", "additionalProperties": false, "required": ["exit", "bars"],
                 "properties": {"exit": {"const": "time"}, "bars": {"type": "integer", "minimum": 1}}},
                {"type": "object", "additionalProperties": false,
                 "required": ["exit", "combine", "filters"],
                 "properties": {"exit": {"const": "when"},
                                "combine": {"$ref": "#/$defs/combine"},
                                "filters": {"type": "array", "minItems": 1,
                                            "items": {"$ref": "#/$defs/filter"}}}},
            ]}),
        ),
        (
            "risk",
            json!({"oneOf": [
                {"type": "object", "additionalProperties": false, "required": ["risk", "value"],
                 "properties": {"risk": {"const": "max_position"}, "value": decimal}},
                {"type": "object", "additionalProperties": false, "required": ["risk", "orders"],
                 "properties": {"risk": {"const": "max_orders_per_min"},
                                "orders": {"type": "integer", "minimum": 1}}},
                {"type": "object", "additionalProperties": false, "required": ["risk", "orders"],
                 "properties": {"risk": {"const": "max_orders_per_sec"},
                                "orders": {"type": "integer", "minimum": 1}}},
            ]}),
        ),
    ]
}

/// One branch per family, each carrying its own arity and parameter ranges.
///
/// This is the half of the grammar that makes `unknown_feature` and
/// `wrong_arity` unreachable for a constrained decoder rather than merely
/// discouraged.
fn feature_schema(cat: &Catalogue) -> Value {
    let mut branches: Vec<Value> = Vec::with_capacity(cat.len());
    let mut names: Vec<&str> = cat.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    for name in names {
        let family = cat.family(name).expect("the name came from the catalogue");
        let prefix: Vec<Value> = family
            .params
            .iter()
            .map(|p| {
                json!({"oneOf": [
                    {"type": "integer", "minimum": p.min, "maximum": p.max,
                     "title": p.name},
                    {"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$",
                     "title": format!("a declared parameter, swept over {}..{}", p.min, p.max)},
                ]})
            })
            .collect();
        let n = family.params.len();
        branches.push(json!({
            "type": "object",
            "additionalProperties": false,
            "required": if n == 0 { vec!["family"] } else { vec!["family", "args"] },
            "properties": {
                "family": {"const": name},
                "args": {"type": "array", "minItems": n, "maxItems": n, "prefixItems": prefix},
            },
        }));
    }
    json!({"oneOf": branches})
}

fn operand_schema(cat: &Catalogue) -> Value {
    let identifier = json!({"type": "string", "pattern": "^[A-Za-z_][A-Za-z0-9_]*$"});
    let decimal = json!({"type": "string", "pattern": "^-?[0-9]+(\\.[0-9]+)?$"});
    let mut feature_branches: Vec<Value> = match feature_schema(cat) {
        Value::Object(mut o) => match o.remove("oneOf") {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    for b in &mut feature_branches {
        if let Some(props) = b.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert("operand".into(), json!({"const": "feature"}));
        }
        if let Some(req) = b.get_mut("required").and_then(Value::as_array_mut) {
            req.insert(0, json!("operand"));
        }
    }
    let mut branches = feature_branches;
    branches.push(json!({
        "type": "object", "additionalProperties": false,
        "required": ["operand", "field"],
        "properties": {"operand": {"const": "bar"},
                       "field": {"enum": ["open", "high", "low", "close", "volume"]}},
    }));
    branches.push(json!({
        "type": "object", "additionalProperties": false,
        "required": ["operand", "name"],
        "properties": {"operand": {"const": "input"}, "name": identifier},
    }));
    branches.push(json!({
        "type": "object", "additionalProperties": false,
        "required": ["operand", "name"],
        "properties": {"operand": {"const": "param"}, "name": identifier},
    }));
    branches.push(json!({
        "type": "object", "additionalProperties": false,
        "required": ["operand", "value"],
        "properties": {"operand": {"const": "number"}, "value": decimal},
    }));
    json!({"oneOf": branches})
}

fn filter_schema() -> Value {
    json!({"oneOf": [
        {"type": "object", "additionalProperties": false,
         "required": ["filter", "lhs", "op", "rhs"],
         "properties": {"filter": {"const": "compare"},
                        "lhs": {"$ref": "#/$defs/operand"},
                        "op": {"$ref": "#/$defs/compare"},
                        "rhs": {"$ref": "#/$defs/operand"}}},
        {"type": "object", "additionalProperties": false,
         "required": ["filter", "value", "low", "high"],
         "properties": {"filter": {"const": "between"},
                        "value": {"$ref": "#/$defs/operand"},
                        "low": {"$ref": "#/$defs/operand"},
                        "high": {"$ref": "#/$defs/operand"}}},
    ]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn minimal(filters: Vec<Filter>) -> FormStrategy {
        FormStrategy {
            header: Header {
                name: "s".into(),
                instrument: "$bound_at_init".into(),
                timeframe: "1h".into(),
            },
            params: Vec::new(),
            constraints: Vec::new(),
            inputs: Vec::new(),
            entries: NonEmpty::any([Entry {
                side: Side::Long,
                combine: Combine::All,
                filters: NonEmpty::any(filters).expect("at least one filter"),
            }])
            .expect("one entry"),
            size: Size::Fixed {
                quantity: "1".into(),
            },
            exits: NonEmpty::any([Exit::Time { bars: 3 }]).expect("one exit"),
            risk: Vec::new(),
        }
    }

    /// The whole reason this module is not its own compiler: what the form
    /// produces is text, and the text goes through the one front door.
    #[test]
    fn the_form_compiles_through_the_language_and_not_beside_it() {
        let form = minimal(vec![Filter::Compare {
            lhs: Operand::Feature {
                family: "rsi".into(),
                args: vec![Arg::Bars(14)],
            },
            op: Compare::Lt,
            rhs: Operand::Number { value: "30".into() },
        }]);
        let text = form.to_text();
        assert!(
            text.contains("entry long when rsi(close, 14) < 30"),
            "{text}"
        );
        assert_eq!(
            form.compile().expect("compiles"),
            crate::text_to_definition(&text).expect("compiles"),
            "the form's definition is the language's definition, or there are two compilers"
        );
    }

    /// §8.7 asks for a compiled strategy in ≤ 6 fills. One clause is one fill,
    /// so the floor is header, entry, size, exit.
    #[test]
    fn a_whole_strategy_fits_in_six_fills() {
        let form = minimal(vec![Filter::Compare {
            lhs: Operand::Bar {
                field: BarField::Close,
            },
            op: Compare::Gt,
            rhs: Operand::Number { value: "1".into() },
        }]);
        assert_eq!(form.fills(), 4);
        assert!(form.fills() <= 6);
        form.compile().expect("compiles");
    }

    /// The property the schema exists for: a decoder constrained by it cannot
    /// name a feature the engine does not serve, and every branch it *can*
    /// choose compiles. Checked by walking every branch, not by sampling.
    #[test]
    fn every_family_in_the_schema_compiles_when_the_form_uses_it() {
        let cat = Catalogue::engine();
        let schema = feature_schema(&cat);
        let branches = schema["oneOf"].as_array().expect("branches");
        assert!(
            branches.len() >= 55,
            "the scan found {} families, which is not the registry",
            branches.len()
        );
        for branch in branches {
            let family = branch["properties"]["family"]["const"]
                .as_str()
                .expect("every branch names a family")
                .to_string();
            let f = cat.family(&family).expect("the branch names a real family");
            let args: Vec<Arg> = f
                .params
                .iter()
                .map(|p| Arg::Bars(p.min.max(2).min(p.max)))
                .collect();
            let form = minimal(vec![Filter::Compare {
                lhs: Operand::Feature {
                    family: family.clone(),
                    args,
                },
                op: Compare::Gt,
                rhs: Operand::Number { value: "0".into() },
            }]);
            let def = form.compile().unwrap_or_else(|e| {
                panic!(
                    "`{family}` is in the schema and does not compile: {e:#?}\n{}",
                    form.to_text()
                )
            });
            assert!(
                def.inputs.iter().any(|i| !i.features.is_empty()),
                "`{family}` compiled without declaring a feature"
            );
        }
    }

    /// Every entry in the closed list is a real narrowing, and what it says
    /// about the language is true *of the live compiler* rather than of a
    /// comment written once.
    #[test]
    fn every_limitation_is_real_and_reachable() {
        assert_eq!(
            Limitation::all().len(),
            14,
            "the list shrank; each entry is a case"
        );
        let mut seen = BTreeSet::new();
        for &l in Limitation::all() {
            let src = l.example();
            let ast =
                parse(src).unwrap_or_else(|e| panic!("{l}: the example does not parse: {e:#?}"));

            // The form refuses it, and names this limitation.
            let refusals = FormStrategy::from_strategy(&ast)
                .err()
                .unwrap_or_else(|| panic!("{l}: the form accepted the example"));
            assert!(!refusals.is_empty(), "{l}: a refusal naming nothing");
            let named: BTreeSet<Limitation> = refusals.iter().map(|r| r.limitation).collect();
            assert_eq!(
                named,
                [l].into_iter().collect::<BTreeSet<_>>(),
                "{l}: the example should isolate one limitation"
            );
            assert_eq!(refusals[0].diagnostic.code, Code::NotInTheForm);
            assert!(refusals[0].diagnostic.fix.is_some(), "{l}: no fix");
            assert!(refusals[0].diagnostic.see.is_some(), "{l}: no guide anchor");

            // And what it claims about the language is true.
            match l.reach() {
                Reach::Compiles => {
                    crate::compile(&ast).unwrap_or_else(|e| {
                        panic!("{l}: claims the free form compiles this, and it does not: {e:#?}")
                    });
                }
                Reach::RefusedByTheCompiler(code) => {
                    let errs = crate::compile(&ast).err().unwrap_or_else(|| {
                        panic!("{l}: claims the compiler refuses this, and it compiled")
                    });
                    assert!(
                        errs.iter().any(|d| d.code == code),
                        "{l}: claims `{code}` and the compiler said {:?}",
                        errs.iter().map(|d| d.code).collect::<Vec<_>>()
                    );
                }
            }
            seen.insert(l);
        }
        assert_eq!(seen.len(), Limitation::all().len(), "a duplicate entry");
    }

    /// The projection is the inverse of the rendering on everything the form
    /// can hold. Without this the two halves could drift and only a model would
    /// find out.
    #[test]
    fn a_form_strategy_survives_text_and_comes_back_the_same_form() {
        let rich = FormStrategy {
            header: Header {
                name: "rich".into(),
                instrument: "BTC-USD".into(),
                timeframe: "15m".into(),
            },
            params: vec![
                Param::Int {
                    name: "fast".into(),
                    min: 5,
                    max: 50,
                    step: Some(5),
                    default: 10,
                },
                Param::Float {
                    name: "gate".into(),
                    min: 0.5,
                    max: 4.0,
                    log: false,
                    default: 2.0,
                },
            ],
            constraints: vec![Constraint {
                lhs: ConstraintOperand::Param {
                    name: "fast".into(),
                },
                op: Compare::Lt,
                rhs: ConstraintOperand::Number { value: "60".into() },
            }],
            inputs: vec![Input {
                name: "trend".into(),
                family: "ema".into(),
                args: vec![Arg::Param("fast".into())],
            }],
            entries: NonEmpty::any([
                Entry {
                    side: Side::Long,
                    combine: Combine::All,
                    filters: NonEmpty::any([
                        Filter::Compare {
                            lhs: Operand::Bar {
                                field: BarField::Close,
                            },
                            op: Compare::Gt,
                            rhs: Operand::Input {
                                name: "trend".into(),
                            },
                        },
                        Filter::Compare {
                            lhs: Operand::Feature {
                                family: "zscore".into(),
                                args: vec![Arg::Bars(20)],
                            },
                            op: Compare::Lt,
                            rhs: Operand::Param {
                                name: "gate".into(),
                            },
                        },
                    ])
                    .unwrap(),
                },
                Entry {
                    side: Side::Short,
                    combine: Combine::Any,
                    filters: NonEmpty::any([
                        Filter::Between {
                            value: Operand::Feature {
                                family: "rsi".into(),
                                args: vec![Arg::Bars(14)],
                            },
                            low: Operand::Number { value: "70".into() },
                            high: Operand::Number {
                                value: "100".into(),
                            },
                        },
                        Filter::Compare {
                            lhs: Operand::Feature {
                                family: "roc".into(),
                                args: vec![Arg::Bars(10)],
                            },
                            op: Compare::Lt,
                            rhs: Operand::Number { value: "-5".into() },
                        },
                    ])
                    .unwrap(),
                },
            ])
            .unwrap(),
            size: Size::RiskPerTrade {
                fraction: "0.01".into(),
            },
            exits: NonEmpty::any([
                Exit::Stop {
                    percent: "2".into(),
                },
                Exit::Trailing {
                    percent: "3".into(),
                },
                Exit::Time { bars: 48 },
                Exit::When {
                    combine: Combine::Any,
                    filters: NonEmpty::any([
                        Filter::Compare {
                            lhs: Operand::Feature {
                                family: "efficiency_ratio".into(),
                                args: vec![Arg::Bars(50)],
                            },
                            op: Compare::Lt,
                            rhs: Operand::Number {
                                value: "0.2".into(),
                            },
                        },
                        Filter::Compare {
                            lhs: Operand::Feature {
                                family: "rsi".into(),
                                args: vec![Arg::Bars(7)],
                            },
                            op: Compare::Gt,
                            rhs: Operand::Number { value: "90".into() },
                        },
                    ])
                    .unwrap(),
                },
            ])
            .unwrap(),
            risk: vec![
                Risk::MaxPosition {
                    value: "0.5".into(),
                },
                Risk::MaxOrdersPerMin { orders: 6 },
            ],
        };
        let text = rich.to_text();
        rich.compile()
            .unwrap_or_else(|e| panic!("the rich form does not compile:\n{text}\n{e:#?}"));
        let ast = parse(&text).expect("the form writes parseable text");
        let back = FormStrategy::from_strategy(&ast)
            .unwrap_or_else(|e| panic!("the form cannot read its own text back: {e:#?}"));
        assert_eq!(back, rich, "--- text ---\n{text}");
        assert_eq!(back.to_text(), text, "the text is not a fixpoint");
    }

    /// Two fills can name the same feature without either being wrong, and the
    /// document rule that forbids it is invisible from inside one clause. The
    /// assembler binds rather than letting the parser refuse text the model
    /// spent two fills on.
    #[test]
    fn a_feature_two_fills_both_named_is_bound_rather_than_refused() {
        let rsi = |op: Compare, v: &str| Filter::Compare {
            lhs: Operand::Feature {
                family: "rsi".into(),
                args: vec![Arg::Bars(14)],
            },
            op,
            rhs: Operand::Number { value: v.into() },
        };
        let form = minimal(vec![rsi(Compare::Gt, "30"), rsi(Compare::Lt, "70")]);
        let text = form.to_text();
        assert!(text.contains("input rsi_14 = rsi(close, 14)"), "{text}");
        assert!(
            text.contains("entry long when rsi_14 > 30 and rsi_14 < 70"),
            "{text}"
        );
        let def = form
            .compile()
            .unwrap_or_else(|e| panic!("the assembled text must compile:\n{text}\n{e:#?}"));
        assert_eq!(def.inputs[1].features, vec!["rsi_14"]);

        // The same for a feature one fill declared as an input and another
        // wrote out in full: the inline one becomes the name.
        let mut declared = minimal(vec![rsi(Compare::Gt, "30")]);
        declared.inputs = vec![Input {
            name: "momentum_gate".into(),
            family: "rsi".into(),
            args: vec![Arg::Bars(14)],
        }];
        let text = declared.to_text();
        assert!(
            text.contains("entry long when momentum_gate > 30"),
            "{text}"
        );
        declared
            .compile()
            .unwrap_or_else(|e| panic!("{text}\n{e:#?}"));

        // And the text is still a fixpoint through the projection.
        let ast = parse(&text).expect("parses");
        let back = FormStrategy::from_strategy(&ast).expect("projects");
        assert_eq!(back.to_text(), text);
    }

    /// The one thing a form does not get back exactly: with a single filter,
    /// `all` and `any` are the same condition and render as the same text.
    /// Stated here so a future reader does not file it as a drift bug.
    #[test]
    fn a_single_filter_normalises_its_combine() {
        let mut form = minimal(vec![Filter::Compare {
            lhs: Operand::Bar {
                field: BarField::Close,
            },
            op: Compare::Gt,
            rhs: Operand::Number { value: "1".into() },
        }]);
        let first = form.entries.iter().next().expect("never empty").clone();
        form.entries = NonEmpty::new(
            Entry {
                combine: Combine::Any,
                ..first
            },
            Vec::new(),
        );
        let text = form.to_text();
        let ast = parse(&text).expect("parses");
        let back = FormStrategy::from_strategy(&ast).expect("projects");
        assert_eq!(
            back.entries.iter().next().unwrap().combine,
            Combine::All,
            "one filter has no join, so the projection reports the canonical one"
        );
        assert_eq!(back.to_text(), text, "and the text is unchanged either way");
    }

    /// The fills survive JSON, because the tool boundary is JSON and a
    /// `select_fill` tier emits nothing else.
    #[test]
    fn the_fills_round_trip_through_json_and_an_empty_list_is_refused() {
        let form = minimal(vec![Filter::Compare {
            lhs: Operand::Bar {
                field: BarField::Close,
            },
            op: Compare::Ge,
            rhs: Operand::Number { value: "1".into() },
        }]);
        let json = serde_json::to_string(&form).expect("serialises");
        let back: FormStrategy = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, form);

        let empty = json.replace(r#""filters":[{"#, r#""filters":[],"unused":[{"#);
        assert!(
            serde_json::from_str::<FormStrategy>(&empty).is_err(),
            "an entry with no filters must not deserialise"
        );
    }

    /// What the schema is for, stated as a test: nothing in it names a feature
    /// the engine does not serve.
    #[test]
    fn the_schema_only_offers_features_the_engine_resolves() {
        let cat = Catalogue::engine();
        let doc = document_schema_with(&cat);
        let branches = doc["$defs"]["feature"]["oneOf"]
            .as_array()
            .expect("branches");
        assert!(branches.len() >= 55, "the scan found {}", branches.len());
        for b in branches {
            let name = b["properties"]["family"]["const"].as_str().expect("a name");
            assert!(
                cat.family(name).is_some(),
                "the schema offers `{name}` and the engine does not serve it"
            );
        }
        for clause in ["header", "param", "entry", "size", "exit", "risk"] {
            assert!(doc["$defs"][clause].is_object(), "no schema for {clause}");
        }
    }

    /// Two entries on the same side are how the form says `or` across
    /// different filter sets — the repair `Limitation::NestedCondition`
    /// recommends. It has to actually work.
    #[test]
    fn the_repair_for_a_nested_condition_compiles() {
        let mut form = minimal(vec![Filter::Compare {
            lhs: Operand::Bar {
                field: BarField::Close,
            },
            op: Compare::Gt,
            rhs: Operand::Number { value: "1".into() },
        }]);
        let second = Entry {
            side: Side::Long,
            combine: Combine::All,
            filters: NonEmpty::any([Filter::Compare {
                lhs: Operand::Feature {
                    family: "volume".into(),
                    args: Vec::new(),
                },
                op: Compare::Gt,
                rhs: Operand::Number { value: "2".into() },
            }])
            .unwrap(),
        };
        let first = form.entries.iter().next().expect("never empty").clone();
        form.entries = NonEmpty::new(first, vec![second]);
        let def = form.compile().expect("compiles");
        assert_eq!(def.actions.len(), 1, "one position, two ways in");
    }
}
