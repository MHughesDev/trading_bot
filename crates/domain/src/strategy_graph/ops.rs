//! The operator registry (FEAT-007 §5.3, ADR-0040, QA-3).
//!
//! One table. The compiler typechecks against it, the editor builds its palette
//! from it, SLv3 takes its function names from it, and the agent's catalogue is
//! generated from it — so an operator that exists is an operator every one of
//! those knows about, and one that does not exist is refused everywhere. Adding an
//! operator is adding a row here plus its evaluation in the engine; nothing else
//! has a list to keep in step.
//!
//! Indicator *families* (the 61 DATA-006 features) are not rows here: they are
//! reached through the `feature` operator and typed by the feature registry. That
//! keeps one source of truth for indicators too.

use serde::Serialize;

use super::graph::StrategyKind;

/// A type in an operator's signature. May contain variables, which the compiler
/// unifies per node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ty {
    Num,
    Bool,
    Text,
    /// Any type, the same everywhere the variable appears in one signature.
    Var(char),
    /// `cat<L>` for a label set `L` shared with other ports of the signature.
    CatOf(char),
    /// `probs<L>`.
    ProbsOf(char),
    AnyDist,
    AnyVec,
    AnyRecord,
    /// A list of numbers / of truth values (each element a wire).
    NumList,
    BoolList,
    /// A feature window for a model input (built by `features`).
    Window,
    /// The model's output record, typed by its contract.
    FromContract,
    /// The record a `parse` schema declares.
    FromSchema,
}

/// One input port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Port {
    pub name: &'static str,
    pub ty: Ty,
    #[serde(skip_serializing_if = "is_false")]
    pub optional: bool,
    /// Must be a literal (e.g. a window length), not a wire. A parameter
    /// reference counts as a literal: it is fixed before the run starts.
    #[serde(skip_serializing_if = "is_false")]
    pub literal: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(b: &bool) -> bool {
    !*b
}

const fn wire(name: &'static str, ty: Ty) -> Port {
    Port {
        name,
        ty,
        optional: false,
        literal: false,
    }
}
const fn lit(name: &'static str, ty: Ty) -> Port {
    Port {
        name,
        ty,
        optional: false,
        literal: true,
    }
}
const fn opt_lit(name: &'static str, ty: Ty) -> Port {
    Port {
        name,
        ty,
        optional: true,
        literal: true,
    }
}

/// How many bars of history an operator needs before its output is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "arg", rename_all = "snake_case")]
pub enum Warmup {
    /// Stateless: present as soon as its inputs are.
    Stateless,
    /// Stateful, but present from its first tick (e.g. `latch`).
    Zero,
    /// A fixed number of ticks (e.g. `crossed_above` needs one previous value).
    Fixed(u32),
    /// The value of a literal port (e.g. `n` in `rolling_mean(x, n)`).
    FromPort(&'static str),
    /// From the feature registry, for the named family.
    FromFeature,
    /// From the model contract's input windows.
    FromContract,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Source,
    Math,
    Compare,
    Logic,
    Branch,
    Temporal,
    Model,
    Adapter,
    Blend,
    CrossSectional,
    Absence,
}

/// One operator.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct OpSpec {
    pub name: &'static str,
    pub family: Family,
    pub inputs: &'static [Port],
    pub output: Ty,
    /// Keeps state between ticks (and therefore per instrument in a selection).
    pub stateful: bool,
    pub warmup: Warmup,
    /// Takes further named inputs beyond `inputs` — `model`, whose inputs are its
    /// contract's, and `switch`, whose cases are one input per label.
    pub open_inputs: bool,
    /// Only meaningful across a universe; refused in a trading strategy.
    pub cross_sectional: bool,
    /// One line, imperative: when to reach for it.
    pub use_when: &'static str,
}

impl OpSpec {
    /// Whether a graph of `kind` may use this operator.
    #[must_use]
    pub const fn allowed_in(&self, kind: StrategyKind) -> bool {
        !self.cross_sectional || matches!(kind, StrategyKind::Selection)
    }

    #[must_use]
    pub fn port(&self, name: &str) -> Option<&Port> {
        self.inputs.iter().find(|p| p.name == name)
    }
}

macro_rules! op {
    ($name:literal, $fam:ident, [$($p:expr),* $(,)?] -> $out:expr, $warm:expr, $use:literal) => {
        OpSpec { name: $name, family: Family::$fam, inputs: &[$($p),*], output: $out,
                 stateful: !matches!($warm, Warmup::Stateless), warmup: $warm,
                 open_inputs: false, cross_sectional: false, use_when: $use }
    };
}

