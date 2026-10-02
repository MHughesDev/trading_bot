//! The process-wide source of a **real** [`GateContext`].
//!
//! Two doors submit orders: the REST handler in `api::routes::orders` and the
//! hot path's stage 4 in `apps/platform`.  Before this module each built its
//! own context, and the hot path's was fabricated — a zero position, a mark of
//! `None`, tick and lot of `1`, a zero daily loss, and `0, 0` order counts.
//! Wiring stage 3 on top of that would have turned the single risk chokepoint
//! into a rubber stamp (FM-2), so both doors now ask the same provider.
//!
//! The provider is installed once, by whoever owns the account and instrument
//! state (`api::AppState`).  It is deliberately **not** optional in effect: a
//! process with no provider installed cannot build a context, so stage 4
//! refuses every order rather than inventing one.  That is the intended
//! behaviour in unit tests and tools, which have no account state to read.

use std::sync::{Arc, OnceLock};

use domain::{order::OrderIntent, RiskRejection};

use crate::gate::{GateContext, InstrumentLimits, Measured};

/// Supplies the measured risk context for an order about to be gated.
///
/// Implementations do no blocking I/O: they read in-memory account state and a
/// metadata snapshot that is refreshed out of band.  Anything they cannot
/// measure is returned as `Measured::Unavailable`, which the gate refuses —
/// an implementation must never substitute a permissive value.
pub trait RiskContextProvider: Send + Sync {
    /// Build the context for `intent`, or explain why it cannot be built.
    fn context_for(&self, intent: &OrderIntent) -> Result<GateContext, RiskRejection>;

    /// Tick and lot increments for `instrument_id`, if known.
    ///
    /// The sizer needs the lot to quantize an equity-relative size *before* the
    /// order is built, which is earlier than [`Self::context_for`].  Not
    /// knowing it is `Unavailable`; there is no "assume no lot" answer, because
    /// assuming one is how a size that the venue will reject gets submitted.
    fn instrument_limits(&self, instrument_id: &str) -> Measured<InstrumentLimits>;
}

static PROVIDER: OnceLock<Arc<dyn RiskContextProvider>> = OnceLock::new();

/// Install the process-wide provider.  The first install wins; a second call
/// is a no-op and returns `false`, so a test cannot silently repoint the
/// provider a running pipeline is already using.
pub fn install(provider: Arc<dyn RiskContextProvider>) -> bool {
    PROVIDER.set(provider).is_ok()
}

/// The installed provider, if any.
#[must_use]
pub fn provider() -> Option<Arc<dyn RiskContextProvider>> {
    PROVIDER.get().cloned()
}

/// Tick/lot increments for `instrument_id`, `Unavailable` when no provider is
/// installed.  The sizer treats that as "cannot size", which surfaces as a
/// refused order naming the missing metadata.
#[must_use]
pub fn instrument_limits(instrument_id: &str) -> Measured<InstrumentLimits> {
    match provider() {
        Some(p) => p.instrument_limits(instrument_id),
        None => Measured::Unavailable,
    }
}

/// Build the context for `intent`, refusing when no provider is installed.
///
/// This is the function the hot path calls.  "No provider" is not "no limits":
/// it is a refusal that names itself.
pub fn context_for(intent: &OrderIntent) -> Result<GateContext, RiskRejection> {
    match provider() {
        Some(p) => p.context_for(intent),
        None => Err(RiskRejection::GateUnavailable {
            detail: "no risk-context provider installed in this process".to_owned(),
        }),
    }
}

// ── The simulated context (Set U U-2.4, ADR-0046) ───────────────────────────
//
// A backtest is gated by the same `RiskGate::check` as paper and live. What
// differs is only where the context comes from: the simulated account, the
// simulated clock, and a kill switch that belongs to the run. Nothing here reads
// the process-wide provider above — a backtest running inside the platform must
// never be judged on the live account's positions, nor trip the live switch.

/// What a backtest measured about one order, from its own simulated state.
#[derive(Clone, Debug)]
pub struct SimulatedInputs {
    /// Signed position in the simulated account.
    pub position: rust_decimal::Decimal,
    /// The simulated venue's latest mark, if it has one.
    pub mark: Option<domain::money::Price>,
    /// Tick and lot increments from instrument metadata, if known.
    pub limits: Option<InstrumentLimits>,
    /// The simulated account's loss so far on the simulated day (positive =
    /// loss), realized plus unrealized — the same number paper supplies.
    pub daily_loss_usd: rust_decimal::Decimal,
    pub strategy_id: Option<String>,
}

