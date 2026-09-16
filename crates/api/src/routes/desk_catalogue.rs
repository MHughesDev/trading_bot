//! The catalogue, as the desk needs it (AGENT-012 §9; Set S task S-6.19).
//!
//! `Catalogue.tsx` has been calling `/api/catalogue` since it was written and
//! nobody served it, so the page rendered an empty state that looked like a
//! platform with no tools.
//!
//! # Why the ring matters more than the listing
//!
//! The desk is a door. Its Tools panel pins tools for the next step, this page
//! offers "use this", and the composer picks a mode — so `lib/desk/rings.ts`
//! filters every one of those surfaces on `tool.ring`, refusing to offer a
//! trial-spending tool inside a Chat turn.
//!
//! That filter was reading a field **nothing on this side assigned**. A missing
//! ring is not a missing badge: `RING_INDEX[undefined]` is `undefined`, and
//! `undefined <= 0` is false, so every tool would have been hidden — or, had the
//! comparison gone the other way, every tool offered. `ToolSpec::ring()` derives
//! it from the risk and envelope the tool already declares, so it cannot
//! disagree with the gate a human sees.
//!
//! # Simulate exposure
//!
//! The page's real purpose: pick a profile and a goal and see the slice the
//! router would expose, with a reason against each tool. An exposure list
//! without reasons is a list nobody can argue with, and the router's mistakes
//! stay invisible.
//!
//! It calls `ToolRegistry::route_goal` and `expose_pinned` — the same two the
//! driver calls (`drive.rs`). Not a reimplementation of the routing rule: the
//! acceptance is that the simulation returns the slice the router *would*
//! choose, and two copies could only make that true by inspection.

use std::collections::BTreeMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::BearerToken, state::AppState};

/// One tool as the desk renders it — `ToolCard` in `frontend/src/api/desk.ts`.
fn card(spec: &mcp_server_lib::spec::ToolSpec) -> Value {
    let tiers: Vec<&str> = [
        (mcp_server_lib::ToolProfile::Mcp, "mcp"),
        (mcp_server_lib::ToolProfile::InternalAgent, "internal_agent"),
    ]
    .into_iter()
    .filter(|(p, _)| spec.exposed_to(*p))
    .map(|(_, name)| name)
    .collect();

    json!({
        "name": spec.name,
        "namespace": spec.namespace,
        "risk": spec.risk.as_str(),
        // A bool, because the desk's question is "may a Chat turn see this",
        // not "which kind of trial". The kind is the gate's business.
        "spends_trial": spec.spends_trial != mcp_server_lib::spec::TrialKind::None,
        "ring": spec.ring(),
        "output_budget_tokens": spec.output.summary_tokens,
        "notes": spec.notes_section,
        "tiers": tiers,
    })
}

/// GET /api/catalogue — every tool the platform declares.
pub async fn catalogue(_token: BearerToken) -> impl IntoResponse {
    // Active tools only. A deprecated spec is kept so old evidence resolves
    // (§2.7), and putting a tombstone on a page whose buttons say "use this"
    // would be offering a tool nobody may call.
    let tools: Vec<Value> = mcp_server_lib::spec::catalog()
        .iter()
        .filter(|s| s.status.is_active())
        .map(card)
        .collect();
    Json(json!({ "tools": tools }))
}

#[derive(Debug, Deserialize)]
pub struct ExposureQuery {
    pub profile: String,
    #[serde(default)]
    pub goal: String,
}

