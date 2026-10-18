//! Running the evidence funnel for a campaign's candidate (SPEC §10 GATE,
//! §12.3; OPEN-TASKS C1.1/C1.6).
//!
//! # What was missing
//!
//! `GateAdvance` judges sixteen gates from numbers the **funnel**
//! (`SuiteManager::advance_funnel`) records: the CPCV 5th percentile, the
//! walk-forward Sharpe, PBO, the deflated Sharpe, the cost ladder, capacity, the
//! neighbourhood, per-instrument P&L. Nothing in a campaign ran it, so a campaign
//! that reached GATE had no evidence for gates 3-8, 10 or 12, and the driver
//! stopped with `gate_evidence_missing`.
//!
//! Running it blind would have gated the wrong thing, for two reasons that are
//! fixed in `backtest::suite`: the funnel built its Runs from the strategy's
//! defaults, not from the campaign's incumbent, and its neighbourhood study and
//! PBO grid perturbed a parameter hard-coded as `fast` around 12. Both are now
//! data on the Experiment (`set_incumbent`, `SweepAxis`).
//!
//! # The axis (ADR-C-12)
//!
//! The funnel perturbs one parameter. Which one is chosen **deterministically from
//! the strategy's declared parameters**: the first numeric one in declaration
//! order. Not the most sensitive: sensitivity is measured by running things, and
//! choosing the axis after seeing which one flatters the candidate is exactly the
//! gate-hacking §12.7 forbids. It is centred on the incumbent's value (the
//! declared midpoint when the incumbent does not set it) and stepped so that
//! `centre ± 4 steps` stays inside the declared range.

#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::sync::Arc;

use backtest::run::ParamMap;
use backtest::suite::{SuiteManager, SweepAxis};
use research::space::{Dim, DimKind};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::study_worker::{CampaignEvaluator, EvaluationSpec};

/// The neighbourhood study perturbs `k = 4` steps each side.
const NEIGHBOURHOOD_STEPS: f64 = 4.0;

/// The axis the funnel perturbs, from the strategy's declared parameters and the
/// incumbent. `None` when the strategy declares no numeric parameter: there is
/// then nothing to perturb, and perturbing a made-up one would be the old bug.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn sweep_axis(dims: &[Dim], incumbent: &ParamMap) -> Option<SweepAxis> {
    let dim = dims.iter().find(|d| d.is_numeric())?;
    let held = incumbent.get(&dim.name).and_then(Value::as_f64);
    match &dim.kind {
        DimKind::Int { min, max, step } => {
            let (lo, hi) = (*min as f64, *max as f64);
            let centre = held.unwrap_or((lo + hi) / 2.0).clamp(lo, hi).round();
            let room = (centre - lo).min(hi - centre);
            let declared = (*step).max(1) as f64;
            // Whole steps only, at least one, and as wide as fits in the range.
            let fit = (room / NEIGHBOURHOOD_STEPS).floor().max(1.0);
            Some(SweepAxis {
                param: dim.name.clone(),
                center: centre,
                step: declared.min(fit),
            })
        }
        DimKind::Float { min, max, .. } => {
            let centre = held.unwrap_or((min + max) / 2.0).clamp(*min, *max);
            let room = (centre - min).min(max - centre);
            let wide = (max - min) / 20.0;
            let fit = room / NEIGHBOURHOOD_STEPS;
            let step = if fit > 0.0 { wide.min(fit) } else { wide };
            (step.is_finite() && step > 0.0).then(|| SweepAxis {
                param: dim.name.clone(),
                center: centre,
                step,
            })
        }
        DimKind::Enum { .. } => None,
    }
}

/// The newest trial of a campaign's experiment that carries funnel statistics
/// (`deflated_sharpe` is written only by `advance_funnel`), and its dataset.
///
/// # Errors
/// Backend failures.
pub async fn funnel_trial(
    pg: &PgPool,
    tenant: &str,
    experiment: &str,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    let row = sqlx::query_as(
        "SELECT t.trial_id, t.dataset_id FROM mlops.trial t
          WHERE t.experiment_id = $1
            AND EXISTS (SELECT 1 FROM mlops.trial_statistic s
                         WHERE s.trial_id = t.trial_id AND s.name = 'deflated_sharpe')
          ORDER BY t.registered_at DESC LIMIT 1",
    )
    .bind(experiment)
    .fetch_optional(&mut *tx)
    .await?;
    let _ = tx.commit().await;
    Ok(row)
}

/// What a funnel run settled on.
#[derive(Debug, Clone, PartialEq)]
pub struct FunnelRan {
    pub experiment: Uuid,
    pub axis: SweepAxis,
}

/// Runs the funnel for a campaign. A trait so the GATE logic is tested without
/// a backtest engine.
#[async_trait::async_trait]
pub trait CampaignFunnel: Send + Sync {
    async fn run(
        &self,
        user: Uuid,
        campaign_id: Uuid,
        spec: &EvaluationSpec,
        delta_practical: f64,
        incumbent: &ParamMap,
    ) -> Result<FunnelRan, String>;
}

/// The production funnel: the Backtest Suite.
pub struct SuiteFunnel {
    suite: Arc<SuiteManager>,
    evaluator: Arc<dyn CampaignEvaluator>,
    pg: PgPool,
}

impl SuiteFunnel {
    #[must_use]
    pub fn new(suite: Arc<SuiteManager>, evaluator: Arc<dyn CampaignEvaluator>, pg: PgPool) -> Self {
        Self {
            suite,
            evaluator,
            pg,
        }
    }

