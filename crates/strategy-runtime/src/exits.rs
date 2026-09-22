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
//!
//! That is the convention for a fill at the **start** of a bar. A fill at a
//! bar's **close** is a different clock and needs
//! [`ExitTracker::opened_at_bar_close`]: the entry bar's high and low happened
//! before the position existed, so the entry bar is not a bar it was exposed
//! for, and the first bar after it is holding bar one. Calling
//! [`ExitTracker::opened`] for a close fill reports every holding period one
//! bar short and fires every `max_holding_bars` one bar late.

use chrono::{DateTime, Utc};
use domain::payloads::bar::Timeframe;
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

    /// Record an opening fill that happened at the **close** of a bar, with
    /// that bar already consumed.
    ///
    /// The backtest simulator submits market orders from a bar handler that
    /// runs after the venue has processed the bar, so an entry fills at that
    /// bar's close. The entry bar's extremes are then prices the position was
    /// not in the market for, and the caller must not pass that bar to
    /// [`ExitTracker::on_bar`]. The first bar it *does* pass is holding bar
    /// one, which is what this sets up: [`ExitRule::MaxHoldingBars`] with
    /// `bars: 1` fires on the bar after the entry, not two bars after it.
    ///
    /// Everything else is [`ExitTracker::opened`]: a stop can still fire on the
    /// very first bar the position is exposed for.
    pub fn opened_at_bar_close(&mut self, side: PositionSide, entry_price: Decimal) {
        self.opened(side, entry_price);
        if let Some(open) = self.open.as_mut() {
            open.bars_held = 1;
        }
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
    ///
    /// This is the whole-bar form, for a caller whose feed delivers one event
    /// per bar — the backtest simulator. A caller whose feed is finer than a
    /// bar drives the same evaluation through
    /// [`ExitTracker::on_bar_update`] and marks the bar boundary with
    /// [`ExitTracker::bar_closed`]; `on_bar` is exactly those two in order, so
    /// there is one evaluation and one holding-bar count for both.
    pub fn on_bar(&mut self, bar: &Bar, signals: &[String]) -> Option<Triggered> {
        let fired = self.on_bar_update(bar, signals);
        self.bar_closed();
        fired
    }

    /// Evaluate the policy against the bar **so far**, without advancing the
    /// holding-bar count.
    ///
    /// A live feed is finer-grained than the bar the strategy declared: the
    /// hot path publishes one event per trade, so a one-minute strategy sees
    /// many events inside one minute. Waiting for the minute to close before
    /// asking whether the stop was breached would leave a position exposed for
    /// up to a bar past its own stop — the exact bound the declaration exists
    /// to impose. Waiting is therefore not an option, and counting each tick as
    /// a bar is not either: `max_holding_bars: 5` would mean five minutes in
    /// replay and five trades live.
    ///
    /// Splitting the two lets the caller do the right thing on both axes:
    /// evaluate prices as often as the feed allows, and advance the holding
    /// count once per declared bar. The evaluation is the same code either
    /// way, including the intrabar policy.
    ///
    /// `bar` is the *running* bar — the extremes reached inside the current
    /// bar up to now, and the latest price as its close. Calling this twice on
    /// the same bar is safe: the favourable extreme only ratchets, and a rule
    /// that fired on the first call fires on the second.
    pub fn on_bar_update(&mut self, bar: &Bar, signals: &[String]) -> Option<Triggered> {
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

        fired.map(|(rule, probe)| Triggered {
            rule: rule.kind().to_string(),
            price: fill_level(rule, open, probe, bar),
            ts: bar.ts,
            bars_held: open.bars_held,
        })
    }

    /// Mark the end of one declared bar.
    ///
    /// The holding-bar count — and therefore [`ExitRule::MaxHoldingBars`] —
    /// advances here and nowhere else, so it counts bars of the timeframe the
    /// strategy declared whatever granularity the feed arrives at. A no-op
    /// when flat: bars that elapse with no position are not held.
    pub fn bar_closed(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.bars_held += 1;
        }
    }
}

