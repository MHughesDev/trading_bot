//! Forward evidence: the realised return series of a deployed strategy, built
//! in **the same shape** as the series the ledger stored for its backtest.
//!
//! ## Same shape is the whole job
//!
//! A divergence figure is only worth having if the two series it subtracts were
//! measured the same way. Two *nearly* comparable series produce a number that
//! looks authoritative and means nothing, and a wrong Gate 15 verdict is worse
//! than an absent one: an absent one is honest about what nobody knows.
//!
//! So "same shape" is not a convention here, it is four enforced properties:
//!
//! | property | how it is enforced |
//! |---|---|
//! | same type | [`RealisedSeries`] wraps [`ledger::neff::ReturnSeries`] — the very type `persist_returns` stores. There is no parallel struct to drift from it. |
//! | same bar alignment | [`RealisedSeriesBuilder::open`] and [`RealisedSeriesBuilder::close_bar`] refuse a bar that is not a multiple of the deployment's `bar_seconds`, and [`RealisedSeries::align`] refuses two series of different bar widths. |
//! | same numeraire | every series carries a [`Numeraire`] and [`RealisedSeries::align`] calls [`Numeraire::assert_comparable`]. USDT is not summed as USD. |
//! | same cost treatment | fills are applied **net of fees**: the fee leaves cash at the moment of the fill, so realised equity is after-cost, like the pessimistic-cost backtest series it is compared against. |
//!
//! ## What this module deliberately does not do
//!
//! It does not convert between numeraires, and it does not fill a missing bar.
//! A realised bar with no backtest counterpart is a *finding* — the live system
//! traded where the tested one would not have — and dropping it quietly is how
//! that finding disappears.

use chrono::{DateTime, Utc};
use domain::forward::{ForwardObservation, ObservationSource};
use domain::money::Price;
use domain::numeraire::{Numeraire, NumeraireMismatch};
use domain::order::Side;
use ledger::neff::ReturnSeries;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

/// The venues a Set T deployment may name.
///
/// One variant. Not a placeholder — the set's hard boundary is that nothing it
/// builds creates a live-venue code path, and an enum with no live variant is
/// that sentence written where the compiler can read it. Adding one is a source
/// change, a review, and roadmap gate H-4, rather than a string that happened to
/// be typed into a config file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperVenue {
    /// Alpaca's paper endpoint (ADR-0006, Q-1).
    AlpacaPaper,
}

impl PaperVenue {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AlpacaPaper => "alpaca_paper",
        }
    }

    /// Always true, by construction. Kept as a method because the entry-point
    /// tests assert on it, and an assertion that reads `venue.is_paper()` keeps
    /// its meaning if the enum ever grows.
    #[must_use]
    pub fn is_paper(self) -> bool {
        matches!(self, Self::AlpacaPaper)
    }
}

/// The per-strategy divergence thresholds, declared at deployment and recorded
/// (§12.6: a stopping rule chosen after watching the result is not a stopping
/// rule).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DivergenceThresholds {
    /// Largest acceptable |realised − expected| on a single bar.
    pub max_abs_return_divergence: f64,
    /// Largest acceptable |cumulative realised − cumulative expected| over the
    /// deployment so far. A per-bar threshold alone misses the failure that
    /// matters most: a small bias, every bar, in the same direction.
    pub max_cum_return_divergence: f64,
    /// Largest acceptable absolute difference between the internal position and
    /// the broker's (FM-4).
    pub max_position_divergence: Decimal,
    /// Smallest acceptable fraction of bars whose live signal reproduced the
    /// backtest's.
    pub min_signal_match: f64,
}

