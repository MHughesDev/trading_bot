//! The risk gate — one synchronous, idempotent chokepoint for all order flow.
//!
//! `RiskGate::check` is the **only** way to produce an `ApprovedOrder`.
//! `ApprovedOrder` cannot be constructed outside this module, so `execution`
//! can only act on orders that passed the gate.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use domain::{
    instrument::HaltPolicy,
    money::Price,
    order::OrderIntent,
    strategy_def::risk_overrides::RiskOverrides,
    RiskRejection, TrustTier,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    kill_switch::KillSwitch,
    limits::{self, GlobalRiskLimits},
    overrides, trust_gate,
};

/// A risk-gate–approved order.
///
/// The private `_sealed` field prevents construction outside this module,
/// ensuring that every `ApprovedOrder` originated from a successful
/// `RiskGate::check` call.
#[allow(clippy::manual_non_exhaustive)]
#[derive(Clone, Debug)]
pub struct ApprovedOrder {
    pub intent: OrderIntent,
    _sealed: (),
}

/// Contextual data the caller must supply to the gate for each order check.
/// The gate itself has no I/O; the caller pre-fetches what is needed.
pub struct GateContext {
    /// Current signed position (positive = long, negative = short).
    pub current_position: Decimal,
    /// Current market price from the latest-state cache (None = unavailable).
    pub market_price: Option<Price>,
    /// Minimum price increment for this instrument.
    pub tick_size: Decimal,
    /// Minimum order size for this instrument.
    pub lot_size: Decimal,
    /// Realized loss today in USD (positive = loss).
    pub daily_loss_usd: Decimal,
    /// Trust tier of the event that generated this order intent.
    pub event_trust_tier: TrustTier,
    /// Minimum trust tier declared by the strategy (or `CentralizedExchange` for manual).
    pub strategy_min_trust_tier: TrustTier,
    /// Per-strategy overrides (tighten-only).
    pub risk_overrides: RiskOverrides,
    /// Whether the instrument is currently active.
    pub instrument_active: bool,
    /// Orders submitted in the last second (for rate limiting).
    pub recent_orders_last_second: u32,
    /// Orders submitted in the last minute (for rate limiting).
    pub recent_orders_last_minute: u32,
    /// Whether the instrument is currently within its trading session.
    /// Always `true` for 24/7 instruments; computed from `TradingSchedule` by the caller.
    pub is_in_session: bool,
    /// Whether the instrument is currently halted at the exchange.
    /// Only meaningful when `halt_policy == Haltable`.
    pub is_halted: bool,
    /// The instrument's halt policy.
    pub halt_policy: HaltPolicy,
}

impl GateContext {
    /// Convenience constructor for manual orders (no strategy overrides, CEX trust).
    ///
    /// Defaults: `is_in_session = true` (24/7 crypto), `is_halted = false`,
    /// `halt_policy = NonHaltable`. Override these fields for equity instruments.
    #[allow(clippy::too_many_arguments)]
    pub fn for_manual_order(
        current_position: Decimal,
        market_price: Option<Price>,
        tick_size: Decimal,
        lot_size: Decimal,
        daily_loss_usd: Decimal,
        instrument_active: bool,
        recent_orders_last_second: u32,
        recent_orders_last_minute: u32,
    ) -> Self {
        Self {
            current_position,
            market_price,
            tick_size,
            lot_size,
            daily_loss_usd,
            event_trust_tier: TrustTier::CentralizedExchange,
            strategy_min_trust_tier: TrustTier::CentralizedExchange,
            risk_overrides: RiskOverrides::default(),
            instrument_active,
            recent_orders_last_second,
            recent_orders_last_minute,
            is_in_session: true,
            is_halted: false,
            halt_policy: HaltPolicy::NonHaltable,
        }
    }
}

#[derive(Clone, Debug)]
enum CachedDecision {
    Approved,
    Rejected(RiskRejection),
}

/// The idempotent, synchronous risk gate.
pub struct RiskGate {
    limits: GlobalRiskLimits,
    kill_switch: Arc<KillSwitch>,
    seen_keys: Mutex<HashMap<Uuid, CachedDecision>>,
}

impl RiskGate {
    pub fn new(limits: GlobalRiskLimits, kill_switch: Arc<KillSwitch>) -> Self {
        Self {
            limits,
            kill_switch,
            seen_keys: Mutex::new(HashMap::new()),
        }
    }

    /// Evaluate an `OrderIntent` against all limits.
    ///
    /// Idempotent: if this `idempotency_key` has been seen before, the prior
    /// decision is returned immediately without re-running checks.
    pub fn check(
        &self,
        intent: OrderIntent,
        ctx: &GateContext,
    ) -> Result<ApprovedOrder, RiskRejection> {
        // 1. Kill switch — synchronous fast path; checked before everything else.
        if self.kill_switch.is_active() {
            return Err(RiskRejection::KillSwitchActive);
        }

        // 2. Idempotency cache — redelivered intents return the prior decision.
        {
            let cache = self.seen_keys.lock().expect("seen_keys lock");
            if let Some(prior) = cache.get(&intent.idempotency_key) {
                return match prior {
                    CachedDecision::Approved => Ok(ApprovedOrder {
                        intent,
                        _sealed: (),
                    }),
                    CachedDecision::Rejected(r) => Err(r.clone()),
                };
            }
        }

        // 3. Run checks.  Apply tighten-only overrides first.
        let decision = self.run_checks(&intent, ctx);

        // 4. Cache the decision.
        {
            let cached = match &decision {
                Ok(_) => CachedDecision::Approved,
                Err(r) => CachedDecision::Rejected(r.clone()),
            };
            self.seen_keys
                .lock()
                .expect("seen_keys lock")
                .insert(intent.idempotency_key, cached);
        }

        decision
    }

