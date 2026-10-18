//! The Gate 3 model-binding policy, over the model registry (Set N N-3.6).
//!
//! `crates/backtest` defines `ModelUsagePolicy` because `crates/model-registry` depends on it,
//! not the other way round. This is the implementation: it reads a bound version's latest
//! successful evaluation through the same function the promotion gate uses
//! (`model_registry::manager::usage_verdict_pg`) and applies the hard usage rules.

use backtest::gates::model_binding::{BindingDecision, ModelBinding, ModelUsagePolicy, ModelUse};
use model_registry::scoring_v2::ForecastUse;
use sqlx::PgPool;

pub struct RegistryUsagePolicy {
    pg: PgPool,
}

impl RegistryUsagePolicy {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }
}

#[must_use]
pub fn forecast_use(u: ModelUse) -> ForecastUse {
    match u {
        ModelUse::Return => ForecastUse::Return,
        ModelUse::Vol => ForecastUse::Vol,
        ModelUse::TailRisk => ForecastUse::TailRisk,
    }
}

#[async_trait::async_trait]
impl ModelUsagePolicy for RegistryUsagePolicy {
    async fn decide(&self, b: &ModelBinding) -> BindingDecision {
        let uses: Vec<ForecastUse> = b.uses.iter().copied().map(forecast_use).collect();
        let report =
            model_registry::manager::usage_verdict_pg(&self.pg, &b.model_id, b.version, &uses)
                .await;
        if report.allowed {
            BindingDecision::allow()
        } else {
            BindingDecision::refuse(report.reasons())
        }
    }
}

/// Judge every binding against the registry; `Err` names each refused model. The gate worker
/// calls this before it spends any evidence gathering on a candidate that cannot pass.
///
/// # Errors
/// The refusals, joined.
pub async fn check_bindings(pg: &PgPool, bindings: &[ModelBinding]) -> Result<(), String> {
    backtest::gates::model_binding::BindingOutcome::judge(
        bindings,
        &RegistryUsagePolicy::new(pg.clone()),
    )
    .await
    .satisfied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_binding_use_maps_to_the_registry_use_of_the_same_name() {
        assert_eq!(forecast_use(ModelUse::Return), ForecastUse::Return);
        assert_eq!(forecast_use(ModelUse::Vol), ForecastUse::Vol);
        assert_eq!(forecast_use(ModelUse::TailRisk), ForecastUse::TailRisk);
    }

    /// With no evaluation to read, the verdict is not an allowance: a missing measurement blocks.
    #[tokio::test]
    async fn a_binding_with_no_recorded_evaluation_is_refused() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(200))
            .connect_lazy("postgres://localhost:1/none")
            .expect("lazy pool");
        let d = RegistryUsagePolicy::new(pool)
            .decide(&ModelBinding {
                model_id: "m".into(),
                version: 1,
                uses: vec![ModelUse::Vol],
            })
            .await;
        assert!(!d.allowed && !d.reasons.is_empty());
    }
}
