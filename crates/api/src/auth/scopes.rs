//! Token scopes (AGENT-001 §6, ADR-0025, DA-09).
//!
//! The agent runs with bash and code in a sandbox, so it can compute anything on any
//! data it can reach and can ignore any instruction it is given. What it *cannot* do
//! is mint itself authority. Every capability the platform offers is behind a scope
//! carried by the session token, checked by middleware.
//!
//! The important part of this list is what is missing from [`RESEARCH_SCOPES`]:
//! holdout reads, order placement, automation arming, model-alias promotion and skill
//! admission. There is no code path that grants those to a project-bound session, and
//! a database trigger refuses them even if one appeared (migration 0038).

use std::collections::HashSet;

/// Read data through the Data API, subject to the project's cutoff.
pub const DATA_READ: &str = "research:data.read";
/// Compute features (DATA-006).
pub const FEATURES: &str = "research:features";
/// Submit jobs.
pub const JOBS: &str = "research:jobs";
/// Read and write artifacts in the project's scope.
pub const ARTIFACTS: &str = "research:artifacts";
/// Read and write the project's research memory.
pub const MEMORY: &str = "research:memory";
/// Submit a `final_report` for validation.
pub const REPORT: &str = "research:report";
/// Call the model through the platform proxy.
pub const LLM_PROXY: &str = "llm:proxy";

/// Trip, scale, halt, resume or reset the live monitor (COMP-007 §5, §6).
///
/// Not a research scope, and never granted to an agent. The monitor is a service
/// the agent may **query** and never **skip**: `check_control_limits` reads these
/// tables, and there is no agent-callable halt, scale or reset at any risk class.
/// An agent that could reset the switch could trade through its own halt.
pub const MONITOR_CONTROL: &str = "monitor.control";

/// Read raw text through the Data API (DATA-005 §5, DA-09; Set P P-3.5).
///
/// Held by the reader role only. Deliberately **not** in [`RESEARCH_SCOPES`]: text
/// is the one series that can steer an agent, so no ordinary research session may
/// hold it. `routes/data.rs::get_text` checks it before it reads.
pub const TEXT_READER: &str = "research:text.reader";

/// Everything a research session may hold.
///
/// Deliberately small and complete: a reader should be able to tell what the agent
/// can do by reading one array.
pub const RESEARCH_SCOPES: &[&str] = &[
    DATA_READ, FEATURES, JOBS, ARTIFACTS, MEMORY, REPORT, LLM_PROXY,
];

/// Scopes that must never be attached to a project-bound session.
///
/// Mirrored by the trigger in migration 0038. Two layers, because this one is easy
/// to bypass by writing the row directly and that one is easy to forget when adding
/// a new capability — each catches the other's blind spot.
pub const NEVER_FOR_AGENTS: &[&str] = &[
    "data.holdout",
    "orders.place",
    "orders.cancel",
    "automations.arm",
    "models.promote",
    "skills.admit",
    // The answer key to the agent's own evaluation (AGENT-004 §2). An agent that
    // can read the planted mechanism does not have to find it, and every number on
    // the scorecard stops meaning what it says.
    "evals.truth",
    // COMP-007 §5/§8.6. The monitor is the last line before money and the agent is
    // one of the things it watches; a watched party that can reset the alarm is not
    // being watched.
    MONITOR_CONTROL,
    "web:full",
];

/// A web login's scope: full user rights. Scopes narrow service sessions only.
pub const WEB_FULL: &str = "web:full";

/// Whether a set of scopes permits `required`.
///
/// `web:full` satisfies everything, which is what keeps the human UI working
/// unchanged; a service session must hold the exact scope.
pub fn permits(held: &[String], required: &str) -> bool {
    held.iter().any(|s| s == WEB_FULL || s == required)
}

/// Whether a token may act on the live monitor (COMP-007 §6).
///
/// A thin wrapper over [`permits`], which is the point: the authority question is
/// asked once, by name, in one place, so a new monitor route cannot be added
/// without answering it. Every monitor POST calls this and nothing else.
pub fn permits_monitor_control(held: &[String]) -> bool {
    permits(held, MONITOR_CONTROL)
}

