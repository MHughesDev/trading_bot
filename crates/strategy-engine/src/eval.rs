//! Stateless operators (FEAT-007 §5.1, §5.3).
//!
//! # How absence flows
//!
//! - Arithmetic, comparisons and adapters: a missing input makes the output
//!   missing **with the input's own reason** — the root cause, not "upstream", so
//!   a journal line reads "hold: vol_forecaster@12 did not answer within 250 ms"
//!   rather than a chain of "upstream node was missing".
//! - `and`, `or`, `not`, `vote`: Kleene three-valued logic (`domain::Truth`).
//! - `if`: a missing condition is missing; otherwise the chosen branch's value,
//!   whatever the other branch holds.
//! - `coalesce`, `is_missing`, `missing_reason`: absence handlers, the only
//!   operators that turn a missing value into a present one.
//!
//! A result that is not a finite number is `missing(undefined)` naming the
//! operator — `Value::num` is the only door from `f64`.

use std::collections::BTreeMap;

use domain::strategy_graph::{Level, MissingReason, Num, Truth, Value};

/// One evaluated argument: a value, or a list of values (`weighted_mean`,
/// `vote`).
#[derive(Clone, Debug, PartialEq)]
pub enum ArgVal {
    One(Value),
    Many(Vec<Value>),
}

impl ArgVal {
    fn one(&self) -> &Value {
        match self {
            Self::One(v) => v,
            Self::Many(_) => &UNDEF_LIST,
        }
    }
}

static UNDEF_LIST: Value = Value::Missing(MissingReason::Undefined { op: String::new() });

/// The first missing value among `ports`, in port order (deterministic).
fn first_missing<'a>(
    args: &'a BTreeMap<String, ArgVal>,
    ports: &[&str],
) -> Option<&'a MissingReason> {
    for p in ports {
        match args.get(*p) {
            Some(ArgVal::One(Value::Missing(r))) => return Some(r),
            Some(ArgVal::Many(vs)) => {
                if let Some(r) = vs.iter().find_map(Value::missing_reason) {
                    return Some(r);
                }
            }
            _ => {}
        }
    }
    None
}

fn num(args: &BTreeMap<String, ArgVal>, p: &str) -> f64 {
    args.get(p)
        .and_then(|a| a.one().as_num())
        .unwrap_or(f64::NAN)
}

fn truth(args: &BTreeMap<String, ArgVal>, p: &str) -> Truth {
    match args.get(p).map(ArgVal::one) {
        Some(Value::Bool(true)) => Truth::True,
        Some(Value::Bool(false)) => Truth::False,
        Some(Value::Missing(r)) => Truth::Unknown(r.clone()),
        _ => Truth::Unknown(MissingReason::Undefined {
            op: format!("{p} is not a condition"),
        }),
    }
}

