//! The re-read falsification reads (S-3.13): `regime_failure_search` and
//! `parameter_fragility_map`.
//!
//! Both read results the platform already holds -- a finished run's equity curve
//! with the platform's own regime labels, and a finished sweep's surface -- and
//! run nothing, so neither spends a trial (ADR-D-12). The arithmetic is in
//! `research::{regime_search, fragility}`; these handlers only resolve the data
//! under the caller's ownership and say so when it is not there.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct RegimeQuery {
    /// Fewest periods a regime needs to be tested. Default 250.
    #[serde(default)]
    pub min_obs: Option<usize>,
}

/// The one significance level for the regime hunt. A platform constant: a
/// caller who can set it can set it until a regime survives.
const REGIME_ALPHA: f64 = 0.05;

/// GET /api/agent/falsification/regime-failure/{run_id}
pub async fn regime_failure(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<String>,
    Query(q): Query<RegimeQuery>,
) -> impl IntoResponse {
    let min_obs = q.min_obs.unwrap_or(250);
    if !(30..=100_000).contains(&min_obs) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "invalid_min_obs", "got": min_obs, "fix": "30 to 100000"})),
        )
            .into_response();
    }
    let Some(result) = state.suite.run_result(token.user_id(), &run_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "run not found in any of your studies"})),
        )
            .into_response();
    };
    let windows = state
        .research
        .regime_windows(token.user_id(), &run_id, &result)
        .await;
    if windows.is_empty() {
        // Unmeasured, never "no regime failed".
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "regimes_unmeasured",
                "message": "no causal regime labels cover this run's window (no configured market \
                            scope, an ambiguous one, or no labels). That is not the same as no \
                            regime failing; the run's behaviour by regime is unknown.",
            })),
        )
            .into_response();
    }
    match research::regime_search::search(&result.equity_curve, &windows, min_obs, REGIME_ALPHA) {
        Ok(r) => Json(json!({"run_id": run_id, "search": r, "spends_trial": false})).into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "not_searchable", "message": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/agent/falsification/fragility/{sweep_id}
pub async fn fragility(
    State(state): State<AppState>,
    token: BearerToken,
    Path(sweep_id): Path<Uuid>,
) -> impl IntoResponse {
    let Some(snap) = state.research.get(token.user_id(), sweep_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "sweep not found"})),
        )
            .into_response();
    };
    let Some(report) = snap.report else {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "sweep_not_finished",
                "status": snap.status,
                "message": "a fragility index is read from a finished sweep's surface",
            })),
        )
            .into_response();
    };
    let f = research::fragility::fragility(
        &report.surface,
        report.neighbourhood_plateau,
        &report.distribution,
    );
    Json(json!({"sweep_id": sweep_id, "fragility": f, "spends_trial": false})).into_response()
}

// ── S-3.13 `ablation_sweep`: the variants, and the verdicts ───────────────────
//
// The `ablation_sweep` tool registers each variant as a draft strategy through
// the ordinary `POST /api/strategies` door, runs one `StudyKind::Ablation` study
// over `[baseline, variants..]` (each member a counted look, so the trial counter
// is charged by the study engine and not here), then asks for the verdicts. These
// two routes are the arithmetic on either side of that; neither writes anything.

/// Seed for the paired bootstrap. A platform constant: re-rolling it until a
/// component reads as load-bearing is the error the tool exists to correct.
const ABLATION_SEED: u64 = 0;

/// The id a variant is stored under. Deterministic, so a retry registers over the
/// same row instead of littering the strategy table with copies.
fn variant_id(base: &str, index: usize) -> String {
    format!("{base}--ablate-{index}")
}

/// GET /api/agent/falsification/ablation/{experiment_id}/variants
pub async fn ablation_variants(
    State(state): State<AppState>,
    token: BearerToken,
    Path(experiment_id): Path<Uuid>,
) -> impl IntoResponse {
    let Some(exp) = state.suite.get_experiment(token.user_id(), experiment_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "experiment not found"})),
        )
            .into_response();
    };
    let def: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
    )
    .bind(&exp.strategy_ref)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();
    let Some(def) = def else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "strategy_not_stored",
                "message": format!("the experiment's strategy `{}` is not in the strategy table", exp.strategy_ref),
            })),
        )
            .into_response();
    };
    match ablation_plan(&exp.strategy_ref, &def) {
        Ok(variants) => Json(json!({
            "strategy_ref": exp.strategy_ref,
            "variants": variants,
            "spends_trial": false,
        }))
        .into_response(),
        Err((error, message)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": error, "message": message})),
        )
            .into_response(),
    }
}

