//! What travels along a strategy graph's wires (FEAT-007 §5.1, ADR-0040).
//!
//! # Absence is a value
//!
//! The v1 runtime represented "no value yet" as `NaN` in a feature slot. A `NaN`
//! compares false, so a condition over a missing indicator silently read as
//! "not true" — which is how indicator strategies came to sit on paper for ever
//! without anyone being told (FEAT-007 §1, defect 2), and how a model that never
//! answered looked exactly like a model that said no.
//!
//! Here, absence is [`Value::Missing`] carrying a [`MissingReason`], and a [`Num`]
//! **cannot** hold `NaN` or an infinity: [`Num::new`] refuses them and
//! [`Value::num`] turns them into `missing(undefined)`. Boolean logic over absence
//! is Kleene three-valued ([`Truth`]) and defined once, in this file.
//!
//! The engine may use a compact internal representation for speed; this type is
//! the contract — what is serialised, journaled, and compared for parity.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

// ── Num ──────────────────────────────────────────────────────────────────────

/// A finite real number. `NaN` and the infinities are unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Num(f64);

impl Num {
    /// `None` for `NaN` and ±∞.
    #[must_use]
    pub fn new(x: f64) -> Option<Self> {
        x.is_finite().then_some(Self(x))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Num {
    type Error = NonFinite;
    fn try_from(x: f64) -> Result<Self, Self::Error> {
        Self::new(x).ok_or(NonFinite(x))
    }
}

impl From<Num> for f64 {
    fn from(n: Num) -> Self {
        n.0
    }
}

/// A non-finite number was offered where a [`Num`] was required.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("{0} is not a finite number")]
pub struct NonFinite(pub f64);

// ── Quantile levels ──────────────────────────────────────────────────────────

/// A probability level in (0, 1), stored in parts per million so that a
/// distribution's *type* can be compared and hashed exactly. `0.05` and
/// `0.050000001` are the same level to a person and must be the same level to the
/// typechecker; storing `f64` would make them two types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Level(u32);

impl Level {
    const SCALE: f64 = 1_000_000.0;

    /// A level strictly between 0 and 1, rounded to the nearest millionth.
    #[must_use]
    pub fn new(p: f64) -> Option<Self> {
        if !(p > 0.0 && p < 1.0) {
            return None;
        }
        let ppm = (p * Self::SCALE).round();
        // Rounding can land exactly on 0 or 1_000_000 for p within half a ppm of
        // the boundary; those are not levels either.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (1.0..Self::SCALE)
            .contains(&ppm)
            .then_some(Self(ppm as u32))
    }

    #[must_use]
    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / Self::SCALE
    }
}

impl TryFrom<f64> for Level {
    type Error = BadLevel;
    fn try_from(p: f64) -> Result<Self, Self::Error> {
        Self::new(p).ok_or(BadLevel(p))
    }
}

impl From<Level> for f64 {
    fn from(l: Level) -> Self {
        l.as_f64()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[error("{0} is not a probability level in (0, 1)")]
pub struct BadLevel(pub f64);

// ── Static types ─────────────────────────────────────────────────────────────

/// The static type of a port or wire.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValueType {
    Num,
    Bool,
    /// One label from a set. An empty `labels` means the set is not declared
    /// (e.g. a derived contract for a model whose classes the registry does not
    /// record); the typechecker can then check structure but not label names.
    Cat {
        #[serde(default)]
        labels: Vec<String>,
    },
    /// A probability for each label, in `labels` order.
    Probs {
        #[serde(default)]
        labels: Vec<String>,
    },
    /// Quantiles at strictly increasing `levels`.
    Dist {
        levels: Vec<Level>,
    },
    /// A fixed-length vector. `dim: None` means the length is not declared.
    Vec {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dim: Option<u32>,
    },
    Text,
    /// Named fields, in a stable (sorted) order.
    Record {
        fields: BTreeMap<String, ValueType>,
    },
}

impl ValueType {
    /// A short name for diagnostics: `num`, `bool`, `cat<down,flat,up>` …
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Num => "num".into(),
            Self::Bool => "bool".into(),
            Self::Cat { labels } if labels.is_empty() => "cat".into(),
            Self::Cat { labels } => format!("cat<{}>", labels.join(",")),
            Self::Probs { labels } if labels.is_empty() => "probs".into(),
            Self::Probs { labels } => format!("probs<{}>", labels.join(",")),
            Self::Dist { levels } => format!(
                "dist<{}>",
                levels
                    .iter()
                    .map(|l| l.as_f64().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Vec { dim: Some(n) } => format!("vec<{n}>"),
            Self::Vec { dim: None } => "vec".into(),
            Self::Text => "text".into(),
            Self::Record { fields } => format!(
                "record{{{}}}",
                fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.label()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

// ── Why a value is missing ───────────────────────────────────────────────────

/// Why a value is absent. Every variant names enough to act on: *which* model,
/// *how many* bars short, *which* node upstream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum MissingReason {
    /// The source has fewer bars than its declared lookback.
    WarmingUp { needed: u32, have: u32 },
    /// The environment had no data for this input at this instant.
    NoData { what: String },
    /// The model answered, and its answer was "no answer".
    ModelAbstained { model: String, detail: String },
    /// The model did not answer within its contract's timeout.
    ModelTimeout { model: String, timeout_ms: u32 },
    /// No model could answer: unreachable, unregistered, or — in a backtest — no
    /// walk-forward fold trained early enough to answer point-in-time.
    ModelUnavailable { model: String, detail: String },
    /// The arithmetic has no answer: division by zero, log of a non-positive,
    /// a result that is not finite.
    Undefined { op: String },
    /// A text could not be parsed into the declared structure.
    Unparseable { detail: String },
    /// An input to this node was missing. `node` is the node it came from.
    Upstream { node: String },
}

impl fmt::Display for MissingReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WarmingUp { needed, have } => {
                write!(f, "warming up ({have} of {needed} bars)")
            }
            Self::NoData { what } => write!(f, "no data for {what}"),
            Self::ModelAbstained { model, detail } => write!(f, "{model} abstained: {detail}"),
            Self::ModelTimeout { model, timeout_ms } => {
                write!(f, "{model} did not answer within {timeout_ms} ms")
            }
            Self::ModelUnavailable { model, detail } => {
                write!(f, "{model} unavailable: {detail}")
            }
            Self::Undefined { op } => write!(f, "{op} is undefined here"),
            Self::Unparseable { detail } => write!(f, "could not parse: {detail}"),
            Self::Upstream { node } => write!(f, "upstream node '{node}' is missing"),
        }
    }
}

// ── Values ───────────────────────────────────────────────────────────────────

/// A value at run time. Any value may be [`Value::Missing`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Value {
    Num(Num),
    Bool(bool),
    Cat(String),
    /// Probabilities in the label order of the value's type. Carried with their
    /// labels so a journal record is readable without the plan beside it.
    Probs {
        labels: Vec<String>,
        p: Vec<Num>,
    },
    Dist {
        levels: Vec<Level>,
        values: Vec<Num>,
    },
    Vec(Vec<Num>),
    Text(String),
    Record(BTreeMap<String, Value>),
    Missing(MissingReason),
}

impl Value {
    /// A number, or `missing(undefined)` naming `op` if `x` is not finite. The
    /// one door through which an `f64` result becomes a value.
    #[must_use]
    pub fn num(x: f64, op: &str) -> Self {
        Num::new(x).map_or_else(
            || Self::Missing(MissingReason::Undefined { op: op.into() }),
            Self::Num,
        )
    }

