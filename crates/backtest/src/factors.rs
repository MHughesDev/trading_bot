//! The factor battery Gate 10 attributes against (SPEC §12.3 Gate 10, ADR-P2-15).
//!
//! Gate 10 asks whether the strategy is doing anything the factor battery is not
//! already doing. It is arithmetic — a Newey–West regression — and it has been
//! correct and unused since it was written, because **nothing produced the
//! factor returns to regress against**.
//!
//! ## Two rules, and the second is the one that matters
//!
//! 1. **The battery is declared per asset class on the profile**, in
//!    `mlops.gate_profile_asset_class` (crypto: `cmkt`, `csmb`, `cmom`,
//!    `ccarry`; equity: FF5 + MOM + STR + BAB + QMJ). Nothing here chooses
//!    factors. [`build_declared`] takes the declared names and builds exactly
//!    those, refusing any name it cannot construct — because a regression
//!    against a battery nobody declared answers a different question, and Gate
//!    10 refuses a mismatch anyway.
//!
//! 2. **The factors come from the market, never from the strategy.** This is
//!    Gate 11's known bug stated in advance: Gate 11 once labelled volatility
//!    regimes from the *strategy's own* returns, so its terciles were noise
//!    dressed as regimes. The same mistake here would be worse — regressing a
//!    strategy on factors derived from itself produces an R² near 1 and an alpha
//!    near 0, failing every candidate for a reason that is an artefact of the
//!    construction. So the only input type is [`MarketPanel`], a cross-section
//!    of *instruments*, and there is no constructor that takes a strategy
//!    return series.
//!
//! ## What is not here
//!
//! No equity battery. There is no equity data source on this platform yet, and a
//! `smb` built from nothing would be a column of zeros that regresses cleanly.
//! [`build_declared`] returns [`FactorGap::Undefined`] naming the factor, Gate 10
//! reports the battery does not match, and the verdict is `inconclusive` — which
//! is the true state of the world.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;

use ledger::evidence::FactorSeries;

use crate::store::LoadedBar;

/// Crypto market: the equal-weighted cross-section.
pub const CRYPTO_MARKET: &str = "cmkt";
/// Crypto size: small minus big, by traded dollar volume.
pub const CRYPTO_SIZE: &str = "csmb";
/// Crypto momentum: winners minus losers, skipping the most recent period.
pub const CRYPTO_MOMENTUM: &str = "cmom";
/// Crypto carry: high funding minus low funding.
pub const CRYPTO_CARRY: &str = "ccarry";

/// The smallest cross-section a tercile sort means anything on.
///
/// Six, so the top and bottom thirds are two instruments each. On three, a
/// "tercile" is one instrument and the size factor is the difference between two
/// individual coins wearing a factor's name.
pub const MIN_CROSS_SECTION: usize = 6;

/// How the sorts are formed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryConfig {
    /// Periods of trailing dollar volume the size sort ranks on.
    pub size_window: usize,
    /// Periods of trailing return the momentum sort ranks on.
    pub momentum_lookback: usize,
    /// Periods skipped between the momentum window and the period being ranked.
    ///
    /// One, always. Momentum measured up to and including the period it predicts
    /// is not momentum, it is the return.
    pub momentum_skip: usize,
    /// Periods of trailing funding the carry sort ranks on.
    pub carry_window: usize,
}

impl Default for BatteryConfig {
    /// Daily-bar defaults: a month of volume, a quarter of momentum, a week of
    /// funding. They are the conventional windows, and they are stated here
    /// rather than guessed per call so two runs of the same battery are the same
    /// battery.
    fn default() -> Self {
        Self {
            size_window: 30,
            momentum_lookback: 90,
            momentum_skip: 1,
            carry_window: 7,
        }
    }
}

