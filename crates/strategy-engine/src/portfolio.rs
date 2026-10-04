//! What the environment tells the engine about the account and the instrument.
//!
//! The engine keeps **no guess** of its own position (FEAT-007 §1 defect 9 — the
//! backtester's handler kept one because its venue reported nothing back). Every
//! `state.*` value a strategy reads comes from the [`PortfolioView`] the
//! environment passes with each event, and every order the planner sizes is
//! rounded by the [`InstrumentSpec`] the environment supplies.
//!
//! Quantities and prices here are `Decimal` (ADR-0002): they come from a venue
//! and go back to one. The graph computes in `f64`, and the conversion happens
//! once each way — reading a `state.*` value, and the planner's rounding.

use domain::order::{Side, TimeInForce};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A working order this strategy instance owns, as the venue reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingOrder {
    /// The tag the planner gave it.
    pub tag: String,
    pub side: Side,
    /// Unfilled quantity, positive.
    pub remaining: Decimal,
    pub reduce_only: bool,
    /// Placed by the planner as protection (a bracket's stop or target, a
    /// `protect`), not as an entry.
    pub protective: bool,
}

/// The account and position as the environment sees them at the decision instant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioView {
    /// Signed: positive long, negative short.
    pub position: Decimal,
    /// Volume-weighted entry price of the open position.
    pub avg_entry_price: Option<Decimal>,
    /// Decision ticks since the position was opened (0 when flat).
    pub bars_in_position: u32,
    /// The best price since entry in the position's favour — the anchor of a
    /// trailing distance.
    pub high_water: Option<Decimal>,
    pub equity: Decimal,
    pub cash: Decimal,
    pub realized_pnl_today: Decimal,
    /// The latest price the environment holds for the bound instrument.
    pub mark: Option<Decimal>,
    pub open_orders: Vec<WorkingOrder>,
    pub last_fill_price: Option<Decimal>,
    /// What the selection this instance trades for said about its instrument,
    /// when it is a member of one (U-6.6). Absent from the serialised view when
    /// `None`, so every journaled view from before it reads the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SelectionContext>,
}

/// A selection-bound member's view of the selection that picked it (U-6.6):
/// which selection, and the numeric attributes its latest run gave this
/// member. `selection_attr` reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionContext {
    /// The selection strategy's name, as its graph declares it.
    pub name: String,
    /// The selection strategy's revision.
    pub revision: String,
    pub attributes: std::collections::BTreeMap<String, Decimal>,
}

impl PortfolioView {
    #[must_use]
    pub fn is_flat(&self) -> bool {
        self.position.is_zero()
    }

    /// Position as a fraction of equity at the mark; `None` without both.
    #[must_use]
    pub fn position_fraction(&self) -> Option<f64> {
        let mark = self.mark?;
        if self.equity.is_zero() {
            return None;
        }
        (self.position * mark / self.equity).to_f64()
    }

    /// Unrealised return of the open position at the mark; `None` when flat or
    /// without an entry price or mark.
    #[must_use]
    pub fn unrealized_return(&self) -> Option<f64> {
        if self.is_flat() {
            return None;
        }
        let (entry, mark) = (self.avg_entry_price?, self.mark?);
        if entry.is_zero() {
            return None;
        }
        let r = (mark - entry) / entry;
        let signed = if self.position.is_sign_negative() {
            -r
        } else {
            r
        };
        signed.to_f64()
    }
}

/// What the venue allows and how it rounds, for the bound instrument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentSpec {
    pub instrument: String,
    /// Smallest quantity increment.
    pub lot_size: Decimal,
    /// Smallest price increment.
    pub tick_size: Decimal,
    /// Smallest order the venue accepts.
    pub min_qty: Decimal,
    pub caps: VenueCaps,
}

/// Order features the venue supports natively. What it lacks the planner
/// emulates (OCO) or refuses by name (FEAT-007 §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct VenueCaps {
    pub limit: bool,
    pub stop: bool,
    pub stop_limit: bool,
    pub trailing_stop: bool,
    pub post_only: bool,
    pub reduce_only: bool,
    /// Native one-cancels-other. Emulated by the planner when false.
    pub oco: bool,
}

impl VenueCaps {
    /// Everything — what the native simulated and paper venues offer.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            limit: true,
            stop: true,
            stop_limit: true,
            trailing_stop: true,
            post_only: true,
            reduce_only: true,
            oco: true,
        }
    }

    /// Market orders only.
    #[must_use]
    pub const fn market_only() -> Self {
        Self {
            limit: false,
            stop: false,
            stop_limit: false,
            trailing_stop: false,
            post_only: false,
            reduce_only: false,
            oco: false,
        }
    }
}

/// An order fill the environment reports back to the engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    pub tag: String,
    pub side: Side,
    pub qty: Decimal,
    pub price: Decimal,
    pub at_ns: i64,
}

/// A working order's terminal state other than a fill, reported back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderClosed {
    pub tag: String,
    pub reason: OrderClosedReason,
    pub at_ns: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderClosedReason {
    Cancelled,
    Rejected,
    Expired,
}

/// The default time in force when an intent does not say: good till cancelled.
pub const DEFAULT_TIF: TimeInForce = TimeInForce::Gtc;

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn unrealized_return_is_signed_by_side() {
        let mut v = PortfolioView {
            position: dec!(2),
            avg_entry_price: Some(dec!(100)),
            mark: Some(dec!(110)),
            equity: dec!(1000),
            ..Default::default()
        };
        assert_eq!(v.unrealized_return(), Some(0.1));
        v.position = dec!(-2);
        assert_eq!(v.unrealized_return(), Some(-0.1));
        v.position = Decimal::ZERO;
        assert_eq!(v.unrealized_return(), None, "flat has no unrealised return");
    }

    #[test]
    fn position_fraction_needs_a_mark_and_equity() {
        let v = PortfolioView {
            position: dec!(1),
            mark: Some(dec!(250)),
            equity: dec!(1000),
            ..Default::default()
        };
        assert_eq!(v.position_fraction(), Some(0.25));
        assert_eq!(
            PortfolioView {
                mark: None,
                ..v.clone()
            }
            .position_fraction(),
            None
        );
    }
}
