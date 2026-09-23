//! Dwell and hysteresis (COMP-007 §4.1).
//!
//! A 3σ EWMA band on a noisy intraday metric will breach regularly by construction.
//! If every excursion moved the state machine, the monitor would scale strategies
//! down all day for nothing, and the operator would learn — correctly, from the
//! evidence — that its alarms do not mean anything. That is the alarm-fatigue
//! failure the HITL research warns about for approvals, and it is worse here,
//! because the thing being clicked through is the last line before money.
//!
//! So: a metric must sit outside its limit for `min_dwell_cadences` **consecutive**
//! evaluations before the state changes, and recovery needs twice that. A single
//! excursion is a `blip` — recorded, because it is evidence about the limit, and
//! acted on by nothing.

use std::collections::HashMap;

/// How many consecutive evaluations a metric must spend outside its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DwellConfig {
    pub min_dwell_cadences: u32,
}

impl Default for DwellConfig {
    fn default() -> Self {
        Self {
            min_dwell_cadences: 3,
        }
    }
}

impl DwellConfig {
    /// Recovery needs twice the dwell, so a metric oscillating on its boundary
    /// cannot flap the book: it can never accumulate enough consecutive inside
    /// readings to release a scale-down while it is still alternating.
    #[must_use]
    pub fn recovery_cadences(self) -> u32 {
        self.min_dwell_cadences.saturating_mul(2)
    }
}

/// What one evaluation of one metric means once dwell is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DwellVerdict {
    /// Inside its limit, and not in the middle of recovering from anything.
    Inside,
    /// Outside, but not for long enough to mean anything yet. Recorded, not acted on.
    Blip,
    /// Outside for `min_dwell_cadences` consecutive evaluations. Act.
    Sustained,
    /// Inside again, but not yet for `2 × min_dwell_cadences`. Hold the action.
    Recovering,
    /// Inside for the full recovery window. Release an automatic scale-down.
    Recovered,
}

