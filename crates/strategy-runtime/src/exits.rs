//! Driving the exit policy against live position state (ADR-P6-01, ADR-P6-09).
//!
//! [`domain::strategy_def::exits`] says what an exit *is*. This says when it
//! fires, and it is the half where backtests lie.
//!
//! ## The intrabar problem
//!
//! A stop-loss evaluated against the close is wrong in the strategy's favour: a
//! bar that traded down through the stop and recovered by the close shows no
//! loss. So exits must be evaluated against the bar's extremes.
//!
//! But evaluating against both extremes creates the opposite lie. On a bar whose
//! low breached the stop *and* whose high breached the target, OHLCV cannot say
//! which came first — and a simulator that resolves the ambiguity in the
//! strategy's favour reports a win on every such bar. Over a long backtest that
//! single choice is worth more than most edges.
//!
//! [`IntrabarPolicy::WorstFirst`] is the default and the only one usable for
//! evidence: when both are breached, the adverse one is deemed to have happened
//! first. It is pessimistic and it is the only assumption that cannot be
//! exploited. [`IntrabarPolicy::CloseOnly`] exists because it is what a
//! bar-close runtime genuinely does, and naming it stops it from being mistaken
//! for the others.
//!
//! There is no `Optimistic` variant. An agent scored on backtests will find any
//! setting that resolves ambiguity in its favour, and the way to stop that is to
//! not build the setting.
//!
//! ## Same-bar entry
//!
//! A position opened on bar *t* is not eligible for a time exit until bar *t+1*
//! (`bars_held` is 0 on the entry bar and `max_holding_bars` requires ≥ 1), but
//! it **is** eligible for a stop: a position that gaps through its stop on the
//! entry bar really did. Treating the entry bar as protected is a subtle way of
//! giving every strategy one free bar of adverse movement.

use chrono::{DateTime, Utc};
use domain::strategy_def::exits::{
    first_triggered, ExitPolicy, ExitRule, PositionSide, PositionState,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// How to resolve a bar on which more than one exit level was breached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntrabarPolicy {
    /// Evaluate against the adverse extreme first: a bar that breached both the
    /// stop and the target is deemed to have hit the stop. The only assumption
    /// that cannot be exploited, and therefore the default.
    #[default]
    WorstFirst,
    /// Evaluate against the close only. What a bar-close live runtime actually
    /// does. Understates stop hits, so a backtest under this policy is not
    /// comparable with live fills under any other.
    CloseOnly,
}

/// One bar, as an exit rule sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub ts: DateTime<Utc>,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
}

/// An exit that fired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Triggered {
    /// Which rule, by its stable kind string.
    pub rule: String,
    /// The price the exit is deemed to have filled at.
    ///
    /// For a stop or target this is the **level**, not the bar's close: a stop
    /// at 98 on a bar that traded to 95 fills at 98, because that is where the
    /// resting order was. Filling at the extreme would be a windfall in one
    /// direction and a penalty in the other, and neither is what happened.
    ///
    /// Slippage against that level belongs to the fill model, not here.
    pub price: Decimal,
    pub ts: DateTime<Utc>,
    pub bars_held: u32,
}

/// Position state plus the policy that closes it.
///
/// Held by whoever owns the position — the live engine or the simulator — so
/// that both drive exits through one implementation and cannot drift.
#[derive(Clone, Debug)]
pub struct ExitTracker {
    policy: ExitPolicy,
    intrabar: IntrabarPolicy,
    open: Option<Open>,
}

#[derive(Clone, Debug)]
struct Open {
    side: PositionSide,
    entry_price: Decimal,
    favourable_extreme: Decimal,
    bars_held: u32,
}

impl ExitTracker {
    /// A tracker for `policy`, resolving intrabar ambiguity pessimistically.
    #[must_use]
    pub fn new(policy: ExitPolicy) -> Self {
        Self { policy, intrabar: IntrabarPolicy::WorstFirst, open: None }
    }

    /// Choose the intrabar policy explicitly. See [`IntrabarPolicy`] for why
    /// there is no optimistic option.
    #[must_use]
    pub fn with_intrabar(mut self, intrabar: IntrabarPolicy) -> Self {
        self.intrabar = intrabar;
        self
    }

    /// Whether a position is open.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Bars the current position has been held, or `None` when flat.
    #[must_use]
    pub fn bars_held(&self) -> Option<u32> {
        self.open.as_ref().map(|o| o.bars_held)
    }

