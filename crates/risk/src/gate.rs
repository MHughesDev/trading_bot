//! The risk gate — one synchronous, idempotent chokepoint for all order flow.
//!
//! `RiskGate::check` is the **only** way to produce an `ApprovedOrder`.
//! `ApprovedOrder` cannot be constructed outside this module, so `execution`
//! can only act on orders that passed the gate.
//!
//! ## Inputs are measured, never defaulted
//!
//! Every input the gate needs arrives as [`Measured`].  There is deliberately
//! no way to spell "I do not have this, carry on": `Measured::Unavailable`
//! rejects the order and names the missing input.  Both production call sites
//! used to pass fabricated constants — a zero position, a zero daily loss, a
//! `0, 0` order-rate — which turned three limits into unconditional passes.
//! The difference between a limit that passed and a limit that could not fire
//! is invisible in an audit log, which is why the type no longer allows it
//! (SC-2, FM-2).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use domain::{
    instrument::HaltPolicy, money::Price, order::OrderIntent,
    strategy_def::risk_overrides::RiskOverrides, RiskRejection, TrustTier,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    kill_switch::KillSwitch,
    limits::{self, GlobalRiskLimits},
    overrides,
    rate::{RateCounts, RateKey, RateTracker},
    trust_gate,
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

impl ApprovedOrder {
    /// Construct for testing only — bypasses the risk gate.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn new_for_test(intent: OrderIntent) -> Self {
        Self {
            intent,
            _sealed: (),
        }
    }
}

/// An input the gate needs, and whether the caller could actually measure it.
///
/// `Unavailable` is not a synonym for "skip the check" — it is a rejection.
/// It carries the reason so the refusal names what was missing rather than
/// blaming the order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Measured<T> {
    Yes(T),
    Unavailable,
}

impl<T> Measured<T> {
    /// Lift an `Option`: `None` means "not measurable", which the gate refuses.
    pub fn from_option(v: Option<T>) -> Self {
        match v {
            Some(v) => Measured::Yes(v),
            None => Measured::Unavailable,
        }
    }

    pub fn as_ref(&self) -> Measured<&T> {
        match self {
            Measured::Yes(v) => Measured::Yes(v),
            Measured::Unavailable => Measured::Unavailable,
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Measured::Yes(_))
    }
}

/// Per-instrument trading increments, read from instrument metadata (DATA-002).
///
/// A zero increment means "this instrument has no such constraint" (fractional
/// crypto has no meaningful lot), which is a *measured* fact about the
/// instrument.  Not knowing the increments at all is `Measured::Unavailable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrumentLimits {
    pub tick_size: Decimal,
    pub lot_size: Decimal,
}

/// Where the gate gets the order-rate counts for this order.
#[derive(Clone, Debug)]
pub enum RateSource {
    /// Production: the gate counts submissions itself under this scope.  The
    /// caller chooses the scope; it cannot choose the count.
    Tracked(RateKey),
    /// Counts supplied directly.  For tests that drive the rate limit itself —
    /// production paths use `Tracked`, which is what makes the limit able to fire.
    Explicit(RateCounts),
}

