//! The ramp as a state machine (Set T T-3.1, T-3.3; ADR-P2-27, SPEC §12.3 Gate 16).
//!
//! Two quantities, one clock. The review **period** is the clock; its
//! **review** is the event; the machine below is what the events fold to.
//!
//! * the capital **rung** ([`AllowedFraction`], migration 0053) — up one rung per
//!   passed period, only for a deployment whose profile can authorise capital,
//!   and only with a [`RampAuthority`] in hand;
//! * the **size** ([`Size`]) — 1, 1/2, 1/4, 1/8, then zero. It can only fall. A
//!   failing period halves it at the boundary; the fourth failing period takes
//!   it to zero. Nothing in this module can make it rise, so "an ungated
//!   deployment may not increase size" (D-T4) is a property of the type.
//!
//! # Why "to zero" takes four periods
//!
//! Halving never reaches zero, so "50% per period, to zero" needs a floor. The
//! floor is the third halving (an eighth of the size): the next failing period
//! is zero. Four failing periods end a deployment; see ADR-T3-01.
//!
//! # Replay
//!
//! [`replay`] folds recorded [`ReviewEvent`]s to the state they led to. It trusts
//! the log (the database enforced every transition when it was written, migration
//! 0181), but it still refuses a log that skips a period or breaks the size
//! arithmetic, because a log that does is evidence of a bug rather than a state.
//! The live path, [`Ramp::step_up`], takes the authority token and refuses
//! without one.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::capital::{AllowedFraction, RampAuthority, RampRefusal};
use crate::ramp_gate::{StepJudgement, StepVerdict};

/// A deployment's size as a count of halvings. `4` is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Size(u8);

/// The number of halvings that is zero.
pub const ZERO_HALVINGS: u8 = 4;

impl Size {
    pub const FULL: Self = Self(0);
    pub const ZERO: Self = Self(ZERO_HALVINGS);

    #[must_use]
    pub fn halvings(self) -> u8 {
        self.0
    }

    /// `None` for a count that is not a size.
    #[must_use]
    pub fn from_halvings(h: u8) -> Option<Self> {
        (h <= ZERO_HALVINGS).then_some(Self(h))
    }

    /// The fraction of full size, exact in binary.
    #[must_use]
    pub fn factor(self) -> f64 {
        if self.0 >= ZERO_HALVINGS {
            0.0
        } else {
            1.0 / f64::from(1_u32 << self.0)
        }
    }

    /// Per cent of full size for the monitor's scale control (`scale_pct`).
    /// Rounded **down** (an eighth is 12, not 13): the unsafe direction for a
    /// cap is up.
    #[must_use]
    pub fn scale_pct(self) -> u8 {
        match self.0 {
            0 => 100,
            1 => 50,
            2 => 25,
            3 => 12,
            _ => 0,
        }
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 >= ZERO_HALVINGS
    }

    /// One step down. Zero stays zero.
    #[must_use]
    pub fn halved(self) -> Self {
        Self((self.0 + 1).min(ZERO_HALVINGS))
    }
}

/// What a review period resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Hold,
    StepUp,
    Halve,
    Zero,
}

impl Outcome {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::StepUp => "step_up",
            Self::Halve => "halve",
            Self::Zero => "zero",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Hold, Self::StepUp, Self::Halve, Self::Zero]
            .into_iter()
            .find(|o| o.as_str() == s)
    }
}

/// One recorded review: the event the ladder's history is made of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEvent {
    pub period_no: u32,
    pub outcome: Outcome,
    pub halvings_before: u8,
    pub halvings_after: u8,
}