/// Why a declared battery could not be built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FactorGap {
    #[error(
        "`{0}` is a declared factor this platform has no construction for; a regression against a \
         factor built from nothing is a regression against a column of zeros"
    )]
    Undefined(String),
    #[error(
        "the universe has {0} instruments and a tercile sort needs at least {1}; a `size` factor \
         over three coins is the difference between two of them wearing a factor's name"
    )]
    TooFewInstruments(usize, usize),
    #[error(
        "the instruments share no common timestamp grid, so there is no cross-section to sort"
    )]
    NoCommonGrid,
    #[error(
        "the common grid is {0} periods and the declared windows need {1}; a factor series shorter \
         than its own formation window is not the factor"
    )]
    TooShort(usize, usize),
    #[error(
        "no funding series for {0}; crypto carry *is* the funding rate, and this platform collects \
         funding forward-only (ADR-0027), so it does not exist for history that predates the \
         collector"
    )]
    CarryUnavailable(String),
}

/// One instrument's contribution to the cross-section.
///
/// Built from bars, by [`InstrumentPanel::from_bars`]. There is deliberately no
/// constructor that takes a strategy's return series: see the module header.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentPanel {
    pub instrument: String,
    /// Period ends, strictly increasing.
    pub ts: Vec<DateTime<Utc>>,
    /// Simple returns over each period.
    pub returns: Vec<f64>,
    /// Dollar volume over each period — the size proxy. Free crypto data has no
    /// float-adjusted market cap, and traded value is the measurable stand-in
    /// that does not require a vendor.
    pub dollar_volume: Vec<f64>,
    /// Per-period funding rate, when a collector has been running. `None` is the
    /// normal state for history, and it makes carry unavailable rather than zero.
    pub funding: Option<Vec<f64>>,
}

impl InstrumentPanel {
    /// Build from this instrument's own bars.
    ///
    /// The first bar produces no return (there is nothing before it), so the
    /// series is one shorter than the bar list and starts at the second bar's
    /// close time.
    #[must_use]
    pub fn from_bars(instrument: impl Into<String>, bars: &[LoadedBar]) -> Self {
        let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or(0.0);
        let mut ts = Vec::new();
        let mut returns = Vec::new();
        let mut dollar_volume = Vec::new();
        for w in bars.windows(2) {
            let (prev, cur) = (&w[0], &w[1]);
            let (p0, p1) = (f(prev.close), f(cur.close));
            if !(p0.is_finite() && p0 > 0.0 && p1.is_finite()) {
                continue;
            }
            ts.push(DateTime::from_timestamp_nanos(cur.ts_ns));
            returns.push(p1 / p0 - 1.0);
            dollar_volume.push(f(cur.volume) * p1);
        }
        Self {
            instrument: instrument.into(),
            ts,
            returns,
            dollar_volume,
            funding: None,
        }
    }

    /// Attach a funding series aligned to [`Self::ts`]. Carry is unavailable
    /// without it, and a mis-aligned series is refused rather than truncated.
    #[must_use]
    pub fn with_funding(mut self, funding: Vec<f64>) -> Self {
        if funding.len() == self.ts.len() {
            self.funding = Some(funding);
        }
        self
    }
}

/// The cross-section the battery is built from.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketPanel {
    /// What universe this is, for the stored series' provenance.
    pub universe: String,
    pub instruments: Vec<InstrumentPanel>,
}

/// The cross-section, re-indexed onto the timestamps every instrument shares.
struct Aligned {
    ts: Vec<DateTime<Utc>>,
    names: Vec<String>,
    /// `returns[i][t]`, `volume[i][t]`, `funding[i][t]`.
    returns: Vec<Vec<f64>>,
    volume: Vec<Vec<f64>>,
    funding: Vec<Option<Vec<f64>>>,
}