/// Evaluate a stateless operator by name over its evaluated arguments.
///
/// `ports` is the operator's declared port order, used to pick the first
/// missing input deterministically.
#[must_use]
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn eval(op: &str, ports: &[&str], args: &BTreeMap<String, ArgVal>) -> Value {
    // Handlers and three-valued logic first: they decide for themselves what
    // absence means.
    match op {
        "and" => return truth(args, "a").and(truth(args, "b")).into_value(),
        "or" => return truth(args, "a").or(truth(args, "b")).into_value(),
        "not" => return (!truth(args, "x")).into_value(),
        "if" => {
            return match truth(args, "cond") {
                Truth::True => args
                    .get("then")
                    .map_or(Value::Missing(undefined("if")), |a| a.one().clone()),
                Truth::False => args
                    .get("else")
                    .map_or(Value::Missing(undefined("if")), |a| a.one().clone()),
                Truth::Unknown(r) => Value::Missing(r),
            };
        }
        "switch" => {
            return match args.get("on").map(ArgVal::one) {
                Some(Value::Cat(label)) => args
                    .get(label.as_str())
                    .or_else(|| args.get("default"))
                    .map_or(Value::Missing(undefined("switch")), |a| a.one().clone()),
                Some(Value::Missing(r)) => Value::Missing(r.clone()),
                _ => Value::Missing(undefined("switch")),
            };
        }
        "coalesce" => {
            let x = args.get("x").map(ArgVal::one);
            return match x {
                Some(v) if !v.is_missing() => v.clone(),
                _ => args
                    .get("default")
                    .map_or(Value::Missing(undefined("coalesce")), |a| a.one().clone()),
            };
        }
        "is_missing" => return Value::Bool(args.get("x").is_none_or(|a| a.one().is_missing())),
        "missing_reason" => {
            return match args.get("x").map(ArgVal::one) {
                Some(Value::Missing(r)) => Value::Text(r.to_string()),
                _ => Value::Missing(MissingReason::Undefined {
                    op: "missing_reason of a present value".into(),
                }),
            };
        }
        "vote" => {
            let k = num(args, "k");
            let Some(ArgVal::Many(xs)) = args.get("xs") else {
                return Value::Missing(undefined("vote"));
            };
            let (mut yes, mut unknown, mut reason) = (0.0, 0.0, None);
            for x in xs {
                match x {
                    Value::Bool(true) => yes += 1.0,
                    Value::Bool(false) => {}
                    Value::Missing(r) => {
                        unknown += 1.0;
                        reason.get_or_insert_with(|| r.clone());
                    }
                    _ => return Value::Missing(undefined("vote")),
                }
            }
            // Kleene-consistent: true if enough are certainly true, false if
            // even every unknown being true would not be enough.
            return if yes >= k {
                Value::Bool(true)
            } else if yes + unknown < k {
                Value::Bool(false)
            } else {
                Value::Missing(reason.unwrap_or_else(|| undefined("vote")))
            };
        }
        _ => {}
    }
    if let Some(r) = first_missing(args, ports) {
        return Value::Missing(r.clone());
    }
    // The three common port names, read lazily. Short names on purpose: they
    // are the operators' own port names.
    let (a, b, x) = (|| num(args, "a"), || num(args, "b"), || num(args, "x"));
    match op {
        "add" => Value::num(a() + b(), op),
        "sub" => Value::num(a() - b(), op),
        "mul" => Value::num(a() * b(), op),
        "div" => {
            if b() == 0.0 {
                Value::Missing(undefined("div by zero"))
            } else {
                Value::num(a() / b(), op)
            }
        }
        "neg" => Value::num(-x(), op),
        "abs" => Value::num(x().abs(), op),
        "sqrt" => {
            if x() < 0.0 {
                Value::Missing(undefined("sqrt of a negative"))
            } else {
                Value::num(x().sqrt(), op)
            }
        }
        "log" => {
            if x() <= 0.0 {
                Value::Missing(undefined("log of a non-positive"))
            } else {
                Value::num(x().ln(), op)
            }
        }
        "exp" => Value::num(x().exp(), op),
        "sign" => Value::num(
            if x() > 0.0 {
                1.0
            } else if x() < 0.0 {
                -1.0
            } else {
                0.0
            },
            op,
        ),
        "pow" => Value::num(num(args, "base").powf(num(args, "exp")), op),
        "min" => Value::num(a().min(b()), op),
        "max" => Value::num(a().max(b()), op),
        "clip" => {
            let (lo, hi) = (num(args, "lo"), num(args, "hi"));
            if lo > hi {
                Value::Missing(undefined("clip with lo above hi"))
            } else {
                Value::num(x().clamp(lo, hi), op)
            }
        }
        "round" => {
            let p = 10f64.powf(num(args, "digits"));
            Value::num((x() * p).round() / p, op)
        }
        "gt" => Value::Bool(a() > b()),
        "lt" => Value::Bool(a() < b()),
        "ge" => Value::Bool(a() >= b()),
        "le" => Value::Bool(a() <= b()),
        #[allow(clippy::float_cmp)]
        "eq" => Value::Bool(a() == b()),
        #[allow(clippy::float_cmp)]
        "ne" => Value::Bool(a() != b()),
        "between" => Value::Bool(num(args, "lo") <= x() && x() <= num(args, "hi")),
        "is" => match (
            args.get("x").map(ArgVal::one),
            args.get("label").map(ArgVal::one),
        ) {
            (Some(Value::Cat(c)), Some(Value::Text(l))) => Value::Bool(c == l),
            _ => Value::Missing(undefined("is")),
        },
        "argmax" => match args.get("p").map(ArgVal::one) {
            Some(Value::Probs { labels, p }) => {
                let best =
                    p.iter()
                        .enumerate()
                        .fold(None::<(usize, f64)>, |acc, (i, v)| match acc {
                            Some((_, bv)) if bv >= v.get() => acc,
                            _ => Some((i, v.get())),
                        });
                match best.and_then(|(i, _)| labels.get(i)) {
                    Some(l) => Value::Cat(l.clone()),
                    None => Value::Missing(undefined("argmax of no classes")),
                }
            }
            _ => Value::Missing(undefined("argmax")),
        },
        "prob" => match (
            args.get("p").map(ArgVal::one),
            args.get("label").map(ArgVal::one),
        ) {
            (Some(Value::Probs { labels, p }), Some(Value::Text(l))) => {
                match labels.iter().position(|x| x == l) {
                    Some(i) => p
                        .get(i)
                        .map_or(Value::Missing(undefined("prob")), |v| Value::Num(*v)),
                    None => Value::Missing(undefined(&format!("prob: no class '{l}'"))),
                }
            }
            _ => Value::Missing(undefined("prob")),
        },
        "quantile" => match args.get("d").map(ArgVal::one) {
            Some(Value::Dist { levels, values }) => {
                match quantile(levels, values, num(args, "q")) {
                    Some(v) => Value::num(v, op),
                    None => Value::Missing(undefined("quantile of an empty distribution")),
                }
            }
            _ => Value::Missing(undefined("quantile")),
        },
        "mean" => match args.get("d").map(ArgVal::one) {
            Some(Value::Dist { levels, values }) => match dist_mean(levels, values) {
                Some(v) => Value::num(v, op),
                None => Value::Missing(undefined(
                    "mean of a distribution with fewer than two quantiles",
                )),
            },
            _ => Value::Missing(undefined("mean")),
        },
        "prob_above" => match args.get("d").map(ArgVal::one) {
            Some(Value::Dist { levels, values }) => match prob_above(levels, values, x()) {
                Some(v) => Value::num(v, op),
                None => Value::Missing(undefined("prob_above of an empty distribution")),
            },
            _ => Value::Missing(undefined("prob_above")),
        },
        "cosine" => match (
            args.get("a").map(ArgVal::one),
            args.get("b").map(ArgVal::one),
        ) {
            (Some(Value::Vec(u)), Some(Value::Vec(v))) if u.len() == v.len() && !u.is_empty() => {
                let dot: f64 = u.iter().zip(v).map(|(a, b)| a.get() * b.get()).sum();
                let nu = u.iter().map(|a| a.get() * a.get()).sum::<f64>().sqrt();
                let nv = v.iter().map(|a| a.get() * a.get()).sum::<f64>().sqrt();
                if nu == 0.0 || nv == 0.0 {
                    Value::Missing(undefined("cosine of a zero vector"))
                } else {
                    Value::num(dot / (nu * nv), op)
                }
            }
            _ => Value::Missing(undefined("cosine of vectors of different lengths")),
        },
        "weighted_mean" => match (args.get("xs"), args.get("weights")) {
            (Some(ArgVal::Many(xs)), Some(ArgVal::Many(ws))) if xs.len() == ws.len() => {
                let (mut sum, mut wsum) = (0.0, 0.0);
                for (xv, wv) in xs.iter().zip(ws) {
                    let (Some(x), Some(w)) = (xv.as_num(), wv.as_num()) else {
                        return Value::Missing(undefined("weighted_mean"));
                    };
                    sum += w * x;
                    wsum += w;
                }
                if wsum == 0.0 {
                    Value::Missing(undefined("weighted_mean with weights summing to zero"))
                } else {
                    Value::num(sum / wsum, op)
                }
            }
            _ => Value::Missing(undefined("weighted_mean")),
        },
        other => Value::Missing(undefined(&format!("{other} is not a stateless operator"))),
    }
}

