//! What a Run actually traded, and the two ladders derived from it (SPEC §12.3
//! gates 3 and 4).
//!
//! Gate 3 asks *how wrong the cost model has to be* before the edge disappears.
//! Gate 4 asks *how much money* the strategy can carry before the edge halves.
//! Both questions are answered from the same place — the trade list — and both
//! have the same failure mode: an answer produced from evidence that is not
//! there.
//!
//! ## The one rule
//!
//! **Every function here returns [`Missing`] rather than a number it cannot
//! support.** No zero, no neutral value, no "assume it's fine". The reason is
//! specific to these two gates and is worth stating:
//!
//! * A cost ladder whose rungs all earn the same metric — because nothing in the
//!   pipeline applied the multiple — *passes* Gate 3, with the verdict "the edge
//!   survives every rung tried". That is the single most dangerous output this
//!   module could produce: a dishonest pass wearing a real gate's name. So
//!   [`cost_ladder`] refuses to build a ladder from a Run that paid no costs,
//!   and the multiples it applies are applied to costs that were actually
//!   charged.
//! * A capacity curve computed without ADV is a curve about nothing. [`Ladder`]
//!   refuses without it, and Gate 4 then says so.
//!
//! ## Where the size-dependence comes from, stated plainly
//!
//! The simulator does not take an AUM: a Run at ten times the size returns the
//! same percentage path. So the decay in [`Ladder::from_runs`] is **modelled**,
//! by Almgren's square-root law over the instrument's own measured ADV and
//! volatility — not observed. What *is* observed is every rung: each is a real,
//! separately registered, separately counted trial (INV-16 has no exception for
//! "runs we did to satisfy a gate"), and each rung records the trial id it cost.
//! ADR-P2-34 records the judgment.

use std::collections::BTreeMap;

use rust_decimal::prelude::ToPrimitive;

use ledger::evidence::{CapacityBase, CapacityPoint, CostRung, InstrumentPnl};

use crate::gates::evaluators::{square_root_impact, CapacityInputs};
use crate::run::RunResult;

/// The cost multiples the ladder is measured at.
///
/// It runs to 6× rather than stopping at the profile's 3× bar because a ladder
/// that stops at the threshold can only ever report "at least 3×", and the
/// difference between an edge that dies at 3.1× and one that survives 6× is the
/// difference between a marginal strategy and a real one. The lowest rung is 1×
/// — the cost model as modelled — because the gate's question is about a
/// *multiple* of it.
pub const COST_MULTIPLES: &[f64] = &[1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0];

/// The AUM multiples the capacity ladder is measured at.
///
/// Seven rungs spanning two orders of magnitude, because a square-root impact
/// curve is flat until it is not and a ladder that only doubles finds nothing.
/// Seven is inside §12.3's "roughly 5–8", and each one is a counted trial, so
/// the count is a real cost rather than a free parameter.
pub const AUM_MULTIPLES: &[f64] = &[1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0];

/// Why a ladder or a cross-section could not be produced.
///
/// Every variant is a sentence a gate verdict can carry verbatim. An enum rather
/// than a bare `Option` because "we could not measure this" and *why* are
/// different facts, and only the second one is actionable.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Missing {
    #[error("the run recorded no trades, so there is nothing to attribute or to charge")]
    NoTrades,
    #[error("the run recorded no equity curve, so there is no denominator to express a return in")]
    NoEquityCurve,
    #[error("the run's starting equity is not a positive amount")]
    NoStartingEquity,
    #[error(
        "the run paid no costs at all, so multiplying them changes nothing: a flat ladder would \
         report `the edge survives every rung tried`, which is a pass earned by the absence of a \
         cost model rather than by the strategy"
    )]
    NoCostsCharged,
    #[error("the return series is too short, or has no dispersion, to carry a Sharpe")]
    NoSharpe,
    #[error("no ADV was measured for {0}; capacity cannot be assessed on a guess")]
    NoAdv(String),
    #[error("no daily volatility was measured for {0}; the impact model has no scale")]
    NoVolatility(String),
    #[error("no instrument traded on more than zero days")]
    NoTradingDays,
    #[error(
        "the rungs were produced by more than one engine ({0}); a ladder that mixes engine \
         versions is a ladder whose rungs were charged differently, and the difference between \
         them is the engine rather than the size"
    )]
    MixedEngineVersions(String),
}

