//! Fee schedules, the tradeable frontier, and the oracle test
//! (EXTERNAL-REVIEW-2026-09 §3.4, §4; ADR-P6-02, ADR-P6-03).
//!
//! Three things live here, and they are one thing.
//!
//! ## 1. A fee is a schedule, not a rate
//!
//! [`FeeSchedule`] separates maker from taker and names the venue and tier it
//! came from. The platform previously carried one number per asset class
//! (`fee_rate: 0.0025`), which cannot express the axis that decides whether a
//! horizon is searchable at all. The algebra below is quadratic in cost: halving
//! the fee tier is worth exactly as much as doubling the information
//! coefficient. One of those is a conversation with an exchange.
//!
//! There is no `Default`. A cost model that guesses its own fees is one that
//! eventually guesses zero.
//!
//! ## 2. The tradeable frontier
//!
//! Below some holding period the expected move is smaller than the round trip,
//! and *no* signal of any quality is profitable there. That horizon is
//! computable per instrument per regime from bars we already store, and it is a
//! property of the market, not of the candidate.
//!
//! [`empirical_frontier`] measures it: the smallest horizon at which
//! `E|Δp(h)|` exceeds the round trip. [`analytic_frontier`] is the closed form
//! `t* ≈ (c / (k·IC·σ₁))²` for planning when no bars are in hand. The empirical
//! one is the one to trust — it makes no assumption about the signal.
//!
//! ## 3. The oracle test
//!
//! [`oracle`] runs a strategy that **knows the next bar's close** through the
//! same arithmetic. Two numbers come back, and they answer different questions:
//!
//! - `always_in_net_bps` is a test of *the simulator*. A perfect-foresight
//!   strategy that trades every bar must **lose money** at short horizons: it
//!   earns `E|Δp| = σ√(2/π)` and pays the round trip. If it is profitable, the
//!   cost model is broken and every result that ever passed through it is
//!   suspect. `oracle_is_unprofitable_at_one_minute` fails the build on that.
//!
//! - `selective_trade_fraction` is a test of *the horizon*. A perfect-foresight
//!   strategy that trades only when the move exceeds costs is always profitable
//!   — trivially, by trading rarely — so its P&L proves nothing. What it does
//!   measure is how rare "rarely" is. When the answer is three bars a day with
//!   perfect foresight, no real signal will do better, and the horizon should be
//!   refused before a search budget is spent inside it.
//!
//! The distinction matters: the first is a bug detector and the second is a
//! feasibility measurement, and conflating them produces a test that passes for
//! the wrong reason.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// `√(2/π)` — the mean absolute value of a zero-mean Gaussian in units of its
/// standard deviation. The constant the whole horizon argument turns on.
pub const MEAN_ABS_GAUSSIAN: f64 = 0.797_884_560_802_865_4;

/// Which side of the book the candidate assumes it trades on.
///
/// This is part of the cost model rather than a strategy setting, because a
/// candidate that assumes maker fills without a queue model is not making a
/// cheaper assumption — it is making an unverified one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStyle {
    /// Cross the spread both ways. The only style honest against OHLCV alone.
    TakerBoth,
    /// Post both ways. Requires a fill model; see [`FeeSchedule::requires_fill_model`].
    MakerBoth,
    /// Post the entry, cross the exit.
    MakerEntryTakerExit,
}

impl ExecutionStyle {
    /// Whether a claim at this style is unverifiable from OHLCV alone.
    ///
    /// A maker fill is contingent on the book coming to you, which minute bars
    /// cannot show. The verdict is not "forbidden" — it is "this number is only
    /// as good as the fill model behind it", and something has to carry that.
    #[must_use]
    pub const fn requires_fill_model(self) -> bool {
        !matches!(self, Self::TakerBoth)
    }
}

/// A venue's fee schedule at one tier, in basis points.
///
/// Both sides are recorded even when a strategy only uses one, because the
/// interesting question is almost always what the *other* side would have cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeeSchedule {
    /// Venue this schedule was read from — e.g. `"coinbase_advanced"`.
    pub venue: String,
    /// The tier's own name at that venue — e.g. `"intro"`, `"tier_3"`.
    pub tier: String,
    /// Fee paid for removing liquidity, in bps of notional.
    pub taker_bps: f64,
    /// Fee paid for adding liquidity, in bps of notional. Negative is a rebate.
    pub maker_bps: f64,
    /// Half the quoted spread, in bps. Paid on every crossing fill and on
    /// neither maker fill.
    pub half_spread_bps: f64,
    /// When this schedule was in force. A fee tier is point-in-time data: a
    /// backtest over 2024 that pays 2026 fees is a different experiment.
    pub effective_from: chrono::NaiveDate,
}

