//! Fee schedules — one definition for every venue the platform simulates
//! (Set U U-2.6).
//!
//! Until 2026-10-02 the platform priced a trade two ways. Backtests read a
//! dated maker/taker [`FeeSchedule`] from `backtest::costs::FeeBook`; the paper
//! venue's simulators carried one hard-coded taker rate per asset class. The
//! two agreed by coincidence where they agreed at all. Since backtests now
//! execute on the paper venue's core (ADR-0044), the schedule moved here so
//! both venues read one object, and a fee-tier change is one edit.

use serde::{Deserialize, Serialize};

/// Which side of the book a fill took.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Liquidity {
    /// Added liquidity: a resting order the market came to.
    Maker,
    /// Removed liquidity: an order that crossed on arrival.
    Taker,
}

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

/// The paper venue's crypto schedule: the retail standard tier of the broker
/// the paper account was originally modelled on (25 bps taker, 15 bps maker,
/// 2 bps half-spread). Paper trading runs in the app (ADR-0048); these are the
/// costs the in-app venue charges, and the backtest's `FeeBook` lists the same
/// object, so the two cannot drift.
#[must_use]
pub fn paper_crypto() -> FeeSchedule {
    FeeSchedule::new(
        "alpaca",
        "standard",
        15.0,
        25.0,
        2.0,
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap_or_default(),
    )
}
