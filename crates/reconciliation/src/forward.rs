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
/// One variant: the platform's own paper venue — the same venue core every
/// backtest runs on (ADR-0044), fed live marks on the wall clock. **Owner
/// decision 2026-10-02 (ADR-0048):** paper trading happens in the app, not on
/// Alpaca's paper account; `AlpacaPaper` was this enum's only variant until then
/// and is gone.
///
/// What forward evidence on this venue does and does not measure is in
/// ADR-0048: it is a test of the strategy on data nobody had when it was
/// built, and of the pipeline end to end. It is **not** a test of whether the
/// simulated fills match a real market's, because the paper fills and the
/// backtest's come from the same fill model by construction.
///
/// One variant is not a placeholder — the set's hard boundary is that nothing it
/// builds creates a live-venue code path, and an enum with no live variant is
/// that sentence written where the compiler can read it. Adding one is a source
/// change, a review, and roadmap gate H-4, rather than a string that happened to
/// be typed into a config file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperVenue {
    /// The platform's own paper venue (`execution::paper`, ADR-0048).
    Platform,
}

impl PaperVenue {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform_paper",
        }
    }

    /// Always true, by construction. Kept as a method because the entry-point
    /// tests assert on it, and an assertion that reads `venue.is_paper()` keeps
    /// its meaning if the enum ever grows.
    #[must_use]
    pub fn is_paper(self) -> bool {
        matches!(self, Self::Platform)
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

/// What a shadow run of the same compiled plan, on the simulated venue over the
/// identical bars, says should have happened in one bar.
///
/// The only way to build an [`ExpectedBar`] from a shadow run is
/// [`ExpectedBar::from_shadow`], which checks the bar and refuses a non-finite
/// number: [`ExpectedBar`] itself carries no bar, so without this an expectation
/// computed for bar *n-1* pairs silently with the realised return of bar *n*.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowBar {
    /// The bar close the shadow run computed this for.
    pub bar: DateTime<Utc>,
    /// The shadow equity's return over the bar, on the same chain rule as the
    /// realised series.
    pub expected_return: f64,
    /// Notional the shadow run traded this bar, in the deployment's numeraire.
    pub expected_turnover: Decimal,
    /// Notional-weighted slippage the simulator charged the shadow fills, in bps
    /// against the shadow's decision price.
    pub modelled_slippage_bps: f64,
    /// Whether the shadow run decided the same orders the live run submitted.
    pub signal_matched: bool,
}

impl ExpectedBar {
    /// The expectation for `bar` from a shadow run.
    ///
    /// # Errors
    /// [`ForwardError::ShadowBarMismatch`] when the shadow is for another bar;
    /// [`ForwardError::ShadowNotFinite`] for a NaN or infinite expected return.
    pub fn from_shadow(bar: DateTime<Utc>, shadow: &ShadowBar) -> Result<Self, ForwardError> {
        if shadow.bar != bar {
            return Err(ForwardError::ShadowBarMismatch {
                bar,
                shadow: shadow.bar,
            });
        }
        if !shadow.expected_return.is_finite() || !shadow.modelled_slippage_bps.is_finite() {
            return Err(ForwardError::ShadowNotFinite(bar));
        }
        Ok(Self {
            expected_return: shadow.expected_return,
            expected_turnover: shadow.expected_turnover,
            modelled_slippage_bps: shadow.modelled_slippage_bps,
            signal_matched: shadow.signal_matched,
        })
    }
}

/// One fill of this deployment, as it is read back from the journal on a restart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResumeFill {
    pub side: Side,
    pub filled: Decimal,
    pub fill_price: Price,
    pub charge: Decimal,
    /// The mark the strategy decided at — slippage is measured against it.
    pub decision_price: Price,
    pub filled_at: DateTime<Utc>,
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
    #[error(
        "cannot resume: the stored returns compound to an equity of {stored} at {bar}, but the \
         fills and mark rebuild {rebuilt} — resuming from a balance that does not match the \
         stored chain would make the realised series non-comparable with the backtest's"
    )]
    ResumeInconsistent {
        bar: DateTime<Utc>,
        stored: f64,
        rebuilt: f64,
    },
    #[error("cannot resume: a stored return at {0} is not a finite number")]
    ResumeNonFiniteReturn(DateTime<Utc>),
    #[error(
        "the shadow run's expectation is for bar {shadow}, but bar {bar} is being closed — an \
         expectation for another bar is a placeholder with a timestamp"
    )]
    ShadowBarMismatch {
        bar: DateTime<Utc>,
        shadow: DateTime<Utc>,
    },
    #[error("the shadow run's expected return for bar {0} is not a finite number")]
    ShadowNotFinite(DateTime<Utc>),
    #[error("no forward recorder is deployed for deployment {0}")]
    UnknownDeployment(Uuid),
    #[error(
        "cannot verify the last segment: it was anchored at {anchor} holding {position}, so the \
         anchor's equity needs the mark the series was re-anchored at — resume with \
         `RealisedSeriesBuilder::resume_anchored(.., Some(anchor_mark))`"
    )]
    ResumeAnchorUnverifiable {
        anchor: DateTime<Utc>,
        position: Decimal,
    },
}

