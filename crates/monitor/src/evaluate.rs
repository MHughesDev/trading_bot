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

/// Why a feed's staleness could not be measured at all.
///
/// The sibling of [`collect::Unobserved`](crate::collect::Unobserved), and it
/// exists for the same reason: this crate's one rule is that an absent
/// measurement is never a zero, never a healthy one and never a small one, and
/// the only way to keep that rule is to give "there was no measurement" a name
/// of its own rather than a number that looks like one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmeasurableFeed {
    /// No last-bar time is known for this instrument, so there is no age.
    ///
    /// Not "the last bar was long ago" and not "the last bar was just now". A
    /// producer that has never seen this instrument, or that read a row whose
    /// `last_bar_at` was NULL, knows only that it does not know — and the two
    /// numbers it would be tempted to substitute are a false halt and a false
    /// all-clear respectively.
    NoLastBarObserved,
    /// No positive bar period is declared for this instrument, so there is no
    /// multiple to measure an age against.
    ///
    /// A feed's staleness is defined *relative to its own timeframe* (four
    /// minutes is fine for a daily bar and a halt for a one-minute one). With no
    /// timeframe the question has no answer, and the fixed number of seconds
    /// somebody would reach for instead is the single global threshold this
    /// module exists to refuse.
    NoTimeframeDeclared,
}

impl UnmeasurableFeed {
    /// The string the event and the log speak.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoLastBarObserved => "no_last_bar_observed",
            Self::NoTimeframeDeclared => "no_timeframe_declared",
        }
    }
}

impl std::fmt::Display for UnmeasurableFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a feed's age means, or an explicit statement that it means nothing yet.
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
    /// The inputs do not support any verdict, and this is the verdict.
    ///
    /// **Not a fourth band.** `Fresh`, `Warning` and `Hard` are three answers to
    /// "how old is this feed"; this is the refusal to answer it. It raises no
    /// [`Fact`] — the fact's own `reason` would have to name an age and a
    /// timeframe, and naming either here would be a measurement nobody took —
    /// and it is deliberately not something a caller can treat as the good news,
    /// because [`Staleness::is_measured`] is the only predicate offered and
    /// there is no `is_fresh`.
    ///
    /// A caller that reaches this has learned something real: it is watching an
    /// instrument it cannot watch. Dropping it silently is the same defect as
    /// reporting `Fresh`, one level up.
    Unmeasurable {
        instrument: String,
        because: UnmeasurableFeed,
    },
}

impl Staleness {
    /// Whether this verdict rests on a measurement at all.
    ///
    /// The only predicate on this type, and the omissions are the point: an
    /// `is_fresh()` would be written `matches!(self, Fresh)` by its author and
    /// read as "the feed is fine" by everyone after them, at which point
    /// `Unmeasurable` — a feed nobody can see — joins `Warning` and `Hard` in
    /// the bucket marked *not fine* and is never looked at again.
    #[must_use]
    pub fn is_measured(&self) -> bool {
        !matches!(self, Self::Unmeasurable { .. })
    }
}