/// The one engine version behind a set of runs, or a refusal naming them all.
///
/// This exists because of a real defect, and the defect is the argument for it.
/// Every fill the simulator produced before ADR-P6-12 was charged **zero**
/// commission: the SDK's instrument constructors left maker/taker unset and the
/// default fee model read that as zero, so the engine computed a gross number
/// and labelled it net. A cost ladder built across that boundary would have
/// rungs charged on two different cost models, and the "sensitivity" would be
/// measuring the fix.
///
/// Two protections, and the first is stronger than this one: a run that paid no
/// costs yields [`Missing::NoCostsCharged`] and no ladder at all, which is
/// exactly the signature of a pre-fix run. This is the second — for statistics
/// aggregated over several runs, where one of them could be post-fix.
///
/// # Errors
/// [`Missing::MixedEngineVersions`] listing what was mixed.
pub fn one_engine<'a>(versions: impl IntoIterator<Item = &'a str>) -> Result<String, Missing> {
    let distinct: std::collections::BTreeSet<&str> = versions.into_iter().collect();
    match distinct.len() {
        1 => Ok((*distinct.iter().next().expect("one")).to_string()),
        _ => Err(Missing::MixedEngineVersions(
            distinct.into_iter().collect::<Vec<_>>().join(", "),
        )),
    }
}

/// The unit an instrument's P&L is denominated in.
///
/// Parsed from the symbol only as a fallback: the executor knows the venue's
/// quote currency and says so on [`crate::run::InstrumentStat`]. When neither is
/// available the instrument gets its own unique unit, which makes aggregation
/// refuse — the conservative direction, because the alternative is summing a
/// USDT P&L into a USD one and reporting a concentration share computed over two
/// different units.
#[must_use]
pub fn numeraire_of(symbol: &str) -> Option<String> {
    for sep in ['-', '/', '_'] {
        if let Some((_, quote)) = symbol.rsplit_once(sep) {
            if !quote.is_empty() {
                return Some(quote.to_ascii_uppercase());
            }
        }
    }
    // Suffix forms (`BTCUSDT`). Longest first, so `USDT` is not read as `USD`
    // with a stray T.
    const QUOTES: &[&str] = &[
        "USDT", "USDC", "TUSD", "USD", "EUR", "GBP", "JPY", "BTC", "ETH",
    ];
    let upper = symbol.to_ascii_uppercase();
    for q in QUOTES {
        if upper.len() > q.len() && upper.ends_with(q) {
            return Some((*q).to_string());
        }
    }
    None
}

/// One instrument's footprint inside a Run.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentExposure {
    pub instrument: String,
    pub numeraire: String,
    /// Realized P&L **net** of costs.
    pub net_pnl: f64,
    pub costs_paid: f64,
    /// Notional transacted, entries and exits both.
    pub traded_notional: f64,
    /// The largest single calendar day's transacted notional.
    pub peak_daily_notional: f64,
    /// Calendar days on which anything was transacted.
    pub trading_days: u32,
    /// 20-day dollar ADV, measured from the run's own bars by the executor.
    pub adv_usd: Option<f64>,
    pub daily_vol: Option<f64>,
}

/// What a Run traded, per instrument and in total.
///
/// Named `Exposure` because that is what it is: the platform's answer to "what
/// was actually put at risk, in what, and at what cost".
#[derive(Clone, Debug, PartialEq)]
pub struct Exposure {
    pub by_instrument: BTreeMap<String, InstrumentExposure>,
    /// P&L before costs.
    pub gross_pnl: f64,
    pub costs_paid: f64,
    pub starting_equity: f64,
}

