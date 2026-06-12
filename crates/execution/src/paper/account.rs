//! Internal paper account — cash, positions, realized P&L for one asset class.
//!
//! All accounting is local `Decimal` arithmetic; nothing here performs I/O.
//! The account enforces the buying-power rules of its [`AccountPolicy`]:
//!
//! - **Cash**: buys debit `qty × price × multiplier + fee` up front and are
//!   rejected when cash is insufficient; sells are rejected beyond the held
//!   quantity (long-only).
//! - **Margin**: long and short allowed.  Orders that increase exposure must
//!   leave `equity − used_margin ≥ 0`; reducing exposure is always allowed and
//!   settles realized P&L to cash.
//! - **Binary**: cash semantics on prices in `[0, 1]`, plus
//!   [`PaperAccount::settle_position`] at 0 or 1 when the market resolves.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use domain::instrument::AssetClass;
use domain::order::Side;
use rust_decimal::prelude::Signed;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ledger::{PaperLedger, PaperLedgerEntry, PaperLedgerKind};
use super::policy::{AccountKind, AccountPolicy};
use super::PaperFill;

/// Why the paper engine refused to execute or settle.
#[derive(Debug, Error)]
pub enum PaperTradeError {
    #[error("insufficient cash: need {required}, have {available}")]
    InsufficientCash {
        required: Decimal,
        available: Decimal,
    },
    #[error("insufficient margin: free collateral {available} < required {required}")]
    InsufficientMargin {
        required: Decimal,
        available: Decimal,
    },
    #[error("insufficient position in {instrument_id}: selling {requested}, hold {held}")]
    InsufficientPosition {
        instrument_id: String,
        requested: Decimal,
        held: Decimal,
    },
    #[error("no mark price for {instrument_id} — cannot fill")]
    NoMarkPrice { instrument_id: String },
    #[error("no open position in {instrument_id}")]
    NoPosition { instrument_id: String },
    #[error("unknown paper order: {0}")]
    UnknownOrder(String),
}

/// One open position inside a paper account (quantity is signed).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperPosition {
    /// Positive = long, negative = short (shorts only on margin accounts).
    pub quantity: Decimal,
    /// Volume-weighted average entry price.
    pub average_entry_price: Decimal,
    pub opened_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Read-only view of a position with mark-dependent fields resolved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperPositionView {
    pub instrument_id: String,
    pub quantity: Decimal,
    pub average_entry_price: Decimal,
    /// Latest mark, when one has been observed for this instrument.
    pub mark_price: Option<Decimal>,
    pub unrealized_pnl: Decimal,
    /// `|quantity| × mark × multiplier` (entry price when no mark yet).
    pub notional: Decimal,
}

/// Point-in-time summary of one paper account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperAccountSnapshot {
    pub asset_class: AssetClass,
    pub currency: &'static str,
    pub cash: Decimal,
    /// Cash + market value of holdings (cash/binary) or cash + unrealized (margin).
    pub equity: Decimal,
    pub used_margin: Decimal,
    pub free_collateral: Decimal,
    pub realized_pnl: Decimal,
    pub fees_paid: Decimal,
    /// Closing trades that realized P&L (sells, margin reductions, settlements).
    pub closed_trades: u64,
    /// Closing trades whose realized P&L was positive.
    pub winning_trades: u64,
    pub positions: Vec<PaperPositionView>,
}

/// Mark-price lookup the engine passes in (reads its internal mark board).
pub type MarkLookup<'a> = &'a dyn Fn(&str) -> Option<Decimal>;

/// Internal paper account for a single asset class.
#[derive(Debug)]
pub struct PaperAccount {
    asset_class: AssetClass,
    policy: AccountPolicy,
    cash: Decimal,
    positions: HashMap<String, PaperPosition>,
    realized_pnl: Decimal,
    fees_paid: Decimal,
    closed_trades: u64,
    winning_trades: u64,
    ledger: PaperLedger,
}

