//! The human's side of a proposal (AGENT-013 §6.8, §7.5; Set S task S-10.9).
//!
//! # What this is
//!
//! `lifecycle.propose` creates an inert row and says so out loud. This is the
//! other half: a human reads the queue, decides, and **the registry write
//! happens at that moment, as that human**. Until this existed the approval had
//! nowhere to happen, so every proposal the platform has ever created is still
//! pending.
//!
//! # Three facts that are not one fact
//!
//! A row here answers three separate questions, and collapsing any two of them
//! is how an approval queue starts lying:
//!
//! * **decided** — a human said yes or no. `decided_by` and `decided_at`.
//! * **performed** — the platform then did something. `performed_at`.
//! * **what it did** — `effect`, an object per kind.
//!
//! An approval whose write fails is `approved` and not performed, and says so.
//! Before migration 0131 there was nowhere to record the difference, so an
//! approved-and-inert row read exactly like an approved-and-performed one.
//!
//! # Only a human decides
//!
//! The scope check is the mechanism. An agent session carries a service scope
//! and is refused here — which is the whole point of the proposal existing: if
//! an agent token could approve, the proposal would be a delay the agent adds to
//! its own writes rather than a door somebody else opens.
//!
//! # `Pending` is not permission
//!
//! `Decision::permits_write` returns true only on `Approved`. "Not yet rejected"
//! is how an approval queue becomes a waiting period.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use ledger::proposal::{Decision, ProposalKind};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// The scope a human web session carries.
///
/// Named here rather than matched inline so the one place that decides "is this
/// a person" is greppable. An agent's service session does not hold it.
const HUMAN_SCOPE: &str = "web:full";

#[derive(Debug, Deserialize)]
pub struct DecisionBody {
    /// `approved` or `rejected`. No default: a body that failed to say which
    /// must not become either.
    decision: String,
    /// Why. Recorded on the decision row, because "who approved this" without
    /// "on what basis" is half an audit trail.
    #[serde(default)]
    note: String,
}

fn bad(code: &str, message: String, fix: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": code, "message": message, "fix": fix })),
    )
}

/// What the platform did about an approved proposal.
///
/// `Performed` and `NotPerformed` are both honest outcomes of an approval, and
/// the second one is not an error: the human's decision stands either way, and
/// what changed is whether the platform could act on it.
#[derive(Debug, Clone, PartialEq)]
enum Effect {
    Performed(Value),
    NotPerformed(String),
}