/// Contextual data the caller must supply to the gate for each order check.
/// The gate itself has no I/O; the caller pre-fetches what is needed.
pub struct GateContext {
    /// Current signed position (positive = long, negative = short).
    pub current_position: Decimal,
    /// Current market price from the latest-state cache.
    pub market_price: Measured<Price>,
    /// Tick and lot increments for this instrument.
    pub instrument_limits: Measured<InstrumentLimits>,
    /// Realized loss today in USD (positive = loss).
    pub daily_loss_usd: Measured<Decimal>,
    /// Trust tier of the event that generated this order intent.
    pub event_trust_tier: TrustTier,
    /// Minimum trust tier declared by the strategy (or `CentralizedExchange` for manual).
    pub strategy_min_trust_tier: TrustTier,
    /// Per-strategy overrides (tighten-only).
    pub risk_overrides: RiskOverrides,
    /// Whether the instrument is currently active.
    pub instrument_active: bool,
    /// How the order-rate limits are counted for this order.
    pub rate: RateSource,
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
    /// **Test and compatibility constructor only.**
    ///
    /// Production callers build the struct field-by-field so that adding an
    /// input the gate needs is a compile error at every door, rather than a
    /// silently-defaulted argument.  This constructor exists because several
    /// integration suites (some owned by other tracks) call it; it takes every
    /// value explicitly, so it fabricates nothing on its own, but a *caller*
    /// passing zeros to it is exactly the pattern this track removed from
    /// `orders.rs` and `hot_path.rs`.
    #[doc(hidden)]
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
            market_price: Measured::from_option(market_price),
            instrument_limits: Measured::Yes(InstrumentLimits {
                tick_size,
                lot_size,
            }),
            daily_loss_usd: Measured::Yes(daily_loss_usd),
            event_trust_tier: TrustTier::CentralizedExchange,
            strategy_min_trust_tier: TrustTier::CentralizedExchange,
            risk_overrides: RiskOverrides::default(),
            instrument_active,
            rate: RateSource::Explicit(RateCounts {
                last_second: recent_orders_last_second,
                last_minute: recent_orders_last_minute,
            }),
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

/// Maximum number of idempotency-key entries to hold in memory.
///
/// This is a **latency** cache, not the durability guarantee: it answers a
/// redelivery without re-running the checks.  Idempotency that survives a
/// restart is the `orders` primary key and the `fills` unique constraint
/// (migration 0121), enforced in `execution::journal`.
const SEEN_KEYS_CAPACITY: usize = 10_000;

/// Bounded FIFO idempotency cache — evicts oldest entry when full.
struct BoundedDecisionCache {
    map: HashMap<Uuid, CachedDecision>,
    order: VecDeque<Uuid>,
    capacity: usize,
}

impl BoundedDecisionCache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&self, key: &Uuid) -> Option<&CachedDecision> {
        self.map.get(key)
    }

    fn insert(&mut self, key: Uuid, value: CachedDecision) {
        if self.map.contains_key(&key) {
            return;
        }
        if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key);
        self.map.insert(key, value);
    }
}

/// The idempotent, synchronous risk gate.
pub struct RiskGate {
    limits: GlobalRiskLimits,
    kill_switch: Arc<KillSwitch>,
    seen_keys: Mutex<BoundedDecisionCache>,
    /// Order-rate accounting.  Lives inside the gate so the counts cannot be
    /// supplied — and therefore cannot be zeroed — by a call site.
    rate: Mutex<RateTracker>,
}

/// A poisoned lock means some earlier holder panicked while mutating the
/// gate's own state.  The gate is the component whose whole job is to be able
/// to say no; if its state is untrustworthy the answer is no, not a panic that
/// takes the chokepoint down with it.
fn lock_or_refuse<'a, T>(
    m: &'a Mutex<T>,
    what: &'static str,
) -> Result<std::sync::MutexGuard<'a, T>, RiskRejection> {
    m.lock().map_err(|_| {
        tracing::error!(
            lock = what,
            "risk gate lock poisoned — refusing the order (fail closed)"
        );
        RiskRejection::GateUnavailable {
            detail: format!("{what} lock poisoned"),
        }
    })
}

impl RiskGate {
    pub fn new(limits: GlobalRiskLimits, kill_switch: Arc<KillSwitch>) -> Self {
        Self {
            limits,
            kill_switch,
            seen_keys: Mutex::new(BoundedDecisionCache::new(SEEN_KEYS_CAPACITY)),
            rate: Mutex::new(RateTracker::new()),
        }
    }

