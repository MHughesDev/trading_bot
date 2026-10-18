//! Dashboard rollup endpoint.
//!
//! `GET /api/dashboard/rollup?mode=PAPER|LIVE`
//!
//! Returns three-tier P&L and win-rate data for **the caller**. Computed
//! on-demand, never cached.
//!
//! # Live mode (Set E 2.2)
//!
//! Live P&L is read from the caller's own ledger — `pnl_lots` and their
//! `pnl_closes` (migration 0008) — not from a broker, and valued at the
//! platform's mark board (the paper engine's marks, which the live collectors
//! feed; the same source the order gate prices against). It used to be a
//! hard-coded all-zero response, so a losing week looked like a flat one.
//!
//! What cannot be valued is **surfaced**, never zeroed (locked decision 4): an
//! open position with no mark, a stale mark, a non-USD quote, or a lot whose
//! side is unreadable is listed in `mark_coverage` and left out of the
//! unrealized total, which therefore says exactly what it covers.
//!
//! Today `orders.execution_mode` is `CHECK (= 'paper')` (migration 0121), so no
//! live fill can exist and a live rollup is a truthful zero with complete
//! coverage — computed, not hard-coded.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::BearerToken;
use crate::rollup::{compute_rollup, MarkCoverage, MarkPrice, RollupResponse};
use crate::state::AppState;
use domain::order::Side;
use storage::ledger::AccountMode;
use storage::pnl::{PnlClose, PnlLot};

/// A mark older than this is stale for valuation.
pub const MARK_STALE_AFTER_SECS: i64 = 300;

#[derive(Debug, Deserialize)]
pub struct RollupQuery {
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "PAPER".to_owned()
}

/// `GET /api/dashboard/rollup?mode=PAPER|LIVE`
///
/// Paper mode is served straight from the internal paper trading engine:
/// asset-class-level account data (cash, equity, positions, P&L) plus
/// bot-wide totals. The paper engine's accounts are platform-wide (one per
/// asset class), not per user — see ADR-0048.
///
/// Live mode is the caller's own ledger, scoped by the session's user id.
pub async fn get_rollup(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<RollupQuery>,
) -> impl IntoResponse {
    let account_mode = match params.mode.to_uppercase().as_str() {
        "PAPER" => AccountMode::Paper,
        "LIVE" => AccountMode::Live,
        _other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "unknown mode; expected PAPER or LIVE" })),
            )
                .into_response();
        }
    };

    let rollup = match account_mode {
        AccountMode::Paper => crate::rollup::paper::paper_rollup(&state.paper_engine),
        AccountMode::Live => {
            let ledger = match load_ledger(&state.pg, token.user_id(), AccountMode::Live).await {
                Ok(l) => l,
                Err(e) => {
                    // An unreadable ledger is an error, not an empty one.
                    tracing::error!(error = %e, "live rollup: ledger read failed");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": "ledger_unavailable" })),
                    )
                        .into_response();
                }
            };
            let engine = state.paper_engine.clone();
            build_rollup(
                token.user_id(),
                AccountMode::Live,
                &ledger,
                &|id| engine.mark_with_time(id).map(|(p, at)| (p.inner(), at)),
                Utc::now(),
            )
        }
    };

    Json(rollup).into_response()
}

/// One ledger lot as stored, with its venue and asset class.
#[derive(Debug, Clone)]
pub struct LedgerLot {
    pub lot: PnlLot,
    /// `None` when the stored side is absent or unreadable.
    pub side_known: bool,
    pub venue: Option<String>,
    pub asset_class: Option<String>,
}

/// The caller's ledger for one mode.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    pub lots: Vec<LedgerLot>,
    pub closes: Vec<PnlClose>,
}

type LotRow = (
    Uuid,
    Uuid,
    String,
    Uuid,
    Option<String>,
    Decimal,
    Decimal,
    Decimal,
    Decimal,
    DateTime<Utc>,
    Option<String>,
    Option<String>,
);

type CloseRow = (Uuid, Uuid, Uuid, Decimal, Decimal, Decimal, DateTime<Utc>);

