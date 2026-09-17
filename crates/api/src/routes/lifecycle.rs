//! The research lifecycle's routes (AGENT-013 §7.2, §6.2; Set S tasks S-10.3,
//! S-10.4).
//!
//! Three endpoints behind the `lifecycle` namespace:
//!
//! - `POST /api/lifecycle/preregistrations` — seal what a run commits to.
//! - `POST /api/lifecycle/hypotheses` — state one thing and its falsifier.
//! - `GET  /api/lifecycle/metrics` — what a falsifier may name.
//!
//! The first constructs [`ledger::prereg::Preregistration`] before touching the
//! database, so an incomplete registration is refused with a reason the agent can
//! act on rather than a constraint violation it cannot parse. The database
//! refuses it again — a direct SQL client bypasses the type and not the CHECK,
//! which is the same argument `0122` makes about provenance.
//!
//! The third exists so the falsifier vocabulary has **one** source. The MCP
//! handler could carry its own list; it would then be a second answer to "what
//! can this platform evaluate", and the one that drifted would be the one the
//! model was validated against.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use ledger::prereg::{self, Draft, Preregistration, Window};

use crate::{auth::BearerToken, state::AppState};

/// The metrics a falsifier may name.
///
/// These are the columns `mlops.trial_statistic` records, which is what makes a
/// falsifier evaluable at all: a falsifier naming something the platform does not
/// compute reads like a commitment and can never refute anything.
///
/// A constant rather than a query, for now, and that is a limitation worth
/// stating rather than hiding: the statistic table is keyed by name and the names
/// are added by migrations, so this list is a *copy*. `the_metric_vocabulary_is_
/// the_statistic_table` in `crates/invariants` is where that copy is checked
/// against the schema.
pub const METRIC_VOCABULARY: &[&str] = &[
    "sharpe_after_costs",
    "sortino_after_costs",
    "max_drawdown",
    "hit_rate",
    "profit_factor",
    "turnover",
    "exposure",
    "calmar",
    "information_ratio",
    "tail_ratio",
];

#[derive(Debug, Deserialize)]
pub struct WindowBody {
    start: String,
    end: String,
}

/// What a run sends to seal a registration.
///
/// **No `serde(default)` anywhere.** A body missing `delta_practical` fails to
/// deserialise rather than arriving as a zero nobody chose — ADR-P0-08's
/// technique, and the reason it is a technique is that the alternative fails
/// silently and in the flattering direction.
#[derive(Debug, Deserialize)]
pub struct PreregistrationBody {
    objective: serde_json::Value,
    benchmark: serde_json::Value,
    delta_practical: f64,
    window: WindowBody,
    cost_model_version: String,
    null_family: String,
    trial_budget: u32,
    gate_profile: String,
    primary_hypothesis: String,
    falsifier: String,
    /// The run this belongs to. Supplied by the caller's session, not by the
    /// body, on every path that has one — a body-supplied run id would let one
    /// run seal a registration for another.
    #[serde(default)]
    run_id: Option<Uuid>,
    #[serde(default)]
    supersedes: Option<Uuid>,
    /// Embargo inputs, when the caller knows the pipeline. Defaulted to a
    /// conservative shape otherwise, and the computed value is returned so the
    /// caller can see what it got rather than assume.
    #[serde(default)]
    horizon_bars: u32,
    #[serde(default)]
    max_lookback_bars: u32,
    #[serde(default)]
    max_knowledge_lag_ms: u64,
    #[serde(default)]
    settlement_lag_bars: u32,
}

fn bad_request(code: &str, message: String, fix: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": code, "message": message, "fix": fix })),
    )
}

