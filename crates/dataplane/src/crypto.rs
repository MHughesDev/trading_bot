//! Crypto specifics (SPEC §1.7): venue quality, funding as a bitemporal series,
//! listing membership, the versioned consolidated-price rule, and venue seasonal
//! deflation.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::identity::{InstrumentKey, VenueKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingKind {
    /// Tradeable information at its knowledge_time.
    Predicted,
    /// Not tradeable until it is realized and published.
    Realized,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FundingObservation {
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    pub kind: FundingKind,
    /// The funding interval the rate applies to (settlement time).
    pub settlement_time: DateTime<Utc>,
    pub rate: Decimal,
    pub knowledge_time: DateTime<Utc>,
}

/// Latest funding rate of `kind` for a settlement, as known by `as_of`.
#[must_use]
pub fn funding_as_of(
    obs: &[FundingObservation],
    instrument: InstrumentKey,
    venue: VenueKey,
    kind: FundingKind,
    settlement: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> Option<Decimal> {
    obs.iter()
        .filter(|o| o.instrument_id == instrument && o.venue_id == venue && o.kind == kind && o.settlement_time == settlement && o.knowledge_time <= as_of)
        .max_by_key(|o| o.knowledge_time)
        .map(|o| o.rate)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListingMembership {
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    pub listed_from: DateTime<Utc>,
    pub delisted_at: Option<DateTime<Utc>>,
    pub knowledge_time: DateTime<Utc>,
}

/// The tradable universe on a venue at `at`, as known by `as_of`. Delisted coins are
/// kept in the history — excluding them is survivorship bias.
#[must_use]
pub fn listed_at(rows: &[ListingMembership], venue: VenueKey, at: DateTime<Utc>, as_of: DateTime<Utc>) -> Vec<InstrumentKey> {
    let mut v: Vec<InstrumentKey> = rows
        .iter()
        .filter(|r| r.venue_id == venue && r.knowledge_time <= as_of && r.listed_from <= at && r.delisted_at.is_none_or(|d| at < d))
        .map(|r| r.instrument_id)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// One venue's latest print for the consolidated-price rule.
#[derive(Clone, Debug, PartialEq)]
pub struct VenueQuote {
    pub venue_id: VenueKey,
    pub quality_tier: u8,
    pub price: Decimal,
    pub volume: Decimal,
    pub knowledge_time: DateTime<Utc>,
}

/// The versioned construction rule for a consolidated cross-venue price. This is an
/// L1 feature, never an L0 fact: storing it would fabricate a price nobody traded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationRule {
    pub version: String,
    pub max_quality_tier: u8,
    pub max_staleness_secs: i64,
}

impl Default for ConsolidationRule {
    fn default() -> Self {
        Self {
            version: "vwap_tier1_v1".into(),
            max_quality_tier: 1,
            max_staleness_secs: 120,
        }
    }
}

/// Volume-weighted price across eligible venues known and fresh at `as_of`.
#[must_use]
pub fn consolidated_price(quotes: &[VenueQuote], rule: &ConsolidationRule, as_of: DateTime<Utc>) -> Option<Decimal> {
    let fresh = |q: &&VenueQuote| {
        q.quality_tier <= rule.max_quality_tier
            && q.knowledge_time <= as_of
            && as_of - q.knowledge_time <= Duration::seconds(rule.max_staleness_secs)
            && q.volume > Decimal::ZERO
    };
    let (num, den) = quotes
        .iter()
        .filter(fresh)
        .fold((Decimal::ZERO, Decimal::ZERO), |(n, d), q| (n + q.price * q.volume, d + q.volume));
    (den > Decimal::ZERO).then(|| num / den)
}

/// Venue-specific intraday seasonal volatility profile keyed by
/// (day-of-week, hour, minute-of-hour bucket). Crypto seasonality is not U-shaped:
/// weekends are quieter, ~16:00 UTC peaks, and funding settlement spikes the
/// :00/:15/:30/:45 minutes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeasonalProfile {
    pub venue_id: VenueKey,
    pub version: String,
    /// Mean absolute return per slot, normalized to mean 1.
    pub factors: HashMap<u32, f64>,
    pub knowledge_time: DateTime<Utc>,
}

#[must_use]
pub fn season_slot(t: DateTime<Utc>) -> u32 {
    let quarter = t.minute() / 15;
    let spike = u32::from(t.minute() % 15 == 0);
    t.weekday().num_days_from_monday() * 24 * 8 + t.hour() * 8 + quarter * 2 + spike
}

impl SeasonalProfile {
    /// Fit from `(time, |return|)` observations known by `as_of`. Returns `None` if
    /// there is too little data to populate the profile.
    #[must_use]
    pub fn fit(venue_id: VenueKey, version: &str, obs: &[(DateTime<Utc>, f64)], as_of: DateTime<Utc>) -> Option<Self> {
        let mut sums: HashMap<u32, (f64, u32)> = HashMap::new();
        for (t, abs_ret) in obs.iter().filter(|(t, r)| *t <= as_of && r.is_finite()) {
            let e = sums.entry(season_slot(*t)).or_default();
            e.0 += abs_ret.abs();
            e.1 += 1;
        }
        if sums.is_empty() {
            return None;
        }
        let means: HashMap<u32, f64> = sums.into_iter().map(|(k, (s, n))| (k, s / f64::from(n))).collect();
        let grand = means.values().sum::<f64>() / means.len() as f64;
        if grand <= 0.0 {
            return None;
        }
        Some(Self {
            venue_id,
            version: version.into(),
            factors: means.into_iter().map(|(k, m)| (k, m / grand)).collect(),
            knowledge_time: as_of,
        })
    }

    /// Divide a return by its slot factor. Must be applied before any volatility
    /// feature, or clustering will discover funding schedules.
    #[must_use]
    pub fn deflate(&self, t: DateTime<Utc>, ret: f64) -> f64 {
        let f = self.factors.get(&season_slot(t)).copied().unwrap_or(1.0);
        if f > 0.0 { ret / f } else { ret }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn t(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 5, h, m, 0).unwrap()
    }

    #[test]
    fn predicted_funding_is_visible_before_realized() {
        let s = t(16, 0);
        let obs = vec![
            FundingObservation { instrument_id: InstrumentKey(1), venue_id: VenueKey(1), kind: FundingKind::Predicted, settlement_time: s, rate: dec!(0.0001), knowledge_time: t(8, 0) },
            FundingObservation { instrument_id: InstrumentKey(1), venue_id: VenueKey(1), kind: FundingKind::Realized, settlement_time: s, rate: dec!(0.00012), knowledge_time: t(16, 1) },
        ];
        assert_eq!(funding_as_of(&obs, InstrumentKey(1), VenueKey(1), FundingKind::Predicted, s, t(12, 0)), Some(dec!(0.0001)));
        assert_eq!(funding_as_of(&obs, InstrumentKey(1), VenueKey(1), FundingKind::Realized, s, t(12, 0)), None);
        assert_eq!(funding_as_of(&obs, InstrumentKey(1), VenueKey(1), FundingKind::Realized, s, t(17, 0)), Some(dec!(0.00012)));
    }

    #[test]
    fn delisted_coins_stay_in_history() {
        let rows = vec![ListingMembership { instrument_id: InstrumentKey(3), venue_id: VenueKey(1), listed_from: t(0, 0), delisted_at: Some(t(10, 0)), knowledge_time: t(0, 0) }];
        assert_eq!(listed_at(&rows, VenueKey(1), t(5, 0), t(23, 0)), vec![InstrumentKey(3)]);
        assert!(listed_at(&rows, VenueKey(1), t(11, 0), t(23, 0)).is_empty());
    }

    #[test]
    fn consolidated_price_excludes_stale_and_low_tier() {
        let as_of = t(12, 0);
        let q = vec![
            VenueQuote { venue_id: VenueKey(1), quality_tier: 1, price: dec!(100), volume: dec!(1), knowledge_time: as_of - Duration::seconds(10) },
            VenueQuote { venue_id: VenueKey(2), quality_tier: 1, price: dec!(110), volume: dec!(3), knowledge_time: as_of - Duration::seconds(30) },
            VenueQuote { venue_id: VenueKey(3), quality_tier: 1, price: dec!(999), volume: dec!(9), knowledge_time: as_of - Duration::seconds(600) },
            VenueQuote { venue_id: VenueKey(4), quality_tier: 3, price: dec!(1), volume: dec!(50), knowledge_time: as_of },
        ];
        assert_eq!(consolidated_price(&q, &ConsolidationRule::default(), as_of), Some(dec!(107.5)));
    }

    #[test]
    fn seasonal_deflation_flattens_the_funding_spike() {
        let mut obs = Vec::new();
        for day in 0..7 {
            for h in 0..24 {
                for m in 0..60 {
                    let tt = Utc.with_ymd_and_hms(2026, 1, 5 + day, h, m, 0).unwrap();
                    let r = if m % 15 == 0 { 0.004 } else { 0.001 };
                    obs.push((tt, r));
                }
            }
        }
        let p = SeasonalProfile::fit(VenueKey(1), "v1", &obs, t(23, 59) + Duration::days(7)).unwrap();
        let spike = p.deflate(t(16, 15), 0.004);
        let calm = p.deflate(t(16, 7), 0.001);
        assert!((spike - calm).abs() < 1e-12, "deflated returns should be comparable: {spike} vs {calm}");
    }
}