use Ty::{AnyDist, AnyRecord, AnyVec, Bool, BoolList, Num, NumList, Text, Window};
use Warmup::{Fixed, FromPort, Stateless, Zero};

const T: Ty = Ty::Var('T');

static OPS: &[OpSpec] = &[
    // ── sources ──────────────────────────────────────────────────────────────
    op!("feature", Source, [lit("name", Text)] -> Num, Warmup::FromFeature,
        "Read any DATA-006 indicator by name — rsi_14, atr_14, roc_20, bb_width_20 — computed from closed bars."),
    op!("bar", Source, [lit("field", Text), opt_lit("input", Text)] -> Num, Stateless,
        "Read a closed bar's open/high/low/close/volume, of this instrument or a declared input."),
    op!("hour", Source, [] -> Num, Stateless,
        "The decision instant's UTC hour, 0–23, for time-of-day rules."),
    op!("weekday", Source, [] -> Num, Stateless,
        "The decision instant's UTC weekday, 0 = Monday, for day-of-week rules."),
    op!("selection_attr", Source, [lit("selection", Text), lit("attr", Text)] -> Num, Stateless,
        "Read an attribute a selection strategy computed for this instrument — e.g. its momentum rank."),
    op!("features", Source, [lit("set", Text), lit("lookback", Num), opt_lit("timeframe", Text)] -> Window,
        Warmup::FromPort("lookback"),
        "Build a model's input window from a named feature set and a lookback."),
    // ── math ─────────────────────────────────────────────────────────────────
    op!("add", Math, [wire("a", Num), wire("b", Num)] -> Num, Stateless, "a + b."),
    op!("sub", Math, [wire("a", Num), wire("b", Num)] -> Num, Stateless, "a − b."),
    op!("mul", Math, [wire("a", Num), wire("b", Num)] -> Num, Stateless, "a × b."),
    op!("div", Math, [wire("a", Num), wire("b", Num)] -> Num, Stateless,
        "a ÷ b; missing (undefined) when b is zero."),
    op!("neg", Math, [wire("x", Num)] -> Num, Stateless, "−x."),
    op!("abs", Math, [wire("x", Num)] -> Num, Stateless, "|x|."),
    op!("sqrt", Math, [wire("x", Num)] -> Num, Stateless, "√x; missing for negative x."),
    op!("log", Math, [wire("x", Num)] -> Num, Stateless, "Natural log; missing for x ≤ 0."),
    op!("exp", Math, [wire("x", Num)] -> Num, Stateless, "eˣ; missing on overflow."),
    op!("sign", Math, [wire("x", Num)] -> Num, Stateless, "−1, 0 or 1."),
    op!("pow", Math, [wire("base", Num), wire("exp", Num)] -> Num, Stateless, "base^exp."),
    op!("min", Math, [wire("a", Num), wire("b", Num)] -> Num, Stateless, "The smaller of two values."),
    op!("max", Math, [wire("a", Num), wire("b", Num)] -> Num, Stateless, "The larger of two values."),
    op!("clip", Math, [wire("x", Num), wire("lo", Num), wire("hi", Num)] -> Num, Stateless,
        "Bound x to [lo, hi] — e.g. a model's size fraction."),
    op!("round", Math, [wire("x", Num), lit("digits", Num)] -> Num, Stateless,
        "Round to a number of decimal places."),
    // ── comparisons ──────────────────────────────────────────────────────────
    op!("gt", Compare, [wire("a", Num), wire("b", Num)] -> Bool, Stateless, "a > b."),
    op!("lt", Compare, [wire("a", Num), wire("b", Num)] -> Bool, Stateless, "a < b."),
    op!("ge", Compare, [wire("a", Num), wire("b", Num)] -> Bool, Stateless, "a ≥ b."),
    op!("le", Compare, [wire("a", Num), wire("b", Num)] -> Bool, Stateless, "a ≤ b."),
    op!("eq", Compare, [wire("a", Num), wire("b", Num)] -> Bool, Stateless, "a = b, exactly."),
    op!("ne", Compare, [wire("a", Num), wire("b", Num)] -> Bool, Stateless, "a ≠ b."),
    op!("between", Compare, [wire("x", Num), wire("lo", Num), wire("hi", Num)] -> Bool, Stateless,
        "lo ≤ x ≤ hi."),
    op!("is", Compare, [wire("x", Ty::CatOf('L')), lit("label", Text)] -> Bool, Stateless,
        "Whether a category — e.g. a classifier's predicted class — is a given label."),
    // ── logic ────────────────────────────────────────────────────────────────
    op!("and", Logic, [wire("a", Bool), wire("b", Bool)] -> Bool, Stateless,
        "Both. False if either is false, even when the other is missing."),
    op!("or", Logic, [wire("a", Bool), wire("b", Bool)] -> Bool, Stateless,
        "Either. True if either is true, even when the other is missing."),
    op!("not", Logic, [wire("x", Bool)] -> Bool, Stateless, "Negation; missing stays missing."),
    // ── branching ────────────────────────────────────────────────────────────
    op!("if", Branch, [wire("cond", Bool), wire("then", T), wire("else", T)] -> T, Stateless,
        "Choose between two branches — each may be its own chain of models and indicators."),
    // `switch` takes one further argument per label — `{"on": "regime",
    // "calm": "a", "stress": "b", "default": "c"}` — so its cases are ordinary
    // wires rather than a nested map, which keeps the canonical form flat.
    OpSpec {
        name: "switch",
        family: Family::Branch,
        inputs: &[wire("on", Ty::CatOf('L')), wire("default", T)],
        output: T,
        stateful: false,
        warmup: Stateless,
        open_inputs: true,
        cross_sectional: false,
        use_when: "Choose a branch per label — e.g. a different sizing per detected regime.",
    },
    // ── over time ────────────────────────────────────────────────────────────
    op!("prev", Temporal, [wire("x", T), lit("n", Num)] -> T, FromPort("n"),
        "The value n decision ticks ago."),
    op!("change", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "x minus its value n ticks ago."),
    op!("crossed_above", Temporal, [wire("a", Num), wire("b", Num)] -> Bool, Fixed(1),
        "True on the tick a moves from at-or-below b to above it."),
    op!("crossed_below", Temporal, [wire("a", Num), wire("b", Num)] -> Bool, Fixed(1),
        "True on the tick a moves from at-or-above b to below it."),
    op!("rising", Temporal, [wire("x", Num), lit("n", Num)] -> Bool, FromPort("n"),
        "Strictly increasing over the last n ticks."),
    op!("falling", Temporal, [wire("x", Num), lit("n", Num)] -> Bool, FromPort("n"),
        "Strictly decreasing over the last n ticks."),
    op!("rolling_mean", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Mean of any value over its last n ticks — including a model's output."),
    op!("rolling_std", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Sample standard deviation of any value over its last n ticks."),
    op!("rolling_min", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Lowest value over the last n ticks."),
    op!("rolling_max", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Highest value over the last n ticks."),
    op!("rolling_sum", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Sum over the last n ticks."),
    op!("ema", Temporal, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Exponential moving average of any value; of a bar field it is the DATA-006 ema_n."),
    op!("bars_since", Temporal, [wire("cond", Bool)] -> Num, Zero,
        "Ticks since cond was last true; missing until it has been."),
    op!("count_true", Temporal, [wire("cond", Bool), lit("n", Num)] -> Num, FromPort("n"),
        "How many of the last n ticks cond was true."),
    op!("latch", Temporal, [wire("set", Bool), wire("reset", Bool)] -> Bool, Zero,
        "Becomes true when set fires and stays true until reset fires."),
    op!("hysteresis", Temporal, [wire("x", Num), wire("on", Num), wire("off", Num)] -> Bool, Zero,
        "True once x rises above on, false again only when it falls below off — no flip-flopping."),
    op!("hold_last", Temporal, [wire("x", T)] -> T, Zero,
        "The last present value of x, when the current one is missing — e.g. a slow model's last answer."),
    op!("cooldown", Temporal, [wire("cond", Bool), lit("n", Num)] -> Bool, FromPort("n"),
        "cond, but at most once every n ticks."),
    // ── models ───────────────────────────────────────────────────────────────
    OpSpec {
        name: "model",
        family: Family::Model,
        inputs: &[lit("ref", Text)],
        output: Ty::FromContract,
        stateful: false,
        warmup: Warmup::FromContract,
        open_inputs: true,
        cross_sectional: false,
        use_when: "Call any MLOps model by name@version; its inputs and outputs are typed by its contract.",
    },
    op!("argmax", Adapter, [wire("p", Ty::ProbsOf('L'))] -> Ty::CatOf('L'), Stateless,
        "The most probable label of a classifier's probabilities."),
    op!("prob", Adapter, [wire("p", Ty::ProbsOf('L')), lit("label", Text)] -> Num, Stateless,
        "The probability a classifier gives one label."),
    op!("quantile", Adapter, [wire("d", AnyDist), lit("q", Num)] -> Num, Stateless,
        "A quantile of a distributional forecast — e.g. q=0.25 as a limit price offset."),
    op!("mean", Adapter, [wire("d", AnyDist)] -> Num, Stateless,
        "The mean implied by a distributional forecast's quantiles."),
    op!("prob_above", Adapter, [wire("d", AnyDist), wire("x", Num)] -> Num, Stateless,
        "The probability a distributional forecast puts above x."),
    op!("cosine", Adapter, [wire("a", AnyVec), wire("b", AnyVec)] -> Num, Stateless,
        "Cosine similarity of two embeddings."),
    op!("parse", Adapter, [wire("text", Text), lit("schema", AnyRecord)] -> Ty::FromSchema, Stateless,
        "Turn an LLM's reply into typed fields; missing when it does not match the schema."),
    // ── blending ─────────────────────────────────────────────────────────────
    op!("weighted_mean", Blend, [wire("xs", NumList), lit("weights", NumList)] -> Num, Stateless,
        "Blend several values — indicators, models, or both — with fixed weights."),
    op!("vote", Blend, [wire("xs", BoolList), lit("k", Num)] -> Bool, Stateless,
        "True when at least k of the conditions are true."),
    op!("zscore", Blend, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "How unusual x is against its own last n ticks."),
    op!("pct_rank", Blend, [wire("x", Num), lit("n", Num)] -> Num, FromPort("n"),
        "Percentile of x within its own last n ticks, 0–1."),
    // ── cross-sectional (selection only) ─────────────────────────────────────
    cs("cs_rank", "Rank of x across the universe, 1 = highest."),
    cs("cs_zscore", "x standardised across the universe at this instant."),
    cs("cs_pct_rank", "Percentile of x across the universe, 0–1."),
    cs("cs_mean", "Mean of x across the universe — the same for every member."),
    cs("cs_median", "Median of x across the universe."),
    OpSpec {
        name: "cs_count",
        family: Family::CrossSectional,
        inputs: &[wire("cond", Bool)],
        output: Num,
        stateful: false,
        warmup: Stateless,
        open_inputs: false,
        cross_sectional: true,
        use_when: "How many universe members satisfy cond — breadth.",
    },
    // ── absence ──────────────────────────────────────────────────────────────
    op!("coalesce", Absence, [wire("x", T), wire("default", T)] -> T, Stateless,
        "x, or default when x is missing — making a fallback a decision rather than an accident."),
    op!("is_missing", Absence, [wire("x", T)] -> Bool, Stateless,
        "Whether x is missing — never itself missing."),
    op!("missing_reason", Absence, [wire("x", T)] -> Text, Stateless,
        "Why x is missing, as text; missing when x is present."),
];

/// The one input every `cs_*` operator but `cs_count` takes. A `const` item, not
/// an inline `&[…]`, because a reference built inside a `const fn` is not promoted
/// to `'static` when its elements come from a function call.
const CS_X: &[Port] = &[wire("x", Num)];

const fn cs(name: &'static str, use_when: &'static str) -> OpSpec {
    OpSpec {
        name,
        family: Family::CrossSectional,
        inputs: CS_X,
        output: Num,
        stateful: false,
        warmup: Stateless,
        open_inputs: false,
        cross_sectional: true,
        use_when,
    }
}

/// Every operator.
#[must_use]
pub fn ops() -> &'static [OpSpec] {
    OPS
}

/// One operator by name.
#[must_use]
pub fn op(name: &str) -> Option<&'static OpSpec> {
    OPS.iter().find(|o| o.name == name)
}

/// Names a graph may use as built-in sources without declaring a node: the
/// bound instrument's closed bar at the decision timeframe.
pub const BAR_FIELDS: [&str; 5] = ["open", "high", "low", "close", "volume"];

/// `state.*` fields (FEAT-007 §5.4) and their types. Read from the
/// environment's portfolio view, never from an engine guess.
pub const STATE_FIELDS: [(&str, Ty); 13] = [
    ("flat", Bool),
    ("long", Bool),
    ("short", Bool),
    ("position", Num),
    ("position_fraction", Num),
    ("entry_price", Num),
    ("bars_in_position", Num),
    ("unrealized_return", Num),
    ("realized_pnl_today", Num),
    ("equity", Num),
    ("cash", Num),
    ("open_orders", Num),
    ("mark", Num),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// FEAT-007 §5.3, transcribed. If the spec gains an operator, it is added
    /// here and in the table; if the table drops one, this fails.
    const SPEC_OPERATORS: &[&str] = &[
        "feature",
        "bar",
        "hour",
        "weekday",
        "selection_attr",
        "features",
        "add",
        "sub",
        "mul",
        "div",
        "neg",
        "abs",
        "min",
        "max",
        "pow",
        "log",
        "exp",
        "sqrt",
        "sign",
        "clip",
        "round",
        "gt",
        "lt",
        "ge",
        "le",
        "eq",
        "ne",
        "between",
        "is",
        "and",
        "or",
        "not",
        "if",
        "switch",
        "prev",
        "change",
        "crossed_above",
        "crossed_below",
        "rising",
        "falling",
        "rolling_mean",
        "rolling_std",
        "rolling_min",
        "rolling_max",
        "rolling_sum",
        "ema",
        "bars_since",
        "count_true",
        "latch",
        "hysteresis",
        "hold_last",
        "cooldown",
        "model",
        "argmax",
        "prob",
        "quantile",
        "mean",
        "prob_above",
        "cosine",
        "parse",
        "weighted_mean",
        "vote",
        "zscore",
        "pct_rank",
        "cs_rank",
        "cs_zscore",
        "cs_pct_rank",
        "cs_mean",
        "cs_median",
        "cs_count",
        "coalesce",
        "is_missing",
        "missing_reason",
    ];

    #[test]
    fn every_operator_in_the_spec_is_in_the_table_and_nothing_else_is() {
        let table: BTreeSet<&str> = ops().iter().map(|o| o.name).collect();
        let spec: BTreeSet<&str> = SPEC_OPERATORS.iter().copied().collect();
        assert_eq!(table, spec, "table and FEAT-007 §5.3 disagree");
    }

    #[test]
    fn no_two_operators_share_a_name() {
        let mut seen = BTreeSet::new();
        for o in ops() {
            assert!(seen.insert(o.name), "duplicate operator {}", o.name);
        }
    }

    #[test]
    fn every_stateful_operator_declares_a_warmup() {
        for o in ops() {
            if o.stateful {
                assert_ne!(o.warmup, Warmup::Stateless, "{} is stateful", o.name);
            } else {
                assert!(
                    matches!(
                        o.warmup,
                        Warmup::Stateless | Warmup::FromFeature | Warmup::FromContract
                    ) || o.family == Family::Source,
                    "{} declares a warm-up but is not stateful",
                    o.name
                );
            }
        }
    }

    #[test]
    fn a_warmup_read_from_a_port_names_a_literal_port_that_exists() {
        for o in ops() {
            if let Warmup::FromPort(p) = o.warmup {
                let port = o
                    .port(p)
                    .unwrap_or_else(|| panic!("{} warms up from missing port {p}", o.name));
                assert!(
                    port.literal,
                    "{}'s warm-up port {p} must be a literal",
                    o.name
                );
            }
        }
    }

    #[test]
    fn cross_sectional_operators_are_refused_in_a_trading_strategy() {
        let cs: Vec<_> = ops().iter().filter(|o| o.cross_sectional).collect();
        assert_eq!(cs.len(), 6);
        for o in cs {
            assert!(o.name.starts_with("cs_"), "{}", o.name);
            assert!(!o.allowed_in(StrategyKind::Trading), "{}", o.name);
            assert!(o.allowed_in(StrategyKind::Selection), "{}", o.name);
        }
        for o in ops().iter().filter(|o| !o.cross_sectional) {
            assert!(o.allowed_in(StrategyKind::Trading), "{}", o.name);
        }
    }

    #[test]
    fn every_operator_says_when_to_use_it() {
        for o in ops() {
            assert!(!o.use_when.trim().is_empty(), "{}", o.name);
            assert!(o.use_when.len() <= 140, "{}: keep it to one line", o.name);
        }
    }

    #[test]
    fn a_type_variable_in_the_output_is_bound_by_some_input() {
        fn vars(t: Ty) -> Option<char> {
            match t {
                Ty::Var(c) | Ty::CatOf(c) | Ty::ProbsOf(c) => Some(c),
                _ => None,
            }
        }
        for o in ops() {
            if let Some(v) = vars(o.output) {
                assert!(
                    o.inputs.iter().any(|p| vars(p.ty) == Some(v)),
                    "{}'s output variable {v} is unbound",
                    o.name
                );
            }
        }
    }

    #[test]
    fn the_table_serialises_for_the_catalogue() {
        let json = serde_json::to_value(ops()).unwrap();
        assert_eq!(json.as_array().unwrap().len(), SPEC_OPERATORS.len());
        assert_eq!(json[0]["name"], "feature");
    }

    #[test]
    fn state_fields_have_unique_names() {
        let names: BTreeSet<_> = STATE_FIELDS.iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), STATE_FIELDS.len());
    }
}