/// Why a transition or a log was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RampError {
    #[error("period {got} cannot follow period {last}: a period is not skipped")]
    PeriodSkipped { last: u32, got: u32 },
    #[error("the deployment is already at zero size; no further period is reviewed")]
    AlreadyZero,
    #[error(
        "the review records {before} -> {after} halvings, which is not what a `{outcome}` does"
    )]
    BadArithmetic {
        outcome: &'static str,
        before: u8,
        after: u8,
    },
    #[error("the review starts from {recorded} halvings but the ladder is at {actual}")]
    Discontinuous { recorded: u8, actual: u8 },
    #[error(
        "this deployment is not on the capital ladder: its profile does not authorise capital"
    )]
    NotOnCapitalLadder,
    #[error("already at the top rung")]
    AtTheTop,
    #[error("a step-up is refused: {0}")]
    StepUpRefused(String),
    #[error(transparent)]
    Capital(#[from] RampRefusal),
}

/// The ladder's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ramp {
    size: Size,
    /// `None` when the profile cannot authorise capital: such a deployment never
    /// enters the capital ladder, and there is no value to raise.
    rung: Option<AllowedFraction>,
    last_period: u32,
}

impl Ramp {
    /// A deployment enters the ladder at full size. It is on the **capital**
    /// ladder (rung zero, the paper floor) only if its profile authorises
    /// capital; a `paper_v1` deployment gets the size ladder and no rung.
    #[must_use]
    pub fn enter(profile_authorises_capital: bool) -> Self {
        Self {
            size: Size::FULL,
            rung: profile_authorises_capital.then_some(AllowedFraction::Zero),
            last_period: 0,
        }
    }

    #[must_use]
    pub fn size(&self) -> Size {
        self.size
    }

    #[must_use]
    pub fn rung(&self) -> Option<AllowedFraction> {
        self.rung
    }

    #[must_use]
    pub fn last_period(&self) -> u32 {
        self.last_period
    }

    fn expect_next(&self, period_no: u32) -> Result<(), RampError> {
        if self.size.is_zero() {
            return Err(RampError::AlreadyZero);
        }
        if period_no != self.last_period + 1 {
            return Err(RampError::PeriodSkipped {
                last: self.last_period,
                got: period_no,
            });
        }
        Ok(())
    }

    /// Resolve a period from its judgement and apply it. The one live entry
    /// point; it returns the event to record.
    ///
    /// A period that did not pass (including one Gate 16 could not compute)
    /// halves the size. A passing period holds, unless the judgement permits a
    /// step *and* `authority` proves it, in which case the rung rises one place.
    ///
    /// # Errors
    /// A skipped period, a period after zero, or a step-up that the judgement or
    /// the profile does not support.
    pub fn review(
        &mut self,
        period_no: u32,
        judgement: &StepJudgement,
        authority: Option<&RampAuthority>,
    ) -> Result<ReviewEvent, RampError> {
        self.expect_next(period_no)?;
        let before = self.size.halvings();
        let outcome = if !judgement.passed() {
            if self.size.halvings() == ZERO_HALVINGS - 1 {
                Outcome::Zero
            } else {
                Outcome::Halve
            }
        } else if judgement.step_up_allowed {
            match (authority, self.rung) {
                (Some(a), Some(_)) => {
                    self.step_up(a)?;
                    Outcome::StepUp
                }
                (None, _) => {
                    return Err(RampError::StepUpRefused(
                        "the judgement permits a step but no capital authority was supplied".into(),
                    ))
                }
                (_, None) => return Err(RampError::NotOnCapitalLadder),
            }
        } else {
            Outcome::Hold
        };
        if matches!(outcome, Outcome::Halve | Outcome::Zero) {
            self.size = self.size.halved();
        }
        self.last_period = period_no;
        Ok(ReviewEvent {
            period_no,
            outcome,
            halvings_before: before,
            halvings_after: self.size.halvings(),
        })
    }

    /// Raise the rung exactly one place, with authority.
    ///
    /// # Errors
    /// Not on the capital ladder, or already at the top.
    pub fn step_up(&mut self, authority: &RampAuthority) -> Result<AllowedFraction, RampError> {
        let rung = self.rung.ok_or(RampError::NotOnCapitalLadder)?;
        if rung == AllowedFraction::Full {
            return Err(RampError::AtTheTop);
        }
        let next = rung.raise(authority);
        self.rung = Some(next);
        Ok(next)
    }