/// `POST /api/lifecycle/proposals/{id}/decision` — a human decides.
pub async fn decide(
    State(state): State<AppState>,
    token: BearerToken,
    Path(proposal_id): Path<Uuid>,
    Json(body): Json<DecisionBody>,
) -> impl IntoResponse {
    if !token.permits(HUMAN_SCOPE) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "not_a_human_session",
                "message": "a proposal is decided by a person, from the desk",
                "fix": "an agent session cannot approve its own proposal. That is the whole \
                        reason the proposal exists: an approval an agent could grant is a \
                        delay it adds to its own writes, not a door somebody else opens.",
            })),
        )
            .into_response();
    }

    let decision = match body.decision.as_str() {
        "approved" => Decision::Approved,
        "rejected" => Decision::Rejected,
        other => {
            return bad(
                "unknown_decision",
                format!("`{other}` is not a decision"),
                "`approved` or `rejected`. `pending` is the absence of a decision and cannot \
                 be written as one.",
            )
            .into_response()
        }
    };

    // The row, and what it is about. Read before the update so a decision on a
    // proposal that is already decided is refused rather than overwriting who
    // decided it first.
    type Row = (String, String, Option<Uuid>, String, Value);
    let row: Option<Row> = match sqlx::query_as::<_, Row>(
        "SELECT kind, decision, subject_id, subject, payload
           FROM agent_lifecycle.proposal WHERE proposal_id = $1",
    )
    .bind(proposal_id)
    .fetch_optional(&state.pg)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "storage", "message": e.to_string() })),
            )
                .into_response()
        }
    };
    let Some((kind, current, subject_id, subject, payload)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown_proposal", "proposal_id": proposal_id })),
        )
            .into_response();
    };
    if current != "pending" {
        return bad(
            "already_decided",
            format!("this proposal was already {current}"),
            "a second decision would overwrite who made the first one, and that is the part \
             worth keeping",
        )
        .into_response();
    }

    // The kill switch, again, and at the moment that matters most. It was
    // checked when the proposal was created; a switch tripped since then is the
    // platform saying something is wrong *now*, and now is when the write would
    // happen. A read that fails answers tripped.
    if decision == Decision::Approved && crate::routes::lifecycle::kill_switch_tripped(&state).await
    {
        return bad(
            "kill_switch_tripped",
            "the kill switch is tripped".to_string(),
            "nothing reaches the registry while the platform has decided something is wrong. \
             The proposal stays pending; decide it after the switch is reset.",
        )
        .into_response();
    }

    let effect = if decision.permits_write() {
        perform(
            &state,
            &kind,
            subject_id,
            &payload,
            token.user_id().to_string(),
        )
        .await
    } else {
        // A rejection performs nothing, and that is not a failure to perform.
        Effect::NotPerformed("rejected".into())
    };

    let (performed_at, effect_json) = match (&effect, decision) {
        (Effect::Performed(v), Decision::Approved) => (true, Some(v.clone())),
        // An approval the platform could not act on is recorded as approved and
        // *not* performed. The human's decision stands; what changed is that
        // nothing happened, and a row that hid the difference would be the
        // failure `lifecycle.propose` already refuses to commit on the agent's
        // side.
        (Effect::NotPerformed(_), _) | (Effect::Performed(_), _) => (false, None),
    };

    let res = sqlx::query(
        "UPDATE agent_lifecycle.proposal
            SET decision = $2, decided_by = $3, decided_at = now(),
                performed_at = CASE WHEN $4 THEN now() ELSE NULL END,
                effect = $5
          WHERE proposal_id = $1 AND decision = 'pending'",
    )
    .bind(proposal_id)
    .bind(decision.as_str())
    .bind(token.user_id())
    .bind(performed_at)
    .bind(&effect_json)
    .execute(&state.pg)
    .await;
    if let Err(e) = res {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response();
    }

    // The decision log, with the **human** as the actor. §7.5's point: "who
    // approved this" is a recorded fact rather than something reconstructed from
    // timestamps, and the actor is the person rather than the run that asked.
    log_decision(&state, &token, proposal_id, &kind, &subject, decision).await;

    Json(json!({
        "proposal_id": proposal_id,
        "decision": decision.as_str(),
        "decided_by": token.user_id(),
        "note": body.note,
        "performed": performed_at,
        "effect": match &effect {
            Effect::Performed(v) => v.clone(),
            Effect::NotPerformed(why) => json!({ "performed": false, "why": why }),
        },
    }))
    .into_response()
}

/// Do the thing the approval authorises.
///
/// Every kind that the platform has no path for returns [`Effect::NotPerformed`]
/// with the reason, rather than being reported as done. The alternative — an
/// approval that silently changes nothing — is exactly what `lifecycle.propose`
/// refuses to do on the agent's side, and a human is owed the same honesty.
async fn perform(
    state: &AppState,
    kind: &str,
    subject_id: Option<Uuid>,
    payload: &Value,
    actor: String,
) -> Effect {
    let Some(subject_id) = subject_id else {
        // A proposal created before migration 0131 has no target, and there is
        // no honest value to invent for one. Named rather than guessed: a UUID
        // nobody chose, acted on under a human's authority, is the worst
        // possible repair.
        return Effect::NotPerformed(
            "this proposal predates the target column and names no row to act on".to_string(),
        );
    };
    match kind {
        // Caller-scoped (Set E 1.4): the approver arms their own automation or
        // nothing. `actor` is the approving human's user id; a subject owned by
        // anyone else reads as "no such automation", the same as a missing one.
        "deploy_paper" => match Uuid::parse_str(&actor) {
            Ok(owner) => {
                match crate::routes::automations::arm_owned(state, owner, subject_id).await {
                    Ok(v) => Effect::Performed(v),
                    Err(why) => Effect::NotPerformed(why.reason()),
                }
            }
            Err(_) => Effect::NotPerformed(
                "the approving actor is not a user id, so no automation can be armed for it"
                    .to_string(),
            ),
        },
        // Ring 1. `promote_strategy` still has no writer, and the reason is
        // not an oversight: a strategy's *promoted set* has no writer outside
        // the agent tools S-10.9 is meant to remove, and which registries count
        // as that set is the decision this task is waiting on. Named, so the
        // approval says what it did not do rather than reporting success.
        "promote_strategy" => Effect::NotPerformed(
            "the strategy registry has no approval-side writer yet; the decision is recorded and nothing was promoted".to_string(),
        ),
        "change_limits" => write_limit(state, subject_id, payload, actor).await,
        _ => Effect::NotPerformed("unknown proposal kind".into()),
    }
}

