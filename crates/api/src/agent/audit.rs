//! The API layer's audit writer (SPEC §15, checklist 2.18, ADR-P2-21,
//! ADR-P2-34, AT-64).
//!
//! One type, two methods, and the order they are called in is the whole
//! mechanism: [`AuditTrail::pre`] runs **before** the call reaches the platform's
//! authorisation, [`AuditTrail::post`] after it comes back. A request that is
//! refused therefore leaves a row saying it was made, which is the only way
//! "the agent never tried" and "the agent tried and was stopped" can be told
//! apart later.
//!
//! # Who calls it
//!
//! Only [`crate::agent::routes::open_audit_record`] and
//! [`crate::agent::routes::close_audit_record`] — the two platform endpoints
//! that `mcp_server_lib::AuditSink` talks to. Every front door, internal or
//! external, reaches the trail through them, so there is exactly one writer and
//! it is on the side of the wire the caller does not control.
//!
//! That matters more than it looks. The principal is derived **here**, from the
//! `sessions` row the bearer token resolves to — not from anything the caller
//! sent. An agent has no field to write "acting for" into, and the external MCP
//! process cannot present itself as the internal agent, because the two hold
//! differently-labelled tokens and the label is read from the database.

use ledger::audit::{AuditRequest, AuditVerdict, PreAudit, Principal};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// The label prefix the agent manager mints its run-scoped tokens under.
///
/// It is how the platform tells its own agent runs from the external MCP front
/// door without either of them saying which they are.
const AGENT_RUN_LABEL: &str = "agent-run-";

/// Who the platform decides a token's holder is.
///
/// A pure function of the authenticated session row, so the one interesting
/// property — that nothing here comes from the request body — is testable
/// without a database.
#[must_use]
pub fn principal_for(user_id: Uuid, kind: &str, label: Option<&str>) -> Principal {
    let tenant_id = user_id.to_string();
    let (actor_kind, actor_id, on_behalf_of) = match (kind, label) {
        // A person at a browser acts for themselves.
        ("web", _) => ("user", tenant_id.clone(), None),
        // The platform's own agent runs. The run id is in the label the manager
        // minted, so the agent does not get to name itself.
        (_, Some(l)) if l.starts_with(AGENT_RUN_LABEL) => (
            "agent",
            l[AGENT_RUN_LABEL.len()..].to_string(),
            Some(tenant_id.clone()),
        ),
        // Everything else holding a service token is an external client: the MCP
        // front door, or anything the user minted a token for. Identified by the
        // label they chose at mint time — never by the token itself.
        (_, label) => (
            "mcp_client",
            label.unwrap_or("service").to_string(),
            Some(tenant_id.clone()),
        ),
    };
    Principal {
        tenant_id,
        actor_kind: actor_kind.to_string(),
        actor_id,
        on_behalf_of,
    }
}

/// Writes the two halves of an audit record for one principal.
pub struct AuditTrail {
    ledger: ledger::pg::PgTrialLedger,
    principal: Principal,
}

impl AuditTrail {
    /// The trail for the session a bearer token resolved to (§8).
    ///
    /// The `kind` and `label` are read from `sessions`, so the attribution is
    /// the platform's reading of who the caller is, not the caller's claim.
    pub async fn for_session(pg: PgPool, token: &str, user_id: Uuid) -> Self {
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT kind, label FROM sessions WHERE token = $1")
                .bind(token)
                .fetch_optional(&pg)
                .await
                .ok()
                .flatten();
        let (kind, label) = row.unwrap_or_else(|| ("service".to_string(), None));
        Self {
            ledger: ledger::pg::PgTrialLedger::new(pg),
            principal: principal_for(user_id, &kind, label.as_deref()),
        }
    }

    /// The principal this trail attributes its rows to.
    #[must_use]
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Record the request, before anyone has decided anything about it.
    ///
    /// Returns `None` when the trail could not be written — and the caller's
    /// only correct response is to refuse the action. That reverses the earlier
    /// fail-open stance (ADR-P2-34): the audit store is the platform store, so
    /// "the trail is unreachable but the work can proceed" describes a state
    /// this system does not have, and a look nobody counted is the one thing the
    /// platform claims it cannot take. `dispatch_tool` enforces the refusal.
    pub async fn pre(&self, action: &str, arguments: &Value) -> Option<(AuditRequest, PreAudit)> {
        let req = AuditRequest::new(action, self.principal.clone(), arguments.clone());
        match self.ledger.record_audit_pre_async(&req).await {
            Ok(pre) => Some((req, pre)),
            Err(e) => {
                tracing::error!(
                    action,
                    envelope = req.envelope.as_str(),
                    error = %e,
                    "the audit trail's pre-record failed; the dispatch that would have followed is refused"
                );
                None
            }
        }
    }

    /// Reopen a record this trail's tenant already opened, by id.
    ///
    /// The action, envelope and principal come back from the stored row rather
    /// than from the closing caller, so a client cannot close a record as though
    /// it had asked for something else.
    pub async fn reopen(&self, audit_id: Uuid) -> Option<(AuditRequest, PreAudit)> {
        match self
            .ledger
            .read_audit_pre_async(&self.principal.tenant_id, audit_id)
            .await
        {
            Ok(found) => found,
            Err(e) => {
                tracing::error!(%audit_id, error = %e, "could not read back the audit pre-record");
                None
            }
        }
    }

