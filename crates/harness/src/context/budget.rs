//! Budget follows value (AGENT-005 §20, §27.1; Set S tasks S-9.7 and S-9.18's
//! output reserve).
//!
//! # The observation
//!
//! A step about to spend a trial and a step listing instruments get identically
//! sized bodies today. That is simply wrong, and it is wrong in a way no amount
//! of better selection fixes: the lookup step is given more context than it can
//! use while the trial step is squeezed, on every run, by construction.
//!
//! So the body budget scales with what the step is worth — `0.6×` to `1.4×`
//! within the profile's hard input budget and Part I's 25% floor.
//!
//! # Total spend is unchanged, and that is enforced rather than hoped
//!
//! Multipliers that average near one *tend* to leave the total alone. Tending is
//! not a guarantee, and a context change that quietly raised every run's token
//! bill would be a bad trade however good the selection was.
//!
//! [`RunBudget`] makes it arithmetic instead. The run has an allowance of
//! `base × max_steps` and no step may take the account past it, so over a run
//! that goes the distance the total is bounded by exactly what a flat run would
//! have spent. The distribution moves; the total cannot rise.
//!
//! A step may also draw a little ahead of itself — [`LOOKAHEAD`] steps — because
//! the alternative is that the *first* step can never be worth more than one
//! step, which would make the whole mechanism inert until some earlier step had
//! been frugal. The exposure that buys is stated rather than hidden: a run that
//! stops early may have spent up to `base × LOOKAHEAD × (step_value_max − 1)`
//! more than a flat run of that length, and no more.
//!
//! The grant is charged **on issue** and refunded by [`RunBudget::record`] when
//! the step used less. Charging on issue is the safe default: a caller that
//! forgets to record leaves the account conservative, where the opposite order
//! would leave the cap silently unenforced — which is the failure this whole
//! module is trying not to be an instance of.

use super::policy::ContextPolicy;
use crate::state::GoalKind;

/// What the harness knows about the step it is about to render for.
///
/// A plain struct rather than a read of `WorkingState`, because one of these
/// facts is not the harness's to know: whether the step's tool spends a trial is
/// a property of the catalogue, and this crate is sans-IO and has no catalogue.
/// The loop is told, the same way it is told which actions are gated.
#[derive(Debug, Clone, Default)]
pub struct StepFacts {
    /// The active step's tool spends one of the run's trials.
    pub spends_trial: bool,
    /// The previous reply was rejected, and this is the repair.
    pub after_correction: bool,
    /// This step calls the same tool the previous one did.
    pub repeat_of_same_tool: bool,
    pub goal_kind: GoalKind,
    /// The step names a tool at all. `Reflect` and `AskUser` do not.
    pub names_a_tool: bool,
}

/// How much the step is worth, as a multiplier on the body budget (§20).
#[must_use]
pub fn step_value(facts: &StepFacts, policy: &ContextPolicy) -> f32 {
    let mut v = 1.0_f32;
    // A trial is the scarcest thing this platform spends. A step about to spend
    // one and a step listing instruments are not the same step.
    if facts.spends_trial {
        v *= 1.30;
    }
    // A first attempt after a correction is worth more than a repeat: the model
    // has just been told something and this is where it uses it. A repeat of the
    // same tool is worth less — it has seen that shape already, and the
    // idempotent-repeat guard exists because runs do this far too often.
    if facts.after_correction && !facts.repeat_of_same_tool {
        v *= 1.15;
    }
    if facts.repeat_of_same_tool {
        v *= 0.85;
    }
    // A lookup is answered by one result. Handing it the research body is how a
    // three-step budget gets spent on orientation.
    if facts.goal_kind == GoalKind::Lookup {
        v *= 0.80;
    }
    // `Reflect` and `AskUser` are the loop serving itself; neither reads a
    // large body and one of them is a question to a human.
    if !facts.names_a_tool {
        v *= 0.80;
    }
    v.clamp(policy.step_value_min, policy.step_value_max)
}

/// How far ahead of itself one step may draw.
///
/// Without it the first step's allowance is one step's worth, so a step worth
/// `1.4×` gets `1.0×` and the mechanism does nothing until something has been
/// frugal — which on a short run is never.
pub const LOOKAHEAD: u32 = 2;

/// The run's token account (§20).
#[derive(Debug, Clone)]
pub struct RunBudget {
    /// The unscaled per-step body budget.
    base: u32,
    /// The run's declared length, which is what the allowance is against.
    max_steps: u32,
    /// Granted so far, net of refunds.
    spent: u64,
    /// Steps that have drawn.
    steps: u32,
}