/// Write the limit this proposal carried, as the approving human (S-10.9).
///
/// # Three fields the proposal does not get to state
///
/// `strategy_id` is `subject_id` — the row the approval acts on, fixed when the
/// proposal was made. `declared_at` is *now*, because the limit takes effect at
/// the decision and not when it was asked for. And `declared_by` is the human,
/// never the run: a proposal that could state who declared a limit would be an
/// agent writing somebody else's name on its own change.
///
/// # Re-checked here, though it was checked at creation
///
/// Not belt-and-braces. The row may have been written by an older version of the
/// creation path, or by hand, and this is the moment it becomes a live control
/// limit. The refusal is a `NotPerformed` with the reason rather than a panic,
/// because the decision itself is already made and worth recording.
async fn write_limit(
    state: &AppState,
    // Already unwrapped by `perform`, which refuses a proposal with no target
    // before reaching any kind. Taking `Uuid` here rather than `Option<Uuid>`
    // keeps that refusal in one place instead of two that could disagree.
    strategy_id: Uuid,
    payload: &Value,
    actor: String,
) -> Effect {
    use monitor::limits::ControlLimit;

    if let Err(why) = crate::routes::lifecycle::check_proposed_limit(payload) {
        return Effect::NotPerformed(format!("the proposed limit is not writable: {why}"));
    }
    let proposed: crate::routes::lifecycle::ProposedLimit =
        match serde_json::from_value(payload.clone()) {
            Ok(v) => v,
            Err(e) => {
                return Effect::NotPerformed(format!("the proposed limit is not a limit: {e}"))
            }
        };
    let (family, severity, method) = match (
        proposed.family.parse::<monitor::family::Family>(),
        proposed
            .severity
            .as_deref()
            .unwrap_or("breach")
            .parse::<monitor::limits::LimitSeverity>(),
        crate::routes::lifecycle::method_from(&proposed.method),
    ) {
        (Ok(f), Ok(s), Ok(m)) => (f, s, m),
        _ => {
            return Effect::NotPerformed(
                "the proposed limit names a family, severity or method this platform does not \
                 have"
                    .to_string(),
            )
        }
    };

    let limit = ControlLimit {
        strategy_id: strategy_id.to_string(),
        metric: proposed.metric,
        family,
        severity,
        version: proposed.version,
        lower: proposed.lower,
        upper: proposed.upper,
        method,
        declared_from: proposed.declared_from,
        // The moment of the decision, not the moment of the request.
        declared_at: chrono::Utc::now(),
        // The human. This is the whole point of the proposal existing.
        declared_by: actor,
    };
    match crate::routes::monitor::limit_store(state)
        .declare_limit(&limit)
        .await
    {
        Ok(()) => Effect::Performed(json!({
            "strategy_id": limit.strategy_id,
            "metric": limit.metric,
            "version": limit.version,
            "declared_by": limit.declared_by,
        })),
        Err(e) => Effect::NotPerformed(format!("the limit store refused the write: {e}")),
    }
}

/// Record the approval in `mlops.decision`, as the human.
///
/// Failure is logged rather than raised: the proposal row is already written and
/// the effect has already happened, and unwinding a registry write because a log
/// line failed would be worse than a missing log line. It is logged loudly
/// because a decision the ledger does not hold is one no later analysis can
/// attribute.
async fn log_decision(
    state: &AppState,
    token: &BearerToken,
    proposal_id: Uuid,
    kind: &str,
    subject: &str,
    decision: Decision,
) {
    let led = ledger::pg::PgTrialLedger::new(state.pg.clone());
    let tenant = token.user_id().to_string();
    let ctx = ledger::DispatchContext {
        actor_kind: ledger::ActorKind::Human,
        ..ledger::DispatchContext::human(&tenant, token.user_id().to_string(), f64::MIN_POSITIVE)
    };
    let d = ledger::Decision {
        kind: match decision {
            Decision::Approved => ledger::DecisionKind::Promote,
            // A rejection is a decision, and recording it is the half that makes
            // an approval rate mean anything.
            Decision::Rejected | Decision::Pending => ledger::DecisionKind::Reject,
        },
        context_hash: format!("proposal:{proposal_id}"),
        context_uri: Some(format!("proposal:{proposal_id}")),
        // Approve and reject were the options; the human took one.
        candidate_set: vec![json!("approved"), json!("rejected")],
        chosen: json!(decision.as_str()),
        // A human's choice is not drawn from a policy, so there is no
        // probability to record — but `mlops.decision` requires one from every
        // non-legacy caller. 1.0 is true here in the only sense that matters:
        // given this person and this queue, this is what they did.
        propensity: Some(1.0),
        exploration_flag: false,
        decision_tier: ledger::DecisionTier::Rule,
        rationale: Some(format!("{kind} on {subject}")),
    };
    if let Err(e) = led
        .log_decision_async(&ctx.with_experiment(format!("proposal:{proposal_id}")), &d)
        .await
    {
        tracing::error!(
            %proposal_id, error = %e,
            "the approval happened and the decision log did not record it"
        );
    }
}