impl MarketPanel {
    fn align(&self) -> Result<Aligned, FactorGap> {
        if self.instruments.len() < MIN_CROSS_SECTION {
            return Err(FactorGap::TooFewInstruments(
                self.instruments.len(),
                MIN_CROSS_SECTION,
            ));
        }
        // The intersection, not the union: a factor period in which half the
        // universe is missing is a factor computed over a different universe,
        // and nothing downstream can see the universe changed.
        let mut common: Option<BTreeSet<DateTime<Utc>>> = None;
        for p in &self.instruments {
            let mine: BTreeSet<DateTime<Utc>> = p.ts.iter().copied().collect();
            common = Some(match common {
                None => mine,
                Some(c) => c.intersection(&mine).copied().collect(),
            });
        }
        let ts: Vec<DateTime<Utc>> = common.unwrap_or_default().into_iter().collect();
        if ts.is_empty() {
            return Err(FactorGap::NoCommonGrid);
        }

        let mut names = Vec::with_capacity(self.instruments.len());
        let mut returns = Vec::with_capacity(self.instruments.len());
        let mut volume = Vec::with_capacity(self.instruments.len());
        let mut funding = Vec::with_capacity(self.instruments.len());
        for p in &self.instruments {
            let index: BTreeMap<DateTime<Utc>, usize> =
                p.ts.iter()
                    .copied()
                    .enumerate()
                    .map(|(i, t)| (t, i))
                    .collect();
            let pick = |src: &[f64]| -> Vec<f64> {
                ts.iter()
                    .map(|t| {
                        index
                            .get(t)
                            .and_then(|i| src.get(*i))
                            .copied()
                            .unwrap_or(0.0)
                    })
                    .collect()
            };
            names.push(p.instrument.clone());
            returns.push(pick(&p.returns));
            volume.push(pick(&p.dollar_volume));
            funding.push(p.funding.as_ref().map(|f| pick(f)));
        }
        Ok(Aligned {
            ts,
            names,
            returns,
            volume,
            funding,
        })
    }
}

/// Build exactly the factors the profile declared for this asset class.
///
/// `declared` comes from `mlops.gate_profile_asset_class`, never from a caller's
/// idea of what a battery should contain. A name with no construction is
/// [`FactorGap::Undefined`], not a zero series: Gate 10 then reports the battery
/// does not match the declared one and returns `inconclusive`, which is the
/// truthful verdict for a strategy nobody can attribute.
///
/// # Errors
/// [`FactorGap`], naming which factor and why.
pub fn build_declared(
    declared: &[String],
    panel: &MarketPanel,
    cfg: BatteryConfig,
) -> Result<Vec<FactorSeries>, FactorGap> {
    let aligned = panel.align()?;
    let formation = cfg
        .size_window
        .max(cfg.momentum_lookback + cfg.momentum_skip)
        .max(cfg.carry_window);
    if aligned.ts.len() <= formation {
        return Err(FactorGap::TooShort(aligned.ts.len(), formation + 1));
    }
    let start = formation;
    let ts: Vec<DateTime<Utc>> = aligned.ts[start..].to_vec();

    let mut out = Vec::with_capacity(declared.len());
    for name in declared {
        let returns = match name.as_str() {
            CRYPTO_MARKET => equal_weight_market(&aligned, start),
            CRYPTO_SIZE => sorted_spread(
                &aligned,
                start,
                cfg.size_window,
                SortOn::Volume,
                Leg::LowMinusHigh,
            )?,
            CRYPTO_MOMENTUM => sorted_spread(
                &aligned,
                start,
                cfg.momentum_lookback,
                SortOn::TrailingReturn(cfg.momentum_skip),
                Leg::HighMinusLow,
            )?,
            CRYPTO_CARRY => sorted_spread(
                &aligned,
                start,
                cfg.carry_window,
                SortOn::Funding,
                Leg::HighMinusLow,
            )?,
            other => return Err(FactorGap::Undefined(other.to_string())),
        };
        out.push(FactorSeries {
            factor: name.clone(),
            ts: ts.clone(),
            returns,
        });
    }
    Ok(out)
}

/// Which characteristic a leg is sorted on.
#[derive(Clone, Copy)]
enum SortOn {
    Volume,
    /// Trailing cumulative return, skipping this many of the most recent
    /// periods. Skipping is not optional: momentum measured up to the period it
    /// predicts is the period's own return.
    TrailingReturn(usize),
    Funding,
}

