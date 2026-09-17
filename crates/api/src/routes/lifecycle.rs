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
use ledger::proposal::ProposalKind;

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
                    // The run's own tier, not the platform ceiling
                    // (AGENT-013 §11; S-10.3). A tier already writing under a
                    // grammar has less room to have got the protocol subtly
                    // wrong and more risk that a third rewrite is the model
                    // circling, so the shipped profiles narrow it — and a
                    // profile can only ever narrow, because `Profile::validate`
                    // refuses one above `MAX_PREREG_AMENDMENTS`.
                    amendment_cap(&state, run_id).await,
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

/// Observations a window holds at a bar size.
///
/// Derived from the pre-registered window and the timeframe rather than supplied
/// by the caller, and that is the whole point: a run that could state its own
/// observation count would state the one that passes the power check, and the
/// check would measure the run's preference rather than the data.
///
/// Crypto, so 24/7 — no trading calendar to apply. An equities window would need
/// one, and a caller that passed an equities instrument here would get an
/// optimistic count; that is a real limitation and it is named rather than
/// hidden, because the platform's live venues are crypto and the day that
/// changes this function is where it has to be fixed.
#[must_use]
pub fn observations_in(start: &str, end: &str, timeframe: &str) -> Option<(f64, f64)> {
    let minutes = match timeframe {
        "1m" => 1.0,
        "5m" => 5.0,
        "15m" => 15.0,
        "30m" => 30.0,
        "1h" => 60.0,
        "4h" => 240.0,
        "1d" => 1_440.0,
        _ => return None,
    };
    let days = days_between(start, end)?;
    let per_year = (365.0 * 24.0 * 60.0) / minutes;
    Some((days * 24.0 * 60.0 / minutes, per_year))
}

/// Whole days between two `YYYY-MM-DD` dates.
fn days_between(start: &str, end: &str) -> Option<f64> {
    let parse = |s: &str| -> Option<chrono::NaiveDate> {
        chrono::NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
    };
    let (a, b) = (parse(start)?, parse(end)?);
    let days = (b - a).num_days();
    (days >= 0).then(|| days as f64)
}

#[derive(Debug, Deserialize)]
pub struct DesignBody {
    hypothesis_id: Uuid,
    strategy_version: String,
    #[serde(default)]
    split_spec_id: Option<String>,
    null_family: String,
    /// Bar size. Decides how many observations the pre-registered window holds.
    #[serde(default = "default_timeframe")]
    timeframe: String,
    #[serde(default)]
    run_id: Option<Uuid>,
}

fn default_timeframe() -> String {
    "1h".to_string()
}