/// Folds a feed of any granularity into the bars the strategy declared.
///
/// The backtest feed delivers one event per bar, so its bars need no
/// assembling. The live feed does not: the hot path publishes one
/// `WorldEvent::Bar` per trade tick, each carrying that tick's price as all
/// four of O/H/L/C. Left alone, an [`ExitTracker`] driven from it would count
/// trades where the strategy declared minutes.
///
/// This is where the two feeds become the same sequence of bars, so that
/// `max_holding_bars` means the same thing in both (SC-3, ADR-P6-14). It is
/// deliberately part of the shared module rather than the live caller's: a
/// second place that decides where a bar ends is a second place the two paths
/// can disagree.
#[derive(Clone, Debug)]
pub struct BarAggregator {
    bucket_secs: i64,
    open: Option<Running>,
}

#[derive(Clone, Debug)]
struct Running {
    bucket: i64,
    ts: DateTime<Utc>,
    high: Decimal,
    low: Decimal,
    close: Decimal,
}

/// One folded observation.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    /// The declared bar as it stands *now*: the extremes reached inside it so
    /// far, and the latest price as its close.
    pub bar: Bar,
    /// Whether this observation opened a new declared bar, meaning the
    /// previous one is complete. The caller advances the holding count here
    /// and nowhere else.
    pub opened_new_bar: bool,
}

impl BarAggregator {
    /// An aggregator for `timeframe`.
    #[must_use]
    pub fn new(timeframe: Timeframe) -> Self {
        Self {
            bucket_secs: bar_seconds(timeframe),
            open: None,
        }
    }

    /// Whether a bar is currently in progress.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Fold one event into the bar in progress, or start a new one.
    ///
    /// Bucketing is by `ts` floored to the timeframe, which is correct whether
    /// the feed stamps a bar with its open or its close: both are stable
    /// within a bar and at least one bar apart between bars.
    pub fn observe(
        &mut self,
        ts: DateTime<Utc>,
        high: Decimal,
        low: Decimal,
        close: Decimal,
    ) -> Observed {
        let bucket = ts.timestamp().div_euclid(self.bucket_secs);
        let opened_new_bar = match self.open.as_mut() {
            Some(run) if run.bucket == bucket => {
                run.ts = ts;
                run.high = run.high.max(high);
                run.low = run.low.min(low);
                run.close = close;
                false
            }
            _ => {
                self.open = Some(Running { bucket, ts, high, low, close });
                // The very first bar of a position opens no *previous* bar, and
                // `ExitTracker::bar_closed` is a no-op while flat in any case.
                true
            }
        };
        let run = self.open.as_ref().expect("just set");
        Observed {
            bar: Bar { ts: run.ts, high: run.high, low: run.low, close: run.close },
            opened_new_bar,
        }
    }

    /// Restart the bar in progress from `price`, discarding the extremes it
    /// reached before now, without opening a new declared bar.
    ///
    /// Called when a position opens part-way through a bar. The prices that
    /// bar traded at before the fill are prices the position was never exposed
    /// to, and letting them stop it out is the defect ADR-P6-11 named: it
    /// would hand every strategy a stop measured against a market it was not
    /// in. The bar itself is unchanged — this is the position's clock, not the
    /// market's.
    pub fn restart_at(&mut self, price: Decimal) {
        if let Some(run) = self.open.as_mut() {
            run.high = price;
            run.low = price;
            run.close = price;
        }
    }
}

/// The netted position an exit policy is tracking, and nothing else.
///
/// This decides *how much* to flatten, never *whether* to — every "should this
/// position close" question belongs to [`ExitTracker`] and is asked there
/// (INV-14). It is here rather than in either caller because both the live
/// runtime and the simulator have to answer the same three questions the same
/// way: what an averaging-in fill does to the entry reference, what a partial
/// close does to it, and what a reversal does.
///
/// `crates/backtest/src/sim.rs` still carries a private twin of this
/// (`SimPosition`), written first and byte-for-byte equivalent. Unifying them
/// was out of this track's file scope; it is a deletion, not a change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetPosition {
    pub side: PositionSide,
    pub qty: Decimal,
    pub entry: Decimal,
}

