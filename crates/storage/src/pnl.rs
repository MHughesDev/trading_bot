//! FIFO P&L lot engine (C-073/C-105).
//!
//! - Opening fills create `PnlLot` rows.
//! - Closing fills consume the oldest open lots first (FIFO) and produce `PnlClose` rows.
//! - Unrealized P&L is computed at current mark price.
//! - Win rate is position-level: % of fully-closed positions that closed net-positive.
//!
//! This module provides an in-memory engine for use in tests and the API rollup.
//! The DB-backed version persists lots/closes via the `pnl_lots` and `pnl_closes` tables
//! (see migration 0008).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ledger::AccountMode;
use domain::order::Side;

/// An open FIFO lot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlLot {
    pub id: Uuid,
    pub user_id: Uuid,
    pub account_mode: AccountMode,
    pub instrument_id: String,
    pub open_event_id: Uuid,
    /// Trade side — stored as the `Side` enum (not a String) to avoid per-lot allocation (#31).
    pub side: Side,
    pub open_qty: Decimal,
    pub remaining_qty: Decimal,
    pub open_price: Decimal,
    /// USD/quote-currency rate at open; 1.0 for USD-quoted instruments.
    pub open_usd_rate: Decimal,
    pub opened_at: DateTime<Utc>,
}

/// A FIFO close record — one per lot consumed on a closing fill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PnlClose {
    pub id: Uuid,
    pub lot_id: Uuid,
    pub close_event_id: Uuid,
    pub close_qty: Decimal,
    pub close_price: Decimal,
    /// Realized P&L in USD for this lot slice.
    pub realized_usd: Decimal,
    pub closed_at: DateTime<Utc>,
}

/// Key for grouping lots: (user_id, account_mode_str, instrument_id).
type LotKey = (Uuid, String, String);

/// In-memory FIFO P&L engine.
///
/// Thread-safety is left to the caller — wrap in `Mutex` / `RwLock` as needed.
///
/// `Arc<PnlLot>` is used in both `lots` and `lot_archive` so that a single heap
/// allocation is shared between the two containers — no struct clone on insert (#35, #36).
#[derive(Debug, Default)]
pub struct FifoEngine {
    /// Open lots, oldest first (front = next to consume).
    lots: HashMap<LotKey, VecDeque<Arc<PnlLot>>>,
    /// Archive of every lot ever opened — used for lookups after the lot is consumed.
    lot_archive: HashMap<Uuid, Arc<PnlLot>>,
    /// All recorded close slices.
    closes: Vec<PnlClose>,
}