impl Exposure {
    /// Read a Run's trade list into an exposure.
    ///
    /// # Errors
    /// [`Missing::NoTrades`], [`Missing::NoEquityCurve`] or
    /// [`Missing::NoStartingEquity`] — each of which makes the gates that read
    /// this inconclusive, never passing.
    pub fn from_run(result: &RunResult) -> Result<Self, Missing> {
        if result.trades.is_empty() {
            return Err(Missing::NoTrades);
        }
        let Some((_, starting_equity)) = result.equity_curve.first().copied() else {
            return Err(Missing::NoEquityCurve);
        };
        if !(starting_equity.is_finite() && starting_equity > 0.0) {
            return Err(Missing::NoStartingEquity);
        }

        let declared: BTreeMap<&str, &crate::run::InstrumentStat> = result
            .instrument_stats
            .iter()
            .map(|s| (s.instrument.as_str(), s))
            .collect();

        let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or(0.0);
        let mut by_instrument: BTreeMap<String, InstrumentExposure> = BTreeMap::new();
        // `(instrument, calendar day) -> notional`, for the peak-day figure Gate 4
        // reads. A day, not the whole run: participation is a question about the
        // worst day, because that is the day the fills were fiction.
        let mut daily: BTreeMap<(String, chrono::NaiveDate), f64> = BTreeMap::new();

        for t in &result.trades {
            let entry_notional = f(t.qty) * f(t.entry_price);
            let exit_notional = f(t.qty) * f(t.exit_price);
            let stat = declared.get(t.symbol.as_str());
            let e = by_instrument
                .entry(t.symbol.clone())
                .or_insert_with(|| InstrumentExposure {
                    instrument: t.symbol.clone(),
                    numeraire: stat
                        .and_then(|s| s.numeraire.clone())
                        .or_else(|| numeraire_of(&t.symbol))
                        // No declared unit and an unparseable symbol: give it a
                        // unit nothing else shares, so aggregation refuses
                        // rather than silently summing across units.
                        .unwrap_or_else(|| format!("unknown:{}", t.symbol)),
                    net_pnl: 0.0,
                    costs_paid: 0.0,
                    traded_notional: 0.0,
                    peak_daily_notional: 0.0,
                    trading_days: 0,
                    adv_usd: stat.and_then(|s| s.adv_usd),
                    daily_vol: stat.and_then(|s| s.daily_vol),
                });
            e.net_pnl += f(t.pnl);
            e.costs_paid += f(t.costs_paid);
            e.traded_notional += entry_notional + exit_notional;
            *daily
                .entry((t.symbol.clone(), t.entry_time.date_naive()))
                .or_default() += entry_notional;
            *daily
                .entry((t.symbol.clone(), t.exit_time.date_naive()))
                .or_default() += exit_notional;
        }

        for ((symbol, _), notional) in &daily {
            if let Some(e) = by_instrument.get_mut(symbol) {
                e.peak_daily_notional = e.peak_daily_notional.max(*notional);
                e.trading_days += 1;
            }
        }

        let costs_paid: f64 = by_instrument.values().map(|e| e.costs_paid).sum();
        let net_pnl: f64 = by_instrument.values().map(|e| e.net_pnl).sum();
        Ok(Self {
            by_instrument,
            gross_pnl: net_pnl + costs_paid,
            costs_paid,
            starting_equity,
        })
    }

    /// The per-instrument rows migration 0119 stores.
    #[must_use]
    pub fn instrument_rows(&self) -> Vec<InstrumentPnl> {
        self.by_instrument
            .values()
            .map(|e| InstrumentPnl {
                instrument: e.instrument.clone(),
                numeraire: e.numeraire.clone(),
                net_pnl: e.net_pnl,
                traded_notional: e.traded_notional,
                peak_daily_notional: e.peak_daily_notional,
                adv_usd: e.adv_usd,
                daily_vol: e.daily_vol,
            })
            .collect()
    }
}

/// The cost ladder: the net return the Run would have earned had costs been
/// `m` times what the cost model charged (Gate 3).
///
/// Linear in `m` because the trade path is fixed: scaling a cost model does not
/// change decisions a strategy made without reading it. That is the assumption
/// every cost-sensitivity analysis makes, and it is exact here — the path is the
/// one that was simulated, and the costs are the ones that were charged.
///
/// # Errors
/// [`Missing::NoCostsCharged`] above all: a Run that paid nothing produces a
/// flat ladder, and a flat ladder *passes* Gate 3 on the strength of the cost
/// model's absence.
pub fn cost_ladder(exposure: &Exposure, multiples: &[f64]) -> Result<Vec<CostRung>, Missing> {
    if !(exposure.costs_paid.is_finite() && exposure.costs_paid > 0.0) {
        return Err(Missing::NoCostsCharged);
    }
    let rungs: Vec<CostRung> = multiples
        .iter()
        .copied()
        .filter(|m| m.is_finite() && *m > 0.0)
        .map(|multiple| CostRung {
            multiple,
            net_metric: (exposure.gross_pnl - multiple * exposure.costs_paid)
                / exposure.starting_equity,
        })
        .collect();
    if rungs.len() < 2 {
        return Err(Missing::NoCostsCharged);
    }
    Ok(rungs)
}

