//! USD P&L rollup engine (C-073/C-105/C-079/C-080/C-081).
//!
//! Aggregates realized/unrealized P&L and win rate into three tiers:
//!   platform-wide → per-asset-class → per-venue
//!
//! Computed on-demand when the Dashboard requests it.  Never runs in the background.
//! Paper and Live are always separate account levels.

pub mod paper;

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use storage::{
    ledger::AccountMode,
    pnl::{FifoEngine, PnlClose, PnlLot},
};
use uuid::Uuid;

/// Per-venue P&L tile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueTile {
    pub venue: String,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub win_rate: f64,
    pub trade_count: usize,
}

/// Per-asset-class P&L tile (aggregates its venue tiles).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetClassTile {
    pub asset_class: String,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub win_rate: f64,
    pub venues: Vec<VenueTile>,
    /// Paper mode only: internal account data for this asset class
    /// (cash, equity, margin, open positions).  `None` in live mode.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub account: Option<paper::PaperAccountInfo>,
}

/// Platform-wide rollup response.
#[derive(Debug, Clone, Serialize)]
pub struct RollupResponse {
    pub mode: &'static str,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub win_rate: f64,
    pub by_asset_class: Vec<AssetClassTile>,
    /// Paper mode only: bot-wide account totals across all asset classes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_totals: Option<paper::PaperAccountTotals>,
}

/// A mark price for an instrument at query time.
#[derive(Debug, Clone)]
pub struct MarkPrice {
    pub instrument_id: String,
    pub mark: Decimal,
    pub usd_rate: Decimal,
}