impl DivergenceThresholds {
    /// # Errors
    /// A non-finite, non-positive, or out-of-range threshold. A threshold of
    /// zero or infinity is a threshold that never fires, and a detector that
    /// never fires is indistinguishable from one that is not wired up.
    pub fn validate(&self) -> Result<(), ForwardError> {
        for (name, v) in [
            ("max_abs_return_divergence", self.max_abs_return_divergence),
            ("max_cum_return_divergence", self.max_cum_return_divergence),
        ] {
            if !v.is_finite() || v <= 0.0 {
                return Err(ForwardError::UndeclaredThreshold(name));
            }
        }
        if self.max_position_divergence < Decimal::ZERO {
            return Err(ForwardError::UndeclaredThreshold("max_position_divergence"));
        }
        if !self.min_signal_match.is_finite()
            || self.min_signal_match <= 0.0
            || self.min_signal_match > 1.0
        {
            return Err(ForwardError::UndeclaredThreshold("min_signal_match"));
        }
        Ok(())
    }
}

/// Everything about a deployment that the realised series has to agree with.
#[derive(Clone, Debug)]
pub struct DeploymentSpec {
    pub deployment_id: Uuid,
    /// As it appears on `OrderIntent.strategy_id` — the join between an order
    /// and its divergence halt.
    pub strategy_id: String,
    /// The trial whose stored return series this is compared against.
    pub trial_id: Uuid,
    pub venue: PaperVenue,
    pub numeraire: Numeraire,
    /// Bar width in seconds. Both series are on this grid or they are not
    /// comparable.
    pub bar_seconds: i64,
    pub thresholds: DivergenceThresholds,
}

impl DeploymentSpec {
    /// # Errors
    /// A non-positive bar width or an undeclared threshold.
    pub fn validate(&self) -> Result<(), ForwardError> {
        if self.bar_seconds <= 0 {
            return Err(ForwardError::BadBarWidth(self.bar_seconds));
        }
        self.thresholds.validate()
    }
}

/// What the backtest says should have happened over one bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExpectedBar {
    pub expected_return: f64,
    /// Notional the backtest expected to turn over, in the deployment's
    /// numeraire.
    pub expected_turnover: Decimal,
    /// What the simulator charged for execution, in basis points.
    pub modelled_slippage_bps: f64,
    /// Whether the live signal reproduced the backtest's for this bar.
    pub signal_matched: bool,
}

/// Anything that stops a realised series from being comparable.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum ForwardError {
    #[error("bar width must be positive, got {0}s")]
    BadBarWidth(i64),
    #[error("divergence threshold {0} was not declared, or is not a threshold that can fire")]
    UndeclaredThreshold(&'static str),
    #[error(
        "bar close {bar} is not aligned to this deployment's {bar_seconds}s grid — \
         a realised series on a different grid is not element-wise comparable with the backtest's"
    )]
    MisalignedBar {
        bar: DateTime<Utc>,
        bar_seconds: i64,
    },
    #[error("bar close {bar} is not after the previous close {previous}")]
    BarOutOfOrder {
        bar: DateTime<Utc>,
        previous: DateTime<Utc>,
    },
    #[error("equity is not a finite positive number at bar {0}; the return chain stops here")]
    DegenerateEquity(DateTime<Utc>),
    #[error(
        "no opening mark has been taken for this deployment — the first bar has no previous \
         equity to return against, and recording it as a 0.0 return would put a bar the \
         strategy did not trade into Gate 15's count"
    )]
    NotOpened,
    #[error("this deployment already has an opening mark at {0}")]
    AlreadyOpened(DateTime<Utc>),
}

/// A return series that knows what it is counted in and what grid it is on.
///
/// The inner series is [`ledger::neff::ReturnSeries`] — the type the ledger
/// persists for a backtest — so a realised series and a backtest series are
/// literally the same type and cannot drift apart by a field being added to one.
#[derive(Clone, Debug, PartialEq)]
pub struct RealisedSeries {
    pub numeraire: Numeraire,
    pub bar_seconds: i64,
    pub series: ReturnSeries,
}

/// One bar where the two series meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlignedBar {
    pub bar: DateTime<Utc>,
    pub realised: f64,
    pub expected: f64,
}