impl PaperAccount {
    /// Open an account seeded with `starting_cash` of the policy's quote currency.
    pub fn new(asset_class: AssetClass, starting_cash: Decimal) -> Self {
        let policy = AccountPolicy::for_asset_class(asset_class);
        let mut ledger = PaperLedger::new(asset_class);
        ledger.record(
            PaperLedgerKind::Deposit,
            None,
            None,
            starting_cash,
            format!("opening balance ({})", policy.quote_currency),
        );
        Self {
            asset_class,
            policy,
            cash: starting_cash,
            positions: HashMap::new(),
            realized_pnl: Decimal::ZERO,
            fees_paid: Decimal::ZERO,
            closed_trades: 0,
            winning_trades: 0,
            ledger,
        }
    }

    pub fn asset_class(&self) -> AssetClass {
        self.asset_class
    }

    pub fn policy(&self) -> &AccountPolicy {
        &self.policy
    }

    pub fn cash(&self) -> Decimal {
        self.cash
    }

    pub fn realized_pnl(&self) -> Decimal {
        self.realized_pnl
    }

    pub fn position(&self, instrument_id: &str) -> Option<&PaperPosition> {
        self.positions.get(instrument_id)
    }

    pub fn positions(&self) -> &HashMap<String, PaperPosition> {
        &self.positions
    }

    /// Apply a non-zero simulated fill, enforcing the account's buying-power
    /// rules.  On `Err` the account is unchanged.
    pub fn apply_fill(
        &mut self,
        order_id: &str,
        fill: &PaperFill,
        marks: MarkLookup<'_>,
    ) -> Result<(), PaperTradeError> {
        debug_assert!(fill.filled_qty > Decimal::ZERO, "zero-qty fill applied");
        match self.policy.kind {
            AccountKind::Cash | AccountKind::Binary => self.apply_cash_fill(order_id, fill),
            AccountKind::Margin => self.apply_margin_fill(order_id, fill, marks),
        }
    }

    /// Record a rejection in the journal (no balance change).
    pub fn record_rejection(&mut self, order_id: &str, fill_instrument: &str, reason: &str) {
        self.ledger.record(
            PaperLedgerKind::Rejection,
            Some(fill_instrument.to_owned()),
            Some(order_id.to_owned()),
            Decimal::ZERO,
            reason.to_owned(),
        );
    }

    // ── Cash / binary semantics ──────────────────────────────────────────────

    fn apply_cash_fill(&mut self, order_id: &str, fill: &PaperFill) -> Result<(), PaperTradeError> {
        let mult = self.policy.contract_multiplier;
        let qty = fill.filled_qty;
        let price = fill.fill_price.inner();
        let notional = qty * price * mult;

        match fill.side {
            Side::Buy => {
                let required = notional + fill.fee;
                if self.cash < required {
                    return Err(PaperTradeError::InsufficientCash {
                        required,
                        available: self.cash,
                    });
                }
                self.cash -= required;
                self.add_to_position(&fill.instrument_id, qty, price);
                self.ledger.record(
                    PaperLedgerKind::Trade,
                    Some(fill.instrument_id.clone()),
                    Some(order_id.to_owned()),
                    -notional,
                    format!("buy {qty} @ {price}"),
                );
            }
            Side::Sell => {
                let held = self
                    .positions
                    .get(&fill.instrument_id)
                    .map_or(Decimal::ZERO, |p| p.quantity);
                if held < qty {
                    return Err(PaperTradeError::InsufficientPosition {
                        instrument_id: fill.instrument_id.clone(),
                        requested: qty,
                        held,
                    });
                }
                let entry = self.positions[&fill.instrument_id].average_entry_price;
                let realized = (price - entry) * qty * mult;
                self.realized_pnl += realized;
                self.record_close(realized);
                self.cash += notional - fill.fee;
                self.reduce_long(&fill.instrument_id, qty);
                self.ledger.record(
                    PaperLedgerKind::Trade,
                    Some(fill.instrument_id.clone()),
                    Some(order_id.to_owned()),
                    notional,
                    format!("sell {qty} @ {price}"),
                );
            }
        }
        self.charge_fee(order_id, &fill.instrument_id, fill.fee);
        Ok(())
    }