    #[must_use]
    pub const fn missing(reason: MissingReason) -> Self {
        Self::Missing(reason)
    }

    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing(_))
    }

    #[must_use]
    pub const fn missing_reason(&self) -> Option<&MissingReason> {
        match self {
            Self::Missing(r) => Some(r),
            _ => None,
        }
    }

    /// The number inside, if this is a present `num`.
    #[must_use]
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(n.get()),
            _ => None,
        }
    }

    /// Read this value as a truth value.
    ///
    /// # Errors
    /// [`NotABool`] when the value is present and not a `bool`. A typechecked
    /// plan never produces this; it exists so a hand-built value cannot be
    /// silently read as false.
    pub fn truth(&self) -> Result<Truth, NotABool> {
        match self {
            Self::Bool(true) => Ok(Truth::True),
            Self::Bool(false) => Ok(Truth::False),
            Self::Missing(r) => Ok(Truth::Unknown(r.clone())),
            other => Err(NotABool(other.kind_name())),
        }
    }

    /// The variant's name, for diagnostics.
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Num(_) => "num",
            Self::Bool(_) => "bool",
            Self::Cat(_) => "cat",
            Self::Probs { .. } => "probs",
            Self::Dist { .. } => "dist",
            Self::Vec(_) => "vec",
            Self::Text(_) => "text",
            Self::Record(_) => "record",
            Self::Missing(_) => "missing",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a {0} was read as a truth value")]