/// Per-run gate state kept on the simulated clock.
///
/// The live gate counts order rate against `Instant::now()`. A backtest replays
/// a month in a second, so wall-clock rate counts would refuse almost every
/// order for a reason that never existed; this counts in simulated time.
#[derive(Debug, Default)]
pub struct SimulatedRiskState {
    submissions: std::collections::VecDeque<chrono::DateTime<chrono::Utc>>,
    halted_on: Option<chrono::NaiveDate>,
}

impl SimulatedRiskState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Approved submissions in the second and the minute before `now`.
    pub fn rate_at(&mut self, now: chrono::DateTime<chrono::Utc>) -> crate::rate::RateCounts {
        let minute = now - chrono::Duration::seconds(60);
        while self.submissions.front().is_some_and(|t| *t <= minute) {
            self.submissions.pop_front();
        }
        let second = now - chrono::Duration::seconds(1);
        let last_second = self.submissions.iter().filter(|t| **t > second).count();
        crate::rate::RateCounts {
            last_second: u32::try_from(last_second).unwrap_or(u32::MAX),
            last_minute: u32::try_from(self.submissions.len()).unwrap_or(u32::MAX),
        }
    }

    /// Count an approved submission.
    pub fn record(&mut self, now: chrono::DateTime<chrono::Utc>) {
        self.submissions.push_back(now);
    }

    /// Whether a daily-loss breach has halted the run for `now`'s day.
    #[must_use]
    pub fn halted(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.halted_on == Some(now.date_naive())
    }

    /// Halt the run for the rest of `now`'s simulated day. The next day lifts it.
    pub fn halt_for_day(&mut self, now: chrono::DateTime<chrono::Utc>) {
        self.halted_on = Some(now.date_naive());
    }
}

impl GateContext {
    /// The context for a backtest order: every input from the simulated run,
    /// nothing from the process. Missing tick/lot or mark stays `Unavailable`
    /// and is refused exactly as live refuses it.
    #[must_use]
    pub fn simulated(inputs: SimulatedInputs, rate: crate::rate::RateCounts) -> Self {
        Self {
            current_position: inputs.position,
            market_price: Measured::from_option(inputs.mark),
            instrument_limits: Measured::from_option(inputs.limits),
            daily_loss_usd: Measured::Yes(inputs.daily_loss_usd),
            event_trust_tier: domain::TrustTier::CentralizedExchange,
            strategy_min_trust_tier: domain::TrustTier::CentralizedExchange,
            risk_overrides: domain::strategy_def::risk_overrides::RiskOverrides::default(),
            instrument_active: true,
            rate: crate::gate::RateSource::Explicit(rate),
            // The simulated venue enforces session hours itself when its engine
            // is configured to; bars outside a session do not exist to trade on.
            is_in_session: true,
            is_halted: false,
            halt_policy: domain::instrument::HaltPolicy::NonHaltable,
            venue_id: Some("simulated".to_owned()),
            strategy_id: inputs.strategy_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::Size;
    use domain::order::{OrderType, Side};
    use std::str::FromStr;

    #[test]
    fn without_a_provider_the_context_is_refused_not_defaulted() {
        let intent = OrderIntent::new(
            "BTC-USD",
            Side::Buy,
            OrderType::Market,
            Size::from_str("1").unwrap(),
            None,
            None,
        );
        // This test runs in a process where no provider is installed.
        let err = match context_for(&intent) {
            Ok(_) => panic!("a context was produced with no provider installed"),
            Err(e) => e,
        };
        assert!(
            matches!(err, RiskRejection::GateUnavailable { .. }),
            "a missing provider must refuse, not produce a permissive context"
        );
    }

    fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn simulated_rate_counts_run_on_simulated_time() {
        let mut s = SimulatedRiskState::new();
        s.record(at("2026-01-01T00:00:00Z"));
        s.record(at("2026-01-01T00:00:00.500Z"));
        let r = s.rate_at(at("2026-01-01T00:00:00.900Z"));
        assert_eq!((r.last_second, r.last_minute), (2, 2));
        let r = s.rate_at(at("2026-01-01T00:00:30Z"));
        assert_eq!((r.last_second, r.last_minute), (0, 2));
        let r = s.rate_at(at("2026-01-01T00:01:00.500Z"));
        assert_eq!((r.last_second, r.last_minute), (0, 0));
    }

    #[test]
    fn a_daily_halt_lasts_until_the_simulated_day_ends() {
        let mut s = SimulatedRiskState::new();
        s.halt_for_day(at("2026-01-01T14:00:00Z"));
        assert!(s.halted(at("2026-01-01T23:59:59Z")));
        assert!(!s.halted(at("2026-01-02T00:00:00Z")));
    }
}
