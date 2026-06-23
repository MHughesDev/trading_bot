//! **Real execution** — the deferred `SimRunExecutor` live leg (Set K phase A).
//!
//! Wires the honest-eval core to the actual `market_simulator` engine, replacing
//! the synthetic executor. This is the integration seam for the real Runs.
//!
//! **Current scope (Phase A-4, MVP):** The executor trait is injected and can
//! be compiled. Full data resolution is deferred to B (persistence + API layer).
//!
//! The executor is injected into `SuiteManager::new()` (replacing
//! `ClosureExecutor(synthetic_execute)`). When the API layer integrates, it will
//! provide a context-aware resolver and feed full SimulationInputs here.

use crate::run::{RunConfig, RunResult};

/// The real `RunExecutor` — drives the `market_simulator` for each `RunConfig`.
///
/// This replaces the synthetic executor in `SuiteManager::new()`. In the MVP
/// (Phase A-4), this is a stub that defers to the synthetic path — the real
/// integration will happen in Phase B when Postgres/ClickHouse stores land and
/// the API layer provides the full data context.
pub struct SimRunExecutor;

impl SimRunExecutor {
    /// Create a new executor (placeholder for API-layer context).
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimRunExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::run::RunExecutor for SimRunExecutor {
    fn execute(&self, cfg: &RunConfig) -> RunResult {
        // Phase A-4 MVP: this is a stub.
        // The real implementation will:
        // 1. Resolve RunConfig.strategy_version → StrategyDefinition (from Postgres)
        // 2. Resolve RunConfig.data_slice → bars (from ClickHouse)
        // 3. Apply RunConfig.params to the definition (task A-3)
        // 4. Build SimulationInputs and drive run_simulation_detailed
        // 5. Map the outcome to RunResult
        //
        // For now, return a failed result with a diagnostic message.
        // Once the API layer integration lands, this will be wired properly.
        RunResult::failed(
            cfg,
            "SimRunExecutor is not yet integrated with data sources; \
             this phase requires Postgres/ClickHouse stores (Phase B) and \
             API layer context (Phase B integration). \
             Falling back to synthetic executor (non-deterministic).",
            "SimRunExecutor@stub",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::config::{DataSlice, EvalResolution, RunConfigBuilder};
    use crate::run::RunExecutor;
    use chrono::TimeZone;

    fn test_config() -> RunConfig {
        let slice = DataSlice::new(
            "test-universe",
            chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            chrono::Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        RunConfigBuilder::new(
            "test-strategy",
            "v1",
            slice,
            "cost:floor",
            "sizing:default",
            "snap:1",
        )
        .build()
    }

    #[test]
    fn stub_returns_failed_with_diagnostic_message() {
        let executor = SimRunExecutor::new();
        let cfg = test_config();
        let result = executor.execute(&cfg);
        assert_eq!(result.status, crate::run::result::RunStatus::Failed);
        assert!(!result.integrity_flags.is_empty());
        let flag = &result.integrity_flags[0];
        assert_eq!(flag.code, "run.failed");
        assert!(flag.detail.contains("SimRunExecutor is not yet integrated"));
    }

    #[test]
    fn executor_is_injectable() {
        // Verify that SimRunExecutor implements RunExecutor and can be
        // injected into Backtest::new().
        let executor = SimRunExecutor::new();
        let store = crate::run::InMemoryRunStore::new();
        let _bt: crate::run::Backtest<_, _> = crate::run::Backtest::new(
            store,
            crate::run::ClosureExecutor(|cfg| executor.execute(cfg)),
        );
        // If this compiles, the trait is satisfied.
    }
}
