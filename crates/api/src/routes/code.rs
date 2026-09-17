//! `POST /api/code/run` — the one route that runs a program a model wrote
//! (AGENT-005 §8, AGENT-006 §5.9, AGENT-010 §10; ADR-0035; Set S task S-8.8).
//!
//! # The platform does not run the script
//!
//! It starts a container that does. The difference is the whole feature: a
//! script running in this process would hold the platform's database pool, its
//! service token and its file descriptors, and "it is only numeric code" is a
//! claim about a program nobody has read.
//!
//! # Refusing is the default, and it is not a stub
//!
//! There is no launcher unless a deployment supplies one (`TBOT_SANDBOX_CMD`).
//! With none, this route refuses. That is deliberate and it is the important
//! half: the tempting shortcut — run the script in a subprocess on the host when
//! no sandbox is configured — would make a development convenience into a
//! production hole, and it would be invisible, because the tool would work.
//!
//! So the failure mode of a missing sandbox is a tool that says so.
//!
//! # One workspace per conversation
//!
//! The launcher command is given `{workspace}`, which is a directory under
//! `TBOT_SANDBOX_WORKSPACES` named for the run. Two runs never share one: a
//! sandbox serving two conversations from one directory is a sandbox in name,
//! because a script can read every file it can reach.
//!
//! A run id that is not a UUID is refused rather than sanitised. Sanitising a
//! path fragment is how `..` becomes `.` and a workspace becomes its parent.

use std::path::PathBuf;
use std::process::Stdio;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// The launcher, as a deployment supplies it.
///
/// `{workspace}` is replaced with the run's directory. Everything else is the
/// deployment's: which image, which runtime, which limits. The platform does not
/// assume Docker — it assumes something that reads a request on stdin and writes
/// a response on stdout, which is what `infra/agent-image/executor.py` is.
const LAUNCHER_ENV: &str = "TBOT_SANDBOX_CMD";
const WORKSPACES_ENV: &str = "TBOT_SANDBOX_WORKSPACES";

#[derive(Debug, Deserialize)]
pub struct RunBody {
    pub script: String,
    #[serde(default)]
    pub payload: Value,
    /// A path inside the workspace the script should read.
    #[serde(default)]
    pub data_path: Option<String>,
    /// The run this belongs to — its workspace.
    pub run_id: Uuid,
}

fn not_configured() -> axum::response::Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "sandbox_not_configured",
            "message": format!(
                "this platform has no code sandbox: {LAUNCHER_ENV} is unset, so there is \
                 nowhere to run a script that is not this process."
            ),
            "fix": "use the templated tools, which run on the skill runner. Free-form code \
                    needs a sandbox, and the platform refuses rather than running your script \
                    where it would hold the platform's own credentials"
        })),
    )
        .into_response()
}

/// The workspace for one run, created if it does not exist.
fn workspace_for(run_id: Uuid) -> Result<PathBuf, String> {
    let root = std::env::var(WORKSPACES_ENV)
        .map_err(|_| format!("{WORKSPACES_ENV} is unset, so there is no workspace to mount"))?;
    // From the parsed UUID's own formatting, never from the request's text. A
    // path fragment that came off the wire is a path fragment that can contain
    // `..`.
    let dir = PathBuf::from(root).join(run_id.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("the workspace could not be created: {e}"))?;
    Ok(dir)
}

/// `POST /api/code/run`
pub async fn run(
    State(_state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<RunBody>,
) -> impl IntoResponse {
    let Ok(launcher) = std::env::var(LAUNCHER_ENV) else {
        return not_configured();
    };
    if body.script.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "missing_field", "field": "script" })),
        )
            .into_response();
    }

    let workspace = match workspace_for(body.run_id) {
        Ok(w) => w,
        Err(message) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "sandbox_not_configured", "message": message })),
            )
                .into_response()
        }
    };

    let rendered = launcher.replace("{workspace}", &workspace.to_string_lossy());
    let mut parts = rendered.split_whitespace();
    let Some(program) = parts.next() else {
        return not_configured();
    };
    let args: Vec<&str> = parts.collect();

    let request = json!({
        "script": body.script,
        "payload": body.payload,
        "data_path": body.data_path,
    });

    let child = tokio::process::Command::new(program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "sandbox_did_not_start",
                    "message": e.to_string(),
                    "fix": "the script was not run; nothing was executed on the host"
                })),
            )
                .into_response()
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let bytes = serde_json::to_vec(&request).unwrap_or_default();
        // A write failure is the sandbox having exited already. Reported, not
        // ignored: a request the sandbox never read is not a script that ran and
        // produced nothing.
        if let Err(e) = stdin.write_all(&bytes).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "sandbox_closed_early", "message": e.to_string() })),
            )
                .into_response();
        }
        drop(stdin);
    }

    match child.wait_with_output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            match serde_json::from_str::<Value>(stdout.trim()) {
                Ok(v) => (StatusCode::OK, Json(v)).into_response(),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": "sandbox_answered_nothing",
                        "message": "the sandbox did not return a JSON response",
                        "stderr": String::from_utf8_lossy(&out.stderr),
                        "stdout": stdout,
                    })),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "sandbox_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace path is built from the parsed UUID, never from request
    /// text. This is the test that says so — `workspace_for` takes a `Uuid`, so
    /// there is no spelling of `..` that can reach it.
    #[test]
    fn a_workspace_path_cannot_come_off_the_wire() {
        let body: Result<RunBody, _> = serde_json::from_value(json!({
            "script": "print(1)",
            "run_id": "../../etc"
        }));
        assert!(body.is_err(), "a run id that is not a UUID does not deserialise");
    }

    /// The launcher is a template with exactly one substitution, and the
    /// workspace is the only thing the platform puts into it.
    #[test]
    fn the_launcher_substitutes_only_the_workspace() {
        let launcher = "docker run --rm -i --network none -v {workspace}:/workspace img";
        let rendered = launcher.replace("{workspace}", "/srv/ws/abc");
        assert!(rendered.contains("/srv/ws/abc:/workspace"));
        assert!(!rendered.contains('{'), "an unsubstituted slot reached the command line");
    }

    /// A body without a run id does not deserialise, because a script with no
    /// workspace would have to be given one — and the only workspace a platform
    /// could invent is a shared one.
    #[test]
    fn a_script_without_a_run_has_no_workspace_and_is_refused() {
        let body: Result<RunBody, _> =
            serde_json::from_value(json!({ "script": "print(1)" }));
        assert!(body.is_err());
    }
}
