//! `POST /api/ledger/reservations` — the route the trial gate has been calling
//! since it was written (ADR-S10-10; resolves OQ-20).
//!
//! # What was broken
//!
//! `gate::ApiTrialRegistrar` posted to `/api/ledger/trials`. That route did not
//! exist anywhere in the repository, a 404 is an `Err`, and `CallGate::admit`
//! returns before the dispatch table is reached — so **every tool declaring a
//! trial was refused, on every path**, and the research loop could not run. The
//! gate's own adversarial tests all passed, correctly: they assert that a
//! registrar which refuses causes the call to be refused, and that is precisely
//! what happened.
//!
//! # Why it is a reservation and not a trial
//!
//! `mlops.trial`'s required columns are facts about **compute** — `config_hash`,
//! `dataset_id`, `code_hash`, `image_digest`, `seed_set` — and a tool call has
//! chosen none of them. Writing them at the door means inventing them. So this
//! records the question the gate is actually asking (*may this run spend, and
//! has it spent one*) in its own table, and `backtest::run` goes on registering
//! the trial where the facts exist. `trials_counted`, the exploration floor and
//! `N_eff` all read `mlops.trial` and are untouched.
//!
//! # The tenant is the session's, never the body's
//!
//! Same rule as every other write here: a caller that could name its tenant
//! could spend another tenant's budget. `run_id` *is* taken from the body,
//! because the MCP client knows which run it is serving and the platform cannot
//! derive it from a service token — and a wrong `run_id` misattributes a look
//! within one tenant rather than across tenants, which is a smaller error and a
//! visible one.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use backtest::gates::tickets::Ticket;
use ledger::reservation::{Reserve, Reserved};
use ledger::workflow_verdict::Record as WorkflowRecord;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// What the gate sends.
#[derive(Debug, Deserialize)]
pub struct ReserveBody {
    tool: String,
    tool_version: i32,
    kind: String,
    /// The call's arguments. Hashed here rather than stored: the audit envelope
    /// already holds them, and a second copy of a payload that may be large buys
    /// nothing a hash does not.
    #[serde(default)]
    arguments: Value,
    /// The run's sealed pre-registration (AGENT-013 §7.2).
    ///
    /// No `serde(default)`: a body without it does not deserialise. The gate
    /// refuses a run with none before it gets here, so a request that arrives
    /// without one is a *path* that has stopped carrying it — which is what
    /// happened until 2026-09-17, silently, because the registrar built a body
    /// that omitted the field its own request type carried.
    prereg_hash: String,
    #[serde(default)]
    run_id: Option<Uuid>,
    /// The stage skeleton the run is following (AGENT-013 §3; S-10.14).
    ///
    /// Optional: a front door with no agent run behind it follows none, and
    /// refusing the reservation for want of one would refuse the call.
    #[serde(default)]
    workflow: Option<String>,
    /// The battery this look belongs to (S-8.1; migration 0134).
    ///
    /// Optional, and from the body for the same reason `run_id` is: the caller
    /// running the battery is the only party that knows which looks belong to
    /// it, and the platform cannot derive it from a service token.
    #[serde(default)]
    family_id: Option<String>,
}

/// The mode of a run this tenant owns, or `None` when no such run exists (a run id
/// the tenant does not hold is not one whose mode it may learn).
///
/// # Errors
/// The read failed.
pub async fn run_mode(
    pg: &sqlx::PgPool,
    run_id: Uuid,
    user_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT mode FROM agent_runs WHERE run_id = $1 AND user_id = $2")
        .bind(run_id)
        .bind(user_id)
        .fetch_optional(pg)
        .await
        .map(Option::flatten)
}

/// The refusal for a reservation naming a Chat run, if it is one.
#[must_use]
pub fn chat_refusal(mode: Option<&str>) -> Option<Value> {
    (mode == Some("chat")).then(|| {
        json!({
            "error": "chat_turn_spends_no_trials",
            "message": "this run is a Chat turn, and a Chat turn spends no trials. Start a                         Research run (the offer card's Start action) to spend one",
        })
    })
}