impl FeeSchedule {
    /// The round-trip cost of one entry and one exit, in bps of notional.
    #[must_use]
    pub fn round_trip_bps(&self, style: ExecutionStyle) -> f64 {
        let taker_leg = self.taker_bps + self.half_spread_bps;
        let maker_leg = self.maker_bps;
        match style {
            ExecutionStyle::TakerBoth => 2.0 * taker_leg,
            ExecutionStyle::MakerBoth => 2.0 * maker_leg,
            ExecutionStyle::MakerEntryTakerExit => maker_leg + taker_leg,
        }
    }

    /// A taker-both schedule from the two numbers a venue's fee page shows.
    #[must_use]
    pub fn new(
        venue: impl Into<String>,
        tier: impl Into<String>,
        maker_bps: f64,
        taker_bps: f64,
        half_spread_bps: f64,
        effective_from: chrono::NaiveDate,
    ) -> Self {
        Self {
            venue: venue.into(),
            tier: tier.into(),
            maker_bps,
            taker_bps,
            half_spread_bps,
            effective_from,
        }
    }
}

/// The fee schedules the platform knows about.
///
/// A book rather than a constant because a fee tier is point-in-time data: a
/// backtest over 2024 that pays 2026 fees is a different experiment, and the
/// only way to notice is for the schedule to be a looked-up object with an
/// `effective_from` rather than a number compiled into the simulator.
///
/// [`FeeBook::resolve`] never returns "free". When it does not recognise a venue
/// it returns the most expensive schedule it holds and says so through
/// [`FeeSchedule::tier`], because the direction of a cost error always flatters
/// the candidate: a strategy priced too cheaply gets promoted, and one priced
/// too dearly gets rejected and re-examined.
#[derive(Clone, Debug)]
pub struct FeeBook {
    schedules: Vec<FeeSchedule>,
}

impl FeeBook {
    /// The schedules shipped with the platform.
    ///
    /// Retail-tier crypto spot, which is where every account starts. These are
    /// published maker/taker rates, not negotiated ones; an account with a
    /// better tier should carry its own book.
    #[must_use]
    pub fn platform_default() -> Self {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default();
        Self {
            schedules: vec![
                // Coinbase Advanced, entry tier. The default for this platform.
                FeeSchedule::new("coinbase_advanced", "intro", 40.0, 60.0, 1.0, d),
                FeeSchedule::new("coinbase_advanced", "tier_3", 6.0, 9.0, 1.0, d),
                // Kraken Pro, entry tier.
                FeeSchedule::new("kraken", "starter", 25.0, 40.0, 1.0, d),
                // Alpaca crypto — the paper venue's model.
                FeeSchedule::new("alpaca", "standard", 15.0, 25.0, 2.0, d),
            ],
        }
    }

    /// Build a book from explicit schedules.
    #[must_use]
    pub fn new(schedules: Vec<FeeSchedule>) -> Self {
        Self { schedules }
    }

    /// The schedule for a venue and tier.
    ///
    /// Unknown venue or tier falls back to the **most expensive** schedule in
    /// the book, whose `tier` then reads `<name>/unknown-fallback` so that a
    /// result priced against a guess says so on its face.
    #[must_use]
    pub fn resolve(&self, venue: Option<&str>, tier: Option<&str>) -> FeeSchedule {
        if let (Some(v), Some(t)) = (venue, tier) {
            if let Some(s) = self
                .schedules
                .iter()
                .find(|s| s.venue == v && s.tier == t)
            {
                return s.clone();
            }
        }
        if let Some(v) = venue {
            if let Some(s) = self
                .schedules
                .iter()
                .filter(|s| s.venue == v)
                .max_by(|a, b| a.taker_bps.total_cmp(&b.taker_bps))
            {
                let mut s = s.clone();
                s.tier = format!("{}/unknown-fallback", s.tier);
                return s;
            }
        }
        let mut s = self
            .schedules
            .iter()
            .max_by(|a, b| a.taker_bps.total_cmp(&b.taker_bps))
            .cloned()
            .unwrap_or_else(|| {
                FeeSchedule::new(
                    "unknown",
                    "no-book",
                    50.0,
                    100.0,
                    5.0,
                    chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
                )
            });
        s.tier = format!("{}/unknown-fallback", s.tier);
        s
    }