/// Compute the rollup from in-memory lot/close data.
///
/// `lots` and `closes` come from the FIFO engine or a DB query.
/// `marks` provides current mark prices for unrealized P&L.
///
/// Returns the platform-wide response with per-asset-class and per-venue breakdown.
pub fn compute_rollup(
    user_id: Uuid,
    mode: AccountMode,
    lots: &[PnlLot],
    closes: &[PnlClose],
    marks: &[MarkPrice],
    // venue_map: instrument_id → (venue_slug, asset_class_slug)
    venue_map: &HashMap<String, (String, String)>,
) -> RollupResponse {
    let mark_by_id: HashMap<&str, &MarkPrice> = marks
        .iter()
        .map(|m| (m.instrument_id.as_str(), m))
        .collect();

    // ── Realized P&L per instrument (lot_id → instrument info via lots) ─────────
    let lot_by_id: HashMap<Uuid, &PnlLot> = lots.iter().map(|l| (l.id, l)).collect();

    // Single pass over closes: accumulate realized P&L and trade counts together.
    let mut realized_by_instrument: HashMap<String, Decimal> = HashMap::new();
    let mut trade_count_by_instrument: HashMap<String, usize> = HashMap::new();
    for close in closes {
        if let Some(lot) = lot_by_id.get(&close.lot_id) {
            if lot.user_id != user_id || lot.account_mode.as_str() != mode.as_str() {
                continue;
            }
            *realized_by_instrument
                .entry(lot.instrument_id.clone())
                .or_default() += close.realized_usd;
            *trade_count_by_instrument
                .entry(lot.instrument_id.clone())
                .or_default() += 1;
        }
    }

    // ── Unrealized P&L per instrument ───────────────────────────────────────────
    let mut unrealized_by_instrument: HashMap<String, Decimal> = HashMap::new();
    for lot in lots {
        if lot.user_id != user_id || lot.account_mode.as_str() != mode.as_str() {
            continue;
        }
        if lot.remaining_qty <= Decimal::ZERO {
            continue;
        }
        if let Some(mark) = mark_by_id.get(lot.instrument_id.as_str()) {
            let upnl = (mark.mark - lot.open_price) * lot.remaining_qty * mark.usd_rate;
            *unrealized_by_instrument
                .entry(lot.instrument_id.clone())
                .or_default() += upnl;
        }
    }

    // ── Win rate (position-level) ────────────────────────────────────────────────
    let total_positions = realized_by_instrument.len();
    let winning_positions = realized_by_instrument
        .values()
        .filter(|&&v| v > Decimal::ZERO)
        .count();
    let platform_win_rate = if total_positions == 0 {
        0.0
    } else {
        winning_positions as f64 / total_positions as f64
    };

    // ── Group by (asset_class, venue) ────────────────────────────────────────────
    // venue_tile: (asset_class, venue) → (realized, unrealized, wins, total, trades)
    #[derive(Default)]
    struct Bucket {
        realized: Decimal,
        unrealized: Decimal,
        wins: usize,
        total: usize,
        trades: usize,
    }

    let mut buckets: HashMap<(String, String), Bucket> = HashMap::new();

    let all_instruments: std::collections::HashSet<_> = realized_by_instrument
        .keys()
        .chain(unrealized_by_instrument.keys())
        .collect();

    for instrument_id in all_instruments {
        let (venue, asset_class) = match venue_map.get(instrument_id.as_str()) {
            Some(t) => t.clone(),
            None => ("unknown".to_owned(), "unknown".to_owned()),
        };
        let bucket = buckets.entry((asset_class, venue)).or_default();
        let r = realized_by_instrument
            .get(instrument_id.as_str())
            .copied()
            .unwrap_or(Decimal::ZERO);
        let u = unrealized_by_instrument
            .get(instrument_id.as_str())
            .copied()
            .unwrap_or(Decimal::ZERO);
        bucket.realized += r;
        bucket.unrealized += u;
        bucket.trades += trade_count_by_instrument
            .get(instrument_id.as_str())
            .copied()
            .unwrap_or(0);
        if r != Decimal::ZERO {
            bucket.total += 1;
            if r > Decimal::ZERO {
                bucket.wins += 1;
            }
        }
    }

    // ── Aggregate into tiers ─────────────────────────────────────────────────────
    let mut by_asset_class: HashMap<String, (Decimal, Decimal, usize, usize, Vec<VenueTile>)> =
        HashMap::new();

    for ((asset_class, venue), bucket) in &buckets {
        let wr = if bucket.total == 0 {
            0.0
        } else {
            bucket.wins as f64 / bucket.total as f64
        };
        let tile = VenueTile {
            venue: venue.clone(),
            realized_pnl_usd: bucket.realized,
            unrealized_pnl_usd: bucket.unrealized,
            win_rate: wr,
            trade_count: bucket.trades,
        };
        let entry = by_asset_class
            .entry(asset_class.clone())
            .or_insert_with(|| (Decimal::ZERO, Decimal::ZERO, 0, 0, vec![]));
        entry.0 += bucket.realized;
        entry.1 += bucket.unrealized;
        entry.2 += bucket.wins;
        entry.3 += bucket.total;
        entry.4.push(tile);
    }

    let mut asset_class_tiles: Vec<AssetClassTile> = by_asset_class
        .into_iter()
        .map(|(ac, (r, u, wins, total, venues))| {
            let wr = if total == 0 {
                0.0
            } else {
                wins as f64 / total as f64
            };
            AssetClassTile {
                asset_class: ac,
                realized_pnl_usd: r,
                unrealized_pnl_usd: u,
                win_rate: wr,
                venues,
                account: None,
            }
        })
        .collect();
    asset_class_tiles.sort_by(|a, b| a.asset_class.cmp(&b.asset_class));

    let platform_realized: Decimal = asset_class_tiles.iter().map(|t| t.realized_pnl_usd).sum();
    let platform_unrealized: Decimal = asset_class_tiles.iter().map(|t| t.unrealized_pnl_usd).sum();

    RollupResponse {
        mode: mode.as_str(),
        realized_pnl_usd: platform_realized,
        unrealized_pnl_usd: platform_unrealized,
        win_rate: platform_win_rate,
        by_asset_class: asset_class_tiles,
        account_totals: None,
    }
}

/// Convenience wrapper: build a `FifoEngine` from lot/close slices and run the rollup.
pub fn rollup_from_engine(
    engine: &FifoEngine,
    user_id: Uuid,
    mode: AccountMode,
    marks: &[MarkPrice],
    venue_map: &HashMap<String, (String, String)>,
) -> RollupResponse {
    // Must include all lots (including fully consumed) so compute_rollup can resolve close → lot.
    let lots: Vec<PnlLot> = engine.all_lots().cloned().collect();
    let closes = engine.closes().to_vec();
    compute_rollup(user_id, mode, &lots, &closes, marks, venue_map)
}
