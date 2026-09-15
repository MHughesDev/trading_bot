//! Exit policy — definition version 1.3 (EXTERNAL-REVIEW-2026-09 §3.2, ADR-P6-01).
//!
//! ## The hole this closes
//!
//! Until 1.3, `ActionKind` had exactly one variant, `PlaceOrder`, and the
//! expression grammar was stateless: every expression is a comparison over the
//! *current* bar. Between them that made every path-dependent exit
//! inexpressible — stop-loss and take-profit need the entry price, a time exit
//! needs bars-since-entry, a trailing stop needs the running extreme since
//! entry. None of those is a function of the current bar.
//!
//! So every strategy the platform could author was entry-only, and the risk
//! layer's position limits were silently doing the exit logic. Position
//! management drives more P&L variance than entry timing in most systematic
//! strategies, which made this the largest single constraint on what the agent
//! could discover.
//!
//! ## Why this is not grammar
//!
//! The obvious fix is to give the grammar state. That is the wrong fix, and the
//! reason is the L0–L3 argument: expressivity belongs in the feature and model
//! layers, which already have versioning, lineage and `TrialTicket`, not in the
//! comparison operator, which has none of those. Widening the grammar widens the
//! hypothesis space in the one place the platform cannot audit.
//!
//! So exits are **typed fields, evaluated by the runtime against position
//! state**, and the grammar stays frozen. [`ExitRule`] is a closed enum: adding
//! a rule is a schema change with a version bump, not an expression somebody
//! writes at 2 a.m.
//!
//! ## Why "no exits" must be said out loud
//!
//! [`ExitPolicy`] has no `Default`, and `RiskLayerOnly` is a variant rather than
//! an absence. A strategy that delegates its exits to the risk layer is making a
//! real and sometimes correct choice; a strategy that *forgot* to have exits
//! looks identical from the outside, and did until now. Making the delegation
//! explicit puts it in the content hash, which means two revisions that differ
//! only in whether they have exits are no longer the same artifact.
//!
//! That last point is the one that matters for the ledger: `RunConfig
//! .strategy_version` hashes the definition, and an exit policy that lived
//! outside the definition would be a way for two runs with the same hash to
//! behave differently.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// What closes a position.
///
/// Deliberately has no `Default`: see the module docs. A definition at version
/// 1.3 or later must carry one, and "the risk layer handles it" is spelled
/// [`ExitPolicy::RiskLayerOnly`] rather than left blank.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExitPolicy {
    /// The definition declares no exit rule. Position limits in `risk_overrides`
    /// and any signal that emits flat are the entire exit logic.
    ///
    /// A legitimate choice for a strategy whose entry signal reverses cleanly.
    /// It is a variant and not an absence so that choosing it is a recorded act.
    RiskLayerOnly,
    /// Path-dependent rules evaluated against live position state each bar.
    ///
    /// The validator rejects an empty rule list: a `Rules` policy with nothing
    /// in it is `RiskLayerOnly` wearing a disguise, and the two must not hash
    /// alike.
    Rules {
        /// Evaluated in order; the first that fires closes the position. Order
        /// is therefore semantic and must never be canonicalised away by a
        /// hashing or normalisation pass.
        rules: Vec<ExitRule>,
    },
}

impl ExitPolicy {
    /// The rules, or an empty slice for [`ExitPolicy::RiskLayerOnly`].
    #[must_use]
    pub fn rules(&self) -> &[ExitRule] {
        match self {
            Self::RiskLayerOnly => &[],
            Self::Rules { rules } => rules,
        }
    }

    /// Whether this policy can ever close a position on its own.
    #[must_use]
    pub fn is_self_closing(&self) -> bool {
        !self.rules().is_empty()
    }
}

/// One path-dependent exit condition.
///
/// Every variant is a function of `(entry, extreme-since-entry, bars-held,
/// current)` — the four pieces of position state the grammar cannot see. A rule
/// that needs a fifth is a schema change, on purpose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum ExitRule {
    /// Close when the position is down `fraction` from the entry price.
    /// `0.02` = a 2% adverse move. Always expressed as a positive fraction of
    /// entry, never as an absolute price, so the rule survives a parameter
    /// sweep across instruments at different price scales.
    StopLoss { fraction: Decimal },
    /// Close when the position is up `fraction` from the entry price.
    TakeProfit { fraction: Decimal },
    /// Close when price retraces `fraction` from the best level reached since
    /// entry. Ratchets one way only — the reference is the running extreme, not
    /// the entry.
    TrailingStop { fraction: Decimal },
    /// Close after `bars` bars in the position, whatever the price has done.
    /// The exit that makes a holding period a declared quantity rather than an
    /// emergent one, which is what the tradeable-frontier check needs to read.
    MaxHoldingBars { bars: u32 },
    /// Close when the named signal fires. The one rule expressible in the old
    /// world; kept here so that a definition's exits are all in one place
    /// rather than split between `actions` and this block.
    OnSignal { emit: String },
}