    /// Every schedule in the book.
    #[must_use]
    pub fn schedules(&self) -> &[FeeSchedule] {
        &self.schedules
    }
}

// ── the tradeable frontier ───────────────────────────────────────────────────

/// What a frontier measurement found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frontier {
    /// The shortest holding period, in bars, at which the expected absolute
    /// move exceeds the round trip. `None` means it never did over the horizons
    /// tried — which is a statement about the horizons tried, not about
    /// infinity, and callers must not read it as "unreachable".
    pub breakeven_bars: Option<usize>,
    /// `(horizon_bars, mean_abs_move_bps)` for every horizon measured. Kept so
    /// the UI can show the curve rather than the crossing: an edge that clears
    /// the round trip by a hair at h=60 is a different object from one that
    /// clears it threefold.
    pub curve: Vec<(usize, f64)>,
    /// The round trip the curve was compared against.
    pub round_trip_bps: f64,
    /// Number of bars the measurement consumed.
    pub sample_bars: usize,
}

impl Frontier {
    /// Whether a candidate holding `h` bars is even worth evaluating.
    ///
    /// Below the frontier the expected move is smaller than the cost of
    /// capturing it, so a positive backtest there is a statement about the
    /// backtest. This is deliberately a *pre-trial* question: refusing here
    /// costs nothing, and refusing at Gate 3 costs a trial.
    #[must_use]
    pub fn admits(&self, holding_bars: usize) -> Admission {
        match self.breakeven_bars {
            _ if holding_bars == 0 => Admission::Refused {
                reason: "a holding period of zero bars cannot pay a round trip".into(),
            },
            Some(b) if holding_bars < b => Admission::Refused {
                reason: format!(
                    "holding {holding_bars} bars sits below the measured tradeable frontier of \
                     {b} bars for a {:.1} bp round trip: the expected absolute move over \
                     {holding_bars} bars is {:.2} bp, so no signal of any quality is profitable \
                     at this horizon",
                    self.round_trip_bps,
                    self.curve
                        .iter()
                        .find(|(h, _)| *h == holding_bars)
                        .map_or(f64::NAN, |(_, m)| *m),
                ),
            },
            Some(_) => Admission::Admitted,
            None => Admission::Refused {
                reason: format!(
                    "the expected absolute move never reached the {:.1} bp round trip over any \
                     horizon measured (longest tried: {} bars); the frontier is beyond the \
                     measured range rather than known",
                    self.round_trip_bps,
                    self.curve.last().map_or(0, |(h, _)| *h),
                ),
            },
        }
    }
}

/// The answer to "may this candidate consume a trial".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Admission {
    Admitted,
    Refused { reason: String },
}

impl Admission {
    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }
}

/// Measure the tradeable frontier from closes: `E|p(t+h) − p(t)| / p(t)` in bps,
/// for each `h` in `horizons`, against the round trip.
///
/// Uses overlapping windows, which inflates the effective sample but not the
/// estimate — the mean absolute move is unbiased under overlap, only its
/// standard error is understated, and nothing here reads a standard error.
///
/// Returns `None` when there are too few closes to measure the shortest horizon
/// asked for, rather than returning a frontier computed from three bars.
#[must_use]
pub fn empirical_frontier(
    closes: &[f64],
    horizons: &[usize],
    round_trip_bps: f64,
) -> Option<Frontier> {
    let shortest = horizons.iter().copied().min()?;
    if shortest == 0 || closes.len() <= shortest + 1 {
        return None;
    }

    let mut sorted: Vec<usize> = horizons.iter().copied().filter(|h| *h > 0).collect();
    sorted.sort_unstable();
    sorted.dedup();

    let mut curve = Vec::with_capacity(sorted.len());
    for h in sorted {
        if closes.len() <= h {
            continue;
        }
        let mut total = 0.0;
        let mut n = 0usize;
        for w in 0..closes.len() - h {
            let (a, b) = (closes[w], closes[w + h]);
            if a > 0.0 && a.is_finite() && b.is_finite() {
                total += ((b - a) / a).abs();
                n += 1;
            }
        }
        if n > 0 {
            curve.push((h, total / n as f64 * 10_000.0));
        }
    }

    let breakeven_bars = curve
        .iter()
        .find(|(_, mean_abs)| *mean_abs > round_trip_bps)
        .map(|(h, _)| *h);

    Some(Frontier {
        breakeven_bars,
        curve,
        round_trip_bps,
        sample_bars: closes.len(),
    })
}

