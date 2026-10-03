//! Trade intents — what a trading strategy wants (FEAT-007 §6, ADR-0041).
//!
//! A strategy says *what it wants*; one shared order planner, in the engine and
//! therefore identical in every environment, works out the orders. Every number
//! in an intent — size, limit price, stop distance, trail — is an [`Arg`], i.e. a
//! graph value, so a model's quantile can set a limit price and a model's
//! fraction can set a size.
//!
//! What this module refuses at deserialisation is what no planner could carry
//! out: a limit without a price, a bracket without a stop, a close of a literal
//! fraction outside (0, 1]. Everything that depends on a value is the planner's
//! to refuse at run time.

use serde::{Deserialize, Serialize};

use super::graph::Arg;
use crate::order::{Side, TimeInForce};

/// What a trading strategy wants this tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TradeIntent {
    /// Change nothing. Working orders stay working.
    Hold,
    /// Be at this position. Idempotent: re-stating an unchanged target emits no
    /// order, which is what makes a restart safe.
    Target {
        size: SizeSpec,
        /// How to get there. Market when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<OrderStyle>,
    },
    /// Place exactly this order.
    Order(OrderSpec),
    /// Enter, then protect: when the entry fills, an OCO pair (stop and optional
    /// take-profit, or a trailing stop) is placed against the position. Boxed:
    /// it is several times the size of every other variant.
    Bracket(Box<Bracket>),
    /// Set or replace protection on the open position — e.g. move the stop to
    /// breakeven. Requires at least one of the three.
    Protect(Protect),
    /// Exit all or part of the position. `fraction` defaults to 1.
    Close {
        #[serde(default = "whole", skip_serializing_if = "is_whole")]
        fraction: Arg,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<OrderStyle>,
    },
    /// Scale in, in the direction of the current position.
    Add {
        size: SizeSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<OrderStyle>,
    },
    /// Scale out. Reduce-only: can never reverse the position.
    Reduce {
        size: SizeSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<OrderStyle>,
    },
    /// Withdraw working orders: those carrying `tag`, or all of this
    /// strategy's working orders on this instrument when absent.
    Cancel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    },
}

fn whole() -> Arg {
    Arg::Num(1.0)
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::float_cmp)]
fn is_whole(a: &Arg) -> bool {
    matches!(a, Arg::Num(x) if *x == 1.0)
}

/// How much.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SizeSpec {
    /// Units of the instrument. Signed for `target` (negative = short).
    Units { x: Arg },
    /// Quote-currency value.
    Notional { x: Arg },
    /// A fraction of equity. Signed for `target`.
    EquityFraction { x: Arg },
    /// Risk `fraction` of equity if the stop at `stop` is hit:
    /// size = fraction × equity ÷ stop distance.
    Risk { fraction: Arg, stop: Distance },
    /// Scale to a target volatility: size fraction = target ÷ vol.
    VolTarget { target: Arg, vol: Arg },
}

/// How an order is placed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrderStyle {
    Market,
    Limit {
        price: Arg,
    },
    /// A stop-market order: market once `trigger` trades.
    Stop {
        trigger: Arg,
    },
    StopLimit {
        trigger: Arg,
        price: Arg,
    },
    /// A stop that follows the price at `trail` behind the best price since it
    /// was placed.
    TrailingStop {
        trail: Distance,
    },
}

impl OrderStyle {
    /// Its name, for planner diagnostics and venue capability checks.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Limit { .. } => "limit",
            Self::Stop { .. } => "stop",
            Self::StopLimit { .. } => "stop_limit",
            Self::TrailingStop { .. } => "trailing_stop",
        }
    }
}

/// One fully specified order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderSpec {
    pub side: Side,
    pub size: SizeSpec,
    #[serde(default = "market")]
    pub style: OrderStyle,
    #[serde(default)]
    pub tif: TimeInForce,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reduce_only: bool,
    /// A limit that would take liquidity is rejected rather than filled.
    #[serde(default, skip_serializing_if = "is_false")]
    pub post_only: bool,
    /// A name for this order, so a later `cancel` or `protect` can find it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

const fn market() -> OrderStyle {
    OrderStyle::Market
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(b: &bool) -> bool {
    !*b
}

/// The entry half of a bracket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Target {
        size: SizeSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        via: Option<OrderStyle>,
    },
    Order(OrderSpec),
}

/// Enter and protect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bracket {
    pub entry: Entry,
    /// Required. A bracket without a stop is a plain entry, and calling it a
    /// bracket would let a reader believe the position is protected.
    pub stop: Distance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<Distance>,
    /// When present, the stop trails at this distance instead of resting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing: Option<Distance>,
}