/// The multiple of modelled costs at which the edge reaches exactly zero.
///
/// Closed-form — `gross / costs` — and identical to what
/// [`crate::gates::evaluators::breakeven_multiple`] interpolates from the ladder
/// above, which is the check that the stored scalar and the stored shape agree.
///
/// # Errors
/// [`Missing::NoCostsCharged`] when nothing was charged.
pub fn breakeven_cost_multiple(exposure: &Exposure) -> Result<f64, Missing> {
    if !(exposure.costs_paid.is_finite() && exposure.costs_paid > 0.0) {
        return Err(Missing::NoCostsCharged);
    }
    Ok((exposure.gross_pnl / exposure.costs_paid).max(0.0))
}

/// The per-period return statistics the capacity curve is built on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SharpeBasis {
    pub mean: f64,
    pub sd: f64,
    pub periods: usize,
    pub periods_per_year: f64,
}

impl SharpeBasis {
    /// Measure the basis from a return series and the span it covers.
    ///
    /// # Errors
    /// [`Missing::NoSharpe`] for a series too short or too flat to carry one.
    /// A standard deviation of zero is not a Sharpe of infinity.
    pub fn measure(returns: &[f64], years: f64) -> Result<Self, Missing> {
        let n = returns.len();
        if n < 2 || !(years.is_finite() && years > 0.0) {
            return Err(Missing::NoSharpe);
        }
        #[allow(clippy::cast_precision_loss)]
        let nf = n as f64;
        let mean = returns.iter().sum::<f64>() / nf;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (nf - 1.0);
        let sd = var.sqrt();
        // The same relative floor `gate_worker::length_inputs` uses, for the same
        // reason: five hundred identical values leave a standard deviation around
        // 1e-19 from rounding alone, and dividing by it produces a Sharpe that
        // passes every gate in the stack.
        let magnitude = returns.iter().fold(0.0_f64, |a, b| a.max(b.abs())).max(1.0);
        if !(sd.is_finite() && sd > magnitude * 1e-12) {
            return Err(Missing::NoSharpe);
        }
        Ok(Self {
            mean,
            sd,
            periods: n,
            periods_per_year: nf / years,
        })
    }

    /// Annualized Sharpe after subtracting a per-period drag.
    #[must_use]
    pub fn sharpe_after(&self, drag_per_period: f64) -> f64 {
        (self.mean - drag_per_period) / self.sd * self.periods_per_year.sqrt()
    }
}

/// The modelled temporary-impact cost of running this exposure at `aum_usd`,
/// as a fraction of that AUM over the whole run.
///
/// `0.6 · σ_d · √(Q/ADV) · Q` per trading day per instrument, with `Q` the
/// instrument's average daily transacted notional scaled by `aum / base_aum`.
///
/// # Errors
/// [`Missing::NoAdv`] / [`Missing::NoVolatility`] naming the instrument that has
/// none. Capacity is not assessed on a guess, so one unmeasured instrument makes
/// the whole curve unavailable rather than quietly contributing nothing.
pub fn impact_drag(exposure: &Exposure, base_aum_usd: f64, aum_usd: f64) -> Result<f64, Missing> {
    if !(base_aum_usd.is_finite() && base_aum_usd > 0.0 && aum_usd.is_finite() && aum_usd > 0.0) {
        return Err(Missing::NoStartingEquity);
    }
    let scale = aum_usd / base_aum_usd;
    let mut total = 0.0_f64;
    let mut any_day = false;
    for e in exposure.by_instrument.values() {
        let Some(adv) = e.adv_usd else {
            return Err(Missing::NoAdv(e.instrument.clone()));
        };
        let Some(vol) = e.daily_vol else {
            return Err(Missing::NoVolatility(e.instrument.clone()));
        };
        if e.trading_days == 0 {
            continue;
        }
        any_day = true;
        let days = f64::from(e.trading_days);
        let per_day = e.traded_notional / days * scale;
        let Some(impact) = square_root_impact(per_day, adv, vol) else {
            return Err(Missing::NoAdv(e.instrument.clone()));
        };
        total += impact * per_day * days;
    }
    if !any_day {
        return Err(Missing::NoTradingDays);
    }
    Ok(total / aum_usd)
}