    // ── Margin semantics ─────────────────────────────────────────────────────

    fn apply_margin_fill(
        &mut self,
        order_id: &str,
        fill: &PaperFill,
        marks: MarkLookup<'_>,
    ) -> Result<(), PaperTradeError> {
        let mult = self.policy.contract_multiplier;
        let price = fill.fill_price.inner();
        let signed_fill = match fill.side {
            Side::Buy => fill.filled_qty,
            Side::Sell => -fill.filled_qty,
        };
        let (old_qty, old_avg) = self
            .positions
            .get(&fill.instrument_id)
            .map_or((Decimal::ZERO, price), |p| {
                (p.quantity, p.average_entry_price)
            });
        let new_qty = old_qty + signed_fill;

        // Realize P&L on whatever portion reduces the existing position.
        let same_direction = old_qty == Decimal::ZERO || old_qty.signum() == signed_fill.signum();
        let realized = if same_direction {
            Decimal::ZERO
        } else {
            let reduce_qty = fill.filled_qty.min(old_qty.abs());
            (price - old_avg) * reduce_qty * old_qty.signum() * mult
        };

        // Prospective entry price after the fill.
        let flipped = old_qty != Decimal::ZERO
            && new_qty != Decimal::ZERO
            && new_qty.signum() != old_qty.signum();
        let new_avg = if old_qty == Decimal::ZERO || flipped {
            price
        } else if same_direction {
            // Same-direction add: VWAP over absolute quantities.
            (old_qty.abs() * old_avg + fill.filled_qty * price) / new_qty.abs()
        } else {
            old_avg // pure reduction keeps the entry
        };

        // Margin check on the prospective state.  Pure reductions are always
        // allowed — closing risk never blocks.
        let increases_exposure = new_qty.abs() > old_qty.abs() || flipped;
        if increases_exposure {
            let prospective_cash = self.cash + realized - fill.fee;
            let equity = prospective_cash
                + self.unrealized_excluding(marks, &fill.instrument_id)
                + (price - new_avg) * new_qty * mult;
            let used = self.used_margin_excluding(marks, &fill.instrument_id)
                + new_qty.abs() * price * mult / self.policy.leverage;
            if equity < used {
                return Err(PaperTradeError::InsufficientMargin {
                    required: used,
                    available: equity,
                });
            }
        }

        // Commit.
        self.cash += realized;
        self.realized_pnl += realized;
        if !same_direction {
            self.record_close(realized);
        }
        if realized != Decimal::ZERO {
            self.ledger.record(
                PaperLedgerKind::RealizedPnl,
                Some(fill.instrument_id.clone()),
                Some(order_id.to_owned()),
                realized,
                format!("realized on {} @ {price}", fill.instrument_id),
            );
        }
        self.set_margin_position(&fill.instrument_id, new_qty, new_avg);
        self.charge_fee(order_id, &fill.instrument_id, fill.fee);
        Ok(())
    }

    // ── Settlement / funding (internal admin operations) ─────────────────────

    /// Close the whole position at `settle_price` with no fee — futures/option
    /// expiry, or binary resolution at 0/1 via [`Self::settle_binary`].
    pub fn settle_position(
        &mut self,
        instrument_id: &str,
        settle_price: Decimal,
    ) -> Result<Decimal, PaperTradeError> {
        let pos =
            self.positions
                .remove(instrument_id)
                .ok_or_else(|| PaperTradeError::NoPosition {
                    instrument_id: instrument_id.to_owned(),
                })?;
        let mult = self.policy.contract_multiplier;
        let realized = (settle_price - pos.average_entry_price) * pos.quantity * mult;
        // Cash accounts get principal back too; margin accounts settle P&L only.
        let cash_delta = match self.policy.kind {
            AccountKind::Cash | AccountKind::Binary => settle_price * pos.quantity * mult,
            AccountKind::Margin => realized,
        };
        self.cash += cash_delta;
        self.realized_pnl += realized;
        self.record_close(realized);
        self.ledger.record(
            PaperLedgerKind::Settlement,
            Some(instrument_id.to_owned()),
            None,
            cash_delta,
            format!("settled {} @ {settle_price}", pos.quantity),
        );
        Ok(realized)
    }