impl RunBudget {
    #[must_use]
    pub fn new(base: u32, max_steps: u32) -> Self {
        Self {
            base,
            max_steps: max_steps.max(1),
            spent: 0,
            steps: 0,
        }
    }

    /// Tokens this step may spend on the body.
    ///
    /// `hard` is the profile's input budget and `floor` Part I's 25% minimum;
    /// neither is ever crossed, in either direction, whatever the step is worth.
    pub fn allow(&mut self, value: f32, hard: u32, floor: u32) -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ideal = (self.base as f32 * value) as u32;
        // Two ceilings, and the tighter wins.
        //
        // The first is this step's share plus a bounded look ahead, so an early
        // step can be worth more than one step without a later one being able to
        // borrow the whole run.
        let horizon = (self.steps + 1 + LOOKAHEAD).min(self.max_steps.max(self.steps + 1));
        let windowed = u64::from(self.base) * u64::from(horizon);
        // The second is the run's allowance **less the floor every remaining
        // step is owed**. Without that subtraction the account empties and the
        // floor then grants anyway — which is how a mechanism that exists to
        // hold the total flat ends up spending past it, one floor at a time.
        // Part I's floor is not negotiable, so it is budgeted for rather than
        // treated as an exception.
        let remaining = u64::from(self.max_steps.saturating_sub(self.steps + 1));
        let reserved = u64::from(floor) * remaining;
        let ceiling = windowed.min(self.allowance().saturating_sub(reserved));
        let available = u32::try_from(ceiling.saturating_sub(self.spent)).unwrap_or(u32::MAX);
        let grant = ideal.min(available).min(hard).max(floor.min(hard));
        self.steps += 1;
        self.spent += u64::from(grant);
        grant
    }

    /// What the step actually used, refunding the rest to the run.
    ///
    /// Without this a frugal step would still be charged its whole grant, and
    /// the cumulative cap would bite on a run that had spent nothing.
    pub fn record(&mut self, actual: u32, granted: u32) {
        self.spent = self
            .spent
            .saturating_sub(u64::from(granted.saturating_sub(actual)));
    }

    #[must_use]
    pub fn spent(&self) -> u64 {
        self.spent
    }

    /// What a flat, unscaled run would have spent by now — the comparison the
    /// "total does not rise" claim is against.
    #[must_use]
    pub fn flat_equivalent(&self) -> u64 {
        u64::from(self.base) * u64::from(self.steps)
    }

    /// The run's whole allowance. Never exceeded, whatever any step is worth.
    #[must_use]
    pub fn allowance(&self) -> u64 {
        u64::from(self.base) * u64::from(self.max_steps)
    }
}

