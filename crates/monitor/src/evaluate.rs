//! Turning readings into decisions (COMP-007 §3, §4, §4.1).
//!
//! One cadence produces a pile of numbers. This module is the part that decides
//! which of them mean anything: it applies the declared limit, then dwell, then the
//! state machine — in that order, and never any other, because skipping dwell is
//! how a 3σ band on a noisy metric becomes an alarm every few minutes.
//!
//! Facts skip dwell. A reconciliation mismatch, a lost lease and a feed stale past
//! the hard multiple are not noisy samples of a distribution; they are the book and
//! the venue disagreeing about what is owned, the safety net being gone, and the
//! strategy trading on prices that are not current.

use std::collections::BTreeMap;

use crate::dwell::{DwellConfig, DwellTracker, DwellVerdict};
use crate::family::{Fact, Family};
use crate::limits::{ControlLimit, LimitBook, LimitSeverity};
use crate::state::{Severity, StrategyState, Transition};

/// Staleness past this multiple of the bar timeframe is a warning (COMP-007 §3).
pub const WARN_STALENESS_MULTIPLE: i64 = 3;

/// Staleness past this multiple halts the strategy, without dwell (§3, §4.1).
pub const HARD_STALENESS_MULTIPLE: i64 = 10;

/// The soft band has to be reachable before the hard one, or there is no warning
/// band at all — every stale feed would jump straight to a halt and the graded
/// response of §3's Data row would exist only on paper.
///
/// A compile error rather than a test, because the two numbers are constants: the
/// mistake is made while editing this file and should be caught there.
const _: () = assert!(WARN_STALENESS_MULTIPLE < HARD_STALENESS_MULTIPLE);

/// One number, measured this cadence.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricReading {
    pub strategy_id: String,
    pub metric: String,
    pub family: Family,
    pub value: f64,
}

impl MetricReading {
    #[must_use]
    pub fn new(strategy_id: &str, metric: &str, family: Family, value: f64) -> Self {
        Self {
            strategy_id: strategy_id.to_owned(),
            metric: metric.to_owned(),
            family,
            value,
        }
    }
}

/// What one reading means, once the limit and dwell have been applied.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// No limit is declared for this metric.
    ///
    /// Distinct from `Inside`, and deliberately so: "inside its limit" and "there
    /// is no limit" look identical on a dashboard and mean opposite things. A
    /// strategy whose every metric is undeclared would otherwise read as healthy.
    Undeclared,
    /// Inside, and nothing was in force.
    Inside,
    /// Outside, but not for long enough. Recorded; acted on by nothing.
    Blip,
    /// Outside for the full dwell. Act.
    Act(Severity),
    /// Inside for the full recovery window. Release an automatic scale-down.
    Release,
    /// Inside, still serving a dwell-released action.
    Recovering,
}

/// A reading, its verdict, and the limit it was judged against.
#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub reading: MetricReading,
    pub outcome: Outcome,
    pub limit_value: Option<f64>,
    pub dwell: Option<DwellVerdict>,
}

/// Which strategies depend on which instrument, so a stale feed halts the
/// strategies that use it **and only those** (COMP-007 §3, S-7.5).
#[derive(Clone, Debug, Default)]
pub struct DependencyMap {
    by_instrument: BTreeMap<String, Vec<String>>,
}

impl DependencyMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, instrument: &str, strategy_id: &str) {
        let entry = self.by_instrument.entry(instrument.to_owned()).or_default();
        if !entry.iter().any(|s| s == strategy_id) {
            entry.push(strategy_id.to_owned());
        }
    }

    #[must_use]
    pub fn strategies_for(&self, instrument: &str) -> &[String] {
        self.by_instrument
            .get(instrument)
            .map_or(&[][..], Vec::as_slice)
    }
}

/// What a feed's age means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Staleness {
    Fresh,
    /// Past `3 ×` the timeframe. Goes through dwell like any other estimate.
    Warning {
        instrument: String,
        age_secs: i64,
    },
    /// Past `10 ×`. A fact: skips dwell, and halts every strategy that depends
    /// on the instrument (COMP-007 §3, §4.1; ADR-D-11). Not a scale-down —
    /// scaling to a quarter still trades on a stale price.
    Hard(Fact),
}

