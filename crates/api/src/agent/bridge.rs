//! The tool bridge: the platform catalogue as a harness registry, and back again.
//!
//! Two directions, and both are narrow on purpose.
//!
//! **Out:** every tool in `ToolProfile::InternalAgent` becomes a
//! [`harness::ToolDef`] carrying the namespace and risk from
//! `mcp_server_lib::taxonomy`. Nothing is re-described here — a second copy of 56
//! descriptions would drift from the first within a month, and the one that drifted
//! would be the one the model reads.
//!
//! **Back:** a call that survived the validation ladder and the policy engine is
//! dispatched through `mcp_server_lib::dispatch_tool` — the identical code path the
//! external MCP front door uses. Tool behaviour cannot differ between the two,
//! because there is only one of it.
//!
//! # What this deliberately does not do
//!
//! It does not decide what the agent may call. Exposure is the registry's job and
//! authorisation is the policy's; this module's opinion about `arm_automation` is
//! irrelevant, because the agent's service token does not carry `automations.arm`
//! and the platform refuses it regardless (D-10). Defence here would be defence
//! inside the thing being defended against.

use serde_json::{Map, Value};

use harness::registry::{Risk, ToolDef, ToolRegistry};
use mcp_server_lib::{dispatch_tool, taxonomy, tool_definitions_for, McpContext, ToolProfile};

/// Tools the harness answers itself, so the bridge must not also register them.
///
/// `search_tools` reads the registry and `finish_task` ends the loop; neither
/// touches platform state, and a platform-side copy would be a second
/// implementation of a decision that has to be made in exactly one place.
const HARNESS_OWNED: &[&str] = &[
    harness::registry::SEARCH_TOOLS,
    harness::registry::FINISH_TASK,
];

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("tool {name:?} is in the agent profile but carries no taxonomy entry; classify it in crates/mcp-server/src/taxonomy.rs before it can reach a model")]
    Unclassified { name: String },
    #[error("the tool catalogue is malformed: {0}")]
    Malformed(String),
    #[error(transparent)]
    Registry(#[from] harness::registry::RegistryError),
}

fn risk_of(r: taxonomy::Risk) -> Risk {
    match r {
        taxonomy::Risk::Read => Risk::Read,
        taxonomy::Risk::Write => Risk::Write,
        taxonomy::Risk::Destructive => Risk::Destructive,
        taxonomy::Risk::Outbound => Risk::Outbound,
    }
}

/// Builds the harness registry from the platform catalogue.
///
/// Starts from [`ToolRegistry::with_core`], so the escape hatch the validation
/// ladder advertises and the typed termination the loop requires are both present
/// before a single platform tool is added.
pub fn registry() -> Result<ToolRegistry, BridgeError> {
    build(ToolRegistry::with_core())
}

/// [`registry`] plus the agent's own filesystem.
///
/// Separate because a harness with no workspace mounted must not advertise tools that
/// would fail on every call.
pub fn registry_with_workspace() -> Result<ToolRegistry, BridgeError> {
    build(ToolRegistry::with_workspace())
}

fn build(mut reg: ToolRegistry) -> Result<ToolRegistry, BridgeError> {
    let defs = tool_definitions_for(ToolProfile::InternalAgent);
    let arr = defs
        .as_array()
        .ok_or_else(|| BridgeError::Malformed("tool definitions are not an array".into()))?;

    for t in arr {
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError::Malformed("a tool definition has no name".into()))?;
        if HARNESS_OWNED.contains(&name) {
            continue;
        }
        let (namespace, risk) =
            taxonomy::classify(name).ok_or_else(|| BridgeError::Unclassified {
                name: name.to_string(),
            })?;
        reg.register(ToolDef {
            name: name.to_string(),
            namespace: namespace.to_string(),
            description: t
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            risk: risk_of(risk),
            // Core is about what the *loop* cannot run without, not about what is
            // useful. Every platform tool is routed like any other; a tool marked
            // core would occupy a slot in the exposure budget on every step of every
            // task forever.
            core: false,
            // Reads can be replayed after a crash without changing anything; writes
            // and destruction cannot. Derived rather than declared per tool, because
            // a per-tool flag would be a second place for this to be wrong.
            idempotent: matches!(risk, taxonomy::Risk::Read),
            input_schema: t
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
        })?;
    }
    Ok(reg)
}

/// The provenance source for a tool's output.
///
/// Everything in today's catalogue reads this platform's own stores, so everything
/// is `platform.*` and classifies as [`harness::Provenance::ToolInternal`]. The
/// moment a collector tool lands — Reddit text, a scraped filing, a third-party MCP
/// server's descriptions — its output must **not** be given a `platform.` prefix, and
/// `provenance::classify` will then tag it untrusted and the context manager will
/// wrap it in a data block.
///
/// The default in `classify` is untrusted, so forgetting to add a new collector here
/// fails safe.
#[must_use]
pub fn source_for(namespace: &str) -> String {
    format!("platform.{namespace}")
}