/// Classify a feed's age against its own timeframe.
///
/// Against its *own* timeframe, not a fixed number of seconds: a daily bar that is
/// four minutes old is fine and a one-minute bar that is four minutes old is not,
/// and a single global staleness threshold gets one of those two wrong.
///
/// # The signature is the safety property
///
/// `age_secs` is an `Option` because the producer that will eventually call this
/// has a third case as common as the other two — an instrument with no row, or a
/// row whose `last_bar_at` is NULL — and the two ways of flattening that case
/// into an `i64` are both wrong in a direction that costs money. A large
/// substitute halts a healthy book; a zero reports a dead feed as current and
/// lets a strategy price a position off it. Neither is representable here.
///
/// A non-positive `timeframe_secs` is the same shape: there is no multiple to
/// measure against, so there is no verdict. This function answered `Fresh` to
/// both of those until 2026-09-21 — an absent measurement reported as a healthy
/// one, on the one surface in the platform whose job is to refuse exactly that.
#[must_use]
pub fn staleness(instrument: &str, age_secs: Option<i64>, timeframe_secs: i64) -> Staleness {
    let unmeasurable = |because| Staleness::Unmeasurable {
        instrument: instrument.to_owned(),
        because,
    };
    if timeframe_secs <= 0 {
        return unmeasurable(UnmeasurableFeed::NoTimeframeDeclared);
    }
    let Some(age_secs) = age_secs else {
        return unmeasurable(UnmeasurableFeed::NoLastBarObserved);
    };
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
        assert_eq!(staleness("BTC-USD", Some(30), 60), Staleness::Fresh);
        assert_eq!(staleness("AAPL", Some(240), 86_400), Staleness::Fresh);
    }

    #[test]
    fn staleness_is_measured_against_the_instruments_own_timeframe() {
        // 240s is fine for a daily bar and a hard halt for a one-minute one. A
        // single global threshold gets exactly one of those two wrong.
        assert!(matches!(staleness("BTC-USD", Some(240), 60), Staleness::Warning { .. }));
        assert!(matches!(staleness("BTC-USD", Some(700), 60), Staleness::Hard(_)));
        assert_eq!(staleness("AAPL", Some(700), 86_400), Staleness::Fresh);
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
        let Staleness::Hard(fact) = staleness("BTC-USD", Some(700), 60) else {
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
        let Staleness::Warning { instrument, age_secs } = staleness("BTC-USD", Some(240), 60) else {
            panic!("240s on a 60s bar is past the warn multiple and short of the hard one");
        };
        assert_eq!(instrument, "BTC-USD");
        assert_eq!(age_secs, 240);
        assert!(
            !matches!(staleness("BTC-USD", Some(240), 60), Staleness::Hard(_)),
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
        assert_eq!(staleness("BTC-USD", Some(warn_edge), tf), Staleness::Fresh);
        assert!(matches!(
            staleness("BTC-USD", Some(warn_edge + 1), tf),
            Staleness::Warning { .. }
        ));
        assert!(matches!(
            staleness("BTC-USD", Some(hard_edge), tf),
            Staleness::Warning { .. }
        ));
        assert!(matches!(
            staleness("BTC-USD", Some(hard_edge + 1), tf),
            Staleness::Hard(_)
        ));
    }

    #[test]
    fn an_undeclared_timeframe_refuses_a_verdict_instead_of_reporting_fresh() {
        // The decision this arm used to defer. Until 2026-09-21 a
        // `timeframe_secs` of zero — which is not a timeframe, because there is
        // no multiple to measure an age against — answered `Fresh`: an absent
        // measurement reported as a healthy one, which is the exact shape this
        // crate exists to refuse. It was pinned rather than changed, with a note
        // saying the producer's author would have to decide it.
        //
        // Deferring it was the wrong call and it is taken here instead. The
        // producer's author is not the person who will re-read this arm; they
        // are the person who will call `staleness` in a loop over rows and ship
        // whatever it returns. A default that is safe only while it is unreached
        // is a loaded gun pointed at the day it becomes reachable.
        assert_eq!(
            staleness("BTC-USD", Some(999_999), 0),
            Staleness::Unmeasurable {
                instrument: "BTC-USD".to_owned(),
                because: UnmeasurableFeed::NoTimeframeDeclared,
            }
        );
        assert_eq!(
            staleness("BTC-USD", Some(999_999), -60),
            Staleness::Unmeasurable {
                instrument: "BTC-USD".to_owned(),
                because: UnmeasurableFeed::NoTimeframeDeclared,
            }
        );
    }

    #[test]
    fn an_unknown_last_bar_time_is_neither_a_stale_feed_nor_a_fresh_one() {
        // The case the old signature could not express, which is why it was the
        // dangerous one. A producer reading a row whose `last_bar_at` is NULL, or
        // finding no row at all for an instrument, had to invent an `i64` — and
        // both inventions are wrong in a direction that costs money. A large
        // substitute halts a book whose feeds are fine; a zero reports a dead
        // feed as current and lets a strategy price a position off it.
        //
        // `None` is now the honest input and it has an honest answer, and the
        // timeframe being perfectly well known does not rescue it.
        assert_eq!(
            staleness("BTC-USD", None, 60),
            Staleness::Unmeasurable {
                instrument: "BTC-USD".to_owned(),
                because: UnmeasurableFeed::NoLastBarObserved,
            }
        );
    }

    #[test]
    fn nothing_the_monitor_did_not_measure_is_ever_reported_fresh() {
        // The property, over the whole input space rather than over the two
        // literals above: `Fresh` is reachable only from a measured age and a
        // declared timeframe. Written as a sweep so that a fourth arm added later
        // — or a `timeframe_secs <= 0` guard someone "simplifies" into a
        // `.max(1)`, which is the tempting one — fails here rather than shipping.
        let timeframes = [-86_400_i64, -60, 0, 1, 60, 86_400];
        let ages = [None, Some(-1_i64), Some(0), Some(1), Some(59), Some(999_999)];
        let mut fresh = 0;
        let mut unmeasurable = 0;
        for tf in timeframes {
            for age in ages {
                let verdict = staleness("BTC-USD", age, tf);
                let measurable = age.is_some() && tf > 0;
                assert_eq!(
                    verdict.is_measured(),
                    measurable,
                    "staleness answered {verdict:?} for age {age:?} on a {tf}s timeframe"
                );
                if verdict.is_measured() {
                    if matches!(verdict, Staleness::Fresh) {
                        fresh += 1;
                    }
                } else {
                    unmeasurable += 1;
                }
            }
        }
        // Anti-vacuity, both ways: a `staleness` that refused everything would
        // satisfy every assertion in the loop, and so would one that measured
        // everything.
        assert!(fresh > 0, "the sweep never reached Fresh; it is broken, not satisfied");
        assert!(
            unmeasurable > 0,
            "the sweep never reached Unmeasurable; it is broken, not satisfied"
        );
    }

    #[test]
    fn an_unmeasurable_feed_raises_no_fact_and_cannot_be_read_as_the_good_news() {
        // Two halves of one rule. It raises no `Fact`, because the fact's own
        // `reason` names an age and a timeframe and neither was measured — a halt
        // whose stated cause is a number nobody took is worse than no halt, since
        // the operator acts on the number. And the type offers no `is_fresh`, so
        // the only way to sort it into the healthy bucket is to write the match
        // out and be seen doing it.
        let verdict = staleness("BTC-USD", None, 0);
        assert!(!matches!(verdict, Staleness::Hard(_)));
        assert!(!matches!(verdict, Staleness::Fresh));
        assert!(!verdict.is_measured());
        // And the refusal says which of the two inputs was missing: "we could not
        // measure it" is not actionable, "no bar period is declared for BTC-USD"
        // is. The timeframe is reported first because it is the one a human fixes
        // by declaring a row, rather than by finding out why a collector stopped.
        let Staleness::Unmeasurable { instrument, because } = verdict else {
            panic!("no age and no timeframe is not measurable");
        };
        assert_eq!(instrument, "BTC-USD");
        assert_eq!(because, UnmeasurableFeed::NoTimeframeDeclared);
        assert_eq!(because.to_string(), "no_timeframe_declared");
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