/// A capacity ladder, and the counted trials that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ladder {
    pub base: CapacityBase,
    pub points: Vec<CapacityPoint>,
}

impl Ladder {
    /// Build the ladder from rungs that have already been dispatched.
    ///
    /// `rungs` is `(aum_usd, the trial id the run at that size was registered
    /// under, that run's own return series)`. Passing the trial id is not
    /// ceremony: a capacity sweep whose rungs are not on the ledger is a set of
    /// uncounted looks, and INV-16 has no exception for looks taken to satisfy a
    /// gate.
    ///
    /// # Errors
    /// Anything [`impact_drag`] or [`SharpeBasis::measure`] refuses, and an
    /// empty rung list.
    pub fn from_runs(
        exposure: &Exposure,
        base_aum_usd: f64,
        single_venue: bool,
        years: f64,
        rungs: &[(f64, uuid::Uuid, Vec<f64>)],
    ) -> Result<Self, Missing> {
        if rungs.len() < 2 {
            return Err(Missing::NoSharpe);
        }
        let mut points = Vec::with_capacity(rungs.len());
        for (aum_usd, trial, returns) in rungs {
            let basis = SharpeBasis::measure(returns, years)?;
            let drag = impact_drag(exposure, base_aum_usd, *aum_usd)?;
            #[allow(clippy::cast_precision_loss)]
            let per_period = drag / basis.periods as f64;
            points.push(CapacityPoint {
                aum_usd: *aum_usd,
                net_sharpe: basis.sharpe_after(per_period),
                member_trial_id: *trial,
            });
        }
        points.sort_by(|a, b| a.aum_usd.total_cmp(&b.aum_usd));
        Ok(Self {
            base: CapacityBase {
                base_aum_usd,
                single_venue,
            },
            points,
        })
    }
}

