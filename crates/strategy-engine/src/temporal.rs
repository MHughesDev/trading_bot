//! Operators that remember (FEAT-007 §5.3 "over time").
//!
//! Each stateful node owns one [`Temporal`] value, advanced **exactly once per
//! decision tick** — never on a model resume, which re-enters the same tick
//! (U-1.4). A window that contains a missing value is missing, with that value's
//! reason, until the missing value has rolled out of it; a window that is not yet
//! full is `warming_up` with how many ticks it has and needs.

use std::collections::VecDeque;

use domain::strategy_graph::{MissingReason, Value};

type Cell = Result<f64, MissingReason>;

/// The state of one stateful node.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Temporal {
    /// `prev`: the last `n + 1` values, oldest first.
    Prev {
        n: usize,
        buf: VecDeque<Value>,
    },
    /// `change`, `rising`, `falling`, `rolling_*`, `zscore`, `pct_rank`,
    /// `count_true` (stored as 1/0): the last `len` cells.
    Window {
        len: usize,
        buf: VecDeque<Cell>,
    },
    /// `crossed_above` / `crossed_below`: the previous pair.
    Cross {
        prev: Option<(f64, f64)>,
    },
    /// `ema` over a node output.
    Ema {
        n: u32,
        value: Option<f64>,
        seen: u32,
    },
    /// `bars_since`: ticks since the condition was last true.
    BarsSince {
        since: Option<u64>,
    },
    Latch {
        on: bool,
    },
    Hysteresis {
        on: bool,
    },
    /// `hold_last`: the last present value and the tick it arrived.
    HoldLast {
        last: Option<(Value, u64)>,
    },
    /// `cooldown`: the tick it last fired.
    Cooldown {
        n: u64,
        last: Option<u64>,
    },
}

impl Temporal {
    /// Fresh state for `op` with period `n` (ignored where meaningless).
    #[must_use]
    pub fn new(op: &str, n: u32) -> Self {
        let n_us = n as usize;
        match op {
            "prev" => Self::Prev {
                n: n_us,
                buf: VecDeque::new(),
            },
            // `change` and `rising`/`falling` compare against n ticks back: n + 1 cells.
            "change" | "rising" | "falling" => Self::Window {
                len: n_us + 1,
                buf: VecDeque::new(),
            },
            "crossed_above" | "crossed_below" => Self::Cross { prev: None },
            "ema" => Self::Ema {
                n,
                value: None,
                seen: 0,
            },
            "bars_since" => Self::BarsSince { since: None },
            "latch" => Self::Latch { on: false },
            "hysteresis" => Self::Hysteresis { on: false },
            "hold_last" => Self::HoldLast { last: None },
            "cooldown" => Self::Cooldown {
                n: u64::from(n),
                last: None,
            },
            _ => Self::Window {
                len: n_us.max(1),
                buf: VecDeque::new(),
            },
        }
    }

