//! The API layer's audit writer (SPEC §15, checklist 2.18, ADR-P2-21, AT-64).
//!
//! One type, two methods, and the order they are called in is the whole
//! mechanism: [`AuditTrail::pre`] runs **before** the call reaches the platform's
//! authorisation, [`AuditTrail::post`] after it comes back. A request that is
//! refused therefore leaves a row saying it was made, which is the only way
//! "the agent never tried" and "the agent tried and was stopped" can be told
//! apart later.
//!
//! It lives on the API side rather than in the agent harness deliberately. The
//! harness is the thing being audited; a trail it writes is a trail an exception
//! path can skip. `bridge::execute` is the one function every internal-agent
//! tool call goes through, and it cannot dispatch without passing here first —
//! a static test (AT-64) asserts the `pre` write precedes the dispatch in the
//! source, because "we always call it first" is otherwise a convention.

use ledger::audit::{AuditRequest, AuditVerdict, PreAudit, Principal};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// Writes the two halves of an audit record for one principal.
pub struct AuditTrail {
    ledger: ledger::pg::PgTrialLedger,
    principal: Principal,
}

impl AuditTrail {
    /// An agent acting for a user (§8).
    ///
    /// `on_behalf_of` comes from the session the API authenticated, never from
    /// anything the agent sent. That is what makes the attribution
    /// non-spoofable: the agent has no field to write it into.
    #[must_use]
    pub fn for_agent(pg: PgPool, user_id: Uuid, session_id: Uuid) -> Self {
        Self {
            ledger: ledger::pg::PgTrialLedger::new(pg),
            principal: Principal {
                tenant_id: user_id.to_string(),
                actor_kind: "agent".to_string(),
                actor_id: session_id.to_string(),
                on_behalf_of: Some(user_id.to_string()),
            },
        }
    }

    /// Record the request, before anyone has decided anything about it.
    ///
    /// Returns `None` when the trail could not be written. The caller proceeds:
    /// refusing to run because the audit database is unreachable would turn an
    /// observability outage into a platform outage, and the miss is logged
    /// loudly rather than swallowed. A `post` without its `pre` is refused by
    /// the table, so the pair stays consistent either way.
    pub async fn pre(&self, action: &str, arguments: &Value) -> Option<(AuditRequest, PreAudit)> {
        let req = AuditRequest::new(action, self.principal.clone(), arguments.clone());
        match self.ledger.record_audit_pre_async(&req).await {
            Ok(pre) => Some((req, pre)),
            Err(e) => {
                tracing::error!(
                    action,
                    envelope = req.envelope.as_str(),
                    error = %e,
                    "the audit trail's pre-record failed; a denial of this action would leave no evidence"
                );
                None
            }
        }
    }

    /// Record what the platform did about it.
    pub async fn post(&self, opened: Option<(AuditRequest, PreAudit)>, verdict: AuditVerdict) {
        let Some((req, pre)) = opened else { return };
        if let Err(e) = self.ledger.record_audit_post_async(&req, pre, verdict, None).await {
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
        assert_eq!(verdict_of(&json!({ "error": null, "run_id": "r" })), AuditVerdict::Executed);
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
        assert_eq!(verdict_of(&json!({ "error": "boom" })), AuditVerdict::Failed);
    }

    // A lazy pool still needs a runtime to exist in.
    #[tokio::test]
    async fn the_principal_is_stamped_by_the_api_not_the_agent() {
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let trail = AuditTrail::for_agent(
            // A pool that is never connected: this test only reads the principal.
            PgPool::connect_lazy("postgres://unused/unused").expect("lazy pool"),
            user,
            session,
        );
        assert_eq!(trail.principal.actor_kind, "agent");
        assert_eq!(trail.principal.actor_id, session.to_string());
        assert_eq!(trail.principal.on_behalf_of.as_deref(), Some(user.to_string().as_str()));
        assert_eq!(trail.principal.tenant_id, user.to_string());
    }
}