/// Which side of the sort is long.
#[derive(Clone, Copy)]
enum Leg {
    HighMinusLow,
    LowMinusHigh,
}

#[allow(clippy::cast_precision_loss)]
fn equal_weight_market(a: &Aligned, start: usize) -> Vec<f64> {
    (start..a.ts.len())
        .map(|t| {
            let n = a.returns.len() as f64;
            a.returns.iter().map(|r| r[t]).sum::<f64>() / n
        })
        .collect()
}

/// The long-minus-short spread of a tercile sort, rebalanced every period.
///
/// Strictly causal: the rank at period `t` uses only data ending at `t-1` (and
/// `t-1-skip` for momentum), so a factor return at `t` is earned by a portfolio
/// that could have been held going into `t`. A sort that reads `t` is a
/// look-ahead dressed as a factor, and it would show up as the strategy having
/// no alpha against a battery that knew the future.
#[allow(clippy::cast_precision_loss)]
fn sorted_spread(
    a: &Aligned,
    start: usize,
    window: usize,
    on: SortOn,
    leg: Leg,
) -> Result<Vec<f64>, FactorGap> {
    if let SortOn::Funding = on {
        for (i, f) in a.funding.iter().enumerate() {
            if f.is_none() {
                return Err(FactorGap::CarryUnavailable(a.names[i].clone()));
            }
        }
    }
    let n = a.returns.len();
    let third = (n / 3).max(1);
    let mut out = Vec::with_capacity(a.ts.len() - start);
    for t in start..a.ts.len() {
        let mut ranked: Vec<(f64, usize)> = (0..n)
            .map(|i| {
                let score = match on {
                    SortOn::Volume => {
                        let lo = t.saturating_sub(window);
                        a.volume[i][lo..t].iter().sum::<f64>() / window as f64
                    }
                    SortOn::TrailingReturn(skip) => {
                        let hi = t.saturating_sub(skip);
                        let lo = hi.saturating_sub(window);
                        a.returns[i][lo..hi]
                            .iter()
                            .fold(1.0_f64, |acc, r| acc * (1.0 + r))
                            - 1.0
                    }
                    SortOn::Funding => {
                        let lo = t.saturating_sub(window);
                        let f = a.funding[i].as_ref().expect("checked above");
                        f[lo..t].iter().sum::<f64>() / window as f64
                    }
                };
                (score, i)
            })
            .collect();
        ranked.sort_by(|x, y| x.0.total_cmp(&y.0));
        let low: f64 = ranked[..third]
            .iter()
            .map(|(_, i)| a.returns[*i][t])
            .sum::<f64>()
            / third as f64;
        let high: f64 = ranked[n - third..]
            .iter()
            .map(|(_, i)| a.returns[*i][t])
            .sum::<f64>()
            / third as f64;
        out.push(match leg {
            Leg::HighMinusLow => high - low,
            Leg::LowMinusHigh => low - high,
        });
    }
    Ok(out)
}