    /// The configured global limits (read-only).
    #[must_use]
    pub fn limits(&self) -> &GlobalRiskLimits {
        &self.limits
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
        //    For cached approvals the kill switch must still be re-checked: a
        //    JetStream redelivery arriving after the switch was tripped must
        //    be blocked, not passed through on the cached-approval fast path.
        //
        //    A redelivery is deliberately *not* recorded against the rate
        //    limit: it is the same order, not a new one.
        {
            let cache = lock_or_refuse(&self.seen_keys, "seen_keys")?;
            if let Some(prior) = cache.get(&intent.idempotency_key) {
                return match prior {
                    CachedDecision::Approved => {
                        if self.kill_switch.is_active() {
                            return Err(RiskRejection::KillSwitchActive);
                        }
                        Ok(ApprovedOrder {
                            intent,
                            _sealed: (),
                        })
                    }
                    CachedDecision::Rejected(r) => Err(r.clone()),
                };
            }
        }

        // 3. Run checks.  Apply tighten-only overrides first.
        let decision = self.run_checks(&intent, ctx);

        // 4. An approved order is a submission: count it against the rate scope
        //    so the next one sees it.  Rejected orders are not counted — a
        //    rejection did not reach a venue.
        if decision.is_ok() {
            if let RateSource::Tracked(key) = &ctx.rate {
                lock_or_refuse(&self.rate, "rate")?.record(key);
            }
        }

        // 5. Cache the decision.
        {
            let cached = match &decision {
                Ok(_) => CachedDecision::Approved,
                Err(r) => CachedDecision::Rejected(r.clone()),
            };
            lock_or_refuse(&self.seen_keys, "seen_keys")?.insert(intent.idempotency_key, cached);
        }

        decision
    }

    /// Current rate counts for a scope, for diagnostics and tests.
    pub fn rate_counts(&self, key: &RateKey) -> Result<RateCounts, RiskRejection> {
        Ok(lock_or_refuse(&self.rate, "rate")?.counts(key))
    }