impl FifoEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(user_id: Uuid, mode: AccountMode, instrument_id: &str) -> LotKey {
        (user_id, mode.as_str().to_owned(), instrument_id.to_owned())
    }

    /// Record an opening fill: creates a new lot.
    ///
    /// The lot is wrapped in `Arc` once and shared between the active queue and the
    /// archive — no struct clone (#35, #36).
    #[allow(clippy::too_many_arguments)]
    pub fn open_lot(
        &mut self,
        user_id: Uuid,
        account_mode: AccountMode,
        instrument_id: &str,
        open_event_id: Uuid,
        side: Side,
        qty: Decimal,
        price: Decimal,
        usd_rate: Decimal,
    ) -> Arc<PnlLot> {
        let lot = Arc::new(PnlLot {
            id: Uuid::new_v4(),
            user_id,
            account_mode,
            instrument_id: instrument_id.to_owned(),
            open_event_id,
            side,
            open_qty: qty,
            remaining_qty: qty,
            open_price: price,
            open_usd_rate: usd_rate,
            opened_at: Utc::now(),
        });
        // Share via refcount — no struct clone.
        self.lot_archive.insert(lot.id, Arc::clone(&lot));
        self.lots
            .entry(Self::key(user_id, account_mode, instrument_id))
            .or_default()
            .push_back(Arc::clone(&lot));
        lot
    }

    /// Consume open lots FIFO for a closing fill.  Returns the close records created.
    ///
    /// `usd_rate` is the USD conversion rate at close time.
    #[allow(clippy::too_many_arguments)]
    pub fn close_lots(
        &mut self,
        user_id: Uuid,
        account_mode: AccountMode,
        instrument_id: &str,
        close_event_id: Uuid,
        mut close_qty: Decimal,
        close_price: Decimal,
        usd_rate: Decimal,
    ) -> Vec<PnlClose> {
        let key = Self::key(user_id, account_mode, instrument_id);
        let queue = match self.lots.get_mut(&key) {
            Some(q) => q,
            None => return vec![],
        };

        let mut records = vec![];
        let now = Utc::now();

        while close_qty > Decimal::ZERO {
            let lot = match queue.front_mut() {
                Some(l) => l,
                None => break,
            };

            let consumed = close_qty.min(lot.remaining_qty);
            // Correct cross-currency P&L (H-6):
            //   realized_usd = close_notional_usd − open_notional_usd
            //                = close_price × qty × close_usd_rate
            //                  − open_price × qty × open_usd_rate
            //
            // Using a single close-time rate for the entire spread
            // (the previous bug) over-/understates P&L whenever the
            // quote-currency FX rate moved between open and close.
            let realized_usd =
                close_price * consumed * usd_rate - lot.open_price * consumed * lot.open_usd_rate;

            let close = PnlClose {
                id: Uuid::new_v4(),
                lot_id: lot.id,
                close_event_id,
                close_qty: consumed,
                close_price,
                realized_usd,
                closed_at: now,
            };
            records.push(close.clone());
            self.closes.push(close);

            lot.remaining_qty -= consumed;
            close_qty -= consumed;

            if lot.remaining_qty <= Decimal::ZERO {
                queue.pop_front();
            }
        }

        records
    }

    /// Total realized P&L in USD for a user + mode.
    pub fn total_realized_usd(&self, user_id: Uuid, account_mode: AccountMode) -> Decimal {
        let mode_str = account_mode.as_str();
        self.closes
            .iter()
            .filter(|c| {
                // Match via the lot reference.
                self.lot_matches(c.lot_id, user_id, mode_str)
            })
            .map(|c| c.realized_usd)
            .sum()
    }

    fn lot_matches(&self, lot_id: Uuid, user_id: Uuid, mode_str: &str) -> bool {
        self.lot_archive
            .get(&lot_id)
            .map(|l| l.user_id == user_id && l.account_mode.as_str() == mode_str)
            .unwrap_or(false)
    }

    /// Unrealized P&L for a specific instrument at `mark_price`.
    pub fn unrealized_usd(
        &self,
        user_id: Uuid,
        account_mode: AccountMode,
        instrument_id: &str,
        mark_price: Decimal,
        usd_rate: Decimal,
    ) -> Decimal {
        let key = Self::key(user_id, account_mode, instrument_id);
        let queue = match self.lots.get(&key) {
            Some(q) => q,
            None => return Decimal::ZERO,
        };
        queue
            .iter()
            .map(|lot| (mark_price - lot.open_price) * lot.remaining_qty * usd_rate)
            .sum()
    }

    /// Win rate: fraction of *position-level* closes that are net-positive.
    ///
    /// A "position" is identified as one continuous open → close cycle per instrument.
    /// Here we aggregate all close records per instrument and compute net realized P&L.
    pub fn win_rate(&self, user_id: Uuid, account_mode: AccountMode) -> f64 {
        let mode_str = account_mode.as_str();

        // Group close realized_usd by instrument_id.
        let mut by_instrument: HashMap<String, Decimal> = HashMap::new();
        for close in &self.closes {
            if let Some(lot) = self.lot_archive.get(&close.lot_id) {
                if lot.user_id == user_id && lot.account_mode.as_str() == mode_str {
                    *by_instrument.entry(lot.instrument_id.clone()).or_default() +=
                        close.realized_usd;
                }
            }
        }

        if by_instrument.is_empty() {
            return 0.0;
        }

        let winners = by_instrument
            .values()
            .filter(|&&v| v > Decimal::ZERO)
            .count();
        winners as f64 / by_instrument.len() as f64
    }

    /// Snapshot of all current closes (for rollup queries).
    pub fn closes(&self) -> &[PnlClose] {
        &self.closes
    }

    /// Snapshot of all open lots (still partially or fully open) across all keys.
    pub fn open_lots(&self) -> impl Iterator<Item = &PnlLot> {
        self.lots.values().flatten()
    }

    /// All lots ever opened (including fully closed ones).
    pub fn all_lots(&self) -> impl Iterator<Item = &PnlLot> {
        self.lot_archive.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn uid() -> Uuid {
        Uuid::new_v4()
    }

    fn eid() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn two_opens_partial_close_fifo() {
        let user = uid();
        let mut engine = FifoEngine::new();
        let mode = AccountMode::Paper;

        // Buy 2 BTC at 100 then 1 BTC at 200.
        engine.open_lot(user, mode, "BTC-USD", eid(), dec(2), dec(100), Decimal::ONE);
        engine.open_lot(user, mode, "BTC-USD", eid(), dec(1), dec(200), Decimal::ONE);

        // Sell 2 BTC at 150 — should consume lot1 fully (2@100) first.
        let closes =
            engine.close_lots(user, mode, "BTC-USD", eid(), dec(2), dec(150), Decimal::ONE);

        assert_eq!(closes.len(), 1, "one close for one lot fully consumed");
        // realized = (150-100)*2*1 = 100
        assert_eq!(closes[0].realized_usd, dec(100));
        assert_eq!(closes[0].close_qty, dec(2));
    }

    #[test]
    fn partial_close_leaves_remainder() {
        let user = uid();
        let mut engine = FifoEngine::new();
        let mode = AccountMode::Paper;

        engine.open_lot(
            user,
            mode,
            "ETH-USD",
            eid(),
            dec(3),
            dec(2000),
            Decimal::ONE,
        );
        // Sell only 1 — lot should have 2 remaining.
        let closes = engine.close_lots(
            user,
            mode,
            "ETH-USD",
            eid(),
            dec(1),
            dec(2500),
            Decimal::ONE,
        );
        assert_eq!(closes.len(), 1);
        // realized = (2500-2000)*1 = 500
        assert_eq!(closes[0].realized_usd, dec(500));

        // Remaining unrealized at 2600.
        let upnl = engine.unrealized_usd(user, mode, "ETH-USD", dec(2600), Decimal::ONE);
        // (2600-2000)*2 = 1200
        assert_eq!(upnl, dec(1200));
    }

    #[test]
    fn win_rate_positive_and_negative() {
        let user = uid();
        let mut engine = FifoEngine::new();
        let mode = AccountMode::Paper;

        // BTC: open 1 @ 100, close @ 200 — win (+100)
        engine.open_lot(user, mode, "BTC-USD", eid(), dec(1), dec(100), Decimal::ONE);
        engine.close_lots(user, mode, "BTC-USD", eid(), dec(1), dec(200), Decimal::ONE);

        // ETH: open 1 @ 2000, close @ 1500 — loss (-500)
        engine.open_lot(
            user,
            mode,
            "ETH-USD",
            eid(),
            dec(1),
            dec(2000),
            Decimal::ONE,
        );
        engine.close_lots(
            user,
            mode,
            "ETH-USD",
            eid(),
            dec(1),
            dec(1500),
            Decimal::ONE,
        );

        let wr = engine.win_rate(user, mode);
        // 1 win, 1 loss → 50%
        assert!((wr - 0.5).abs() < 1e-9);
    }

    #[test]
    fn no_lots_returns_zero_unrealized() {
        let user = uid();
        let engine = FifoEngine::new();
        let upnl =
            engine.unrealized_usd(user, AccountMode::Paper, "SOL-USD", dec(100), Decimal::ONE);
        assert_eq!(upnl, Decimal::ZERO);
    }

    #[test]
    fn cross_currency_pnl_uses_correct_fx_rates() {
        // H-6: BTC/EUR position — EUR/USD moved between open and close.
        // Open: 1 BTC @ EUR 30_000, EUR/USD = 1.10  → open USD = 33_000
        // Close: 1 BTC @ EUR 31_000, EUR/USD = 1.20 → close USD = 37_200
        // Realized USD = 37_200 − 33_000 = 4_200
        let user = uid();
        let mut engine = FifoEngine::new();
        let mode = AccountMode::Paper;

        let open_eur_usd = Decimal::from_str("1.10").unwrap();
        let close_eur_usd = Decimal::from_str("1.20").unwrap();
        let open_price = Decimal::from(30_000);
        let close_price = Decimal::from(31_000);

        engine.open_lot(
            user,
            mode,
            "BTC-EUR",
            eid(),
            dec(1),
            open_price,
            open_eur_usd,
        );
        let closes = engine.close_lots(
            user,
            mode,
            "BTC-EUR",
            eid(),
            dec(1),
            close_price,
            close_eur_usd,
        );

        assert_eq!(closes.len(), 1);
        let expected = Decimal::from_str("4200").unwrap();
        assert_eq!(
            closes[0].realized_usd, expected,
            "cross-currency P&L must use lot.open_usd_rate for open leg"
        );
    }

    fn dec(n: i64) -> Decimal {
        Decimal::from(n)
    }
}