/// `GET /api/lifecycle/proposals` — the queue a human decides from.
pub async fn list(State(state): State<AppState>, _token: BearerToken) -> impl IntoResponse {
    let rows: Result<
        Vec<(
            Uuid,
            String,
            i16,
            String,
            String,
            String,
            String,
            Option<Uuid>,
        )>,
        _,
    > = sqlx::query_as(
        "SELECT proposal_id, kind, ring, envelope, dossier_ref, subject, rationale, subject_id
               FROM agent_lifecycle.proposal
              WHERE decision = 'pending'
              ORDER BY created_at",
    )
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => Json(json!({
            "pending": rows
                .into_iter()
                .map(|(id, kind, ring, envelope, dossier, subject, rationale, subject_id)| {
                    json!({
                        "proposal_id": id,
                        "kind": kind,
                        "ring": ring,
                        "envelope": envelope,
                        "dossier_ref": dossier,
                        "subject": subject,
                        "subject_id": subject_id,
                        "rationale": rationale,
                        // Whether approving it would do anything. Shown in the
                        // queue rather than discovered after deciding.
                        //
                        // Derived from `performable()` rather than matched by
                        // hand here. It was a hand-written `matches!` naming one
                        // kind, beside a function that answers the same
                        // question - so `change_limits` gaining a writer would
                        // have left the queue telling people that approving it
                        // changes nothing, which is the kind of stale warning
                        // that teaches a reader to ignore warnings.
                        "actionable": subject_id.is_some()
                            && performable().iter().any(|k| k.as_str() == kind),
                    })
                })
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// The kinds an approval can currently perform.
///
/// Public so the invariant in `crates/invariants` can assert that every kind
/// `ProposalKind::ALL` names is either performable or listed as not, rather than
/// silently falling into a `_` arm.
#[must_use]
pub fn performable() -> &'static [ProposalKind] {
    // `ChangeLimits` joined `DeployPaper` when the proposal gained a payload
    // (migration `0137`): a limit change could not be performed before, not
    // because nothing could write a limit, but because the proposal carried
    // none of the eight fields one has.
    //
    // `PromoteStrategy` is still absent, and the reason is a decision rather
    // than a missing function: which registries count as the *promoted set* is
    // what S-10.9's last piece is waiting on, so there is no shape to validate
    // a payload against and no table to write it to.
    &[ProposalKind::DeployPaper, ProposalKind::ChangeLimits]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A limit change is performable now, and a promotion is not — and the
    /// difference is a decision nobody has made rather than a function nobody
    /// wrote.
    #[test]
    fn the_performable_set_is_the_one_with_a_writer() {
        let names: Vec<&str> = performable().iter().map(|k| k.as_str()).collect();
        assert!(names.contains(&"change_limits"));
        assert!(names.contains(&"deploy_paper"));
        assert!(
            !names.contains(&"promote_strategy"),
            "if a promoted-set writer now exists, `perform` must call it rather than \
             reporting `not_performed` by name"
        );
    }

    /// Every kind that carries a payload must be performable, and the other way
    /// round for the one that is not.
    ///
    /// The pairing is the property: a kind that demands a payload at creation
    /// and cannot perform one at approval would refuse a caller for not sending
    /// something nothing would read.
    #[test]
    fn a_kind_that_demands_a_payload_can_perform_one() {
        for kind in ProposalKind::ALL {
            if kind.carries_a_payload() {
                assert!(
                    performable().contains(kind),
                    "`{}` demands a payload at creation and cannot perform one",
                    kind.as_str()
                );
            }
        }
    }

    /// The three fields the proposal does not get to state.
    ///
    /// `declared_by` is the human, `declared_at` is the decision, and
    /// `strategy_id` is the target fixed when the proposal was made. A payload
    /// that could carry any of them would be an agent writing somebody else's
    /// name on its own change — so `ProposedLimit` has no field for them, and
    /// this asserts that by round-tripping a payload that tries.
    #[test]
    fn a_proposal_cannot_state_who_approved_it() {
        let sneaky = json!({
            "metric": "max_drawdown",
            "family": "behaviour",
            "version": 1,
            "upper": 0.2,
            "method": "paper_ewma_3sigma",
            "declared_from": "bt_1",
            "declared_by": "somebody-else",
            "declared_at": "2020-01-01T00:00:00Z",
            "strategy_id": "another-strategy"
        });
        // It still parses — unknown fields are ignored, which is the right
        // behaviour for a payload written by an older client — and the three
        // fields simply have nowhere to go.
        let parsed: crate::routes::lifecycle::ProposedLimit =
            serde_json::from_value(sneaky).expect("the known fields parse");
        assert_eq!(parsed.metric, "max_drawdown");
        assert_eq!(parsed.declared_from, "bt_1");
    }

    /// The creation check names the field, because a refusal a caller cannot
    /// act on costs a whole round trip.
    #[test]
    fn an_unwritable_limit_is_refused_with_the_reason() {
        use crate::routes::lifecycle::check_proposed_limit;
        let complete = json!({
            "metric": "max_drawdown",
            "family": "behaviour",
            "version": 1,
            "upper": 0.2,
            "method": "paper_ewma_3sigma",
            "declared_from": "bt_1"
        });
        check_proposed_limit(&complete).expect("a complete limit");

        // A baseline with no provenance is a number somebody typed.
        let mut orphan = complete.clone();
        orphan["declared_from"] = json!("   ");
        let why = check_proposed_limit(&orphan).expect_err("no provenance");
        assert!(why.contains("declared_from"), "{why}");

        // A method the platform does not have would be admitted at creation and
        // fail at the decision, which is the ordering this check exists to
        // prevent.
        let mut bad_method = complete.clone();
        bad_method["method"] = json!("vibes");
        let why = check_proposed_limit(&bad_method).expect_err("unknown method");
        assert!(why.contains("vibes"), "{why}");

        let mut bad_family = complete;
        bad_family["family"] = json!("not_a_family");
        assert!(check_proposed_limit(&bad_family).is_err());
    }

    /// The creation check and the approval read the same method vocabulary.
    ///
    /// Two copies would let a proposal be admitted under one and fail under the
    /// other, at the decision — after a human had already approved it.
    #[test]
    fn creation_and_approval_agree_on_what_a_method_is() {
        use crate::routes::lifecycle::{check_proposed_limit, method_from};
        for name in [
            "paper_ewma_3sigma",
            "backtest_3sigma",
            "venue_rate_ceiling",
            "risk_gate_mirror",
            "declared_absolute",
            "mc_p95_drawdown",
            "cusum",
        ] {
            let limit = json!({
                "metric": "m",
                "family": "behaviour",
                "version": 1,
                "upper": 1.0,
                "method": name,
                "declared_from": "bt_1"
            });
            assert!(
                check_proposed_limit(&limit).is_ok(),
                "{name} refused at creation"
            );
            assert!(method_from(name).is_ok(), "{name} refused at approval");
        }
    }

    /// Only an approval performs anything, and only an explicit one.
    #[test]
    fn pending_is_not_permission() {
        assert!(Decision::Approved.permits_write());
        assert!(!Decision::Rejected.permits_write());
        assert!(!Decision::Pending.permits_write());
    }

    /// Every kind is either performable or has a stated reason it is not. A `_`
    /// arm that swallowed a new kind would report an approval as done.
    #[test]
    fn every_kind_is_performable_or_says_why_not() {
        for kind in ProposalKind::ALL {
            let performable = performable().contains(kind);
            // The match in `perform` names all three by hand; this asserts the
            // two lists agree about which of them does something.
            //
            // `ChangeLimits` joined the set when the proposal gained a payload
            // (migration `0137`). `PromoteStrategy` is still out, and stays out
            // until the promoted set has a writer.
            let expected = matches!(kind, ProposalKind::DeployPaper | ProposalKind::ChangeLimits);
            assert_eq!(
                performable,
                expected,
                "`{}` disagrees with `perform`'s arms",
                kind.as_str()
            );
        }
    }

    /// An approval that could not be performed is still an approval, and the
    /// row says both halves.
    #[test]
    fn an_unperformable_approval_is_recorded_as_approved_and_not_performed() {
        let e = Effect::NotPerformed("no writer yet".into());
        assert!(matches!(e, Effect::NotPerformed(_)));
        // The shape the response carries, so a caller reading `performed: false`
        // is also told why.
        let rendered = match &e {
            Effect::Performed(v) => v.clone(),
            Effect::NotPerformed(why) => json!({ "performed": false, "why": why }),
        };
        assert_eq!(rendered["performed"], false);
        assert_eq!(rendered["why"], "no writer yet");
    }
}
