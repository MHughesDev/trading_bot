//! Postgres queries for the instruments table.
//!
//! `equity_seed_instruments()` returns the canonical in-memory representations
//! of the equity instruments added in Phase 6 — usable in tests and seeding
//! scripts without a live database connection.

use domain::{
    instrument::{AssetClass, HaltPolicy, TradingSchedule, TradingSession},
    trust::TrustTier,
    Instrument,
};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use std::str::FromStr;

use super::PgError;

struct InstrumentRow {
    id: String,
    venue_id: String,
    asset_class: String,
    tick_size: String,
    lot_size: String,
    base_precision: i32,
    quote_precision: i32,
    is_active: bool,
    trust_tier: Option<String>,
    halt_policy: Option<String>,
    watermark_secs: Option<i64>,
}

fn parse_asset_class(s: &str) -> AssetClass {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .unwrap_or(AssetClass::CryptoSpotCex)
}

fn parse_halt_policy(s: Option<&str>) -> HaltPolicy {
    match s {
        Some(v) => serde_json::from_value(serde_json::Value::String(v.to_owned()))
            .unwrap_or(HaltPolicy::Haltable),
        None => HaltPolicy::Haltable,
    }
}

fn parse_trust_tier(s: Option<&str>) -> TrustTier {
    match s {
        Some(v) => serde_json::from_value(serde_json::Value::String(v.to_owned()))
            .unwrap_or(TrustTier::CentralizedExchange),
        None => TrustTier::CentralizedExchange,
    }
}

fn row_to_instrument(row: InstrumentRow) -> Instrument {
    Instrument {
        instrument_id: row.id,
        venue_id: row.venue_id,
        asset_class: parse_asset_class(&row.asset_class),
        tick_size: Decimal::from_str(&row.tick_size).unwrap_or_default(),
        lot_size: Decimal::from_str(&row.lot_size).unwrap_or_default(),
        base_precision: u32::try_from(row.base_precision.max(0)).unwrap_or(0),
        quote_precision: u32::try_from(row.quote_precision.max(0)).unwrap_or(0),
        active: row.is_active,
        trading_hours: TradingSchedule::always_open(),
        halt_behavior: parse_halt_policy(row.halt_policy.as_deref()),
        trust_tier: parse_trust_tier(row.trust_tier.as_deref()),
        watermark_secs: row.watermark_secs.unwrap_or(2),
    }
}

const SELECT_COLUMNS: &str = "id, venue_id, asset_class, \
    tick_size::TEXT, lot_size::TEXT, \
    COALESCE(base_precision, 8)  AS base_precision, \
    COALESCE(quote_precision, 2) AS quote_precision, \
    is_active, trust_tier, halt_policy, watermark_secs";

fn map_row(r: sqlx::postgres::PgRow) -> InstrumentRow {
    InstrumentRow {
        id: r.get("id"),
        venue_id: r.get("venue_id"),
        asset_class: r.get("asset_class"),
        tick_size: r.get("tick_size"),
        lot_size: r.get("lot_size"),
        base_precision: r.get("base_precision"),
        quote_precision: r.get("quote_precision"),
        is_active: r.get("is_active"),
        trust_tier: r.get("trust_tier"),
        halt_policy: r.get("halt_policy"),
        watermark_secs: r.get("watermark_secs"),
    }
}

/// Fetch one instrument by its string id (e.g. `"BTC-USDT"`).
pub async fn fetch_by_id(
    pool: &PgPool,
    instrument_id: &str,
) -> Result<Option<Instrument>, PgError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM instruments WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(instrument_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| row_to_instrument(map_row(r))))
}

/// List all active instruments.
pub async fn list_active(pool: &PgPool) -> Result<Vec<Instrument>, PgError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM instruments WHERE is_active = true");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| row_to_instrument(map_row(r)))
        .collect())
}

// ── Equity seed data (Phase 6) ────────────────────────────────────────────────

