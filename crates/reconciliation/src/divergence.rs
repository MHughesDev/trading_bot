//! Divergence detection between internal position view and broker position.
//!
//! On divergence: halts the venue that disagrees with our book, and logs an
//! alarm.  Does **not** force-close the position.
//!
//! ## Why the venue halt goes through `monitor`
//!
//! A halt raised here used to be `KillSwitch::trip_scope` and nothing else — an
//! in-memory scoped view with no row behind it. [`KillSwitch::replace_halted`][rh]
//! **replaces** that view from `monitor.kill_switch` on every refresh, so a halt
//! raised because our book and the venue disagree about what is owned was erased
//! within one refresh cadence (about five seconds in the platform) and the venue
//! silently resumed trading.
//!
//! [`MonitorStore::carry_out_mismatch`][cm] is the receiving half: it writes the
//! durable row first and mirrors the trip second. It is the only way a halt
//! leaves this module, which `tests/a_venue_mismatch_survives_the_mirror_refresh.rs`
//! asserts against the source rather than trusting this paragraph.
//!
//! [rh]: risk::KillSwitch::replace_halted
//! [cm]: monitor::store::MonitorStore::carry_out_mismatch

use std::sync::Arc;

use monitor::store::{MonitorStore, StoreError};
use rust_decimal::Decimal;
use tracing::{error, warn};

use risk::{DivergenceHalt, DivergenceKind, KillSwitch, StrategyHaltRegistry};

use crate::forward::{AlignedBar, DeploymentSpec, ForwardError};

/// Tolerance within which positions are considered equal (rounding noise).
const QUANTITY_TOLERANCE: &str = "0.000001";

/// Outcome of a position reconciliation check.
#[derive(Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    /// Internal and broker positions match within tolerance.
    Match,
    /// Positions diverge; kill switch tripped for this instrument.
    Diverged {
        instrument_id: String,
        internal_qty: String,
        broker_qty: String,
    },
}

/// Compare `internal_qty` against `broker_qty` and say what disagrees.
///
/// Raises nothing. This is the arithmetic on its own, so the comparison can be
/// tested without a database and so that no caller can reach a halt without
/// naming the venue it is at.
#[must_use]
pub fn compare_position(
    instrument_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
) -> ReconcileOutcome {
    let tolerance: Decimal = QUANTITY_TOLERANCE.parse().expect("constant is valid");
    let delta = (internal_qty - broker_qty).abs();

    if delta > tolerance {
        ReconcileOutcome::Diverged {
            instrument_id: instrument_id.to_owned(),
            internal_qty: internal_qty.to_string(),
            broker_qty: broker_qty.to_string(),
        }
    } else {
        ReconcileOutcome::Match
    }
}

/// Compare `internal_qty` against `broker_qty`.  If they diverge beyond
/// tolerance, trip the kill switch and return `Diverged`.
///
/// Trips the **global** scope, because this signature does not know which venue
/// the divergence is at. Prefer [`check_position_divergence_at_venue`] wherever
/// the venue is known: COMP-007 §4 halts the venue that disagrees and leaves the
/// others trading, and a global halt for one venue's bookkeeping is the kind of
/// over-reaction that gets a safety mechanism switched off.
///
/// # This halt does not survive a mirror refresh
///
/// `KillSwitch::trip()` writes the in-memory global flag and no row, and
/// [`replace_halted`][rh] sets that flag from `monitor.kill_switch` on every
/// refresh — so in a process that runs the lease watcher (every process that
/// builds an `api::AppState`) this halt is erased within one cadence. The
/// venue-scoped form below does not have that defect. There is no global
/// equivalent of [`Fact::ReconciliationMismatch`][f] to record, so closing this
/// one is a change in `crates/monitor`, not here.
///
/// [rh]: risk::KillSwitch::replace_halted
/// [f]: monitor::family::Fact
pub fn check_position_divergence(
    instrument_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    let outcome = compare_position(instrument_id, internal_qty, broker_qty);
    if let ReconcileOutcome::Diverged { .. } = &outcome {
        error!(
            %instrument_id,
            %internal_qty,
            %broker_qty,
            "POSITION DIVERGENCE — halting all order flow"
        );
        kill_switch.trip();
    } else {
        warn!(
            %instrument_id,
            %internal_qty,
            %broker_qty,
            "position reconciliation OK"
        );
    }
    outcome
}