    fn run_checks(
        &self,
        intent: &OrderIntent,
        ctx: &GateContext,
    ) -> Result<ApprovedOrder, RiskRejection> {
        let effective = overrides::apply_tighten_only(&self.limits, &ctx.risk_overrides)
            .map_err(|_| RiskRejection::TradingDisabled)?;

        limits::check_instrument_active(&intent.instrument_id, ctx.instrument_active)?;
        limits::check_trading_session(&intent.instrument_id, ctx.is_in_session)?;
        limits::check_halt(&intent.instrument_id, &ctx.halt_policy, ctx.is_halted)?;
        limits::check_rate_second(&effective, ctx.recent_orders_last_second)?;
        limits::check_rate_minute(&effective, ctx.recent_orders_last_minute)?;
        limits::check_position(
            &effective,
            &intent.instrument_id,
            ctx.current_position,
            intent.size,
        )?;
        limits::check_price_sanity(
            &effective,
            &intent.instrument_id,
            intent.limit_price,
            ctx.market_price,
        )?;
        limits::check_lot_size(&intent.instrument_id, intent.size, ctx.lot_size)?;
        limits::check_daily_loss(&effective, ctx.daily_loss_usd)?;
        trust_gate::check_trust(ctx.event_trust_tier, ctx.strategy_min_trust_tier)?;

        Ok(ApprovedOrder {
            intent: intent.clone(),
            _sealed: (),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::order::{OrderType, Side};
    use std::str::FromStr;

    fn gate() -> RiskGate {
        RiskGate::new(
            GlobalRiskLimits::default(),
            Arc::new(KillSwitch::new(false)),
        )
    }

    fn simple_intent(instrument_id: &str) -> OrderIntent {
        OrderIntent::new(
            instrument_id,
            Side::Buy,
            OrderType::Market,
            domain::money::Size::from_str("1").unwrap(),
            None,
            None,
        )
    }

    fn simple_ctx() -> GateContext {
        GateContext::for_manual_order(
            Decimal::ZERO,
            None,
            Decimal::from_str("0.01").unwrap(),
            Decimal::from_str("0.001").unwrap(),
            Decimal::ZERO,
            true,
            0,
            0,
        )
    }

    #[test]
    fn valid_order_approved() {
        let gate = gate();
        let result = gate.check(simple_intent("BTC-USD"), &simple_ctx());
        assert!(result.is_ok());
    }

    #[test]
    fn kill_switch_blocks_order() {
        let ks = Arc::new(KillSwitch::new(false));
        let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
        ks.trip();
        let err = gate.check(simple_intent("BTC-USD"), &simple_ctx());
        assert!(matches!(err, Err(RiskRejection::KillSwitchActive)));
    }

    #[test]
    fn inactive_instrument_blocked() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.instrument_active = false;
        let err = gate.check(simple_intent("BTC-USD"), &ctx);
        assert!(matches!(err, Err(RiskRejection::InstrumentInactive { .. })));
    }

    #[test]
    fn rejection_carries_structured_reason() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.instrument_active = false;
        let err = gate.check(simple_intent("BTC-USD"), &ctx);
        assert!(err.unwrap_err().to_string().contains("BTC-USD"));
    }

    // ── Cross-asset gate tests (P6-T01 adversarial) ──────────────────────────

    #[test]
    fn equity_outside_session_rejected() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.is_in_session = false;
        ctx.halt_policy = HaltPolicy::Haltable;
        let err = gate.check(simple_intent("AAPL"), &ctx);
        assert!(
            matches!(err, Err(RiskRejection::OutsideTradingHours { .. })),
            "expected OutsideTradingHours"
        );
    }

    #[test]
    fn equity_halted_rejected() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.is_in_session = true;
        ctx.halt_policy = HaltPolicy::Haltable;
        ctx.is_halted = true;
        let err = gate.check(simple_intent("AAPL"), &ctx);
        assert!(
            matches!(err, Err(RiskRejection::InstrumentHalted { .. })),
            "expected InstrumentHalted"
        );
    }

    #[test]
    fn equity_in_session_not_halted_approved() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.is_in_session = true;
        ctx.halt_policy = HaltPolicy::Haltable;
        ctx.is_halted = false;
        assert!(gate.check(simple_intent("AAPL"), &ctx).is_ok());
    }

    #[test]
    fn crypto_non_haltable_ignores_halt_flag() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.halt_policy = HaltPolicy::NonHaltable;
        ctx.is_halted = true; // would be rejected for Haltable
        ctx.is_in_session = true;
        assert!(
            gate.check(simple_intent("BTC-USDT"), &ctx).is_ok(),
            "NonHaltable crypto must not be blocked by is_halted"
        );
    }

    #[test]
    fn crypto_24_7_always_in_session() {
        let gate = gate();
        let ctx = simple_ctx(); // defaults: is_in_session=true, NonHaltable
        assert!(gate.check(simple_intent("BTC-USDT"), &ctx).is_ok());
    }
}