/// The variants of a stored strategy, each with the id it will be stored under.
///
/// Pure, so the refusals are testable without a database.
///
/// # Errors
/// A v3 graph (no single meaning for neutralising a node), or a strategy with
/// nothing to leave out.
pub fn ablation_plan(
    strategy_ref: &str,
    def: &serde_json::Value,
) -> Result<Vec<serde_json::Value>, (&'static str, String)> {
    if backtest::runner::is_v3(def) {
        return Err((
            "ablation_not_defined_for_v3",
            "leave-one-rule-out is defined over a v1 definition's conditions and exit rules; a \
             tbot.strategy/3 graph has no single meaning for neutralising a node, and a guess \
             would be a result with a false label"
                .to_string(),
        ));
    }
    let variants: Vec<serde_json::Value> = research::ablation::variants(def)
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let id = variant_id(strategy_ref, i);
            let mut d = v.definition;
            d["strategy_id"] = json!(id);
            json!({"component": v.component, "strategy_id": id, "definition": d})
        })
        .collect();
    if variants.is_empty() {
        return Err((
            "nothing_to_ablate",
            "the strategy has no condition (other than a tautology) and no exit rule to leave out"
                .to_string(),
        ));
    }
    Ok(variants)
}

#[derive(Debug, Deserialize)]
pub struct AblationClassifyBody {
    pub study_id: String,
    /// The components, in the order the variants were put in the study after the
    /// baseline.
    pub components: Vec<String>,
}

/// POST /api/agent/falsification/ablation/{experiment_id}/classify
pub async fn ablation_classify(
    State(state): State<AppState>,
    token: BearerToken,
    Path(experiment_id): Path<Uuid>,
    Json(body): Json<AblationClassifyBody>,
) -> impl IntoResponse {
    let Some(studies) = state.suite.list_studies(token.user_id(), experiment_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "experiment not found"})),
        )
            .into_response();
    };
    let Some(study) = studies.iter().find(|s| s.study_id == body.study_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "study_not_found", "study_id": body.study_id})),
        )
            .into_response();
    };
    if study.kind != backtest::study::StudyKind::Ablation {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "not_an_ablation_study", "kind": study.kind})),
        )
            .into_response();
    }
    if study.members.len() != body.components.len() + 1 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "members_do_not_match",
                "message": format!(
                    "the study ran {} members but {} components were named; the baseline plus one \
                     run per component is expected",
                    study.members.len(),
                    body.components.len()
                ),
            })),
        )
            .into_response();
    }
    let baseline = state
        .suite
        .run_result(token.user_id(), &study.members[0])
        .filter(|r| r.status.is_ok());
    let Some(baseline) = baseline else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "baseline_failed",
                "message": "the baseline run did not complete, so no variant has anything to be \
                            compared with",
            })),
        )
            .into_response();
    };
    let mut rows = Vec::new();
    for (component, run_id) in body.components.iter().zip(study.members.iter().skip(1)) {
        let r = state.suite.run_result(token.user_id(), run_id);
        let row = match r {
            Some(r) if r.status.is_ok() => {
                let c = research::ablation::classify_equity(
                    &baseline.equity_curve,
                    &r.equity_curve,
                    ABLATION_SEED,
                );
                json!({"component": component, "run_id": run_id, "result": c})
            }
            _ => json!({
                "component": component,
                "run_id": run_id,
                "result": {"verdict": "inconclusive", "reason": "the variant's run did not complete"},
            }),
        };
        rows.push(row);
    }
    Json(json!({
        "study_id": body.study_id,
        "baseline_run_id": study.members[0],
        "components": rows,
        "trial_delta": study.trial_delta,
        "seed": ABLATION_SEED,
    }))
    .into_response()
}

#[cfg(test)]
mod ablation_tests {
    use super::*;

    #[test]
    fn a_v1_strategy_plans_one_variant_per_rule_each_under_a_stable_id() {
        let def = json!({
            "strategy_id": "s",
            "nodes": [
                {"id": "n1", "type": "condition", "expr": "bar('close') > bar('open')"},
                {"id": "n2", "type": "signal", "when": "n1", "emit": "long"}
            ],
            "exits": {"mode": "rules", "rules": [{"rule": "stop_loss", "fraction": "0.02"}]}
        });
        let plan = ablation_plan("s", &def).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0]["strategy_id"], "s--ablate-0");
        assert_eq!(plan[0]["definition"]["strategy_id"], "s--ablate-0");
        assert_eq!(plan[1]["component"], "exit:0:stop_loss");
        // Planning twice gives the same ids, so a retry overwrites rather than litters.
        assert_eq!(plan, ablation_plan("s", &def).unwrap());
    }

    #[test]
    fn a_strategy_with_nothing_to_leave_out_is_refused_by_name() {
        let def = json!({"strategy_id": "s", "nodes": [], "exits": {"mode": "risk_layer_only"}});
        assert_eq!(ablation_plan("s", &def).unwrap_err().0, "nothing_to_ablate");
    }

    #[test]
    fn a_v3_graph_is_refused_rather_than_guessed_at() {
        let def = json!({"format": "tbot.strategy/3", "nodes": []});
        assert_eq!(ablation_plan("s", &def).unwrap_err().0, "ablation_not_defined_for_v3");
    }
}