/// GET /api/catalogue/exposure — the slice the router would expose, and why.
pub async fn exposure(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(q): Query<ExposureQuery>,
) -> impl IntoResponse {
    let Some(profile) = state.profiles.get(&q.profile) else {
        let known: Vec<String> = state.profiles.ids().into_iter().map(Into::into).collect();
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "unknown_profile",
                "profile": q.profile,
                "known": known,
            })),
        )
            .into_response();
    };

    let registry = match crate::agent::bridge::registry() {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "registry_unavailable", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    let (namespaces, pinned) = registry.route_goal(&q.goal);
    let exposed = match registry.expose_pinned(&profile, &namespaces, &pinned) {
        Ok(e) => e,
        Err(e) => {
            // A `routing: none` profile whose catalogue outgrew its budget. That
            // is a real configuration error and the page should say so rather
            // than render a plausible-looking slice.
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "catalogue_exceeds_budget",
                    "message": e.to_string(),
                    "profile": q.profile,
                })),
            )
                .into_response();
        }
    };

    let budget = profile.tools.max_exposed_per_step;
    let mut reasons: BTreeMap<String, String> = BTreeMap::new();
    for name in &exposed.names {
        let reason = if registry.get(name).is_some_and(|t| t.core) {
            "core: the loop does not work without it, and it counts against the budget".to_string()
        } else if pinned.contains(name) {
            format!("your goal's search hit it directly (top {})", harness::registry::ToolRegistry::GOAL_HITS)
        } else if let Some(ns) = registry.get(name).map(|t| t.namespace.clone()) {
            format!("namespace `{ns}`, which the goal's hits selected")
        } else {
            "selected by the router".to_string()
        };
        reasons.insert(name.clone(), reason);
    }

    // The tools that did *not* make it, with the reason, because that is the half
    // a reader actually wants to argue with.
    for t in mcp_server_lib::spec::catalog() {
        if exposed.names.contains(t.name) || !t.status.is_active() {
            continue;
        }
        let reason = match registry.get(t.name) {
            None => "not offered to the internal-agent profile".to_string(),
            Some(def) if !namespaces.contains(&def.namespace) => format!(
                "namespace `{}` was not selected: the goal's hits chose {}",
                def.namespace,
                if namespaces.is_empty() {
                    "nothing".to_string()
                } else {
                    namespaces.join(", ")
                }
            ),
            Some(_) => format!(
                "its namespace was selected but the budget of {budget} filled first; core and \
                 pinned tools are placed ahead of namespace filling"
            ),
        };
        reasons.insert(t.name.to_string(), reason);
    }

    let tools: Vec<Value> = mcp_server_lib::spec::catalog()
        .iter()
        .filter(|s| exposed.names.contains(s.name))
        .map(card)
        .collect();

    Json(json!({
        "tools": tools,
        "reasons": reasons,
        "namespaces": exposed.namespaces,
        "pinned": pinned,
        "budget": budget,
        "profile": q.profile,
        "goal": q.goal,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every card carries a ring the desk can index.
    ///
    /// `lib/desk/rings.ts` does `RING_INDEX[tool.ring] <= RING_INDEX[maxRingFor(mode)]`.
    /// A missing or unknown ring makes the left side `undefined`, and
    /// `undefined <= 0` is `false` — so the tool silently vanishes from every
    /// surface rather than erroring anywhere a developer would look.
    #[test]
    fn every_card_has_a_ring_the_desk_indexes() {
        for spec in mcp_server_lib::spec::catalog() {
            let c = card(spec);
            let ring = c["ring"].as_str().unwrap_or_default();
            assert!(
                ["research", "platform_artifacts", "paper"].contains(&ring),
                "`{}` renders ring `{ring}`, which `RING_INDEX` has no entry for",
                spec.name
            );
        }
    }

    /// `spends_trial` is a boolean, because that is what the desk's predicate
    /// branches on. A `TrialKind` serialised as a string would be truthy for
    /// every value including "none", and a Chat turn would be offered the whole
    /// catalogue.
    #[test]
    fn spends_trial_is_a_boolean_and_not_a_kind() {
        let mut spending = 0;
        for spec in mcp_server_lib::spec::catalog() {
            let c = card(spec);
            assert!(
                c["spends_trial"].is_boolean(),
                "`{}` renders spends_trial as {}",
                spec.name,
                c["spends_trial"]
            );
            if c["spends_trial"] == json!(true) {
                spending += 1;
            }
        }
        assert!(
            spending > 0,
            "no tool spends a trial, so the desk's Chat filter is never exercised"
        );
    }

    /// A card says who may see it, so the page's "visible to …" line is never
    /// blank for a tool somebody can actually call.
    #[test]
    fn an_active_tool_is_visible_to_someone() {
        for spec in mcp_server_lib::spec::catalog() {
            if !spec.status.is_active() {
                continue;
            }
            let c = card(spec);
            let tiers = c["tiers"].as_array().expect("tiers is an array");
            assert!(
                !tiers.is_empty(),
                "`{}` is active and exposed to no profile, so it is in the catalogue and \
                 reachable from nowhere",
                spec.name
            );
        }
    }
}
