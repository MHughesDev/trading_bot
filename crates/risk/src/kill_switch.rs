//! The scoped kill switch and the monitor lease it fails closed on (COMP-007 §5).
//!
//! This used to be an `AtomicBool` with `trip()` and `reset()`, persisted by "the
//! caller". Nothing tripped it automatically, nothing scoped it, nothing recorded
//! why, and — the part that mattered — it lived in the same process as everything
//! it was meant to stop, so the failure that required killing also disabled the
//! kill.
//!
//! What lives here now is only the *mirror*: an in-memory, hot-path-safe view of
//! state that an independent process (`tbot-monitor`, `crates/monitor`) owns and
//! persists. The mirror is refreshed on every lease renewal and on `NOTIFY`, so
//! `is_active` stays a non-blocking read on the order path while the authority for
//! what it says sits in a different failure domain.
//!
//! # The lease, and why it is one-way
//!
//! The runtime does not ask "has the monitor told me to stop". It asks "has the
//! monitor told me it is alive, recently". Silence is a halt. That inversion is the
//! whole fail-closed design: a monitor that crashed, wedged, lost its database or
//! was never started produces the same answer as one that deliberately halted.
//!
//! [`KillSwitch::require_monitor_lease`] arms that check and **cannot be undone**.
//! There is no `disarm`, no `set_lease_not_required`, no flag, and
//! `crates/risk/tests/lease_fails_closed.rs` asserts by scanning this file that no
//! such thing has appeared. A safety property that a later caller can switch off is
//! a safety property only until someone is in a hurry.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use dashmap::DashMap;

/// What a kill-switch row applies to.
///
/// A single global flag could only ever answer "is everything stopped", so one bad
/// venue stopped the whole book and one bad strategy did too — which is why nobody
/// used it and why it was always reset in a hurry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// The platform's order gate. Refuses everything.
    Global,
    /// One venue's order flow. The other venues keep trading.
    Venue(String),
    /// One strategy instance.
    Strategy(String),
    /// A rehearsal (COMP-007 §4.2). Never consulted by the real order gate —
    /// a drill that refused a real order would be an incident, not a drill.
    Drill(String),
}

impl Scope {
    #[must_use]
    pub fn venue(id: impl Into<String>) -> Self {
        Self::Venue(id.into())
    }

    #[must_use]
    pub fn strategy(id: impl Into<String>) -> Self {
        Self::Strategy(id.into())
    }

    #[must_use]
    pub fn drill(label: impl Into<String>) -> Self {
        Self::Drill(label.into())
    }

    /// Whether this scope can ever refuse a real order.
    ///
    /// Exactly one scope kind cannot, and it is the one the rehearsal uses.
    #[must_use]
    pub fn affects_real_flow(&self) -> bool {
        !matches!(self, Self::Drill(_))
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => f.write_str("global"),
            Self::Venue(v) => write!(f, "venue:{v}"),
            Self::Strategy(s) => write!(f, "strategy:{s}"),
            Self::Drill(d) => write!(f, "drill:{d}"),
        }
    }
}

/// A scope string that does not name any of the four shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownScope(pub String);

impl std::fmt::Display for UnknownScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unrecognised kill-switch scope {:?}", self.0)
    }
}

impl std::error::Error for UnknownScope {}

impl std::str::FromStr for Scope {
    type Err = UnknownScope;

    /// Fail-closed parsing: an unrecognised scope is an error, never `Global` and
    /// never a silently-ignored no-op. A typo that parsed as "everything" would
    /// stop the book; a typo that parsed as "nothing" would fail to.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "global" {
            return Ok(Self::Global);
        }
        if let Some(v) = s.strip_prefix("venue:") {
            if !v.is_empty() {
                return Ok(Self::Venue(v.to_owned()));
            }
        }
        if let Some(v) = s.strip_prefix("strategy:") {
            if !v.is_empty() {
                return Ok(Self::Strategy(v.to_owned()));
            }
        }
        if let Some(v) = s.strip_prefix("drill:") {
            if !v.is_empty() {
                return Ok(Self::Drill(v.to_owned()));
            }
        }
        Err(UnknownScope(s.to_owned()))
    }
}

/// State of the monitor lease as the order path sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseGate {
    /// No monitor is expected of this process — the pre-COMP-007 behaviour, kept
    /// for unit tests and tools that construct a gate with no platform around it.
    /// [`KillSwitch::require_monitor_lease`] leaves this state permanently.
    NotRequired,
    /// A monitor is required and has not been seen alive. **Refuse.**
    Missing,
    /// A monitor renewed its lease within its expiry. Allow.
    Valid,
}

const LEASE_NOT_REQUIRED: u8 = 0;
const LEASE_MISSING: u8 = 1;
const LEASE_VALID: u8 = 2;