/// Read one user's lots and closes. Every predicate carries `user_id`.
///
/// # Errors
/// A database error.
pub async fn load_ledger(
    pg: &sqlx::PgPool,
    user_id: Uuid,
    mode: AccountMode,
) -> Result<Ledger, sqlx::Error> {
    let lots: Vec<LotRow> = sqlx::query_as(
        "SELECT l.lot_id, l.user_id, l.instrument_id, l.open_event_id, l.side, \
                l.open_qty, l.remaining_qty, l.open_price, l.open_usd_rate, l.opened_at, \
                i.venue_id, i.asset_class \
           FROM pnl_lots l LEFT JOIN instruments i ON i.instrument_id = l.instrument_id \
          WHERE l.user_id = $1 AND l.account_mode = $2",
    )
    .bind(user_id)
    .bind(mode.as_str())
    .fetch_all(pg)
    .await?;
    let closes: Vec<CloseRow> = sqlx::query_as(
        "SELECT c.close_id, c.lot_id, c.close_event_id, c.close_qty, c.close_price, \
                c.realized_usd, c.closed_at \
           FROM pnl_closes c JOIN pnl_lots l ON l.lot_id = c.lot_id \
          WHERE l.user_id = $1 AND l.account_mode = $2",
    )
    .bind(user_id)
    .bind(mode.as_str())
    .fetch_all(pg)
    .await?;

    Ok(Ledger {
        lots: lots
            .into_iter()
            .map(
                |(
                    id,
                    uid,
                    instrument_id,
                    open_event_id,
                    side,
                    open_qty,
                    remaining,
                    price,
                    rate,
                    at,
                    venue,
                    ac,
                )| {
                    let parsed = match side.as_deref() {
                        Some("buy") => Some(Side::Buy),
                        Some("sell") => Some(Side::Sell),
                        _ => None,
                    };
                    LedgerLot {
                        lot: PnlLot {
                            id,
                            user_id: uid,
                            account_mode: mode,
                            instrument_id,
                            open_event_id,
                            side: parsed.unwrap_or(Side::Buy),
                            open_qty,
                            remaining_qty: remaining,
                            open_price: price,
                            open_usd_rate: rate,
                            opened_at: at,
                        },
                        side_known: parsed.is_some(),
                        venue,
                        asset_class: ac,
                    }
                },
            )
            .collect(),
        closes: closes
            .into_iter()
            .map(
                |(id, lot_id, close_event_id, close_qty, close_price, realized_usd, closed_at)| {
                    PnlClose {
                        id,
                        lot_id,
                        close_event_id,
                        close_qty,
                        close_price,
                        realized_usd,
                        closed_at,
                    }
                },
            )
            .collect(),
    })
}

/// The USD conversion rate for an instrument's quote currency, when the
/// rollup can name one without a rate feed: USD and the dollar stablecoins.
/// Anything else is `None` and is surfaced as unconverted, not valued at 1.
#[must_use]
pub fn usd_rate_for(instrument_id: &str) -> Option<Decimal> {
    let quote = instrument_id
        .rsplit(['-', '/', '_'])
        .next()
        .unwrap_or(instrument_id)
        .to_ascii_uppercase();
    // ADR-P6-07: the dollar peg is an assumption, so it is recorded in `Numeraire`, not spelled here.
    matches!(
        domain::numeraire::Numeraire::of(quote.as_str()),
        domain::numeraire::Numeraire::Usd | domain::numeraire::Numeraire::Stablecoin { .. }
    )
    .then_some(Decimal::ONE)
}

/// Value a ledger: marks for open lots, coverage for what could not be valued.
///
/// `mark_of` returns the latest mark and when it was observed. Pure, so the
/// whole valuation is testable without a database or an engine.
pub fn build_rollup(
    user_id: Uuid,
    mode: AccountMode,
    ledger: &Ledger,
    mark_of: &dyn Fn(&str) -> Option<(Decimal, DateTime<Utc>)>,
    now: DateTime<Utc>,
) -> RollupResponse {
    let mut coverage = MarkCoverage::default();
    let mut marks: HashMap<String, MarkPrice> = HashMap::new();
    let mut venue_map: HashMap<String, (String, String)> = HashMap::new();

    // Lots with an unreadable side never reach the valuation, in either
    // direction; their closes still count (realized P&L is stored, not derived).
    let mut lots: Vec<PnlLot> = Vec::with_capacity(ledger.lots.len());
    for l in &ledger.lots {
        if let (Some(v), Some(ac)) = (&l.venue, &l.asset_class) {
            venue_map.insert(l.lot.instrument_id.clone(), (v.clone(), ac.clone()));
        }
        if !l.side_known {
            if l.lot.remaining_qty > Decimal::ZERO {
                coverage.unreadable_lots += 1;
            }
            let mut closed_only = l.lot.clone();
            closed_only.remaining_qty = Decimal::ZERO;
            lots.push(closed_only);
            continue;
        }
        lots.push(l.lot.clone());
    }

    let open: std::collections::BTreeSet<&str> = lots
        .iter()
        .filter(|l| l.remaining_qty > Decimal::ZERO)
        .map(|l| l.instrument_id.as_str())
        .collect();
    for id in open {
        let Some(rate) = usd_rate_for(id) else {
            coverage.unconverted.push(id.to_owned());
            continue;
        };
        match mark_of(id) {
            None => {
                coverage.missing.push(id.to_owned());
            }
            Some((_, at)) if (now - at).num_seconds() > MARK_STALE_AFTER_SECS => {
                coverage.stale.push(id.to_owned());
            }
            Some((mark, _)) => {
                coverage.marked.push(id.to_owned());
                marks.insert(
                    id.to_owned(),
                    MarkPrice {
                        instrument_id: id.to_owned(),
                        mark,
                        usd_rate: rate,
                    },
                );
            }
        }
    }
    let marks: Vec<MarkPrice> = marks.into_values().collect();
    let mut rollup = compute_rollup(user_id, mode, &lots, &ledger.closes, &marks, &venue_map);
    rollup.mark_coverage = Some(coverage);
    rollup
}