    /// Advance one tick and return this tick's output.
    ///
    /// `get(port)` reads the node's evaluated inputs.
    #[allow(clippy::too_many_lines)]
    pub fn step(&mut self, op: &str, tick: u64, get: &dyn Fn(&str) -> Value) -> Value {
        match self {
            Self::Prev { n, buf } => {
                buf.push_back(get("x"));
                while buf.len() > *n + 1 {
                    buf.pop_front();
                }
                if buf.len() <= *n {
                    return warming(*n + 1, buf.len());
                }
                buf.front().cloned().unwrap_or_else(|| warming(*n + 1, 0))
            }
            Self::Window { len, buf } => {
                let cell: Cell = match op {
                    "count_true" => match get("cond") {
                        Value::Bool(b) => Ok(if b { 1.0 } else { 0.0 }),
                        Value::Missing(r) => Err(r),
                        _ => Err(undefined(op)),
                    },
                    _ => match get("x") {
                        Value::Num(x) => Ok(x.get()),
                        Value::Missing(r) => Err(r),
                        _ => Err(undefined(op)),
                    },
                };
                buf.push_back(cell);
                while buf.len() > *len {
                    buf.pop_front();
                }
                if buf.len() < *len {
                    return warming(*len, buf.len());
                }
                // The most recent missing value's reason, if any is in the window.
                if let Some(Err(r)) = buf.iter().rev().find(|c| c.is_err()) {
                    return Value::Missing(r.clone());
                }
                let xs: Vec<f64> = buf
                    .iter()
                    .filter_map(|c| c.as_ref().ok().copied())
                    .collect();
                window_value(op, &xs)
            }
            Self::Cross { prev } => {
                let (a, b) = (get("a"), get("b"));
                let (Some(a), Some(b)) = (a.as_num(), b.as_num()) else {
                    *prev = None;
                    let r = a
                        .missing_reason()
                        .or_else(|| b.missing_reason())
                        .cloned()
                        .unwrap_or_else(|| undefined(op));
                    return Value::Missing(r);
                };
                let out = match *prev {
                    None => warming(2, 1),
                    Some((pa, pb)) => Value::Bool(if op == "crossed_above" {
                        pa <= pb && a > b
                    } else {
                        pa >= pb && a < b
                    }),
                };
                *prev = Some((a, b));
                out
            }
            Self::Ema { n, value, seen } => {
                let x = get("x");
                let Some(x) = x.as_num() else {
                    return x;
                };
                let alpha = 2.0 / (f64::from(*n) + 1.0);
                *value = Some(value.map_or(x, |v| v + alpha * (x - v)));
                *seen += 1;
                if *seen < *n {
                    return warming(*n as usize, *seen as usize);
                }
                Value::num(value.unwrap_or(x), op)
            }
            Self::BarsSince { since } => match get("cond") {
                Value::Bool(true) => {
                    *since = Some(tick);
                    num(0.0)
                }
                Value::Bool(false) => since.map_or_else(
                    || {
                        Value::Missing(MissingReason::NoData {
                            what: "the condition has not been true yet".into(),
                        })
                    },
                    #[allow(clippy::cast_precision_loss)]
                    |s| num((tick - s) as f64),
                ),
                Value::Missing(r) => Value::Missing(r),
                _ => Value::Missing(undefined(op)),
            },
            Self::Latch { on } => match (get("set"), get("reset")) {
                (_, Value::Bool(true)) => {
                    *on = false;
                    Value::Bool(false)
                }
                (Value::Bool(true), Value::Bool(false)) => {
                    *on = true;
                    Value::Bool(true)
                }
                (Value::Bool(false), Value::Bool(false)) => Value::Bool(*on),
                (Value::Missing(r), _) | (_, Value::Missing(r)) => Value::Missing(r),
                _ => Value::Missing(undefined(op)),
            },
            Self::Hysteresis { on } => {
                let (x, hi, lo) = (get("x"), get("on"), get("off"));
                match (x.as_num(), hi.as_num(), lo.as_num()) {
                    (Some(x), Some(hi), Some(lo)) => {
                        if x > hi {
                            *on = true;
                        } else if x < lo {
                            *on = false;
                        }
                        Value::Bool(*on)
                    }
                    _ => Value::Missing(
                        [x, hi, lo]
                            .iter()
                            .find_map(Value::missing_reason)
                            .cloned()
                            .unwrap_or_else(|| undefined(op)),
                    ),
                }
            }
            Self::HoldLast { last } => {
                let x = get("x");
                if x.is_missing() {
                    last.as_ref().map_or(x, |(v, _)| v.clone())
                } else {
                    *last = Some((x.clone(), tick));
                    x
                }
            }
            Self::Cooldown { n, last } => match get("cond") {
                Value::Bool(true) if last.is_none_or(|l| tick - l >= *n) => {
                    *last = Some(tick);
                    Value::Bool(true)
                }
                Value::Bool(_) => Value::Bool(false),
                Value::Missing(r) => Value::Missing(r),
                _ => Value::Missing(undefined(op)),
            },
        }
    }
}