/// Protection on an open position.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Protect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Distance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<Distance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trailing: Option<Distance>,
}

impl<'de> Deserialize<'de> for Protect {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(default)]
            stop: Option<Distance>,
            #[serde(default)]
            take_profit: Option<Distance>,
            #[serde(default)]
            trailing: Option<Distance>,
        }
        let r = Raw::deserialize(d)?;
        if r.stop.is_none() && r.take_profit.is_none() && r.trailing.is_none() {
            return Err(serde::de::Error::custom(
                "`protect` must set at least one of `stop`, `take_profit`, `trailing`",
            ));
        }
        Ok(Self {
            stop: r.stop,
            take_profit: r.take_profit,
            trailing: r.trailing,
        })
    }
}

/// A price distance, e.g. "2 ATR(14) from entry".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distance {
    pub amount: Arg,
    pub unit: DistanceUnit,
    #[serde(default, skip_serializing_if = "Anchor::is_entry")]
    pub from: Anchor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case")]
pub enum DistanceUnit {
    /// Percent of the anchor price: `amount = 2` is 2%.
    Pct,
    /// Price units.
    Abs,
    /// Multiples of the bound instrument's ATR over `n` bars of the decision
    /// timeframe.
    Atr { n: u32 },
    /// Multiples of a volatility value (e.g. a model's `sigma`) times the anchor
    /// price.
    Sigma { of: Arg },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    Entry,
    Mark,
    /// The best price since entry — the anchor of a trailing distance.
    HighWater,
}

impl Anchor {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    const fn is_entry(&self) -> bool {
        matches!(self, Self::Entry)
    }
}

/// Literal checks no run-time value can rescue. Called by the compiler; exposed
/// here so the rule lives beside the type.
///
/// # Errors
/// The first literal that cannot be carried out.
pub fn check_literals(intent: &TradeIntent) -> Result<(), IntentError> {
    if let TradeIntent::Close { fraction, .. } = intent {
        if let Some(f) = fraction.literal_num() {
            if !(f > 0.0 && f <= 1.0) {
                return Err(IntentError::CloseFraction(f));
            }
        }
    }
    for (field, d) in distances(intent) {
        if let Some(x) = d.amount.literal_num() {
            if x <= 0.0 {
                return Err(IntentError::DistanceNotPositive { field, amount: x });
            }
        }
    }
    Ok(())
}