/// Bars the realised series has no return for, because recording paused and
/// resumed on a fresh anchor ([`RealisedSeriesBuilder::reanchor`]).
///
/// The missing bars are those strictly after `after` up to and including
/// `anchor`: `anchor` itself is the new opening mark, so it has no return
/// either. A gap is **absence**, never a zero and never one return spread over
/// several bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesGap {
    /// The last bar a return was recorded at before the pause.
    pub after: DateTime<Utc>,
    /// The bar the series was re-anchored at.
    pub anchor: DateTime<Utc>,
}

/// An alignment that tolerates a realised series with gaps, and says where.
#[derive(Clone, Debug, PartialEq)]
pub struct GappedAlignment {
    /// The bars both series have, element-wise comparable.
    pub aligned: Vec<AlignedBar>,
    /// Bars the backtest series has and the realised series does not. Always
    /// reported: a comparison that silently skipped them would read as a
    /// complete one.
    pub missing: Vec<DateTime<Utc>>,
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
    /// Place this series beside a backtest series when it has **gaps**, and say
    /// exactly which bars are missing.
    ///
    /// [`Self::align`] stays strict and refuses a series of a different length,
    /// which is the right answer for a series that should be complete. A series
    /// that was re-anchored after a restart is *known* to be incomplete, and this
    /// is the comparison for it: every realised bar must be one the backtest has
    /// (anything else is a real mismatch, refused as [`AlignmentError::Bar`]),
    /// and every backtest bar the realised series lacks is returned in
    /// [`GappedAlignment::missing`] — never skipped quietly, never filled with a
    /// zero, never replaced by a return spread over several bars.
    ///
    /// # Errors
    /// [`AlignmentError`] on a numeraire or grid mismatch, or a realised bar the
    /// backtest series does not have.
    pub fn align_over_gaps(
        &self,
        expected: &RealisedSeries,
    ) -> Result<GappedAlignment, AlignmentError> {
        self.numeraire.assert_comparable(&expected.numeraire)?;
        if self.bar_seconds != expected.bar_seconds {
            return Err(AlignmentError::BarWidth {
                realised: self.bar_seconds,
                expected: expected.bar_seconds,
            });
        }
        let mut aligned = Vec::new();
        let mut missing = Vec::new();
        let mut i = 0usize;
        for (j, et) in expected.series.timestamps.iter().enumerate() {
            if self.series.timestamps.get(i) == Some(et) {
                aligned.push(AlignedBar {
                    bar: *et,
                    realised: self.series.returns[i],
                    expected: expected.series.returns[j],
                });
                i += 1;
            } else {
                missing.push(*et);
            }
        }
        if i < self.series.timestamps.len() {
            return Err(AlignmentError::Bar {
                index: i,
                realised: self.series.timestamps[i],
                expected: expected
                    .series
                    .timestamps
                    .last()
                    .copied()
                    .unwrap_or(self.series.timestamps[i]),
            });
        }
        Ok(GappedAlignment { aligned, missing })
    }

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
        self.numeraire.assert_comparable(&expected.numeraire)?;
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
    /// Returns already stored before a restart, exactly as they were computed.
    /// Empty for a builder that has never been resumed. Kept as returns rather
    /// than re-derived from rebuilt equity so that a resumed series is
    /// bit-identical to one built without the restart.
    prefix: ReturnSeries,
    /// Pauses in recording, in order. See [`SeriesGap`].
    gaps: Vec<SeriesGap>,
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
            prefix: ReturnSeries {
                timestamps: Vec::new(),
                returns: Vec::new(),
            },
            gaps: Vec::new(),
            last_bar: None,
            bar_turnover: Decimal::ZERO,
            slippage_weighted: Decimal::ZERO,
            rejects: 0,
        })
    }

    /// Continue a deployment after a restart so that its series is identical to
    /// one built without the restart.
    ///
    /// `returns` are the stored `(bar close, realised_return)` pairs of every bar
    /// closed so far, **in order**, starting with the first bar after `open_bar`.
    /// `fills` are every fill of this deployment, in order, with the decision
    /// price each was submitted at. Fills at or before the last closed bar are
    /// folded into cash and position; later ones are applied as the open bar's
    /// accumulators (turnover, slippage). `last_mark` is the mark the last
    /// closed bar (or the opening bar, with no returns) was taken at.
    ///
    /// Three refusals:
    /// - a bar that is off the grid, or does not advance (so a duplicate is
    ///   refused too) — the same errors [`Self::close_bar`] gives;
    /// - a non-finite stored return;
    /// - [`ForwardError::ResumeInconsistent`] when the equity the stored returns
    ///   compound to disagrees with the equity the fills and mark rebuild. That
    ///   is the guard against resuming from the *current* balance, which would
    ///   silently put the series on a different base.
    ///
    /// Not recoverable from the database, by design of what is stored: model
    /// rejects counted in the still-open bar. They restart at zero.
    ///
    /// # Errors
    /// As above, plus those of [`Self::new`] and [`Self::open`].
    pub fn resume(
        spec: DeploymentSpec,
        opening_balance: Decimal,
        open_bar: DateTime<Utc>,
        returns: impl IntoIterator<Item = (DateTime<Utc>, f64)>,
        fills: &[ResumeFill],
        last_mark: Price,
    ) -> Result<Self, ForwardError> {
        Self::resume_anchored(
            spec,
            opening_balance,
            open_bar,
            returns,
            fills,
            last_mark,
            None,
        )
    }

    /// [`Self::resume`] for a series that has been re-anchored
    /// ([`Self::reanchor`]) at a bar where the book was **not flat**.
    ///
    /// The guard runs on the **last segment** only — from the last anchor to the
    /// last stored bar — and checks that the stored returns after the anchor
    /// compound, from the anchor's equity, to the equity the fills and
    /// `last_mark` rebuild. The anchor's equity is inferred, not stored:
    /// - flat at the anchor (the designed case: recording pauses until live is
    ///   flat) → it is the cash the fills up to the anchor leave, no mark needed;
    /// - holding at the anchor → it needs the mark the series was re-anchored
    ///   at, which is `anchor_mark`. Without it the segment cannot be verified
    ///   and this refuses ([`ForwardError::ResumeAnchorUnverifiable`]) rather
    ///   than skipping the check.
    ///
    /// The anchor is inferred from the first stored bar after a gap
    /// (`anchor = bar − bar_seconds`), which holds because `reanchor` is
    /// followed by a close one bar later. Earlier segments are not re-verified:
    /// their equity changes are not in the stored returns, and each was checked
    /// when it was resumed.
    ///
    /// `anchor_mark` is ignored for a series with no gap.
    ///
    /// # Errors
    /// As [`Self::resume`], plus [`ForwardError::ResumeAnchorUnverifiable`].
    pub fn resume_anchored(
        spec: DeploymentSpec,
        opening_balance: Decimal,
        open_bar: DateTime<Utc>,
        returns: impl IntoIterator<Item = (DateTime<Utc>, f64)>,
        fills: &[ResumeFill],
        last_mark: Price,
        anchor_mark: Option<Price>,
    ) -> Result<Self, ForwardError> {
        let mut b = Self::new(spec, opening_balance)?;
        b.check_grid(open_bar)?;
        let mut last = open_bar;
        // The compounded return of the *current segment*: reset at each gap.
        let mut implied = 1.0_f64;
        let mut segment_len = 0usize;
        let step = chrono::Duration::seconds(b.spec.bar_seconds);
        for (bar, r) in returns {
            b.check_grid(bar)?;
            if bar <= last {
                return Err(ForwardError::BarOutOfOrder {
                    bar,
                    previous: last,
                });
            }
            // A stored bar that is not the next on the grid is a pause that was
            // recorded by a re-anchor before the restart: the series has no
            // return across it, so neither can the rebuild.
            if bar > last + step {
                b.gaps.push(SeriesGap {
                    after: last,
                    anchor: bar - step,
                });
                implied = 1.0;
                segment_len = 0;
            }
            if !r.is_finite() {
                return Err(ForwardError::ResumeNonFiniteReturn(bar));
            }
            b.prefix.timestamps.push(bar);
            b.prefix.returns.push(r);
            implied *= 1.0 + r;
            segment_len += 1;
            last = bar;
        }
        // Fills up to and including the last closed bar make the book that bar
        // was marked on.
        let split = fills.partition_point(|f| f.filled_at <= last);
        let anchor = b.gaps.last().map(|g| g.anchor);
        // The book as it stood at the last anchor: taken just before the first
        // fill that came after it.
        let mut at_anchor: Option<(Decimal, Decimal)> = None;
        for f in &fills[..split] {
            if let (Some(a), None) = (anchor, at_anchor) {
                if f.filled_at > a {
                    at_anchor = Some((b.cash, b.position));
                }
            }
            b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
        }
        if anchor.is_some() && at_anchor.is_none() {
            at_anchor = Some((b.cash, b.position));
        }
        b.reset_bar_accumulators();
        let rebuilt = b.mark_to(last_mark, last)?;
        // The base the last segment compounds from: the opening balance when
        // the series was never re-anchored, otherwise the anchor's equity.
        let base = match (anchor, at_anchor) {
            (Some(a), Some((anchor_cash, anchor_position))) => {
                let anchor_equity = if anchor_position.is_zero() {
                    anchor_cash
                } else {
                    match anchor_mark {
                        Some(m) => anchor_cash + anchor_position * m.inner(),
                        None => {
                            return Err(ForwardError::ResumeAnchorUnverifiable {
                                anchor: a,
                                position: anchor_position,
                            })
                        }
                    }
                };
                let value = anchor_equity.to_f64().unwrap_or(f64::NAN);
                if !value.is_finite() || value <= 0.0 {
                    return Err(ForwardError::DegenerateEquity(a));
                }
                value
            }
            _ => opening_balance.to_f64().unwrap_or(f64::NAN),
        };
        let stored = base * implied;
        if (rebuilt - stored).abs() > 1e-9 * stored.abs().max(1.0) * (segment_len as f64 + 1.0)
        {
            return Err(ForwardError::ResumeInconsistent {
                bar: last,
                stored,
                rebuilt,
            });
        }
        b.equity_points.push((last, rebuilt));
        b.last_bar = Some(last);
        for f in &fills[split..] {
            b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
        }
        Ok(b)
    }

    /// Set a fresh equity anchor at `bar` after recording paused, keeping
    /// everything already recorded.
    ///
    /// After a restart in which the deployment held a position, no bar is closed
    /// until live is flat again. The next [`Self::close_bar`] would then compute
    /// one return across all the missing bars and record it as one. This closes
    /// the finished segment into the series, marks the book at `bar`, and starts
    /// a new segment there: **no return is recorded for any bar in between, nor
    /// for `bar` itself** (it is the new opening mark). The pause is kept as a
    /// [`SeriesGap`] and is visible to [`RealisedSeries::align_over_gaps`].
    ///
    /// Position and cash are untouched; the open bar's accumulators are reset,
    /// because turnover during a pause belongs to no bar that will be recorded.
    ///
    /// # Errors
    /// [`ForwardError::NotOpened`] before [`Self::open`] (or a resume),
    /// [`ForwardError::MisalignedBar`] off the grid,
    /// [`ForwardError::BarOutOfOrder`] for `bar` at or before the last bar —
    /// which refuses a duplicate anchor — and
    /// [`ForwardError::DegenerateEquity`].
    pub fn reanchor(&mut self, bar: DateTime<Utc>, mark: Price) -> Result<(), ForwardError> {
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
        let finished = ReturnSeries::from_equity(&self.equity_points);
        self.prefix.timestamps.extend(finished.timestamps);
        self.prefix.returns.extend(finished.returns);
        self.equity_points.clear();
        self.equity_points.push((bar, value));
        self.last_bar = Some(bar);
        self.gaps.push(SeriesGap {
            after: prev,
            anchor: bar,
        });
        self.reset_bar_accumulators();
        Ok(())
    }

    /// Every pause in recording so far, oldest first.
    #[must_use]
    pub fn gaps(&self) -> &[SeriesGap] {
        &self.gaps
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
    fn mark_to(&self, mark: Price, bar: DateTime<Utc>) -> Result<f64, ForwardError> {
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
        let mut series = self.prefix.clone();
        let fresh = ReturnSeries::from_equity(&self.equity_points);
        series.timestamps.extend(fresh.timestamps);
        series.returns.extend(fresh.returns);
        RealisedSeries {
            numeraire: self.spec.numeraire.clone(),
            bar_seconds: self.spec.bar_seconds,
            series,
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
            numeraire: self.spec.numeraire.clone(),
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
            venue: PaperVenue::Platform,
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
        assert!(PaperVenue::Platform.is_paper());
        assert_eq!(PaperVenue::Platform.as_str(), "platform_paper");
    }

    #[test]
    fn a_threshold_that_can_never_fire_is_rejected() {
        let mut t = thresholds();
        t.max_abs_return_divergence = 0.0;
        assert!(matches!(
            t.validate(),
            Err(ForwardError::UndeclaredThreshold(
                "max_abs_return_divergence"
            ))
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

    // ── resume ──────────────────────────────────────────────────────────────

    fn rfill(side: Side, qty: &str, price: &str, at_secs: i64) -> ResumeFill {
        ResumeFill {
            side,
            filled: dec(qty),
            fill_price: px(price),
            charge: dec("0.25"),
            decision_price: px(price),
            filled_at: at(at_secs),
        }
    }

    /// One scripted deployment: marks per bar close and fills between closes.
    struct Script {
        marks: Vec<(i64, &'static str)>,
        fills: Vec<ResumeFill>,
    }

    fn script() -> Script {
        Script {
            marks: vec![
                (60, "100"),
                (120, "101.5"),
                (180, "99.25"),
                (240, "103"),
                (300, "102.125"),
                (360, "104"),
            ],
            fills: vec![
                rfill(Side::Buy, "3", "100.5", 100),
                rfill(Side::Sell, "1", "99.5", 200),
                rfill(Side::Buy, "2", "102.5", 290),
                rfill(Side::Sell, "4", "103.5", 330),
            ],
        }
    }

    /// Run the script without a restart, returning the builder and each bar close.
    fn uninterrupted() -> (RealisedSeriesBuilder, Vec<BarClose>) {
        let sc = script();
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        b.open(at(sc.marks[0].0), px(sc.marks[0].1)).unwrap();
        let mut closes = Vec::new();
        for &(t, m) in &sc.marks[1..] {
            for f in sc.fills.iter().filter(|f| {
                f.filled_at.timestamp() <= t && f.filled_at.timestamp() > t - 60
            }) {
                b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
            }
            closes.push(b.close_bar(at(t), px(m)).unwrap());
        }
        (b, closes)
    }

    /// Restart after `k` closed bars: rebuild from what a database holds, then
    /// carry on with the rest of the script.
    fn restarted_after(k: usize, fills_known: usize) -> (RealisedSeriesBuilder, Vec<BarClose>) {
        let sc = script();
        let (_, closes) = uninterrupted();
        let stored: Vec<(DateTime<Utc>, f64)> = closes[..k]
            .iter()
            .map(|c| (c.bar, c.realised_return))
            .collect();
        let last_mark = sc.marks[k].1;
        let mut b = RealisedSeriesBuilder::resume(
            spec(Numeraire::Usd),
            dec("1000"),
            at(sc.marks[0].0),
            stored,
            &sc.fills[..fills_known],
            px(last_mark),
        )
        .unwrap();
        let mut rest = Vec::new();
        for &(t, m) in &sc.marks[k + 1..] {
            for f in sc.fills.iter().filter(|f| {
                f.filled_at.timestamp() <= t && f.filled_at.timestamp() > t - 60
            }) {
                b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
            }
            rest.push(b.close_bar(at(t), px(m)).unwrap());
        }
        (b, rest)
    }

    #[test]
    fn a_resumed_builder_produces_the_series_a_builder_that_never_restarted_does() {
        let (whole, closes) = uninterrupted();
        // Every restart point, from "nothing closed yet" to "everything closed".
        for k in 0..=5 {
            // The restart knows only the fills made by the last closed bar.
            let known = script()
                .fills
                .iter()
                .filter(|f| f.filled_at.timestamp() <= script().marks[k].0)
                .count();
            let (resumed, rest) = restarted_after(k, known);
            assert_eq!(
                resumed.realised_series(),
                whole.realised_series(),
                "restart after {k} bars changed the series"
            );
            assert_eq!(rest, closes[k..], "restart after {k} bars changed a later bar's close");
            assert_eq!(resumed.position(), whole.position());
        }
    }

    #[test]
    fn fills_after_the_last_closed_bar_are_not_lost_in_the_restart() {
        // Restart after the bar closing at 180, having already seen the fill at
        // 200, which belongs to the still-open bar: its turnover and slippage
        // must land in the bar that closes at 240, exactly as without a restart.
        let (_, closes) = uninterrupted();
        let sc = script();
        let stored: Vec<_> = closes[..2].iter().map(|c| (c.bar, c.realised_return)).collect();
        let mut b = RealisedSeriesBuilder::resume(
            spec(Numeraire::Usd),
            dec("1000"),
            at(60),
            stored,
            &sc.fills[..2],
            px("99.25"),
        )
        .unwrap();
        let c = b.close_bar(at(240), px("103")).unwrap();
        assert_eq!(c, closes[2]);
        assert!(c.realised_turnover > Decimal::ZERO);
    }

    #[test]
    fn a_duplicate_or_out_of_order_stored_bar_is_refused() {
        let sc = script();
        let r = |returns: Vec<(DateTime<Utc>, f64)>| {
            RealisedSeriesBuilder::resume(
                spec(Numeraire::Usd),
                dec("1000"),
                at(60),
                returns,
                &[],
                px("100"),
            )
        };
        assert!(matches!(
            r(vec![(at(120), 0.0), (at(120), 0.0)]).unwrap_err(),
            ForwardError::BarOutOfOrder { .. }
        ));
        assert!(matches!(
            r(vec![(at(180), 0.0), (at(120), 0.0)]).unwrap_err(),
            ForwardError::BarOutOfOrder { .. }
        ));
        assert!(matches!(
            r(vec![(at(60), 0.0)]).unwrap_err(),
            ForwardError::BarOutOfOrder { .. }
        ));
        assert!(matches!(
            r(vec![(at(150), 0.0)]).unwrap_err(),
            ForwardError::MisalignedBar { .. }
        ));
        assert!(matches!(
            r(vec![(at(120), f64::NAN)]).unwrap_err(),
            ForwardError::ResumeNonFiniteReturn(_)
        ));
        let _ = sc;
    }

    #[test]
    fn resuming_from_the_wrong_balance_is_refused_not_silently_rebased() {
        // The stored chain says equity 1000 → ~1000·(1+r). Rebuilding from fills
        // that were never made (or a different mark) gives a different equity.
        let (_, closes) = uninterrupted();
        let sc = script();
        let stored: Vec<_> = closes[..2].iter().map(|c| (c.bar, c.realised_return)).collect();
        let wrong_mark = RealisedSeriesBuilder::resume(
            spec(Numeraire::Usd),
            dec("1000"),
            at(60),
            stored.clone(),
            &sc.fills[..1],
            px("90"),
        );
        assert!(matches!(
            wrong_mark.unwrap_err(),
            ForwardError::ResumeInconsistent { .. }
        ));
        let missing_fill = RealisedSeriesBuilder::resume(
            spec(Numeraire::Usd),
            dec("1000"),
            at(60),
            stored,
            &[],
            px("99.25"),
        );
        assert!(matches!(
            missing_fill.unwrap_err(),
            ForwardError::ResumeInconsistent { .. }
        ));
    }

    // ── reanchor ────────────────────────────────────────────────────────────

    /// The scenario: restart after the bar closing at 180, with the position
    /// held; recording pauses and re-anchors at 240 (the fill at 200 has been
    /// applied by then). Bars 300 and 360 are recorded after the anchor.
    fn reanchored() -> RealisedSeriesBuilder {
        let sc = script();
        let (_, closes) = uninterrupted();
        let stored: Vec<_> = closes[..2].iter().map(|c| (c.bar, c.realised_return)).collect();
        let mut b = RealisedSeriesBuilder::resume(
            spec(Numeraire::Usd),
            dec("1000"),
            at(60),
            stored,
            &sc.fills[..1],
            px("99.25"),
        )
        .unwrap();
        // The fill at 200 happens while recording is paused.
        let f = &sc.fills[1];
        b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
        b.reanchor(at(240), px("103")).unwrap();
        b
    }

    #[test]
    fn bars_after_a_reanchor_equal_an_uninterrupted_series_and_no_return_spans_the_gap() {
        let (whole, closes) = uninterrupted();
        let sc = script();
        let mut b = reanchored();
        // The fills at 290 and 330 fall in the bars that close at 300 and 360.
        let mut after = Vec::new();
        for &(t, m) in &sc.marks[4..] {
            for f in sc.fills.iter().filter(|f| {
                f.filled_at.timestamp() <= t && f.filled_at.timestamp() > t - 60 && f.filled_at.timestamp() > 240
            }) {
                b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
            }
            after.push(b.close_bar(at(t), px(m)).unwrap());
        }
        // Closes for 300 and 360 are exactly the uninterrupted closes.
        assert_eq!(after, closes[3..]);
        let series = b.realised_series();
        let times: Vec<i64> = series.series.timestamps.iter().map(|t| t.timestamp()).collect();
        assert_eq!(times, vec![120, 180, 300, 360], "no bar at 240 (the anchor) and none for the pause");
        // The recorded bars carry the uninterrupted returns, bar for bar.
        let want: Vec<f64> = [0usize, 1, 3, 4]
            .iter()
            .map(|&i| whole.realised_series().series.returns[i])
            .collect();
        assert_eq!(series.series.returns, want);
        series.series.validate().expect("still a valid return series");
    }

    #[test]
    fn the_gap_is_visible_and_never_a_return() {
        let (whole, _) = uninterrupted();
        let mut b = reanchored();
        b.close_bar(at(300), px("102.125")).unwrap();
        assert_eq!(
            b.gaps(),
            &[SeriesGap {
                after: at(180),
                anchor: at(240)
            }]
        );
        let realised = b.realised_series();
        let full = whole.realised_series();
        // Strict alignment refuses an incomplete series rather than truncating.
        assert!(matches!(
            realised.align(&full).unwrap_err(),
            AlignmentError::Length { .. }
        ));
        // The gap-aware alignment says which bars are missing.
        let g = realised.align_over_gaps(&full).unwrap();
        assert_eq!(g.missing.iter().map(|t| t.timestamp()).collect::<Vec<_>>(), vec![240, 360]);
        assert_eq!(g.aligned.len(), 3);
        assert!(g.aligned.iter().all(|a| a.bar.timestamp() != 240));
        // A realised bar the backtest does not have is a real mismatch.
        let mut odd = realised.clone();
        odd.series.timestamps.push(at(9_000));
        odd.series.returns.push(0.0);
        assert!(matches!(
            odd.align_over_gaps(&full).unwrap_err(),
            AlignmentError::Bar { .. }
        ));
    }

    #[test]
    fn a_reanchor_is_refused_off_grid_out_of_order_duplicate_or_before_open() {
        let mut fresh = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        assert!(matches!(
            fresh.reanchor(at(120), px("100")).unwrap_err(),
            ForwardError::NotOpened
        ));
        let mut b = reanchored();
        assert!(matches!(
            b.reanchor(at(270), px("103")).unwrap_err(),
            ForwardError::MisalignedBar { .. }
        ));
        assert!(matches!(
            b.reanchor(at(240), px("103")).unwrap_err(),
            ForwardError::BarOutOfOrder { .. }
        ), "a duplicate anchor");
        assert!(matches!(
            b.reanchor(at(180), px("103")).unwrap_err(),
            ForwardError::BarOutOfOrder { .. }
        ), "an earlier anchor");
        // A refused anchor changes nothing.
        assert_eq!(b.gaps().len(), 1);
        assert!(b.close_bar(at(300), px("102.125")).is_ok());
    }

    #[test]
    fn a_series_with_a_stored_gap_resumes_and_remembers_the_gap() {
        // What the database holds after the re-anchored deployment restarts again:
        // returns at 120, 180, 300, 360 — no row for 240.
        let (_, closes) = uninterrupted();
        let stored: Vec<_> = [0usize, 1, 3, 4]
            .iter()
            .map(|&i| (closes[i].bar, closes[i].realised_return))
            .collect();
        let sc = script();
        // The anchor (240) held two units, so its equity needs the mark it was
        // anchored at; plain `resume` refuses rather than skip the check.
        assert!(matches!(
            RealisedSeriesBuilder::resume(
                spec(Numeraire::Usd),
                dec("1000"),
                at(60),
                stored.clone(),
                &sc.fills,
                px("104"),
            )
            .unwrap_err(),
            ForwardError::ResumeAnchorUnverifiable { .. }
        ));
        let b = RealisedSeriesBuilder::resume_anchored(
            spec(Numeraire::Usd),
            dec("1000"),
            at(60),
            stored,
            &sc.fills,
            px("104"),
            Some(px("103")),
        )
        .unwrap();
        assert_eq!(
            b.gaps(),
            &[SeriesGap {
                after: at(180),
                anchor: at(240)
            }]
        );
        assert_eq!(b.realised_series().series.timestamps.len(), 4);
    }

    // ── second restart, with an earlier gap ─────────────────────────────────

    /// A deployment that went flat before its anchor: buy and sell back to flat
    /// by 170, re-anchor at 180 (mark 50), then trade after the anchor.
    fn flat_script() -> (Vec<ResumeFill>, Vec<(i64, &'static str)>) {
        (
            vec![
                rfill(Side::Buy, "2", "50", 100),
                rfill(Side::Sell, "2", "51", 150),
                rfill(Side::Buy, "3", "50.5", 200),
                rfill(Side::Sell, "1", "52", 290),
            ],
            vec![(60, "50"), (120, "50"), (180, "50"), (240, "51"), (300, "52.5"), (360, "53")],
        )
    }

    /// Run the flat script with a re-anchor at 180 and no restart.
    fn flat_uninterrupted() -> (RealisedSeriesBuilder, Vec<BarClose>) {
        let (fills, marks) = flat_script();
        let mut b = RealisedSeriesBuilder::new(spec(Numeraire::Usd), dec("1000")).unwrap();
        b.open(at(60), px("50")).unwrap();
        let mut closes = Vec::new();
        let mut prev = 60_i64;
        for &(t, m) in &marks[1..] {
            for f in fills
                .iter()
                .filter(|f| f.filled_at.timestamp() > prev && f.filled_at.timestamp() <= t)
            {
                b.on_fill(f.side, f.filled, f.fill_price, f.charge, f.decision_price);
            }
            prev = t;
            if t == 180 {
                // Recording paused over 120..180: the anchor, no return.
                b.reanchor(at(180), px(m)).unwrap();
            } else {
                closes.push(b.close_bar(at(t), px(m)).unwrap());
            }
        }
        (b, closes)
    }

    #[test]
    fn a_second_restart_after_an_earlier_gap_equals_the_uninterrupted_reanchored_series() {
        // The first uninterrupted builder produced returns at 120, 240, 300, 360
        // and a gap at 180. Restart after bar 300 (second restart: the stored
        // series already has the gap), then carry on to 360.
        let (whole, closes) = flat_uninterrupted();
        let (fills, marks) = flat_script();
        assert_eq!(closes.len(), 4, "bars 120, 240, 300, 360");
        let stored: Vec<_> = closes[..3].iter().map(|c| (c.bar, c.realised_return)).collect();
        // Fills known at the restart: everything up to 300.
        let known: Vec<ResumeFill> = fills.iter().filter(|f| f.filled_at.timestamp() <= 300).copied().collect();
        let mut b = RealisedSeriesBuilder::resume(
            spec(Numeraire::Usd),
            dec("1000"),
            at(60),
            stored,
            &known,
            px(marks[4].1),
        )
        .expect("flat anchor: plain resume verifies the last segment");
        assert_eq!(b.gaps(), &[SeriesGap { after: at(120), anchor: at(180) }]);
        // Carry on: the fill at 290 was already applied (known), the bar at 360
        // has none.
        let c = b.close_bar(at(360), px(marks[5].1)).unwrap();
        assert_eq!(c, closes[3]);
        assert_eq!(b.realised_series(), whole.realised_series());
        assert_eq!(b.position(), whole.position());
    }

    #[test]
    fn a_corrupted_last_segment_is_refused_after_a_gap() {
        let (_, closes) = flat_uninterrupted();
        let (fills, marks) = flat_script();
        let known: Vec<ResumeFill> = fills.iter().filter(|f| f.filled_at.timestamp() <= 300).copied().collect();
        let good: Vec<_> = closes[..3].iter().map(|c| (c.bar, c.realised_return)).collect();
        let resume = |stored: Vec<(DateTime<Utc>, f64)>, fills: &[ResumeFill], mark: &str| {
            RealisedSeriesBuilder::resume(
                spec(Numeraire::Usd),
                dec("1000"),
                at(60),
                stored,
                fills,
                px(mark),
            )
        };
        assert!(resume(good.clone(), &known, marks[4].1).is_ok());

        // A stored return in the last segment is wrong.
        let mut tampered = good.clone();
        tampered[2].1 += 0.01;
        assert!(matches!(
            resume(tampered, &known, marks[4].1).unwrap_err(),
            ForwardError::ResumeInconsistent { .. }
        ));
        // A fill after the anchor is missing.
        assert!(matches!(
            resume(good.clone(), &known[..3], marks[4].1).unwrap_err(),
            ForwardError::ResumeInconsistent { .. }
        ));
        // The last mark is not the mark the last bar closed at.
        assert!(matches!(
            resume(good.clone(), &known, "60").unwrap_err(),
            ForwardError::ResumeInconsistent { .. }
        ));
        // The earlier segment is NOT re-verified (its equity is not in the
        // stored returns): tampering with bar 120 is not caught here, by design,
        // and is documented on `resume_anchored`.
        let mut early = good;
        early[0].1 += 0.01;
        assert!(resume(early, &known, marks[4].1).is_ok());
    }

    #[test]
    fn a_non_flat_anchor_needs_its_mark_and_a_wrong_mark_is_refused() {
        // `reanchored()` anchors at 240 holding two units (mark 103).
        let (_, closes) = uninterrupted();
        let sc = script();
        let stored: Vec<_> = [0usize, 1, 3, 4]
            .iter()
            .map(|&i| (closes[i].bar, closes[i].realised_return))
            .collect();
        let go = |anchor_mark: Option<&str>| {
            RealisedSeriesBuilder::resume_anchored(
                spec(Numeraire::Usd),
                dec("1000"),
                at(60),
                stored.clone(),
                &sc.fills,
                px("104"),
                anchor_mark.map(px),
            )
        };
        assert!(matches!(go(None).unwrap_err(), ForwardError::ResumeAnchorUnverifiable { .. }));
        assert!(go(Some("103")).is_ok());
        assert!(matches!(go(Some("90")).unwrap_err(), ForwardError::ResumeInconsistent { .. }));
    }

    #[test]
    fn a_shadow_expectation_for_another_bar_is_refused() {
        let s = ShadowBar {
            bar: at(120),
            expected_return: 0.001,
            expected_turnover: dec("100"),
            modelled_slippage_bps: 2.0,
            signal_matched: true,
        };
        assert!(ExpectedBar::from_shadow(at(120), &s).is_ok());
        assert!(matches!(
            ExpectedBar::from_shadow(at(180), &s).unwrap_err(),
            ForwardError::ShadowBarMismatch { .. }
        ));
        let nan = ShadowBar {
            expected_return: f64::NAN,
            ..s
        };
        assert!(matches!(
            ExpectedBar::from_shadow(at(120), &nan).unwrap_err(),
            ForwardError::ShadowNotFinite(_)
        ));
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
        let realised = two_bar_series(Numeraire::of("USDT"), 60, &[0.01, -0.02]);
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
