//! Account-creation seeding.
//!
//! Every new account is seeded with the default 7/21 EMA cross-asset discovery
//! strategy.  The fixture is embedded at compile time so the seed is always
//! consistent regardless of the filesystem layout.

use domain::strategy_def::StrategyDefinition;

/// The default 7/21 EMA discovery strategy fixture, embedded from the
/// `strategy-runtime` crate's `fixtures/default_ema.json` at compile time.
const DEFAULT_EMA_JSON: &str = include_str!("../../../strategy-runtime/fixtures/default_ema.json");

/// Return the default cross-asset EMA discovery strategy.
///
/// This strategy:
/// - Requires only `market.ohlcv` (cross-asset compatible).
/// - Has no `PlaceOrder` action → `StrategyKind::Discovery`.
/// - Detects 7-period EMA crossing over 21-period EMA on 1-minute bars.
///
/// # Panics
/// Panics only if the embedded fixture JSON is malformed (compile-time invariant).
pub fn default_ema_strategy() -> StrategyDefinition {
    serde_json::from_str(DEFAULT_EMA_JSON)
        .expect("default_ema.json fixture must be valid StrategyDefinition JSON")
}