impl AlignedBar {
    #[must_use]
    pub fn divergence(&self) -> f64 {
        self.realised - self.expected
    }
}

/// Why two series could not be placed beside each other.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum AlignmentError {
    #[error(transparent)]
    Numeraire(#[from] NumeraireMismatch),
    #[error("bar widths differ: realised {realised}s, backtest {expected}s")]
    BarWidth { realised: i64, expected: i64 },
    #[error(
        "the series do not line up: realised has {realised} bars, backtest {expected} — \
         a truncated comparison would silently drop the bars that disagree most"
    )]
    Length { realised: usize, expected: usize },
    #[error("bar {index} differs: realised {realised}, backtest {expected}")]
    Bar {
        index: usize,
        realised: DateTime<Utc>,
        expected: DateTime<Utc>,
    },
}

impl RealisedSeries {
    /// Place this series element-wise beside a backtest series.
    ///
    /// Strict on purpose. Every looser join — inner join on common bars, zip to
    /// the shorter, nearest-timestamp match — silently discards exactly the
    /// bars where the two disagree about whether a trade happened, which are
    /// the bars Gate 15 exists to look at.
    ///
    /// # Errors
    /// [`AlignmentError`] on a numeraire, grid, length or bar mismatch.
    pub fn align(&self, expected: &RealisedSeries) -> Result<Vec<AlignedBar>, AlignmentError> {
        self.numeraire.assert_comparable(expected.numeraire)?;
        if self.bar_seconds != expected.bar_seconds {
            return Err(AlignmentError::BarWidth {
                realised: self.bar_seconds,
                expected: expected.bar_seconds,
            });
        }
        if self.series.returns.len() != expected.series.returns.len() {
            return Err(AlignmentError::Length {
                realised: self.series.returns.len(),
                expected: expected.series.returns.len(),
            });
        }
        let mut out = Vec::with_capacity(self.series.returns.len());
        for i in 0..self.series.returns.len() {
            let (lt, rt) = (self.series.timestamps[i], expected.series.timestamps[i]);
            if lt != rt {
                return Err(AlignmentError::Bar {
                    index: i,
                    realised: lt,
                    expected: rt,
                });
            }
            out.push(AlignedBar {
                bar: lt,
                realised: self.series.returns[i],
                expected: expected.series.returns[i],
            });
        }
        Ok(out)
    }
}

/// What one closed bar realised, before it becomes a database row.
#[derive(Clone, Debug, PartialEq)]
pub struct BarClose {
    pub bar: DateTime<Utc>,
    pub realised_return: f64,
    pub realised_slippage_bps: f64,
    pub realised_turnover: Decimal,
    pub model_attributable_rejects: u32,
}

/// Builds the realised series from paper fills, one bar at a time.
///
/// Money stays in `Decimal` throughout (SC-1); the single conversion to `f64`
/// happens where a *ratio* is formed, which is what a return is.
#[derive(Debug)]
pub struct RealisedSeriesBuilder {
    spec: DeploymentSpec,
    /// Signed position: positive long, negative short.
    position: Decimal,
    cash: Decimal,
    equity_points: Vec<(DateTime<Utc>, f64)>,
    last_bar: Option<DateTime<Utc>>,
    // ── accumulators for the bar currently open ──
    bar_turnover: Decimal,
    /// Σ(slippage_bps × notional) for the open bar, so the bar's slippage is
    /// notional-weighted rather than a mean over unequal trades.
    slippage_weighted: Decimal,
    rejects: u32,
}

impl RealisedSeriesBuilder {
    /// # Errors
    /// An invalid [`DeploymentSpec`], or a non-positive opening balance — a
    /// return series cannot start from zero equity.
    pub fn new(spec: DeploymentSpec, opening_balance: Decimal) -> Result<Self, ForwardError> {
        spec.validate()?;
        if opening_balance <= Decimal::ZERO {
            return Err(ForwardError::DegenerateEquity(DateTime::<Utc>::MIN_UTC));
        }
        Ok(Self {
            spec,
            position: Decimal::ZERO,
            cash: opening_balance,
            equity_points: Vec::new(),
            last_bar: None,
            bar_turnover: Decimal::ZERO,
            slippage_weighted: Decimal::ZERO,
            rejects: 0,
        })
    }

