//! Postgres queries for the instruments table.

use domain::{
    instrument::{AssetClass, HaltPolicy, TradingSchedule},
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
