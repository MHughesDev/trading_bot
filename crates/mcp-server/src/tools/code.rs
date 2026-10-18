//! `run_python` — the one tool that runs a program the model wrote (AGENT-005
//! §8, AGENT-006 §5.9, AGENT-010 §10; ADR-0035; Set S task S-8.8).
//!
//! # Why there is exactly one
//!
//! Because the alternative to free-form code is *more narrow tools*, and there
//! is a point past which a narrow tool for every question is a worse interface
//! than a language. The templated tier is where that line sits for smaller
//! models: `orchestration.code_execution` is `denied` (narrow tools only),
//! `templated` (scripts with templates in the charter) or `full` (this).
//!
//! Writing correct multi-step programs is exactly where small local models are
//! weakest, which is why the tier exists at all — a `denied` profile does not see
//! this tool, and the templated tools stay.
//!
//! # The exposure is the gate, and it is not the only one
//!
//! `ToolRegistry::expose_within` hides the whole `code` namespace below `full`,
//! so a model on a `templated` tier never sees the tool. A model that reaches
//! for it anyway — from a transcript, a memory, a guess — is refused by the
//! route, because a platform with no sandbox has nowhere to run a script and
//! says so rather than running it in the platform's own process.
//!
//! # No credential travels with the script
//!
//! The sandbox holds none: not a provider key, not a platform token. That is
//! enforced in `infra/agent-image/executor.py`, which refuses to start if it
//! finds one, and checked in CI. A script that could call a model could
//! exfiltrate the workspace by describing it.

use serde_json::{json, Value};

use crate::spec::{ToolSpec, TrialKind};
use crate::taxonomy::Risk;

/// The namespace the exposure filter gates. One string, read by
/// `harness::registry` and by this file.
pub const NAMESPACE: &str = "code";

/// `run_python` — run a script in this conversation's sandbox.
pub async fn run_python(ctx: &crate::McpContext, params: &Value) -> Value {
    let Some(script) = params.get("script").and_then(Value::as_str) else {
        return crate::tool_error::missing_field_because(
            "script",
            "the Python to run. Write your result to the path in RESULT_PATH as JSON; \
                    stdout is captured but a result is what the platform can cite",
        );
    };
    if ctx.run_id.is_nil() {
        // A workspace per conversation, and this front door has no conversation.
        // The alternative is a shared workspace, which is not a sandbox.
        return crate::tool_error::refuse(
            "no_workspace",
            "`run_python` runs in the conversation's own workspace and this front door has no run behind it.",
            "call run_python from an agent run (it has a workspace); there is no shared one to fall back to",
        );
    }
    let body = json!({
        "script": script,
        "payload": params.get("payload").cloned().unwrap_or_else(|| json!({})),
        "data_path": params.get("data_path").cloned(),
        "run_id": ctx.run_id.to_string(),
    });
    match ctx.api.post("/api/code/run", body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// The catalogue entry.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        "run_python",
        NAMESPACE,
        // A write: it changes the conversation's workspace. Not `Destructive` —
        // the workspace is the conversation's own and goes away with it — and
        // emphatically not `Read`, which is what would put it in ring 0.
        Risk::Write,
        "Run a Python script in this conversation's sandbox. The research stack is available \
         (polars, numpy, scipy, statsmodels, scikit-learn, lightgbm, torch-cpu). Read your \
         input from the JSON at PAYLOAD_PATH and write your answer as JSON to RESULT_PATH; \
         stdout is captured but only the result can be cited. The sandbox has no network and \
         no credential of any kind, so it cannot call the platform — use the platform's tools \
         for that.",
        json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "The Python to run."
                },
                "payload": {
                    "type": "object",
                    "description": "JSON your script reads from PAYLOAD_PATH."
                },
                "data_path": {
                    "type": "string",
                    "description": "A file in this conversation's workspace, passed as DATA_PATH. \
                                    A path outside the workspace is refused."
                }
            },
            "required": ["script"]
        }),
    )
    .internal()
    // It runs arithmetic, not the simulator. A script that wanted to run a
    // backtest would have to call a tool, and it has no way to call one — which
    // is what keeps INV-16 true with this tool in the catalogue.
    .spends_trial(TrialKind::None)
    // Expensive and easy to reach for. Three is enough to iterate on a
    // calculation and few enough that "write a program" is not the first move.
    .max_calls_per_run(3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property that keeps INV-16 true with a code tool in the
    /// catalogue: this tool cannot run the simulator, so it declares no trial —
    /// and it cannot run the simulator because the sandbox cannot reach the
    /// platform.
    #[test]
    fn the_sandbox_cannot_run_the_simulator_so_it_spends_no_trial() {
        let spec = &specs()[0];
        assert_eq!(spec.spends_trial, TrialKind::None);
        assert!(
            spec.description.contains("no credential"),
            "the description does not tell the model the sandbox cannot call the platform"
        );
    }

    /// The namespace is the thing the exposure filter reads. A tool in this file
    /// under a different namespace would be offered on every tier.
    #[test]
    fn every_tool_here_is_in_the_gated_namespace() {
        for s in specs() {
            assert_eq!(
                s.namespace, NAMESPACE,
                "`{}` is in this file and not in the namespace the exposure filter gates",
                s.name
            );
        }
    }

    /// A write, so it is ring 1. A `Read` classification would put free-form
    /// code execution in the ring a Chat turn can reach.
    #[test]
    fn free_form_code_is_not_a_read() {
        let spec = &specs()[0];
        assert_eq!(spec.risk, Risk::Write);
        assert_eq!(spec.ring(), crate::spec::RING_PLATFORM_ARTIFACTS);
    }

    /// Capped. The budget is the honest form of "this is expensive"; the
    /// alternative is a run that writes a program every step.
    #[test]
    fn it_is_capped_per_run() {
        assert_eq!(specs()[0].max_calls_per_run, Some(3));
    }
}