fn num(x: f64) -> Value {
    Value::num(x, "temporal")
}

fn warming(needed: usize, have: usize) -> Value {
    Value::Missing(MissingReason::WarmingUp {
        needed: u32::try_from(needed).unwrap_or(u32::MAX),
        have: u32::try_from(have).unwrap_or(u32::MAX),
    })
}

fn undefined(op: &str) -> MissingReason {
    MissingReason::Undefined { op: op.to_string() }
}

/// A full window's value. `xs` is oldest first.
#[allow(clippy::cast_precision_loss)]
fn window_value(op: &str, xs: &[f64]) -> Value {
    let n = xs.len() as f64;
    let last = xs.last().copied().unwrap_or(f64::NAN);
    match op {
        "change" => num(last - xs[0]),
        "rising" => Value::Bool(xs.windows(2).all(|w| w[1] > w[0])),
        "falling" => Value::Bool(xs.windows(2).all(|w| w[1] < w[0])),
        "rolling_mean" => num(xs.iter().sum::<f64>() / n),
        "rolling_sum" | "count_true" => num(xs.iter().sum::<f64>()),
        "rolling_min" => num(xs.iter().copied().fold(f64::INFINITY, f64::min)),
        "rolling_max" => num(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        "rolling_std" => {
            if xs.len() < 2 {
                return Value::Missing(undefined("rolling_std of one value"));
            }
            let m = xs.iter().sum::<f64>() / n;
            num((xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt())
        }
        "zscore" => {
            if xs.len() < 2 {
                return Value::Missing(undefined("zscore of one value"));
            }
            let m = xs.iter().sum::<f64>() / n;
            let sd = (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
            if sd == 0.0 {
                Value::Missing(undefined("zscore of a constant series"))
            } else {
                num((last - m) / sd)
            }
        }
        "pct_rank" => {
            let below = xs.iter().filter(|x| **x < last).count() as f64;
            #[allow(clippy::float_cmp)]
            let equal = xs.iter().filter(|x| **x == last).count() as f64;
            // Mid-rank for ties, normalised to [0, 1].
            num(if n <= 1.0 {
                0.5
            } else {
                (below + (equal - 1.0) / 2.0) / (n - 1.0)
            })
        }
        other => Value::Missing(undefined(&format!("{other} is not a window operator"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_graph::Num;

    fn run(op: &str, n: u32, inputs: &[(&str, Value)], ticks: usize) -> Vec<Value> {
        let mut t = Temporal::new(op, n);
        (0..ticks)
            .map(|i| {
                let get = |p: &str| {
                    inputs
                        .iter()
                        .find(|(k, _)| *k == p)
                        .map_or(Value::Missing(undefined("absent")), |(_, v)| v.clone())
                };
                let _ = i;
                t.step(op, i as u64, &get)
            })
            .collect()
    }

    fn series(op: &str, n: u32, xs: &[f64]) -> Vec<Value> {
        let mut t = Temporal::new(op, n);
        xs.iter()
            .enumerate()
            .map(|(i, x)| {
                let v = Value::Num(Num::new(*x).unwrap());
                t.step(op, i as u64, &|_| v.clone())
            })
            .collect()
    }

    #[test]
    fn crossed_above_fires_once_per_crossing() {
        let mut t = Temporal::new("crossed_above", 0);
        let pairs = [
            (1.0, 2.0),
            (1.5, 2.0),
            (2.5, 2.0),
            (3.0, 2.0),
            (1.0, 2.0),
            (2.1, 2.0),
        ];
        let fired: Vec<bool> = pairs
            .iter()
            .enumerate()
            .map(|(i, (a, b))| {
                let (a, b) = (
                    Value::Num(Num::new(*a).unwrap()),
                    Value::Num(Num::new(*b).unwrap()),
                );
                let get = |p: &str| if p == "a" { a.clone() } else { b.clone() };
                matches!(t.step("crossed_above", i as u64, &get), Value::Bool(true))
            })
            .collect();
        assert_eq!(fired, vec![false, false, true, false, false, true]);
    }

    #[test]
    fn a_window_is_warming_until_full_then_present() {
        let out = series("rolling_mean", 3, &[1.0, 2.0, 3.0, 4.0]);
        assert!(matches!(
            out[0],
            Value::Missing(MissingReason::WarmingUp { needed: 3, have: 1 })
        ));
        assert!(matches!(
            out[1],
            Value::Missing(MissingReason::WarmingUp { needed: 3, have: 2 })
        ));
        assert_eq!(out[2].as_num(), Some(2.0));
        assert_eq!(out[3].as_num(), Some(3.0));
    }

    #[test]
    fn a_missing_value_poisons_the_window_until_it_rolls_out() {
        let mut t = Temporal::new("rolling_sum", 2);
        let seq = [
            Value::Num(Num::new(1.0).unwrap()),
            Value::Missing(MissingReason::NoData { what: "gap".into() }),
            Value::Num(Num::new(1.0).unwrap()),
            Value::Num(Num::new(1.0).unwrap()),
        ];
        let out: Vec<Value> = seq
            .iter()
            .enumerate()
            .map(|(i, v)| t.step("rolling_sum", i as u64, &|_| v.clone()))
            .collect();
        assert!(out[1].is_missing());
        assert!(matches!(&out[2], Value::Missing(MissingReason::NoData { what }) if what == "gap"));
        assert_eq!(out[3].as_num(), Some(2.0), "rolled out");
    }

    #[test]
    fn prev_returns_the_value_n_ticks_ago() {
        let out = series("prev", 2, &[10.0, 20.0, 30.0, 40.0]);
        assert!(out[0].is_missing() && out[1].is_missing());
        assert_eq!(out[2].as_num(), Some(10.0));
        assert_eq!(out[3].as_num(), Some(20.0));
    }

    #[test]
    fn latch_holds_until_reset() {
        let mut t = Temporal::new("latch", 0);
        let seq = [
            (true, false),
            (false, false),
            (false, false),
            (false, true),
            (false, false),
        ];
        let out: Vec<bool> = seq
            .iter()
            .enumerate()
            .map(|(i, (s, r))| {
                let (s, r) = (Value::Bool(*s), Value::Bool(*r));
                matches!(
                    t.step("latch", i as u64, &|p| if p == "set" {
                        s.clone()
                    } else {
                        r.clone()
                    }),
                    Value::Bool(true)
                )
            })
            .collect();
        assert_eq!(out, vec![true, true, true, false, false]);
    }

    #[test]
    fn hold_last_bridges_a_gap_and_cooldown_spaces_firings() {
        let mut h = Temporal::new("hold_last", 0);
        let a = Value::Num(Num::new(5.0).unwrap());
        assert_eq!(h.step("hold_last", 0, &|_| a.clone()).as_num(), Some(5.0));
        let gap = Value::Missing(MissingReason::ModelTimeout {
            model: "m".into(),
            timeout_ms: 1,
        });
        assert_eq!(h.step("hold_last", 1, &|_| gap.clone()).as_num(), Some(5.0));

        let out = run("cooldown", 3, &[("cond", Value::Bool(true))], 7);
        let fired: Vec<bool> = out.iter().map(|v| matches!(v, Value::Bool(true))).collect();
        assert_eq!(fired, vec![true, false, false, true, false, false, true]);
    }

    #[test]
    fn rising_compares_n_ticks() {
        let out = series("rising", 2, &[1.0, 2.0, 3.0, 2.5]);
        assert!(out[1].is_missing());
        assert_eq!(out[2], Value::Bool(true));
        assert_eq!(out[3], Value::Bool(false));
    }
}
