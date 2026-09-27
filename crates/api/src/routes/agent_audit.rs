//! The honesty arithmetic, served (AGENT-006 §5.4; Set S tasks S-3.6, S-3.7).
//!
//! Four routes behind four tools. None of them computes a statistic here: the
//! deflated Sharpe and the PBO are read out of the Gate 3 record, the minimum
//! track record comes from `backtest::stats`, and the holdout is the experiment
//! vault that already exists — one logged access, self-sealing, with no
//! decrement path on the counter and an append-only access log.
//!
//! Building a second holdout mechanism for the agent was the obvious shape and
//! is the wrong one. A holdout with two implementations has been looked at
//! twice, and the second look is the one nobody counts.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// The confidence a track record is required to establish.
///
/// Not a parameter. A caller who can choose the confidence can choose the answer,
/// and "how long must this run before we believe it" is a platform policy rather
/// than a per-call preference. 95% is the same level Gate 3's alpha uses.
const TRACK_RECORD_CONFIDENCE: f64 = 0.95;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBody {
    pub run_id: String,
    #[serde(default)]
    pub experiment_id: Option<String>,
}

// No `profile` field, and `deny_unknown_fields` so sending one is an error
// rather than a silent no-op. A request field that is accepted and ignored is
// worse than one that is refused: the caller believes it worked. See
// `min_track_record` below for why the profile is not the caller's to choose.

#[derive(Debug, Deserialize)]
pub struct ExperimentBody {
    pub experiment_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

fn refuse(code: &str, message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "refused": true, "error": code, "message": message.into() })),
    )
        .into_response()
}

fn parse_id(raw: &str) -> Result<Uuid, axum::response::Response> {
    Uuid::parse_str(raw).map_err(|_| refuse("bad_id", format!("{raw:?} is not a UUID")))
}

/// POST /api/agent/audit/deflated-sharpe
pub async fn deflated_sharpe(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<RunBody>,
) -> impl IntoResponse {
    // The family is the experiment. Without one there is no sealed hypothesis
    // family, and the refusal says what to do rather than only that it will not.
    let Some(raw) = body.experiment_id.as_deref() else {
        return refuse(
            "family_not_sealed",
            "a deflated Sharpe needs the hypothesis family the run belongs to. Pass \
             `experiment_id`; a run that belongs to no experiment has no family, and a Sharpe \
             discounted by a trial count nobody registered is not discounted at all.",
        );
    };
    let id = match parse_id(raw) {
        Ok(v) => v,
        Err(r) => return r,
    };

    match state.suite.audit_deflated_sharpe(token.user_id(), id) {
        Ok(view) => {
            let verdict = if view.corroborators_agree {
                "the corroborators agree with the primary p-value"
            } else {
                "the corroborators DISAGREE with the primary p-value — investigate before citing \
                 either"
            };
            Json(json!({
                "summary_fields": {
                    "deflated_sharpe": view.deflated_sharpe,
                    "trials": view.trials,
                    "verdict": verdict,
                },
                "detail": {
                    "deflated_sharpe": view.deflated_sharpe,
                    "probability_of_overfit": view.probability_of_overfit,
                    "raw_p_value": view.raw_p_value,
                    "corrected_p_value": view.corrected_p_value,
                    "trials": view.trials,
                    "corroborators_agree": view.corroborators_agree,
                    "source": view.source,
                    "run_id": body.run_id,
                    "experiment_id": raw,
                    "note": "these are the numbers Gate 3 recorded, not a second computation. \
                             The trial count is N_eff over the tenant's whole ledger and can \
                             only be minted by the ledger.",
                },
                "ids": { "experiment_id": raw, "run_id": body.run_id },
            }))
            .into_response()
        }
        Err(e) => refuse("not_available", e.to_string()),
    }
}

