//! Trade intents → orders (FEAT-007 §6, ADR-0041, U-1.8).
//!
//! The one place that decides what orders an intent becomes. It lives inside the
//! engine, so it is identical in backtest, paper and live: only the venue's
//! *answer* to an order differs between environments, never the order.
//!
//! Its rules, each tested:
//!
//! - **`target` is idempotent.** It diffs the target against position *plus
//!   working entry orders*; re-stating an unchanged target emits nothing, which
//!   is what makes a restart safe. A target that working orders contradict
//!   cancels them and places the remainder against the position alone.
//! - **Rounding never reverses a position.** Quantities round toward zero to the
//!   lot; a reduce-only order is capped at the position.
//! - **Protection follows the fill.** A bracket's stop and target are placed when
//!   its entry fills, sized to what filled and anchored at the fill price — not
//!   at the decision, when neither is known.
//! - **What a venue cannot express is refused by name**, never silently
//!   downgraded to a market order.
//!
//! Intents from different rule groups in one tick are planned independently
//! against the decision-instant portfolio view, in rule order.

use domain::order::{Side, TimeInForce};
use domain::strategy_graph::intent::Anchor;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

use crate::portfolio::{InstrumentSpec, PortfolioView, WorkingOrder};

// ── resolved intents: every graph value is now a number ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RSize {
    Units { x: f64 },
    Notional { x: f64 },
    EquityFraction { x: f64 },
    Risk { fraction: f64, stop: RDist },
    VolTarget { target: f64, vol: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RDist {
    pub amount: f64,
    pub unit: RUnit,
    pub from: Anchor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case")]
pub enum RUnit {
    Pct,
    Abs,
    /// The ATR value already read.
    Atr {
        atr: f64,
    },
    /// The volatility value already read.
    Sigma {
        sigma: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RStyle {
    Market,
    Limit { price: f64 },
    Stop { trigger: f64 },
    StopLimit { trigger: f64, price: f64 },
    TrailingStop { trail: RDist },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ROrder {
    pub side: Side,
    pub size: RSize,
    pub style: RStyle,
    pub tif: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum REntry {
    Target { size: RSize, via: Option<RStyle> },
    Order(ROrder),
}

/// A trade intent with every value resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedIntent {
    Hold,
    Target {
        size: RSize,
        via: Option<RStyle>,
    },
    Order(ROrder),
    Bracket {
        entry: REntry,
        stop: RDist,
        take_profit: Option<RDist>,
        trailing: Option<RDist>,
    },
    Protect {
        stop: Option<RDist>,
        take_profit: Option<RDist>,
        trailing: Option<RDist>,
    },
    Close {
        fraction: f64,
        via: Option<RStyle>,
    },
    Add {
        size: RSize,
        via: Option<RStyle>,
    },
    Reduce {
        size: RSize,
        via: Option<RStyle>,
    },
    Cancel {
        tag: Option<String>,
    },
}

// ── what the planner produces ─────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderRole {
    Entry,
    Exit,
    Protective,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlannedStyle {
    Market,
    Limit {
        price: Decimal,
    },
    Stop {
        trigger: Decimal,
    },
    StopLimit {
        trigger: Decimal,
        price: Decimal,
    },
    /// Trails the best price by `trail`, in price units.
    TrailingStop {
        trail: Decimal,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedOrder {
    pub tag: String,
    pub side: Side,
    pub qty: Decimal,
    pub style: PlannedStyle,
    pub tif: TimeInForce,
    pub reduce_only: bool,
    pub post_only: bool,
    /// One-cancels-other group; the engine emulates it where the venue cannot.
    pub oco_group: Option<String>,
    pub role: OrderRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OrderAction {
    Submit(PlannedOrder),
    Cancel { tag: String },
}

/// An intent the planner could not carry out, and why. Recorded, never hidden.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub rule: usize,
    pub reason: String,
}

/// A bracket waiting for its entry to fill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Armed {
    pub entry_side: Side,
    pub stop: RDist,
    pub take_profit: Option<RDist>,
    pub trailing: Option<RDist>,
    pub filled: Decimal,
    /// Volume-weighted fill price so far — the anchor for `from: entry`.
    pub avg_price: Decimal,
    pub fills: u32,
}

/// Mutable state the planner keeps across ticks: armed brackets, the tag
/// counter. Checkpointed with the engine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlannerState {
    pub armed: std::collections::BTreeMap<String, Armed>,
    pub seq: u64,
}

impl PlannerState {
    fn tag(&mut self, tick: u64, user: Option<&str>) -> String {
        self.seq += 1;
        user.map_or_else(|| format!("t{tick}-{}", self.seq), str::to_string)
    }
}

/// The planner's output for one tick.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    pub actions: Vec<OrderAction>,
    pub refusals: Vec<Refusal>,
}

struct Ctx<'a> {
    view: &'a PortfolioView,
    spec: &'a InstrumentSpec,
    tick: u64,
}

fn dec(x: f64) -> Option<Decimal> {
    Decimal::from_f64(x)
}

impl Ctx<'_> {
    fn mark(&self) -> Result<Decimal, String> {
        self.view
            .mark
            .filter(|m| m.is_sign_positive() && !m.is_zero())
            .ok_or_else(|| {
                "no mark price for the instrument, so the size cannot be computed".to_string()
            })
    }

    /// Absolute price distance of `d` from `anchor`.
    fn distance(&self, d: &RDist, anchor: Decimal) -> Result<Decimal, String> {
        let amount = dec(d.amount).ok_or("distance is not a number")?;
        let abs = match d.unit {
            RUnit::Pct => anchor * amount / Decimal::ONE_HUNDRED,
            RUnit::Abs => amount,
            RUnit::Atr { atr } => amount * dec(atr).ok_or("ATR is not a number")?,
            RUnit::Sigma { sigma } => amount * dec(sigma).ok_or("sigma is not a number")? * anchor,
        };
        if abs.is_sign_negative() {
            return Err(format!("distance {abs} is negative"));
        }
        Ok(abs)
    }

    fn anchor(&self, from: Anchor, entry: Option<Decimal>) -> Result<Decimal, String> {
        match from {
            Anchor::Entry => entry
                .or(self.view.avg_entry_price)
                .ok_or_else(|| "no entry price to measure the distance from".to_string()),
            Anchor::Mark => self.mark(),
            Anchor::HighWater => self
                .view
                .high_water
                .or(self.view.mark)
                .ok_or_else(|| "no high-water price to measure the distance from".to_string()),
        }
    }

    /// Unsigned quantity a size spec asks for (target sign handled by caller).
    fn qty(&self, s: &RSize) -> Result<Decimal, String> {
        let equity = self.view.equity;
        let q = match s {
            RSize::Units { x } => dec(*x).ok_or("size is not a number")?,
            RSize::Notional { x } => dec(*x).ok_or("notional is not a number")? / self.mark()?,
            RSize::EquityFraction { x } => {
                dec(*x).ok_or("fraction is not a number")? * equity / self.mark()?
            }
            RSize::Risk { fraction, stop } => {
                let mark = self.mark()?;
                let d = self.distance(stop, mark)?;
                if d.is_zero() {
                    return Err(
                        "risk sizing with a zero stop distance would be an unbounded position"
                            .into(),
                    );
                }
                dec(*fraction).ok_or("risk fraction is not a number")? * equity / d
            }
            RSize::VolTarget { target, vol } => {
                if *vol <= 0.0 {
                    return Err("vol-target sizing needs a positive volatility".into());
                }
                dec(target / vol).ok_or("vol-target ratio is not a number")? * equity
                    / self.mark()?
            }
        };
        Ok(q)
    }

    /// Toward zero, to the lot.
    fn round_qty(&self, q: Decimal) -> Decimal {
        let lot = self.spec.lot_size;
        if lot.is_zero() {
            return q;
        }
        (q / lot).trunc() * lot
    }

    /// To the tick, in the direction that never makes the order more
    /// aggressive than written: a buy limit rounds down, a sell limit up; a
    /// buy stop rounds up, a sell stop down.
    fn round_price(&self, p: Decimal, side: Side, is_stop: bool) -> Decimal {
        let tick = self.spec.tick_size;
        if tick.is_zero() {
            return p;
        }
        let up = matches!((side, is_stop), (Side::Buy, true) | (Side::Sell, false));
        let strategy = if up {
            RoundingStrategy::ToPositiveInfinity
        } else {
            RoundingStrategy::ToNegativeInfinity
        };
        (p / tick).round_dp_with_strategy(0, strategy) * tick
    }

    fn style(
        &self,
        s: &RStyle,
        side: Side,
        anchor_for_trail: Option<Decimal>,
    ) -> Result<PlannedStyle, String> {
        let caps = self.spec.caps;
        let price = |x: f64| {
            dec(x)
                .filter(|p| p.is_sign_positive() && !p.is_zero())
                .ok_or_else(|| format!("price {x} is not a positive number"))
        };
        Ok(match s {
            RStyle::Market => PlannedStyle::Market,
            RStyle::Limit { price: p } => {
                if !caps.limit {
                    return Err("this venue does not accept limit orders".into());
                }
                PlannedStyle::Limit {
                    price: self.round_price(price(*p)?, side, false),
                }
            }
            RStyle::Stop { trigger } => {
                if !caps.stop {
                    return Err("this venue does not accept stop orders".into());
                }
                PlannedStyle::Stop {
                    trigger: self.round_price(price(*trigger)?, side, true),
                }
            }
            RStyle::StopLimit { trigger, price: p } => {
                if !caps.stop_limit {
                    return Err("this venue does not accept stop-limit orders".into());
                }
                PlannedStyle::StopLimit {
                    trigger: self.round_price(price(*trigger)?, side, true),
                    price: self.round_price(price(*p)?, side, false),
                }
            }
            RStyle::TrailingStop { trail } => {
                if !caps.trailing_stop {
                    return Err("this venue does not accept trailing stops".into());
                }
                let anchor = match anchor_for_trail {
                    Some(a) => a,
                    None => self.anchor(trail.from, None)?,
                };
                PlannedStyle::TrailingStop {
                    trail: self.distance(trail, anchor)?,
                }
            }
        })
    }

    fn working_entries(&self) -> impl Iterator<Item = &WorkingOrder> {
        self.view
            .open_orders
            .iter()
            .filter(|o| !o.protective && !o.reduce_only)
    }

    fn signed_working(&self) -> Decimal {
        self.working_entries()
            .map(|o| {
                if o.side == Side::Buy {
                    o.remaining
                } else {
                    -o.remaining
                }
            })
            .sum()
    }
}

/// Plan one tick's intents. `rules[i]` is the rule index each intent came from.
pub fn plan(
    intents: &[(usize, ResolvedIntent)],
    view: &PortfolioView,
    spec: &InstrumentSpec,
    state: &mut PlannerState,
    tick: u64,
) -> Planned {
    let ctx = Ctx { view, spec, tick };
    let mut out = Planned::default();
    for (rule, intent) in intents {
        if let Err(reason) = plan_one(&ctx, intent, state, &mut out.actions) {
            out.refusals.push(Refusal {
                rule: *rule,
                reason,
            });
        }
    }
    out
}

#[allow(clippy::too_many_lines)]
fn plan_one(
    ctx: &Ctx<'_>,
    intent: &ResolvedIntent,
    state: &mut PlannerState,
    acts: &mut Vec<OrderAction>,
) -> Result<(), String> {
    let pos = ctx.view.position;
    match intent {
        ResolvedIntent::Hold => Ok(()),
        ResolvedIntent::Target { size, via } => {
            let tag = target(ctx, size, via.as_ref(), state, acts, OrderRole::Entry)?;
            let _ = tag;
            Ok(())
        }
        ResolvedIntent::Order(o) => {
            let q = ctx.round_qty(ctx.qty(&o.size)?.abs());
            let q = if o.reduce_only { q.min(pos.abs()) } else { q };
            if q < ctx.spec.min_qty || q.is_zero() {
                return Err(format!(
                    "order quantity {q} is below the venue minimum {}",
                    ctx.spec.min_qty
                ));
            }
            if o.post_only && !ctx.spec.caps.post_only {
                return Err("this venue does not accept post-only orders".into());
            }
            if o.reduce_only && !ctx.spec.caps.reduce_only {
                return Err("this venue does not accept reduce-only orders".into());
            }
            let style = ctx.style(&o.style, o.side, None)?;
            if let Some(user) = &o.tag {
                if ctx.view.open_orders.iter().any(|w| &w.tag == user) {
                    acts.push(OrderAction::Cancel { tag: user.clone() });
                }
            }
            let tag = state.tag(ctx.tick, o.tag.as_deref());
            acts.push(OrderAction::Submit(PlannedOrder {
                tag,
                side: o.side,
                qty: q,
                style,
                tif: o.tif,
                reduce_only: o.reduce_only,
                post_only: o.post_only,
                oco_group: None,
                role: if o.reduce_only {
                    OrderRole::Exit
                } else {
                    OrderRole::Entry
                },
            }));
            Ok(())
        }
        ResolvedIntent::Bracket {
            entry,
            stop,
            take_profit,
            trailing,
        } => {
            let before = acts.len();
            let (entry_tag, side) = match entry {
                REntry::Target { size, via } => {
                    let Some(tag) = target(ctx, size, via.as_ref(), state, acts, OrderRole::Entry)?
                    else {
                        // Already at the target: nothing to enter, so nothing to arm.
                        return Ok(());
                    };
                    let side = acts[before..]
                        .iter()
                        .find_map(|a| match a {
                            OrderAction::Submit(o) if o.tag == tag => Some(o.side),
                            _ => None,
                        })
                        .unwrap_or(Side::Buy);
                    (tag, side)
                }
                REntry::Order(o) => {
                    plan_one(ctx, &ResolvedIntent::Order(o.clone()), state, acts)?;
                    let tag = acts[before..]
                        .iter()
                        .rev()
                        .find_map(|a| match a {
                            OrderAction::Submit(p) => Some(p.tag.clone()),
                            OrderAction::Cancel { .. } => None,
                        })
                        .ok_or("the bracket's entry produced no order")?;
                    (tag, o.side)
                }
            };
            state.armed.insert(
                entry_tag,
                Armed {
                    entry_side: side,
                    stop: stop.clone(),
                    take_profit: take_profit.clone(),
                    trailing: trailing.clone(),
                    filled: Decimal::ZERO,
                    avg_price: Decimal::ZERO,
                    fills: 0,
                },
            );
            Ok(())
        }
        ResolvedIntent::Protect {
            stop,
            take_profit,
            trailing,
        } => {
            if pos.is_zero() {
                return Err("`protect` while flat: there is no position to protect".into());
            }
            for w in ctx.view.open_orders.iter().filter(|w| w.protective) {
                acts.push(OrderAction::Cancel { tag: w.tag.clone() });
            }
            let group = format!("protect-{}", ctx.tick);
            let side = if pos.is_sign_positive() {
                Side::Buy
            } else {
                Side::Sell
            };
            let armed = Armed {
                entry_side: side,
                stop: stop.clone().unwrap_or(RDist {
                    amount: 0.0,
                    unit: RUnit::Abs,
                    from: Anchor::Entry,
                }),
                take_profit: take_profit.clone(),
                trailing: trailing.clone(),
                filled: pos.abs(),
                avg_price: ctx.view.avg_entry_price.unwrap_or_default(),
                fills: 1,
            };
            protective_orders(ctx, &group, &armed, stop.is_some(), acts)
        }
        ResolvedIntent::Close { fraction, via } => {
            if pos.is_zero() {
                return Ok(());
            }
            let f = dec(*fraction).ok_or("close fraction is not a number")?;
            if f <= Decimal::ZERO || f > Decimal::ONE {
                return Err(format!("close fraction {fraction} is outside (0, 1]"));
            }
            let whole = f == Decimal::ONE;
            let q = if whole {
                pos.abs()
            } else {
                ctx.round_qty(pos.abs() * f)
            };
            if q.is_zero() || q < ctx.spec.min_qty {
                return Err(format!(
                    "closing {fraction} of {pos} rounds to {q}, below the venue minimum"
                ));
            }
            let side = if pos.is_sign_positive() {
                Side::Sell
            } else {
                Side::Buy
            };
            let style = match via {
                Some(s) => ctx.style(s, side, None)?,
                None => PlannedStyle::Market,
            };
            if whole {
                for w in &ctx.view.open_orders {
                    acts.push(OrderAction::Cancel { tag: w.tag.clone() });
                }
                state.armed.clear();
            }
            let tag = state.tag(ctx.tick, None);
            acts.push(OrderAction::Submit(PlannedOrder {
                tag,
                side,
                qty: q,
                style,
                tif: TimeInForce::Gtc,
                reduce_only: true,
                post_only: false,
                oco_group: None,
                role: OrderRole::Exit,
            }));
            Ok(())
        }
        ResolvedIntent::Add { size, via } => {
            if pos.is_zero() {
                return Err(
                    "`add` while flat has no direction; use `target` or `order` to open".into(),
                );
            }
            let side = if pos.is_sign_positive() {
                Side::Buy
            } else {
                Side::Sell
            };
            let q = ctx.round_qty(ctx.qty(size)?.abs());
            if q.is_zero() || q < ctx.spec.min_qty {
                return Err(format!("add of {q} is below the venue minimum"));
            }
            let style = match via {
                Some(s) => ctx.style(s, side, None)?,
                None => PlannedStyle::Market,
            };
            let tag = state.tag(ctx.tick, None);
            acts.push(OrderAction::Submit(PlannedOrder {
                tag,
                side,
                qty: q,
                style,
                tif: TimeInForce::Gtc,
                reduce_only: false,
                post_only: false,
                oco_group: None,
                role: OrderRole::Entry,
            }));
            Ok(())
        }
        ResolvedIntent::Reduce { size, via } => {
            if pos.is_zero() {
                return Ok(());
            }
            let side = if pos.is_sign_positive() {
                Side::Sell
            } else {
                Side::Buy
            };
            let q = ctx.round_qty(ctx.qty(size)?.abs()).min(pos.abs());
            if q.is_zero() || q < ctx.spec.min_qty {
                return Err(format!("reduce of {q} is below the venue minimum"));
            }
            let style = match via {
                Some(s) => ctx.style(s, side, None)?,
                None => PlannedStyle::Market,
            };
            let tag = state.tag(ctx.tick, None);
            acts.push(OrderAction::Submit(PlannedOrder {
                tag,
                side,
                qty: q,
                style,
                tif: TimeInForce::Gtc,
                reduce_only: true,
                post_only: false,
                oco_group: None,
                role: OrderRole::Exit,
            }));
            Ok(())
        }
        ResolvedIntent::Cancel { tag } => {
            for w in &ctx.view.open_orders {
                if tag.as_ref().is_none_or(|t| &w.tag == t) {
                    acts.push(OrderAction::Cancel { tag: w.tag.clone() });
                }
            }
            if tag.is_none() {
                state.armed.clear();
            } else if let Some(t) = tag {
                state.armed.remove(t);
            }
            Ok(())
        }
    }
}

/// Plan a target; the tag of the order placed, if one was needed.
fn target(
    ctx: &Ctx<'_>,
    size: &RSize,
    via: Option<&RStyle>,
    state: &mut PlannerState,
    acts: &mut Vec<OrderAction>,
    role: OrderRole,
) -> Result<Option<String>, String> {
    let signed = match size {
        RSize::Units { x } | RSize::Notional { x } | RSize::EquityFraction { x } => {
            let q = ctx.qty(size)?;
            if *x < 0.0 {
                -q.abs()
            } else {
                q.abs()
            }
        }
        // Risk and vol-target sizes have no sign: they size a long.
        RSize::Risk { .. } | RSize::VolTarget { .. } => ctx.qty(size)?.abs(),
    };
    let goal = ctx.round_qty(signed);
    let pos = ctx.view.position;
    let working = ctx.signed_working();
    if pos + working == goal {
        return Ok(None);
    }
    // Working entries that do not add up to the goal are withdrawn, and the
    // remainder is placed against the position alone. Simpler than netting
    // partial working orders, and idempotent: next tick, position + working is
    // the goal and nothing is emitted.
    for w in ctx.working_entries() {
        acts.push(OrderAction::Cancel { tag: w.tag.clone() });
    }
    let remainder = goal - pos;
    if remainder.is_zero() || remainder.abs() < ctx.spec.min_qty {
        return Ok(None);
    }
    // `direction`, not `side`: clippy reads `side` beside the `size` argument
    // as a likely typo, and in a planner that is exactly the confusion to avoid.
    let direction = if remainder.is_sign_positive() {
        Side::Buy
    } else {
        Side::Sell
    };
    let style = match via {
        Some(s) => ctx.style(s, direction, None)?,
        None => PlannedStyle::Market,
    };
    let tag = state.tag(ctx.tick, None);
    acts.push(OrderAction::Submit(PlannedOrder {
        tag: tag.clone(),
        side: direction,
        qty: remainder.abs(),
        style,
        tif: TimeInForce::Gtc,
        reduce_only: false,
        post_only: false,
        oco_group: None,
        role,
    }));
    Ok(Some(tag))
}

/// The protective orders for an armed bracket (or a `protect`), sized to what
/// has filled and anchored at the fill price.
fn protective_orders(
    ctx: &Ctx<'_>,
    group: &str,
    a: &Armed,
    with_stop: bool,
    acts: &mut Vec<OrderAction>,
) -> Result<(), String> {
    let qty = a.filled;
    if qty.is_zero() {
        return Ok(());
    }
    let exit_side = match a.entry_side {
        Side::Buy => Side::Sell,
        Side::Sell => Side::Buy,
    };
    let long = a.entry_side == Side::Buy;
    let entry = (!a.avg_price.is_zero()).then_some(a.avg_price);
    let mut placed = 0;
    if with_stop {
        let anchor = ctx.anchor(a.stop.from, entry)?;
        let d = ctx.distance(&a.stop, anchor)?;
        let trigger = if long { anchor - d } else { anchor + d };
        if trigger.is_sign_negative() || trigger.is_zero() {
            return Err(format!("the stop lands at {trigger}, which is not a price"));
        }
        if !ctx.spec.caps.stop {
            return Err(
                "this venue does not accept stop orders, so the bracket cannot be protected".into(),
            );
        }
        acts.push(OrderAction::Submit(PlannedOrder {
            tag: format!("{group}:sl{}", a.fills),
            side: exit_side,
            qty,
            style: PlannedStyle::Stop {
                trigger: ctx.round_price(trigger, exit_side, true),
            },
            tif: TimeInForce::Gtc,
            reduce_only: true,
            post_only: false,
            oco_group: Some(group.to_string()),
            role: OrderRole::Protective,
        }));
        placed += 1;
    }
    if let Some(tp) = &a.take_profit {
        let anchor = ctx.anchor(tp.from, entry)?;
        let d = ctx.distance(tp, anchor)?;
        let price = if long { anchor + d } else { anchor - d };
        if price.is_sign_negative() || price.is_zero() {
            return Err(format!(
                "the take-profit lands at {price}, which is not a price"
            ));
        }
        acts.push(OrderAction::Submit(PlannedOrder {
            tag: format!("{group}:tp{}", a.fills),
            side: exit_side,
            qty,
            style: PlannedStyle::Limit {
                price: ctx.round_price(price, exit_side, false),
            },
            tif: TimeInForce::Gtc,
            reduce_only: true,
            post_only: false,
            oco_group: Some(group.to_string()),
            role: OrderRole::Protective,
        }));
        placed += 1;
    }
    if let Some(tr) = &a.trailing {
        if !ctx.spec.caps.trailing_stop {
            return Err("this venue does not accept trailing stops".into());
        }
        let anchor = ctx.anchor(tr.from, entry)?;
        acts.push(OrderAction::Submit(PlannedOrder {
            tag: format!("{group}:tr{}", a.fills),
            side: exit_side,
            qty,
            style: PlannedStyle::TrailingStop {
                trail: ctx.distance(tr, anchor)?,
            },
            tif: TimeInForce::Gtc,
            reduce_only: true,
            post_only: false,
            oco_group: Some(group.to_string()),
            role: OrderRole::Protective,
        }));
        placed += 1;
    }
    let _ = placed;
    Ok(())
}

/// React to a fill: place or re-size a bracket's protection when its entry
/// fills; cancel the other legs when a protective order fills (OCO, emulated
/// here so it holds on venues without native OCO).
pub fn on_fill(
    fill: &crate::portfolio::Fill,
    view: &PortfolioView,
    spec: &InstrumentSpec,
    state: &mut PlannerState,
    tick: u64,
) -> Planned {
    let ctx = Ctx { view, spec, tick };
    let mut out = Planned::default();
    if let Some(a) = state.armed.get_mut(&fill.tag) {
        let total = a.filled + fill.qty;
        if !total.is_zero() {
            a.avg_price = (a.avg_price * a.filled + fill.price * fill.qty) / total;
        }
        a.filled = total;
        a.fills += 1;
        let snapshot = a.clone();
        // Re-size: cancel the previous protective legs of this group, place new
        // ones for the cumulative fill.
        for w in view
            .open_orders
            .iter()
            .filter(|w| w.tag.starts_with(&format!("{}:", fill.tag)))
        {
            out.actions.push(OrderAction::Cancel { tag: w.tag.clone() });
        }
        if let Err(reason) = protective_orders(&ctx, &fill.tag, &snapshot, true, &mut out.actions) {
            out.refusals.push(Refusal {
                rule: usize::MAX,
                reason,
            });
        }
        return out;
    }
    // A protective leg filled: cancel its siblings.
    if let Some((group, _)) = fill.tag.split_once(':') {
        for w in view
            .open_orders
            .iter()
            .filter(|w| w.tag != fill.tag && w.tag.starts_with(&format!("{group}:")))
        {
            out.actions.push(OrderAction::Cancel { tag: w.tag.clone() });
        }
        state.armed.remove(group);
    }
    out
}

/// Forget an armed bracket whose entry was cancelled, rejected or expired with
/// nothing filled.
pub fn on_closed(tag: &str, state: &mut PlannerState) {
    if state.armed.get(tag).is_some_and(|a| a.filled.is_zero()) {
        state.armed.remove(tag);
    }
}

/// Quantity as `f64`, for traces.
#[must_use]
pub fn qty_f64(q: Decimal) -> f64 {
    q.to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::{Fill, VenueCaps};
    use rust_decimal_macros::dec;

    fn spec() -> InstrumentSpec {
        InstrumentSpec {
            instrument: "BTC-USD".into(),
            lot_size: dec!(0.001),
            tick_size: dec!(0.01),
            min_qty: dec!(0.001),
            caps: VenueCaps::all(),
        }
    }

    fn flat() -> PortfolioView {
        PortfolioView {
            equity: dec!(10000),
            cash: dec!(10000),
            mark: Some(dec!(100)),
            ..Default::default()
        }
    }

    fn submits(p: &Planned) -> Vec<&PlannedOrder> {
        p.actions
            .iter()
            .filter_map(|a| match a {
                OrderAction::Submit(o) => Some(o),
                OrderAction::Cancel { .. } => None,
            })
            .collect()
    }

    fn cancels(p: &Planned) -> Vec<&str> {
        p.actions
            .iter()
            .filter_map(|a| match a {
                OrderAction::Cancel { tag } => Some(tag.as_str()),
                OrderAction::Submit(_) => None,
            })
            .collect()
    }

    fn tgt(x: f64) -> ResolvedIntent {
        ResolvedIntent::Target {
            size: RSize::EquityFraction { x },
            via: None,
        }
    }

    #[test]
    fn a_target_from_flat_buys_the_difference_rounded_to_the_lot() {
        let mut st = PlannerState::default();
        // 0.5 × 10000 / 100 = 50 units.
        let p = plan(&[(0, tgt(0.5))], &flat(), &spec(), &mut st, 1);
        let s = submits(&p);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].side, Side::Buy);
        assert_eq!(s[0].qty, dec!(50));
        assert_eq!(s[0].style, PlannedStyle::Market);
    }

    #[test]
    fn restating_an_unchanged_target_emits_nothing() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(50);
        let p = plan(&[(0, tgt(0.5))], &v, &spec(), &mut st, 2);
        assert!(p.actions.is_empty(), "idempotent: {p:?}");
    }

    #[test]
    fn a_target_half_filled_by_a_working_order_emits_nothing_more() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(20);
        v.open_orders = vec![WorkingOrder {
            tag: "t1-1".into(),
            side: Side::Buy,
            remaining: dec!(30),
            reduce_only: false,
            protective: false,
        }];
        let p = plan(&[(0, tgt(0.5))], &v, &spec(), &mut st, 2);
        assert!(
            p.actions.is_empty(),
            "position + working already equals the target"
        );
    }

    #[test]
    fn a_contradicted_working_entry_is_cancelled_and_the_remainder_placed() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(20);
        v.open_orders = vec![WorkingOrder {
            tag: "old".into(),
            side: Side::Buy,
            remaining: dec!(30),
            reduce_only: false,
            protective: false,
        }];
        // New target 10 units: cancel the working buy, sell 10 against position 20.
        let p = plan(&[(0, tgt(0.1))], &v, &spec(), &mut st, 3);
        assert_eq!(cancels(&p), vec!["old"]);
        let s = submits(&p);
        assert_eq!((s[0].side, s[0].qty), (Side::Sell, dec!(10)));
    }

    #[test]
    fn a_negative_target_goes_short_and_a_target_can_flip() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(10);
        let p = plan(&[(0, tgt(-0.1))], &v, &spec(), &mut st, 1);
        let s = submits(&p);
        assert_eq!(
            (s[0].side, s[0].qty),
            (Side::Sell, dec!(20)),
            "from +10 to −10"
        );
    }

    #[test]
    fn rounding_never_turns_a_reduce_into_a_reversal() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(1.0005); // not on the lot
        let p = plan(
            &[(
                0,
                ResolvedIntent::Reduce {
                    size: RSize::Units { x: 5.0 },
                    via: None,
                },
            )],
            &v,
            &spec(),
            &mut st,
            1,
        );
        let s = submits(&p);
        assert!(s[0].reduce_only);
        assert!(s[0].qty <= v.position, "capped at the position");
        assert_eq!(s[0].side, Side::Sell);
    }

    #[test]
    fn close_half_and_close_all() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(-3);
        v.open_orders = vec![WorkingOrder {
            tag: "g:sl1".into(),
            side: Side::Buy,
            remaining: dec!(3),
            reduce_only: true,
            protective: true,
        }];
        let half = plan(
            &[(
                0,
                ResolvedIntent::Close {
                    fraction: 0.5,
                    via: None,
                },
            )],
            &v,
            &spec(),
            &mut st,
            1,
        );
        let s = submits(&half);
        assert_eq!(
            (s[0].side, s[0].qty, s[0].reduce_only),
            (Side::Buy, dec!(1.5), true)
        );
        assert!(
            cancels(&half).is_empty(),
            "a partial close keeps protection"
        );
        let all = plan(
            &[(
                0,
                ResolvedIntent::Close {
                    fraction: 1.0,
                    via: None,
                },
            )],
            &v,
            &spec(),
            &mut st,
            2,
        );
        assert_eq!(
            cancels(&all),
            vec!["g:sl1"],
            "a full close withdraws protection"
        );
        assert_eq!(submits(&all)[0].qty, dec!(3));
        let flat_close = plan(
            &[(
                0,
                ResolvedIntent::Close {
                    fraction: 1.0,
                    via: None,
                },
            )],
            &flat(),
            &spec(),
            &mut st,
            3,
        );
        assert!(flat_close.actions.is_empty() && flat_close.refusals.is_empty());
    }

    #[test]
    fn a_bracket_places_protection_only_when_its_entry_fills() {
        let mut st = PlannerState::default();
        let intent = ResolvedIntent::Bracket {
            entry: REntry::Target {
                size: RSize::Units { x: 2.0 },
                via: None,
            },
            stop: RDist {
                amount: 2.0,
                unit: RUnit::Pct,
                from: Anchor::Entry,
            },
            take_profit: Some(RDist {
                amount: 4.0,
                unit: RUnit::Atr { atr: 1.5 },
                from: Anchor::Entry,
            }),
            trailing: None,
        };
        let p = plan(&[(0, intent)], &flat(), &spec(), &mut st, 7);
        let s = submits(&p);
        assert_eq!(s.len(), 1, "only the entry now: {p:?}");
        let entry_tag = s[0].tag.clone();
        assert!(st.armed.contains_key(&entry_tag));

        let mut v = flat();
        v.position = dec!(2);
        let fill = Fill {
            tag: entry_tag.clone(),
            side: Side::Buy,
            qty: dec!(2),
            price: dec!(101),
            at_ns: 0,
        };
        let prot = on_fill(&fill, &v, &spec(), &mut st, 8);
        let s = submits(&prot);
        assert_eq!(s.len(), 2);
        // Stop 2% under the FILL price (101 → 98.98), target 4 × 1.5 over it (107).
        let stop = s
            .iter()
            .find(|o| matches!(o.style, PlannedStyle::Stop { .. }))
            .unwrap();
        assert_eq!(
            stop.style,
            PlannedStyle::Stop {
                trigger: dec!(98.98)
            }
        );
        assert!(stop.reduce_only && stop.side == Side::Sell && stop.qty == dec!(2));
        let tp = s
            .iter()
            .find(|o| matches!(o.style, PlannedStyle::Limit { .. }))
            .unwrap();
        assert_eq!(
            tp.style,
            PlannedStyle::Limit {
                price: dec!(107.00)
            }
        );
        assert_eq!(stop.oco_group.as_deref(), Some(entry_tag.as_str()));
    }

    #[test]
    fn when_one_protective_leg_fills_the_others_are_cancelled() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.open_orders = vec![
            WorkingOrder {
                tag: "e1:sl1".into(),
                side: Side::Sell,
                remaining: dec!(2),
                reduce_only: true,
                protective: true,
            },
            WorkingOrder {
                tag: "e1:tp1".into(),
                side: Side::Sell,
                remaining: dec!(2),
                reduce_only: true,
                protective: true,
            },
        ];
        let fill = Fill {
            tag: "e1:sl1".into(),
            side: Side::Sell,
            qty: dec!(2),
            price: dec!(98),
            at_ns: 0,
        };
        let out = on_fill(&fill, &v, &spec(), &mut st, 9);
        assert_eq!(cancels(&out), vec!["e1:tp1"]);
    }

    #[test]
    fn what_a_venue_cannot_express_is_refused_by_name_not_downgraded() {
        let mut st = PlannerState::default();
        let mut s = spec();
        s.caps = VenueCaps::market_only();
        let intent = ResolvedIntent::Order(ROrder {
            side: Side::Buy,
            size: RSize::Units { x: 1.0 },
            style: RStyle::Limit { price: 99.0 },
            tif: TimeInForce::Gtc,
            reduce_only: false,
            post_only: false,
            tag: None,
        });
        let p = plan(&[(4, intent)], &flat(), &s, &mut st, 1);
        assert!(p.actions.is_empty(), "no market order in its place");
        assert_eq!(p.refusals[0].rule, 4);
        assert!(p.refusals[0].reason.contains("limit"), "{:?}", p.refusals);
    }

    #[test]
    fn prices_round_in_the_direction_that_is_never_more_aggressive() {
        let mut st = PlannerState::default();
        let buy_limit = ResolvedIntent::Order(ROrder {
            side: Side::Buy,
            size: RSize::Units { x: 1.0 },
            style: RStyle::Limit { price: 99.999 },
            tif: TimeInForce::Gtc,
            reduce_only: false,
            post_only: false,
            tag: None,
        });
        let p = plan(&[(0, buy_limit)], &flat(), &spec(), &mut st, 1);
        assert_eq!(
            submits(&p)[0].style,
            PlannedStyle::Limit { price: dec!(99.99) }
        );
    }

    #[test]
    fn add_while_flat_is_refused_and_cancel_by_tag_cancels_only_that_tag() {
        let mut st = PlannerState::default();
        let p = plan(
            &[(
                1,
                ResolvedIntent::Add {
                    size: RSize::Units { x: 1.0 },
                    via: None,
                },
            )],
            &flat(),
            &spec(),
            &mut st,
            1,
        );
        assert!(p.refusals[0].reason.contains("no direction"));

        let mut v = flat();
        v.open_orders = ["a", "b"]
            .iter()
            .map(|t| WorkingOrder {
                tag: (*t).into(),
                side: Side::Buy,
                remaining: dec!(1),
                reduce_only: false,
                protective: false,
            })
            .collect();
        let p = plan(
            &[(
                0,
                ResolvedIntent::Cancel {
                    tag: Some("b".into()),
                },
            )],
            &v,
            &spec(),
            &mut st,
            2,
        );
        assert_eq!(cancels(&p), vec!["b"]);
    }

    #[test]
    fn protect_replaces_existing_protection_and_refuses_when_flat() {
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(4);
        v.avg_entry_price = Some(dec!(100));
        v.open_orders = vec![WorkingOrder {
            tag: "old:sl1".into(),
            side: Side::Sell,
            remaining: dec!(4),
            reduce_only: true,
            protective: true,
        }];
        // Move the stop to breakeven: zero distance from entry.
        let breakeven = ResolvedIntent::Protect {
            stop: Some(RDist {
                amount: 0.0,
                unit: RUnit::Abs,
                from: Anchor::Entry,
            }),
            take_profit: None,
            trailing: None,
        };
        let p = plan(&[(0, breakeven.clone())], &v, &spec(), &mut st, 5);
        assert_eq!(cancels(&p), vec!["old:sl1"]);
        let s = submits(&p);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].style, PlannedStyle::Stop { trigger: dec!(100) });
        assert!(s[0].reduce_only && s[0].qty == dec!(4) && s[0].side == Side::Sell);
        let p = plan(&[(2, breakeven)], &flat(), &spec(), &mut st, 6);
        assert!(p.actions.is_empty());
        assert!(p.refusals[0].reason.contains("no position"));
    }

    #[test]
    fn every_intent_kind_in_the_spec_table_is_planned() {
        // FEAT-007 §6's table, one row each; the detailed behaviour of each is
        // the test named after it above.
        let kinds = [
            "hold", "target", "order", "bracket", "protect", "close", "add", "reduce", "cancel",
        ];
        let mut st = PlannerState::default();
        let mut v = flat();
        v.position = dec!(1);
        v.avg_entry_price = Some(dec!(100));
        let size = RSize::Units { x: 1.0 };
        let d = RDist {
            amount: 1.0,
            unit: RUnit::Pct,
            from: Anchor::Entry,
        };
        let intents = [
            ResolvedIntent::Hold,
            ResolvedIntent::Target {
                size: size.clone(),
                via: None,
            },
            ResolvedIntent::Order(ROrder {
                side: Side::Buy,
                size: size.clone(),
                style: RStyle::Market,
                tif: TimeInForce::Gtc,
                reduce_only: false,
                post_only: false,
                tag: None,
            }),
            ResolvedIntent::Bracket {
                entry: REntry::Target {
                    size: RSize::Units { x: 2.0 },
                    via: None,
                },
                stop: d.clone(),
                take_profit: None,
                trailing: None,
            },
            ResolvedIntent::Protect {
                stop: Some(d.clone()),
                take_profit: None,
                trailing: None,
            },
            ResolvedIntent::Close {
                fraction: 1.0,
                via: None,
            },
            ResolvedIntent::Add {
                size: size.clone(),
                via: None,
            },
            ResolvedIntent::Reduce { size, via: None },
            ResolvedIntent::Cancel { tag: None },
        ];
        assert_eq!(intents.len(), kinds.len());
        for (k, i) in kinds.iter().zip(intents) {
            let p = plan(&[(0, i)], &v, &spec(), &mut st, 1);
            assert!(p.refusals.is_empty(), "{k}: {:?}", p.refusals);
        }
    }

    #[test]
    fn risk_sizing_sizes_by_the_stop_distance() {
        let mut st = PlannerState::default();
        // Risk 1% of 10000 = 100 with a stop 2 price units away → 50 units.
        let intent = ResolvedIntent::Target {
            size: RSize::Risk {
                fraction: 0.01,
                stop: RDist {
                    amount: 2.0,
                    unit: RUnit::Abs,
                    from: Anchor::Mark,
                },
            },
            via: None,
        };
        let p = plan(&[(0, intent)], &flat(), &spec(), &mut st, 1);
        assert_eq!(submits(&p)[0].qty, dec!(50));
    }
}