/// What a fill did to the netted position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NetEffect {
    /// Untouched, or reduced without changing the entry reference.
    Unchanged,
    /// Opened, averaged into, or reversed — the entry reference moved.
    Reopened { side: PositionSide, entry: Decimal },
    /// Back to flat.
    Flat,
}

impl NetPosition {
    /// Apply one fill of `qty` at `fill` on `side` to `pos`.
    pub fn apply(
        pos: &mut Option<Self>,
        side: PositionSide,
        qty: Decimal,
        fill: Decimal,
    ) -> NetEffect {
        if qty <= Decimal::ZERO {
            return NetEffect::Unchanged;
        }
        match *pos {
            None => {
                *pos = Some(Self { side, qty, entry: fill });
                NetEffect::Reopened { side, entry: fill }
            }
            Some(mut open) if open.side == side => {
                let total = open.qty + qty;
                open.entry = (open.entry * open.qty + fill * qty) / total;
                open.qty = total;
                *pos = Some(open);
                NetEffect::Reopened { side, entry: open.entry }
            }
            Some(mut open) => match qty.cmp(&open.qty) {
                std::cmp::Ordering::Less => {
                    open.qty -= qty;
                    *pos = Some(open);
                    NetEffect::Unchanged
                }
                std::cmp::Ordering::Equal => {
                    *pos = None;
                    NetEffect::Flat
                }
                // A reversal is an exit and a new entry, and the new position's
                // stop is measured from the new entry.
                std::cmp::Ordering::Greater => {
                    *pos = Some(Self { side, qty: qty - open.qty, entry: fill });
                    NetEffect::Reopened { side, entry: fill }
                }
            },
        }
    }
}