impl ExitRule {
    /// A stable identifier for logging and for the reason string on a fill.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::StopLoss { .. } => "stop_loss",
            Self::TakeProfit { .. } => "take_profit",
            Self::TrailingStop { .. } => "trailing_stop",
            Self::MaxHoldingBars { .. } => "max_holding_bars",
            Self::OnSignal { .. } => "on_signal",
        }
    }

    /// The declared holding-period bound this rule implies, if it implies one.
    ///
    /// Only [`ExitRule::MaxHoldingBars`] does. A stop-loss bounds *loss*, not
    /// time, and reading it as a holding period is how a strategy ends up being
    /// admitted below the tradeable frontier.
    #[must_use]
    pub const fn holding_bound(&self) -> Option<u32> {
        match self {
            Self::MaxHoldingBars { bars } => Some(*bars),
            _ => None,
        }
    }

    /// Whether the rule's parameters are in range. A fraction outside `(0, 1]`
    /// is not a conservative stop, it is a typo: a stop at 1.5 can never fire on
    /// a long, and one at 0 fires on the entry bar.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        match self {
            Self::StopLoss { fraction }
            | Self::TakeProfit { fraction }
            | Self::TrailingStop { fraction } => {
                *fraction > Decimal::ZERO && *fraction <= Decimal::ONE
            }
            Self::MaxHoldingBars { bars } => *bars > 0,
            Self::OnSignal { emit } => !emit.trim().is_empty(),
        }
    }
}

/// Which way a position faces. Exits are asymmetric and the sign is not
/// recoverable from the prices alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

/// The position state an exit rule reads. Assembled by whoever holds the
/// position — the live runtime or the simulator — so that both evaluate exits
/// through the same function and cannot drift apart.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionState {
    pub side: PositionSide,
    /// Price the position was opened at.
    pub entry_price: Decimal,
    /// Most favourable price reached since entry: the high for a long, the low
    /// for a short. The trailing stop's reference.
    pub favourable_extreme: Decimal,
    /// Bars elapsed since the entry bar. The entry bar itself is 0.
    pub bars_held: u32,
    /// Price being evaluated against.
    pub current_price: Decimal,
    /// Signals emitted on this bar, for [`ExitRule::OnSignal`].
    pub signals: Vec<String>,
}

impl PositionState {
    /// Signed return from entry, as a fraction: positive is profit for the side
    /// the position is on.
    #[must_use]
    pub fn pnl_fraction(&self) -> Option<Decimal> {
        (self.entry_price > Decimal::ZERO).then(|| {
            let raw = (self.current_price - self.entry_price) / self.entry_price;
            match self.side {
                PositionSide::Long => raw,
                PositionSide::Short => -raw,
            }
        })
    }

    /// Retracement from the favourable extreme, as a positive fraction.
    #[must_use]
    pub fn retracement_fraction(&self) -> Option<Decimal> {
        (self.favourable_extreme > Decimal::ZERO).then(|| {
            let raw = (self.current_price - self.favourable_extreme) / self.favourable_extreme;
            match self.side {
                PositionSide::Long => -raw,
                PositionSide::Short => raw,
            }
        })
    }
}

/// Evaluate a policy against position state, returning the first rule that
/// fires.
///
/// Order is the declared order. A policy whose stop is listed after its target
/// behaves differently from one whose stop is first on a bar where both are
/// breached, and that is the author's decision to make — so nothing here sorts.
#[must_use]
pub fn first_triggered<'a>(policy: &'a ExitPolicy, state: &PositionState) -> Option<&'a ExitRule> {
    policy.rules().iter().find(|rule| triggers(rule, state))
}

