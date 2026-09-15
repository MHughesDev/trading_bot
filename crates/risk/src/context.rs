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

use crate::gate::GateContext;

/// Supplies the measured risk context for an order about to be gated.
///
/// Implementations do no blocking I/O: they read in-memory account state and a
/// metadata snapshot that is refreshed out of band.  Anything they cannot
/// measure is returned as `Measured::Unavailable`, which the gate refuses —
/// an implementation must never substitute a permissive value.
pub trait RiskContextProvider: Send + Sync {
    /// Build the context for `intent`, or explain why it cannot be built.
    fn context_for(&self, intent: &OrderIntent) -> Result<GateContext, RiskRejection>;
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
        let err = context_for(&intent).unwrap_err();
        assert!(
            matches!(err, RiskRejection::GateUnavailable { .. }),
            "a missing provider must refuse, not produce a permissive context"
        );
    }
}
