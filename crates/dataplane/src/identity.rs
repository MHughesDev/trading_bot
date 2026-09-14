//! Surrogate instrument identity; symbols are bitemporal attributes (SPEC §1.1, INV-04).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Opaque, never-reused instrument identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstrumentKey(pub i64);

/// Venue is a stored coordinate, not metadata (SPEC §1.1, R-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VenueKey(pub i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    Equity,
    Etf,
    Future,
    Option,
    Crypto,
    Pool,
    Fx,
    PredictionMarket,
}

impl AssetClass {
    /// Map this codebase's `domain::AssetClass` snake-case names onto the spec's
    /// storage classes.
    #[must_use]
    pub fn from_platform(name: &str) -> Option<Self> {
        Some(match name {
            "equity" => Self::Equity,
            "etf" => Self::Etf,
            "futures_expiring" | "future" => Self::Future,
            "option" => Self::Option,
            "crypto_spot_cex" | "perpetual_swap" | "crypto" => Self::Crypto,
            "pool" | "defi_pool" | "dex" => Self::Pool,
            "fx" => Self::Fx,
            "prediction_market" => Self::PredictionMarket,
            _ => return None,
        })
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equity => "equity",
            Self::Etf => "etf",
            Self::Future => "future",
            Self::Option => "option",
            Self::Crypto => "crypto",
            Self::Pool => "pool",
            Self::Fx => "fx",
            Self::PredictionMarket => "prediction_market",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionModel {
    Continuous247,
    RthPlusExt,
    ChainBlock,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Venue {
    pub venue_id: VenueKey,
    pub name: String,
    pub chain_id: Option<i32>,
    pub session_model: SessionModel,
    /// Pinned calendar version; enters every dataset hash that touches the venue.
    pub calendar_id: String,
    /// 1 = primary, 2 = usable, 3 = reference-only.
    pub quality_tier: u8,
}

/// One bitemporal fact: "on `venue`, `symbol` denoted `instrument` over
/// `[valid_from, valid_to)`, as known from `knowledge_time`".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolFact {
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    pub symbol: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub knowledge_time: DateTime<Utc>,
}

impl SymbolFact {
    fn valid_at(&self, t: DateTime<Utc>) -> bool {
        self.valid_from <= t && self.valid_to.is_none_or(|end| t < end)
    }
}

/// Resolve `(venue, symbol)` at event time `at`, using only facts known by `as_of`.
///
/// A recycled ticker resolves to different instruments at different event times;
/// a later correction of a symbol's validity only takes effect for readers whose
/// `as_of` is past that correction's `knowledge_time`.
#[must_use]
pub fn resolve_symbol(
    facts: &[SymbolFact],
    venue: VenueKey,
    symbol: &str,
    at: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> Option<InstrumentKey> {
    current_versions(facts, as_of)
        .into_iter()
        .filter(|f| f.venue_id == venue && f.symbol == symbol && f.valid_at(at))
        .max_by_key(|f| (f.valid_from, f.knowledge_time))
        .map(|f| f.instrument_id)
}

/// For each fact identity `(instrument, venue, symbol, valid_from)`, the version
/// with the greatest `knowledge_time ≤ as_of`. A correction supersedes the
/// version it restates rather than coexisting with it.
fn current_versions(facts: &[SymbolFact], as_of: DateTime<Utc>) -> Vec<&SymbolFact> {
    let mut latest: std::collections::HashMap<(InstrumentKey, VenueKey, &str, DateTime<Utc>), &SymbolFact> =
        std::collections::HashMap::new();
    for f in facts.iter().filter(|f| f.knowledge_time <= as_of) {
        let key = (f.instrument_id, f.venue_id, f.symbol.as_str(), f.valid_from);
        latest
            .entry(key)
            .and_modify(|cur| {
                if f.knowledge_time > cur.knowledge_time {
                    *cur = f;
                }
            })
            .or_insert(f);
    }
    latest.into_values().collect()
}

/// Symbols an instrument carried on a venue, as known by `as_of`.
#[must_use]
pub fn symbols_of(
    facts: &[SymbolFact],
    instrument: InstrumentKey,
    as_of: DateTime<Utc>,
) -> Vec<&SymbolFact> {
    let mut out: Vec<&SymbolFact> = current_versions(facts, as_of)
        .into_iter()
        .filter(|f| f.instrument_id == instrument)
        .collect();
    out.sort_by_key(|f| (f.valid_from, f.knowledge_time));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    fn fact(inst: i64, sym: &str, from: DateTime<Utc>, to: Option<DateTime<Utc>>, k: DateTime<Utc>) -> SymbolFact {
        SymbolFact {
            instrument_id: InstrumentKey(inst),
            venue_id: VenueKey(1),
            symbol: sym.into(),
            valid_from: from,
            valid_to: to,
            knowledge_time: k,
        }
    }

    #[test]
    fn recycled_ticker_resolves_by_event_time() {
        let facts = vec![
            fact(1, "ABC", t(2010, 1, 1), Some(t(2015, 1, 1)), t(2010, 1, 1)),
            fact(2, "ABC", t(2018, 1, 1), None, t(2018, 1, 1)),
        ];
        let now = t(2026, 1, 1);
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "ABC", t(2012, 6, 1), now), Some(InstrumentKey(1)));
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "ABC", t(2020, 6, 1), now), Some(InstrumentKey(2)));
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "ABC", t(2016, 6, 1), now), None);
    }

    #[test]
    fn a_fact_is_invisible_before_its_knowledge_time() {
        let facts = vec![fact(2, "ABC", t(2018, 1, 1), None, t(2018, 3, 1))];
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "ABC", t(2018, 2, 1), t(2018, 2, 1)), None);
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "ABC", t(2018, 2, 1), t(2018, 4, 1)), Some(InstrumentKey(2)));
    }

    #[test]
    fn later_correction_wins_only_after_it_is_known() {
        // Originally believed valid forever; a later correction ends it in 2019.
        let facts = vec![
            fact(1, "XYZ", t(2010, 1, 1), None, t(2010, 1, 1)),
            fact(1, "XYZ", t(2010, 1, 1), Some(t(2019, 1, 1)), t(2020, 1, 1)),
        ];
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "XYZ", t(2019, 6, 1), t(2019, 12, 1)), Some(InstrumentKey(1)));
        assert_eq!(resolve_symbol(&facts, VenueKey(1), "XYZ", t(2019, 6, 1), t(2021, 1, 1)), None);
    }

    #[test]
    fn platform_asset_class_mapping() {
        assert_eq!(AssetClass::from_platform("crypto_spot_cex"), Some(AssetClass::Crypto));
        assert_eq!(AssetClass::from_platform("futures_expiring"), Some(AssetClass::Future));
        assert_eq!(AssetClass::from_platform("nft"), None);
    }
}