/// Full size, in per cent. The unit the monitor's scale-down speaks in.
const FULL_SCALE_PCT: u8 = 100;

/// The order gate's in-memory mirror of the monitor's authority.
///
/// Every read on the order path is a plain atomic load or a lock-free map lookup;
/// nothing here does I/O, blocks, or awaits. The writes come from the refresher
/// task that follows `monitor.lease` and `monitor.kill_switch`.
pub struct KillSwitch {
    global: AtomicBool,
    lease: AtomicU8,
    halted_venues: DashMap<String, ()>,
    halted_strategies: DashMap<String, ()>,
    /// Scope-string keyed, so a drill trip is recorded and observable without
    /// ever being consulted by [`KillSwitch::refuses`].
    halted_drills: DashMap<String, ()>,
    scaled_strategies: DashMap<String, u8>,
}

impl KillSwitch {
    /// Construct the mirror.
    ///
    /// The lease starts [`LeaseGate::NotRequired`]: this constructor is what every
    /// existing caller uses, and silently turning it into a gate that refuses
    /// everything would be a behaviour change nobody asked for in a function that
    /// looks like it only sets a boolean. Arming is a separate, explicit,
    /// irreversible call — see [`KillSwitch::require_monitor_lease`].
    #[must_use]
    pub fn new(initially_active: bool) -> Self {
        Self {
            global: AtomicBool::new(initially_active),
            lease: AtomicU8::new(LEASE_NOT_REQUIRED),
            halted_venues: DashMap::new(),
            halted_strategies: DashMap::new(),
            halted_drills: DashMap::new(),
            scaled_strategies: DashMap::new(),
        }
    }

    // ── The lease (COMP-007 §4 "fail closed") ───────────────────────────────