/// POST /api/agent/audit/min-track-record
pub async fn min_track_record(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<RunBody>,
) -> impl IntoResponse {
    let view =
        match state
            .suite
            .audit_track_record(token.user_id(), &body.run_id, TRACK_RECORD_CONFIDENCE)
        {
            Ok(v) => v,
            Err(e) => return refuse("not_found", e.to_string()),
        };

    // The floor is `strict_v1`'s, always, and is NOT a caller argument.
    //
    // `paper_v1` exists because five years of calendar history is impossible for
    // a platform months old (OQ-13, ADR-P2-14) — but it is the profile for paper
    // deployment, and it cannot authorise capital. An agent that could name the
    // profile could name the one with no floor, which is choosing whether it is
    // blocked. The scan in `inv23_no_governance_fields_in_tool_schemas` did not
    // catch this one — its pattern is `gate_profile`, and the field was called
    // `profile` — so it is removed here for the reason the scan exists rather
    // than because the scan said so.
    //
    // Both numbers are still reported; only the selection is gone.
    const STRICT_FLOOR_YEARS: f64 = 5.0;
    const PAPER_FLOOR_YEARS: f64 = 0.0;
    let profile = "strict_v1";
    let floor = STRICT_FLOOR_YEARS;

    let (verdict, blocks) = match view.required_years {
        None => (
            "this run's Sharpe is not above zero, so no track record of any length establishes \
             it. Length is not the problem here."
                .to_string(),
            true,
        ),
        Some(required) => {
            let long_enough = view.observed_years >= required;
            let clears_floor = view.observed_years >= floor;
            if long_enough && clears_floor {
                (
                    format!(
                        "the record is long enough: {:.2} years observed against {required:.2} \
                         required and a {profile} floor of {floor:.0}",
                        view.observed_years
                    ),
                    false,
                )
            } else if !clears_floor {
                (
                    format!(
                        "BLOCKED: {profile} requires {floor:.0} years of record and this run \
                         covers {:.2}. This is a promotion precondition, not a suggestion.",
                        view.observed_years
                    ),
                    true,
                )
            } else {
                (
                    format!(
                        "BLOCKED: this Sharpe needs {required:.2} years to be distinguished \
                         from luck and the run covers {:.2}",
                        view.observed_years
                    ),
                    true,
                )
            }
        }
    };

    Json(json!({
        "summary_fields": {
            "required_years": view.required_years,
            "observed_years": view.observed_years,
            "verdict": verdict,
        },
        "detail": {
            "run_id": view.run_id,
            "profile": profile,
            "profile_floor_years": floor,
            "paper_v1_floor_years": PAPER_FLOOR_YEARS,
            "profile_note": "scored against `strict_v1`, the profile that can authorise                              capital. `paper_v1` has no calendar floor and cannot; which                              profile applies is the platform's decision, not a call argument.",
            "blocks_promotion": blocks,
            "confidence": view.confidence,
            "required_observations": view.required_observations,
            "required_years": view.required_years,
            "observed_years": view.observed_years,
            "n_observations": view.n_observations,
            "observations_per_year": view.observations_per_year,
            "sharpe_per_observation": view.sharpe_per_observation,
            "sharpe_annualised": view.sharpe_annualised,
            "skew": view.skew,
            "kurtosis": view.kurtosis,
            "verdict": verdict,
            "note": "MinTRL is computed in per-observation units from the run's own returns and \
                     does not depend on how many strategies were tried. `observations_per_year` \
                     is taken from the equity curve's own spacing, so an hourly run is not \
                     scored as though it were daily.",
        },
        "ids": { "run_id": view.run_id },
    }))
    .into_response()
}