/// What a dispatched tool returned.
pub struct Executed {
    pub text: String,
    pub is_error: bool,
    pub raw: Value,
}

/// Dispatches one validated, gated call.
///
/// The audit trail's `pre` record is written **here, before the dispatch**
/// (SPEC §15, ADR-P2-21, AT-64). The dispatch is where the call meets the
/// platform's authorisation, so a request refused by it has already been
/// recorded as having been made — which is the only way a denial leaves any
/// evidence at all. `post` follows with what the platform decided.
///
/// `trail` is `None` only for callers with no principal to attribute to (the
/// external MCP front door authenticates separately); an internal agent session
/// always has one.
pub async fn execute(
    ctx: &McpContext,
    name: &str,
    arguments: &Map<String, Value>,
    trail: Option<&crate::agent::audit::AuditTrail>,
) -> Executed {
    let params = Value::Object(arguments.clone());
    let opened = match trail {
        Some(t) => t.pre(name, &params).await,
        None => None,
    };
    let raw = dispatch_tool(ctx, name, &params, None).await;
    if let Some(t) = trail {
        t.post(opened, crate::agent::audit::verdict_of(&raw)).await;
    }
    // `error: null` is NOT an error. `is_some()` on an Option<&Value> is true for a
    // JSON null, so any tool that reports its error slot explicitly — the natural way
    // to write a uniform result shape — had every success classified as a failure.
    //
    // Found in a trace: `train_model` returned `{"ok": true, "error": null, "run_id":
    // ...}` after a training run that genuinely succeeded, the agent was told it had
    // failed, retried it twice more, and reported to the user that training failed.
    // Three real models were trained while the agent believed none had been.
    let is_error = raw.get("error").is_some_and(|e| !e.is_null());
    let text = match raw.get("error").filter(|e| !e.is_null()) {
        // A failing tool is information the model can act on, so the message travels
        // as text rather than as a status code it cannot read.
        Some(e) => format!("ERROR: {}", compact(e)),
        None => compact(&raw),
    };
    Executed {
        text,
        is_error,
        raw,
    }
}