    /// Apply one recorded event, as replay.
    fn apply(&mut self, e: &ReviewEvent) -> Result<(), RampError> {
        self.expect_next(e.period_no)?;
        if e.halvings_before != self.size.halvings() {
            return Err(RampError::Discontinuous {
                recorded: e.halvings_before,
                actual: self.size.halvings(),
            });
        }
        let ok = match e.outcome {
            Outcome::Hold | Outcome::StepUp => e.halvings_after == e.halvings_before,
            Outcome::Halve => {
                e.halvings_before < ZERO_HALVINGS - 1 && e.halvings_after == e.halvings_before + 1
            }
            Outcome::Zero => {
                e.halvings_before == ZERO_HALVINGS - 1 && e.halvings_after == ZERO_HALVINGS
            }
        };
        if !ok {
            return Err(RampError::BadArithmetic {
                outcome: e.outcome.as_str(),
                before: e.halvings_before,
                after: e.halvings_after,
            });
        }
        if e.outcome == Outcome::StepUp {
            // The log was written under 0181's trigger, which required the
            // authority; replay does not have the token and does not need it.
            let rung = self.rung.ok_or(RampError::NotOnCapitalLadder)?;
            let i = AllowedFraction::LADDER
                .iter()
                .position(|r| *r == rung)
                .unwrap_or(0);
            let next = AllowedFraction::LADDER
                .get(i + 1)
                .copied()
                .ok_or(RampError::AtTheTop)?;
            self.rung = Some(next);
        }
        self.size = Size(e.halvings_after);
        self.last_period = e.period_no;
        Ok(())
    }
}

/// Fold a deployment's recorded reviews to the state they lead to.
///
/// # Errors
/// A log that skips a period, steps after zero, or breaks the size arithmetic.
pub fn replay(profile_authorises_capital: bool, events: &[ReviewEvent]) -> Result<Ramp, RampError> {
    let mut ramp = Ramp::enter(profile_authorises_capital);
    for e in events {
        ramp.apply(e)?;
    }
    Ok(ramp)
}

/// The next period's recorded boundaries: it starts exactly where the last one
/// ended. Computed from the **recorded** end, never from "now" or from the period
/// count, so a worker that ran late does not shift the schedule.
#[must_use]
pub fn next_period(
    previous_end: DateTime<Utc>,
    period_days: i64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    (previous_end, previous_end + Duration::days(period_days))
}