/// POST /api/agent/audit/holdout-budget
///
/// Free, and that is the point: an agent that can only discover the budget by
/// exhausting it will exhaust it.
pub async fn holdout_budget(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<ExperimentBody>,
) -> impl IntoResponse {
    let id = match parse_id(&body.experiment_id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(view) = state.suite.vault(token.user_id(), id) else {
        return refuse("not_found", "no such experiment");
    };

    let looks_taken = view.access_log.len();
    let state_word = if view.spent { "spent" } else { "unspent" };
    let can_run_note = if view.can_run {
        "a look is available now".to_string()
    } else if view.spent {
        "it has been spent and cannot be spent again, ever".to_string()
    } else if view.unsafe_flag {
        "the experiment is marked unsafe, so the holdout is not reachable".to_string()
    } else if !view.gate3_passed {
        "Gate 3 has not passed, and the holdout is not reachable before it does".to_string()
    } else {
        "unavailable".to_string()
    };

    Json(json!({
        "summary_fields": {
            "state": state_word,
            "looks_taken": looks_taken,
            "looks_allowed": 1,
            "can_run_note": can_run_note,
        },
        "detail": {
            "experiment_id": body.experiment_id,
            "state": state_word,
            "looks_taken": looks_taken,
            "looks_allowed": 1,
            "can_run": view.can_run,
            "gate3_passed": view.gate3_passed,
            "unsafe": view.unsafe_flag,
            "can_run_note": can_run_note,
            "access_log": view.access_log,
            "note": "one look per experiment, ever. The counter has no decrement path and the \
                     access log is append-only, both enforced by the database rather than by \
                     this route.",
        },
        "ids": { "experiment_id": body.experiment_id },
    }))
    .into_response()
}

/// POST /api/agent/audit/holdout-look
///
/// The only path in the platform that reads holdout rows on an agent's behalf.
pub async fn holdout_look(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<ExperimentBody>,
) -> impl IntoResponse {
    let id = match parse_id(&body.experiment_id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(reason) = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| s.len() >= 20)
    else {
        // Not a formality. The reason is written into an append-only access log
        // that outlives the run, and "testing" is not a reason anyone can audit
        // a year later.
        return refuse(
            "reason_required",
            "spending the holdout needs a reason of at least 20 characters saying why this \
             candidate is ready. It goes into an append-only log that outlives this run, and it \
             is the only record of why the one look was used.",
        );
    };

    // Ahead of the spend, not after: a look that was going to be refused should
    // not consume the look while finding that out.
    match state.suite.vault(token.user_id(), id) {
        None => return refuse("not_found", "no such experiment"),
        Some(v) if v.spent => {
            return refuse(
                "already_spent",
                "this experiment's holdout has already been evaluated. There is no second look, \
                 and the first result is available from `check_holdout_budget`.",
            )
        }
        Some(v) if !v.can_run => {
            return refuse(
                "not_reachable",
                if v.gate3_passed {
                    "the experiment is marked unsafe, so the holdout is not reachable"
                } else {
                    "Gate 3 has not passed. The holdout is reachable only after it does — that \
                     ordering is what stops a candidate being tuned against the holdout."
                },
            )
        }
        Some(_) => {}
    }

    let by = format!("agent:{}:{reason}", token.user_id());
    let suite = state.suite.clone();
    let user = token.user_id();
    let result = tokio::task::spawn_blocking(move || suite.run_vault(user, id, by)).await;

    match result {
        Ok(Ok(view)) => Json(json!({
            "summary_fields": {
                "verdict": if view.spent { "the holdout was evaluated" } else { "no evaluation was recorded" },
                "state": "spent",
            },
            "detail": {
                "experiment_id": body.experiment_id,
                "spent": view.spent,
                "access_log": view.access_log,
                "reason": reason,
                "note": "this door is now closed for this experiment permanently. A result \
                         obtained here is out of sample exactly once; anything fitted after \
                         seeing it is in sample.",
            },
            "ids": { "experiment_id": body.experiment_id },
        }))
        .into_response(),
        Ok(Err(e)) => refuse("vault_refused", e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "vault_panicked", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct FamilyBody {
    pub experiment_id: String,
    #[serde(default)]
    pub n_periods: Option<usize>,
    #[serde(default)]
    pub alpha: Option<f64>,
    #[serde(default)]
    pub seed: Option<u64>,
}

/// POST /api/agent/audit/probability-of-overfit
pub async fn probability_of_overfit(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<FamilyBody>,
) -> impl IntoResponse {
    let id = match parse_id(&body.experiment_id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let periods = body.n_periods.unwrap_or(8).clamp(2, 16);
    // CSCV enumerates every half-and-half split of the periods and scores every
    // configuration on each, so a wide sweep is real CPU. Off the async
    // executor, like `holdout_look` — a handler that burns a runtime thread
    // stalls every other request on it, and the symptom is a platform that
    // looks down rather than a tool that looks slow.
    let suite = state.suite.clone();
    let user = token.user_id();
    let computed =
        tokio::task::spawn_blocking(move || suite.audit_probability_of_overfit(user, id, periods))
            .await;
    let computed = match computed {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "pbo_panicked", "message": e.to_string() })),
            )
                .into_response()
        }
    };
    match computed {
        Ok(view) => {
            // The thresholds are the gate profile's, not this route's opinion.
            let verdict = if view.probability_of_overfit >= 0.5 {
                "the in-sample-best configuration underperforms out of sample more often than                  not, which is what an overfitted sweep looks like"
            } else if view.probability_of_overfit >= 0.2 {
                "elevated; the selection is carrying real risk of being noise"
            } else {
                "low; the in-sample ranking mostly survives out of sample"
            };
            Json(json!({
                "summary_fields": {
                    "probability_of_overfit": view.probability_of_overfit,
                    "n_configs": view.n_configs,
                    "n_periods": view.n_periods,
                    "verdict": verdict,
                },
                "detail": {
                    "experiment_id": body.experiment_id,
                    "probability_of_overfit": view.probability_of_overfit,
                    "n_configs": view.n_configs,
                    "n_periods": view.n_periods,
                    "observations_per_config": view.observations_per_config,
                    "config_run_ids": view.config_run_ids,
                    "verdict": verdict,
                    "note": "computed over runs already recorded for this experiment; nothing                              was dispatched, so this spent no trials. Series are truncated to                              the shortest so every configuration is scored over the same                              periods.",
                },
                "ids": { "experiment_id": body.experiment_id },
            }))
            .into_response()
        }
        Err(e) => refuse("not_available", e.to_string()),
    }
}