/// Validates a scope set destined for a project-bound agent session.
pub fn validate_agent_scopes(scopes: &[String]) -> Result<(), String> {
    let forbidden: HashSet<&str> = NEVER_FOR_AGENTS.iter().copied().collect();
    for scope in scopes {
        if forbidden.contains(scope.as_str()) {
            return Err(format!(
                "scope {scope:?} may never be granted to an agent session"
            ));
        }
        if !RESEARCH_SCOPES.contains(&scope.as_str()) {
            return Err(format!(
                "scope {scope:?} is not a research scope; the agent's authority is \
                 exactly {RESEARCH_SCOPES:?}"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_research_scope_set_is_accepted() {
        assert!(validate_agent_scopes(&owned(RESEARCH_SCOPES)).is_ok());
    }

    #[test]
    fn every_forbidden_scope_is_refused_individually() {
        // Checked one at a time so that adding a new forbidden scope without a test
        // is impossible: this iterates the list itself.
        for scope in NEVER_FOR_AGENTS {
            let outcome = validate_agent_scopes(&owned(&[scope]));
            assert!(outcome.is_err(), "{scope} must be refused for an agent");
        }
    }

    #[test]
    fn the_two_lists_do_not_overlap() {
        // If a scope were in both, the agent could hold something the design says it
        // must never hold, and each list would look correct on its own.
        for scope in RESEARCH_SCOPES {
            assert!(
                !NEVER_FOR_AGENTS.contains(scope),
                "{scope} is both granted and forbidden"
            );
        }
    }

    #[test]
    fn the_holdout_is_not_reachable_by_any_research_scope() {
        // The single most important property in the file: nothing in the agent's
        // vocabulary reads past the cutoff (D-12, DA-04).
        assert!(!RESEARCH_SCOPES.contains(&"data.holdout"));
        assert!(!permits(&owned(RESEARCH_SCOPES), "data.holdout"));
    }

    #[test]
    fn trading_is_not_reachable_by_any_research_scope() {
        for capability in ["orders.place", "orders.cancel", "automations.arm"] {
            assert!(
                !permits(&owned(RESEARCH_SCOPES), capability),
                "{capability} must be unreachable: the agent has research authority only"
            );
        }
    }

    #[test]
    fn an_unknown_scope_is_refused_rather_than_ignored() {
        // Silently dropping an unrecognised scope would make a typo look like a
        // granted capability in one direction and a missing one in the other.
        assert!(validate_agent_scopes(&owned(&["research:whatever"])).is_err());
    }

    #[test]
    fn no_research_scope_reaches_the_monitor() {
        // COMP-007 §8.6, from the authority side. Asserted over the whole research
        // vocabulary rather than against one scope, so a scope added to
        // RESEARCH_SCOPES cannot quietly become a way to reset the kill switch.
        assert!(!permits_monitor_control(&owned(RESEARCH_SCOPES)));
        for scope in RESEARCH_SCOPES {
            assert!(
                !permits_monitor_control(&owned(&[scope])),
                "{scope} must not reach the monitor"
            );
        }
    }

    #[test]
    fn an_agent_session_cannot_be_minted_with_monitor_control() {
        assert!(validate_agent_scopes(&owned(&[MONITOR_CONTROL])).is_err());
        assert!(NEVER_FOR_AGENTS.contains(&MONITOR_CONTROL));
    }

    #[test]
    fn an_empty_service_token_reaches_nothing_on_the_monitor() {
        // What `POST /auth/service-tokens` actually mints: a session with no
        // scopes at all. The failure this guards is an `is_empty()` check
        // somewhere reading as "unrestricted".
        assert!(!permits_monitor_control(&[]));
    }

    #[test]
    fn web_sessions_keep_full_rights() {
        let web = owned(&[WEB_FULL]);
        assert!(permits(&web, DATA_READ));
        assert!(permits(&web, "anything.at.all"));
    }

    #[test]
    fn a_service_session_holds_only_what_it_was_given() {
        let held = owned(&[DATA_READ, JOBS]);
        assert!(permits(&held, DATA_READ));
        assert!(permits(&held, JOBS));
        assert!(!permits(&held, ARTIFACTS));
        assert!(!permits(&held, "data.holdout"));
    }
}