/// Line the stored factor series up with the strategy's own stored return
/// series, period for period.
///
/// Returns `None` when the overlap is too short to regress, rather than padding:
/// a regression over a padded series is a regression over invented periods.
/// `min_periods` is the caller's floor — `newey_west_fit` needs 30 and
/// `3 × (factors + 1)`, and it refuses below that anyway.
#[must_use]
pub fn align_to_strategy(
    strategy_ts: &[DateTime<Utc>],
    strategy_returns: &[f64],
    factors: &[FactorSeries],
    min_periods: usize,
) -> Option<(Vec<f64>, BTreeMap<String, Vec<f64>>)> {
    if factors.is_empty() || strategy_ts.len() != strategy_returns.len() {
        return None;
    }
    let mut common: BTreeSet<DateTime<Utc>> = strategy_ts.iter().copied().collect();
    for f in factors {
        let mine: BTreeSet<DateTime<Utc>> = f.ts.iter().copied().collect();
        common = common.intersection(&mine).copied().collect();
    }
    if common.len() < min_periods {
        return None;
    }
    let grid: Vec<DateTime<Utc>> = common.into_iter().collect();
    let index = |ts: &[DateTime<Utc>]| -> BTreeMap<DateTime<Utc>, usize> {
        ts.iter()
            .copied()
            .enumerate()
            .map(|(i, t)| (t, i))
            .collect()
    };
    let s_idx = index(strategy_ts);
    let y: Vec<f64> = grid
        .iter()
        .filter_map(|t| s_idx.get(t).map(|i| strategy_returns[*i]))
        .collect();
    let mut x: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for f in factors {
        let f_idx = index(&f.ts);
        let col: Vec<f64> = grid
            .iter()
            .filter_map(|t| f_idx.get(t).map(|i| f.returns[*i]))
            .collect();
        if col.len() != y.len() {
            return None;
        }
        x.insert(f.factor.clone(), col);
    }
    Some((y, x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn panel(n_instruments: usize, n_periods: usize, funding: bool) -> MarketPanel {
        let t0 = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let instruments = (0..n_instruments)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let drift = 0.0002 * (i as f64 - 2.0);
                let p = InstrumentPanel {
                    instrument: format!("SYM{i}-USD"),
                    ts: (0..n_periods)
                        .map(|k| t0 + Duration::days(i64::try_from(k).unwrap()))
                        .collect(),
                    returns: (0..n_periods)
                        .map(|k| {
                            #[allow(clippy::cast_precision_loss)]
                            let wobble = ((k * 7 + i * 13) % 11) as f64 / 1000.0 - 0.005;
                            drift + wobble
                        })
                        .collect(),
                    #[allow(clippy::cast_precision_loss)]
                    dollar_volume: (0..n_periods)
                        .map(|k| 1e6 * (i as f64 + 1.0) + (k % 5) as f64)
                        .collect(),
                    funding: None,
                };
                if funding {
                    #[allow(clippy::cast_precision_loss)]
                    let f: Vec<f64> = (0..n_periods)
                        .map(|k| 0.0001 * ((k + i) % 3) as f64)
                        .collect();
                    p.with_funding(f)
                } else {
                    p
                }
            })
            .collect();
        MarketPanel {
            universe: "test-universe".into(),
            instruments,
        }
    }

    fn declared(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_declared_crypto_battery_is_built_from_the_cross_section() {
        let p = panel(9, 200, true);
        let built = build_declared(
            &declared(&[CRYPTO_MARKET, CRYPTO_SIZE, CRYPTO_MOMENTUM, CRYPTO_CARRY]),
            &p,
            BatteryConfig::default(),
        )
        .expect("a nine-instrument panel with funding");
        assert_eq!(built.len(), 4);
        let names: Vec<&str> = built.iter().map(|f| f.factor.as_str()).collect();
        assert_eq!(
            names,
            vec![CRYPTO_MARKET, CRYPTO_SIZE, CRYPTO_MOMENTUM, CRYPTO_CARRY]
        );
        for f in &built {
            assert_eq!(f.ts.len(), f.returns.len());
            assert!(!f.returns.is_empty());
            assert!(f.returns.iter().all(|r| r.is_finite()));
        }
        // The market factor is the cross-sectional mean, so it cannot equal any
        // one member's series — the check that it is a *market*, not a coin.
        let market = &built[0].returns;
        for inst in &p.instruments {
            assert_ne!(
                &inst.returns[inst.returns.len() - market.len()..],
                &market[..]
            );
        }
    }

    /// The trap the module header names. A factor the platform cannot construct
    /// must be refused by name, not shipped as zeros that regress beautifully.
    #[test]
    fn an_undefined_factor_is_refused_rather_than_zeroed() {
        let p = panel(9, 200, true);
        assert_eq!(
            build_declared(&declared(&["smb", "hml"]), &p, BatteryConfig::default()),
            Err(FactorGap::Undefined("smb".into()))
        );
    }

    /// Carry *is* the funding rate. Without a funding series there is no carry
    /// factor, and inventing a flat one would make Gate 10 regress against a
    /// constant — which contributes nothing to R² and quietly changes nothing,
    /// while letting the battery claim to match the declared one.
    #[test]
    fn carry_without_funding_is_refused_by_instrument_name() {
        let p = panel(9, 200, false);
        assert_eq!(
            build_declared(&declared(&[CRYPTO_CARRY]), &p, BatteryConfig::default()),
            Err(FactorGap::CarryUnavailable("SYM0-USD".into()))
        );
        // …and the rest of the battery still builds, so the gap is precisely
        // located rather than taking the whole battery down anonymously.
        assert!(build_declared(&declared(&[CRYPTO_MARKET]), &p, BatteryConfig::default()).is_ok());
    }

    #[test]
    fn a_universe_too_small_to_sort_is_refused() {
        let p = panel(3, 200, true);
        assert_eq!(
            build_declared(&declared(&[CRYPTO_SIZE]), &p, BatteryConfig::default()),
            Err(FactorGap::TooFewInstruments(3, MIN_CROSS_SECTION))
        );
    }

    #[test]
    fn a_panel_shorter_than_its_formation_window_is_refused() {
        let p = panel(9, 40, true);
        assert!(matches!(
            build_declared(&declared(&[CRYPTO_MOMENTUM]), &p, BatteryConfig::default()),
            Err(FactorGap::TooShort(40, _))
        ));
    }

    /// Momentum must not read the period it is predicting. Ranking on a window
    /// that includes `t` makes the "factor" the cross-sectional return sorted by
    /// itself, which is mechanically enormous and entirely fake.
    #[test]
    fn momentum_skips_the_period_it_ranks() {
        let p = panel(9, 200, true);
        let honest = build_declared(&declared(&[CRYPTO_MOMENTUM]), &p, BatteryConfig::default())
            .expect("builds");
        let peeking = build_declared(
            &declared(&[CRYPTO_MOMENTUM]),
            &p,
            BatteryConfig {
                momentum_skip: 0,
                ..BatteryConfig::default()
            },
        )
        .expect("builds");
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        assert!(
            mean(&peeking[0].returns) > mean(&honest[0].returns),
            "a peeking sort earns more by construction: {} vs {}",
            mean(&peeking[0].returns),
            mean(&honest[0].returns)
        );
    }

    #[test]
    fn alignment_refuses_an_overlap_too_short_to_regress() {
        let t0 = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let f = FactorSeries {
            factor: CRYPTO_MARKET.into(),
            ts: (0..10).map(|i| t0 + Duration::days(i)).collect(),
            returns: vec![0.001; 10],
        };
        let s_ts: Vec<_> = (0..10).map(|i| t0 + Duration::days(i)).collect();
        assert!(align_to_strategy(&s_ts, &[0.002; 10], std::slice::from_ref(&f), 30).is_none());
        assert!(align_to_strategy(&s_ts, &[0.002; 10], std::slice::from_ref(&f), 5).is_some());
    }

    #[test]
    fn alignment_intersects_rather_than_pads() {
        let t0 = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let f = FactorSeries {
            factor: CRYPTO_MARKET.into(),
            ts: (0..50).map(|i| t0 + Duration::days(i)).collect(),
            returns: vec![0.001; 50],
        };
        // The strategy starts twenty days later and runs longer.
        let s_ts: Vec<_> = (20..100).map(|i| t0 + Duration::days(i)).collect();
        let (y, x) = align_to_strategy(&s_ts, &vec![0.002; 80], std::slice::from_ref(&f), 10)
            .expect("thirty overlapping days");
        assert_eq!(y.len(), 30);
        assert_eq!(x[CRYPTO_MARKET].len(), 30);
    }
}