    /// Record what the platform did about it.
    pub async fn post(&self, opened: Option<(AuditRequest, PreAudit)>, verdict: AuditVerdict) {
        let Some((req, pre)) = opened else { return };
        if let Err(e) = self
            .ledger
            .record_audit_post_async(&req, pre, verdict, None)
            .await
        {
            tracing::error!(
                action = %req.action,
                verdict = verdict.as_str(),
                error = %e,
                "the audit trail's post-record failed"
            );
        }
    }
}

/// The verdict a dispatched call's result implies.
///
/// `denied` is read from the platform's own refusal shape rather than guessed
/// from prose: the API answers a refused scope with an `error` object carrying
/// a `code`, and the two codes that mean "not allowed" are named here. Anything
/// else that errored is `failed` — the action was permitted and did not work,
/// which is a different fact about the same call.
#[must_use]
pub fn verdict_of(raw: &Value) -> AuditVerdict {
    let Some(error) = raw.get("error").filter(|e| !e.is_null()) else {
        return AuditVerdict::Executed;
    };
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or_default();
    match code {
        "forbidden" | "unauthorized" | "scope_denied" | "policy_denied" => AuditVerdict::Denied,
        "pending_approval" | "awaiting_approval" => AuditVerdict::PendingApproval,
        _ => AuditVerdict::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_clean_result_is_an_executed_action() {
        assert_eq!(verdict_of(&json!({ "ok": true })), AuditVerdict::Executed);
        // `error: null` is not an error — the bug that made three successful
        // training runs read as failures (see `bridge::execute`).
        assert_eq!(
            verdict_of(&json!({ "error": null, "run_id": "r" })),
            AuditVerdict::Executed
        );
    }

    #[test]
    fn a_refusal_is_recorded_as_a_denial_and_a_fault_is_not() {
        for code in ["forbidden", "unauthorized", "scope_denied", "policy_denied"] {
            assert_eq!(
                verdict_of(&json!({ "error": { "code": code } })),
                AuditVerdict::Denied,
                "{code}"
            );
        }
        assert_eq!(
            verdict_of(&json!({ "error": { "code": "pending_approval" } })),
            AuditVerdict::PendingApproval
        );
        // A permitted action that broke is `failed`, not `denied`: they are
        // different facts and the trail is read to tell them apart.
        assert_eq!(
            verdict_of(&json!({ "error": { "code": "clickhouse_query_failed" } })),
            AuditVerdict::Failed
        );
        assert_eq!(
            verdict_of(&json!({ "error": "boom" })),
            AuditVerdict::Failed
        );
    }

    /// The attribution is read off the session, and the caller has no field to
    /// write it into. The external front door in particular cannot present
    /// itself as the platform's own agent.
    #[test]
    fn the_principal_is_derived_by_the_platform_not_declared_by_the_caller() {
        let user = Uuid::new_v4();
        let run = Uuid::new_v4();

        let agent = principal_for(user, "service", Some(&format!("agent-run-{run}")));
        assert_eq!(agent.actor_kind, "agent");
        assert_eq!(agent.actor_id, run.to_string());
        assert_eq!(
            agent.on_behalf_of.as_deref(),
            Some(user.to_string().as_str())
        );
        assert_eq!(agent.tenant_id, user.to_string());

        // The external MCP front door. Distinct actor_kind, and its actor_id is
        // the label the user minted the token under — never the token.
        let external = principal_for(user, "service", Some("claude-desktop"));
        assert_eq!(external.actor_kind, "mcp_client");
        assert_eq!(external.actor_id, "claude-desktop");
        assert_eq!(
            external.on_behalf_of.as_deref(),
            Some(user.to_string().as_str())
        );

        // An unlabelled service token is still an external client, not an agent.
        assert_eq!(
            principal_for(user, "service", None).actor_kind,
            "mcp_client"
        );

        // A person at a browser acts for themselves.
        let human = principal_for(user, "web", None);
        assert_eq!(human.actor_kind, "user");
        assert_eq!(human.on_behalf_of, None);
    }

    /// The only way to be recorded as the internal agent is to hold a token the
    /// manager minted, because the prefix is matched against the session's own
    /// label. A client that *calls itself* `agent-run-…` never reaches here:
    /// this function is handed the database's label, not the request's.
    #[test]
    fn an_external_label_that_imitates_an_agent_run_is_still_read_from_the_session() {
        let user = Uuid::new_v4();
        // Same string, but it arrived as the session label, which only the
        // platform writes at mint time.
        let p = principal_for(user, "service", Some("agent-run-not-a-uuid"));
        assert_eq!(p.actor_kind, "agent");
        assert_eq!(p.actor_id, "not-a-uuid");
        // …and the tenant is still the token's owner, which is the field that
        // decides what the row can ever be read alongside.
        assert_eq!(p.tenant_id, user.to_string());
    }

    // A lazy pool still needs a runtime to exist in.
    #[tokio::test]
    async fn a_session_the_database_cannot_answer_for_is_treated_as_external() {
        let user = Uuid::new_v4();
        let trail = AuditTrail::for_session(
            // A pool that is never connected: the lookup fails, and the fallback
            // must be the *less* privileged reading, not "agent".
            PgPool::connect_lazy("postgres://unused/unused").expect("lazy pool"),
            "some-token",
            user,
        )
        .await;
        assert_eq!(trail.principal().actor_kind, "mcp_client");
        assert_eq!(trail.principal().tenant_id, user.to_string());
    }
}
