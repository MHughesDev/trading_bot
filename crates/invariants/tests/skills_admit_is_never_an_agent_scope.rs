//! `skills.admit` is never an agent scope, and no route that admits is reachable by one
//! (Set R-a RA-1.3, SK-03; ADR-0028).
//!
//! ```bash
//! cargo test -j 2 -p invariants --test skills_admit_is_never_an_agent_scope
//! ```
//!
//! The agent *proposes*; the platform admits. That is only true if three things hold at
//! once, and each is asserted from the side that would break it:
//!
//! 1. **The scope table.** `skills.admit` is in `NEVER_FOR_AGENTS`, is not in
//!    `RESEARCH_SCOPES`, and `validate_agent_scopes` refuses to mint it, alone or among
//!    legitimate ones. The same is true of `knowledge.write`, the platform's authority
//!    over knowledge measurements (move outcomes, study scores, a random arm).
//! 2. **The database.** Migration 0038's trigger names the scope too, so a row written
//!    around the Rust still cannot carry it.
//! 3. **The routes.** Every `pub async fn` in `routes/skills.rs` is either on a short,
//!    closed list of handlers a non-admin may call (read-only, or guarded by a research
//!    scope) or begins with `require_admit`. And no MCP tool in the catalogue posts to a
//!    route outside that closed list.
//!
//! The mutation each guards: add `"skills.admit"` to `RESEARCH_SCOPES` and (1) fails;
//! delete `require_admit(&token)` from one handler and (3) fails by name.

use std::fs;
use std::path::{Path, PathBuf};

use api::auth::scopes::{permits, validate_agent_scopes, NEVER_FOR_AGENTS, RESEARCH_SCOPES};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn no_agent_scope_set_can_carry_skills_admit() {
    assert!(NEVER_FOR_AGENTS.contains(&"skills.admit"));
    assert!(!RESEARCH_SCOPES.contains(&"skills.admit"));
    assert!(validate_agent_scopes(&owned(&["skills.admit"])).is_err());
    let mut with_legit = owned(RESEARCH_SCOPES);
    with_legit.push("skills.admit".into());
    assert!(validate_agent_scopes(&with_legit).is_err(), "not even among legitimate scopes");
    // An agent's whole vocabulary does not permit it.
    assert!(!permits(&owned(RESEARCH_SCOPES), "skills.admit"));
    // Only a web login does, which is how a person admits.
    assert!(permits(&owned(&["web:full"]), "skills.admit"));
}

#[test]
fn the_platform_write_scope_for_knowledge_is_not_an_agent_scope_either() {
    assert!(!RESEARCH_SCOPES.contains(&"knowledge.write"));
    assert!(validate_agent_scopes(&owned(&["knowledge.write"])).is_err());
    assert!(!permits(&owned(RESEARCH_SCOPES), "knowledge.write"));
}

#[test]
fn the_database_trigger_names_the_scope_too() {
    let sql = read("migrations/0038_token_scopes.sql");
    assert!(sql.contains("'skills.admit'"), "0038's trigger no longer lists skills.admit");
}

/// Handlers a caller without `skills.admit` may reach, each with the guard that stands in
/// its place. Closed: adding a handler to `routes/skills.rs` that is not here and does not
/// begin with `require_admit` fails the test below.
const NON_ADMIN_HANDLERS: [(&str, &str); 8] = [
    ("glossary", "read"),
    ("search", "read"),
    ("get_skill", "read, own-scope only"),
    ("propose", "research:artifacts"),
    ("list_proposals", "own proposals only"),
    ("vote", "research:memory, exposed sessions only"),
    ("notices", "read"),
    ("bill_of_materials", "read"),
];

fn handler_names(src: &str) -> Vec<(String, String)> {
    let prod = src.split("#[cfg(test)]").next().unwrap();
    prod.split("pub async fn ")
        .skip(1)
        .map(|chunk| {
            let name: String = chunk.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let body_at = chunk.find('{').unwrap_or(0);
            (name, chunk[body_at..].chars().take(500).collect::<String>())
        })
        .collect()
}

#[test]
fn every_skills_handler_is_admin_guarded_or_on_the_closed_list() {
    let src = read("crates/api/src/routes/skills.rs");
    assert!(src.contains("pub const ADMIT: &str = \"skills.admit\";"));
    let handlers = handler_names(&src);
    assert!(handlers.len() >= 16, "found only {} handlers; the scan has drifted", handlers.len());
    let mut admin = 0;
    for (name, head) in &handlers {
        if NON_ADMIN_HANDLERS.iter().any(|(n, _)| n == name) {
            continue;
        }
        assert!(
            head.contains("require_admit(&token)") || head.contains("retire_with(state, token"),
            "`{name}` changes what the registry admits but does not begin with require_admit"
        );
        admin += 1;
    }
    assert!(admin >= 8, "only {admin} admin handlers found");
}

#[test]
fn every_mounted_skills_route_points_at_a_known_handler() {
    // A route mounted to a handler this scan has not classified would be an unguarded door
    // the previous test cannot see.
    let router = read("crates/api/src/routes/mod.rs");
    let handlers: Vec<String> = handler_names(&read("crates/api/src/routes/skills.rs"))
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let mut mounted = 0;
    for line in router.lines().filter(|l| l.contains("\"/api/skills/")) {
        for part in line.split("(skills::").skip(1) {
            let name: String = part.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            assert!(handlers.contains(&name), "route line {line:?} names an unknown handler {name}");
            mounted += 1;
        }
    }
    assert!(mounted >= 16, "only {mounted} skills routes found in the router");
}

#[test]
fn no_mcp_tool_posts_to_an_admin_route() {
    let defs = mcp_server_lib::tool_definitions_for(mcp_server_lib::ToolProfile::Mcp);
    let names: Vec<String> = defs
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    // There is no tool for any admin action, by name.
    for forbidden in [
        "admit_skill", "promote_skill", "quarantine_skill", "deprecate_skill", "verify_skill",
        "record_skill_condition", "run_curator",
    ] {
        assert!(!names.iter().any(|n| n == forbidden), "{forbidden} is an agent tool");
    }
    // And the one file that talks to the registry names none of the admin paths.
    let tool_src = read("crates/mcp-server/src/tools/skills_registry.rs");
    let prod = tool_src.split("#[cfg(test)]").next().unwrap();
    for path in ["/admit", "/verify", "/steps", "/pairs", "/quarantine", "/deprecate", "/promote", "/conditions", "/curator"] {
        assert!(!prod.contains(&format!("{path}\"")), "skills tool reaches {path}");
    }
}
