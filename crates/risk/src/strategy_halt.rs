//! Per-strategy divergence halts — the mechanism `artifact.md` SC-6 and FM-4
//! describe, living inside the one risk chokepoint so there is nowhere else to
//! put it.
//!
//! ## Why this is in `risk` and not in `reconciliation`
//!
//! The halt has to be consulted *before the next order*, and the only place
//! every order passes is [`crate::RiskGate::check`] (SC-2). A registry owned by
//! `reconciliation` would have to be consulted by each caller, and "each
//! caller" is how a second chokepoint gets built by accident. So the registry
//! lives here, the gate reads it unconditionally, and `reconciliation` — which
//! already depends on `risk` — writes to it.
//!
//! ## Why it is per strategy rather than the global kill switch
//!
//! [`crate::KillSwitch`] stops everything, which is right for a position
//! divergence on a shared account and wrong for "this one strategy's realised
//! returns stopped matching its backtest". A divergence is evidence about *that
//! strategy's* simulator fidelity. Halting the whole platform on it would teach
//! operators to clear halts reflexively, and a halt people clear reflexively is
//! a log line, not a safety mechanism.
//!
//! ## What is deliberately absent
//!
//! There is no `set_enabled`, no `bypass`, no `clear_all`, and no way to ask
//! this type to stop halting. Clearing one halt takes a named human and a
//! recorded reason. This absence is asserted statically by
//! `crates/risk/tests/no_divergence_bypass.rs` — if it were only a convention,
//! the first urgent Friday would end it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// What kind of disagreement stopped the strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceKind {
    /// The internal position view disagrees with the broker's (FM-4).
    Position,
    /// Realised returns diverged from the backtest's expectation beyond the
    /// threshold declared for this deployment.
    Return,
    /// Cash or buying power disagrees with the broker's (SC-6's "balance").
    Balance,
}

impl DivergenceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Return => "return",
            Self::Balance => "balance",
        }
    }
}

/// A live halt on one strategy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DivergenceHalt {
    pub halt_id: Uuid,
    pub strategy_id: String,
    pub kind: DivergenceKind,
    /// Human-readable, carries the numbers that tripped it. Rendered in the UI.
    pub reason: String,
    pub raised_at: DateTime<Utc>,
}

/// A person. Not an enum of principals and not `Option<String>`: clearing a
/// divergence halt is the one action in this module that a machine must not be
/// able to take, and a type that cannot be constructed empty is how that is
/// said in the type system rather than in a comment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HumanActor(String);

impl HumanActor {
    /// # Errors
    /// An empty or whitespace-only identifier.
    pub fn new(id: impl Into<String>) -> Result<Self, ClearError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ClearError::AnonymousActor);
        }
        Ok(Self(id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The record written when a halt is lifted. Returned so the caller persists it;
/// a clearance that left no trace would make the halt reversible in silence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HaltClearance {
    pub halt_id: Uuid,
    pub strategy_id: String,
    pub kind: DivergenceKind,
    pub halt_reason: String,
    pub raised_at: DateTime<Utc>,
    pub cleared_by: HumanActor,
    /// Why the human believes it is safe to resume. Required, non-empty.
    pub note: String,
    pub cleared_at: DateTime<Utc>,
}

/// Why a clearance was refused.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ClearError {
    #[error("no active divergence halt for strategy {0}")]
    NotHalted(String),
    #[error("a divergence halt may only be cleared by a named human")]
    AnonymousActor,
    #[error("a divergence halt may only be cleared with a stated reason")]
    NoReason,
}

/// The set of strategies currently stopped on divergence.
///
/// In-memory and synchronous, mirroring [`crate::KillSwitch`]: the hot path
/// cannot afford a database round trip before every order, and a halt that
/// depended on one would fail *open* when the database was slow. Durability is
/// the caller's job — `mlops.divergence_halt` (migration 0105) is where the
/// raise and the clearance are written — and boot re-hydrates from it.
#[derive(Debug, Default)]
pub struct StrategyHaltRegistry {
    halted: Mutex<HashMap<String, DivergenceHalt>>,
    /// Orders this registry has refused. Read by the drill in
    /// `crates/reconciliation/tests/divergence_halts_before_next_order.rs`.
    blocked: AtomicU64,
}

