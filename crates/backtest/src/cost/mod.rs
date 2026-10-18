//! The dated per-instrument cost model (Set M M-2.1, M-2.2, M-3.4 refusal, M-3.5; FEAT-005 §5).
//!
//! `cost:<instrument>@<date>` names one of these. It is the thing a run's `cost_model_ref`
//! resolves to, and its design rule is the platform's usual one: **a leg that is not there
//! refuses the build rather than being assumed zero.** A backtest that quietly charges no
//! spread or no impact is the one whose numbers look best, so the builder has no defaults:
//!
//! * fee: from the `FeeBook` (ADR-P6), cited by source;
//! * spread: `edge` (measured from bars, citing the instrument-profile row and the window it
//!   was measured over), `quoted` (needs quote history, which refuses when there is none and
//!   never falls back to EDGE) or `fixed`;
//! * impact: square-root, with `Y`, and the sigma and ADV it was fed, each citing where it was
//!   measured;
//! * borrow, stated explicitly (zero is a statement, not an omission);
//! * latency in bars: resolved from the venue class ([`latency_for`]); an unclassed venue refuses.
//!
//! A model dated after the start of the window it prices is refused ([`CostModel::check_point_in_time`]):
//! it would be today's liquidity applied to last year's trades.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::position_series::{Costs, SqrtImpact};

/// `cost:<instrument>@<yyyy-mm-dd>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostModelRef {
    pub instrument: String,
    pub dated: NaiveDate,
}

impl CostModelRef {
    #[must_use]
    pub fn format(&self) -> String {
        format!("cost:{}@{}", self.instrument, self.dated)
    }

    /// # Errors
    /// Anything that is not `cost:<instrument>@<date>`.
    pub fn parse(s: &str) -> Result<Self, CostModelError> {
        let bad = || CostModelError::BadRef(s.to_string());
        let rest = s.strip_prefix("cost:").ok_or_else(bad)?;
        let (instrument, date) = rest.rsplit_once('@').ok_or_else(bad)?;
        if instrument.is_empty() {
            return Err(bad());
        }
        Ok(Self { instrument: instrument.to_string(), dated: date.parse().map_err(|_| bad())? })
    }
}

/// The class of venue an instrument trades on, which decides its default latency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueClass {
    CryptoSpot,
    CryptoPerp,
    EquityBroker,
    PredictionMarket,
    Synthetic,
}

/// Decision-to-order latency, in bars **beyond** the platform's decide-at-close,
/// fill-at-next-open structure (FEAT-005 §11 question 1; ADR-M-09).
///
/// 0 for every liquid class: at one-minute resolution or coarser, API latency of hundreds of
/// milliseconds is far inside the bar the order already waits for. 1 for prediction markets,
/// whose books are thin enough that an order resting a full bar is stale.
#[must_use]
pub const fn latency_for(class: VenueClass) -> u32 {
    match class {
        VenueClass::CryptoSpot | VenueClass::CryptoPerp | VenueClass::EquityBroker | VenueClass::Synthetic => 0,
        VenueClass::PredictionMarket => 1,
    }
}