    /// Resolve a binary contract: winning side pays 1 per contract, losing pays 0.
    pub fn settle_binary(
        &mut self,
        instrument_id: &str,
        won: bool,
    ) -> Result<Decimal, PaperTradeError> {
        let price = if won { Decimal::ONE } else { Decimal::ZERO };
        self.settle_position(instrument_id, price)
    }

    /// Charge a perpetual-swap funding payment: longs pay when `rate > 0`.
    /// Returns the signed payment debited from cash.
    pub fn apply_funding(
        &mut self,
        instrument_id: &str,
        mark: Decimal,
        rate: Decimal,
    ) -> Result<Decimal, PaperTradeError> {
        let pos = self
            .positions
            .get(instrument_id)
            .ok_or_else(|| PaperTradeError::NoPosition {
                instrument_id: instrument_id.to_owned(),
            })?;
        let payment = pos.quantity * mark * rate * self.policy.contract_multiplier;
        self.cash -= payment;
        self.ledger.record(
            PaperLedgerKind::Funding,
            Some(instrument_id.to_owned()),
            None,
            -payment,
            format!("funding rate {rate} @ {mark}"),
        );
        Ok(payment)
    }

    // ── Valuation ────────────────────────────────────────────────────────────

    /// Account equity at the given marks.
    pub fn equity(&self, marks: MarkLookup<'_>) -> Decimal {
        let mult = self.policy.contract_multiplier;
        match self.policy.kind {
            // Cash/binary: cash + market value of holdings.
            AccountKind::Cash | AccountKind::Binary => {
                self.cash
                    + self
                        .positions
                        .iter()
                        .map(|(id, p)| {
                            p.quantity * marks(id).unwrap_or(p.average_entry_price) * mult
                        })
                        .sum::<Decimal>()
            }
            // Margin: cash + unrealized P&L.
            AccountKind::Margin => self.cash + self.unrealized_excluding(marks, ""),
        }
    }

    /// Collateral reserved by open positions (`Margin` accounts; 0 elsewhere).
    pub fn used_margin(&self, marks: MarkLookup<'_>) -> Decimal {
        if self.policy.kind != AccountKind::Margin {
            return Decimal::ZERO;
        }
        self.used_margin_excluding(marks, "")
    }