    /// Record an opening fill. Replaces any existing position: a reversal is an
    /// exit and a new entry, and the new position's stop is measured from the
    /// new entry.
    pub fn opened(&mut self, side: PositionSide, entry_price: Decimal) {
        self.open = Some(Open {
            side,
            entry_price,
            favourable_extreme: entry_price,
            bars_held: 0,
        });
    }

    /// Record that the position closed for any reason — including one this
    /// tracker did not cause.
    pub fn closed(&mut self) {
        self.open = None;
    }

    /// Advance one bar and report the exit that fired, if any.
    ///
    /// `signals` are the names emitted on this bar, for
    /// [`ExitRule::OnSignal`]. Returns `None` when flat, when nothing fired, or
    /// when the policy has no rules.
    ///
    /// The tracker does **not** close itself on a trigger: the caller owns the
    /// position and may not be able to fill. Call [`ExitTracker::closed`] when
    /// the fill is done.
    pub fn on_bar(&mut self, bar: &Bar, signals: &[String]) -> Option<Triggered> {
        let open = self.open.as_mut()?;

        open.favourable_extreme = match open.side {
            PositionSide::Long => open.favourable_extreme.max(bar.high),
            PositionSide::Short => {
                if open.favourable_extreme.is_zero() {
                    bar.low
                } else {
                    open.favourable_extreme.min(bar.low)
                }
            }
        };

        let adverse = match open.side {
            PositionSide::Long => bar.low,
            PositionSide::Short => bar.high,
        };
        let favourable = match open.side {
            PositionSide::Long => bar.high,
            PositionSide::Short => bar.low,
        };

        // Probe prices in the order the policy dictates. `WorstFirst` asks the
        // adverse extreme before the favourable one, so a bar that breached both
        // resolves to the loss.
        let probes: &[Decimal] = match self.intrabar {
            IntrabarPolicy::WorstFirst => &[adverse, favourable, bar.close],
            IntrabarPolicy::CloseOnly => &[bar.close],
        };

        let mut fired: Option<(&ExitRule, Decimal)> = None;
        for probe in probes {
            let state = PositionState {
                side: open.side,
                entry_price: open.entry_price,
                favourable_extreme: open.favourable_extreme,
                bars_held: open.bars_held,
                current_price: *probe,
                signals: signals.to_vec(),
            };
            if let Some(rule) = first_triggered(&self.policy, &state) {
                fired = Some((rule, *probe));
                break;
            }
        }

        let result = fired.map(|(rule, probe)| Triggered {
            rule: rule.kind().to_string(),
            price: fill_level(rule, open, probe, bar),
            ts: bar.ts,
            bars_held: open.bars_held,
        });

        open.bars_held += 1;
        result
    }
}

