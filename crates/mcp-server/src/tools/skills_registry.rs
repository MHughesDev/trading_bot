//! The agent's side of the skill registry (Set R-a RA-3.1, RA-4.1, RA-4.2; ADR-0028).
//!
//! Four thin clients of `/api/skills/*`. The agent can search for a skill by intent,
//! **propose** one, check what became of a proposal and vote on a skill it was shown.
//! It cannot admit, promote, quarantine or verify: those routes need `skills.admit`,
//! which no agent session can hold, and there is no tool for them here on purpose.
//! `no_skills_tool_reaches_an_admit_route` pins that over this file's source.
//!
//! The existing `create_skill` / `list_skills` / `run_skill` in `research_tools.rs` are
//! the Python-transformation library (migration 0060). These are *procedures*: a
//! `SKILL.md` an agent reads. The namespace is shared because both answer "what do I
//! already know how to do", and a model looking in `skills` should find both.

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;
use crate::tools::market::urlencode;
use crate::ApiClient;

/// `propose_skill` — submit a procedure for verification.
pub async fn propose_skill(api: &ApiClient, params: &Value) -> Value {
    for field in ["project_id", "name", "category", "reason", "source_ref", "files"] {
        if params.get(field).is_none() {
            return json!({ "error": "missing_field", "field": field });
        }
    }
    let files = match params.get("files").and_then(Value::as_object) {
        Some(f) if f.contains_key("SKILL.md") => f,
        _ => {
            return json!({
                "error": "bad_field",
                "field": "files",
                "message": "files is an object of path to text and must contain `SKILL.md`",
            })
        }
    };
    let body = json!({
        "project_id": params["project_id"],
        "name": params["name"],
        "category": params["category"],
        "reason": params["reason"],
        "source_ref": params["source_ref"],
        "sources": params.get("sources").cloned().unwrap_or_else(|| json!([])),
        "files": files,
    });
    match api.post("/api/skills/proposals", body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `search_skills` — find a skill by what it is for.
pub async fn search_skills(api: &ApiClient, params: &Value) -> Value {
    let Some(q) = params
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({ "error": "missing_field", "field": "query" });
    };
    let mut url = format!("/api/skills/search?q={}", urlencode(q));
    if let Some(p) = params.get("project_id").and_then(Value::as_str) {
        url.push_str(&format!("&project={}", urlencode(p)));
    }
    match api.get(&url).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_skill_proposal` — what became of the proposals this caller made.
pub async fn get_skill_proposal(api: &ApiClient, params: &Value) -> Value {
    let url = match params.get("status").and_then(Value::as_str) {
        Some(s) => format!("/api/skills/proposals?status={}", urlencode(s)),
        None => "/api/skills/proposals".to_string(),
    };
    match api.get(&url).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `rate_skill` — a helpfulness vote on a skill this session was shown.
pub async fn rate_skill(api: &ApiClient, params: &Value) -> Value {
    let (Some(skill), Some(session), Some(vote)) = (
        params.get("skill_id").and_then(Value::as_str),
        params.get("session_id").and_then(Value::as_str),
        params.get("vote").and_then(Value::as_i64),
    ) else {
        return json!({ "error": "missing_field", "fields": ["skill_id", "session_id", "vote"] });
    };
    match api
        .post(
            &format!("/api/skills/{}/vote", urlencode(skill)),
            json!({ "session_id": session, "vote": vote }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "propose_skill",
            "skills",
            Risk::Write,
            "PROPOSE a reusable procedure for verification; cite the run it came from (`source_ref`). You cannot admit it. \n             A near-duplicate becomes a patch, so search first.",
            json!({
                "type": "object",
                "required": ["project_id", "name", "category", "reason", "source_ref", "files"],
                "additionalProperties": false,
                "properties": {
                    "project_id": { "type": "string", "description": "source project" },
                    "name": { "type": "string", "description": "lowercase-with-hyphens" },
                    "category": { "type": "string", "description": "e.g. research" },
                    "reason": {
                        "type": "string",
                        "enum": ["T1", "T2", "T3", "T4"],
                        "description": "T1 repeat, T2 hard-won, T3 pitfall, T4 asked"
                    },
                    "source_ref": { "type": "string", "description": "e.g. run:<id>" },
                    "sources": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "evidence kinds, and finding:<id> per finding"
                    },
                    "files": {
                        "type": "object",
                        "description": "path to text; needs SKILL.md",
                        "properties": { "SKILL.md": { "type": "string", "description": "frontmatter, then steps" } },
                        "additionalProperties": { "type": "string" }
                    }
                }
            }),
        )
        .internal()
        .max_calls_per_run(5),
        ToolSpec::new(
            "search_skills",
            "skills",
            Risk::Read,
            "FIND a procedure by what it is for, over the full text of every skill you can see. The \
             response says whether the search was lexical or dense; read a hit's SKILL.md before using it.",
            json!({
                "type": "object",
                "required": ["query"],
                "additionalProperties": false,
                "properties": {
                    "query": { "type": "string", "description": "what you are trying to do, in your own words" },
                    "project_id": { "type": "string", "description": "include this project's skills" }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "get_skill_proposal",
            "skills",
            Risk::Read,
            "What became of your skill proposals: submitted, rejected with the step that failed, or admitted.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string", "enum": ["submitted", "verifying", "rejected", "admitted"] }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "rate_skill",
            "skills",
            Risk::Write,
            "VOTE on whether a skill you were shown helped: 1 helped, -1 hurt, 0 clears your vote. Only a \
             skill this session was exposed to can be rated.",
            json!({
                "type": "object",
                "required": ["skill_id", "session_id", "vote"],
                "additionalProperties": false,
                "properties": {
                    "skill_id": { "type": "string" },
                    "session_id": { "type": "string", "description": "this run's id" },
                    "vote": { "type": "integer", "enum": [-1, 0, 1] }
                }
            }),
        )
        .internal()
        .max_calls_per_run(20),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No tool in this file posts to a route that changes what is admitted. The admin
    /// routes are the ones `routes/skills.rs` guards with `require_admit`; this reads
    /// the source for any path to them.
    #[test]
    fn no_skills_tool_reaches_an_admit_route() {
        let src = include_str!("skills_registry.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "/admit", "/verify", "/steps", "/pairs", "/quarantine", "/deprecate", "/promote", "/conditions",
        ] {
            assert!(
                !prod.contains(&format!("\"{forbidden}")) && !prod.contains(&format!("{forbidden}\"")) && !prod.contains(&format!("{forbidden}`")),
                "a skills tool mentions {forbidden}, an admin route"
            );
        }
        // And the only POSTs are the two this file declares.
        assert_eq!(prod.matches(".post(").count(), 2);
    }

    #[test]
    fn every_schema_spells_out_its_fields() {
        for s in specs() {
            let props = s.input_schema.get("properties").and_then(Value::as_object);
            assert!(props.is_some_and(|p| !p.is_empty() || s.name == "get_skill_proposal"), "{} has no declared properties", s.name);
        }
    }

    #[test]
    fn four_tools_in_the_skills_namespace_beside_the_library() {
        let names: Vec<&str> = specs().iter().map(|s| s.name).collect();
        assert_eq!(names, ["propose_skill", "search_skills", "get_skill_proposal", "rate_skill"]);
        assert!(specs().iter().all(|s| s.namespace == "skills"));
    }
}