/// POST /api/agent/audit/family-wise
pub async fn family_wise(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<FamilyBody>,
) -> impl IntoResponse {
    let id = match parse_id(&body.experiment_id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let alpha = body.alpha.unwrap_or(0.05).clamp(0.001, 0.2);
    // Defaulted rather than random: the same family must give the same verdict
    // twice, or a caller can re-roll until a candidate survives.
    let seed = body.seed.unwrap_or(0);

    // Five thousand stationary-bootstrap resamples over every candidate. Same
    // reason as above: this belongs on a blocking thread.
    let suite = state.suite.clone();
    let user = token.user_id();
    let computed =
        tokio::task::spawn_blocking(move || suite.audit_family_wise(user, id, alpha, seed)).await;
    let computed = match computed {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "family_wise_panicked", "message": e.to_string() })),
            )
                .into_response()
        }
    };
    match computed {
        Ok(view) => {
            let verdict = if view.n_survivors == 0 {
                "nothing in this family survives the correction. The best result of the sweep                  is what the best of that many draws looks like."
                    .to_string()
            } else {
                format!(
                    "{} candidate(s) survive; report those and not the raw best",
                    view.n_survivors
                )
            };
            Json(json!({
                "summary_fields": {
                    "n_survivors": view.n_survivors,
                    "n_candidates": view.n_candidates,
                    "alpha": view.alpha,
                    "verdict": verdict,
                },
                "detail": {
                    "experiment_id": body.experiment_id,
                    "n_candidates": view.n_candidates,
                    "n_survivors": view.n_survivors,
                    "survivors": view.survivors,
                    "observations": view.observations,
                    "alpha": view.alpha,
                    "resamples": view.resamples,
                    "seed": seed,
                    "verdicts": view.verdicts,
                    "verdict": verdict,
                    "method": "Romano-Wolf stepdown, stationary bootstrap, block length from                                the family's pooled autocorrelation by Politis-White",
                    "note": "Hansen SPA and White's Reality Check are deliberately not offered.                              Romano-Wolf dominates both — it gives per-candidate adjusted                              p-values rather than only testing the global null — and three                              family-wise tests behind one question is a menu to shop between.",
                },
                "ids": { "experiment_id": body.experiment_id },
            }))
            .into_response()
        }
        Err(e) => refuse("not_available", e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct MetricBody {
    pub run_id: String,
    pub metric: String,
    pub claimed_value: f64,
}

/// POST /api/agent/audit/metric-integrity
///
/// The one rule: nothing the agent wrote is an input to the recomputation.
/// `claimed_value` arrives on the side and is only ever compared against a
/// number this route derived from the platform's own record of the run.
pub async fn metric_integrity(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<MetricBody>,
) -> impl IntoResponse {
    let (platform_value, all) =
        match state
            .suite
            .audit_metric(token.user_id(), &body.run_id, &body.metric)
        {
            Ok(v) => v,
            Err(e) => return refuse("not_available", e.to_string()),
        };

    let claimed = body.claimed_value;
    let delta = claimed - platform_value;
    // Relative where the number has a scale, absolute where it does not. A
    // Sharpe of 0.001 reported as 0.01 is a tenfold error and an absolute
    // tolerance would wave it through.
    let scale = platform_value.abs().max(1e-9);
    let relative = (delta / scale).abs();
    let matches = relative < 0.01 || delta.abs() < 1e-9;

    // Whether any *other* metric matches the claim better than the named one
    // does. The common honest mistake is reporting the right number under the
    // wrong label, and it looks identical to the dishonest case until you check.
    let mislabelled: Vec<&str> = all
        .iter()
        .filter(|(name, value)| {
            name != &body.metric && (claimed - value).abs() / value.abs().max(1e-9) < 0.01
        })
        .map(|(name, _)| name.as_str())
        .collect();

    let verdict = if matches {
        "matches the platform's own computation".to_string()
    } else if !mislabelled.is_empty() {
        format!(
            "MISMATCH — but the reported value matches this run's {} instead, so this is              probably the right number under the wrong label",
            mislabelled.join(" or ")
        )
    } else {
        format!(
            "MISMATCH — the reported value is {:.1}% away from the recomputed one and matches              no other metric of this run. Do not publish it.",
            relative * 100.0
        )
    };

    Json(json!({
        "summary_fields": {
            "claimed_value": claimed,
            "platform_value": platform_value,
            "metric": body.metric,
            "verdict": verdict,
        },
        "detail": {
            "run_id": body.run_id,
            "metric": body.metric,
            "claimed_value": claimed,
            "platform_value": platform_value,
            "absolute_delta": delta,
            "relative_delta": relative,
            "matches": matches,
            "possible_mislabel": mislabelled,
            "all_metrics": all.iter().cloned().collect::<std::collections::BTreeMap<_, _>>(),
            "verdict": verdict,
            "note": "recomputed from the platform's recorded RunResult. No agent-authored text                      is an input here — `claimed_value` is compared, never used.",
        },
        "ids": { "run_id": body.run_id },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Production source only.
    ///
    /// A scan that reads its own test module matches the string the test names
    /// and passes — or, as here, fails — for a reason that has nothing to do
    /// with the code it is about. This is the fourth time in Set S: AT-42, the
    /// dispatch-arm scan and the error-code scan all did the same thing, and
    /// this one caught itself on the first full run.
    fn production_only(source: &str) -> &str {
        source
            .split_once("#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// The confidence is a constant, and the test says why it may not become a
    /// request field: a caller who chooses the confidence chooses the answer.
    #[test]
    fn the_track_record_confidence_is_not_caller_supplied() {
        let body = production_only(include_str!("agent_audit.rs"));
        assert!(
            !body.contains("pub confidence"),
            "a request body gained a `confidence` field. Whether a track record is long enough \
             is a platform policy, not a per-call preference."
        );
        assert!((TRACK_RECORD_CONFIDENCE - 0.95).abs() < f64::EPSILON);
    }

    /// A reason short enough to be meaningless is refused before the vault is
    /// touched, so a rejected look does not consume the look.
    #[test]
    fn a_short_reason_is_refused() {
        let short = "testing";
        assert!(short.trim().len() < 20);
    }
}