    /// Full account summary at the given marks.
    pub fn snapshot(&self, marks: MarkLookup<'_>) -> PaperAccountSnapshot {
        let mult = self.policy.contract_multiplier;
        let mut positions: Vec<PaperPositionView> = self
            .positions
            .iter()
            .map(|(id, p)| {
                let mark = marks(id);
                let effective = mark.unwrap_or(p.average_entry_price);
                PaperPositionView {
                    instrument_id: id.clone(),
                    quantity: p.quantity,
                    average_entry_price: p.average_entry_price,
                    mark_price: mark,
                    unrealized_pnl: (effective - p.average_entry_price) * p.quantity * mult,
                    notional: p.quantity.abs() * effective * mult,
                }
            })
            .collect();
        positions.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));

        let equity = self.equity(marks);
        let used_margin = self.used_margin(marks);
        PaperAccountSnapshot {
            asset_class: self.asset_class,
            currency: self.policy.quote_currency,
            cash: self.cash,
            equity,
            used_margin,
            free_collateral: equity - used_margin,
            realized_pnl: self.realized_pnl,
            fees_paid: self.fees_paid,
            closed_trades: self.closed_trades,
            winning_trades: self.winning_trades,
            positions,
        }
    }

    /// Journal entries at or after `since`.
    pub fn transactions_since(&self, since: Option<DateTime<Utc>>) -> Vec<PaperLedgerEntry> {
        self.ledger.entries_since(since)
    }

    // ── Internals ────────────────────────────────────────────────────────────

    /// Count a closing trade toward the account's win-rate statistics.
    fn record_close(&mut self, realized: Decimal) {
        self.closed_trades += 1;
        if realized > Decimal::ZERO {
            self.winning_trades += 1;
        }
    }

    fn charge_fee(&mut self, order_id: &str, instrument_id: &str, fee: Decimal) {
        if fee == Decimal::ZERO {
            return;
        }
        // Cash-account fees were already debited with the principal; margin
        // fees are debited here.  Either way the journal carries one Fee line.
        if self.policy.kind == AccountKind::Margin {
            self.cash -= fee;
        }
        self.fees_paid += fee;
        self.ledger.record(
            PaperLedgerKind::Fee,
            Some(instrument_id.to_owned()),
            Some(order_id.to_owned()),
            -fee,
            String::new(),
        );
    }

    fn add_to_position(&mut self, instrument_id: &str, qty: Decimal, price: Decimal) {
        let now = Utc::now();
        let pos = self
            .positions
            .entry(instrument_id.to_owned())
            .or_insert(PaperPosition {
                quantity: Decimal::ZERO,
                average_entry_price: price,
                opened_at: now,
                updated_at: now,
            });
        let new_qty = pos.quantity + qty;
        if new_qty != Decimal::ZERO {
            pos.average_entry_price =
                (pos.quantity * pos.average_entry_price + qty * price) / new_qty;
        }
        pos.quantity = new_qty;
        pos.updated_at = now;
    }

    fn reduce_long(&mut self, instrument_id: &str, qty: Decimal) {
        if let Some(pos) = self.positions.get_mut(instrument_id) {
            pos.quantity -= qty;
            pos.updated_at = Utc::now();
            if pos.quantity == Decimal::ZERO {
                self.positions.remove(instrument_id);
            }
        }
    }

    /// Store a margin position's post-fill quantity and entry price.
    fn set_margin_position(&mut self, instrument_id: &str, new_qty: Decimal, new_avg: Decimal) {
        let now = Utc::now();
        if new_qty == Decimal::ZERO {
            self.positions.remove(instrument_id);
            return;
        }
        let pos = self
            .positions
            .entry(instrument_id.to_owned())
            .or_insert(PaperPosition {
                quantity: Decimal::ZERO,
                average_entry_price: new_avg,
                opened_at: now,
                updated_at: now,
            });
        pos.quantity = new_qty;
        pos.average_entry_price = new_avg;
        pos.updated_at = now;
    }

    /// Unrealized P&L across positions, skipping `skip_id` (pass `""` to skip
    /// nothing).  Instruments without a mark are valued at their entry price.
    fn unrealized_excluding(&self, marks: MarkLookup<'_>, skip_id: &str) -> Decimal {
        let mult = self.policy.contract_multiplier;
        self.positions
            .iter()
            .filter(|(id, _)| id.as_str() != skip_id)
            .map(|(id, p)| {
                let mark = marks(id).unwrap_or(p.average_entry_price);
                (mark - p.average_entry_price) * p.quantity * mult
            })
            .sum()
    }

    /// Collateral reserved by positions other than `skip_id`
    /// (`Σ |qty| × mark × multiplier / leverage`).
    fn used_margin_excluding(&self, marks: MarkLookup<'_>, skip_id: &str) -> Decimal {
        let mult = self.policy.contract_multiplier;
        let gross: Decimal = self
            .positions
            .iter()
            .filter(|(id, _)| id.as_str() != skip_id)
            .map(|(id, p)| {
                let mark = marks(id).unwrap_or(p.average_entry_price);
                p.quantity.abs() * mark * mult
            })
            .sum();
        gross / self.policy.leverage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::Price;
    use rust_decimal_macros::dec;
    use uuid::Uuid;

    fn fill(instrument: &str, side: Side, qty: Decimal, price: Decimal, fee: Decimal) -> PaperFill {
        PaperFill {
            idempotency_key: Uuid::new_v4(),
            instrument_id: instrument.to_owned(),
            side,
            filled_qty: qty,
            fill_price: Price::from_decimal(price),
            fee,
        }
    }

    fn no_marks(_: &str) -> Option<Decimal> {
        None
    }

    #[test]
    fn cash_buy_then_sell_settles_pnl_to_cash() {
        let mut acct = PaperAccount::new(AssetClass::CryptoSpotCex, dec!(100_000));
        acct.apply_fill(
            "o-1",
            &fill("BTC-USD", Side::Buy, dec!(1), dec!(50_000), dec!(50)),
            &no_marks,
        )
        .unwrap();
        assert_eq!(acct.cash(), dec!(49_950));
        assert_eq!(acct.position("BTC-USD").unwrap().quantity, dec!(1));

        acct.apply_fill(
            "o-2",
            &fill("BTC-USD", Side::Sell, dec!(1), dec!(55_000), dec!(55)),
            &no_marks,
        )
        .unwrap();
        assert_eq!(acct.cash(), dec!(104_895)); // 49 950 + 55 000 − 55
        assert_eq!(acct.realized_pnl(), dec!(5_000));
        assert!(acct.position("BTC-USD").is_none());
    }

    #[test]
    fn cash_buy_beyond_balance_is_rejected_unchanged() {
        let mut acct = PaperAccount::new(AssetClass::Equity, dec!(1_000));
        let err = acct
            .apply_fill(
                "o-1",
                &fill("AAPL", Side::Buy, dec!(100), dec!(200), Decimal::ZERO),
                &no_marks,
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::InsufficientCash { .. }));
        assert_eq!(acct.cash(), dec!(1_000));
        assert!(acct.position("AAPL").is_none());
    }

    #[test]
    fn cash_oversell_is_rejected() {
        let mut acct = PaperAccount::new(AssetClass::Equity, dec!(10_000));
        acct.apply_fill(
            "o-1",
            &fill("AAPL", Side::Buy, dec!(10), dec!(100), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        let err = acct
            .apply_fill(
                "o-2",
                &fill("AAPL", Side::Sell, dec!(20), dec!(100), Decimal::ZERO),
                &no_marks,
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::InsufficientPosition { .. }));
        assert_eq!(acct.position("AAPL").unwrap().quantity, dec!(10));
    }

    #[test]
    fn option_fills_use_contract_multiplier() {
        let mut acct = PaperAccount::new(AssetClass::Option, dec!(100_000));
        // 2 contracts at premium 5.00 → 2 × 5 × 100 = 1 000 debit.
        acct.apply_fill(
            "o-1",
            &fill("AAPL240621C200", Side::Buy, dec!(2), dec!(5), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        assert_eq!(acct.cash(), dec!(99_000));
        acct.apply_fill(
            "o-2",
            &fill(
                "AAPL240621C200",
                Side::Sell,
                dec!(2),
                dec!(8),
                Decimal::ZERO,
            ),
            &no_marks,
        )
        .unwrap();
        assert_eq!(acct.realized_pnl(), dec!(600)); // (8 − 5) × 2 × 100
        assert_eq!(acct.cash(), dec!(100_600));
    }

    #[test]
    fn margin_short_then_cover_realizes_profit() {
        let mut acct = PaperAccount::new(AssetClass::FuturesExpiring, dec!(100_000));
        acct.apply_fill(
            "o-1",
            &fill("ESM6", Side::Sell, dec!(2), dec!(5_000), dec!(4)),
            &no_marks,
        )
        .unwrap();
        // Short opened: no principal cash movement, only the fee.
        assert_eq!(acct.cash(), dec!(99_996));
        assert_eq!(acct.position("ESM6").unwrap().quantity, dec!(-2));

        acct.apply_fill(
            "o-2",
            &fill("ESM6", Side::Buy, dec!(2), dec!(4_900), dec!(4)),
            &no_marks,
        )
        .unwrap();
        // Realized: (4 900 − 5 000) × 2 × (−1 short sign) = +200.
        assert_eq!(acct.realized_pnl(), dec!(200));
        assert_eq!(acct.cash(), dec!(100_192));
        assert!(acct.position("ESM6").is_none());
    }

    #[test]
    fn margin_flip_reopens_at_fill_price() {
        let mut acct = PaperAccount::new(AssetClass::PerpetualSwap, dec!(100_000));
        acct.apply_fill(
            "o-1",
            &fill("BTC-PERP", Side::Buy, dec!(1), dec!(50_000), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        // Sell 3 → close 1 long, open 2 short at 52 000.
        acct.apply_fill(
            "o-2",
            &fill("BTC-PERP", Side::Sell, dec!(3), dec!(52_000), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        let pos = acct.position("BTC-PERP").unwrap();
        assert_eq!(pos.quantity, dec!(-2));
        assert_eq!(pos.average_entry_price, dec!(52_000));
        assert_eq!(acct.realized_pnl(), dec!(2_000));
    }

    #[test]
    fn margin_exposure_beyond_leverage_is_rejected() {
        // 10× leverage on 10 000 cash → max ~100 000 notional.
        let mut acct = PaperAccount::new(AssetClass::FuturesExpiring, dec!(10_000));
        let err = acct
            .apply_fill(
                "o-1",
                &fill("ESM6", Side::Buy, dec!(30), dec!(5_000), Decimal::ZERO),
                &no_marks,
            )
            .unwrap_err();
        assert!(matches!(err, PaperTradeError::InsufficientMargin { .. }));
        assert!(acct.position("ESM6").is_none());

        // Within leverage passes.
        acct.apply_fill(
            "o-2",
            &fill("ESM6", Side::Buy, dec!(15), dec!(5_000), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        assert_eq!(acct.position("ESM6").unwrap().quantity, dec!(15));
    }

    #[test]
    fn margin_reduce_is_allowed_even_when_underwater() {
        let mut acct = PaperAccount::new(AssetClass::Fx, dec!(10_000));
        acct.apply_fill(
            "o-1",
            &fill(
                "EUR-USD",
                Side::Buy,
                dec!(100_000),
                dec!(1.10),
                Decimal::ZERO,
            ),
            &no_marks,
        )
        .unwrap();
        // Price collapsed; closing must still be allowed.
        let marks = |_: &str| Some(dec!(1.02));
        acct.apply_fill(
            "o-2",
            &fill(
                "EUR-USD",
                Side::Sell,
                dec!(100_000),
                dec!(1.02),
                Decimal::ZERO,
            ),
            &marks,
        )
        .unwrap();
        assert_eq!(acct.realized_pnl(), dec!(-8_000.00));
        assert_eq!(acct.cash(), dec!(2_000.00));
    }

    #[test]
    fn binary_buy_and_settle_win_pays_one_per_contract() {
        let mut acct = PaperAccount::new(AssetClass::PredictionMarket, dec!(10_000));
        acct.apply_fill(
            "o-1",
            &fill("KX-RAIN-NYC", Side::Buy, dec!(100), dec!(0.40), dec!(0.28)),
            &no_marks,
        )
        .unwrap();
        assert_eq!(acct.cash(), dec!(9_959.72)); // 10 000 − 40 − 0.28

        let realized = acct.settle_binary("KX-RAIN-NYC", true).unwrap();
        assert_eq!(realized, dec!(60.00)); // (1 − 0.4) × 100
        assert_eq!(acct.cash(), dec!(10_059.72));
        assert!(acct.position("KX-RAIN-NYC").is_none());
    }

    #[test]
    fn binary_settle_loss_pays_nothing() {
        let mut acct = PaperAccount::new(AssetClass::PredictionMarket, dec!(10_000));
        acct.apply_fill(
            "o-1",
            &fill(
                "KX-RAIN-NYC",
                Side::Buy,
                dec!(100),
                dec!(0.40),
                Decimal::ZERO,
            ),
            &no_marks,
        )
        .unwrap();
        let realized = acct.settle_binary("KX-RAIN-NYC", false).unwrap();
        assert_eq!(realized, dec!(-40.00));
        assert_eq!(acct.cash(), dec!(9_960));
    }

    #[test]
    fn funding_debits_longs_when_rate_positive() {
        let mut acct = PaperAccount::new(AssetClass::PerpetualSwap, dec!(100_000));
        acct.apply_fill(
            "o-1",
            &fill("BTC-PERP", Side::Buy, dec!(1), dec!(50_000), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        let payment = acct
            .apply_funding("BTC-PERP", dec!(50_000), dec!(0.0001))
            .unwrap();
        assert_eq!(payment, dec!(5.00));
        assert_eq!(acct.cash(), dec!(99_995.00));
    }

    #[test]
    fn futures_settlement_realizes_at_expiry_price() {
        let mut acct = PaperAccount::new(AssetClass::FuturesExpiring, dec!(100_000));
        acct.apply_fill(
            "o-1",
            &fill("ESM6", Side::Buy, dec!(2), dec!(5_000), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        let realized = acct.settle_position("ESM6", dec!(5_100)).unwrap();
        assert_eq!(realized, dec!(200));
        assert_eq!(acct.cash(), dec!(100_200));
    }

    #[test]
    fn win_rate_counters_track_closes() {
        let mut acct = PaperAccount::new(AssetClass::Equity, dec!(10_000));
        acct.apply_fill(
            "o-1",
            &fill("AAPL", Side::Buy, dec!(10), dec!(100), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        // One winning close, one losing close.
        acct.apply_fill(
            "o-2",
            &fill("AAPL", Side::Sell, dec!(5), dec!(110), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        acct.apply_fill(
            "o-3",
            &fill("AAPL", Side::Sell, dec!(5), dec!(90), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        let snap = acct.snapshot(&no_marks);
        assert_eq!(snap.closed_trades, 2);
        assert_eq!(snap.winning_trades, 1);
    }

    #[test]
    fn ledger_net_flow_matches_cash_change() {
        let mut acct = PaperAccount::new(AssetClass::CryptoSpotCex, dec!(100_000));
        acct.apply_fill(
            "o-1",
            &fill("BTC-USD", Side::Buy, dec!(2), dec!(40_000), dec!(80)),
            &no_marks,
        )
        .unwrap();
        acct.apply_fill(
            "o-2",
            &fill("BTC-USD", Side::Sell, dec!(1), dec!(42_000), dec!(42)),
            &no_marks,
        )
        .unwrap();
        let net: Decimal = acct
            .transactions_since(None)
            .iter()
            .map(|e| e.cash_delta)
            .sum();
        assert_eq!(net, acct.cash());
    }

    #[test]
    fn snapshot_reports_unrealized_at_mark() {
        let mut acct = PaperAccount::new(AssetClass::Equity, dec!(10_000));
        acct.apply_fill(
            "o-1",
            &fill("AAPL", Side::Buy, dec!(10), dec!(100), Decimal::ZERO),
            &no_marks,
        )
        .unwrap();
        let marks = |_: &str| Some(dec!(110));
        let snap = acct.snapshot(&marks);
        assert_eq!(snap.cash, dec!(9_000));
        assert_eq!(snap.equity, dec!(10_100)); // 9 000 + 10 × 110
        assert_eq!(snap.positions.len(), 1);
        assert_eq!(snap.positions[0].unrealized_pnl, dec!(100));
    }
}