/// Where a triggered rule is deemed to have filled.
///
/// A stop or target rests at a level, so it fills at that level — **clamped into
/// the range the bar actually traded**. The clamp is what handles a gap: a long
/// stop at 98 on a bar that opened at 95 and never traded above it fills at 95,
/// because filling at 98 would invent liquidity that was not there. In the other
/// direction a gap through a target fills at the bar's low, which is better than
/// the level and is what really would have happened.
///
/// Slippage against the fill belongs to the fill model, not here.
fn fill_level(rule: &ExitRule, open: &Open, probe: Decimal, bar: &Bar) -> Decimal {
    let sign = match open.side {
        PositionSide::Long => Decimal::ONE,
        PositionSide::Short => -Decimal::ONE,
    };
    let level = match rule {
        ExitRule::StopLoss { fraction } => open.entry_price * (Decimal::ONE - sign * *fraction),
        ExitRule::TakeProfit { fraction } => open.entry_price * (Decimal::ONE + sign * *fraction),
        ExitRule::TrailingStop { fraction } => {
            open.favourable_extreme * (Decimal::ONE - sign * *fraction)
        }
        // No level to rest at: these fill wherever the probe found them.
        ExitRule::MaxHoldingBars { .. } | ExitRule::OnSignal { .. } => return probe,
    };
    level.clamp(bar.low, bar.high)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn ts(n: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + n * 60, 0).unwrap()
    }

    fn bar(n: i64, high: Decimal, low: Decimal, close: Decimal) -> Bar {
        Bar { ts: ts(n), high, low, close }
    }

    fn policy(rules: Vec<ExitRule>) -> ExitPolicy {
        ExitPolicy::Rules { rules }
    }

    // ── the intrabar rule, which is the reason this module exists ────────────

    /// The bar that breached both. OHLCV cannot say which came first, and a
    /// simulator that guesses in the strategy's favour reports a win every time.
    #[test]
    fn a_bar_that_breaches_both_the_stop_and_the_target_resolves_to_the_stop() {
        let mut t = ExitTracker::new(policy(vec![
            ExitRule::TakeProfit { fraction: dec!(0.05) },
            ExitRule::StopLoss { fraction: dec!(0.02) },
        ]));
        t.opened(PositionSide::Long, dec!(100));

        // High 106 clears the 105 target; low 97 breaches the 98 stop.
        let fired = t.on_bar(&bar(1, dec!(106), dec!(97), dec!(104)), &[]).unwrap();
        assert_eq!(fired.rule, "stop_loss", "the adverse extreme is probed first");
        assert_eq!(fired.price, dec!(98), "fills at the stop level, not the bar low");
    }

    /// Order within the policy does not rescue the optimist: even with the
    /// target listed first, the *price probe* order decides.
    #[test]
    fn declaration_order_cannot_defeat_worst_first() {
        for rules in [
            vec![
                ExitRule::TakeProfit { fraction: dec!(0.05) },
                ExitRule::StopLoss { fraction: dec!(0.02) },
            ],
            vec![
                ExitRule::StopLoss { fraction: dec!(0.02) },
                ExitRule::TakeProfit { fraction: dec!(0.05) },
            ],
        ] {
            let mut t = ExitTracker::new(policy(rules));
            t.opened(PositionSide::Long, dec!(100));
            let fired = t.on_bar(&bar(1, dec!(106), dec!(97), dec!(104)), &[]).unwrap();
            assert_eq!(fired.rule, "stop_loss");
        }
    }

    #[test]
    fn a_short_resolves_its_adverse_extreme_first_too() {
        let mut t = ExitTracker::new(policy(vec![
            ExitRule::TakeProfit { fraction: dec!(0.05) },
            ExitRule::StopLoss { fraction: dec!(0.02) },
        ]));
        t.opened(PositionSide::Short, dec!(100));
        // Adverse for a short is the high: 103 breaches the 102 stop.
        let fired = t.on_bar(&bar(1, dec!(103), dec!(94), dec!(96)), &[]).unwrap();
        assert_eq!(fired.rule, "stop_loss");
        assert_eq!(fired.price, dec!(102));
    }

    /// Close-only understates stop hits. Named so nobody mistakes it for the
    /// evidence-grade policy.
    #[test]
    fn close_only_misses_a_stop_the_bar_traded_through() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]))
            .with_intrabar(IntrabarPolicy::CloseOnly);
        t.opened(PositionSide::Long, dec!(100));
        assert!(t.on_bar(&bar(1, dec!(101), dec!(95), dec!(100)), &[]).is_none());

        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened(PositionSide::Long, dec!(100));
        assert!(t.on_bar(&bar(1, dec!(101), dec!(95), dec!(100)), &[]).is_some());
    }

    // ── fill levels ──────────────────────────────────────────────────────────

    /// A resting stop fills where it rested, not at the bar's extreme.
    #[test]
    fn a_stop_fills_at_its_level_when_the_bar_traded_through_it() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened(PositionSide::Long, dec!(100));
        let fired = t.on_bar(&bar(1, dec!(100), dec!(90), dec!(91)), &[]).unwrap();
        assert_eq!(fired.price, dec!(98));
    }

    /// A gap opens past the stop and never trades back to it. Filling at the
    /// level would invent liquidity that was not there.
    #[test]
    fn a_gap_through_the_stop_fills_at_the_gap_not_at_the_level() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened(PositionSide::Long, dec!(100));
        // The whole bar is below the 98 stop: high 95.
        let fired = t.on_bar(&bar(1, dec!(95), dec!(93), dec!(94)), &[]).unwrap();
        assert_eq!(fired.price, dec!(95), "the best price the bar actually traded");
    }

    #[test]
    fn a_target_fills_at_its_level_rather_than_the_bar_high() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::TakeProfit { fraction: dec!(0.05) }]));
        t.opened(PositionSide::Long, dec!(100));
        let fired = t.on_bar(&bar(1, dec!(120), dec!(100), dec!(118)), &[]).unwrap();
        assert_eq!(fired.price, dec!(105), "no windfall from the bar's extreme");
    }

    // ── the trailing stop ratchets across bars ───────────────────────────────

    #[test]
    fn the_trailing_stop_follows_the_running_high_and_fires_on_the_retrace() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::TrailingStop { fraction: dec!(0.03) }]));
        t.opened(PositionSide::Long, dec!(100));

        // Each bar's own low must stay inside the 3% band, or the ratchet fires
        // within the bar that set the new high — which is correct, and not what
        // this test is measuring.
        assert!(t.on_bar(&bar(1, dec!(105), dec!(103), dec!(104)), &[]).is_none());
        assert!(t.on_bar(&bar(2, dec!(110), dec!(108.5), dec!(109)), &[]).is_none());

        // Extreme is now 110; the stop sits at 106.7.
        let fired = t.on_bar(&bar(3, dec!(109), dec!(105), dec!(106)), &[]).unwrap();
        assert_eq!(fired.rule, "trailing_stop");
        assert!(fired.price > dec!(106) && fired.price < dec!(107), "got {}", fired.price);
    }

    /// The trailing stop never loosens, even after a deep retrace that did not
    /// fire. A ratchet that un-ratchets is not a trailing stop.
    #[test]
    fn the_extreme_never_moves_backwards() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::TrailingStop { fraction: dec!(0.50) }]));
        t.opened(PositionSide::Long, dec!(100));
        t.on_bar(&bar(1, dec!(200), dec!(100), dec!(150)), &[]);
        // A much lower bar must not reset the extreme to 150.
        let fired = t.on_bar(&bar(2, dec!(150), dec!(99), dec!(100)), &[]).unwrap();
        assert_eq!(fired.rule, "trailing_stop", "measured from 200, not from 150");
    }

    // ── time ─────────────────────────────────────────────────────────────────

    /// The entry bar is bar zero. A `max_holding_bars: 2` closes on the second
    /// bar after entry, not the first.
    #[test]
    fn the_time_exit_counts_from_the_entry_bar() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::MaxHoldingBars { bars: 2 }]));
        t.opened(PositionSide::Long, dec!(100));
        let flat = |n| bar(n, dec!(100), dec!(100), dec!(100));

        assert!(t.on_bar(&flat(1), &[]).is_none(), "bars_held 0");
        assert!(t.on_bar(&flat(2), &[]).is_none(), "bars_held 1");
        let fired = t.on_bar(&flat(3), &[]).unwrap();
        assert_eq!(fired.rule, "max_holding_bars");
        assert_eq!(fired.bars_held, 2);
    }

    /// Giving a position one protected bar is a quiet gift of one bar of
    /// adverse movement to every strategy on the platform.
    #[test]
    fn a_stop_can_fire_on_the_entry_bar() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened(PositionSide::Long, dec!(100));
        let fired = t.on_bar(&bar(1, dec!(100), dec!(90), dec!(95)), &[]).unwrap();
        assert_eq!(fired.bars_held, 0);
    }

    // ── signals and lifecycle ────────────────────────────────────────────────

    #[test]
    fn the_signal_exit_fires_on_a_matching_emit() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::OnSignal { emit: "flat".into() }]));
        t.opened(PositionSide::Long, dec!(100));
        let b = bar(1, dec!(101), dec!(99), dec!(100));
        assert!(t.on_bar(&b, &["long".into()]).is_none());
        assert_eq!(t.on_bar(&b, &["flat".into()]).unwrap().rule, "on_signal");
    }

    #[test]
    fn a_flat_tracker_reports_nothing() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        assert!(!t.is_open());
        assert_eq!(t.bars_held(), None);
        assert!(t.on_bar(&bar(1, dec!(1), dec!(1), dec!(1)), &[]).is_none());
    }

    #[test]
    fn risk_layer_only_never_fires() {
        let mut t = ExitTracker::new(ExitPolicy::RiskLayerOnly);
        t.opened(PositionSide::Long, dec!(100));
        assert!(t.on_bar(&bar(1, dec!(1000), dec!(1), dec!(1)), &["flat".into()]).is_none());
    }

    /// A reversal resets the reference. A stop measured from the *previous*
    /// entry would fire immediately on the new position.
    #[test]
    fn reopening_resets_the_entry_and_the_extreme() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened(PositionSide::Long, dec!(100));
        t.on_bar(&bar(1, dec!(120), dec!(100), dec!(119)), &[]);
        t.opened(PositionSide::Long, dec!(120));
        assert_eq!(t.bars_held(), Some(0));
        assert!(t.on_bar(&bar(2, dec!(121), dec!(119), dec!(120)), &[]).is_none());
    }

    /// The tracker reports; the caller fills. A tracker that closed itself would
    /// mark a position flat that the venue had not yet closed.
    #[test]
    fn a_trigger_does_not_close_the_position_by_itself() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened(PositionSide::Long, dec!(100));
        assert!(t.on_bar(&bar(1, dec!(100), dec!(90), dec!(91)), &[]).is_some());
        assert!(t.is_open(), "the caller owns the fill");
        t.closed();
        assert!(!t.is_open());
    }

    #[test]
    fn worst_first_is_the_default() {
        assert_eq!(IntrabarPolicy::default(), IntrabarPolicy::WorstFirst);
    }
}