/// Every distance an intent carries, with the field it sits in.
fn distances(intent: &TradeIntent) -> Vec<(&'static str, &Distance)> {
    fn style<'a>(s: Option<&'a OrderStyle>, out: &mut Vec<(&'static str, &'a Distance)>) {
        if let Some(OrderStyle::TrailingStop { trail }) = s {
            out.push(("trail", trail));
        }
    }
    fn size<'a>(s: &'a SizeSpec, out: &mut Vec<(&'static str, &'a Distance)>) {
        if let SizeSpec::Risk { stop, .. } = s {
            out.push(("size.stop", stop));
        }
    }
    let mut out = Vec::new();
    match intent {
        TradeIntent::Target { size: s, via }
        | TradeIntent::Add { size: s, via }
        | TradeIntent::Reduce { size: s, via } => {
            size(s, &mut out);
            style(via.as_ref(), &mut out);
        }
        TradeIntent::Close { via, .. } => style(via.as_ref(), &mut out),
        TradeIntent::Order(o) => {
            size(&o.size, &mut out);
            style(Some(&o.style), &mut out);
        }
        TradeIntent::Bracket(b) => {
            match &b.entry {
                Entry::Target { size: s, via } => {
                    size(s, &mut out);
                    style(via.as_ref(), &mut out);
                }
                Entry::Order(o) => {
                    size(&o.size, &mut out);
                    style(Some(&o.style), &mut out);
                }
            }
            out.push(("stop", &b.stop));
            if let Some(d) = &b.take_profit {
                out.push(("take_profit", d));
            }
            if let Some(d) = &b.trailing {
                out.push(("trailing", d));
            }
        }
        TradeIntent::Protect(p) => {
            for (f, d) in [
                ("stop", &p.stop),
                ("take_profit", &p.take_profit),
                ("trailing", &p.trailing),
            ] {
                if let Some(d) = d {
                    out.push((f, d));
                }
            }
        }
        TradeIntent::Hold | TradeIntent::Cancel { .. } => {}
    }
    out
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum IntentError {
    #[error("`close` fraction {0} is outside (0, 1]; close all with 1, half with 0.5")]
    CloseFraction(f64),
    #[error("`{field}` is {amount} away; a distance must be positive — the side it is on follows from the position")]
    DistanceNotPositive { field: &'static str, amount: f64 },
}

impl IntentError {
    /// A stable code for each, for validators.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CloseFraction(_) => "close_fraction_out_of_range",
            Self::DistanceNotPositive { .. } => "distance_not_positive",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(json: serde_json::Value) -> TradeIntent {
        let i: TradeIntent =
            serde_json::from_value(json.clone()).unwrap_or_else(|e| panic!("{json}: {e}"));
        let back: TradeIntent = serde_json::from_str(&serde_json::to_string(&i).unwrap()).unwrap();
        assert_eq!(back, i);
        i
    }

    #[test]
    fn every_intent_kind_round_trips() {
        rt(serde_json::json!({"kind":"hold"}));
        rt(serde_json::json!({"kind":"target","size":{"mode":"equity_fraction","x":-0.25}}));
        rt(
            serde_json::json!({"kind":"target","size":{"mode":"units","x":"qty"},
                              "via":{"type":"limit","price":"m.q25"}}),
        );
        rt(
            serde_json::json!({"kind":"order","side":"buy","size":{"mode":"notional","x":500},
                              "style":{"type":"stop_limit","trigger":"hi","price":"hi_plus"},
                              "tif":"ioc","reduce_only":false,"post_only":false,"tag":"breakout"}),
        );
        rt(
            serde_json::json!({"kind":"order","side":"sell","size":{"mode":"units","x":1},
                              "style":{"type":"trailing_stop","trail":{"amount":1.5,"unit":{"unit":"pct"},"from":"high_water"}}}),
        );
        rt(serde_json::json!({"kind":"bracket",
                              "entry":{"kind":"target","size":{"mode":"risk","fraction":0.01,
                                       "stop":{"amount":2,"unit":{"unit":"atr","n":14}}}},
                              "stop":{"amount":2,"unit":{"unit":"atr","n":14}},
                              "take_profit":{"amount":"tp_sigmas","unit":{"unit":"sigma","of":"vol.sigma"}}}));
        rt(
            serde_json::json!({"kind":"protect","stop":{"amount":0,"unit":{"unit":"abs"},"from":"entry"}}),
        );
        rt(serde_json::json!({"kind":"close"}));
        rt(serde_json::json!({"kind":"close","fraction":0.5,"via":{"type":"limit","price":"tp"}}));
        rt(serde_json::json!({"kind":"add","size":{"mode":"vol_target","target":0.2,"vol":"rv"}}));
        rt(serde_json::json!({"kind":"reduce","size":{"mode":"equity_fraction","x":0.1}}));
        rt(serde_json::json!({"kind":"cancel","tag":"breakout"}));
        rt(serde_json::json!({"kind":"cancel"}));
    }

    #[test]
    fn a_bracket_without_a_stop_is_refused_by_name() {
        let err = serde_json::from_value::<TradeIntent>(serde_json::json!({
            "kind":"bracket","entry":{"kind":"target","size":{"mode":"units","x":1}},
            "take_profit":{"amount":2,"unit":{"unit":"pct"}}
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("stop"), "{err}");
    }

    #[test]
    fn a_limit_order_without_a_price_is_refused_by_name() {
        let err = serde_json::from_value::<TradeIntent>(serde_json::json!({
            "kind":"order","side":"buy","size":{"mode":"units","x":1},"style":{"type":"limit"}
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("price"), "{err}");
    }

    #[test]
    fn protect_with_nothing_to_protect_is_refused() {
        let err = serde_json::from_value::<TradeIntent>(serde_json::json!({"kind":"protect"}))
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least one"), "{err}");
    }

    #[test]
    fn a_literal_close_fraction_outside_zero_one_is_refused() {
        for f in [0.0, -0.5, 1.5] {
            let i = TradeIntent::Close {
                fraction: Arg::Num(f),
                via: None,
            };
            assert_eq!(check_literals(&i), Err(IntentError::CloseFraction(f)));
        }
        for f in [0.01, 0.5, 1.0] {
            assert!(check_literals(&TradeIntent::Close {
                fraction: Arg::Num(f),
                via: None
            })
            .is_ok());
        }
        // A fraction that comes from the graph is the planner's to check.
        assert!(check_literals(&TradeIntent::Close {
            fraction: Arg::Ref("half".into()),
            via: None
        })
        .is_ok());
    }

    #[test]
    fn an_order_defaults_to_market_and_gtc_and_says_so_when_serialised() {
        let i = rt(serde_json::json!({"kind":"order","side":"buy","size":{"mode":"units","x":1}}));
        let TradeIntent::Order(o) = i else { panic!() };
        assert_eq!(o.style, OrderStyle::Market);
        assert_eq!(o.tif, TimeInForce::Gtc);
        assert!(!o.reduce_only && !o.post_only);
    }
}