/// Classify a feed's age against its own timeframe.
///
/// Against its *own* timeframe, not a fixed number of seconds: a daily bar that is
/// four minutes old is fine and a one-minute bar that is four minutes old is not,
/// and a single global staleness threshold gets one of those two wrong.
#[must_use]
pub fn staleness(instrument: &str, age_secs: i64, timeframe_secs: i64) -> Staleness {
    if timeframe_secs <= 0 {
        return Staleness::Fresh;
    }
    if age_secs > timeframe_secs * HARD_STALENESS_MULTIPLE {
        return Staleness::Hard(Fact::HardDataStaleness {
            instrument: instrument.to_owned(),
            age_secs,
            timeframe_secs,
        });
    }
    if age_secs > timeframe_secs * WARN_STALENESS_MULTIPLE {
        return Staleness::Warning {
            instrument: instrument.to_owned(),
            age_secs,
        };
    }
    Staleness::Fresh
}

/// Applies limits, then dwell, then the state machine.
#[derive(Clone, Debug, Default)]
pub struct Evaluator {
    pub limits: LimitBook,
    dwell: DwellTracker,
}

impl Evaluator {
    #[must_use]
    pub fn new(limits: LimitBook, dwell: DwellConfig) -> Self {
        Self {
            limits,
            dwell: DwellTracker::new(dwell),
        }
    }

    #[must_use]
    pub fn dwell_tracker(&self) -> &DwellTracker {
        &self.dwell
    }

    pub fn dwell_tracker_mut(&mut self) -> &mut DwellTracker {
        &mut self.dwell
    }

    /// Judge one reading.
    pub fn evaluate(&mut self, reading: MetricReading) -> Evaluation {
        let Some(limit) = self
            .limits
            .get(&reading.strategy_id, &reading.metric)
            .cloned()
        else {
            return Evaluation {
                reading,
                outcome: Outcome::Undeclared,
                limit_value: None,
                dwell: None,
            };
        };

        let outside = limit.breached_by(reading.value);
        let verdict = self
            .dwell
            .observe(&reading.strategy_id, &reading.metric, outside);
        let outcome = match verdict {
            DwellVerdict::Inside => Outcome::Inside,
            DwellVerdict::Blip => Outcome::Blip,
            DwellVerdict::Sustained => Outcome::Act(severity_of(&limit)),
            DwellVerdict::Recovering => Outcome::Recovering,
            DwellVerdict::Recovered => Outcome::Release,
        };
        let limit_value = limit.crossed_bound(reading.value).or(limit.lower).or(limit.upper);

        Evaluation {
            reading,
            outcome,
            limit_value,
            dwell: Some(verdict),
        }
    }

    /// The state move an evaluation implies, if any.
    #[must_use]
    pub fn transition(
        &self,
        evaluation: &Evaluation,
        from: StrategyState,
        policy: crate::state::HaltPolicy,
    ) -> Transition {
        match &evaluation.outcome {
            Outcome::Act(severity) => crate::state::on_sustained(from, *severity, policy),
            Outcome::Release => crate::state::on_recovered(from),
            _ => Transition::none(from),
        }
    }

    /// A fact: acts now, no dwell (COMP-007 §4.1).
    #[must_use]
    pub fn on_fact(fact: &Fact, from: StrategyState, policy: crate::state::HaltPolicy) -> Transition {
        crate::state::on_sustained(from, Severity::Breach(fact.family()), policy)
    }
}