/// Seconds in one bar of `timeframe`.
///
/// Shared with the history registers so a cross is measured over the same unit
/// a holding period is counted in. A live feed publishing one event per trade
/// would otherwise have `crossed_above` comparing two trades where the
/// strategy declared minutes.
pub(crate) const fn bar_seconds(timeframe: Timeframe) -> i64 {
    match timeframe {
        Timeframe::Seconds1 => 1,
        Timeframe::Minutes1 => 60,
        Timeframe::Minutes5 => 300,
        Timeframe::Minutes15 => 900,
        Timeframe::Hours1 => 3_600,
        Timeframe::Hours4 => 14_400,
        Timeframe::Daily => 86_400,
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

    /// A fill at the close of bar *t* means bar *t+1* is the first bar held.
    /// Counting it as bar zero fires every time exit one bar late, which reads
    /// as a strategy that holds longer than it declared.
    #[test]
    fn a_close_fill_counts_the_next_bar_as_the_first_bar_held() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::MaxHoldingBars { bars: 1 }]));
        t.opened_at_bar_close(PositionSide::Long, dec!(100));
        let flat = |n| bar(n, dec!(100), dec!(100), dec!(100));

        let fired = t.on_bar(&flat(1), &[]).expect("one bar held is one bar held");
        assert_eq!(fired.rule, "max_holding_bars");
        assert_eq!(fired.bars_held, 1);
    }

    /// The close-fill convention must not buy the position a protected bar:
    /// the first bar it is exposed for can stop it out.
    #[test]
    fn a_close_fill_is_still_stoppable_on_its_first_exposed_bar() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::StopLoss { fraction: dec!(0.02) }]));
        t.opened_at_bar_close(PositionSide::Long, dec!(100));
        let fired = t.on_bar(&bar(1, dec!(100), dec!(90), dec!(95)), &[]).unwrap();
        assert_eq!(fired.rule, "stop_loss");
        assert_eq!(fired.price, dec!(98));
    }

    /// The two conventions differ by exactly one bar, and by nothing else.
    #[test]
    fn the_two_entry_conventions_differ_by_one_bar_of_holding() {
        let flat = |n| bar(n, dec!(100), dec!(100), dec!(100));
        let rules = || policy(vec![ExitRule::MaxHoldingBars { bars: 2 }]);

        let mut open_fill = ExitTracker::new(rules());
        open_fill.opened(PositionSide::Long, dec!(100));
        let mut close_fill = ExitTracker::new(rules());
        close_fill.opened_at_bar_close(PositionSide::Long, dec!(100));

        assert!(open_fill.on_bar(&flat(1), &[]).is_none());
        assert!(close_fill.on_bar(&flat(1), &[]).is_none());
        assert!(open_fill.on_bar(&flat(2), &[]).is_none(), "bars_held 1");
        assert!(close_fill.on_bar(&flat(2), &[]).is_some(), "two bars held");
        assert!(open_fill.on_bar(&flat(3), &[]).is_some());
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

    // ── the split form, for a feed finer than a bar (ADR-P6-14) ──────────────

    /// `on_bar` must remain exactly `on_bar_update` followed by `bar_closed`,
    /// or the simulator and the live runtime are counting different things.
    #[test]
    fn the_whole_bar_form_is_the_split_form_in_order() {
        let bars = [
            bar(1, dec!(101), dec!(99), dec!(100)),
            bar(2, dec!(103), dec!(97), dec!(99)),
            bar(3, dec!(104), dec!(96), dec!(98)),
        ];
        let rules = || {
            policy(vec![
                ExitRule::MaxHoldingBars { bars: 3 },
                ExitRule::StopLoss { fraction: dec!(0.05) },
            ])
        };

        let mut whole = ExitTracker::new(rules());
        let mut split = ExitTracker::new(rules());
        whole.opened(PositionSide::Long, dec!(100));
        split.opened(PositionSide::Long, dec!(100));

        for b in &bars {
            let a = whole.on_bar(b, &[]);
            let c = split.on_bar_update(b, &[]);
            split.bar_closed();
            assert_eq!(a, c, "the two forms disagreed on {}", b.ts);
            assert_eq!(whole.bars_held(), split.bars_held());
        }
    }

    /// Many updates inside one bar are one bar of holding. Counting each
    /// update would make `max_holding_bars: 2` mean two trades live and two
    /// minutes in replay.
    #[test]
    fn updates_within_one_bar_do_not_advance_the_holding_count() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::MaxHoldingBars { bars: 2 }]));
        t.opened(PositionSide::Long, dec!(100));

        // Twenty ticks inside bar one; none of them is a second bar.
        for _ in 0..20 {
            assert!(t.on_bar_update(&bar(1, dec!(100), dec!(100), dec!(100)), &[]).is_none());
        }
        assert_eq!(t.bars_held(), Some(0));
        t.bar_closed();

        for _ in 0..20 {
            assert!(t.on_bar_update(&bar(2, dec!(100), dec!(100), dec!(100)), &[]).is_none());
        }
        t.bar_closed();

        let fired = t
            .on_bar_update(&bar(3, dec!(100), dec!(100), dec!(100)), &[])
            .expect("two bars held");
        assert_eq!(fired.rule, "max_holding_bars");
        assert_eq!(fired.bars_held, 2);
    }

    /// The running-bar form still resolves a bar that breached both adversely.
    /// The intrabar policy is not something the split form gets to skip.
    #[test]
    fn a_running_bar_that_breached_both_still_resolves_to_the_stop() {
        let mut t = ExitTracker::new(policy(vec![
            ExitRule::TakeProfit { fraction: dec!(0.05) },
            ExitRule::StopLoss { fraction: dec!(0.02) },
        ]));
        t.opened(PositionSide::Long, dec!(100));
        let fired = t
            .on_bar_update(&bar(1, dec!(106), dec!(97), dec!(104)), &[])
            .unwrap();
        assert_eq!(fired.rule, "stop_loss");
        assert_eq!(fired.price, dec!(98));
    }

    // ── the aggregator: one bar sequence from either feed ────────────────────

    /// Sixty ticks inside a minute are one minute.
    #[test]
    fn ticks_inside_one_minute_are_one_declared_bar() {
        let mut agg = BarAggregator::new(Timeframe::Minutes1);
        // Minute-aligned on purpose: this test is about what happens *inside*
        // one bar, and the boundary is what the next assertion measures.
        let base = DateTime::from_timestamp(1_699_999_980, 0).unwrap();
        let mut opened = 0;
        for s in 0..60 {
            let o = agg.observe(base + chrono::Duration::seconds(s), dec!(100), dec!(100), dec!(100));
            if o.opened_new_bar {
                opened += 1;
            }
        }
        assert_eq!(opened, 1, "one minute is one bar, not sixty");

        let o = agg.observe(base + chrono::Duration::seconds(60), dec!(100), dec!(100), dec!(100));
        assert!(o.opened_new_bar, "the next minute is the next bar");
    }

    /// The running bar carries the extremes the position was exposed to, not
    /// only the latest tick. A stop must see the low the minute traded to.
    #[test]
    fn the_running_bar_keeps_the_extremes_reached_inside_it() {
        let mut agg = BarAggregator::new(Timeframe::Minutes1);
        let base = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        agg.observe(base, dec!(100), dec!(100), dec!(100));
        agg.observe(base + chrono::Duration::seconds(10), dec!(95), dec!(95), dec!(95));
        let o = agg.observe(base + chrono::Duration::seconds(20), dec!(103), dec!(103), dec!(103));
        assert_eq!(o.bar.high, dec!(103));
        assert_eq!(o.bar.low, dec!(95));
        assert_eq!(o.bar.close, dec!(103));
    }

    /// A one-event-per-bar feed produces exactly one declared bar per event,
    /// whether the stamp is the bar's open or its close.
    #[test]
    fn a_bar_per_event_feed_opens_one_declared_bar_per_event() {
        for offset in [0, 60] {
            let mut agg = BarAggregator::new(Timeframe::Minutes1);
            let base = DateTime::from_timestamp(1_700_000_000 + offset, 0).unwrap();
            for n in 0..5 {
                let o = agg.observe(
                    base + chrono::Duration::seconds(60 * n),
                    dec!(101),
                    dec!(99),
                    dec!(100),
                );
                assert!(o.opened_new_bar, "event {n} at offset {offset}");
            }
        }
    }

    /// The prices a bar traded at before the position existed must not be able
    /// to stop it out.
    #[test]
    fn restarting_discards_the_extremes_from_before_the_fill() {
        let mut agg = BarAggregator::new(Timeframe::Minutes1);
        let base = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        agg.observe(base, dec!(90), dec!(90), dec!(90));
        agg.restart_at(dec!(100));
        let o = agg.observe(base + chrono::Duration::seconds(5), dec!(101), dec!(101), dec!(101));
        assert!(!o.opened_new_bar, "still the same minute");
        assert_eq!(o.bar.low, dec!(100), "the 90 happened before the position");
        assert_eq!(o.bar.high, dec!(101));
    }

    /// A bar that elapses while flat is not a bar held. Otherwise a position
    /// opened after a long quiet stretch inherits somebody else's clock.
    #[test]
    fn bars_that_close_while_flat_are_not_counted() {
        let mut t = ExitTracker::new(policy(vec![ExitRule::MaxHoldingBars { bars: 1 }]));
        for _ in 0..5 {
            t.bar_closed();
        }
        t.opened(PositionSide::Long, dec!(100));
        assert_eq!(t.bars_held(), Some(0));
    }
}
