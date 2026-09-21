//! The per-strategy state machine (COMP-007 §4).
//!
//! `nominal → warning → breach → scaled_down → halted`, plus `resumed`.
//!
//! The division that decides which transitions are automatic is P4: **reversible
//! actions are automatic, irreversible ones are human.** Scaling a strategy to a
//! quarter of its size is reversible, so it fires on its own. Releasing a halt is
//! not — by the time anyone looks, the position is different, the market has moved,
//! and the reason for the halt may or may not still be true — so it needs a person
//! who typed the scope name and a reason.

use serde::{Deserialize, Serialize};

use crate::family::Family;

/// What happens to open positions when a strategy is halted (COMP-007 §4).
///
/// Named `HaltPolicy` after the spec. This is **not** `domain::instrument::HaltPolicy`,
/// which is a different question — whether the *exchange* can halt the instrument —
/// and carries `Haltable | NonHaltable`. See the dated correction in COMP-007 §4;
/// the spec cited `crates/domain/src/instrument.rs:198` for `hold / flatten /
/// reduce`, and that enum has never had those variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltPolicy {
    /// Stop new orders; leave the position alone.
    Hold,
    /// Stop new orders; close the position.
    Flatten,
    /// Stop new orders; cut the position back.
    Reduce,
}

impl HaltPolicy {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Flatten => "flatten",
            Self::Reduce => "reduce",
        }
    }
}

impl std::str::FromStr for HaltPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "hold" => Ok(Self::Hold),
            "flatten" => Ok(Self::Flatten),
            "reduce" => Ok(Self::Reduce),
            other => Err(format!("unknown halt policy {other:?}")),
        }
    }
}

/// Where a strategy stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyState {
    Nominal,
    Warning,
    Breach,
    ScaledDown,
    Halted,
}

impl StrategyState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Warning => "warning",
            Self::Breach => "breach",
            Self::ScaledDown => "scaled_down",
            Self::Halted => "halted",
        }
    }

    /// Whether the order gate refuses this strategy's orders.
    #[must_use]
    pub fn refuses_orders(self) -> bool {
        matches!(self, Self::Halted)
    }
}

impl std::str::FromStr for StrategyState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nominal" => Ok(Self::Nominal),
            "warning" => Ok(Self::Warning),
            "breach" => Ok(Self::Breach),
            "scaled_down" => Ok(Self::ScaledDown),
            "halted" => Ok(Self::Halted),
            other => Err(format!("unknown strategy state {other:?}")),
        }
    }
}

/// What the monitor does about a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Write the `monitor_events` row. Always first — §8.5 requires the trip
    /// record to exist *before* the first refused order, so the record is not a
    /// side effect of the halt, it is a precondition of it.
    Record,
    /// Scale the strategy to this percentage of full size.
    ScaleTo(u8),
    /// Restore full size after the hysteresis window.
    ReleaseScale,
    /// Stop new orders and apply the policy to the open position.
    Halt(HaltPolicy),
}

/// A decided move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub from: StrategyState,
    pub to: StrategyState,
    pub actions: Vec<Action>,
}

