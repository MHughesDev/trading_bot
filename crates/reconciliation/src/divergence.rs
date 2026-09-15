//! Divergence detection between internal position view and broker position.
//!
//! On divergence: trips the kill switch for the affected instrument and
//! logs an alarm.  Does **not** force-close the position.

use std::sync::Arc;

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

/// Compare `internal_qty` against `broker_qty`.  If they diverge beyond
/// tolerance, trip the kill switch and return `Diverged`.
pub fn check_position_divergence(
    instrument_id: &str,
    internal_qty: Decimal,
    broker_qty: Decimal,
    kill_switch: &Arc<KillSwitch>,
) -> ReconcileOutcome {
    let tolerance: Decimal = QUANTITY_TOLERANCE.parse().expect("constant is valid");
    let delta = (internal_qty - broker_qty).abs();

    if delta > tolerance {
        error!(
            %instrument_id,
            %internal_qty,
            %broker_qty,
            %delta,
            "POSITION DIVERGENCE — tripping kill switch"
        );
        kill_switch.trip();
        ReconcileOutcome::Diverged {
            instrument_id: instrument_id.to_owned(),
            internal_qty: internal_qty.to_string(),
            broker_qty: broker_qty.to_string(),
        }
    } else {
        warn!(
            %instrument_id,
            %internal_qty,
            %broker_qty,
            "position reconciliation OK"
        );
        ReconcileOutcome::Match
    }
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