/// The venue-scoped form (COMP-007 §4, §8.3; S-7.6).
///
/// Halts `venue:<id>` and nothing else. The other venues keep trading, because
/// the fact established here is "this venue and our book disagree about what is
/// owned", which says nothing at all about the others.
///
/// The halt is raised through [`MonitorStore::carry_out_mismatch`], which writes
/// the durable `monitor.kill_switch` row **first** and mirrors the trip second.
/// That ordering is the whole point: the venue is halted before this returns and
/// is still halted after the next [`replace_halted`][rh] refresh.
///
/// `venue_id` is the id the order gate sees — `risk::GateContext::venue_id`,
/// which the platform fills from the instrument's metadata row. A halt raised
/// against any other spelling refuses nothing.
///
/// # Errors
/// [`StoreError`] if the durable row could not be written. The mirror is then
/// left alone deliberately: the database that could not be written is the same
/// one the monitor lease lives in, so the lease gate has already refused every
/// order platform-wide, and an in-memory trip with nothing behind it would add
/// one refresh cadence of protection and then silently resume.
///
/// [rh]: risk::KillSwitch::replace_halted
pub async fn check_position_divergence_at_venue(
    instrument_id: &str,
    venue_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
    store: &MonitorStore,
    kill_switch: &Arc<KillSwitch>,
) -> Result<ReconcileOutcome, StoreError> {
    let outcome = compare_position(instrument_id, internal_qty, broker_qty);
    if let ReconcileOutcome::Diverged { .. } = &outcome {
        error!(
            %instrument_id,
            %venue_id,
            %internal_qty,
            %broker_qty,
            "POSITION DIVERGENCE — halting this venue"
        );
        let halted = store.carry_out_mismatch(venue_id, kill_switch).await?;
        warn!(scope = %halted, "venue halted and recorded");
    } else {
        warn!(
            %instrument_id,
            %venue_id,
            %internal_qty,
            %broker_qty,
            "position reconciliation OK"
        );
    }
    Ok(outcome)
}

// ───────────────────────────────────────────────────────────────────────────────
// Per-strategy divergence (Set T T-0.3; `artifact.md` SC-6, FM-4)
// ───────────────────────────────────────────────────────────────────────────────

/// What a divergence check decided.
///
/// `Halted` carries the halt that was raised so the caller can persist it to
/// `mlops.divergence_halt` — the halt itself is already in force by the time
/// this value is returned. That ordering is the point of the whole task: the
/// registry is written *before* the caller can do anything with the answer, so
/// there is no window in which code holding a `Halted` could still submit.
#[derive(Debug, Clone, PartialEq)]
pub enum DivergenceVerdict {
    /// Within every declared threshold. Submission continues.
    WithinThreshold,
    /// Submission for this strategy has already stopped.
    Halted(DivergenceHalt),
}

impl DivergenceVerdict {
    #[must_use]
    pub fn is_halted(&self) -> bool {
        matches!(self, Self::Halted(_))
    }
}

/// Watches one deployment against the thresholds it declared, and stops its
/// order flow the moment either is breached.
///
/// The halt is the deliverable. A report that says "we diverged" after the next
/// order went out is precisely the failure this exists to prevent, so every
/// method here writes to the [`StrategyHaltRegistry`] the risk gate reads —
/// which means the stop is in force before the method returns, not after a
/// caller decides to act on the return value.
#[derive(Debug)]
pub struct DivergenceMonitor {
    spec: DeploymentSpec,
    halts: Arc<StrategyHaltRegistry>,
    /// Compounded realised and expected growth since deployment. A per-bar
    /// threshold alone misses the failure that matters most: a small bias, in
    /// the same direction, every bar.
    realised_growth: f64,
    expected_growth: f64,
    bars: usize,
    signal_matches: usize,
}

impl DivergenceMonitor {
    /// # Errors
    /// An invalid [`DeploymentSpec`] — in particular a threshold that could
    /// never fire.
    pub fn new(
        spec: DeploymentSpec,
        halts: Arc<StrategyHaltRegistry>,
    ) -> Result<Self, ForwardError> {
        spec.validate()?;
        Ok(Self {
            spec,
            halts,
            realised_growth: 1.0,
            expected_growth: 1.0,
            bars: 0,
            signal_matches: 0,
        })
    }

    #[must_use]
    pub fn spec(&self) -> &DeploymentSpec {
        &self.spec
    }