impl StrategyHaltRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop this strategy's order flow.
    ///
    /// Idempotent in the direction that matters: a second divergence on an
    /// already-halted strategy keeps the **first** halt, because the first one
    /// is the one whose timestamp bounds the window in which orders may have
    /// gone out on a wrong view.
    pub fn halt(
        &self,
        strategy_id: impl Into<String>,
        kind: DivergenceKind,
        reason: impl Into<String>,
    ) -> DivergenceHalt {
        let strategy_id = strategy_id.into();
        let mut guard = self.halted.lock().expect("halt registry lock");
        guard
            .entry(strategy_id.clone())
            .or_insert_with(|| {
                let halt = DivergenceHalt {
                    halt_id: Uuid::new_v4(),
                    strategy_id: strategy_id.clone(),
                    kind,
                    reason: reason.into(),
                    raised_at: Utc::now(),
                };
                tracing::error!(
                    strategy_id = %halt.strategy_id,
                    kind = halt.kind.as_str(),
                    reason = %halt.reason,
                    "DIVERGENCE HALT — order submission stopped for this strategy"
                );
                halt
            })
            .clone()
    }

    /// Re-establish a halt read back from the database at boot, preserving its
    /// identity and the time it was originally raised.
    ///
    /// Without this, a restart would clear every halt — which is the failure
    /// mode of any in-memory safety flag, and the one that looks like the
    /// system deciding on its own that the divergence is over.
    pub fn rehydrate(&self, halt: DivergenceHalt) {
        let mut guard = self.halted.lock().expect("halt registry lock");
        guard.entry(halt.strategy_id.clone()).or_insert(halt);
    }

    /// Whether this strategy may submit. `None` (a manual order with no
    /// strategy) is never halted by this registry — the global kill switch
    /// covers that case.
    #[must_use]
    pub fn is_halted(&self, strategy_id: &str) -> bool {
        self.halted
            .lock()
            .expect("halt registry lock")
            .contains_key(strategy_id)
    }

    /// The live halt for a strategy, if any — carries the reason for the UI and
    /// for the rejection message.
    #[must_use]
    pub fn halt_for(&self, strategy_id: &str) -> Option<DivergenceHalt> {
        self.halted
            .lock()
            .expect("halt registry lock")
            .get(strategy_id)
            .cloned()
    }

    /// Every live halt, oldest first. The UI surface.
    #[must_use]
    pub fn active(&self) -> Vec<DivergenceHalt> {
        let mut v: Vec<_> = self
            .halted
            .lock()
            .expect("halt registry lock")
            .values()
            .cloned()
            .collect();
        v.sort_by_key(|h| h.raised_at);
        v
    }

    /// How many orders this registry has refused since process start.
    #[must_use]
    pub fn blocked_orders(&self) -> u64 {
        self.blocked.load(Ordering::Acquire)
    }

    /// Called by [`crate::RiskGate::check`] on every order. Increments the
    /// refusal counter when it says no.
    pub(crate) fn admit(&self, strategy_id: Option<&str>) -> Result<(), DivergenceHalt> {
        let Some(id) = strategy_id else {
            return Ok(());
        };
        match self.halt_for(id) {
            None => Ok(()),
            Some(halt) => {
                self.blocked.fetch_add(1, Ordering::AcqRel);
                Err(halt)
            }
        }
    }

    /// Lift one halt. The only way a halted strategy resumes.
    ///
    /// Takes a [`HumanActor`] and a non-empty note, and returns the
    /// [`HaltClearance`] the caller must persist. There is deliberately no
    /// variant of this that takes neither.
    ///
    /// # Errors
    /// [`ClearError::NotHalted`] when the strategy is not halted, and
    /// [`ClearError::NoReason`] when the note is empty.
    pub fn clear(
        &self,
        strategy_id: &str,
        cleared_by: HumanActor,
        note: impl Into<String>,
    ) -> Result<HaltClearance, ClearError> {
        let note = note.into();
        if note.trim().is_empty() {
            return Err(ClearError::NoReason);
        }
        let mut guard = self.halted.lock().expect("halt registry lock");
        let halt = guard
            .remove(strategy_id)
            .ok_or_else(|| ClearError::NotHalted(strategy_id.to_owned()))?;
        tracing::warn!(
            strategy_id = %halt.strategy_id,
            halt_id = %halt.halt_id,
            cleared_by = cleared_by.as_str(),
            %note,
            "divergence halt cleared by a human — order flow resumes for this strategy"
        );
        Ok(HaltClearance {
            halt_id: halt.halt_id,
            strategy_id: halt.strategy_id,
            kind: halt.kind,
            halt_reason: halt.reason,
            raised_at: halt.raised_at,
            cleared_by,
            note,
            cleared_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_halted_strategy_is_not_admitted() {
        let reg = StrategyHaltRegistry::new();
        assert!(reg.admit(Some("s1")).is_ok());
        reg.halt("s1", DivergenceKind::Return, "realised -3% vs expected +1%");
        assert!(reg.admit(Some("s1")).is_err());
        assert_eq!(reg.blocked_orders(), 1);
    }

    #[test]
    fn a_halt_on_one_strategy_does_not_halt_another() {
        let reg = StrategyHaltRegistry::new();
        reg.halt("s1", DivergenceKind::Position, "broker says 2, we say 1");
        assert!(reg.admit(Some("s2")).is_ok());
        assert!(reg.admit(None).is_ok());
    }

    #[test]
    fn a_second_divergence_keeps_the_first_halt_and_its_timestamp() {
        let reg = StrategyHaltRegistry::new();
        let first = reg.halt("s1", DivergenceKind::Return, "first");
        let second = reg.halt("s1", DivergenceKind::Position, "second");
        assert_eq!(first.halt_id, second.halt_id);
        assert_eq!(second.reason, "first");
        assert_eq!(second.kind, DivergenceKind::Return);
    }

    #[test]
    fn clearing_requires_a_named_human() {
        assert_eq!(HumanActor::new("  "), Err(ClearError::AnonymousActor));
        assert!(HumanActor::new("mason").is_ok());
    }

    #[test]
    fn clearing_requires_a_stated_reason() {
        let reg = StrategyHaltRegistry::new();
        reg.halt("s1", DivergenceKind::Return, "diverged");
        let err = reg
            .clear("s1", HumanActor::new("mason").unwrap(), "   ")
            .expect_err("an empty note must not clear a halt");
        assert_eq!(err, ClearError::NoReason);
        assert!(reg.is_halted("s1"), "the halt must survive a refused clear");
    }

    #[test]
    fn clearing_an_unhalted_strategy_is_an_error_not_a_no_op() {
        let reg = StrategyHaltRegistry::new();
        let err = reg
            .clear("ghost", HumanActor::new("mason").unwrap(), "looks fine")
            .expect_err("clearing nothing must not succeed");
        assert_eq!(err, ClearError::NotHalted("ghost".into()));
    }

    #[test]
    fn a_cleared_halt_resumes_and_leaves_a_record() {
        let reg = StrategyHaltRegistry::new();
        let raised = reg.halt("s1", DivergenceKind::Balance, "cash off by 412.00");
        let rec = reg
            .clear(
                "s1",
                HumanActor::new("mason").unwrap(),
                "broker restated an overnight sweep; positions agree",
            )
            .unwrap();
        assert_eq!(rec.halt_id, raised.halt_id);
        assert_eq!(rec.cleared_by.as_str(), "mason");
        assert_eq!(rec.halt_reason, "cash off by 412.00");
        assert!(reg.admit(Some("s1")).is_ok());
    }

    #[test]
    fn rehydrate_restores_a_halt_across_a_restart() {
        let before = StrategyHaltRegistry::new();
        let halt = before.halt("s1", DivergenceKind::Position, "diverged");

        // Process restarts; the in-memory map is empty until boot reads the row.
        let after = StrategyHaltRegistry::new();
        assert!(!after.is_halted("s1"));
        after.rehydrate(halt.clone());
        assert!(after.is_halted("s1"));
        assert_eq!(after.halt_for("s1").unwrap().halt_id, halt.halt_id);
        assert_eq!(after.halt_for("s1").unwrap().raised_at, halt.raised_at);
    }
}
