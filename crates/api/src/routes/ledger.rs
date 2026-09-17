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
use ledger::reservation::{Reserve, Reserved};
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
    };
    if let Err(e) = r.validate() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_reservation", "message": e.to_string() })),
        )
            .into_response();
    }

    let reservation_id = Uuid::new_v4();
    let res = sqlx::query(
        "INSERT INTO mlops.trial_reservation
             (reservation_id, tenant_id, run_id, tool, tool_version, kind,
              prereg_hash, arguments_hash, workflow)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
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