/// The closed-form frontier `t* ≈ (c / (k·IC·σ₁))²`, in units of the bar `σ₁`
/// was measured on.
///
/// For planning only. It assumes a signal with information coefficient `IC`
/// against the horizon's return, `√t` volatility scaling, and a selectivity
/// factor `k` for how much of the distribution a selective entry captures
/// (≈1.5 for a moderately selective rule). Every one of those is an assumption
/// [`empirical_frontier`] does not make, which is why this is the fallback.
///
/// `None` when any input is non-positive or non-finite — there is no sensible
/// frontier for a signal with no information.
#[must_use]
pub fn analytic_frontier(
    round_trip_bps: f64,
    ic: f64,
    sigma_bar_bps: f64,
    selectivity: f64,
) -> Option<f64> {
    let ok = round_trip_bps > 0.0
        && ic > 0.0
        && sigma_bar_bps > 0.0
        && selectivity > 0.0
        && [round_trip_bps, ic, sigma_bar_bps, selectivity]
            .iter()
            .all(|x| x.is_finite());
    ok.then(|| (round_trip_bps / (selectivity * ic * sigma_bar_bps)).powi(2))
}

// ── the oracle ───────────────────────────────────────────────────────────────

/// What a perfect-foresight run found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Oracle {
    /// Net bps per bar for a strategy that knows the next close and is always
    /// in the market. **This must be negative at short horizons.** It is the
    /// cost model's own test: a perfect-foresight strategy trading every bar
    /// earns the mean absolute move and pays the round trip, so a positive
    /// number here means the simulator is not charging what it claims.
    pub always_in_net_bps: f64,
    /// Gross bps per bar before costs: the mean absolute move. The most an
    /// always-in strategy can possibly earn at this horizon.
    pub always_in_gross_bps: f64,
    /// Net bps per bar for a strategy that knows the next close and trades only
    /// when the move exceeds the round trip. Always non-negative by
    /// construction, which is exactly why it proves nothing on its own.
    pub selective_net_bps: f64,
    /// Fraction of bars the selective oracle traded. This is the number that
    /// matters: with perfect foresight it is the ceiling on how often *any*
    /// strategy can profitably trade at this horizon.
    pub selective_trade_fraction: f64,
    /// The round trip charged.
    pub round_trip_bps: f64,
    /// Bars the measurement consumed.
    pub sample_bars: usize,
}

impl Oracle {
    /// Whether the cost model passes its own sanity test at this horizon.
    ///
    /// Reads as a question about the simulator, not the candidate: a profitable
    /// always-in oracle means costs are being under-charged somewhere between
    /// the fee schedule and the fill.
    #[must_use]
    pub const fn cost_model_is_sane(&self) -> bool {
        self.always_in_net_bps < 0.0
    }
}

/// Run a perfect-foresight strategy over `closes` at `round_trip_bps`.
///
/// The always-in arm takes the sign of every next-bar return and pays a round
/// trip on every bar (a sign flip is a round trip; holding through a same-sign
/// bar is not, which makes this *generous* to the oracle rather than harsh).
/// The selective arm trades only the bars whose absolute move clears the round
/// trip.
///
/// `None` when fewer than two usable closes are supplied.
#[must_use]
pub fn oracle(closes: &[f64], round_trip_bps: f64) -> Option<Oracle> {
    if closes.len() < 2 {
        return None;
    }

    let returns: Vec<f64> = closes
        .windows(2)
        .filter(|w| w[0] > 0.0 && w[0].is_finite() && w[1].is_finite())
        .map(|w| (w[1] - w[0]) / w[0] * 10_000.0)
        .collect();
    if returns.is_empty() {
        return None;
    }
    let n = returns.len() as f64;

    let gross: f64 = returns.iter().map(|r| r.abs()).sum::<f64>() / n;

    // A sign flip between consecutive bars is a round trip. The first bar is an
    // entry, so it costs one too.
    let flips = 1 + returns
        .windows(2)
        .filter(|w| w[0].signum() != w[1].signum())
        .count();
    let always_in_net = gross - round_trip_bps * flips as f64 / n;

    let traded: Vec<f64> = returns
        .iter()
        .copied()
        .filter(|r| r.abs() > round_trip_bps)
        .collect();
    let selective_net = traded.iter().map(|r| r.abs() - round_trip_bps).sum::<f64>() / n;

    Some(Oracle {
        always_in_net_bps: always_in_net,
        always_in_gross_bps: gross,
        selective_net_bps: selective_net,
        selective_trade_fraction: traded.len() as f64 / n,
        round_trip_bps,
        sample_bars: closes.len(),
    })
}

