//! Instrument trading rules — the metadata the risk gate cannot judge an order
//! without, and where it comes from.
//!
//! The gate's lot and tick checks need `Instrument.tick_size` / `lot_size`
//! (DATA-002).  Both order doors used to pass `Decimal::ZERO` for these, with a
//! comment explaining that a zero disables the check — which is true, and is
//! the problem: a disabled check and a passed check look identical afterwards.
//! `orders.rs` also noted that passing a nonzero lot "would wrongly reject
//! fractional crypto like 0.01 BTC", which is only true when the lot is
//! *guessed*.  The venue publishes the real one.
//!
//! So: the increments are **fetched from the venue** when an asset is
//! initialised and written to `instruments`.  If they cannot be fetched, no row
//! is written, and the gate refuses orders on that instrument naming the
//! missing metadata rather than waving them through.

use std::collections::HashMap;
use std::sync::RwLock;

use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::{info, warn};

use domain::instrument::{HaltPolicy, TradingSchedule, TradingSession};
use domain::TrustTier;

/// What the gate needs to know about an instrument to judge an order.
#[derive(Clone, Debug)]
pub struct InstrumentMeta {
    pub instrument_id: String,
    pub tick_size: Decimal,
    pub lot_size: Decimal,
    pub trading_hours: TradingSchedule,
    pub halt_policy: HaltPolicy,
    /// The venue's trust tier for this instrument's market data.
    pub trust_tier: TrustTier,
    pub active: bool,
}

/// In-memory snapshot of `instruments`, refreshed out of band.
///
/// The risk-context provider is synchronous — it is called from the hot path's
/// stage 4, which must not block on a database round trip per order — so the
/// rows are cached here and reloaded periodically and on demand.
#[derive(Debug, Default)]
pub struct InstrumentMetaCache {
    rows: RwLock<HashMap<String, InstrumentMeta>>,
}

impl InstrumentMetaCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Metadata for one instrument, or `None` when it is not known.
    ///
    /// `None` is a refusal upstream, never a default.
    #[must_use]
    pub fn get(&self, instrument_id: &str) -> Option<InstrumentMeta> {
        self.rows.read().ok()?.get(instrument_id).cloned()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.read().map(|r| r.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Replace the cache from the `instruments` table.
    pub async fn reload(&self, pg: &PgPool) -> Result<usize, sqlx::Error> {
        let rows: Vec<(String, Decimal, Decimal, serde_json::Value, String, String, bool)> =
            sqlx::query_as(
                "SELECT instrument_id, tick_size, lot_size, trading_hours_json, halt_policy, \
                        trust_tier, active \
                 FROM instruments",
            )
            .fetch_all(pg)
            .await?;

        let mut map = HashMap::with_capacity(rows.len());
        for (instrument_id, tick_size, lot_size, hours, halt, trust, active) in rows {
            let trading_hours =
                serde_json::from_value(hours).unwrap_or_else(|_| TradingSchedule::always_open());
            map.insert(
                instrument_id.clone(),
                InstrumentMeta {
                    instrument_id,
                    tick_size,
                    lot_size,
                    trading_hours,
                    halt_policy: if halt == "haltable" {
                        HaltPolicy::Haltable
                    } else {
                        HaltPolicy::NonHaltable
                    },
                    trust_tier: parse_trust_tier(&trust),
                    active,
                },
            );
        }
        let n = map.len();
        if let Ok(mut w) = self.rows.write() {
            *w = map;
        }
        Ok(n)
    }

    /// Insert one row directly.  Used by the seeding path and by tests.
    pub fn put(&self, meta: InstrumentMeta) {
        if let Ok(mut w) = self.rows.write() {
            w.insert(meta.instrument_id.clone(), meta);
        }
    }
}

fn parse_trust_tier(s: &str) -> TrustTier {
    match s {
        "regulated" => TrustTier::Regulated,
        "centralized_exchange" => TrustTier::CentralizedExchange,
        "onchain_confirmed" => TrustTier::OnchainConfirmed,
        "onchain_tentative" => TrustTier::OnchainTentative,
        "social_derived" => TrustTier::SocialDerived,
        // An unrecognised tier is treated as the **lowest** trust, not the
        // highest: a strategy declaring a minimum tier must not clear it on the
        // strength of a value we could not read.
        _ => TrustTier::SocialDerived,
    }
}

/// Trading rules as published by the venue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueTradingRules {
    pub tick_size: Decimal,
    pub lot_size: Decimal,
    pub base_precision: i32,
    pub quote_precision: i32,
}