pub struct NotABool(pub &'static str);

// ── Kleene logic ─────────────────────────────────────────────────────────────

/// A three-valued truth: true, false, or unknown-because.
///
/// `and`, `or` and `not` follow Kleene's strong logic, which is the only choice
/// that lets a strategy say something sensible with partial information:
/// `false and anything` is false whatever *anything* is, so "flat and the model
/// says buy" is false while flat is false even if the model timed out — but
/// `true and unknown` is unknown, so the strategy does not trade on a model it
/// never heard from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "truth", content = "reason", rename_all = "snake_case")]
pub enum Truth {
    True,
    False,
    Unknown(MissingReason),
}

impl Truth {
    /// Kleene conjunction. When both sides are unknown the left reason is kept,
    /// so the result is deterministic in argument order.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            (Self::Unknown(r), _) | (_, Self::Unknown(r)) => Self::Unknown(r),
        }
    }

    /// Kleene disjunction.
    #[must_use]
    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            (Self::Unknown(r), _) | (_, Self::Unknown(r)) => Self::Unknown(r),
        }
    }

    /// Only `True` is true. A rule whose condition is unknown does not match.
    #[must_use]
    pub const fn is_true(&self) -> bool {
        matches!(self, Self::True)
    }

    #[must_use]
    pub fn into_value(self) -> Value {
        match self {
            Self::True => Value::Bool(true),
            Self::False => Value::Bool(false),
            Self::Unknown(r) => Value::Missing(r),
        }
    }
}

/// Kleene negation: unknown stays unknown.
impl std::ops::Not for Truth {
    type Output = Self;
    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            u @ Self::Unknown(_) => u,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown(tag: &str) -> Truth {
        Truth::Unknown(MissingReason::NoData { what: tag.into() })
    }