/// The mean absolute move of a zero-mean Gaussian with per-bar volatility
/// `sigma_bps`: `σ√(2/π)`. The analytic counterpart of
/// [`Oracle::always_in_gross_bps`], used to check a measured series against the
/// number the algebra predicts.
#[must_use]
pub fn expected_abs_move_bps(sigma_bps: f64) -> f64 {
    sigma_bps * (2.0 / PI).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    }

    /// Coinbase Advanced intro tier, roughly: 60 bps taker, 40 bps maker, and a
    /// bp of half-spread on a liquid major.
    fn coinbase_intro() -> FeeSchedule {
        FeeSchedule::new("coinbase_advanced", "intro", 40.0, 60.0, 1.0, date())
    }

    /// The tier the memo's 20 bp round trip assumes: 9 bp taker, 1 bp spread.
    fn tier_3() -> FeeSchedule {
        FeeSchedule::new("coinbase_advanced", "tier_3", 6.0, 9.0, 1.0, date())
    }

    /// A deterministic pseudo-Gaussian walk with a known per-bar volatility.
    /// Box–Muller over a fixed LCG: the point is reproducibility, not entropy.
    fn walk(n: usize, sigma_bps: f64, seed: u64) -> Vec<f64> {
        let mut s = seed | 1;
        let mut next = || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((s >> 11) as f64) / ((1u64 << 53) as f64)
        };
        let mut closes = Vec::with_capacity(n);
        let mut p = 50_000.0f64;
        for _ in 0..n {
            let (u1, u2) = (next().max(1e-12), next());
            let z = (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos();
            p *= 1.0 + z * sigma_bps / 10_000.0;
            closes.push(p);
        }
        closes
    }

    // ── fee schedules ────────────────────────────────────────────────────────

    #[test]
    fn taker_round_trip_charges_both_legs_and_both_spreads() {
        let f = tier_3();
        assert!((f.round_trip_bps(ExecutionStyle::TakerBoth) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn a_maker_rebate_can_make_the_round_trip_negative() {
        let f = FeeSchedule::new("v", "maker_rebate", -2.0, 5.0, 1.0, date());
        assert!(f.round_trip_bps(ExecutionStyle::MakerBoth) < 0.0);
    }

    #[test]
    fn maker_legs_pay_no_spread_and_taker_legs_do() {
        let f = tier_3();
        let mixed = f.round_trip_bps(ExecutionStyle::MakerEntryTakerExit);
        assert!((mixed - (6.0 + 10.0)).abs() < 1e-9, "got {mixed}");
    }

    #[test]
    fn every_style_but_taker_needs_a_fill_model() {
        assert!(!ExecutionStyle::TakerBoth.requires_fill_model());
        assert!(ExecutionStyle::MakerBoth.requires_fill_model());
        assert!(ExecutionStyle::MakerEntryTakerExit.requires_fill_model());
    }

    // ── the fee book ─────────────────────────────────────────────────────────

    #[test]
    fn a_known_venue_and_tier_resolve_exactly() {
        let b = FeeBook::platform_default();
        let s = b.resolve(Some("coinbase_advanced"), Some("tier_3"));
        assert_eq!(s.tier, "tier_3");
        assert!((s.taker_bps - 9.0).abs() < 1e-9);
    }

    /// The direction of a cost error always flatters the candidate, so an
    /// unknown tier resolves to the most expensive one rather than the cheapest
    /// or to zero.
    #[test]
    fn an_unknown_tier_falls_back_to_the_most_expensive_schedule_and_says_so() {
        let b = FeeBook::platform_default();
        let s = b.resolve(Some("coinbase_advanced"), Some("whale"));
        assert!(s.tier.ends_with("/unknown-fallback"), "{}", s.tier);
        assert!((s.taker_bps - 60.0).abs() < 1e-9, "the dearer of the two");
    }

    #[test]
    fn an_unknown_venue_falls_back_across_the_whole_book() {
        let b = FeeBook::platform_default();
        let s = b.resolve(Some("nowhere"), None);
        assert!(s.tier.ends_with("/unknown-fallback"));
        assert!(s.taker_bps >= 40.0);
    }

    #[test]
    fn resolving_from_an_empty_book_is_expensive_rather_than_free() {
        let s = FeeBook::new(vec![]).resolve(None, None);
        assert!(s.taker_bps > 0.0, "a cost model that defaults to free reports free strategies");
        assert!(s.tier.contains("unknown-fallback"));
    }

    #[test]
    fn the_default_book_holds_a_schedule_for_every_venue_the_platform_paper_trades() {
        let b = FeeBook::platform_default();
        for venue in ["coinbase_advanced", "kraken", "alpaca"] {
            assert!(b.schedules().iter().any(|s| s.venue == venue), "{venue}");
        }
    }

    // ── the oracle test: this is the one that matters ────────────────────────

    /// **The cost-model sanity test.** A strategy that knows the next minute's
    /// close exactly and trades every minute must lose money at a realistic
    /// crypto taker round trip. If this ever passes a profit, the cost model is
    /// under-charging and every backtest that used it is suspect.
    #[test]
    fn oracle_is_unprofitable_at_one_minute() {
        let closes = walk(20_000, 7.0, 0x00C0_FFEE);
        let rt = tier_3().round_trip_bps(ExecutionStyle::TakerBoth);
        let o = oracle(&closes, rt).expect("enough bars");

        assert!(
            o.cost_model_is_sane(),
            "a perfect-foresight strategy netted {:+.2} bp/bar at a {rt:.1} bp round trip. \
             It earns the mean absolute move ({:.2} bp) and pays the round trip on every flip; \
             a profit here means the cost model is not charging what it says it charges.",
            o.always_in_net_bps,
            o.always_in_gross_bps,
        );
    }

    /// The same test at the retail tier, where the gap is not close.
    #[test]
    fn oracle_is_hopeless_at_the_intro_tier() {
        let closes = walk(20_000, 7.0, 7);
        let rt = coinbase_intro().round_trip_bps(ExecutionStyle::TakerBoth);
        let o = oracle(&closes, rt).unwrap();
        assert!(o.always_in_net_bps < -50.0, "got {:+.2}", o.always_in_net_bps);
    }

    /// The test is not vacuous: give the oracle a long enough horizon and it
    /// wins. A test that can only fail is not measuring anything.
    #[test]
    fn the_oracle_becomes_profitable_once_the_move_outgrows_the_round_trip() {
        // 200 bp per-bar vol stands in for a multi-day bar.
        let closes = walk(20_000, 200.0, 99);
        let rt = tier_3().round_trip_bps(ExecutionStyle::TakerBoth);
        let o = oracle(&closes, rt).unwrap();
        assert!(
            o.always_in_net_bps > 0.0,
            "oracle should win when E|Δp| ≫ round trip, got {:+.2}",
            o.always_in_net_bps
        );
    }

    /// The gross figure must match `σ√(2/π)` — the whole horizon argument rests
    /// on that identity, so it is checked rather than assumed.
    #[test]
    fn gross_matches_the_mean_absolute_gaussian() {
        let closes = walk(60_000, 7.0, 1234);
        let o = oracle(&closes, 20.0).unwrap();
        let predicted = expected_abs_move_bps(7.0);
        assert!(
            (o.always_in_gross_bps - predicted).abs() < 0.15,
            "measured {:.3} bp, algebra says {predicted:.3} bp",
            o.always_in_gross_bps
        );
    }

    /// The selective arm is always non-negative — which is exactly why its P&L
    /// is not evidence. Pinned so nobody later reads it as one.
    #[test]
    fn the_selective_oracle_is_profitable_by_construction_and_therefore_proves_nothing() {
        let closes = walk(20_000, 7.0, 555);
        let o = oracle(&closes, 20.0).unwrap();
        assert!(o.selective_net_bps >= 0.0);
        assert!(
            o.selective_trade_fraction < 0.02,
            "at σ=7bp against a 20bp round trip a selective oracle should clear the cost on \
             well under 2% of bars; got {:.4}",
            o.selective_trade_fraction
        );
    }

    #[test]
    fn oracle_refuses_to_answer_from_one_bar() {
        assert!(oracle(&[1.0], 20.0).is_none());
        assert!(oracle(&[], 20.0).is_none());
    }

    // ── the frontier ─────────────────────────────────────────────────────────

    #[test]
    fn the_frontier_is_where_the_mean_absolute_move_passes_the_round_trip() {
        let closes = walk(40_000, 7.0, 424_242);
        let horizons: Vec<usize> = vec![1, 5, 15, 30, 60, 120, 240, 480, 1440];
        let f = empirical_frontier(&closes, &horizons, 20.0).unwrap();

        let b = f.breakeven_bars.expect("a √t walk crosses 20bp well inside a day");
        // σ√h·√(2/π) > 20 ⟹ h > (20/(7·0.7979))² ≈ 12.8 bars.
        assert!((5..=30).contains(&b), "frontier at {b} bars, expected ~15");
        assert!(f.curve.windows(2).all(|w| w[0].1 <= w[1].1), "E|Δp| must grow with horizon");
    }

    #[test]
    fn the_frontier_refuses_horizons_below_it_and_says_why() {
        let closes = walk(40_000, 7.0, 31_337);
        let f = empirical_frontier(&closes, &[1, 5, 15, 60, 240], 20.0).unwrap();
        match f.admits(1) {
            Admission::Refused { reason } => {
                assert!(reason.contains("tradeable frontier"), "{reason}");
                assert!(reason.contains("20.0 bp round trip"), "{reason}");
            }
            Admission::Admitted => panic!("one bar must not be admitted at a 20bp round trip"),
        }
        assert!(f.admits(240).is_admitted());
    }

    #[test]
    fn a_zero_bar_holding_period_is_always_refused() {
        let closes = walk(5_000, 7.0, 6);
        let f = empirical_frontier(&closes, &[1, 60], 20.0).unwrap();
        assert!(!f.admits(0).is_admitted());
    }

    /// When the curve never crosses, the verdict says "beyond the measured
    /// range" rather than silently admitting. An unmeasured frontier is not an
    /// infinite one.
    #[test]
    fn a_frontier_that_never_crosses_refuses_rather_than_admitting() {
        let closes = walk(5_000, 7.0, 8);
        let f = empirical_frontier(&closes, &[1, 2, 3], 10_000.0).unwrap();
        assert_eq!(f.breakeven_bars, None);
        match f.admits(3) {
            Admission::Refused { reason } => assert!(reason.contains("beyond the measured range")),
            Admission::Admitted => panic!("must not admit against an unmeasured frontier"),
        }
    }

    #[test]
    fn the_frontier_refuses_to_answer_from_too_few_bars() {
        assert!(empirical_frontier(&[1.0, 2.0], &[60], 20.0).is_none());
        assert!(empirical_frontier(&[1.0; 100], &[0], 20.0).is_none());
    }

    // ── the analytic form ────────────────────────────────────────────────────

    /// The table from the review: at c=20bp, σ₁=7bp, k=1.5, an IC of 0.05 puts
    /// break-even around a day of minutes and IC 0.10 around a quarter of that.
    #[test]
    fn the_analytic_frontier_reproduces_the_reviews_table() {
        let t_05 = analytic_frontier(20.0, 0.05, 7.0, 1.5).unwrap();
        let t_10 = analytic_frontier(20.0, 0.10, 7.0, 1.5).unwrap();
        assert!((1_000.0..=2_000.0).contains(&t_05), "IC 0.05 → {t_05:.0} bars");
        assert!((t_05 / t_10 - 4.0).abs() < 1e-6, "halving IC must quadruple the frontier");
    }

    /// Cost and signal quality are symmetric and quadratic. This is the
    /// business consequence of the algebra and it is worth a test.
    #[test]
    fn halving_the_fee_is_worth_exactly_as_much_as_doubling_the_ic() {
        let half_cost = analytic_frontier(10.0, 0.05, 7.0, 1.5).unwrap();
        let double_ic = analytic_frontier(20.0, 0.10, 7.0, 1.5).unwrap();
        assert!((half_cost - double_ic).abs() < 1e-9);
    }

    #[test]
    fn a_signal_with_no_information_has_no_frontier() {
        assert!(analytic_frontier(20.0, 0.0, 7.0, 1.5).is_none());
        assert!(analytic_frontier(20.0, f64::NAN, 7.0, 1.5).is_none());
        assert!(analytic_frontier(-1.0, 0.05, 7.0, 1.5).is_none());
    }
}