/// Assemble Gate 4's inputs from what the ledger stored.
///
/// Separate from the gate so the gate stays arithmetic and this stays assembly,
/// and so a caller that has only part of the evidence gets `None` rather than a
/// half-filled struct with zeros in it.
#[must_use]
pub fn capacity_inputs<'a>(
    rows: &'a [InstrumentPnl],
    base: CapacityBase,
    curve: &'a [CapacityPoint],
    adv: &'a mut BTreeMap<String, f64>,
    vol: &'a mut BTreeMap<String, f64>,
    peak: &'a mut BTreeMap<String, f64>,
    sharpe_by_aum: &'a mut Vec<(f64, f64)>,
) -> Option<CapacityInputs<'a>> {
    if rows.is_empty() || curve.is_empty() {
        return None;
    }
    for r in rows {
        // An instrument with no measured ADV is *left out of the ADV map* rather
        // than given a stand-in: Gate 4 refuses when something it traded has no
        // ADV, and that refusal is the honest verdict.
        if let Some(a) = r.adv_usd {
            adv.insert(r.instrument.clone(), a);
        }
        if let Some(v) = r.daily_vol {
            vol.insert(r.instrument.clone(), v);
        }
        peak.insert(r.instrument.clone(), r.peak_daily_notional);
    }
    sharpe_by_aum.extend(curve.iter().map(|p| (p.aum_usd, p.net_sharpe)));
    Some(CapacityInputs {
        adv_usd: adv,
        daily_vol: vol,
        peak_daily_notional: peak,
        sharpe_by_aum,
        base_aum_usd: base.base_aum_usd,
        single_venue: base.single_venue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::evaluators::breakeven_multiple;
    use crate::run::{ComputeCost, MetricSet, RunId, RunStatus, Side, Trade};
    use chrono::{Duration, TimeZone, Utc};
    use rust_decimal_macros::dec;

    fn trade(symbol: &str, day: i64, qty: f64, pnl: f64, costs: f64) -> Trade {
        let t0 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap() + Duration::days(day);
        Trade {
            symbol: symbol.into(),
            side: Side::Long,
            entry_time: t0,
            exit_time: t0 + Duration::hours(6),
            entry_price: dec!(100),
            exit_price: dec!(101),
            qty: rust_decimal::Decimal::from_f64_retain(qty).unwrap(),
            mae: -0.01,
            mfe: 0.02,
            holding_period_secs: 21_600,
            costs_paid: rust_decimal::Decimal::from_f64_retain(costs).unwrap(),
            pnl: rust_decimal::Decimal::from_f64_retain(pnl).unwrap(),
        }
    }

    fn run(trades: Vec<Trade>, stats: Vec<crate::run::InstrumentStat>) -> RunResult {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        RunResult {
            run_id: RunId::from_canonical_bytes(b"capacity-test"),
            status: RunStatus::Ok,
            #[allow(clippy::cast_precision_loss)]
            equity_curve: (0..30i64)
                .map(|i| (start + Duration::days(i), 100_000.0 + i as f64 * 10.0))
                .collect(),
            net_exposure: vec![],
            trades,
            metrics: MetricSet::empty(),
            integrity_flags: vec![],
            instrument_stats: stats,
            compute_cost: ComputeCost::default(),
            produced_at: start,
            produced_by: "test".into(),
            unsafe_: false,
            manifest: None,
        }
    }

    #[test]
    fn numeraires_are_parsed_from_both_symbol_shapes() {
        assert_eq!(numeraire_of("BTC-USD").as_deref(), Some("USD"));
        assert_eq!(numeraire_of("BTC/USDT").as_deref(), Some("USDT"));
        assert_eq!(numeraire_of("BTCUSDT").as_deref(), Some("USDT"));
        assert_eq!(numeraire_of("BTCUSD").as_deref(), Some("USD"));
        assert_eq!(numeraire_of("MYSTERY"), None);
    }

    /// A symbol nobody can parse gets a unit nothing else shares, so the
    /// aggregation refuses rather than summing across units it guessed at.
    #[test]
    fn an_unparseable_symbol_does_not_join_someone_elses_numeraire() {
        let e = Exposure::from_run(&run(
            vec![
                trade("MYSTERY", 0, 10.0, 50.0, 5.0),
                trade("BTC-USD", 0, 10.0, 50.0, 5.0),
            ],
            vec![],
        ))
        .expect("has trades");
        let rows = e.instrument_rows();
        assert!(matches!(
            ledger::evidence::per_instrument_pnl(&rows),
            Err(ledger::evidence::AggregationRefused::MixedNumeraire(2, _))
        ));
    }

    #[test]
    fn exposure_totals_the_trade_list() {
        let e = Exposure::from_run(&run(
            vec![
                trade("BTC-USD", 0, 10.0, 100.0, 10.0),
                trade("BTC-USD", 1, 10.0, -40.0, 10.0),
                trade("ETH-USD", 0, 5.0, 20.0, 5.0),
            ],
            vec![],
        ))
        .expect("has trades");
        assert!((e.costs_paid - 25.0).abs() < 1e-9);
        assert!((e.gross_pnl - 105.0).abs() < 1e-9, "{}", e.gross_pnl);
        assert_eq!(e.by_instrument.len(), 2);
        assert_eq!(e.by_instrument["BTC-USD"].trading_days, 2, "two entry days");
    }

    /// The trap. A Run that paid nothing produces a ladder every rung of which
    /// earns the same metric, and `gate_3_cost_sensitivity` reads that as "the
    /// edge survives every rung tried" — a pass earned by the cost model's
    /// absence. This refuses to build it.
    #[test]
    fn a_costless_run_yields_no_ladder_rather_than_a_flat_one() {
        let e = Exposure::from_run(&run(vec![trade("BTC-USD", 0, 10.0, 100.0, 0.0)], vec![]))
            .expect("has trades");
        assert_eq!(
            cost_ladder(&e, COST_MULTIPLES),
            Err(Missing::NoCostsCharged)
        );
        assert_eq!(breakeven_cost_multiple(&e), Err(Missing::NoCostsCharged));
    }

    /// The scalar the funnel records and the shape the gate interpolates have to
    /// be the same number, or one of the two is decoration.
    #[test]
    fn the_recorded_breakeven_matches_what_the_ladder_interpolates() {
        let e = Exposure::from_run(&run(
            vec![
                trade("BTC-USD", 0, 10.0, 300.0, 100.0),
                trade("BTC-USD", 1, 10.0, 100.0, 100.0),
            ],
            vec![],
        ))
        .expect("has trades");
        // gross = 400 + 200 = 600; costs = 200 → breakeven at 3.0×.
        let scalar = breakeven_cost_multiple(&e).expect("costs were charged");
        assert!((scalar - 3.0).abs() < 1e-9, "{scalar}");

        let ladder = cost_ladder(&e, COST_MULTIPLES).expect("costs were charged");
        let rungs: Vec<crate::gates::evaluators::CostRung> = ladder
            .iter()
            .map(|r| crate::gates::evaluators::CostRung {
                multiple: r.multiple,
                net_metric: r.net_metric,
            })
            .collect();
        let interpolated = breakeven_multiple(&rungs).expect("the ladder crosses zero");
        assert!(
            (interpolated - scalar).abs() < 1e-6,
            "{interpolated} vs {scalar}"
        );
    }

    #[test]
    fn capacity_without_adv_is_refused_by_name() {
        let e = Exposure::from_run(&run(vec![trade("BTC-USD", 0, 10.0, 100.0, 10.0)], vec![]))
            .expect("has trades");
        assert_eq!(
            impact_drag(&e, 100_000.0, 100_000.0),
            Err(Missing::NoAdv("BTC-USD".into()))
        );
    }

    /// The property that makes the capacity curve mean anything: drag grows with
    /// size, so net Sharpe falls. A curve that did not would make
    /// `capacity_at_half_sharpe` return `None` for every strategy forever.
    #[test]
    fn modelled_impact_grows_with_size() {
        let e = Exposure::from_run(&run(
            vec![trade("BTC-USD", 0, 100.0, 500.0, 50.0)],
            vec![crate::run::InstrumentStat {
                instrument: "BTC-USD".into(),
                numeraire: Some("USD".into()),
                adv_usd: Some(50_000_000.0),
                daily_vol: Some(0.04),
            }],
        ))
        .expect("has trades");
        let small = impact_drag(&e, 100_000.0, 100_000.0).expect("adv measured");
        let large = impact_drag(&e, 100_000.0, 10_000_000.0).expect("adv measured");
        assert!(large > small * 5.0, "square-root impact: {small} → {large}");
    }

    /// Before ADR-P6-12 the simulator charged zero commission on every fill and
    /// called the result net. A statistic aggregated across that boundary is
    /// measuring the fix, not the strategy.
    #[test]
    fn a_statistic_is_refused_across_an_engine_boundary() {
        assert_eq!(
            one_engine(["sim@1+exits", "sim@1+exits"]).as_deref(),
            Ok("sim@1+exits")
        );
        assert!(matches!(
            one_engine(["sim@1", "sim@1+exits"]),
            Err(Missing::MixedEngineVersions(_))
        ));
        assert!(matches!(
            one_engine([]),
            Err(Missing::MixedEngineVersions(_))
        ));
    }

    #[test]
    fn a_flat_return_series_has_no_sharpe_basis() {
        assert_eq!(
            SharpeBasis::measure(&[0.001; 200], 1.0),
            Err(Missing::NoSharpe)
        );
        assert_eq!(SharpeBasis::measure(&[], 1.0), Err(Missing::NoSharpe));
    }

    #[test]
    fn the_ladder_names_the_counted_trial_behind_every_rung() {
        let e = Exposure::from_run(&run(
            vec![trade("BTC-USD", 0, 100.0, 500.0, 50.0)],
            vec![crate::run::InstrumentStat {
                instrument: "BTC-USD".into(),
                numeraire: Some("USD".into()),
                adv_usd: Some(5_000_000.0),
                daily_vol: Some(0.04),
            }],
        ))
        .expect("has trades");
        let returns: Vec<f64> = (0..300)
            .map(|i| if i % 3 == 0 { 0.004 } else { -0.001 })
            .collect();
        let rungs: Vec<(f64, uuid::Uuid, Vec<f64>)> = AUM_MULTIPLES
            .iter()
            .map(|m| (1_000_000.0 * m, uuid::Uuid::new_v4(), returns.clone()))
            .collect();
        let ladder = Ladder::from_runs(&e, 1_000_000.0, true, 1.2, &rungs).expect("measurable");
        assert_eq!(ladder.points.len(), AUM_MULTIPLES.len());
        let ids: std::collections::BTreeSet<uuid::Uuid> =
            ladder.points.iter().map(|p| p.member_trial_id).collect();
        assert_eq!(ids.len(), AUM_MULTIPLES.len(), "one counted trial per rung");
        assert!(
            ladder.points.first().unwrap().net_sharpe > ladder.points.last().unwrap().net_sharpe,
            "net Sharpe must decay with size or the gate measures nothing"
        );
    }
}