/// Whether a verdict is the one that steps a deployment down.
#[must_use]
pub fn steps_down(v: &StepVerdict) -> bool {
    !matches!(v, StepVerdict::Pass)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capital::authorise;
    use crate::ramp_gate::judge;
    use crate::ramp_gate::tests::{evidence, inputs};

    fn passing(profile: &str, capital: bool, rung: Option<AllowedFraction>) -> StepJudgement {
        judge(&inputs(profile, capital, rung), &evidence())
    }

    fn failing() -> StepJudgement {
        let mut ev = evidence();
        ev.divergence_halts = 1;
        judge(&inputs("paper_v1", false, None), &ev)
    }

    fn all_pass() -> Vec<(i32, bool)> {
        (1..=16).map(|g| (g, true)).collect()
    }

    #[test]
    fn a_failing_deployment_halves_each_period_and_is_zero_after_four() {
        let mut r = Ramp::enter(false);
        let mut factors = vec![r.size().factor()];
        let mut events = Vec::new();
        for n in 1..=4 {
            events.push(r.review(n, &failing(), None).unwrap());
            factors.push(r.size().factor());
        }
        assert_eq!(factors, vec![1.0, 0.5, 0.25, 0.125, 0.0]);
        let outcomes: Vec<_> = events.iter().map(|e| e.outcome).collect();
        assert_eq!(
            outcomes,
            vec![
                Outcome::Halve,
                Outcome::Halve,
                Outcome::Halve,
                Outcome::Zero
            ]
        );
        assert!(r.size().is_zero());
        // And nothing follows zero.
        assert_eq!(r.review(5, &failing(), None), Err(RampError::AlreadyZero));
    }

    #[test]
    fn the_monitor_scale_rounds_down_never_up() {
        let pcts: Vec<u8> = (0..=4)
            .map(|h| Size::from_halvings(h).unwrap().scale_pct())
            .collect();
        assert_eq!(pcts, vec![100, 50, 25, 12, 0]);
        for h in 0..=4 {
            let s = Size::from_halvings(h).unwrap();
            assert!(f64::from(s.scale_pct()) / 100.0 <= s.factor() + 1e-12);
        }
        assert_eq!(Size::from_halvings(5), None);
    }

    /// An ungated deployment may not increase size: there is no operation on
    /// `Size` that raises it, so the sequence is non-increasing for every input.
    #[test]
    fn size_never_rises_whatever_the_period_resolves_to() {
        let mut r = Ramp::enter(false);
        let seq = [
            failing(),
            passing("paper_v1", false, None),
            failing(),
            passing("paper_v1", false, None),
        ];
        let mut last = r.size();
        for (i, j) in seq.iter().enumerate() {
            r.review(u32::try_from(i).unwrap() + 1, j, None).unwrap();
            assert!(r.size() >= last, "size (in halvings) fell back up");
            last = r.size();
        }
    }

    #[test]
    fn a_not_computable_period_steps_down_like_a_failed_one() {
        let mut ev = evidence();
        ev.observed_bars = 0;
        let j = judge(&inputs("paper_v1", false, None), &ev);
        assert!(matches!(j.verdict, StepVerdict::NotComputable { .. }));
        let mut r = Ramp::enter(false);
        let e = r.review(1, &j, None).unwrap();
        assert_eq!(e.outcome, Outcome::Halve);
    }

    #[test]
    fn a_period_is_not_skipped() {
        let mut r = Ramp::enter(false);
        assert_eq!(
            r.review(2, &failing(), None),
            Err(RampError::PeriodSkipped { last: 0, got: 2 })
        );
        r.review(1, &failing(), None).unwrap();
        assert_eq!(
            r.review(1, &failing(), None),
            Err(RampError::PeriodSkipped { last: 1, got: 1 })
        );
    }

    #[test]
    fn a_paper_v1_deployment_is_never_on_the_capital_ladder() {
        let r = Ramp::enter(false);
        assert_eq!(r.rung(), None);
        let auth = authorise("strict_v1", true, &all_pass()).unwrap();
        let mut r = Ramp::enter(false);
        assert_eq!(r.step_up(&auth), Err(RampError::NotOnCapitalLadder));
        // A judgement that wrongly allowed a step still cannot move a paper_v1 ramp.
        let mut j = passing("paper_v1", false, None);
        j.step_up_allowed = true;
        assert_eq!(
            r.review(1, &j, Some(&auth)),
            Err(RampError::NotOnCapitalLadder)
        );
    }

    #[test]
    fn a_capital_ladder_climbs_one_rung_per_passed_period_and_cannot_skip() {
        let auth = authorise("strict_v1", true, &all_pass()).unwrap();
        let mut r = Ramp::enter(true);
        assert_eq!(r.rung(), Some(AllowedFraction::Zero));
        let mut rungs = vec![];
        for n in 1..=5 {
            let j = passing("strict_v1", true, r.rung());
            let e = r.review(n, &j, Some(&auth));
            if n <= 4 {
                assert_eq!(e.unwrap().outcome, Outcome::StepUp);
            } else {
                // At the top the judgement itself stops allowing a step.
                assert_eq!(e.unwrap().outcome, Outcome::Hold);
            }
            rungs.push(r.rung().unwrap());
        }
        assert_eq!(
            rungs,
            vec![
                AllowedFraction::Ten,
                AllowedFraction::TwentyFive,
                AllowedFraction::Fifty,
                AllowedFraction::Full,
                AllowedFraction::Full
            ]
        );
    }

    /// A step-up with no authority is refused at the state machine, not warned about.
    #[test]
    fn a_step_up_without_authority_is_refused() {
        let mut r = Ramp::enter(true);
        let j = passing("strict_v1", true, r.rung());
        assert!(j.step_up_allowed);
        assert!(matches!(
            r.review(1, &j, None),
            Err(RampError::StepUpRefused(_))
        ));
        assert_eq!(r.rung(), Some(AllowedFraction::Zero), "nothing moved");
        assert_eq!(r.last_period(), 0, "and the period was not consumed");
    }

    #[test]
    fn a_failing_gate_16_never_reaches_the_state_machine_as_a_step() {
        let auth = authorise("strict_v1", true, &all_pass()).unwrap();
        let mut ev = evidence();
        ev.signal_match = Some(0.1);
        let j = judge(&inputs("strict_v1", true, Some(AllowedFraction::Zero)), &ev);
        let mut r = Ramp::enter(true);
        let e = r.review(1, &j, Some(&auth)).unwrap();
        assert_eq!(e.outcome, Outcome::Halve);
        assert_eq!(r.rung(), Some(AllowedFraction::Zero));
    }

    #[test]
    fn the_history_replays_from_events_to_the_same_state() {
        let auth = authorise("strict_v1", true, &all_pass()).unwrap();
        let mut live = Ramp::enter(true);
        let mut log = Vec::new();
        for n in 1..=6 {
            let j = if n == 3 || n == 4 {
                let mut ev = evidence();
                ev.divergence_halts = 1;
                judge(&inputs("strict_v1", true, live.rung()), &ev)
            } else {
                passing("strict_v1", true, live.rung())
            };
            log.push(live.review(n, &j, Some(&auth)).unwrap());
        }
        let replayed = replay(true, &log).unwrap();
        assert_eq!(replayed, live);
        // Through JSON, as the stored rows would be.
        let stored = serde_json::to_string(&log).unwrap();
        let back: Vec<ReviewEvent> = serde_json::from_str(&stored).unwrap();
        assert_eq!(replay(true, &back).unwrap(), live);
    }

    #[test]
    fn a_log_that_skips_a_period_or_breaks_the_arithmetic_is_not_a_state() {
        let e = |p, o, b, a| ReviewEvent {
            period_no: p,
            outcome: o,
            halvings_before: b,
            halvings_after: a,
        };
        assert!(matches!(
            replay(false, &[e(2, Outcome::Hold, 0, 0)]),
            Err(RampError::PeriodSkipped { .. })
        ));
        assert!(matches!(
            replay(false, &[e(1, Outcome::Halve, 0, 2)]),
            Err(RampError::BadArithmetic { .. })
        ));
        assert!(matches!(
            replay(false, &[e(1, Outcome::Hold, 1, 1)]),
            Err(RampError::Discontinuous { .. })
        ));
        // A step-up recorded for a deployment that cannot be on the ladder.
        assert!(matches!(
            replay(false, &[e(1, Outcome::StepUp, 0, 0)]),
            Err(RampError::NotOnCapitalLadder)
        ));
    }

    #[test]
    fn period_boundaries_chain_from_the_recorded_end_not_the_clock() {
        use chrono::TimeZone;
        let end = Utc.with_ymd_and_hms(2026, 10, 15, 0, 0, 0).unwrap();
        let (s, e) = next_period(end, 14);
        assert_eq!(s, end);
        assert_eq!(e, end + Duration::days(14));
        // A worker that ran three days late still opens the next one at `end`.
        let (s2, _) = next_period(end, 14);
        assert_eq!(s2, s);
    }
}