impl Transition {
    /// No change: the state stands and nothing is done.
    #[must_use]
    pub fn none(state: StrategyState) -> Self {
        Self {
            from: state,
            to: state,
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn changed(&self) -> bool {
        self.from != self.to || !self.actions.is_empty()
    }
}

/// How bad a sustained reading is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Record and notify; no automatic action.
    Warning,
    /// Act. Which action depends on the family.
    Breach(Family),
}

/// Decide the move for a strategy whose metric has *sustained* a reading.
///
/// Dwell has already been applied by the caller: this function only ever sees
/// verdicts that act (COMP-007 §4.1).
#[must_use]
pub fn on_sustained(from: StrategyState, severity: Severity, policy: HaltPolicy) -> Transition {
    // A halted strategy is not moved by further readings. It is already refusing
    // orders, and re-halting it would write a second trip record for a state that
    // never changed — noise in exactly the log an operator reads under pressure.
    if from == StrategyState::Halted {
        return Transition::none(from);
    }

    match severity {
        Severity::Warning => {
            // A warning never *lowers* an automatic action that is already in
            // force. A scaled-down strategy whose next reading is merely a warning
            // is still scaled down; releasing on a warning would restore size to a
            // strategy the monitor is still unhappy about.
            if matches!(from, StrategyState::ScaledDown) {
                return Transition::none(from);
            }
            Transition {
                from,
                to: StrategyState::Warning,
                actions: vec![Action::Record],
            }
        }
        // Some families halt. Which ones is `Family::breach_halts` and is not
        // restated here: a list written twice is how §3 came to say `halt` for a
        // hard-stale feed while §4's table said `scale down`, and the code
        // followed the table. Scaling to a quarter still trades on a stale price.
        Severity::Breach(family) if family.breach_halts() => Transition {
            from,
            to: StrategyState::Halted,
            actions: vec![Action::Record, Action::Halt(policy)],
        },
        // Everything else scales down: reversible, therefore automatic.
        Severity::Breach(_) => {
            if from == StrategyState::ScaledDown {
                return Transition::none(from);
            }
            Transition {
                from,
                to: StrategyState::ScaledDown,
                actions: vec![Action::Record, Action::ScaleTo(crate::SCALE_DOWN_PCT)],
            }
        }
    }
}

/// Decide the move for a strategy that has recovered for the full hysteresis
/// window.
///
/// A halted strategy does **not** come back this way. Halt release is a human,
/// typed, reasoned action (§5), and an automatic resume would make the typed
/// confirmation decorative: the operator who did not resume would find the
/// strategy trading anyway.
#[must_use]
pub fn on_recovered(from: StrategyState) -> Transition {
    match from {
        StrategyState::ScaledDown => Transition {
            from,
            to: StrategyState::Nominal,
            actions: vec![Action::Record, Action::ReleaseScale],
        },
        StrategyState::Warning | StrategyState::Breach => Transition {
            from,
            to: StrategyState::Nominal,
            actions: vec![Action::Record],
        },
        StrategyState::Nominal | StrategyState::Halted => Transition::none(from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_execution_breach_scales_to_a_quarter() {
        let t = on_sustained(
            StrategyState::Nominal,
            Severity::Breach(Family::Execution),
            HaltPolicy::Hold,
        );
        assert_eq!(t.to, StrategyState::ScaledDown);
        assert_eq!(t.actions, vec![Action::Record, Action::ScaleTo(25)]);
    }

    #[test]
    fn a_pnl_breach_halts_and_applies_the_policy() {
        for policy in [HaltPolicy::Hold, HaltPolicy::Flatten, HaltPolicy::Reduce] {
            let t = on_sustained(StrategyState::Nominal, Severity::Breach(Family::Pnl), policy);
            assert_eq!(t.to, StrategyState::Halted);
            assert_eq!(t.actions, vec![Action::Record, Action::Halt(policy)]);
        }
    }

    #[test]
    fn a_data_breach_halts_rather_than_trading_a_quarter_size_on_a_dead_feed() {
        // ADR-D-11. §3 said halt, §4's table said scale down, and the code
        // followed the table. §3 wins: every other response in that table
        // reduces exposure to a risk being *measured*, and a feed past the hard
        // multiple is an absent measurement.
        let t = on_sustained(
            StrategyState::Nominal,
            Severity::Breach(Family::Data),
            HaltPolicy::Flatten,
        );
        assert_eq!(t.to, StrategyState::Halted);
        assert_eq!(
            t.actions,
            vec![Action::Record, Action::Halt(HaltPolicy::Flatten)]
        );
        assert!(
            !t.actions
                .iter()
                .any(|a| matches!(a, Action::ScaleTo(_))),
            "a quarter-size position priced off a dead feed is the same error at 25% of the size"
        );
    }

    #[test]
    fn every_family_moves_the_way_breach_halts_says_it_does() {
        // The anti-drift assertion, and the reason `Family::breach_halts` exists
        // rather than a second list here or in a test name. This iterates the
        // whole enum and takes its expectation from the same function the code
        // branches on, so a family whose answer changes cannot leave a test
        // asserting the old behaviour — and a family added later is covered the
        // day it is added.
        for family in Family::all() {
            let t = on_sustained(
                StrategyState::Nominal,
                Severity::Breach(family),
                HaltPolicy::Flatten,
            );
            if family.breach_halts() {
                assert_eq!(t.to, StrategyState::Halted, "{family}");
                assert_eq!(
                    t.actions,
                    vec![Action::Record, Action::Halt(HaltPolicy::Flatten)],
                    "{family}"
                );
            } else {
                assert_eq!(t.to, StrategyState::ScaledDown, "{family}");
                assert_eq!(
                    t.actions,
                    vec![Action::Record, Action::ScaleTo(crate::SCALE_DOWN_PCT)],
                    "{family}"
                );
            }
        }
    }

    #[test]
    fn the_record_always_comes_before_the_halt() {
        // COMP-007 §8.5: "the trip record is present before the first refused
        // order". Asserted on the ordering of the plan rather than on a comment,
        // because a log written after the halt is a log that is missing exactly
        // when the process died between the two.
        for severity in [
            Severity::Breach(Family::Pnl),
            Severity::Breach(Family::Execution),
            Severity::Breach(Family::Data),
            Severity::Warning,
        ] {
            let t = on_sustained(StrategyState::Nominal, severity, HaltPolicy::Flatten);
            assert_eq!(t.actions.first(), Some(&Action::Record), "{severity:?}");
        }
    }

    #[test]
    fn a_halted_strategy_never_auto_resumes() {
        // The adversarial one. If recovery released a halt, the typed human
        // confirmation in §5 would be decorative: wait long enough and the
        // strategy trades again on its own.
        let t = on_recovered(StrategyState::Halted);
        assert_eq!(t.to, StrategyState::Halted);
        assert!(t.actions.is_empty());
        assert!(!t.changed());
    }

    #[test]
    fn a_halted_strategy_is_not_scaled_back_up_by_a_later_warning() {
        let t = on_sustained(StrategyState::Halted, Severity::Warning, HaltPolicy::Hold);
        assert_eq!(t.to, StrategyState::Halted);
        assert!(t.actions.is_empty());
    }

    #[test]
    fn a_warning_does_not_release_a_scale_down() {
        let t = on_sustained(
            StrategyState::ScaledDown,
            Severity::Warning,
            HaltPolicy::Hold,
        );
        assert_eq!(t.to, StrategyState::ScaledDown);
        assert!(t.actions.is_empty());
    }

    #[test]
    fn a_second_breach_while_scaled_down_does_not_rescale() {
        let t = on_sustained(
            StrategyState::ScaledDown,
            Severity::Breach(Family::Behaviour),
            HaltPolicy::Hold,
        );
        assert!(!t.changed(), "a repeated breach must not write a second trip record");
    }

    #[test]
    fn a_halting_family_still_halts_a_strategy_that_is_already_scaled_down() {
        // Scaling down is not a lesser halt; a strategy whose returns have broken
        // — or whose prices have stopped arriving — must stop trading even though
        // an earlier execution breach already cut its size. Driven off
        // `breach_halts` so it covers both halting families and any later one.
        for family in Family::all().into_iter().filter(|f| f.breach_halts()) {
            let t = on_sustained(
                StrategyState::ScaledDown,
                Severity::Breach(family),
                HaltPolicy::Reduce,
            );
            assert_eq!(t.to, StrategyState::Halted, "{family}");
        }
    }

    #[test]
    fn recovery_releases_a_scale_down_and_returns_to_nominal() {
        let t = on_recovered(StrategyState::ScaledDown);
        assert_eq!(t.to, StrategyState::Nominal);
        assert_eq!(t.actions, vec![Action::Record, Action::ReleaseScale]);
    }

    #[test]
    fn only_halted_refuses_orders() {
        let refusing: Vec<StrategyState> = [
            StrategyState::Nominal,
            StrategyState::Warning,
            StrategyState::Breach,
            StrategyState::ScaledDown,
            StrategyState::Halted,
        ]
        .into_iter()
        .filter(|s| s.refuses_orders())
        .collect();
        assert_eq!(refusing, vec![StrategyState::Halted]);
    }
}