/// The NYSE/NASDAQ regular session: 09:30–16:00 Eastern time.
fn nyse_session() -> TradingSchedule {
    TradingSchedule {
        timezone: "America/New_York".into(),
        sessions: vec![TradingSession {
            open: "09:30".into(),
            close: "16:00".into(),
        }],
        has_pre_market: true,
        has_post_market: true,
    }
}

/// Canonical equity instrument definitions for Phase 6.
///
/// These match the SQL rows that would be inserted by the Phase 6 migration:
/// ```sql
/// INSERT INTO instruments (id, venue_id, asset_class, tick_size, lot_size,
///   base_precision, quote_precision, is_active, trust_tier, halt_policy)
/// VALUES
///   ('AAPL', 'alpaca', 'equity', 0.01, 1, 2, 2, true, 'regulated', 'haltable'),
///   ('SPY',  'alpaca', 'equity', 0.01, 1, 2, 2, true, 'regulated', 'haltable');
/// ```
pub fn equity_seed_instruments() -> Vec<Instrument> {
    let tick = Decimal::from_str("0.01").expect("const");
    let lot = Decimal::ONE;
    vec![
        Instrument {
            instrument_id: "AAPL".into(),
            asset_class: AssetClass::Equity,
            venue_id: "alpaca".into(),
            base_precision: 2,
            quote_precision: 2,
            tick_size: tick,
            lot_size: lot,
            trading_hours: nyse_session(),
            halt_behavior: HaltPolicy::Haltable,
            trust_tier: TrustTier::Regulated,
            active: true,
            watermark_secs: 2,
        },
        Instrument {
            instrument_id: "SPY".into(),
            asset_class: AssetClass::Equity,
            venue_id: "alpaca".into(),
            base_precision: 2,
            quote_precision: 2,
            tick_size: tick,
            lot_size: lot,
            trading_hours: nyse_session(),
            halt_behavior: HaltPolicy::Haltable,
            trust_tier: TrustTier::Regulated,
            active: true,
            watermark_secs: 2,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equity_seeds_have_correct_metadata() {
        let seeds = equity_seed_instruments();
        assert_eq!(seeds.len(), 2);

        for inst in &seeds {
            assert_eq!(inst.asset_class, AssetClass::Equity);
            assert_eq!(inst.venue_id, "alpaca");
            assert_eq!(inst.trust_tier, TrustTier::Regulated);
            assert_eq!(inst.halt_behavior, HaltPolicy::Haltable);
            assert!(!inst.trading_hours.is_24_7(), "equity must not be 24/7");
            assert!(inst.active);
        }
    }

    #[test]
    fn crypto_and_equity_coexist() {
        // Crypto: 24/7, non-haltable, CentralizedExchange
        let crypto = Instrument {
            instrument_id: "BTC-USDT".into(),
            asset_class: AssetClass::CryptoSpotCex,
            venue_id: "kraken".into(),
            base_precision: 8,
            quote_precision: 2,
            tick_size: Decimal::from_str("0.01").unwrap(),
            lot_size: Decimal::from_str("0.00001").unwrap(),
            trading_hours: TradingSchedule::always_open(),
            halt_behavior: HaltPolicy::NonHaltable,
            trust_tier: TrustTier::CentralizedExchange,
            active: true,
            watermark_secs: 2,
        };

        let equity = &equity_seed_instruments()[0]; // AAPL

        // Different schedules — same metadata model, no asset_class branches needed.
        assert!(crypto.trading_hours.is_24_7());
        assert!(!equity.trading_hours.is_24_7());
        assert_eq!(crypto.halt_behavior, HaltPolicy::NonHaltable);
        assert_eq!(equity.halt_behavior, HaltPolicy::Haltable);
        assert_eq!(crypto.trust_tier, TrustTier::CentralizedExchange);
        assert_eq!(equity.trust_tier, TrustTier::Regulated);
    }
}