/// Build a rollup response from raw slices (used in tests and the task queue).
pub fn rollup_from_slices(
    user_id: uuid::Uuid,
    mode: AccountMode,
    lots: &[storage::pnl::PnlLot],
    closes: &[storage::pnl::PnlClose],
    marks: &[MarkPrice],
    venue_map: &HashMap<String, (String, String)>,
) -> RollupResponse {
    compute_rollup(user_id, mode, lots, closes, marks, venue_map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn lot(user: Uuid, id: &str, side: Option<Side>, qty: Decimal, price: Decimal) -> LedgerLot {
        LedgerLot {
            lot: PnlLot {
                id: Uuid::new_v4(),
                user_id: user,
                account_mode: AccountMode::Live,
                instrument_id: id.to_owned(),
                open_event_id: Uuid::new_v4(),
                side: side.unwrap_or(Side::Buy),
                open_qty: qty,
                remaining_qty: qty,
                open_price: price,
                open_usd_rate: Decimal::ONE,
                opened_at: Utc::now(),
            },
            side_known: side.is_some(),
            venue: Some("coinbase".into()),
            asset_class: Some("crypto_spot_cex".into()),
        }
    }

    #[test]
    fn a_seeded_ledger_produces_a_real_non_zero_rollup() {
        let user = Uuid::new_v4();
        let mut long = lot(user, "BTC-USD", Some(Side::Buy), dec!(2), dec!(100));
        long.lot.remaining_qty = dec!(1);
        let close = PnlClose {
            id: Uuid::new_v4(),
            lot_id: long.lot.id,
            close_event_id: Uuid::new_v4(),
            close_qty: dec!(1),
            close_price: dec!(130),
            realized_usd: dec!(30),
            closed_at: Utc::now(),
        };
        let short = lot(user, "ETH-USD", Some(Side::Sell), dec!(1), dec!(50));
        let ledger = Ledger {
            lots: vec![long, short],
            closes: vec![close],
        };
        let now = Utc::now();
        let marks = |id: &str| match id {
            "BTC-USD" => Some((dec!(110), now)),
            "ETH-USD" => Some((dec!(40), now)),
            _ => None,
        };
        let r = build_rollup(user, AccountMode::Live, &ledger, &marks, now);
        assert_eq!(r.realized_pnl_usd, dec!(30));
        // Long +10, short +10 (mark fell from 50 to 40).
        assert_eq!(r.unrealized_pnl_usd, dec!(20));
        assert!(r.mark_coverage.unwrap().is_complete());
    }

    #[test]
    fn missing_stale_and_unconverted_marks_are_surfaced_not_zeroed() {
        let user = Uuid::new_v4();
        let ledger = Ledger {
            lots: vec![
                lot(user, "BTC-USD", Some(Side::Buy), dec!(1), dec!(100)),
                lot(user, "ETH-USD", Some(Side::Buy), dec!(1), dec!(100)),
                lot(user, "EUR-GBP", Some(Side::Buy), dec!(1), dec!(1)),
                lot(user, "SOL-USD", None, dec!(1), dec!(10)),
            ],
            closes: vec![],
        };
        let now = Utc::now();
        let marks = |id: &str| match id {
            "ETH-USD" => Some((dec!(200), now - chrono::Duration::hours(1))),
            "EUR-GBP" => Some((dec!(2), now)),
            _ => None,
        };
        let r = build_rollup(user, AccountMode::Live, &ledger, &marks, now);
        let c = r.mark_coverage.unwrap();
        assert_eq!(c.missing, vec!["BTC-USD".to_string()]);
        assert_eq!(c.stale, vec!["ETH-USD".to_string()]);
        assert_eq!(c.unconverted, vec!["EUR-GBP".to_string()]);
        assert_eq!(c.unreadable_lots, 1);
        assert!(!c.is_complete());
        assert_eq!(r.unrealized_pnl_usd, Decimal::ZERO);
    }

    #[test]
    fn another_users_lots_never_count() {
        let me = Uuid::new_v4();
        let ledger = Ledger {
            lots: vec![lot(
                Uuid::new_v4(),
                "BTC-USD",
                Some(Side::Buy),
                dec!(1),
                dec!(100),
            )],
            closes: vec![],
        };
        let now = Utc::now();
        let r = build_rollup(
            me,
            AccountMode::Live,
            &ledger,
            &|_| Some((dec!(200), now)),
            now,
        );
        assert_eq!(r.unrealized_pnl_usd, Decimal::ZERO);
    }

    #[test]
    fn the_rollup_route_resolves_its_caller() {
        let src = include_str!("dashboard.rs");
        let sig = src.split("pub async fn get_rollup(").nth(1).unwrap();
        let sig = &sig[..sig.find(") ->").unwrap()];
        assert!(sig.contains("token: BearerToken"), "{sig}");
        assert!(src.contains("load_ledger(&state.pg, token.user_id()"));
    }
}