    /// Fold one aligned bar in and judge it.
    ///
    /// Call this on every bar close, before the strategy's next order.
    pub fn on_bar(&mut self, bar: &AlignedBar, signal_matched: bool) -> DivergenceVerdict {
        self.bars += 1;
        if signal_matched {
            self.signal_matches += 1;
        }
        self.realised_growth *= 1.0 + bar.realised;
        self.expected_growth *= 1.0 + bar.expected;

        let t = &self.spec.thresholds;
        let per_bar = bar.divergence().abs();
        if !per_bar.is_finite() || per_bar > t.max_abs_return_divergence {
            return self.halt(
                DivergenceKind::Return,
                format!(
                    "bar {}: realised {:.6} vs expected {:.6} — |divergence| {:.6} over the \
                     declared {:.6}",
                    bar.bar, bar.realised, bar.expected, per_bar, t.max_abs_return_divergence
                ),
            );
        }

        let cumulative = (self.realised_growth - self.expected_growth).abs();
        if !cumulative.is_finite() || cumulative > t.max_cum_return_divergence {
            return self.halt(
                DivergenceKind::Return,
                format!(
                    "cumulative over {} bars: realised {:.6} vs expected {:.6} — drift {:.6} \
                     over the declared {:.6}",
                    self.bars,
                    self.realised_growth - 1.0,
                    self.expected_growth - 1.0,
                    cumulative,
                    t.max_cum_return_divergence
                ),
            );
        }

        #[allow(clippy::cast_precision_loss)]
        let match_rate = self.signal_matches as f64 / self.bars as f64;
        if match_rate < t.min_signal_match {
            return self.halt(
                DivergenceKind::Return,
                format!(
                    "signal reproduction {:.4} over {} bars, under the declared {:.4} — the live \
                     system is not running the strategy that was tested",
                    match_rate, self.bars, t.min_signal_match
                ),
            );
        }

        DivergenceVerdict::WithinThreshold
    }

    /// Compare the internal position against the broker's for this deployment
    /// and halt the strategy on disagreement (FM-4).
    ///
    /// The global kill switch is tripped as well, through the existing
    /// [`check_position_divergence`], because a position disagreement is about
    /// the account and not only about this strategy. The per-strategy halt is
    /// added so that clearing the global switch does not silently resume a
    /// strategy whose book nobody has reconciled.
    pub fn on_position(
        &mut self,
        instrument_id: &str,
        internal: Decimal,
        broker: Decimal,
        kill_switch: &Arc<KillSwitch>,
    ) -> DivergenceVerdict {
        let delta = (internal - broker).abs();
        let outcome = check_position_divergence(instrument_id, internal, broker, kill_switch);
        let breached = delta > self.spec.thresholds.max_position_divergence
            || matches!(outcome, ReconcileOutcome::Diverged { .. });
        if breached {
            return self.halt(
                DivergenceKind::Position,
                format!(
                    "{instrument_id}: internal {internal} vs broker {broker} — delta {delta} \
                     over the declared {}",
                    self.spec.thresholds.max_position_divergence
                ),
            );
        }
        DivergenceVerdict::WithinThreshold
    }

    /// Cash or buying power disagrees with the broker's (SC-6's "balance").
    pub fn on_balance(
        &mut self,
        internal: Decimal,
        broker: Decimal,
        tolerance: Decimal,
    ) -> DivergenceVerdict {
        let delta = (internal - broker).abs();
        if delta > tolerance {
            return self.halt(
                DivergenceKind::Balance,
                format!(
                    "balance: internal {internal} vs broker {broker} — delta {delta} over \
                     tolerance {tolerance}"
                ),
            );
        }
        DivergenceVerdict::WithinThreshold
    }

    fn halt(&self, kind: DivergenceKind, reason: String) -> DivergenceVerdict {
        DivergenceVerdict::Halted(self.halts.halt(&self.spec.strategy_id, kind, reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn ks() -> Arc<KillSwitch> {
        Arc::new(KillSwitch::new(false))
    }

    #[test]
    fn matching_positions_return_ok() {
        let ks = ks();
        let result = check_position_divergence(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("1.0").unwrap(),
            &ks,
        );
        assert_eq!(result, ReconcileOutcome::Match);
        assert!(!ks.is_active());
    }

    #[test]
    fn divergent_positions_trip_kill_switch() {
        let ks = ks();
        let result = check_position_divergence(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("2.0").unwrap(), // divergence of 1.0
            &ks,
        );
        assert!(matches!(result, ReconcileOutcome::Diverged { .. }));
        assert!(ks.is_active(), "kill switch must be tripped on divergence");
    }

    #[test]
    fn the_comparison_raises_nothing_on_its_own() {
        // COMP-007 §8.3's halt is venue-scoped and durable, so it needs a
        // database and lives in `tests/a_venue_mismatch_survives_the_mirror_refresh.rs`.
        // What can be judged without one is the arithmetic, and it must raise
        // nothing at all — a comparison that halts as a side effect is how a
        // caller ends up with a halt it never named a venue for.
        let ks = ks();
        let diverged = compare_position(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("2.0").unwrap(),
        );
        assert!(matches!(diverged, ReconcileOutcome::Diverged { .. }));
        assert!(!ks.is_active());
        assert!(!ks.refuses(Some("kraken"), None));
    }

    #[test]
    fn tiny_rounding_noise_within_tolerance_is_ignored() {
        let ks = ks();
        let result = check_position_divergence(
            "BTC-USD",
            Decimal::from_str("1.0").unwrap(),
            Decimal::from_str("1.0000005").unwrap(), // sub-tolerance
            &ks,
        );
        assert_eq!(result, ReconcileOutcome::Match);
        assert!(!ks.is_active());
    }
}