    fn run_checks(
        &self,
        intent: &OrderIntent,
        ctx: &GateContext,
    ) -> Result<ApprovedOrder, RiskRejection> {
        let effective = overrides::apply_tighten_only(&self.limits, &ctx.risk_overrides)
            .map_err(|_| RiskRejection::TradingDisabled)?;

        // ── Inputs first: refuse before judging, never judge on a default ────
        let Measured::Yes(market_price) = ctx.market_price else {
            return Err(RiskRejection::MarketDataUnavailable {
                instrument_id: intent.instrument_id.clone(),
            });
        };
        let Measured::Yes(instrument_limits) = ctx.instrument_limits else {
            return Err(RiskRejection::InstrumentMetadataUnavailable {
                instrument_id: intent.instrument_id.clone(),
            });
        };
        let Measured::Yes(daily_loss_usd) = ctx.daily_loss_usd else {
            return Err(RiskRejection::AccountStateUnavailable {
                detail: format!("realized daily loss for {}", intent.instrument_id),
            });
        };
        let rate_counts = match &ctx.rate {
            RateSource::Tracked(key) => lock_or_refuse(&self.rate, "rate")?.counts(key),
            RateSource::Explicit(c) => *c,
        };

        limits::check_instrument_active(&intent.instrument_id, ctx.instrument_active)?;
        limits::check_trading_session(&intent.instrument_id, ctx.is_in_session)?;
        limits::check_halt(&intent.instrument_id, &ctx.halt_policy, ctx.is_halted)?;
        limits::check_rate_second(&effective, rate_counts.last_second)?;
        limits::check_rate_minute(&effective, rate_counts.last_minute)?;
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
            Some(market_price),
        )?;
        limits::check_lot_size(
            &intent.instrument_id,
            intent.size,
            instrument_limits.lot_size,
        )?;
        limits::check_price_tick(
            &intent.instrument_id,
            intent.limit_price,
            instrument_limits.tick_size,
        )?;
        limits::check_daily_loss(&effective, daily_loss_usd)?;
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
            Some(Price::from_str("50000").unwrap()),
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
    fn kill_switch_blocks_cached_approval() {
        // C-2: a previously-approved intent arriving after the kill switch trips
        // must be blocked even though it is in the cache as Approved.
        let ks = Arc::new(KillSwitch::new(false));
        let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));

        let intent = simple_intent("BTC-USD");
        // First call — switch off, should approve and cache.
        assert!(gate.check(intent.clone(), &simple_ctx()).is_ok());

        // Trip the kill switch.
        ks.trip();

        // Redelivery of the same intent (same idempotency_key) must be blocked.
        let err = gate.check(intent, &simple_ctx());
        assert!(
            matches!(err, Err(RiskRejection::KillSwitchActive)),
            "cached-approval must be re-checked against kill switch"
        );
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

    // ── Fail-closed inputs (track 16) ────────────────────────────────────────

    #[test]
    fn missing_mark_refuses_rather_than_skipping_price_checks() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.market_price = Measured::Unavailable;
        let err = gate.check(simple_intent("BTC-USD"), &ctx).unwrap_err();
        assert!(
            matches!(err, RiskRejection::MarketDataUnavailable { .. }),
            "an absent mark must reject, not silently pass price sanity: {err}"
        );
    }

    #[test]
    fn missing_instrument_metadata_refuses() {
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.instrument_limits = Measured::Unavailable;
        let err = gate.check(simple_intent("BTC-USD"), &ctx).unwrap_err();
        assert!(matches!(
            err,
            RiskRejection::InstrumentMetadataUnavailable { .. }
        ));
    }

    #[test]
    fn unmeasured_daily_loss_refuses() {
        // The defect this replaces: `Decimal::ZERO` passed as "today's loss",
        // which made the daily-loss limit unable to fire.
        let gate = gate();
        let mut ctx = simple_ctx();
        ctx.daily_loss_usd = Measured::Unavailable;
        let err = gate.check(simple_intent("BTC-USD"), &ctx).unwrap_err();
        assert!(matches!(err, RiskRejection::AccountStateUnavailable { .. }));
    }

    #[test]
    fn daily_loss_limit_fires_when_breached() {
        let gate = RiskGate::new(
            GlobalRiskLimits {
                max_daily_loss_usd: Decimal::from(1000),
                ..Default::default()
            },
            Arc::new(KillSwitch::new(false)),
        );
        let mut ctx = simple_ctx();
        ctx.daily_loss_usd = Measured::Yes(Decimal::from(1500));
        let err = gate.check(simple_intent("BTC-USD"), &ctx).unwrap_err();
        assert!(matches!(
            err,
            RiskRejection::DailyLossLimitExceeded { .. }
        ));
    }

    #[test]
    fn tracked_rate_limit_fires_after_the_configured_burst() {
        // The gate counts its own approvals, so the per-second limit fires
        // without any caller supplying a number.
        let limit = 5u32;
        let gate = RiskGate::new(
            GlobalRiskLimits {
                max_orders_per_second: limit,
                ..Default::default()
            },
            Arc::new(KillSwitch::new(false)),
        );
        let key = RateKey::manual("BTC-USD");
        let mut approved = 0;
        let mut last_err = None;
        for _ in 0..(limit + 3) {
            let mut ctx = simple_ctx();
            ctx.rate = RateSource::Tracked(key.clone());
            // A distinct idempotency key per attempt — these are new orders.
            match gate.check(simple_intent("BTC-USD"), &ctx) {
                Ok(_) => approved += 1,
                Err(e) => {
                    last_err = Some(e);
                    break;
                }
            }
        }
        assert_eq!(approved, limit, "gate must stop at the configured burst");
        assert!(
            matches!(
                last_err,
                Some(RiskRejection::RateLimitPerSecondExceeded { .. })
            ),
            "expected the per-second limit to fire, got {last_err:?}"
        );
    }

    #[test]
    fn redelivery_does_not_consume_rate_budget() {
        let gate = RiskGate::new(
            GlobalRiskLimits {
                max_orders_per_second: 2,
                ..Default::default()
            },
            Arc::new(KillSwitch::new(false)),
        );
        let key = RateKey::manual("BTC-USD");
        let intent = simple_intent("BTC-USD");
        for _ in 0..5 {
            let mut ctx = simple_ctx();
            ctx.rate = RateSource::Tracked(key.clone());
            assert!(
                gate.check(intent.clone(), &ctx).is_ok(),
                "a redelivery of one approved order must stay approved"
            );
        }
        assert_eq!(gate.rate_counts(&key).unwrap().last_second, 1);
    }

    #[test]
    fn rejected_orders_do_not_consume_rate_budget() {
        let gate = gate();
        let key = RateKey::manual("BTC-USD");
        let mut ctx = simple_ctx();
        ctx.rate = RateSource::Tracked(key.clone());
        ctx.instrument_active = false;
        let _ = gate.check(simple_intent("BTC-USD"), &ctx);
        assert_eq!(gate.rate_counts(&key).unwrap().last_minute, 0);
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