    #[must_use]
    pub fn spec(&self) -> &DeploymentSpec {
        &self.spec
    }

    /// The internal signed position, for reconciliation against the broker.
    #[must_use]
    pub fn position(&self) -> Decimal {
        self.position
    }

    /// Apply one paper fill.
    ///
    /// `decision_price` is the mark the strategy saw when it decided — realised
    /// slippage is measured against that, not against the fill, because
    /// measuring a fill against itself is always zero.
    ///
    /// The fee leaves cash here, which is what makes the realised series
    /// after-cost and therefore comparable with the pessimistic-cost backtest
    /// series.
    pub fn on_fill(
        &mut self,
        side: Side,
        filled: Decimal,
        fill_price: Price,
        charge: Decimal,
        decision_price: Price,
    ) {
        let px = fill_price.inner();
        let notional = (filled * px).abs();
        match side {
            Side::Buy => {
                self.position += filled;
                self.cash -= notional;
            }
            Side::Sell => {
                self.position -= filled;
                self.cash += notional;
            }
        }
        self.cash -= charge;
        self.bar_turnover += notional;

        let reference = decision_price.inner();
        if reference > Decimal::ZERO {
            // Signed so that paying up on a buy and selling down on a sell both
            // read as positive slippage — cost, in both directions.
            let signed = match side {
                Side::Buy => px - reference,
                Side::Sell => reference - px,
            };
            let bps = signed / reference * Decimal::from(10_000);
            self.slippage_weighted += bps * notional;
        }
    }

    /// Record a broker reject attributable to the model's own output — not to
    /// connectivity, not to venue maintenance (§12.6's fourth check).
    pub fn on_model_reject(&mut self) {
        self.rejects += 1;
    }

    /// Take the deployment's opening mark.
    ///
    /// Separate from [`Self::close_bar`] on purpose. The first bar has no
    /// previous equity point to return against, and the obvious shortcut —
    /// reporting `0.0` for it — puts a bar the strategy did not trade into Gate
    /// 15's count as a real observation, where it dilutes the signal-match rate
    /// and can trip a divergence halt against whatever the backtest expected on
    /// that bar. Neither is a small error in a gate whose whole difficulty is
    /// that its evidence accrues one day at a time.
    ///
    /// # Errors
    /// [`ForwardError::AlreadyOpened`], [`ForwardError::MisalignedBar`], and
    /// [`ForwardError::DegenerateEquity`].
    pub fn open(&mut self, bar: DateTime<Utc>, mark: Price) -> Result<(), ForwardError> {
        if let Some(prev) = self.last_bar {
            return Err(ForwardError::AlreadyOpened(prev));
        }
        self.check_grid(bar)?;
        let value = self.mark_to(mark, bar)?;
        self.equity_points.push((bar, value));
        self.last_bar = Some(bar);
        self.reset_bar_accumulators();
        Ok(())
    }

    fn check_grid(&self, bar: DateTime<Utc>) -> Result<(), ForwardError> {
        if bar.timestamp().rem_euclid(self.spec.bar_seconds) == 0 {
            Ok(())
        } else {
            Err(ForwardError::MisalignedBar {
                bar,
                bar_seconds: self.spec.bar_seconds,
            })
        }
    }

    // Signature split across lines deliberately: `cargo xtask check-money-f64`
    // flags any line carrying both a float and a money word, and `mark: Price`
    // beside `-> Result<f64, ..>` would read as exactly the mistake the check
    // exists to catch. The float here is a *ratio's* denominator, not money.
    fn mark_to(
        &self,
        mark: Price,
        bar: DateTime<Utc>,
    ) -> Result<f64, ForwardError> {
        let marked = self.cash + self.position * mark.inner();
        let value = marked.to_f64().unwrap_or(f64::NAN);
        if !value.is_finite() || value <= 0.0 {
            return Err(ForwardError::DegenerateEquity(bar));
        }
        Ok(value)
    }