    async fn declared_dims(&self, strategy_ref: &str) -> Result<Vec<Dim>, String> {
        let row: Option<(Value,)> = sqlx::query_as(
            "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
        )
        .bind(strategy_ref)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| format!("strategy lookup failed: {e}"))?;
        let (json,) = row.ok_or_else(|| format!("strategy '{strategy_ref}' not found"))?;
        let def: domain::strategy_def::StrategyDefinition =
            serde_json::from_value(json).map_err(|e| format!("invalid stored definition: {e}"))?;
        let space = research::SearchSpace::from_definition(&def, &BTreeMap::new())
            .map_err(|e| e.to_string())?;
        Ok(space.dims)
    }
}

#[async_trait::async_trait]
impl CampaignFunnel for SuiteFunnel {
    async fn run(
        &self,
        user: Uuid,
        campaign_id: Uuid,
        spec: &EvaluationSpec,
        delta_practical: f64,
        incumbent: &ParamMap,
    ) -> Result<FunnelRan, String> {
        let experiment = self
            .evaluator
            .ensure_experiment(user, campaign_id, spec, delta_practical)
            .await?;
        let dims = self.declared_dims(&spec.strategy_ref).await?;
        let axis = sweep_axis(&dims, incumbent).ok_or_else(|| {
            format!(
                "strategy `{}` declares no numeric parameter, so the funnel has nothing to \
                 perturb; refusing to perturb a made-up one",
                spec.strategy_ref
            )
        })?;
        let suite = Arc::clone(&self.suite);
        let params = incumbent.clone();
        let chosen = axis.clone();
        tokio::task::spawn_blocking(move || {
            suite
                .set_incumbent(user, experiment, params, Some(chosen))
                .map_err(|e| e.to_string())?;
            // The recommended null for the strategy's declared type: the choice
            // that needs no override reason, and the one a human is shown first.
            let null = suite
                .recommended_null(user, experiment)
                .ok_or("the experiment vanished")?;
            suite
                .choose_null(user, experiment, null, None)
                .map_err(|e| e.to_string())?;
            suite
                .advance_funnel(user, experiment)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("the funnel task panicked: {e}"))??;
        Ok(FunnelRan { experiment, axis })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn int(name: &str, min: i64, max: i64, step: i64) -> Dim {
        Dim {
            name: name.into(),
            kind: DimKind::Int { min, max, step },
        }
    }
    fn float(name: &str, min: f64, max: f64) -> Dim {
        Dim {
            name: name.into(),
            kind: DimKind::Float { min, max, log: false },
        }
    }
    fn held(k: &str, v: Value) -> ParamMap {
        let mut m = ParamMap::new();
        m.insert(k.into(), v);
        m
    }

    /// The first numeric parameter in declaration order; enums are skipped. Not
    /// the most sensitive: that would be chosen by looking at results.
    #[test]
    fn the_axis_is_the_first_numeric_parameter_and_centres_on_the_incumbent() {
        let dims = vec![
            Dim { name: "mode".into(), kind: DimKind::Enum { choices: vec!["a".into()] } },
            int("window", 5, 50, 1),
            float("threshold", 0.0, 1.0),
        ];
        let a = sweep_axis(&dims, &held("window", json!(20))).unwrap();
        assert_eq!(a.param, "window");
        assert_eq!(a.center, 20.0);
        // Room 15 each side, four steps: whole steps, as wide as the declared one.
        assert_eq!(a.step, 1.0);
    }

    /// Centre ± 4 steps must stay inside the declared range.
    #[test]
    fn the_steps_fit_inside_the_declared_range() {
        let near_edge = sweep_axis(&[int("w", 5, 50, 5)], &held("w", json!(8))).unwrap();
        assert_eq!(near_edge.center, 8.0);
        assert_eq!(near_edge.step, 1.0, "room 3 / 4 steps floors to the minimum whole step");
        let roomy = sweep_axis(&[int("w", 0, 100, 5)], &held("w", json!(50))).unwrap();
        assert_eq!(roomy.step, 5.0, "the declared step when it fits");
        let f = sweep_axis(&[float("t", 0.0, 1.0)], &held("t", json!(0.5))).unwrap();
        assert!((f.step - 0.05).abs() < 1e-12);
        assert!(f.center - 4.0 * f.step >= 0.0 && f.center + 4.0 * f.step <= 1.0);
    }

    /// An incumbent that does not set the parameter is centred on the declared
    /// midpoint, and one outside the range is clamped into it.
    #[test]
    fn a_missing_or_out_of_range_incumbent_value_falls_back_inside_the_range() {
        let a = sweep_axis(&[int("w", 10, 30, 1)], &ParamMap::new()).unwrap();
        assert_eq!(a.center, 20.0);
        let b = sweep_axis(&[int("w", 10, 30, 1)], &held("w", json!(99))).unwrap();
        assert_eq!(b.center, 30.0);
    }

    /// No numeric parameter means no axis, never the legacy `fast`.
    #[test]
    fn a_strategy_with_nothing_numeric_has_no_axis() {
        let dims = vec![Dim { name: "mode".into(), kind: DimKind::Enum { choices: vec!["a".into()] } }];
        assert_eq!(sweep_axis(&dims, &ParamMap::new()), None);
        assert_eq!(sweep_axis(&[], &ParamMap::new()), None);
    }

    #[test]
    fn the_chosen_axis_is_never_the_legacy_one_unless_the_strategy_declares_it() {
        let a = sweep_axis(&[int("slow", 10, 90, 2)], &ParamMap::new()).unwrap();
        assert_ne!(a.param, SweepAxis::legacy().param);
    }
}