fn undefined(op: &str) -> MissingReason {
    MissingReason::Undefined { op: op.to_string() }
}

/// Linear interpolation of the quantile function at `q`, clamped to the end
/// quantiles outside the declared levels — the distribution says nothing beyond
/// them, and extrapolating would invent tail behaviour.
pub(crate) fn quantile(levels: &[Level], values: &[Num], q: f64) -> Option<f64> {
    let pts: Vec<(f64, f64)> = levels
        .iter()
        .zip(values)
        .map(|(l, v)| (l.as_f64(), v.get()))
        .collect();
    let first = *pts.first()?;
    let last = *pts.last()?;
    if q <= first.0 {
        return Some(first.1);
    }
    if q >= last.0 {
        return Some(last.1);
    }
    for w in pts.windows(2) {
        let ((l0, v0), (l1, v1)) = (w[0], w[1]);
        if q <= l1 {
            let t = if l1 > l0 { (q - l0) / (l1 - l0) } else { 0.0 };
            return Some(v0 + t * (v1 - v0));
        }
    }
    Some(last.1)
}

/// Mean of the distribution truncated to its declared levels: the integral of
/// the piecewise-linear quantile function, normalised by the probability mass it
/// covers. Exact for a symmetric distribution whose levels are symmetric.
pub(crate) fn dist_mean(levels: &[Level], values: &[Num]) -> Option<f64> {
    if levels.len() < 2 || levels.len() != values.len() {
        return None;
    }
    let (mut area, mut mass) = (0.0, 0.0);
    for i in 0..levels.len() - 1 {
        let dl = levels[i + 1].as_f64() - levels[i].as_f64();
        area += dl * (values[i].get() + values[i + 1].get()) / 2.0;
        mass += dl;
    }
    (mass > 0.0).then(|| area / mass)
}

