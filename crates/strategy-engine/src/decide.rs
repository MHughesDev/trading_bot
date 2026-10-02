//! Rules → resolved intents (FEAT-007 §6, U-1.7).
//!
//! Rules are matched first-wins **within a group**, in rule order. A rule whose
//! condition is missing does not match — only `true` matches (Kleene). A matched
//! rule whose intent needs a value that is missing becomes a **hold** carrying the
//! missing reasons (QA-5): the strategy never trades on a value it does not have,
//! and the journal says exactly which value and why.

use domain::strategy_graph::graph::Arg;
use domain::strategy_graph::intent::{
    Distance, DistanceUnit, Entry, OrderSpec, OrderStyle, SizeSpec, TradeIntent,
};
use domain::strategy_graph::{MissingReason, Value};
use serde::{Deserialize, Serialize};

use crate::planner::{RDist, REntry, ROrder, RSize, RStyle, RUnit, ResolvedIntent};

/// What one rule did this tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RuleOutcome {
    /// The condition was false.
    NotMatched,
    /// The condition was unknown, so the rule did not match.
    Unknown { reason: MissingReason },
    /// An earlier rule in the same group matched first.
    Shadowed,
    /// Matched and resolved.
    Fired,
    /// Matched, but a value its intent needs is missing: held.
    Held { reasons: Vec<MissingReason> },
}

/// Resolve an intent template. `operand(k)` reads the k-th compiled operand;
/// `atr(n)` reads ATR(n) on the bound instrument.
pub(crate) fn resolve(
    template: &TradeIntent,
    operand: &dyn Fn(usize) -> Value,
    atr: &dyn Fn(u32) -> Value,
) -> Result<ResolvedIntent, Vec<MissingReason>> {
    let mut r = Resolver {
        operand,
        atr,
        missing: Vec::new(),
    };
    let out = r.intent(template);
    if r.missing.is_empty() {
        Ok(out)
    } else {
        Err(r.missing)
    }
}

struct Resolver<'a> {
    operand: &'a dyn Fn(usize) -> Value,
    atr: &'a dyn Fn(u32) -> Value,
    missing: Vec<MissingReason>,
}

impl Resolver<'_> {
    fn num(&mut self, a: &Arg) -> f64 {
        let v = match a {
            Arg::Num(x) => return *x,
            Arg::Ref(r) => match r.strip_prefix('#').and_then(|k| k.parse::<usize>().ok()) {
                Some(k) => (self.operand)(k),
                None => Value::Missing(MissingReason::Undefined {
                    op: format!("unresolved intent value {r}"),
                }),
            },
            _ => Value::Missing(MissingReason::Undefined {
                op: "intent value is not a number".into(),
            }),
        };
        match v {
            Value::Num(n) => n.get(),
            Value::Missing(m) => {
                if !self.missing.contains(&m) {
                    self.missing.push(m);
                }
                0.0
            }
            other => {
                self.missing.push(MissingReason::Undefined {
                    op: format!("intent value is a {}", other.kind_name()),
                });
                0.0
            }
        }
    }

    fn dist(&mut self, d: &Distance) -> RDist {
        let amount = self.num(&d.amount);
        let unit = match &d.unit {
            DistanceUnit::Pct => RUnit::Pct,
            DistanceUnit::Abs => RUnit::Abs,
            DistanceUnit::Atr { n } => match (self.atr)(*n) {
                Value::Num(a) => RUnit::Atr { atr: a.get() },
                Value::Missing(m) => {
                    self.missing.push(m);
                    RUnit::Atr { atr: 0.0 }
                }
                _ => RUnit::Atr { atr: 0.0 },
            },
            DistanceUnit::Sigma { of } => RUnit::Sigma {
                sigma: self.num(of),
            },
        };
        RDist {
            amount,
            unit,
            from: d.from,
        }
    }

    fn size(&mut self, s: &SizeSpec) -> RSize {
        match s {
            SizeSpec::Units { x } => RSize::Units { x: self.num(x) },
            SizeSpec::Notional { x } => RSize::Notional { x: self.num(x) },
            SizeSpec::EquityFraction { x } => RSize::EquityFraction { x: self.num(x) },
            SizeSpec::Risk { fraction, stop } => RSize::Risk {
                fraction: self.num(fraction),
                stop: self.dist(stop),
            },
            SizeSpec::VolTarget { target, vol } => RSize::VolTarget {
                target: self.num(target),
                vol: self.num(vol),
            },
        }
    }

    fn style(&mut self, s: &OrderStyle) -> RStyle {
        match s {
            OrderStyle::Market => RStyle::Market,
            OrderStyle::Limit { price } => RStyle::Limit {
                price: self.num(price),
            },
            OrderStyle::Stop { trigger } => RStyle::Stop {
                trigger: self.num(trigger),
            },
            OrderStyle::StopLimit { trigger, price } => RStyle::StopLimit {
                trigger: self.num(trigger),
                price: self.num(price),
            },
            OrderStyle::TrailingStop { trail } => RStyle::TrailingStop {
                trail: self.dist(trail),
            },
        }
    }

    fn order(&mut self, o: &OrderSpec) -> ROrder {
        ROrder {
            side: o.side,
            size: self.size(&o.size),
            style: self.style(&o.style),
            tif: o.tif,
            reduce_only: o.reduce_only,
            post_only: o.post_only,
            tag: o.tag.clone(),
        }
    }

    fn via(&mut self, v: Option<&OrderStyle>) -> Option<RStyle> {
        v.map(|s| self.style(s))
    }

    fn intent(&mut self, i: &TradeIntent) -> ResolvedIntent {
        match i {
            TradeIntent::Hold => ResolvedIntent::Hold,
            TradeIntent::Target { size, via } => ResolvedIntent::Target {
                size: self.size(size),
                via: self.via(via.as_ref()),
            },
            TradeIntent::Order(o) => ResolvedIntent::Order(self.order(o)),
            TradeIntent::Bracket(b) => ResolvedIntent::Bracket {
                entry: match &b.entry {
                    Entry::Target { size, via } => REntry::Target {
                        size: self.size(size),
                        via: self.via(via.as_ref()),
                    },
                    Entry::Order(o) => REntry::Order(self.order(o)),
                },
                stop: self.dist(&b.stop),
                take_profit: b.take_profit.as_ref().map(|d| self.dist(d)),
                trailing: b.trailing.as_ref().map(|d| self.dist(d)),
            },
            TradeIntent::Protect(p) => ResolvedIntent::Protect {
                stop: p.stop.as_ref().map(|d| self.dist(d)),
                take_profit: p.take_profit.as_ref().map(|d| self.dist(d)),
                trailing: p.trailing.as_ref().map(|d| self.dist(d)),
            },
            TradeIntent::Close { fraction, via } => ResolvedIntent::Close {
                fraction: self.num(fraction),
                via: self.via(via.as_ref()),
            },
            TradeIntent::Add { size, via } => ResolvedIntent::Add {
                size: self.size(size),
                via: self.via(via.as_ref()),
            },
            TradeIntent::Reduce { size, via } => ResolvedIntent::Reduce {
                size: self.size(size),
                via: self.via(via.as_ref()),
            },
            TradeIntent::Cancel { tag } => ResolvedIntent::Cancel { tag: tag.clone() },
        }
    }
}