/// Whether one rule fires against this state.
#[must_use]
pub fn triggers(rule: &ExitRule, state: &PositionState) -> bool {
    match rule {
        ExitRule::StopLoss { fraction } => {
            state.pnl_fraction().is_some_and(|p| p <= -*fraction)
        }
        ExitRule::TakeProfit { fraction } => {
            state.pnl_fraction().is_some_and(|p| p >= *fraction)
        }
        ExitRule::TrailingStop { fraction } => state
            .retracement_fraction()
            .is_some_and(|r| r >= *fraction),
        ExitRule::MaxHoldingBars { bars } => state.bars_held >= *bars,
        ExitRule::OnSignal { emit } => state.signals.iter().any(|s| s == emit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn long_at(entry: f64, now: f64) -> PositionState {
        PositionState {
            side: PositionSide::Long,
            entry_price: Decimal::try_from(entry).unwrap(),
            favourable_extreme: Decimal::try_from(entry.max(now)).unwrap(),
            bars_held: 1,
            current_price: Decimal::try_from(now).unwrap(),
            signals: vec![],
        }
    }

    fn short_at(entry: f64, now: f64) -> PositionState {
        PositionState {
            side: PositionSide::Short,
            entry_price: Decimal::try_from(entry).unwrap(),
            favourable_extreme: Decimal::try_from(entry.min(now)).unwrap(),
            bars_held: 1,
            current_price: Decimal::try_from(now).unwrap(),
            signals: vec![],
        }
    }

    // ── the policy itself ────────────────────────────────────────────────────

    /// The point of the whole module: "no exits" is a statement, not a blank.
    #[test]
    fn risk_layer_only_is_a_variant_not_an_absence() {
        let json = serde_json::to_value(&ExitPolicy::RiskLayerOnly).unwrap();
        assert_eq!(json, serde_json::json!({ "mode": "risk_layer_only" }));
    }

    /// Two policies that behave differently must not serialise alike, because
    /// the definition hash is what makes a `strategy_version` mean anything.
    #[test]
    fn delegating_and_declaring_do_not_hash_alike() {
        let delegated = serde_json::to_string(&ExitPolicy::RiskLayerOnly).unwrap();
        let declared = serde_json::to_string(&ExitPolicy::Rules {
            rules: vec![ExitRule::StopLoss { fraction: dec!(0.02) }],
        })
        .unwrap();
        assert_ne!(delegated, declared);
    }

    #[test]
    fn a_policy_with_no_rules_cannot_close_a_position() {
        assert!(!ExitPolicy::RiskLayerOnly.is_self_closing());
        assert!(!ExitPolicy::Rules { rules: vec![] }.is_self_closing());
        assert!(ExitPolicy::Rules {
            rules: vec![ExitRule::MaxHoldingBars { bars: 10 }]
        }
        .is_self_closing());
    }

    // ── stops and targets ────────────────────────────────────────────────────

    #[test]
    fn a_long_stop_fires_on_an_adverse_move_and_not_a_favourable_one() {
        let stop = ExitRule::StopLoss { fraction: dec!(0.02) };
        assert!(triggers(&stop, &long_at(100.0, 98.0)));
        assert!(triggers(&stop, &long_at(100.0, 97.0)));
        assert!(!triggers(&stop, &long_at(100.0, 98.5)));
        assert!(!triggers(&stop, &long_at(100.0, 102.0)));
    }

    /// The sign error this catches is the classic one: a short's stop is above
    /// its entry, and a rule written from the long's point of view never fires.
    #[test]
    fn a_short_stop_fires_when_price_rises() {
        let stop = ExitRule::StopLoss { fraction: dec!(0.02) };
        assert!(triggers(&stop, &short_at(100.0, 102.0)));
        assert!(!triggers(&stop, &short_at(100.0, 98.0)));
    }

    #[test]
    fn a_short_target_fires_when_price_falls() {
        let tp = ExitRule::TakeProfit { fraction: dec!(0.02) };
        assert!(triggers(&tp, &short_at(100.0, 98.0)));
        assert!(!triggers(&tp, &short_at(100.0, 102.0)));
    }

    // ── the trailing stop ────────────────────────────────────────────────────

    /// A trailing stop measures from the extreme, not the entry. A position
    /// that ran to +10% and gave back 3% must exit even though it is still
    /// profitable — the case a stop-loss cannot express and the reason the rule
    /// exists.
    #[test]
    fn a_trailing_stop_measures_from_the_extreme_not_the_entry() {
        let trail = ExitRule::TrailingStop { fraction: dec!(0.03) };
        let mut s = long_at(100.0, 107.0);
        s.favourable_extreme = dec!(110);
        assert!(!triggers(&trail, &s), "110 → 107 is 2.73%, under the 3% band");

        s.current_price = dec!(106);
        assert!(
            triggers(&trail, &s),
            "110 → 106 is a 3.6% retrace and must fire even though the position is still              +6% on the entry — the case a stop-loss cannot express"
        );

        s.current_price = dec!(109);
        assert!(!triggers(&trail, &s), "110 → 109 is under 1%");
    }

    #[test]
    fn a_short_trailing_stop_ratchets_downward() {
        let trail = ExitRule::TrailingStop { fraction: dec!(0.03) };
        let mut s = short_at(100.0, 93.0);
        s.favourable_extreme = dec!(90);
        assert!(triggers(&trail, &s), "90 → 93 is a 3.3% retrace on a short");
        s.current_price = dec!(91);
        assert!(!triggers(&trail, &s));
    }

    // ── time and signal ──────────────────────────────────────────────────────

    #[test]
    fn the_time_exit_fires_at_the_bound_not_after_it() {
        let t = ExitRule::MaxHoldingBars { bars: 5 };
        let mut s = long_at(100.0, 100.0);
        s.bars_held = 4;
        assert!(!triggers(&t, &s));
        s.bars_held = 5;
        assert!(triggers(&t, &s));
    }

    #[test]
    fn the_signal_exit_matches_by_name() {
        let r = ExitRule::OnSignal { emit: "flat".into() };
        let mut s = long_at(100.0, 100.0);
        assert!(!triggers(&r, &s));
        s.signals = vec!["long".into()];
        assert!(!triggers(&r, &s));
        s.signals = vec!["long".into(), "flat".into()];
        assert!(triggers(&r, &s));
    }

    // ── ordering ─────────────────────────────────────────────────────────────

    /// Declared order is semantic. On a bar that gaps through both the stop and
    /// the target, which one you are deemed to have hit is the difference
    /// between a win and a loss, and nothing in the platform is allowed to
    /// reorder that silently.
    #[test]
    fn declaration_order_decides_which_rule_fires_on_a_bar_that_breaches_both() {
        let mut s = long_at(100.0, 95.0);
        s.favourable_extreme = dec!(106);

        let stop_first = ExitPolicy::Rules {
            rules: vec![
                ExitRule::StopLoss { fraction: dec!(0.02) },
                ExitRule::TrailingStop { fraction: dec!(0.03) },
            ],
        };
        let trail_first = ExitPolicy::Rules {
            rules: vec![
                ExitRule::TrailingStop { fraction: dec!(0.03) },
                ExitRule::StopLoss { fraction: dec!(0.02) },
            ],
        };
        assert_eq!(first_triggered(&stop_first, &s).unwrap().kind(), "stop_loss");
        assert_eq!(first_triggered(&trail_first, &s).unwrap().kind(), "trailing_stop");
    }

    #[test]
    fn nothing_fires_when_nothing_is_breached() {
        let p = ExitPolicy::Rules {
            rules: vec![
                ExitRule::StopLoss { fraction: dec!(0.05) },
                ExitRule::MaxHoldingBars { bars: 100 },
            ],
        };
        assert!(first_triggered(&p, &long_at(100.0, 101.0)).is_none());
    }

    // ── well-formedness ──────────────────────────────────────────────────────

    /// A stop at 150% can never fire on a long and a stop at 0% fires on the
    /// entry bar. Both are typos, and the validator refuses both.
    #[test]
    fn fractions_outside_zero_to_one_are_not_well_formed() {
        assert!(ExitRule::StopLoss { fraction: dec!(0.02) }.is_well_formed());
        assert!(ExitRule::StopLoss { fraction: dec!(1) }.is_well_formed());
        assert!(!ExitRule::StopLoss { fraction: dec!(1.5) }.is_well_formed());
        assert!(!ExitRule::StopLoss { fraction: Decimal::ZERO }.is_well_formed());
        assert!(!ExitRule::TakeProfit { fraction: dec!(-0.1) }.is_well_formed());
        assert!(!ExitRule::MaxHoldingBars { bars: 0 }.is_well_formed());
        assert!(!ExitRule::OnSignal { emit: "  ".into() }.is_well_formed());
    }

    /// Only the time exit bounds holding period. Reading a stop-loss as one is
    /// how a candidate gets admitted below the tradeable frontier.
    #[test]
    fn only_the_time_exit_bounds_the_holding_period() {
        assert_eq!(ExitRule::MaxHoldingBars { bars: 30 }.holding_bound(), Some(30));
        assert_eq!(ExitRule::StopLoss { fraction: dec!(0.02) }.holding_bound(), None);
        assert_eq!(ExitRule::TrailingStop { fraction: dec!(0.02) }.holding_bound(), None);
    }

    #[test]
    fn rules_round_trip_through_json_with_their_tag() {
        let p = ExitPolicy::Rules {
            rules: vec![
                ExitRule::StopLoss { fraction: dec!(0.02) },
                ExitRule::TrailingStop { fraction: dec!(0.03) },
                ExitRule::MaxHoldingBars { bars: 240 },
                ExitRule::OnSignal { emit: "flat".into() },
            ],
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains(r#""rule":"stop_loss""#), "{s}");
        assert_eq!(serde_json::from_str::<ExitPolicy>(&s).unwrap(), p);
    }

    #[test]
    fn a_zero_entry_price_yields_no_verdict_rather_than_a_division_by_zero() {
        let mut s = long_at(100.0, 100.0);
        s.entry_price = Decimal::ZERO;
        assert!(s.pnl_fraction().is_none());
        assert!(!triggers(&ExitRule::StopLoss { fraction: dec!(0.02) }, &s));
    }
}