/// P(X > x) from the piecewise-linear CDF through the quantiles. Outside the
/// quantile range the answer is bounded by the end levels: below the lowest
/// quantile, at least 1 − its level; above the highest, at most 1 − its level.
pub(crate) fn prob_above(levels: &[Level], values: &[Num], x: f64) -> Option<f64> {
    let pts: Vec<(f64, f64)> = levels
        .iter()
        .zip(values)
        .map(|(l, v)| (l.as_f64(), v.get()))
        .collect();
    let first = *pts.first()?;
    let last = *pts.last()?;
    if x <= first.1 {
        return Some(1.0 - first.0);
    }
    if x >= last.1 {
        return Some(1.0 - last.0);
    }
    for w in pts.windows(2) {
        let ((l0, v0), (l1, v1)) = (w[0], w[1]);
        if x <= v1 {
            let t = if v1 > v0 { (x - v0) / (v1 - v0) } else { 1.0 };
            return Some(1.0 - (l0 + t * (l1 - l0)));
        }
    }
    Some(1.0 - last.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64) -> ArgVal {
        ArgVal::One(Value::Num(Num::new(x).unwrap()))
    }
    fn b(x: bool) -> ArgVal {
        ArgVal::One(Value::Bool(x))
    }
    fn miss(tag: &str) -> ArgVal {
        ArgVal::One(Value::Missing(MissingReason::NoData { what: tag.into() }))
    }
    fn args(xs: &[(&str, ArgVal)]) -> BTreeMap<String, ArgVal> {
        xs.iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn arithmetic_on_a_missing_input_carries_the_root_reason() {
        let out = eval(
            "add",
            &["a", "b"],
            &args(&[("a", n(1.0)), ("b", miss("vol"))]),
        );
        assert_eq!(
            out,
            Value::Missing(MissingReason::NoData { what: "vol".into() })
        );
        // Both missing: the first port's reason, deterministically.
        let out = eval(
            "mul",
            &["a", "b"],
            &args(&[("a", miss("first")), ("b", miss("second"))]),
        );
        assert_eq!(
            out,
            Value::Missing(MissingReason::NoData {
                what: "first".into()
            })
        );
    }

    #[test]
    fn undefined_arithmetic_is_missing_not_nan() {
        for (op, xs) in [
            ("div", args(&[("a", n(1.0)), ("b", n(0.0))])),
            ("sqrt", args(&[("x", n(-1.0))])),
            ("log", args(&[("x", n(0.0))])),
            ("exp", args(&[("x", n(1e6))])),
        ] {
            let v = eval(op, &["a", "b", "x"], &xs);
            assert!(v.is_missing(), "{op} gave {v:?}");
        }
    }

    #[test]
    fn logic_is_kleene() {
        let f_and_m = eval(
            "and",
            &["a", "b"],
            &args(&[("a", b(false)), ("b", miss("m"))]),
        );
        assert_eq!(f_and_m, Value::Bool(false), "false and missing is false");
        let t_or_m = eval(
            "or",
            &["a", "b"],
            &args(&[("a", b(true)), ("b", miss("m"))]),
        );
        assert_eq!(t_or_m, Value::Bool(true), "true or missing is true");
        assert!(eval(
            "and",
            &["a", "b"],
            &args(&[("a", b(true)), ("b", miss("m"))])
        )
        .is_missing());
    }

    #[test]
    fn if_takes_the_chosen_branch_whatever_the_other_holds() {
        let v = eval(
            "if",
            &["cond", "then", "else"],
            &args(&[
                ("cond", b(true)),
                ("then", n(2.0)),
                ("else", miss("unused")),
            ]),
        );
        assert_eq!(v.as_num(), Some(2.0));
        assert!(eval(
            "if",
            &["cond"],
            &args(&[("cond", miss("c")), ("then", n(1.0)), ("else", n(2.0))])
        )
        .is_missing());
    }

    #[test]
    fn coalesce_is_the_only_way_to_turn_missing_into_a_value() {
        let v = eval(
            "coalesce",
            &["x", "default"],
            &args(&[("x", miss("m")), ("default", n(3.0))]),
        );
        assert_eq!(v.as_num(), Some(3.0));
        assert_eq!(
            eval("is_missing", &["x"], &args(&[("x", miss("m"))])),
            Value::Bool(true)
        );
        assert_eq!(
            eval("is_missing", &["x"], &args(&[("x", n(0.0))])),
            Value::Bool(false)
        );
    }

    #[test]
    fn vote_is_kleene_consistent() {
        let xs = |v: Vec<ArgVal>| ArgVal::Many(v.into_iter().map(|a| a.one().clone()).collect());
        let k = n(2.0);
        let two_true = args(&[
            ("xs", xs(vec![b(true), b(true), miss("m")])),
            ("k", k.clone()),
        ]);
        assert_eq!(eval("vote", &["xs", "k"], &two_true), Value::Bool(true));
        let cannot_reach = args(&[
            ("xs", xs(vec![b(false), b(false), miss("m")])),
            ("k", k.clone()),
        ]);
        assert_eq!(
            eval("vote", &["xs", "k"], &cannot_reach),
            Value::Bool(false)
        );
        let depends = args(&[("xs", xs(vec![b(true), b(false), miss("m")])), ("k", k)]);
        assert!(eval("vote", &["xs", "k"], &depends).is_missing());
    }

    #[test]
    fn a_classifier_is_read_without_reshaping() {
        let p = ArgVal::One(Value::Probs {
            labels: vec!["down".into(), "flat".into(), "up".into()],
            p: [0.1, 0.2, 0.7]
                .iter()
                .map(|x| Num::new(*x).unwrap())
                .collect(),
        });
        assert_eq!(
            eval("argmax", &["p"], &args(&[("p", p.clone())])),
            Value::Cat("up".into())
        );
        let up = eval(
            "prob",
            &["p", "label"],
            &args(&[("p", p), ("label", ArgVal::One(Value::Text("up".into())))]),
        );
        assert_eq!(up.as_num(), Some(0.7));
    }

    #[test]
    fn distribution_adapters_interpolate_and_clamp() {
        let levels: Vec<Level> = [0.1, 0.5, 0.9]
            .iter()
            .map(|p| Level::new(*p).unwrap())
            .collect();
        let values: Vec<Num> = [-1.0, 0.0, 1.0]
            .iter()
            .map(|x| Num::new(*x).unwrap())
            .collect();
        assert_eq!(quantile(&levels, &values, 0.5), Some(0.0));
        assert_eq!(quantile(&levels, &values, 0.3), Some(-0.5));
        assert_eq!(
            quantile(&levels, &values, 0.01),
            Some(-1.0),
            "clamped, not extrapolated"
        );
        assert_eq!(
            dist_mean(&levels, &values),
            Some(0.0),
            "symmetric → zero mean"
        );
        assert!((prob_above(&levels, &values, 0.0).unwrap() - 0.5).abs() < 1e-12);
        assert!((prob_above(&levels, &values, 5.0).unwrap() - 0.1).abs() < 1e-12);
    }

    #[test]
    fn cosine_needs_matching_non_zero_vectors() {
        let v = |xs: &[f64]| {
            ArgVal::One(Value::Vec(
                xs.iter().map(|x| Num::new(*x).unwrap()).collect(),
            ))
        };
        let same = eval(
            "cosine",
            &["a", "b"],
            &args(&[("a", v(&[1.0, 0.0])), ("b", v(&[2.0, 0.0]))]),
        );
        assert_eq!(same.as_num(), Some(1.0));
        assert!(eval(
            "cosine",
            &["a", "b"],
            &args(&[("a", v(&[1.0])), ("b", v(&[1.0, 2.0]))])
        )
        .is_missing());
    }
}