fn compact(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::registry::{FINISH_TASK, RECORD_FINDING, SEARCH_TOOLS};

    /// The check that makes an unclassified tool impossible to ship. `stamp` already
    /// defaults an unknown tool to destructive, which is the safe direction — but
    /// silently shipping a misclassified tool is not the same as refusing to build
    /// the registry at all.
    #[test]
    fn every_agent_tool_crosses_the_bridge_classified() {
        let reg = registry().expect("every tool in the agent profile is classified");
        assert!(reg.len() > 20, "the catalogue should be substantial");
        for name in reg.names() {
            // Core harness tools are answered inside the loop and never cross the
            // bridge, so the platform taxonomy has nothing to say about them.
            if name == SEARCH_TOOLS || name == FINISH_TASK || name == RECORD_FINDING {
                continue;
            }
            assert!(
                taxonomy::classify(name).is_some(),
                "{name} reached the registry unclassified"
            );
        }
    }

    /// The agent's folder reaches the model, and is routed away from the platform.
    ///
    /// Both halves matter: if the tools are missing the agent has no memory of its own
    /// work, and if `fs` were dispatched over HTTP like every other namespace the
    /// platform would answer "no such tool" for all four.
    #[test]
    fn the_workspace_registry_offers_the_fs_tools_in_their_own_namespace() {
        let reg = registry_with_workspace().unwrap();
        for name in harness::registry::FS_TOOLS {
            let def = reg
                .get(name)
                .unwrap_or_else(|| panic!("{name} did not reach the model"));
            assert_eq!(def.namespace, "fs");
            assert!(
                super::super::workspace_tools::is_workspace_tool(&def.namespace),
                "{name} would be dispatched to the platform instead of the workspace"
            );
        }
        // And the plain registry still does not, so a harness with no workspace
        // mounted cannot advertise them.
        let plain = registry().unwrap();
        assert!(plain.get("write_file").is_none());
    }

    /// The loop depends on both of these existing, and the bridge must not shadow
    /// them with platform copies that would be dispatched over HTTP instead of
    /// answered in the harness.
    #[test]
    fn the_core_tools_survive_the_bridge_exactly_once() {
        let reg = registry().unwrap();
        assert!(reg.get(SEARCH_TOOLS).is_some());
        assert!(reg.get(FINISH_TASK).is_some());
        assert_eq!(
            reg.names().iter().filter(|n| **n == SEARCH_TOOLS).count(),
            1
        );
    }

    #[test]
    fn risk_survives_the_crossing_unchanged() {
        let reg = registry().unwrap();
        assert_eq!(reg.get("get_bars").unwrap().risk, Risk::Read);
        assert_eq!(reg.get("create_strategy").unwrap().risk, Risk::Write);
        assert_eq!(reg.get("choose_null").unwrap().risk, Risk::Destructive);
    }

    /// Routing, against the real catalogue, for the query that got it wrong.
    ///
    /// Scoring counted how many query words appeared anywhere in a tool's name,
    /// namespace or description. "run a backtest on the strategy and report the
    /// results" is nine words, four of which — a, on, the, and — match essentially
    /// every tool, so the score was mostly noise and an alphabetical tie-break
    /// decided the rest: `get_authoring_guide` was routed in, ahead of anything that
    /// could run anything.
    ///
    /// Note what this test does NOT assert. `create_backtest` is deliberately absent
    /// from the agent's allowlist — the charter is explicit that experiments are the
    /// only path to a backtest — so the right outcome is that the EXPERIMENT path is
    /// offered, not that a withheld tool appears.
    #[test]
    fn running_a_backtest_routes_to_the_sanctioned_path() {
        let reg = registry().unwrap();
        assert!(
            reg.get("create_backtest").is_none(),
            "raw backtests are withheld from the agent on purpose; if that changed,              this test and the charter both need revisiting"
        );
        let hits: Vec<&str> = reg
            .search("run a backtest on the strategy and report the results", 4)
            .into_iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            hits.contains(&"run_sweep") || hits.contains(&"create_experiment"),
            "the step needs a tool that RUNS something; got {hits:?}"
        );
        assert!(
            !hits.contains(&"get_authoring_guide"),
            "a generic protocol guide must not outrank the tools for the job; got {hits:?}"
        );
    }

    /// `error: null` is not an error.
    ///
    /// `Option::is_some` is true for a JSON null, so a tool reporting its error slot
    /// explicitly — the natural way to write a uniform result shape — had every
    /// success read as a failure. Observed on `train_model`: three models trained
    /// successfully while the agent was told each had failed, and it reported to the
    /// user that training was impossible.
    #[test]
    fn an_explicit_null_error_is_a_success() {
        let ok = serde_json::json!({ "ok": true, "error": null, "run_id": "r1" });
        assert!(ok.get("error").is_none_or(serde_json::Value::is_null));

        let bad = serde_json::json!({ "error": "validation_failed" });
        assert!(bad.get("error").is_some_and(|e| !e.is_null()));

        let absent = serde_json::json!({ "ok": true });
        assert!(absent.get("error").is_none_or(serde_json::Value::is_null));
    }

    /// Only reads may be replayed after a crash. A write that looked idempotent would
    /// be spent twice, and for `run_sweep` that is trials the count cannot give back
    /// (INV-1).
    #[test]
    fn only_reads_are_idempotent() {
        let reg = registry().unwrap();
        assert!(reg.get("get_bars").unwrap().idempotent);
        assert!(reg.get("get_diagnostics").unwrap().idempotent);
        assert!(!reg.get("run_sweep").unwrap().idempotent);
        assert!(!reg.get("create_experiment").unwrap().idempotent);
        assert!(!reg.get("choose_null").unwrap().idempotent);
    }

    /// FEAT-003 §11: the agent may only cause Runs through Experiments and Studies,
    /// so every result it sees is sealed and counted. The bridge must not quietly
    /// widen that.
    #[test]
    fn the_bridge_does_not_widen_what_the_agent_profile_allows() {
        let reg = registry().unwrap();
        for forbidden in ["create_backtest", "arm_automation", "delete_backtest"] {
            assert!(
                reg.get(forbidden).is_none(),
                "{forbidden} is outside the agent profile and must not cross the bridge"
            );
        }
    }

    /// The routing unit. If a namespace held more tools than a local step may expose,
    /// routing to it would truncate deterministically and some tool would become
    /// permanently invisible — reachable only through the escape hatch.
    #[test]
    fn no_namespace_is_too_large_to_expose_on_a_local_step() {
        let reg = registry().unwrap();
        let index = reg.namespace_index();
        let worst = index.iter().max_by_key(|(_, n)| **n).unwrap();
        assert!(
            *worst.1 <= 16,
            "namespace {:?} holds {} tools; a step that routes to it cannot see them all",
            worst.0,
            worst.1
        );
    }

    #[test]
    fn platform_tools_are_tool_internal_and_the_default_is_not() {
        use harness::Provenance;
        assert_eq!(
            harness::provenance::classify(&source_for("data")),
            Provenance::ToolInternal
        );
        assert_eq!(
            harness::provenance::classify("reddit"),
            Provenance::ExternalUntrusted,
            "a collector that forgets to register here must fail safe"
        );
    }
}