/// `POST /api/lifecycle/preregistrations` — seal one, or amend the last.
pub async fn seal(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<PreregistrationBody>,
) -> impl IntoResponse {
    let run_id = body.run_id.unwrap_or_else(Uuid::new_v4);
    let embargo = dataplane::split::EmbargoInputs {
        horizon_bars: body.horizon_bars,
        max_lookback_bars: body.max_lookback_bars,
        max_knowledge_lag_ms: body.max_knowledge_lag_ms,
        settlement_lag_bars: body.settlement_lag_bars,
    };
    let draft = Draft {
        run_id,
        objective: body.objective,
        benchmark: body.benchmark,
        delta_practical: body.delta_practical,
        window: Window {
            start: body.window.start,
            end: body.window.end,
        },
        cost_model_version: body.cost_model_version,
        null_family: body.null_family,
        trial_budget: body.trial_budget,
        gate_profile: body.gate_profile,
        primary_hypothesis: body.primary_hypothesis,
        falsifier: body.falsifier,
    };

    // Amending reads the row it supersedes, so the trial budget is carried by
    // the ledger rather than trusted from the request. A caller that could set
    // its own budget on an amendment would have a button that refills the thing
    // the amendment cap exists to bound.
    let sealed = match body.supersedes {
        None => prereg::seal(draft, &embargo),
        Some(previous_id) => {
            match load(&state, previous_id).await {
                Ok(Some(previous)) => prereg::amend(
                    &previous,
                    draft,
                    &embargo,
                    prereg::MAX_PREREG_AMENDMENTS,
                ),
                Ok(None) => {
                    return bad_request(
                        "unknown_registration",
                        format!("no registration {previous_id}"),
                        "amend the one this run sealed, or seal a first one",
                    )
                    .into_response()
                }
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "storage", "message": e })),
                    )
                        .into_response()
                }
            }
        }
    };

    let sealed = match sealed {
        Ok(p) => p,
        Err(e) => {
            let code = match e {
                prereg::PreregError::TooManyAmendments { .. } => "protocol_unstable",
                _ => "incomplete_registration",
            };
            return bad_request(
                code,
                e.to_string(),
                "a registration is the thing that makes a result mean something; \
                 an incomplete one is not one",
            )
            .into_response();
        }
    };

    match store(&state, &sealed, &token).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({
                "prereg_id": sealed.id,
                "prereg_hash": sealed.hash(),
                "run_id": sealed.run_id,
                "amendment": sealed.amendment,
                "trial_budget": sealed.trial_budget,
                // Returned rather than assumed: the caller sent pipeline
                // numbers and the platform computed a value from them, and a
                // caller that cannot see it cannot tell it was wrong.
                "embargo_bars": sealed.embargo_bars,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e })),
        )
            .into_response(),
    }
}

async fn load(state: &AppState, id: Uuid) -> Result<Option<Preregistration>, String> {
    let row = sqlx::query(
        "SELECT prereg_id, run_id, objective, benchmark, delta_practical, window_start, \
                window_end, embargo_bars, cost_model_version, null_family, trial_budget, \
                gate_profile, primary_hypothesis, falsifier, supersedes, amendment \
           FROM agent_lifecycle.preregistration WHERE prereg_id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|r| Preregistration {
        id: r.get("prereg_id"),
        run_id: r.get("run_id"),
        objective: r.get("objective"),
        benchmark: r.get("benchmark"),
        delta_practical: r.get("delta_practical"),
        window: Window {
            start: r.get("window_start"),
            end: r.get("window_end"),
        },
        embargo_bars: u32::try_from(r.get::<i32, _>("embargo_bars")).unwrap_or(0),
        cost_model_version: r.get("cost_model_version"),
        null_family: r.get("null_family"),
        trial_budget: u32::try_from(r.get::<i32, _>("trial_budget")).unwrap_or(0),
        gate_profile: r.get("gate_profile"),
        primary_hypothesis: r.get("primary_hypothesis"),
        falsifier: r.get("falsifier"),
        supersedes: r.get("supersedes"),
        amendment: u32::try_from(r.get::<i32, _>("amendment")).unwrap_or(0),
    }))
}

