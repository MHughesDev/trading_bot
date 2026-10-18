//! Three more read-only `lifecycle` tools (AGENT-014 section 10; Set S task
//! S-11.14): the namespace goes from 9 to 12, `research` stays at 16.
//!
//! * `get_stage` - AGENT-013 section 8's own tool: where this run is in the
//!   lifecycle, its hypotheses, and what is waiting for a human.
//! * `get_charter` - the charter a continuous session's runs are chartered
//!   under, with its version and amendments, so a child reads its standing
//!   constraints from the platform rather than from its brief alone.
//! * `list_contradictions` - the pairs source 4.5 raises `diagnose` items from,
//!   resolved and not, so a run can see that two findings it is about to build on
//!   disagree.
//!
//! Every one is a read of a route that already refuses a write from an agent
//! token (`web:full`, which `NEVER_FOR_AGENTS` will not attach to a project-bound
//! session). A tool here cannot start, approve, pause or amend a session.
//!
//! `refine`, `compare_candidates` and `reproduce` are named in AGENT-013 section
//! 8 and are **not** built here: each needs a platform backend (a refinement
//! chain table, the comparison protocol over real measurements, a receipt
//! re-run) that is another task's, and a tool with no backend is the defect this
//! repository keeps finding. See ADR-P11-08.

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;

/// `get_stage` - the calling run's own position in the lifecycle.
pub async fn get_stage(api: &crate::ApiClient, _params: &Value) -> Value {
    match api.get("/api/research/sessions/current/stage").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_charter` - the charter, its version and its amendments.
pub async fn get_charter(api: &crate::ApiClient, params: &Value) -> Value {
    let id = match crate::tools::lifecycle::session_of(api, params).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    match api
        .get(&format!(
            "/api/research/sessions/{}",
            crate::tools::market::urlencode(&id)
        ))
        .await
    {
        Ok(v) => json!({
            "session_id": v.get("session_id"),
            "continuous": v.get("continuous"),
            "question": v.get("question"),
            "charter": v.get("charter"),
            "charter_version": v.get("charter_version"),
            "amendments": v.get("amendments"),
            "autonomy": v.get("autonomy"),
            "budget": v.get("budget"),
            "trials_per_day": v.get("trials_per_day"),
        }),
        Err(e) => e.to_tool_error(),
    }
}

/// `list_contradictions` - the unresolved pairs first.
pub async fn list_contradictions(api: &crate::ApiClient, _params: &Value) -> Value {
    match api.get("/api/research/contradictions").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// The catalogue entries for these three.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "get_stage",
            "lifecycle",
            Risk::Read,
            "Where this run is in the research lifecycle: its stage and workflow, the \
             hypotheses it has registered with their status, and how many proposals are \
             waiting on a human. Read it when you are unsure what the platform thinks you \
             have already committed to.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "get_charter",
            "lifecycle",
            Risk::Read,
            "The charter your session runs under: scope, objective, permitted workflows, \
             exclusions, standing constraints, its version and every amendment. Omit \
             `session_id`; the platform knows which session you are in.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "Omit this. The platform knows which session you are in."
                    }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "list_contradictions",
            "lifecycle",
            Risk::Read,
            "Pairs of findings the platform has recorded as disagreeing, unresolved first, \
             each with the question it raises. Check it before building on a finding: two \
             entries that conflict are both in your brief until somebody settles them.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_tools_are_reads_in_the_lifecycle_namespace() {
        let specs = specs();
        assert_eq!(specs.len(), 3);
        for s in &specs {
            assert_eq!(s.namespace, "lifecycle");
            assert!(matches!(s.risk, Risk::Read), "{} must not write", s.name);
        }
    }
}