/// `POST /api/ledger/reservations`
pub async fn reserve(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<ReserveBody>,
) -> impl IntoResponse {
    let arguments_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(
            serde_json::to_vec(&body.arguments).unwrap_or_default()
        ))
    );
    let r = Reserve {
        // The session's, never the body's.
        tenant_id: token.user_id().to_string(),
        run_id: body.run_id,
        tool: body.tool,
        tool_version: body.tool_version,
        kind: body.kind,
        prereg_hash: body.prereg_hash,
        arguments_hash,
        workflow: body.workflow,
        family_id: body.family_id,
    };
    if let Err(e) = r.validate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_reservation", "message": e.to_string() })),
        )
            .into_response();
    }

    // A Chat turn spends no trial, whoever asks (AGENT-013 §2.2; S-10.2). The gate
    // refuses at the door (`CallGate::for_chat_turn`) and the exposure filter hides
    // the tools, but both are properties of the *client*: a request that reaches this
    // route by another way -- a forged body naming a Chat run's id -- would otherwise
    // be granted. The ledger is the party that cannot be talked out of it. A run the
    // platform cannot read the mode of is refused too: not knowing is not "research".
    if let Some(run_id) = r.run_id {
        match run_mode(&state.pg, run_id, token.user_id()).await {
            Ok(mode) => {
                if let Some(refusal) = chat_refusal(mode.as_deref()) {
                    return (StatusCode::FORBIDDEN, Json(refusal)).into_response();
                }
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": "run_mode_unreadable",
                        "message": format!("the run's mode could not be read, so the reservation is refused: {e}"),
                    })),
                )
                    .into_response();
            }
        }
    }

    let reservation_id = Uuid::new_v4();
    let res = sqlx::query(
        "INSERT INTO mlops.trial_reservation
             (reservation_id, tenant_id, run_id, tool, tool_version, kind,
              prereg_hash, arguments_hash, workflow, family_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(reservation_id)
    .bind(&r.tenant_id)
    .bind(r.run_id)
    .bind(&r.tool)
    .bind(r.tool_version)
    .bind(&r.kind)
    .bind(&r.prereg_hash)
    .bind(&r.arguments_hash)
    .bind(&r.workflow)
    .bind(&r.family_id)
    .execute(&state.pg)
    .await;

    match res {
        Ok(_) => (
            StatusCode::CREATED,
            // `reservation_id`, not `ticket_id`. A reservation is permission that
            // was granted and a ticket is compute that ran; returning one under
            // the other's name would let a model cite a reservation as the trial
            // that produced a number.
            Json(json!(Reserved { reservation_id })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            // The gate treats any failure as a refusal, which is the right
            // direction: a look that could not be recorded must not happen.
            Json(json!({ "error": "reservation_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── S-8.4: the battery's conclusion, so a gate can demand it ─────────────────

/// What the workflow engine sends when a battery finishes.
#[derive(Debug, Deserialize)]
pub struct VerdictBody {
    /// The family the battery minted before its first step was admitted.
    family_id: String,
    workflow: String,
    verdict: String,
    because: String,
    /// What it was run over — the experiment id.
    subject: String,
    #[serde(default)]
    run_id: Option<Uuid>,
    /// Every step, as the engine rendered them. No `serde(default)`: a body
    /// without them does not deserialise, because a verdict with no steps is a
    /// claim and the gate that reads these exists to stop accepting claims.
    steps: Value,
    complete: bool,
}

/// `POST /api/ledger/workflow-verdicts`
///
/// Written by the engine, from a battery it ran. There is no path here for a
/// verdict a model asserts: the body carries the steps, and
/// [`WorkflowRecord::validate`] refuses one with none.
pub async fn record_verdict(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<VerdictBody>,
) -> impl IntoResponse {
    let r = WorkflowRecord {
        family_id: body.family_id,
        // The session's, never the body's — same rule as every other write here.
        tenant_id: token.user_id().to_string(),
        workflow: body.workflow,
        verdict: body.verdict,
        because: body.because,
        // Normalised, because the gate looks a ticket up by the experiment's
        // canonical id and the battery was handed whatever spelling the model
        // typed. A ticket filed under `A1B2...` that a gate looks for under
        // `a1b2...` is a battery that ran and cannot be found.
        subject: Uuid::parse_str(body.subject.trim())
            .map_or_else(|_| body.subject.trim().to_string(), |u| u.to_string()),
        run_id: body.run_id,
        steps: body.steps,
        complete: body.complete,
    };
    if let Err(e) = r.validate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_verdict", "message": e.to_string() })),
        )
            .into_response();
    }

    let res = sqlx::query(
        "INSERT INTO mlops.workflow_verdict
             (family_id, tenant_id, workflow, verdict, because, subject, run_id, steps, complete)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (family_id) DO NOTHING",
    )
    .bind(&r.family_id)
    .bind(&r.tenant_id)
    .bind(&r.workflow)
    .bind(&r.verdict)
    .bind(&r.because)
    .bind(&r.subject)
    .bind(r.run_id)
    .bind(&r.steps)
    .bind(r.complete)
    .execute(&state.pg)
    .await;

    match res {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({ "family_id": r.family_id, "verdict": r.verdict })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "verdict_not_recorded", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// Every battery verdict recorded for one subject, oldest first.
///
/// Oldest first because `backtest::gates::tickets::check` takes the **newest**
/// ticket per battery rather than the best one, and it does that positionally.
/// Handing it rows in the other order would silently invert that rule.
///
/// # Errors
/// Backend failures. A checkpoint treats an error as "no tickets", which
/// refuses — the only safe direction for a lookup that licenses a gate.
pub async fn tickets_for(
    pg: &sqlx::PgPool,
    tenant_id: &str,
    subject: &str,
) -> Result<Vec<Ticket>, sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT family_id, workflow, verdict
           FROM mlops.workflow_verdict
          WHERE tenant_id = $1 AND subject = $2
          ORDER BY created_at",
    )
    .bind(tenant_id)
    .bind(subject)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(family_id, workflow, verdict)| Ticket {
            family_id,
            workflow,
            verdict,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body the gate sends must deserialise, and must not deserialise
    /// without the registration.
    #[test]
    fn the_registration_is_required_at_the_boundary() {
        let ok = json!({
            "tool": "run_sweep",
            "tool_version": 1,
            "kind": "search",
            "arguments": { "instrument_id": "BTC-USD" },
            "prereg_hash": "sha256:abc"
        });
        serde_json::from_value::<ReserveBody>(ok).expect("a complete body");

        let missing = json!({
            "tool": "run_sweep",
            "tool_version": 1,
            "kind": "search",
            "arguments": {}
        });
        let err = serde_json::from_value::<ReserveBody>(missing)
            .expect_err("no default, so no deserialisation");
        assert!(err.to_string().contains("prereg_hash"), "{err}");
    }

    /// Two calls with the same arguments hash the same, and different arguments
    /// do not — otherwise the hash could not match a reservation to the compute
    /// it authorised.
    #[test]
    fn the_arguments_hash_distinguishes_calls() {
        let h = |v: Value| {
            format!(
                "sha256:{}",
                hex::encode(Sha256::digest(serde_json::to_vec(&v).unwrap_or_default()))
            )
        };
        assert_eq!(h(json!({ "a": 1 })), h(json!({ "a": 1 })));
        assert_ne!(h(json!({ "a": 1 })), h(json!({ "a": 2 })));
    }
}

#[cfg(test)]
mod chat_reservation_tests {
    use super::chat_refusal;

    /// S-10.2's adversarial clause, at the one place a forged request lands: only a
    /// Chat run is refused, and the reason names what to do instead.
    #[test]
    fn a_reservation_naming_a_chat_run_is_refused_and_a_research_run_is_not() {
        let refusal = chat_refusal(Some("chat")).expect("a chat run spends no trials");
        assert_eq!(refusal["error"], "chat_turn_spends_no_trials");
        assert!(refusal["message"]
            .as_str()
            .unwrap()
            .contains("Research run"));
        assert!(chat_refusal(Some("research")).is_none());
        // A run with no recorded mode (a front door with no agent behind it) is not
        // a Chat turn; the mode that cannot be *read* is refused by the handler.
        assert!(chat_refusal(None).is_none());
    }
}