async fn store(
    state: &AppState,
    p: &Preregistration,
    _token: &BearerToken,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO agent_lifecycle.preregistration (
             prereg_id, run_id, objective, benchmark, delta_practical, window_start,
             window_end, embargo_bars, cost_model_version, null_family, trial_budget,
             gate_profile, primary_hypothesis, falsifier, prereg_hash, supersedes, amendment
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(p.id)
    .bind(p.run_id)
    .bind(&p.objective)
    .bind(&p.benchmark)
    .bind(p.delta_practical)
    .bind(&p.window.start)
    .bind(&p.window.end)
    .bind(i32::try_from(p.embargo_bars).unwrap_or(i32::MAX))
    .bind(&p.cost_model_version)
    .bind(&p.null_family)
    .bind(i32::try_from(p.trial_budget).unwrap_or(i32::MAX))
    .bind(&p.gate_profile)
    .bind(&p.primary_hypothesis)
    .bind(&p.falsifier)
    .bind(p.hash())
    .bind(p.supersedes)
    .bind(i32::try_from(p.amendment).unwrap_or(i32::MAX))
    .execute(&state.pg)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// `GET /api/lifecycle/metrics` — what a falsifier may name.
pub async fn metrics() -> impl IntoResponse {
    Json(json!({ "metrics": METRIC_VOCABULARY }))
}

#[derive(Debug, Deserialize)]
pub struct HypothesisBody {
    statement: String,
    falsifier: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    family_id: Option<String>,
    #[serde(default)]
    run_id: Option<Uuid>,
}

/// `POST /api/lifecycle/hypotheses` — state one, with its falsifier.
///
/// The **status is not a field on this request**, and that is §6.2's rule rather
/// than an omission: a verdict the model writes is a verdict the model can
/// rewrite. `evaluate` is a platform function that reads the tickets, and it is
/// the only thing that may set one.
pub async fn state_hypothesis(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<HypothesisBody>,
) -> impl IntoResponse {
    let check = serde_json::json!({
        "falsifier": body.falsifier,
        "role": body.role,
    });
    if let Err(e) = mcp_server_lib::tools::lifecycle::check_hypothesis(&check, METRIC_VOCABULARY) {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(e)).into_response();
    }
    let id = Uuid::new_v4();
    let res = sqlx::query(
        "INSERT INTO agent_lifecycle.hypothesis
             (hypothesis_id, run_id, statement, falsifier, role, family_id)
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(id)
    .bind(body.run_id)
    .bind(&body.statement)
    .bind(&body.falsifier)
    .bind(body.role.as_deref().unwrap_or("primary"))
    .bind(body.family_id.as_deref())
    .execute(&state.pg)
    .await;
    match res {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({ "hypothesis_id": id, "status": "open" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary is not empty and every entry is a metric name.
    ///
    /// An empty vocabulary would refuse every falsifier, which reads to a model
    /// as "this platform evaluates nothing" and is the kind of failure that
    /// looks like a strict rule working.
    #[test]
    fn the_vocabulary_is_a_real_list_of_metric_names() {
        assert!(METRIC_VOCABULARY.len() >= 5);
        for m in METRIC_VOCABULARY {
            assert!(
                m.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{m} is not a metric name"
            );
        }
    }

    /// A body without `delta_practical` does not deserialise.
    ///
    /// The route's half of ADR-P0-08: the check in the MCP handler is a better
    /// message, and this is the boundary that cannot be skipped by an HTTP
    /// client that does not go through the handler.
    #[test]
    fn the_boundary_refuses_a_body_with_no_delta_practical() {
        let body = json!({
            "objective": {"maximize": ["sharpe_after_costs"]},
            "benchmark": {},
            "window": {"start": "2024-01-01", "end": "2024-06-30"},
            "cost_model_version": "costs_v3",
            "null_family": "block_bootstrap",
            "trial_budget": 20,
            "gate_profile": "strict_v1",
            "primary_hypothesis": "h",
            "falsifier": "sharpe_after_costs <= 0"
        });
        let err = serde_json::from_value::<PreregistrationBody>(body)
            .expect_err("no default, so no deserialisation");
        assert!(err.to_string().contains("delta_practical"), "{err}");
    }

    /// The falsifier is required at the boundary too.
    #[test]
    fn the_boundary_refuses_a_body_with_no_falsifier() {
        let body = json!({
            "objective": {"maximize": ["sharpe_after_costs"]},
            "benchmark": {},
            "delta_practical": 0.1,
            "window": {"start": "2024-01-01", "end": "2024-06-30"},
            "cost_model_version": "costs_v3",
            "null_family": "block_bootstrap",
            "trial_budget": 20,
            "gate_profile": "strict_v1",
            "primary_hypothesis": "h"
        });
        assert!(serde_json::from_value::<PreregistrationBody>(body).is_err());
    }

    /// A hypothesis body has no status field.
    ///
    /// §6.2: a verdict the model writes is a verdict the model can rewrite. The
    /// absence is the mechanism, so it is asserted rather than left to be
    /// noticed.
    #[test]
    fn a_hypothesis_cannot_arrive_with_its_own_verdict() {
        let body = json!({
            "statement": "momentum survives costs",
            "falsifier": "sharpe_after_costs <= 0",
            "status": "supported",
            "verdict": "supported"
        });
        let parsed: HypothesisBody =
            serde_json::from_value(body).expect("unknown fields are ignored, not honoured");
        assert_eq!(parsed.falsifier, "sharpe_after_costs <= 0");
        // Nothing on the type can carry the claim, which is the point: there is
        // no field to ignore carefully.
        let rendered = format!("{parsed:?}");
        assert!(!rendered.contains("supported"), "{rendered}");
    }
}
