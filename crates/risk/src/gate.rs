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
    instrument::HaltPolicy,
    money::{Price, Size},
    order::{OrderIntent, Side},
    strategy_def::risk_overrides::RiskOverrides, RiskRejection, TrustTier,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::{
    kill_switch::KillSwitch,
    limits::{self, GlobalRiskLimits},
    overrides,
    rate::{RateCounts, RateKey, RateTracker},
    strategy_halt::StrategyHaltRegistry,
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
    /// The venue this order would reach, if known (COMP-007 §4: a reconciliation
    /// mismatch halts one venue's flow and leaves the others trading).
    pub venue_id: Option<String>,
    /// The strategy instance this order came from, if any. `None` for a manual
    /// order, which no strategy-scoped halt applies to.
    pub strategy_id: Option<String>,
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
            venue_id: None,
            strategy_id: None,
        }
    }

    /// Name the venue this order would reach, so a venue-scoped halt applies.
    #[must_use]
    pub fn on_venue(mut self, venue_id: impl Into<String>) -> Self {
        self.venue_id = Some(venue_id.into());
        self
    }

    /// Name the strategy instance this order came from, so a strategy-scoped
    /// halt applies.
    #[must_use]
    pub fn from_strategy(mut self, strategy_id: impl Into<String>) -> Self {
        self.strategy_id = Some(strategy_id.into());
        self
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
    /// Per-strategy divergence halts (SC-6, FM-4). Always present — there is no
    /// construction of a gate without one, so there is no configuration in
    /// which divergence is not checked.
    halts: Arc<StrategyHaltRegistry>,
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
            halts: Arc::new(StrategyHaltRegistry::new()),
            seen_keys: Mutex::new(BoundedDecisionCache::new(SEEN_KEYS_CAPACITY)),
            rate: Mutex::new(RateTracker::new()),
        }
    }

    /// The configured global limits (read-only).
    #[must_use]
    pub fn limits(&self) -> &GlobalRiskLimits {
        &self.limits
    }

    /// The gate's divergence-halt registry.
    ///
    /// `reconciliation` halts through this handle, so the halt is enforced at
    /// the same chokepoint every order already passes rather than at a second
    /// one built beside it (SC-2). Deliberately an accessor and not a
    /// constructor parameter: a gate cannot be built *without* a registry, so
    /// there is no wiring mistake that silently disables divergence halting.
    #[must_use]
    pub fn halt_registry(&self) -> &Arc<StrategyHaltRegistry> {
        &self.halts
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
        // 1. The monitor lease — fail closed before anything else (COMP-007 §4).
        //
        //    Checked ahead of the kill switch on purpose. The switch says what the
        //    monitor decided; the lease says whether there *is* a monitor. A
        //    platform that cannot tell whether it should stop has already failed
        //    the only question this gate exists to answer.
        if !self.kill_switch.monitor_available() {
            return Err(RiskRejection::MonitorUnavailable);
        }

        // 2. Kill switch — scoped, synchronous fast path.
        if self
            .kill_switch
            .refuses(ctx.venue_id.as_deref(), ctx.strategy_id.as_deref())
        {
            return Err(RiskRejection::KillSwitchActive);
        }

        // 3. Divergence halt for this strategy (SC-6, FM-4) — checked here,
        //    ahead of the idempotency cache, for the same reason the kill
        //    switch is: a redelivered intent that was approved before the
        //    divergence was found must not ride the cached-approval fast path
        //    out to the venue. This is what "halts *before* the next order"
        //    means in code.
        if let Err(halt) = self.halts.admit(intent.strategy_id.as_deref()) {
            return Err(RiskRejection::StrategyHalted {
                strategy_id: halt.strategy_id,
                reason: halt.reason,
            });
        }

        // 3b. A strategy scaled below full size (the ramp's step-down, ADR-P2-27;
        //     the monitor's `scale` control) trades at that fraction. Before the
        //     idempotency cache, like the checks above it: a redelivery after the
        //     scale changed is a new order at the new size. Between full and zero
        //     only: zero is a halt, and a halt is the kill switch's job.
        let intent = self.scaled(intent, ctx)?;

        // 4. Idempotency cache — redelivered intents return the prior decision.
        //    For cached approvals the monitor lease and the kill switch are both
        //    re-checked: a JetStream redelivery arriving after the switch was
        //    tripped, or after the monitor died, must be blocked rather than
        //    passed through on the cached-approval fast path. A redelivery is a
        //    *new* order at the venue.
        //
        //    A redelivery is deliberately *not* recorded against the rate
        //    limit: it is the same order, not a new one.
        {
            let cache = lock_or_refuse(&self.seen_keys, "seen_keys")?;
            if let Some(prior) = cache.get(&intent.idempotency_key) {
                return match prior {
                    CachedDecision::Approved => {
                        if !self.kill_switch.monitor_available() {
                            return Err(RiskRejection::MonitorUnavailable);
                        }
                        if self
                            .kill_switch
                            .refuses(ctx.venue_id.as_deref(), ctx.strategy_id.as_deref())
                        {
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

        // 5. Run checks.  Apply tighten-only overrides first.
        let decision = self.run_checks(&intent, ctx);

        // 6. An approved order is a submission: count it against the rate scope
        //    so the next one sees it.  Rejected orders are not counted — a
        //    rejection did not reach a venue.
        if decision.is_ok() {
            if let RateSource::Tracked(key) = &ctx.rate {
                lock_or_refuse(&self.rate, "rate")?.record(key);
            }
        }

        // 7. Cache the decision.
        {
            let cached = match &decision {
                Ok(_) => CachedDecision::Approved,
                Err(r) => CachedDecision::Rejected(r.clone()),
            };
            lock_or_refuse(&self.seen_keys, "seen_keys")?.insert(intent.idempotency_key, cached);
        }

        decision
    }

    /// The order at the size its strategy is currently allowed (ADR-T3-03).
    ///
    /// `KillSwitch::scale_strategy` stored a percentage and nothing read it, so a
    /// deployment stepped down to 50% kept trading at 100% (the ramp pass applied
    /// the size to a map the gate never consulted). Now an order that would
    /// **add exposure** is cut to `scale_pct`% of its size, rounded down to a
    /// whole lot. An order that rounds to nothing is refused rather than sent
    /// as a zero-size order or rounded up past the allowance. An order that
    /// **reduces** the position (`reduce_only`, or the opposite side and no larger
    /// than the position) is never cut: a strategy being sized down must still be
    /// able to get out.
    fn scaled(&self, mut intent: OrderIntent, ctx: &GateContext) -> Result<OrderIntent, RiskRejection> {
        let key = intent.strategy_id.as_deref().or(ctx.strategy_id.as_deref());
        let Some(key) = key else { return Ok(intent) };
        let pct = self.kill_switch.scale_pct(key);
        if pct >= 100 || intent.reduce_only {
            return Ok(intent);
        }
        let held = ctx.current_position;
        let reduces = match intent.side {
            Side::Sell => held > Decimal::ZERO && intent.size.inner() <= held,
            Side::Buy => held < Decimal::ZERO && intent.size.inner() <= -held,
        };
        if reduces {
            return Ok(intent);
        }
        let mut scaled = intent.size.inner() * Decimal::from(pct) / Decimal::from(100u8);
        if let Measured::Yes(l) = ctx.instrument_limits {
            if !l.lot_size.is_zero() {
                scaled = (scaled / l.lot_size).floor() * l.lot_size;
            }
        }
        if scaled <= Decimal::ZERO {
            return Err(RiskRejection::StrategyHalted {
                strategy_id: key.to_owned(),
                reason: format!(
                    "sized down to {pct}% of full; this order's {} is less than one lot at that size                      (ramp step-down, ADR-P2-27)",
                    intent.size.inner()
                ),
            });
        }
        intent.size = Size(scaled);
        Ok(intent)
    }

    /// Current rate counts for a scope, for diagnostics and tests.
    pub fn rate_counts(&self, key: &RateKey) -> Result<RateCounts, RiskRejection> {
        Ok(lock_or_refuse(&self.rate, "rate")?.counts(key))
    }

    /// Poison the gate's own state, to prove the refusal is real.
    ///
    /// A poisoned `Mutex` is what a panic inside the gate leaves behind.  There
    /// is no way to produce one from outside the module, so the adversarial
    /// test that the gate *fails closed* rather than panicking needs this.
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn poison_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.seen_keys.lock().expect("seen_keys lock");
            panic!("deliberately poisoning the gate's decision cache");
        }));
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
        limits::check_position_after(
            &effective,
            &intent.instrument_id,
            ctx.current_position,
            intent.side,
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

    fn strategy_intent(instrument_id: &str, strategy_id: &str) -> OrderIntent {
        OrderIntent::new(
            instrument_id,
            Side::Buy,
            OrderType::Market,
            domain::money::Size::from_str("1").unwrap(),
            None,
            Some(strategy_id.to_owned()),
        )
    }

    #[test]
    fn divergence_halt_blocks_that_strategy_only() {
        let gate = gate();
        gate.halt_registry().halt(
            "sma-cross-1",
            crate::DivergenceKind::Return,
            "realised -3.1% vs expected +0.4%",
        );

        let err = gate.check(strategy_intent("AAPL", "sma-cross-1"), &simple_ctx());
        assert!(
            matches!(err, Err(RiskRejection::StrategyHalted { .. })),
            "a halted strategy must not pass the gate"
        );

        // A different strategy, and a manual order, are untouched.
        assert!(gate
            .check(strategy_intent("AAPL", "other"), &simple_ctx())
            .is_ok());
        assert!(gate.check(simple_intent("BTC-USD"), &simple_ctx()).is_ok());
    }

    #[test]
    fn divergence_halt_blocks_a_cached_approval() {
        // The adversarial ordering: the intent was approved and cached, *then*
        // reconciliation found the divergence, *then* JetStream redelivered.
        // A cache consulted before the halt would send this order.
        let gate = gate();
        let intent = strategy_intent("AAPL", "sma-cross-1");
        assert!(gate.check(intent.clone(), &simple_ctx()).is_ok());

        gate.halt_registry()
            .halt("sma-cross-1", crate::DivergenceKind::Position, "1 vs 2");

        let err = gate.check(intent, &simple_ctx());
        assert!(
            matches!(err, Err(RiskRejection::StrategyHalted { .. })),
            "a cached approval must be re-checked against the divergence halt"
        );
    }

    #[test]
    fn a_cleared_halt_lets_the_strategy_resume() {
        let gate = gate();
        gate.halt_registry()
            .halt("sma-cross-1", crate::DivergenceKind::Return, "diverged");
        gate.halt_registry()
            .clear(
                "sma-cross-1",
                crate::HumanActor::new("mason").unwrap(),
                "data gap repaired; series agree again",
            )
            .unwrap();
        assert!(gate
            .check(strategy_intent("AAPL", "sma-cross-1"), &simple_ctx())
            .is_ok());
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
        assert!(matches!(err, RiskRejection::DailyLossLimitExceeded { .. }));
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
    fn a_poisoned_gate_refuses_instead_of_panicking() {
        // The gate is the one component that must always be able to say no.
        // Before this, both `check` call sites did `.expect("seen_keys lock")`,
        // so any earlier panic while holding that lock turned every subsequent
        // order approval into a panic — taking down the chokepoint itself.
        let gate = gate();
        gate.poison_for_test();

        let err = gate
            .check(simple_intent("BTC-USD"), &simple_ctx())
            .unwrap_err();
        assert!(
            matches!(err, RiskRejection::GateUnavailable { .. }),
            "a poisoned gate must refuse, not panic and not approve: {err}"
        );

        // And it stays refused — a poisoned lock does not heal.
        let err2 = gate
            .check(simple_intent("BTC-USD"), &simple_ctx())
            .unwrap_err();
        assert!(matches!(err2, RiskRejection::GateUnavailable { .. }));
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

    // ── a size between full and zero takes effect (ADR-T3-03) ───────────────

    fn sized(strategy: &str, side: Side, size: &str) -> OrderIntent {
        OrderIntent::new(
            "BTC-USD",
            side,
            OrderType::Market,
            Size::from_str(size).unwrap(),
            None,
            Some(strategy.to_owned()),
        )
    }

    fn gate_with(ks: &Arc<KillSwitch>) -> RiskGate {
        RiskGate::new(GlobalRiskLimits::default(), Arc::clone(ks))
    }

    #[test]
    fn a_strategy_scaled_to_half_trades_half_size() {
        let ks = Arc::new(KillSwitch::new(false));
        let gate = gate_with(&ks);
        ks.scale_strategy("dep-1", 50);
        let ok = gate.check(sized("dep-1", Side::Buy, "1"), &simple_ctx()).unwrap();
        assert_eq!(ok.intent.size.inner(), Decimal::from_str("0.5").unwrap());
        // Another strategy, and a manual order, keep their size.
        let other = gate.check(sized("dep-2", Side::Buy, "1"), &simple_ctx()).unwrap();
        assert_eq!(other.intent.size.inner(), Decimal::ONE);
        let manual = gate.check(simple_intent("BTC-USD"), &simple_ctx()).unwrap();
        assert_eq!(manual.intent.size.inner(), Decimal::ONE);
    }

    #[test]
    fn a_scaled_size_rounds_down_to_a_whole_lot_and_never_up() {
        let ks = Arc::new(KillSwitch::new(false));
        let gate = gate_with(&ks);
        ks.scale_strategy("dep-1", 25);
        // 0.0107 * 25% = 0.002675 -> lot 0.001 -> 0.002
        let ok = gate.check(sized("dep-1", Side::Buy, "0.0107"), &simple_ctx()).unwrap();
        assert_eq!(ok.intent.size.inner(), Decimal::from_str("0.002").unwrap());
    }

    #[test]
    fn an_order_that_scales_to_less_than_a_lot_is_refused_not_rounded_up() {
        let ks = Arc::new(KillSwitch::new(false));
        let gate = gate_with(&ks);
        ks.scale_strategy("dep-1", 10);
        let err = gate.check(sized("dep-1", Side::Buy, "0.005"), &simple_ctx());
        assert!(matches!(err, Err(RiskRejection::StrategyHalted { .. })), "{err:?}");
    }

    #[test]
    fn an_order_that_reduces_the_position_is_never_cut() {
        let ks = Arc::new(KillSwitch::new(false));
        let gate = gate_with(&ks);
        ks.scale_strategy("dep-1", 10);
        let mut ctx = simple_ctx();
        ctx.current_position = Decimal::from_str("2").unwrap();
        let out = gate.check(sized("dep-1", Side::Sell, "1.5"), &ctx).unwrap();
        assert_eq!(out.intent.size.inner(), Decimal::from_str("1.5").unwrap());
        // reduce_only too.
        let mut ro = sized("dep-1", Side::Buy, "1");
        ro.reduce_only = true;
        let out = gate.check(ro, &simple_ctx()).unwrap();
        assert_eq!(out.intent.size.inner(), Decimal::ONE);
    }

    #[test]
    fn a_redelivered_intent_is_scaled_at_the_size_current_when_it_arrives() {
        let ks = Arc::new(KillSwitch::new(false));
        let gate = gate_with(&ks);
        let intent = sized("dep-1", Side::Buy, "1");
        let first = gate.check(intent.clone(), &simple_ctx()).unwrap();
        assert_eq!(first.intent.size.inner(), Decimal::ONE);
        ks.scale_strategy("dep-1", 50);
        let again = gate.check(intent, &simple_ctx()).unwrap();
        assert_eq!(
            again.intent.size.inner(),
            Decimal::from_str("0.5").unwrap(),
            "the cached approval must not carry the old full size past a step-down"
        );
    }
}