    fn reset_bar_accumulators(&mut self) {
        self.bar_turnover = Decimal::ZERO;
        self.slippage_weighted = Decimal::ZERO;
        self.rejects = 0;
    }

    /// Close the open bar at `bar` with the strategy marked at `mark`.
    ///
    /// # Errors
    /// [`ForwardError::NotOpened`] before [`Self::open`],
    /// [`ForwardError::MisalignedBar`] when `bar` is off the deployment's grid,
    /// [`ForwardError::BarOutOfOrder`] when it does not advance, and
    /// [`ForwardError::DegenerateEquity`] when equity is not finite and positive.
    pub fn close_bar(&mut self, bar: DateTime<Utc>, mark: Price) -> Result<BarClose, ForwardError> {
        self.check_grid(bar)?;
        let Some(prev) = self.last_bar else {
            return Err(ForwardError::NotOpened);
        };
        if bar <= prev {
            return Err(ForwardError::BarOutOfOrder {
                bar,
                previous: prev,
            });
        }

        let value = self.mark_to(mark, bar)?;
        let previous = self
            .equity_points
            .last()
            .map(|(_, v)| *v)
            .ok_or(ForwardError::NotOpened)?;
        let realised_return = value / previous - 1.0;

        let turnover = self.bar_turnover;
        let slippage_bps = if turnover > Decimal::ZERO {
            (self.slippage_weighted / turnover).to_f64().unwrap_or(0.0)
        } else {
            0.0
        };
        let rejects = self.rejects;

        self.equity_points.push((bar, value));
        self.last_bar = Some(bar);
        self.reset_bar_accumulators();

        Ok(BarClose {
            bar,
            realised_return,
            realised_slippage_bps: slippage_bps,
            realised_turnover: turnover,
            model_attributable_rejects: rejects,
        })
    }

    /// The realised series so far, in the ledger's own type.
    ///
    /// Built with [`ReturnSeries::from_equity`] — the same constructor the
    /// backtest path uses — so the two series are produced by one piece of code
    /// rather than by two that agree today.
    #[must_use]
    pub fn realised_series(&self) -> RealisedSeries {
        RealisedSeries {
            numeraire: self.spec.numeraire,
            bar_seconds: self.spec.bar_seconds,
            series: ReturnSeries::from_equity(&self.equity_points),
        }
    }