    /// Require a live monitor lease for every new order, starting now.
    ///
    /// Arming is **one-way**. The state moves `NotRequired → Missing` and there is
    /// no path back: `observe_lease` can only move between `Missing` and `Valid`
    /// once armed. Calling this twice is harmless.
    ///
    /// It arms into `Missing`, not `Valid`, because a process that has just decided
    /// it needs a monitor has by definition not heard from one yet.
    pub fn require_monitor_lease(&self) {
        let _ = self.lease.compare_exchange(
            LEASE_NOT_REQUIRED,
            LEASE_MISSING,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        tracing::info!("monitor lease required — orders refuse until a lease is observed valid");
    }

    /// Record what the refresher saw. Ignored while the lease is not required, so a
    /// stray observation cannot arm the gate by accident and — more importantly —
    /// cannot disarm it.
    pub fn observe_lease(&self, valid: bool) {
        let next = if valid { LEASE_VALID } else { LEASE_MISSING };
        let mut current = self.lease.load(Ordering::Acquire);
        loop {
            if current == LEASE_NOT_REQUIRED || current == next {
                return;
            }
            match self.lease.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
        if valid {
            tracing::info!("monitor lease valid — order flow permitted by the lease gate");
        } else {
            tracing::warn!("monitor lease lost — new orders refused with monitor_unavailable");
        }
    }

    #[must_use]
    pub fn lease_gate(&self) -> LeaseGate {
        match self.lease.load(Ordering::Acquire) {
            LEASE_VALID => LeaseGate::Valid,
            LEASE_MISSING => LeaseGate::Missing,
            _ => LeaseGate::NotRequired,
        }
    }

    /// Whether the lease gate is armed. Once true, always true.
    #[must_use]
    pub fn lease_required(&self) -> bool {
        self.lease.load(Ordering::Acquire) != LEASE_NOT_REQUIRED
    }

    /// The hot-path question: may an order proceed as far as the limit checks?
    ///
    /// `false` means `monitor_unavailable` — not "halted". The distinction matters
    /// to the operator: one of them is a decision and the other is a silence.
    #[must_use]
    pub fn monitor_available(&self) -> bool {
        self.lease.load(Ordering::Acquire) != LEASE_MISSING
    }

    // ── Scoped state ────────────────────────────────────────────────────────

    /// Whether `scope` is halted.
    #[must_use]
    pub fn is_active_scope(&self, scope: &Scope) -> bool {
        match scope {
            Scope::Global => self.global.load(Ordering::Acquire),
            Scope::Venue(v) => self.halted_venues.contains_key(v),
            Scope::Strategy(s) => self.halted_strategies.contains_key(s),
            Scope::Drill(d) => self.halted_drills.contains_key(d),
        }
    }

    /// Whether the switch refuses an order for this venue and strategy.
    ///
    /// Drill scopes are deliberately unreachable from here: a rehearsal asks the
    /// gate and the gate answers, and nothing real is stopped (COMP-007 §4.2).
    #[must_use]
    pub fn refuses(&self, venue: Option<&str>, strategy: Option<&str>) -> bool {
        if self.global.load(Ordering::Acquire) {
            return true;
        }
        if let Some(v) = venue {
            if self.halted_venues.contains_key(v) {
                return true;
            }
        }
        if let Some(s) = strategy {
            if self.halted_strategies.contains_key(s) {
                return true;
            }
        }
        false
    }

    /// Trip `scope`. Idempotent.
    pub fn trip_scope(&self, scope: &Scope) {
        match scope {
            Scope::Global => {
                self.global.store(true, Ordering::Release);
                tracing::warn!("kill switch tripped: global — all new order flow halted");
            }
            Scope::Venue(v) => {
                self.halted_venues.insert(v.clone(), ());
                tracing::warn!(venue = %v, "kill switch tripped: venue order flow halted");
            }
            Scope::Strategy(s) => {
                self.halted_strategies.insert(s.clone(), ());
                tracing::warn!(strategy = %s, "kill switch tripped: strategy halted");
            }
            Scope::Drill(d) => {
                self.halted_drills.insert(d.clone(), ());
                tracing::info!(drill = %d, "DRILL kill switch tripped — nothing real is refused");
            }
        }
    }

    /// Release `scope`. The authority for whether this was allowed lives in the API
    /// route (typed confirmation, reason, rate limit); the mirror only reflects it.
    pub fn reset_scope(&self, scope: &Scope) {
        match scope {
            Scope::Global => self.global.store(false, Ordering::Release),
            Scope::Venue(v) => {
                self.halted_venues.remove(v);
            }
            Scope::Strategy(s) => {
                self.halted_strategies.remove(s);
            }
            Scope::Drill(d) => {
                self.halted_drills.remove(d);
            }
        }
        tracing::info!(scope = %scope, "kill switch reset");
    }

    /// Replace the whole scoped view in one pass — what the refresher calls after
    /// reading `monitor.kill_switch`.
    ///
    /// A replace rather than a merge, because a scope that disappeared from the
    /// table has been reset, and a mirror that only ever learns about new halts
    /// drifts into refusing things nothing is halting any more.
    pub fn replace_halted(&self, halted: &[Scope]) {
        let mut global = false;
        self.halted_venues.clear();
        self.halted_strategies.clear();
        self.halted_drills.clear();
        for scope in halted {
            match scope {
                Scope::Global => global = true,
                Scope::Venue(v) => {
                    self.halted_venues.insert(v.clone(), ());
                }
                Scope::Strategy(s) => {
                    self.halted_strategies.insert(s.clone(), ());
                }
                Scope::Drill(d) => {
                    self.halted_drills.insert(d.clone(), ());
                }
            }
        }
        self.global.store(global, Ordering::Release);
    }

    /// Every scope currently halted, sorted, for the API's read surface.
    #[must_use]
    pub fn halted_scopes(&self) -> Vec<Scope> {
        let mut out = Vec::new();
        if self.global.load(Ordering::Acquire) {
            out.push(Scope::Global);
        }
        out.extend(
            self.halted_venues
                .iter()
                .map(|e| Scope::Venue(e.key().clone())),
        );
        out.extend(
            self.halted_strategies
                .iter()
                .map(|e| Scope::Strategy(e.key().clone())),
        );
        out.extend(
            self.halted_drills
                .iter()
                .map(|e| Scope::Drill(e.key().clone())),
        );
        out.sort();
        out
    }

    // ── Automatic scale-down (COMP-007 §4 P4) ───────────────────────────────

    /// Set a strategy's size as a percentage of full. Reversible, therefore
    /// automatic — that is the whole reason scale-down does not need a human and
    /// a halt reset does.
    pub fn scale_strategy(&self, strategy: &str, pct: u8) {
        let pct = pct.min(FULL_SCALE_PCT);
        if pct >= FULL_SCALE_PCT {
            self.scaled_strategies.remove(strategy);
        } else {
            self.scaled_strategies.insert(strategy.to_owned(), pct);
        }
    }

    /// A strategy's current size, in per cent of full. 100 when unscaled.
    #[must_use]
    pub fn scale_pct(&self, strategy: &str) -> u8 {
        self.scaled_strategies
            .get(strategy)
            .map_or(FULL_SCALE_PCT, |e| *e.value())
    }

    // ── Legacy global surface ───────────────────────────────────────────────

    /// Whether the **global** scope is tripped.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.global.load(Ordering::Acquire)
    }

    /// Trip the global scope.
    pub fn trip(&self) {
        self.trip_scope(&Scope::Global);
    }

    /// Reset the global scope.
    pub fn reset(&self) {
        self.reset_scope(&Scope::Global);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn new_switch_defaults_inactive() {
        assert!(!KillSwitch::new(false).is_active());
    }

    #[test]
    fn new_switch_can_start_active() {
        assert!(KillSwitch::new(true).is_active());
    }

    #[test]
    fn trip_and_reset_move_the_global_scope() {
        let ks = KillSwitch::new(false);
        ks.trip();
        assert!(ks.is_active());
        ks.reset();
        assert!(!ks.is_active());
    }

    #[test]
    fn a_venue_halt_leaves_the_other_venues_trading() {
        let ks = KillSwitch::new(false);
        ks.trip_scope(&Scope::venue("kraken"));
        assert!(ks.refuses(Some("kraken"), None));
        assert!(!ks.refuses(Some("alpaca"), None));
        assert!(!ks.refuses(None, None));
    }

    #[test]
    fn a_strategy_halt_leaves_the_other_strategies_trading() {
        let ks = KillSwitch::new(false);
        ks.trip_scope(&Scope::strategy("s-1"));
        assert!(ks.refuses(None, Some("s-1")));
        assert!(!ks.refuses(None, Some("s-2")));
    }

    #[test]
    fn a_drill_trip_refuses_nothing_real() {
        // COMP-007 §4.2: the rehearsal asks the gate and the gate answers.
        let ks = KillSwitch::new(false);
        ks.trip_scope(&Scope::drill("global"));
        assert!(
            ks.is_active_scope(&Scope::drill("global")),
            "the drill state is observable"
        );
        assert!(
            !ks.refuses(Some("kraken"), Some("s-1")),
            "and refuses nothing real"
        );
        assert!(!ks.is_active());
    }

    #[test]
    fn arming_the_lease_refuses_before_any_monitor_is_seen() {
        let ks = KillSwitch::new(false);
        assert!(ks.monitor_available(), "unarmed, the lease gate is open");
        ks.require_monitor_lease();
        assert!(
            !ks.monitor_available(),
            "armed and unseen is a refusal, not a pass"
        );
    }

    #[test]
    fn a_lost_lease_cannot_be_argued_back_open() {
        let ks = KillSwitch::new(false);
        ks.require_monitor_lease();
        ks.observe_lease(true);
        assert!(ks.monitor_available());
        ks.observe_lease(false);
        assert!(!ks.monitor_available());
        // And the only way back is a genuine observation of a live lease.
        ks.observe_lease(true);
        assert!(ks.monitor_available());
    }

    #[test]
    fn observing_a_lease_on_an_unarmed_switch_changes_nothing() {
        let ks = KillSwitch::new(false);
        ks.observe_lease(false);
        assert_eq!(ks.lease_gate(), LeaseGate::NotRequired);
        assert!(ks.monitor_available());
    }

    #[test]
    fn arming_twice_does_not_reopen_the_gate() {
        let ks = KillSwitch::new(false);
        ks.require_monitor_lease();
        ks.observe_lease(true);
        ks.require_monitor_lease();
        assert!(
            ks.monitor_available(),
            "a second arm must not throw away a valid lease"
        );
        assert!(ks.lease_required());
    }

    #[test]
    fn replace_halted_drops_scopes_that_are_no_longer_in_the_table() {
        let ks = KillSwitch::new(false);
        ks.replace_halted(&[Scope::Global, Scope::venue("kraken")]);
        assert!(ks.is_active());
        assert!(ks.refuses(Some("kraken"), None));
        ks.replace_halted(&[Scope::venue("alpaca")]);
        assert!(
            !ks.is_active(),
            "global was reset in the table, so the mirror drops it"
        );
        assert!(!ks.refuses(Some("kraken"), None));
        assert!(ks.refuses(Some("alpaca"), None));
    }

    #[test]
    fn scale_is_per_strategy_and_full_is_the_default() {
        let ks = KillSwitch::new(false);
        assert_eq!(ks.scale_pct("s-1"), 100);
        ks.scale_strategy("s-1", 25);
        assert_eq!(ks.scale_pct("s-1"), 25);
        assert_eq!(ks.scale_pct("s-2"), 100);
        ks.scale_strategy("s-1", 100);
        assert_eq!(ks.scale_pct("s-1"), 100);
    }

    #[test]
    fn scope_round_trips_through_its_string_form() {
        for scope in [
            Scope::Global,
            Scope::venue("kraken"),
            Scope::strategy("11111111-2222-3333-4444-555555555555"),
            Scope::drill("global"),
        ] {
            assert_eq!(Scope::from_str(&scope.to_string()), Ok(scope));
        }
    }

    #[test]
    fn an_unrecognised_scope_is_an_error_rather_than_global() {
        // A typo that parsed as Global would stop the book; one that parsed as a
        // no-op would fail to stop it. Neither is an acceptable reading of "eh?".
        for bad in [
            "",
            "GLOBAL",
            "venue:",
            "strategy:",
            "venues:kraken",
            "everything",
        ] {
            assert!(Scope::from_str(bad).is_err(), "{bad:?} must not parse");
        }
    }
}