impl DwellVerdict {
    /// Whether this verdict moves the state machine.
    #[must_use]
    pub fn acts(self) -> bool {
        matches!(self, Self::Sustained | Self::Recovered)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Counters {
    outside: u32,
    inside: u32,
    acted: bool,
}

/// Per-`(strategy, metric)` dwell counters.
///
/// Keyed on both because dwell is a property of one metric on one strategy: a fill
/// rate that has been low for two cadences on strategy A says nothing about
/// strategy B, and merging them would let two unrelated one-cadence excursions add
/// up to an action neither of them earned.
#[derive(Clone, Debug, Default)]
pub struct DwellTracker {
    config: DwellConfig,
    counters: HashMap<(String, String), Counters>,
}

impl DwellTracker {
    #[must_use]
    pub fn new(config: DwellConfig) -> Self {
        Self {
            config,
            counters: HashMap::new(),
        }
    }

    #[must_use]
    pub fn config(&self) -> DwellConfig {
        self.config
    }

    /// Feed one evaluation. `outside` is whether the reading breached its limit.
    pub fn observe(&mut self, strategy: &str, metric: &str, outside: bool) -> DwellVerdict {
        let key = (strategy.to_owned(), metric.to_owned());
        let c = self.counters.entry(key).or_default();

        if outside {
            c.inside = 0;
            c.outside = c.outside.saturating_add(1);
            // `c.acted` short-circuits the dwell on a relapse. A strategy that
            // has already been acted on and goes outside again during recovery
            // is still sustained — it does not have to re-earn the full dwell
            // before the caller is told so. Reporting `Blip` there reads as "no
            // action needed" and is how a strategy that is still misbehaving
            // gets its size back.
            if c.acted || c.outside >= self.config.min_dwell_cadences {
                c.acted = true;
                return DwellVerdict::Sustained;
            }
            return DwellVerdict::Blip;
        }

        c.outside = 0;
        c.inside = c.inside.saturating_add(1);
        if !c.acted {
            return DwellVerdict::Inside;
        }
        if c.inside >= self.config.recovery_cadences() {
            c.acted = false;
            return DwellVerdict::Recovered;
        }
        DwellVerdict::Recovering
    }

    /// Restore counters read back from `monitor.dwell`, so a monitor restart does
    /// not throw away a strategy's progress through a sustained breach.
    pub fn restore(&mut self, strategy: &str, metric: &str, outside: u32, inside: u32) {
        self.counters.insert(
            (strategy.to_owned(), metric.to_owned()),
            Counters {
                outside,
                inside,
                acted: outside >= self.config.min_dwell_cadences,
            },
        );
    }

    /// The counters, for persistence.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(String, String, u32, u32)> {
        let mut rows: Vec<(String, String, u32, u32)> = self
            .counters
            .iter()
            .map(|((s, m), c)| (s.clone(), m.clone(), c.outside, c.inside))
            .collect();
        rows.sort();
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> DwellTracker {
        DwellTracker::new(DwellConfig::default())
    }

    #[test]
    fn a_single_cadence_excursion_is_a_blip_and_nothing_else() {
        // COMP-007 §8.9, first half.
        let mut t = tracker();
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Blip);
        assert!(!DwellVerdict::Blip.acts());
        assert_eq!(t.observe("s", "fill_rate", false), DwellVerdict::Inside);
    }

    #[test]
    fn three_consecutive_cadences_act() {
        let mut t = tracker();
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Blip);
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Blip);
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Sustained);
        assert!(DwellVerdict::Sustained.acts());
    }

    #[test]
    fn a_reading_back_inside_restarts_the_count_from_zero() {
        // The adversarial case: two excursions either side of a good cadence must
        // not add up to three. "Consecutive" is the whole word in the spec.
        let mut t = tracker();
        t.observe("s", "fill_rate", true);
        t.observe("s", "fill_rate", true);
        t.observe("s", "fill_rate", false);
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Blip);
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Blip);
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Sustained);
    }

    #[test]
    fn a_synthetic_oscillating_series_never_flaps_the_book() {
        // COMP-007 §8.9, second half, and the reason hysteresis exists: a metric
        // sitting exactly on its limit and alternating must produce no action at
        // all — not an action every other cadence.
        let mut t = tracker();
        let mut actions = 0;
        for i in 0..200 {
            if t.observe("s", "cancel_ratio", i % 2 == 0).acts() {
                actions += 1;
            }
        }
        assert_eq!(
            actions, 0,
            "an alternating series produced {actions} state changes"
        );
    }

    #[test]
    fn recovery_needs_twice_the_dwell() {
        let mut t = tracker();
        for _ in 0..3 {
            t.observe("s", "fill_rate", true);
        }
        // Five good cadences are not enough; the sixth is.
        for i in 0..5 {
            assert_eq!(
                t.observe("s", "fill_rate", false),
                DwellVerdict::Recovering,
                "cadence {i} released too early"
            );
        }
        assert_eq!(t.observe("s", "fill_rate", false), DwellVerdict::Recovered);
    }

    #[test]
    fn one_good_cadence_during_recovery_does_not_bank_progress_after_a_relapse() {
        let mut t = tracker();
        for _ in 0..3 {
            t.observe("s", "fill_rate", true);
        }
        for _ in 0..5 {
            t.observe("s", "fill_rate", false);
        }
        // Relapse: outside again. The recovery count must go back to zero, or a
        // strategy that is still misbehaving gets its size back.
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Sustained);
        assert_eq!(t.observe("s", "fill_rate", false), DwellVerdict::Recovering);
    }

    #[test]
    fn dwell_is_per_strategy_and_per_metric() {
        let mut t = tracker();
        t.observe("a", "fill_rate", true);
        t.observe("b", "fill_rate", true);
        t.observe("a", "cancel_ratio", true);
        // None of those three share a counter, so none of them is near acting.
        assert_eq!(t.observe("a", "fill_rate", true), DwellVerdict::Blip);
    }

    #[test]
    fn restored_counters_continue_rather_than_restart() {
        let mut t = tracker();
        t.restore("s", "fill_rate", 2, 0);
        assert_eq!(t.observe("s", "fill_rate", true), DwellVerdict::Sustained);
    }
}