fn severity_of(limit: &ControlLimit) -> Severity {
    match limit.severity {
        LimitSeverity::Warning => Severity::Warning,
        LimitSeverity::Breach => Severity::Breach(limit.family),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::LimitMethod;
    use chrono::Utc;

    fn limit(metric: &str, family: Family, lower: f64, severity: LimitSeverity) -> ControlLimit {
        ControlLimit {
            strategy_id: "s".to_owned(),
            metric: metric.to_owned(),
            family,
            severity,
            version: 1,
            lower: Some(lower),
            upper: None,
            method: LimitMethod::PaperEwma3Sigma,
            declared_from: "paper-run-7".to_owned(),
            declared_at: Utc::now(),
            declared_by: "human:mason".to_owned(),
        }
    }

    fn evaluator() -> Evaluator {
        let mut book = LimitBook::new();
        book.declare(limit("fill_rate", Family::Execution, 0.8, LimitSeverity::Breach));
        book.declare(limit("sharpe_60d", Family::Pnl, 0.5, LimitSeverity::Warning));
        Evaluator::new(book, DwellConfig::default())
    }

    #[test]
    fn an_undeclared_metric_is_not_reported_as_healthy() {
        // The quiet failure this guards: a dashboard where "inside its limit" and
        // "there is no limit" render the same, so a strategy nobody declared
        // limits for looks like the best-behaved one on the page.
        let mut e = evaluator();
        let ev = e.evaluate(MetricReading::new("s", "cancel_ratio", Family::Behaviour, 0.99));
        assert_eq!(ev.outcome, Outcome::Undeclared);
        assert_ne!(ev.outcome, Outcome::Inside);
    }

    #[test]
    fn a_sustained_execution_breach_scales_down_and_names_the_numbers() {
        // COMP-007 §8.4.
        let mut e = evaluator();
        let mut last = None;
        for _ in 0..3 {
            last = Some(e.evaluate(MetricReading::new("s", "fill_rate", Family::Execution, 0.4)));
        }
        let ev = last.expect("three cadences");
        assert_eq!(ev.outcome, Outcome::Act(Severity::Breach(Family::Execution)));
        assert_eq!(ev.limit_value, Some(0.8), "the event must name the limit");
        assert!((ev.reading.value - 0.4).abs() < f64::EPSILON, "and the value");

        let t = e.transition(&ev, StrategyState::Nominal, crate::state::HaltPolicy::Hold);
        assert_eq!(t.to, StrategyState::ScaledDown);
    }

    #[test]
    fn a_warning_limit_warns_and_does_not_scale() {
        let mut e = evaluator();
        let mut last = None;
        for _ in 0..3 {
            last = Some(e.evaluate(MetricReading::new("s", "sharpe_60d", Family::Pnl, 0.1)));
        }
        let ev = last.expect("three cadences");
        assert_eq!(ev.outcome, Outcome::Act(Severity::Warning));
        let t = e.transition(&ev, StrategyState::Nominal, crate::state::HaltPolicy::Hold);
        assert_eq!(t.to, StrategyState::Warning);
        assert!(!t.actions.contains(&crate::state::Action::ScaleTo(25)));
    }

    #[test]
    fn one_bad_cadence_changes_nothing() {
        let mut e = evaluator();
        let ev = e.evaluate(MetricReading::new("s", "fill_rate", Family::Execution, 0.1));
        assert_eq!(ev.outcome, Outcome::Blip);
        let t = e.transition(&ev, StrategyState::Nominal, crate::state::HaltPolicy::Hold);
        assert!(!t.changed());
    }

    #[test]
    fn a_fresh_feed_is_fresh_and_a_four_minute_old_daily_bar_is_too() {
        assert_eq!(staleness("BTC-USD", 30, 60), Staleness::Fresh);
        assert_eq!(staleness("AAPL", 240, 86_400), Staleness::Fresh);
    }

    #[test]
    fn staleness_is_measured_against_the_instruments_own_timeframe() {
        // 240s is fine for a daily bar and a hard halt for a one-minute one. A
        // single global threshold gets exactly one of those two wrong.
        assert!(matches!(staleness("BTC-USD", 240, 60), Staleness::Warning { .. }));
        assert!(matches!(staleness("BTC-USD", 700, 60), Staleness::Hard(_)));
        assert_eq!(staleness("AAPL", 700, 86_400), Staleness::Fresh);
    }

    #[test]
    fn a_hard_stale_feed_halts_without_waiting_three_cadences() {
        // COMP-007 §4.1's exemption, and §3's Data row (ADR-D-11). Three cadences
        // of a ten-minutes-stale one-minute feed is three more minutes of trading
        // on prices that are not current, and dwell exists to filter noise, not
        // facts.
        //
        // This test previously carried this name and asserted `ScaledDown`, which
        // is worse than a plain contradiction: a reader who greps for the
        // behaviour finds the name and stops. The assertion now reads
        // `Family::breach_halts` alongside the literal, so the name, the
        // assertion and the policy move together or not at all.
        let Staleness::Hard(fact) = staleness("BTC-USD", 700, 60) else {
            panic!("700s on a 60s bar is past the hard multiple");
        };
        assert!(fact.family().breach_halts(), "COMP-007 §3: a hard-stale feed halts");
        let t = Evaluator::on_fact(&fact, StrategyState::Nominal, crate::state::HaltPolicy::Hold);
        assert_eq!(
            t.to,
            StrategyState::Halted,
            "scaling to a quarter still trades on a stale price"
        );
        assert!(t
            .actions
            .contains(&crate::state::Action::Halt(crate::state::HaltPolicy::Hold)));
        assert!(t.changed());
    }

    #[test]
    fn the_soft_band_still_only_warns() {
        // The other half of §3's Data row, and the reason the hard band is worth
        // separating: a feed three cadences late is late, not absent, and halting
        // the book on it is the alarm fatigue §4.1 exists to prevent. Asserted so
        // that widening the halt to the whole Data family — which ADR-D-11 did —
        // cannot quietly swallow the warning band with it.
        let Staleness::Warning { instrument, age_secs } = staleness("BTC-USD", 240, 60) else {
            panic!("240s on a 60s bar is past the warn multiple and short of the hard one");
        };
        assert_eq!(instrument, "BTC-USD");
        assert_eq!(age_secs, 240);
        assert!(
            !matches!(staleness("BTC-USD", 240, 60), Staleness::Hard(_)),
            "the soft band raises no fact, so nothing skips dwell and nothing halts"
        );
    }

    #[test]
    fn both_staleness_bands_come_from_the_two_constants_and_nowhere_else() {
        // The multiples are declared once, and this drives every boundary off
        // those declarations rather than off `240` and `700` typed into a test.
        // A test carrying its own copy of a threshold is the same defect shape as
        // a policy carrying its own copy of a family list: change the constant,
        // and the test goes on asserting the band that no longer exists.
        let tf = 60;
        let warn_edge = tf * WARN_STALENESS_MULTIPLE;
        let hard_edge = tf * HARD_STALENESS_MULTIPLE;
        // `past the multiple`, not `at` it: a bar exactly 3× old is on time for a
        // feed that publishes on the boundary.
        assert_eq!(staleness("BTC-USD", warn_edge, tf), Staleness::Fresh);
        assert!(matches!(
            staleness("BTC-USD", warn_edge + 1, tf),
            Staleness::Warning { .. }
        ));
        assert!(matches!(
            staleness("BTC-USD", hard_edge, tf),
            Staleness::Warning { .. }
        ));
        assert!(matches!(
            staleness("BTC-USD", hard_edge + 1, tf),
            Staleness::Hard(_)
        ));
    }

    #[test]
    fn a_nonpositive_timeframe_reports_fresh_and_that_is_a_gap_not_a_result() {
        // Named for what it asserts, not for what one would want it to assert.
        // A `timeframe_secs` of zero is not a timeframe: there is no multiple to
        // measure an age against, and `staleness` answers `Fresh` — an absent
        // measurement reported as a healthy one, which is the shape this crate
        // exists to refuse.
        //
        // It is not wrong *today* only because nothing calls `staleness`:
        // `FactKind::HardDataStaleness` is `Wiring::Unwired` and the producer it
        // names does not exist yet. The day that producer is written, it must
        // either guarantee a positive timeframe per instrument or this arm must
        // become a halt — and this assertion is what makes that a decision
        // somebody has to take rather than a default they inherit.
        assert_eq!(staleness("BTC-USD", 999_999, 0), Staleness::Fresh);
        assert_eq!(staleness("BTC-USD", 999_999, -60), Staleness::Fresh);
        assert!(
            !crate::family::FactKind::HardDataStaleness
                .wiring()
                .is_raised_in_production(),
            "a producer now exists, so the zero-timeframe arm must be decided rather than inherited"
        );
    }

    #[test]
    fn a_stale_feed_reaches_only_the_strategies_that_use_it() {
        // COMP-007 §3 / S-7.5: "halts the strategy that depends on it and only
        // that one".
        let mut deps = DependencyMap::new();
        deps.add("BTC-USD", "s-btc");
        deps.add("ETH-USD", "s-eth");
        deps.add("BTC-USD", "s-both");
        deps.add("ETH-USD", "s-both");
        assert_eq!(deps.strategies_for("BTC-USD"), ["s-btc", "s-both"]);
        assert!(deps.strategies_for("SOL-USD").is_empty());
    }

    #[test]
    fn a_reconciliation_mismatch_is_a_fact_and_does_not_wait() {
        let fact = Fact::ReconciliationMismatch {
            venue: "kraken".to_owned(),
        };
        assert_eq!(fact.family(), Family::Venue);
        let t = Evaluator::on_fact(&fact, StrategyState::Nominal, crate::state::HaltPolicy::Hold);
        assert!(t.changed());
    }
}