    /// Turn a closed bar plus the backtest's expectation for it into the row
    /// Gate 15 reads.
    #[must_use]
    pub fn observation(
        &self,
        close: &BarClose,
        expected: ExpectedBar,
        source: ObservationSource,
    ) -> ForwardObservation {
        ForwardObservation {
            deployment_id: self.spec.deployment_id,
            strategy_id: self.spec.strategy_id.clone(),
            bar: close.bar,
            realised_return: close.realised_return,
            expected_return: expected.expected_return,
            numeraire: self.spec.numeraire,
            source,
            signal_matched: expected.signal_matched,
            realised_slippage_bps: close.realised_slippage_bps,
            modelled_slippage_bps: expected.modelled_slippage_bps,
            realised_turnover: close.realised_turnover.to_f64().unwrap_or(0.0),
            expected_turnover: expected.expected_turnover.to_f64().unwrap_or(0.0),
            model_attributable_rejects: close.model_attributable_rejects,
            observed_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn thresholds() -> DivergenceThresholds {
        DivergenceThresholds {
            max_abs_return_divergence: 0.02,
            max_cum_return_divergence: 0.05,
            max_position_divergence: Decimal::from_str("0.000001").unwrap(),
            min_signal_match: 0.99,
        }
    }

    fn spec(numeraire: Numeraire) -> DeploymentSpec {
        DeploymentSpec {
            deployment_id: Uuid::nil(),
            strategy_id: "sma-cross-1".into(),
            trial_id: Uuid::nil(),
            venue: PaperVenue::AlpacaPaper,
            numeraire,
            bar_seconds: 60,
            thresholds: thresholds(),
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn px(s: &str) -> Price {
        Price::from_str(s).unwrap()
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn the_only_venue_is_paper() {
        assert!(PaperVenue::AlpacaPaper.is_paper());
        assert_eq!(PaperVenue::AlpacaPaper.as_str(), "alpaca_paper");
    }

    #[test]
    fn a_threshold_that_can_never_fire_is_rejected() {
        let mut t = thresholds();
        t.max_abs_return_divergence = 0.0;
        assert!(matches!(
            t.validate(),
            Err(ForwardError::UndeclaredThreshold("max_abs_return_divergence"))
        ));
        let mut t = thresholds();
        t.max_cum_return_divergence = f64::INFINITY;
        assert!(t.validate().is_err());
        let mut t = thresholds();
        t.min_signal_match = 1.5;
        assert!(t.validate().is_err());
    }

    #[test]
    fn a_bar_off_the_grid_is_refused() {
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        // The grid is checked before anything else, on open and on close alike.
        assert!(matches!(
            b.open(at(90), px("10")).unwrap_err(),
            ForwardError::MisalignedBar { .. }
        ));
        b.open(at(60), px("10")).unwrap();
        let err = b.close_bar(at(90), px("10")).unwrap_err();
        assert!(matches!(err, ForwardError::MisalignedBar { .. }));
        // The aligned close on the same grid is fine.
        assert!(b.close_bar(at(120), px("10")).is_ok());
    }

    #[test]
    fn a_bar_that_does_not_advance_is_refused() {
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        b.open(at(60), px("10")).unwrap();
        b.close_bar(at(120), px("10")).unwrap();
        assert!(matches!(
            b.close_bar(at(120), px("10")).unwrap_err(),
            ForwardError::BarOutOfOrder { .. }
        ));
    }

    #[test]
    fn realised_equity_is_net_of_the_fee_charged() {
        // Two identical books, one charged 10 on the fill. The charged one must
        // return less — this is what "same cost treatment" means against a
        // pessimistic-cost backtest series.
        let mut free = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        free.open(at(60), px("10")).unwrap();
        free.on_fill(Side::Buy, dec("10"), px("10"), Decimal::ZERO, px("10"));
        let free_bar = free.close_bar(at(120), px("10")).unwrap();

        let mut charged = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        charged.open(at(60), px("10")).unwrap();
        charged.on_fill(Side::Buy, dec("10"), px("10"), dec("10"), px("10"));
        let charged_bar = charged.close_bar(at(120), px("10")).unwrap();

        assert!(
            charged_bar.realised_return < free_bar.realised_return,
            "a charged fill must reduce the realised return"
        );
    }

    #[test]
    fn slippage_is_measured_against_the_decision_price_and_signed_as_cost() {
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("10000")).unwrap();
        b.open(at(60), px("100")).unwrap();
        // Bought at 101 having decided at 100 — paid up 100 bps.
        b.on_fill(Side::Buy, dec("1"), px("101"), Decimal::ZERO, px("100"));
        let close = b.close_bar(at(120), px("101")).unwrap();
        assert!((close.realised_slippage_bps - 100.0).abs() < 1e-6);

        // Sold at 99 having decided at 100 — also 100 bps of cost, not −100.
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("10000")).unwrap();
        b.open(at(60), px("100")).unwrap();
        b.on_fill(Side::Sell, dec("1"), px("99"), Decimal::ZERO, px("100"));
        let close = b.close_bar(at(120), px("99")).unwrap();
        assert!((close.realised_slippage_bps - 100.0).abs() < 1e-6);
    }

    fn two_bar_series(numeraire: Numeraire, bar_seconds: i64, returns: &[f64]) -> RealisedSeries {
        RealisedSeries {
            numeraire,
            bar_seconds,
            series: ReturnSeries {
                timestamps: (1..=returns.len())
                    .map(|i| at(i as i64 * bar_seconds))
                    .collect(),
                returns: returns.to_vec(),
            },
        }
    }

    #[test]
    fn aligned_series_compare_element_wise_with_no_adjustment() {
        let realised = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02]);
        let expected = two_bar_series(Numeraire::Usd, 60, &[0.011, -0.018]);
        let bars = realised.align(&expected).unwrap();
        assert_eq!(bars.len(), 2);
        assert!((bars[0].divergence() + 0.001).abs() < 1e-12);
    }

    /// The adversarial case T-0.1 exists for: a USDT-denominated strategy must
    /// not be summed as USD. The arithmetic would work; the answer would be
    /// wrong by whatever the peg moved.
    #[test]
    fn a_usdt_series_is_not_comparable_with_a_usd_series() {
        let realised = two_bar_series(Numeraire::Usdt, 60, &[0.01, -0.02]);
        let expected = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02]);
        let err = realised
            .align(&expected)
            .expect_err("USDT must not align against USD");
        assert!(matches!(err, AlignmentError::Numeraire(_)));
    }

    #[test]
    fn series_on_different_bar_grids_do_not_align() {
        let realised = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02]);
        let expected = two_bar_series(Numeraire::Usd, 300, &[0.01, -0.02]);
        assert!(matches!(
            realised.align(&expected).unwrap_err(),
            AlignmentError::BarWidth { .. }
        ));
    }

    #[test]
    fn a_shorter_series_is_not_quietly_truncated() {
        let realised = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02, 0.003]);
        let expected = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02]);
        assert!(matches!(
            realised.align(&expected).unwrap_err(),
            AlignmentError::Length {
                realised: 3,
                expected: 2
            }
        ));
    }

    #[test]
    fn series_whose_bars_disagree_do_not_align() {
        let realised = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02]);
        let mut expected = two_bar_series(Numeraire::Usd, 60, &[0.01, -0.02]);
        expected.series.timestamps[1] = at(180);
        assert!(matches!(
            realised.align(&expected).unwrap_err(),
            AlignmentError::Bar { index: 1, .. }
        ));
    }

    /// The first bar has no previous equity to return against. Reporting `0.0`
    /// for it would put a bar the strategy did not trade into Gate 15's count,
    /// where it dilutes the signal-match rate and can trip a divergence halt
    /// against whatever the backtest expected on that bar.
    #[test]
    fn a_bar_cannot_close_before_an_opening_mark_is_taken() {
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        assert_eq!(
            b.close_bar(at(60), px("10")).unwrap_err(),
            ForwardError::NotOpened
        );
        b.open(at(60), px("10")).unwrap();
        assert!(b.close_bar(at(120), px("10")).is_ok());
        assert!(matches!(
            b.open(at(180), px("10")).unwrap_err(),
            ForwardError::AlreadyOpened(_)
        ));
    }

    /// The opening mark is not an observation: five marks produce four returns,
    /// the same count the ledger's `from_equity` would give over five points.
    #[test]
    fn the_opening_mark_is_not_counted_as_a_bar() {
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        b.open(at(60), px("10")).unwrap();
        for i in 2..=5 {
            b.close_bar(at(i * 60), px("10")).unwrap();
        }
        assert_eq!(b.realised_series().series.returns.len(), 4);
    }

    #[test]
    fn the_realised_series_is_the_ledgers_own_type_and_validates_as_one() {
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        b.open(at(60), px("10")).unwrap();
        for i in 2..=5 {
            b.close_bar(at(i * 60), px("10")).unwrap();
        }
        let realised = b.realised_series();
        // `ReturnSeries::validate` is what the ledger runs before persisting a
        // backtest series. The realised one passes the same check.
        realised.series.validate().unwrap();
        assert_eq!(realised.series.returns.len(), 4);
    }
}