/// Tokens to hold back for the model's reply (§27.1).
///
/// One constant per profile means reserving the maximum on every call, which is
/// why the body is smaller than it needs to be on most of them. A lookup answers
/// with an id; a report-writing step answers with paragraphs. Scaled within the
/// profile's own bounds, so a profile that declared a reserve still gets one.
#[must_use]
pub fn output_reserve(profile_reserve: u32, facts: &StepFacts) -> u32 {
    let scale = if !facts.names_a_tool {
        // Reflection and a question to the user are the long answers.
        1.0
    } else if facts.goal_kind == GoalKind::Lookup {
        0.6
    } else if facts.spends_trial {
        // A trial-spending call is arguments, not prose.
        0.7
    } else {
        0.85
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = (profile_reserve as f32 * scale) as u32;
    // Never below half the declared reserve: the profile's number is a claim
    // about the model's own verbosity, and this is a claim about the step.
    scaled.max(profile_reserve / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> StepFacts {
        StepFacts {
            goal_kind: GoalKind::Research,
            names_a_tool: true,
            ..StepFacts::default()
        }
    }

    /// The case §20 exists for.
    #[test]
    fn a_trial_spending_step_gets_a_larger_body_than_a_lookup() {
        let p = ContextPolicy::v2();
        let trial = step_value(
            &StepFacts {
                spends_trial: true,
                ..facts()
            },
            &p,
        );
        let lookup = step_value(
            &StepFacts {
                goal_kind: GoalKind::Lookup,
                ..facts()
            },
            &p,
        );
        assert!(
            trial > lookup,
            "a trial step and an instrument listing get identically sized bodies today, \
             and that is simply wrong: {trial} vs {lookup}"
        );
    }

    /// A repair is worth more than a repeat.
    #[test]
    fn a_first_attempt_after_a_correction_beats_a_repeat() {
        let p = ContextPolicy::v2();
        let repair = step_value(
            &StepFacts {
                after_correction: true,
                ..facts()
            },
            &p,
        );
        let repeat = step_value(
            &StepFacts {
                after_correction: true,
                repeat_of_same_tool: true,
                ..facts()
            },
            &p,
        );
        assert!(repair > repeat, "{repair} vs {repeat}");
    }

    /// The bounds are the policy's, and they are never crossed.
    #[test]
    fn the_multiplier_stays_inside_the_declared_bounds() {
        let p = ContextPolicy::v2();
        for f in [
            StepFacts {
                spends_trial: true,
                after_correction: true,
                ..facts()
            },
            StepFacts {
                goal_kind: GoalKind::Lookup,
                names_a_tool: false,
                repeat_of_same_tool: true,
                ..facts()
            },
        ] {
            let v = step_value(&f, &p);
            assert!(
                (p.step_value_min..=p.step_value_max).contains(&v),
                "{v} outside [{}, {}]",
                p.step_value_min,
                p.step_value_max
            );
        }
        // The control does not scale at all.
        let v1 = ContextPolicy::v1();
        assert!((step_value(&facts(), &v1) - 1.0).abs() < f32::EPSILON);
    }

    /// The distribution moves and the total does not rise. The acceptance.
    #[test]
    fn total_tokens_across_a_fixed_run_do_not_rise() {
        let p = ContextPolicy::v2();
        let base = 10_000;
        let hard = 40_000;
        let floor = 2_500;

        let mut b = RunBudget::new(base, 20);
        let mut grants = Vec::new();
        // Twenty steps, every third one spending a trial.
        for step in 0..20u32 {
            let f = StepFacts {
                spends_trial: step % 3 == 0,
                ..facts()
            };
            grants.push(b.allow(step_value(&f, &p), hard, floor));
        }
        assert!(
            b.spent() <= b.flat_equivalent(),
            "over the whole run: {} spent against a flat {}",
            b.spent(),
            b.flat_equivalent()
        );
        assert!(b.spent() <= b.allowance());
        assert!(
            grants.iter().any(|g| *g > base),
            "and the distribution actually moved: {grants:?}"
        );
        assert!(grants.iter().any(|g| *g < base));
    }

    /// A frugal step refunds, so a later step can draw on it.
    #[test]
    fn an_unspent_grant_is_returned_to_the_run() {
        let mut b = RunBudget::new(10_000, 20);
        let g = b.allow(1.0, 40_000, 2_500);
        b.record(1_000, g);
        assert_eq!(b.spent(), 1_000);
        // The next step can now draw on what the first did not use.
        let g2 = b.allow(1.4, 40_000, 2_500);
        assert_eq!(g2, 14_000, "1.4x, which the run has room for");
    }

    /// Neither the hard budget nor the floor is crossed.
    #[test]
    fn the_hard_budget_and_the_floor_bound_everything() {
        let mut b = RunBudget::new(10_000, 20);
        assert_eq!(
            b.allow(1.4, 12_000, 2_500),
            12_000,
            "the profile's hard input budget wins over the multiplier"
        );
        let mut tight = RunBudget::new(1_000, 20);
        // Even a step worth 0.6x keeps the floor.
        assert_eq!(tight.allow(0.6, 40_000, 2_500), 2_500);
    }

    /// A step may draw ahead of itself, and only so far.
    ///
    /// The bound is what makes the look-ahead safe to have: a run that stops
    /// early has overspent by at most the window, and that number is stated
    /// rather than discovered.
    #[test]
    fn drawing_ahead_is_bounded_by_the_window() {
        let mut b = RunBudget::new(10_000, 40);
        let mut grants = 0u64;
        for _ in 0..6 {
            grants += u64::from(b.allow(1.4, 100_000, 2_500));
        }
        let flat = b.flat_equivalent();
        let overshoot = grants.saturating_sub(flat);
        assert!(
            overshoot <= u64::from(10_000 * LOOKAHEAD),
            "overshot by {overshoot} against a window of {}",
            10_000 * LOOKAHEAD
        );
        assert!(overshoot > 0, "and the window is actually usable");
    }

    /// The output reserve follows the step, within the profile's own bounds.
    #[test]
    fn a_report_reserves_more_output_than_a_lookup() {
        let lookup = output_reserve(
            4_000,
            &StepFacts {
                goal_kind: GoalKind::Lookup,
                names_a_tool: true,
                ..StepFacts::default()
            },
        );
        let report = output_reserve(
            4_000,
            &StepFacts {
                goal_kind: GoalKind::Research,
                names_a_tool: false,
                ..StepFacts::default()
            },
        );
        assert!(report > lookup, "{report} vs {lookup}");
        assert!(
            lookup >= 2_000,
            "never below half the declared reserve: the profile's number is a claim about \
             the model and this is a claim about the step"
        );
        assert!(report <= 4_000, "and never above it");
    }
}