/// Ensure `instruments` has a row for `symbol`, fetching the venue's published
/// increments if it does not.
///
/// Returns `Ok(true)` when a row now exists.  `Ok(false)` means the venue could
/// not be asked and no row was written — orders on this instrument will be
/// refused with `InstrumentMetadataUnavailable`, which is the intended
/// behaviour: an order sized against a guessed lot is an order the venue
/// rejects, or worse, accepts at a size nobody chose.
pub async fn ensure_instrument_metadata(
    pg: &PgPool,
    symbol: &str,
    asset_class: &str,
    venue_id: &str,
) -> Result<bool, sqlx::Error> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT instrument_id FROM instruments WHERE instrument_id = $1")
            .bind(symbol)
            .fetch_optional(pg)
            .await?;
    if existing.is_some() {
        return Ok(true);
    }

    let rules = match asset_class {
        "crypto_spot_cex" => fetch_kraken_rules(symbol).await,
        _ => None,
    };

    let Some(rules) = rules else {
        warn!(
            symbol,
            asset_class,
            venue_id,
            "no published trading rules for this instrument — no `instruments` row written. \
             Orders on it will be refused (InstrumentMetadataUnavailable) until one exists; \
             the gate does not guess a tick or a lot."
        );
        return Ok(false);
    };

    let (hours, halt, trust) = match asset_class {
        "equity" | "etf" => (nyse_schedule(), "haltable", "regulated"),
        _ => (
            TradingSchedule::always_open(),
            "non_haltable",
            "centralized_exchange",
        ),
    };

    sqlx::query(
        "INSERT INTO instruments \
           (instrument_id, asset_class, venue_id, base_precision, quote_precision, \
            tick_size, lot_size, trading_hours_json, halt_policy, trust_tier, active) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, true) \
         ON CONFLICT (instrument_id) DO NOTHING",
    )
    .bind(symbol)
    .bind(asset_class)
    .bind(venue_id)
    .bind(rules.base_precision)
    .bind(rules.quote_precision)
    .bind(rules.tick_size)
    .bind(rules.lot_size)
    .bind(serde_json::to_value(&hours).unwrap_or(serde_json::Value::Null))
    .bind(halt)
    .bind(trust)
    .execute(pg)
    .await?;

    info!(
        symbol,
        tick_size = %rules.tick_size,
        lot_size = %rules.lot_size,
        "instrument trading rules recorded from the venue"
    );
    Ok(true)
}

fn nyse_schedule() -> TradingSchedule {
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

/// Kraken's published increments for one pair.
///
/// `pair_decimals` is the price precision and `lot_decimals` the quantity
/// precision, so the increments are `10^-decimals`.  Both come from the venue;
/// neither is assumed.
async fn fetch_kraken_rules(instrument_id: &str) -> Option<VenueTradingRules> {
    let candidates = kraken_pair_candidates(instrument_id);
    let client = reqwest::Client::new();
    for candidate in &candidates {
        let url = format!("https://api.kraken.com/0/public/AssetPairs?pair={candidate}");
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        let Ok(body) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        let Some(result) = body.get("result").and_then(|r| r.as_object()) else {
            continue;
        };
        let Some((_, pair)) = result.iter().next() else {
            continue;
        };
        let price_dp = pair.get("pair_decimals")?.as_i64()? as u32;
        let lot_dp = pair.get("lot_decimals")?.as_i64()? as u32;
        return Some(VenueTradingRules {
            tick_size: pow10_negative(price_dp),
            lot_size: pow10_negative(lot_dp),
            base_precision: lot_dp as i32,
            quote_precision: price_dp as i32,
        });
    }
    None
}

/// `BTC-USD` → the spellings Kraken might answer to.
fn kraken_pair_candidates(instrument_id: &str) -> Vec<String> {
    let flat = instrument_id.replace('-', "").to_uppercase();
    let mut out = vec![flat.clone()];
    // Kraken's legacy asset code for bitcoin.
    if flat.starts_with("BTC") {
        out.push(flat.replacen("BTC", "XBT", 1));
    }
    out
}

/// `10^-n` as a `Decimal`.  Never an f64 — this is a price/size increment (SC-1).
fn pow10_negative(n: u32) -> Decimal {
    if n == 0 {
        return Decimal::ONE;
    }
    Decimal::new(1, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increments_are_exact_decimals() {
        assert_eq!(pow10_negative(0), Decimal::ONE);
        assert_eq!(pow10_negative(1).to_string(), "0.1");
        assert_eq!(pow10_negative(8).to_string(), "0.00000001");
    }

    #[test]
    fn bitcoin_is_also_tried_as_xbt() {
        let c = kraken_pair_candidates("BTC-USD");
        assert_eq!(c, vec!["BTCUSD".to_string(), "XBTUSD".to_string()]);
        assert_eq!(kraken_pair_candidates("ETH-USD"), vec!["ETHUSD"]);
    }

    #[test]
    fn an_unreadable_trust_tier_is_the_lowest_not_the_highest() {
        // A tier we cannot parse must not let a strategy's min_trust_tier pass.
        assert_eq!(parse_trust_tier("something_new"), TrustTier::SocialDerived);
        assert_eq!(parse_trust_tier("regulated"), TrustTier::Regulated);
    }

    #[test]
    fn an_unknown_instrument_is_absent_not_defaulted() {
        let cache = InstrumentMetaCache::new();
        assert!(cache.get("BTC-USD").is_none());
    }
}