    #[test]
    fn a_num_cannot_hold_nan_or_an_infinity() {
        for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -f64::NAN] {
            assert!(Num::new(x).is_none(), "{x} must be refused");
            assert!(
                Value::num(x, "div").is_missing(),
                "{x} must become missing(undefined)"
            );
        }
        // And every finite number is accepted, including the awkward ones.
        for x in [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            1e-308,
            -1.234_567_890_123_456_7,
        ] {
            assert_eq!(Num::new(x).map(Num::get), Some(x));
        }
    }

    /// A broad sweep over the bit patterns of `f64`, standing in for a property
    /// test without adding a dependency: every non-finite pattern is refused and
    /// every finite one accepted.
    #[test]
    fn num_accepts_exactly_the_finite_floats() {
        let mut bits: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..200_000 {
            // xorshift64*: deterministic, covers sign, exponent and mantissa.
            bits ^= bits >> 12;
            bits ^= bits << 25;
            bits ^= bits >> 27;
            let x = f64::from_bits(bits.wrapping_mul(0x2545_F491_4F6C_DD1D));
            assert_eq!(Num::new(x).is_some(), x.is_finite(), "{x:?}");
        }
        // The exponent-all-ones patterns specifically, which a random sweep
        // reaches rarely.
        for mantissa in [0u64, 1, 0x000F_FFFF_FFFF_FFFF] {
            for sign in [0u64, 1u64 << 63] {
                let x = f64::from_bits(sign | 0x7FF0_0000_0000_0000 | mantissa);
                assert!(Num::new(x).is_none());
            }
        }
    }

    #[test]
    fn a_non_finite_number_does_not_deserialize_into_a_num() {
        // JSON has no NaN literal, but a hand-built document or a lenient parser
        // can still deliver one; the type refuses it at the boundary.
        assert!(
            serde_json::from_str::<Num>("1e400").is_err(),
            "overflow to inf"
        );
        assert_eq!(serde_json::from_str::<Num>("1.5").unwrap().get(), 1.5);
    }

    #[test]
    fn the_kleene_truth_table_in_full() {
        use Truth::{False as F, True as T};
        let u = unknown("u");
        let cases_and = [
            (T, T, T),
            (T, F, F),
            (F, T, F),
            (F, F, F),
            (T, u.clone(), u.clone()),
            (u.clone(), T, u.clone()),
            (F, u.clone(), F),
            (u.clone(), F, F),
            (u.clone(), u.clone(), u.clone()),
        ];
        for (a, b, want) in cases_and {
            assert_eq!(a.clone().and(b.clone()), want, "{a:?} and {b:?}");
        }
        let cases_or = [
            (T, T, T),
            (T, F, T),
            (F, T, T),
            (F, F, F),
            (T, u.clone(), T),
            (u.clone(), T, T),
            (F, u.clone(), u.clone()),
            (u.clone(), F, u.clone()),
            (u.clone(), u.clone(), u.clone()),
        ];
        for (a, b, want) in cases_or {
            assert_eq!(a.clone().or(b.clone()), want, "{a:?} or {b:?}");
        }
        assert_eq!(!T, F);
        assert_eq!(!F, T);
        assert_eq!(!u.clone(), u);
    }

    #[test]
    fn two_unknowns_keep_the_left_reason_so_the_result_is_deterministic() {
        let a = unknown("left");
        let b = unknown("right");
        assert_eq!(a.clone().and(b.clone()), a.clone());
        assert_eq!(a.clone().or(b), a);
    }

    #[test]
    fn only_true_matches_a_rule() {
        assert!(Truth::True.is_true());
        assert!(!Truth::False.is_true());
        assert!(
            !unknown("x").is_true(),
            "a condition nobody could evaluate must not fire"
        );
    }

    #[test]
    fn reading_a_number_as_a_truth_value_is_an_error_not_false() {
        let v = Value::Num(Num::new(0.0).unwrap());
        assert_eq!(v.truth(), Err(NotABool("num")));
        assert_eq!(
            Value::Missing(MissingReason::NoData { what: "x".into() }).truth(),
            Ok(unknown("x"))
        );
    }

    #[test]
    fn every_missing_reason_round_trips() {
        let reasons = [
            MissingReason::WarmingUp {
                needed: 48,
                have: 3,
            },
            MissingReason::NoData {
                what: "BTC-USD 1d".into(),
            },
            MissingReason::ModelAbstained {
                model: "vol@12".into(),
                detail: "input 'rv_20' absent".into(),
            },
            MissingReason::ModelTimeout {
                model: "vol@12".into(),
                timeout_ms: 250,
            },
            MissingReason::ModelUnavailable {
                model: "vol@12".into(),
                detail: "no fold trained before t − 4h".into(),
            },
            MissingReason::Undefined { op: "div".into() },
            MissingReason::Unparseable {
                detail: "expected field 'score'".into(),
            },
            MissingReason::Upstream { node: "vol".into() },
        ];
        for r in reasons {
            let json = serde_json::to_string(&r).unwrap();
            let back: MissingReason = serde_json::from_str(&json).unwrap();
            assert_eq!(back, r, "{json}");
            assert!(!r.to_string().is_empty());
        }
    }

    /// The serialised form is the parity contract (ADR-0047): golden bytes, so a
    /// change to it is a decision rather than a side effect.
    #[test]
    fn value_serialisation_is_stable() {
        let v = Value::Record(BTreeMap::from([
            ("sigma".to_string(), Value::Num(Num::new(0.0125).unwrap())),
            ("up".to_string(), Value::Bool(true)),
            (
                "why".to_string(),
                Value::Missing(MissingReason::ModelTimeout {
                    model: "m@3".into(),
                    timeout_ms: 250,
                }),
            ),
        ]));
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"kind":"record","value":{"sigma":{"kind":"num","value":0.0125},"up":{"kind":"bool","value":true},"why":{"kind":"missing","value":{"reason":"model_timeout","model":"m@3","timeout_ms":250}}}}"#
        );
    }

    #[test]
    fn levels_are_exact_and_bounded() {
        assert_eq!(Level::new(0.05), Level::new(0.050_000_000_1));
        assert!(Level::new(0.0).is_none());
        assert!(Level::new(1.0).is_none());
        assert!(Level::new(1e-9).is_none(), "rounds to zero ppm");
        assert!(Level::new(f64::NAN).is_none());
        let t = ValueType::Dist {
            levels: vec![Level::new(0.05).unwrap(), Level::new(0.5).unwrap()],
        };
        assert_eq!(t.label(), "dist<0.05,0.5>");
    }
}