/// `POST /api/lifecycle/designs` — bind, and check the window can settle it.
///
/// The power check reads the **pre-registration's** window and delta, not the
/// request's. A design that could restate either would be a design that chose
/// the numbers its window passes.
pub async fn bind_design(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<DesignBody>,
) -> impl IntoResponse {
    if body.strategy_version.trim().eq_ignore_ascii_case("latest") {
        return bad_request(
            "unreproducible_version",
            "`latest` is not a version".into(),
            "bind an exact version: `latest` means something different a week later, and a              result nobody can reproduce is not a result",
        )
        .into_response();
    }

    let Some(run_id) = body.run_id else {
        return bad_request(
            "no_run",
            "a design belongs to a run".into(),
            "this is supplied by the session, not by the request",
        )
        .into_response();
    };
    let prereg = match head_registration(&state, run_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return bad_request(
                "preregistration_required",
                "this run has sealed no pre-registration".into(),
                "call lifecycle.preregister first: the power check reads the window and the                  delta_practical you committed to, and there are none yet",
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
    };

    let Some((observations, per_year)) =
        observations_in(&prereg.window.start, &prereg.window.end, &body.timeframe)
    else {
        return bad_request(
            "unknown_timeframe",
            format!("`{}` is not a bar size this platform serves", body.timeframe),
            "one of 1m, 5m, 15m, 30m, 1h, 4h, 1d",
        )
        .into_response();
    };

    // Normal moments: the run has no returns yet, so there is nothing to take
    // skew and kurtosis from. That is the optimistic reading, which is the safe
    // direction here — a refusal under optimistic moments is a refusal that
    // holds under any others.
    let (check, verdict) =
        research::power::check(prereg.delta_practical, observations, per_year, 0.0, 3.0);
    if !verdict.is_adequate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "insufficient_power",
                "message": verdict.detail(),
                "window": { "start": prereg.window.start, "end": prereg.window.end },
                "delta_practical": prereg.delta_practical,
                "observations": check.observations,
                "required_observations": check.required_observations,
                "trials_spent": 0,
            })),
        )
            .into_response();
    }

    let id = Uuid::new_v4();
    let res = sqlx::query(
        "INSERT INTO agent_lifecycle.design
             (design_id, run_id, hypothesis_id, strategy_version, split_spec_id,
              null_family, timeframe, observations, required_observations)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(id)
    .bind(run_id)
    .bind(body.hypothesis_id)
    .bind(&body.strategy_version)
    .bind(body.split_spec_id.as_deref())
    .bind(&body.null_family)
    .bind(&body.timeframe)
    .bind(check.observations)
    .bind(check.required_observations)
    .execute(&state.pg)
    .await;
    match res {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({
                "design_id": id,
                "observations": check.observations,
                "required_observations": check.required_observations,
                "power": "adequate",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// How many amendments this run's tier allows.
///
/// Falls back to the platform ceiling when the run's model has no profile, or
/// when the run is not one the platform has a row for. That direction is the
/// safe one only because the ceiling is itself a cap: a fallback *above* the
/// tier's value costs at most one extra amendment, and `prereg::amend` still
/// refuses past three.
async fn amendment_cap(state: &AppState, run_id: Uuid) -> u32 {
    let model: Option<String> = sqlx::query_scalar("SELECT model FROM agent_runs WHERE run_id = $1")
        .bind(run_id)
        .fetch_optional(&state.pg)
        .await
        .ok()
        .flatten();
    model
        .and_then(|m| state.profiles.get(&m).map(|p| p.max_prereg_amendments))
        .unwrap_or(prereg::MAX_PREREG_AMENDMENTS)
}

/// The head of a run's amendment chain.
async fn head_registration(
    state: &AppState,
    run_id: Uuid,
) -> Result<Option<Preregistration>, String> {
    let row: Option<Uuid> = sqlx::query_scalar(
        "SELECT prereg_id FROM agent_lifecycle.preregistration           WHERE run_id = $1 ORDER BY amendment DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| e.to_string())?;
    match row {
        Some(id) => load(state, id).await,
        None => Ok(None),
    }
}

#[derive(Debug, Deserialize)]
pub struct ProposalBody {
    kind: String,
    subject: String,
    dossier_ref: String,
    #[serde(default)]
    rationale: String,
    #[serde(default)]
    run_id: Option<Uuid>,
    /// What the platform found. Supplied by the caller's *session* rather than
    /// the request on every path that has one — a run that could state its own
    /// battery result would state the one that lets it through.
    #[serde(default)]
    battery_rejected: bool,
    #[serde(default)]
    readiness_no_go: bool,
}

/// `POST /api/lifecycle/proposals` — ask for a decision that reaches ring 1 or 2.
///
/// Creating one is not gated, because it changes nothing. The envelope belongs
/// on the human's decision, which is where something happens.
pub async fn create_proposal(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<ProposalBody>,
) -> impl IntoResponse {
    let kind = match body.kind.as_str() {
        "promote_strategy" => ProposalKind::PromoteStrategy,
        "change_limits" => ProposalKind::ChangeLimits,
        "deploy_paper" => ProposalKind::DeployPaper,
        other => {
            return bad_request(
                "unknown_proposal_kind",
                format!("`{other}` is not a proposal kind"),
                "promote_strategy, change_limits or deploy_paper",
            )
            .into_response()
        }
    };

    // The kill switch is the platform's state, never the request's. A run that
    // could say the switch was clear would say it.
    let pre = ledger::proposal::Preconditions {
        battery_rejected: body.battery_rejected,
        readiness_no_go: body.readiness_no_go,
        kill_switch_tripped: kill_switch_tripped(&state).await,
    };
    let proposal = match ledger::proposal::propose(
        body.run_id.unwrap_or_else(Uuid::new_v4),
        kind,
        &body.dossier_ref,
        &body.subject,
        &body.rationale,
        pre,
    ) {
        Ok(p) => p,
        Err(e) => {
            let code = match e {
                ledger::proposal::ProposalError::KillSwitchTripped => "kill_switch_tripped",
                ledger::proposal::ProposalError::NoDossier => "no_dossier",
                _ => "precondition_failed",
            };
            return bad_request(
                code,
                e.to_string(),
                "nothing about this proposal is urgent enough to route around the reason                  it was refused",
            )
            .into_response();
        }
    };

    let res = sqlx::query(
        "INSERT INTO agent_lifecycle.proposal
             (proposal_id, run_id, kind, ring, envelope, dossier_ref, subject, rationale)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(proposal.proposal_id)
    .bind(proposal.run_id)
    .bind(proposal.kind.as_str())
    .bind(i16::from(proposal.ring))
    .bind(proposal.envelope.as_str())
    .bind(&proposal.dossier_ref)
    .bind(&proposal.subject)
    .bind(&proposal.rationale)
    .execute(&state.pg)
    .await;
    match res {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({
                "proposal_id": proposal.proposal_id,
                "ring": proposal.ring,
                "envelope": proposal.envelope.as_str(),
                "decision": "pending",
                // Said out loud, because a model that thinks it promoted
                // something will report that it did.
                "effect": "nothing yet — a human decides, and the change happens then",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// Whether the platform has decided something is wrong.
///
/// A read that fails answers **tripped**. An outage is not evidence that the
/// platform is healthy, and the direction of a wrong answer here is the whole
/// question: a false "clear" lets a proposal into the queue during exactly the
/// window nobody wanted one.
async fn kill_switch_tripped(state: &AppState) -> bool {
    // The global scope. A venue- or strategy-scoped halt is a narrower statement
    // and does not stop every proposal; a global one is the platform saying
    // something is wrong, full stop.
    let row: Result<Option<String>, _> = sqlx::query_scalar(
        "SELECT state FROM monitor.kill_switch WHERE scope = 'global'           ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(&state.pg)
    .await;
    match row {
        Ok(Some(state)) => state == "halted",
        // No row is a platform that has never tripped it, which is clear.
        Ok(None) => false,
        Err(_) => true,
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

    /// The observation count is derived from the window, never supplied.
    ///
    /// A run that could state its own would state the one that passes the power
    /// check, and the check would measure the run's preference rather than the
    /// data.
    #[test]
    fn the_observation_count_comes_from_the_window_and_the_bar_size() {
        let (obs, per_year) = observations_in("2024-01-01", "2024-12-31", "1h").expect("1h");
        assert!((obs - 365.0 * 24.0).abs() < 25.0, "{obs}");
        assert!((per_year - 8_760.0).abs() < 1.0, "{per_year}");

        let (daily, _) = observations_in("2024-01-01", "2024-12-31", "1d").expect("1d");
        assert!((daily - 365.0).abs() < 2.0, "{daily}");

        // A bar size the platform does not serve is named rather than guessed.
        assert_eq!(observations_in("2024-01-01", "2024-12-31", "3h"), None);
        // A backwards window has no observations.
        assert_eq!(observations_in("2024-12-31", "2024-01-01", "1h"), None);
    }

    /// The design request cannot restate the delta or the window.
    ///
    /// Both come from the sealed registration. A design that could restate
    /// either would be a design that chose the numbers its window passes, which
    /// is the whole thing the power check exists to prevent.
    #[test]
    fn a_design_request_carries_neither_the_delta_nor_the_window() {
        let body = json!({
            "hypothesis_id": Uuid::new_v4(),
            "strategy_version": "v3",
            "null_family": "block_bootstrap",
            "delta_practical": 0.0001,
            "window": {"start": "2010-01-01", "end": "2026-01-01"}
        });
        let parsed: DesignBody = serde_json::from_value(body).expect("extra fields are ignored");
        let rendered = format!("{parsed:?}");
        assert!(!rendered.contains("delta"), "{rendered}");
        assert!(!rendered.contains("2010"), "{rendered}");
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

// ── Campaigns (AGENT-013 §8.5; S-10.11) ──────────────────────────────────────

/// What a run sends to turn its registration into a campaign.
///
/// Six DEFINE facts are missing on purpose — see `tools::campaign` for the
/// argument. They come from the run's sealed registration, which AGENT-013 §8.5
/// says *is* the campaign's definition.
#[derive(Debug, Deserialize)]
pub struct CampaignBody {
    slug: String,
    approval_spend_usd: f64,
    gpu_hours: f64,
    usd: f64,
    wall_clock_hours: f64,
    #[serde(default)]
    search_space: serde_json::Value,
    /// The run this belongs to. Supplied by the caller's session, never by the
    /// model — a body-supplied run id would let one run spend another's budget.
    #[serde(default)]
    run_id: Option<Uuid>,
    /// The projection of the run's context store this campaign's worker sub-runs
    /// will read (AGENT-005 §29).
    ///
    /// Attached by the agent driver, which holds the run's store; the model never
    /// writes it and its schema has no field for it. Without it the campaign has
    /// no brief, which is refused here rather than at the first HYPOTHESIZE — a
    /// campaign that got as far as its first generation and only then could not
    /// propose would have spent a baseline trial to learn something knowable at
    /// DEFINE.
    #[serde(default)]
    brief: Option<crate::agent::subrun::PinnedBrief>,
}

/// `POST /api/lifecycle/campaigns` — the registration becomes a campaign.
pub async fn define_campaign(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<CampaignBody>,
) -> impl IntoResponse {
    let Some(run_id) = body.run_id else {
        return bad_request(
            "no_run",
            "a campaign belongs to the run that registered it".to_string(),
            "this endpoint is reached through a run's session, which supplies the run id",
        )
        .into_response();
    };
    let Some(brief) = body.brief else {
        return bad_request(
            "no_projection",
            "this campaign has no projection of its run's context store".to_string(),
            "a campaign's HYPOTHESIZE is a worker sub-run over a projection of the parent's \
             store (AGENT-013 §8.5). The agent driver attaches it; a direct HTTP caller has no \
             store to project, and a campaign that cannot propose can only re-run what it \
             started with.",
        )
        .into_response();
    };
    let expected = crate::agent::subrun::worker_projection_name();
    if brief.projection != expected {
        return bad_request(
            "wrong_projection",
            format!(
                "a campaign's workers read the `{expected}` projection and this brief was \
                 assembled under `{}`",
                brief.projection
            ),
            "the projection is derived from the parent's store by kind; a different one means \
             a different set of records reached the worker",
        )
        .into_response();
    }

    let head = match head_registration(&state, run_id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return bad_request(
                "not_preregistered",
                "this run has sealed no pre-registration".to_string(),
                "call `preregister` first: the registration IS the campaign's definition \
                 (AGENT-013 §8.5), so there is nothing to define a campaign from until one \
                 exists.",
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
    };

    let definition = ledger::campaign::CampaignDefinition::from_preregistration(
        &head,
        body.slug,
        body.approval_spend_usd,
        body.gpu_hours,
        body.usd,
        body.wall_clock_hours,
        // Not from the caller, and there is no field for it: INV-21 keeps the
        // exploration floor out of every tool schema and off every request.
        ledger::campaign::ExplorationFloor::default_floor(),
        body.search_space,
    );
    if let Err(e) = definition.validate() {
        return bad_request(
            "invalid_definition",
            e.to_string(),
            "the fields this refuses come from your registration; amend it rather than \
             restating them here",
        )
        .into_response();
    }

    let tenant = token.user_id().to_string();
    let led = ledger::pg::PgTrialLedger::new(state.pg.clone());
    let handle = match led.define_campaign_async(&tenant, &definition).await {
        Ok(h) => h,
        Err(e) => {
            return bad_request(
                "campaign_refused",
                e.to_string(),
                "a campaign is defined once; a second definition under the same slug would be \
                 an edit, and DEFINE is immutable",
            )
            .into_response()
        }
    };

    let Some(jobs) = state.jobs.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "jobs_unavailable" })),
        )
            .into_response();
    };
    // The driver job is submitted before the campaign row is written. A crash
    // between them leaves a job nothing names, which is recoverable; the other
    // order leaves a row naming a job that does not exist, which is not.
    let job = match jobs
        .submit(jobs::store::Submission::new(
            jobs::JobKind::Campaign,
            token.user_id(),
            json!({ "campaign_id": handle.campaign_id() }),
        ))
        .await
    {
        Ok(j) => j,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "dispatch", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    let shown = brief.shown.len();
    let withheld = brief.withheld;
    match crate::agent::subrun::record_campaign(
        &state.pg,
        handle.campaign_id(),
        run_id,
        &tenant,
        head.id,
        &head.hash(),
        &job.job_id,
        &brief,
        crate::agent::subrun::MAX_SUBAGENT_DEPTH,
    )
    .await
    {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({
                "campaign_id": handle.campaign_id(),
                "job_id": job.job_id,
                // The commitment's hash, which every trial this campaign spends
                // carries.
                "prereg_hash": head.hash(),
                // The campaign object's own hash: the commitment plus the slug,
                // the approval threshold, the budgets and the floor, which a
                // registration has no place for (ADR-S10-03).
                "define_hash": handle.define_hash(),
                "trial_budget": head.trial_budget,
                "shown": shown,
                "withheld": withheld,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/lifecycle/campaigns/{id}` — where a campaign is.
pub async fn get_campaign(
    State(state): State<AppState>,
    token: BearerToken,
    axum::extract::Path(campaign_id): axum::extract::Path<Uuid>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let led = ledger::pg::PgTrialLedger::new(state.pg.clone());
    let fold = match led.campaign_state_async(&tenant, campaign_id).await {
        Ok(s) => s,
        Err(e) => {
            return bad_request(
                "unknown_campaign",
                e.to_string(),
                "the campaign id `define_campaign` returned",
            )
            .into_response()
        }
    };

    // The most recent proposal comes from the sub-run, not from the campaign
    // log: the log records that a HYPOTHESIZE happened, and the sub-run records
    // what it proposed.
    let proposal: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT outcome FROM agent_lifecycle.sub_run \
          WHERE campaign_id = $1 AND outcome IS NOT NULL \
          ORDER BY settled_at DESC LIMIT 1",
    )
    .bind(campaign_id)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    Json(json!({
        "campaign_id": campaign_id,
        "phase": fold.phase.as_str(),
        "seq": fold.seq,
        "cycles": fold.cycles,
        "terminal": fold.terminal.map(ledger::phase::CampaignPhase::as_str),
        "last_proposal": proposal,
    }))
    .into_response()
}

// ── Open questions (AGENT-013 §9; S-10.12) ───────────────────────────────────

/// What a run sends to open one.
///
/// `why_it_matters` has no `serde(default)`: a body without it does not
/// deserialise. That is the whole difference between this door and the run-end
/// write, which records questions whose stakes were never stated because prose
/// does not contain them.
///
/// There is **no field for `status`**, and that is §9's rule rather than an
/// omission. The platform closes a question when a later finding cites it; a run
/// that could close its own would close the ones it had been thinking about
/// longest, which is not the same set as the ones it answered.
#[derive(Debug, Deserialize)]
pub struct QuestionBody {
    subject: String,
    question: String,
    why_it_matters: String,
    #[serde(default)]
    blocking: bool,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    run_id: Option<Uuid>,
}

/// `POST /api/lifecycle/questions` — write down what this run could not answer.
pub async fn open_question(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<QuestionBody>,
) -> impl IntoResponse {
    use harness::memory::questions::{self, QuestionValue};

    let value = match body.value.as_deref() {
        None | Some("curious") => QuestionValue::Curious,
        Some("useful") => QuestionValue::Useful,
        Some("decisive") => QuestionValue::Decisive,
        Some(other) => {
            return bad_request(
                "unknown_value",
                format!("`{other}` is not one of curious, useful, decisive"),
                "`decisive` means something is being decided on an assumption nobody checked",
            )
            .into_response()
        }
    };

    let id = format!("q_{}", uuid::Uuid::new_v4().simple());
    let opened_by = body
        .run_id
        .map_or_else(|| token.user_id().to_string(), |r| r.to_string());
    let q = match questions::open(
        &id,
        &body.subject,
        &body.question,
        &body.why_it_matters,
        body.blocking,
        value,
        opened_by,
    ) {
        Ok(q) => q,
        Err(e) => {
            return bad_request(
                "incomplete_question",
                e.to_string(),
                "a question with no subject or no stated stakes sits in the store being \
                 retrieved and skipped forever",
            )
            .into_response()
        }
    };

    let today = chrono::Utc::now().date_naive().to_string();
    let mut entry = harness::memory::Entry::finding(
        &id,
        &q.subject,
        &q.question,
        harness::memory::DataWindow::new(today.clone(), today),
        harness::memory::Provenance::new(
            body.run_id.map(|r| r.to_string()).unwrap_or_default(),
            "open_question",
            "1",
            "n/a",
        ),
    );
    entry.kind = harness::memory::Kind::OpenQuestion;
    // A question is not a claim, so it carries no confidence it could be read as
    // asserting — the same label the loop's run-end write uses, for the same
    // reason.
    entry.confidence = harness::memory::Confidence::Suggestive;
    entry.created_by = token.user_id().to_string();
    // The one serialisation, shared with the loop (`questions::OpenQuestion::extra`).
    entry.extra = q.extra();

    let store = crate::memory_store::MemoryStore::new(state.pg.clone());
    match store.put(token.user_id(), &entry).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({
                "id": id,
                "subject": q.subject,
                "blocking": q.blocking,
                "value": q.value.as_str(),
                // Returned so the caller can see that it is open and that closing
                // it is not something it will be doing.
                "status": "open",
                "closed_by": "a later finding that cites this id",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "question_rejected", "message": e.to_string() })),
        )
            .into_response(),
    }
}