/// The class of a venue key, or an error: an unclassed venue gets no guessed latency.
///
/// # Errors
/// A venue this table does not know.
pub fn venue_class(venue: &str) -> Result<VenueClass, CostModelError> {
    Ok(match venue {
        "coinbase" | "kraken" | "binance" | "bybit" => VenueClass::CryptoSpot,
        "kraken_futures" | "binance_futures" | "deribit" => VenueClass::CryptoPerp,
        "alpaca" | "ibkr" => VenueClass::EquityBroker,
        "kalshi" | "polymarket" => VenueClass::PredictionMarket,
        "synthetic" => VenueClass::Synthetic,
        other => return Err(CostModelError::UnclassedVenue(other.to_string())),
    })
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CostModelError {
    #[error("the cost model has no `{0}` leg; a missing leg is refused, never assumed zero")]
    MissingLeg(&'static str),
    #[error("`{0}` is not cost:<instrument>@<yyyy-mm-dd>")]
    BadRef(String),
    #[error("the cost model is dated {dated}, after the window it prices starts ({window_start}): a point-in-time violation")]
    DatedAfterWindow { dated: NaiveDate, window_start: NaiveDate },
    #[error("an `edge` spread needs an instrument-profile row (and the window it was measured over); none was supplied")]
    NoProfileRow,
    #[error("a `quoted` spread needs quote history, and none exists for this instrument; this does not fall back to EDGE")]
    NoQuotes,
    #[error("venue `{0}` has no class, so no latency default; classify it rather than guessing")]
    UnclassedVenue(String),
    #[error("`{0}` must be finite and non-negative")]
    BadNumber(&'static str),
}

/// A measured quantity and where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measured {
    pub value: f64,
    /// The instrument-profile row id (or series ref) it was read from.
    pub source: String,
    pub window_start: NaiveDate,
    pub window_end: NaiveDate,
}

/// The venue fee leg.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeeLeg {
    pub taker_bps: f64,
    pub maker_bps: f64,
    /// The `FeeBook` entry it was resolved from.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpreadLeg {
    /// Estimated from bars (EDGE); the half-spread cites a profile row and its window.
    Edge { half_spread_bps: Measured },
    /// Read from quote history.
    Quoted { half_spread_bps: Measured },
    Fixed { half_spread_bps: f64 },
}

impl SpreadLeg {
    /// An EDGE leg from a profile row. `None` refuses (M-2.2).
    ///
    /// # Errors
    /// No profile row.
    pub fn edge(profile_row: Option<Measured>) -> Result<Self, CostModelError> {
        profile_row.map(|m| Self::Edge { half_spread_bps: m }).ok_or(CostModelError::NoProfileRow)
    }

    /// A quoted leg. With no quotes it refuses, and says it will not become EDGE (M-3.4).
    ///
    /// # Errors
    /// No quote history.
    pub fn quoted(quotes: Option<Measured>) -> Result<Self, CostModelError> {
        quotes.map(|m| Self::Quoted { half_spread_bps: m }).ok_or(CostModelError::NoQuotes)
    }

    #[must_use]
    pub fn half_spread_bps(&self) -> f64 {
        match self {
            Self::Edge { half_spread_bps } | Self::Quoted { half_spread_bps } => half_spread_bps.value,
            Self::Fixed { half_spread_bps } => *half_spread_bps,
        }
    }
}

/// Square-root impact `dp = Y sigma sqrt(Q / V)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactLeg {
    pub y: f64,
    /// Where `Y` came from (a calibration, or the spec default and its citation).
    pub y_source: String,
    pub sigma: Measured,
    pub volume_usd: Measured,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostModel {
    pub instrument: String,
    pub dated: NaiveDate,
    pub venue_class: VenueClass,
    pub fee: FeeLeg,
    pub spread: SpreadLeg,
    pub impact: ImpactLeg,
    /// Charged per day on short notional; an explicit 0 is a statement.
    pub borrow_bps_per_day: f64,
    pub latency_bars: u32,
}

/// Builds a [`CostModel`]. Every leg is optional here so that a missing one is a named refusal.
#[derive(Clone, Debug, Default)]
pub struct CostModelBuilder {
    instrument: Option<String>,
    dated: Option<NaiveDate>,
    venue: Option<String>,
    fee: Option<FeeLeg>,
    spread: Option<SpreadLeg>,
    impact: Option<ImpactLeg>,
    borrow_bps_per_day: Option<f64>,
    latency_override: Option<u32>,
}

impl CostModelBuilder {
    #[must_use]
    pub fn new(instrument: impl Into<String>, dated: NaiveDate, venue: impl Into<String>) -> Self {
        Self { instrument: Some(instrument.into()), dated: Some(dated), venue: Some(venue.into()), ..Self::default() }
    }
    #[must_use]
    pub fn fee(mut self, f: FeeLeg) -> Self {
        self.fee = Some(f);
        self
    }
    #[must_use]
    pub fn spread(mut self, s: SpreadLeg) -> Self {
        self.spread = Some(s);
        self
    }
    #[must_use]
    pub fn impact(mut self, i: ImpactLeg) -> Self {
        self.impact = Some(i);
        self
    }
    #[must_use]
    pub fn borrow_bps_per_day(mut self, b: f64) -> Self {
        self.borrow_bps_per_day = Some(b);
        self
    }
    /// A deliberate departure from the venue class's default latency.
    #[must_use]
    pub fn latency_bars(mut self, n: u32) -> Self {
        self.latency_override = Some(n);
        self
    }

    /// # Errors
    /// The first missing leg, an unclassed venue, or a non-finite or negative number.
    pub fn build(self) -> Result<CostModel, CostModelError> {
        let instrument = self.instrument.ok_or(CostModelError::MissingLeg("instrument"))?;
        let dated = self.dated.ok_or(CostModelError::MissingLeg("date"))?;
        let class = venue_class(&self.venue.ok_or(CostModelError::MissingLeg("venue"))?)?;
        let fee = self.fee.ok_or(CostModelError::MissingLeg("fee"))?;
        let spread = self.spread.ok_or(CostModelError::MissingLeg("spread"))?;
        let impact = self.impact.ok_or(CostModelError::MissingLeg("impact"))?;
        let borrow = self.borrow_bps_per_day.ok_or(CostModelError::MissingLeg("borrow"))?;
        let ok = |x: f64| x.is_finite() && x >= 0.0;
        for (name, x) in [
            ("fee.taker_bps", fee.taker_bps),
            ("fee.maker_bps", fee.maker_bps),
            ("spread", spread.half_spread_bps()),
            ("impact.y", impact.y),
            ("impact.sigma", impact.sigma.value),
            ("impact.volume_usd", impact.volume_usd.value),
            ("borrow", borrow),
        ] {
            if !ok(x) {
                return Err(CostModelError::BadNumber(name));
            }
        }
        Ok(CostModel {
            instrument,
            dated,
            venue_class: class,
            fee,
            spread,
            impact,
            borrow_bps_per_day: borrow,
            latency_bars: self.latency_override.unwrap_or_else(|| latency_for(class)),
        })
    }
}

impl CostModel {
    #[must_use]
    pub fn reference(&self) -> String {
        CostModelRef { instrument: self.instrument.clone(), dated: self.dated }.format()
    }

    /// A model may price only a window that starts on or after its date.
    ///
    /// # Errors
    /// A model dated after the window's start.
    pub fn check_point_in_time(&self, window_start: NaiveDate) -> Result<(), CostModelError> {
        if self.dated > window_start {
            return Err(CostModelError::DatedAfterWindow { dated: self.dated, window_start });
        }
        Ok(())
    }

    /// What the position-series evaluator charges: the taker fee (a decision fills at the next
    /// open, a market order), the half-spread, and square-root impact.
    #[must_use]
    pub fn to_eval(&self) -> (Costs, SqrtImpact) {
        (
            Costs { fee_bps: self.fee.taker_bps, half_spread_bps: self.spread.half_spread_bps() },
            SqrtImpact { y: self.impact.y, sigma: self.impact.sigma.value, volume_usd: self.impact.volume_usd.value },
        )
    }

    /// Borrow on `short_notional_usd` held `days`.
    #[must_use]
    pub fn borrow_cost(&self, short_notional_usd: f64, days: f64) -> f64 {
        short_notional_usd.abs() * self.borrow_bps_per_day / 1e4 * days
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_series::{evaluate_with_impact, Bar, EvalConfig, PositionPoint, PositionSeries, PositionUnits};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn measured(v: f64, src: &str) -> Measured {
        Measured { value: v, source: src.into(), window_start: d(2025, 1, 1), window_end: d(2025, 6, 1) }
    }

    fn builder() -> CostModelBuilder {
        CostModelBuilder::new("BTC-USD", d(2025, 6, 1), "coinbase")
            .fee(FeeLeg { taker_bps: 10.0, maker_bps: 5.0, source: "feebook:coinbase:2025-06".into() })
            .spread(SpreadLeg::edge(Some(measured(2.0, "profile:btc:7"))).unwrap())
            .impact(ImpactLeg { y: 1.0, y_source: "spec default".into(), sigma: measured(0.01, "profile:btc:7"), volume_usd: measured(5e6, "profile:btc:7") })
            .borrow_bps_per_day(0.0)
    }

    #[test]
    fn a_model_with_no_spread_or_impact_leg_is_refused_with_the_leg_named() {
        let mut b = builder();
        b.spread = None;
        assert_eq!(b.build().unwrap_err(), CostModelError::MissingLeg("spread"));
        let mut b = builder();
        b.impact = None;
        assert_eq!(b.build().unwrap_err(), CostModelError::MissingLeg("impact"));
        let mut b = builder();
        b.fee = None;
        assert_eq!(b.build().unwrap_err(), CostModelError::MissingLeg("fee"));
        let mut b = builder();
        b.borrow_bps_per_day = None;
        assert_eq!(b.build().unwrap_err(), CostModelError::MissingLeg("borrow"), "zero is a statement, absence is not");
    }

    #[test]
    fn a_model_dated_after_the_window_is_refused() {
        let m = builder().build().unwrap();
        assert!(m.check_point_in_time(d(2025, 6, 1)).is_ok());
        assert!(m.check_point_in_time(d(2026, 1, 1)).is_ok(), "a model older than the window is fine");
        assert!(matches!(m.check_point_in_time(d(2025, 1, 1)), Err(CostModelError::DatedAfterWindow { .. })));
    }

    #[test]
    fn an_edge_spread_with_no_profile_row_and_quotes_with_no_history_refuse() {
        assert_eq!(SpreadLeg::edge(None).unwrap_err(), CostModelError::NoProfileRow);
        assert_eq!(SpreadLeg::quoted(None).unwrap_err(), CostModelError::NoQuotes);
        let leg = SpreadLeg::edge(Some(measured(2.0, "profile:btc:7"))).unwrap();
        let SpreadLeg::Edge { half_spread_bps } = leg else { panic!() };
        assert_eq!(half_spread_bps.source, "profile:btc:7", "the leg cites its row and window");
    }

    #[test]
    fn each_venue_class_resolves_to_a_latency_and_an_unclassed_venue_refuses() {
        for v in ["coinbase", "kraken", "alpaca", "synthetic", "deribit"] {
            assert_eq!(latency_for(venue_class(v).unwrap()), 0, "{v}");
        }
        assert_eq!(latency_for(venue_class("kalshi").unwrap()), 1);
        assert_eq!(venue_class("somewhere").unwrap_err(), CostModelError::UnclassedVenue("somewhere".into()));
        let unclassed = CostModelBuilder::new("X", d(2025, 1, 1), "somewhere");
        assert!(matches!(unclassed.build(), Err(CostModelError::UnclassedVenue(_))));
    }

    #[test]
    fn the_reference_round_trips() {
        let m = builder().build().unwrap();
        let r = CostModelRef::parse(&m.reference()).unwrap();
        assert_eq!(r, CostModelRef { instrument: "BTC-USD".into(), dated: d(2025, 6, 1) });
        assert!(CostModelRef::parse("cost:BTC-USD").is_err() && CostModelRef::parse("x:A@2025-01-01").is_err());
    }

    fn series_and_bars() -> (PositionSeries, Vec<Bar>) {
        let bars: Vec<Bar> = (0..80)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let p = 100.0 + (f64::from(i) * 0.4).sin() * 3.0;
                Bar { ts_ns: (i64::from(i) + 1) * 60_000_000_000, open: p, close: p + 0.2 }
            })
            .collect();
        let points = bars.iter().enumerate().map(|(i, b)| PositionPoint::new(b.ts_ns, if (i / 10) % 2 == 0 { 0.8 } else { 0.0 })).collect();
        (PositionSeries { points, position_units: PositionUnits::Fraction, allow_short: false }, bars)
    }

    fn final_equity(model: &CostModel) -> f64 {
        let (series, bars) = series_and_bars();
        let (costs, impact) = model.to_eval();
        evaluate_with_impact(&series, &bars, &EvalConfig { initial_equity: 1_000_000.0, costs, lot_size: None }, Some(impact))
            .unwrap()
            .final_equity
    }

    #[test]
    fn the_same_trades_cost_strictly_more_under_a_larger_y() {
        let mut lo = builder().build().unwrap();
        lo.impact.volume_usd.value = 1e6; // small enough that a 800k order moves the price
        let mut hi = lo.clone();
        hi.impact.y = 3.0;
        assert!(final_equity(&hi) < final_equity(&lo));
        let mut zero = lo.clone();
        zero.impact.y = 0.0;
        assert!(final_equity(&lo) < final_equity(&zero), "impact is charged at all");
    }

    #[test]
    fn a_larger_edge_spread_costs_more_and_the_numbers_flow_from_the_legs() {
        let m = builder().build().unwrap();
        let (c, i) = m.to_eval();
        assert_eq!((c.fee_bps, c.half_spread_bps, i.y), (10.0, 2.0, 1.0));
        assert!((m.borrow_cost(1_000_000.0, 2.0) - 0.0).abs() < 1e-12);
        let mut b = builder().borrow_bps_per_day(3.0).build().unwrap();
        assert!((b.borrow_cost(-1_000_000.0, 2.0) - 600.0).abs() < 1e-9);
        b.spread = SpreadLeg::Fixed { half_spread_bps: 8.0 };
        assert!(final_equity(&b) < final_equity(&m));
    }
}
