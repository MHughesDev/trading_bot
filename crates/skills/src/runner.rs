//! Spawning the Python runner and enforcing the wall clock.
//!
//! The division of labour, and it matters which side does which:
//!
//! - **The child** applies kernel resource limits to itself and blocks the
//!   socket layer, because those have to happen inside the process.
//! - **The parent** — this module — enforces the wall clock, because a child
//!   asked to police its own deadline can be prevented from doing so by the code
//!   it is policing. A tight loop inside a C extension never yields to a Python
//!   signal handler, and a skill that spins there would run until something else
//!   killed it.
//!
//! So the timeout lives here, and it is a kill rather than a request.

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

/// What the child produced. A small struct rather than `std::process::Output`
/// because the status is only ever rendered, and keeping it as text avoids
/// carrying a platform type through the error path.
struct Output {
    status_text: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

use crate::{Contract, Outcome, RunRecord, Skill, SkillHash};

/// What a skill is allowed to consume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub wall_seconds: u64,
    pub cpu_seconds: u64,
    pub memory_mb: u64,
    pub output_bytes: u64,
}

impl Default for Limits {
    /// Enough for a real data transformation over a few hundred thousand rows,
    /// and not enough to be a problem. Every one of these is a deliberate
    /// number rather than a round one: a skill that needs more than thirty
    /// seconds and half a gigabyte is a job, and jobs go through the job store
    /// where they can be cancelled and observed.
    fn default() -> Self {
        Self {
            wall_seconds: 30,
            cpu_seconds: 25,
            memory_mb: 512,
            output_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Where Python and the runner script live.
#[derive(Clone, Debug)]
pub struct RunnerConfig {
    pub python: String,
    pub script: std::path::PathBuf,
    pub limits: Limits,
    /// Directories the platform's analysis stack lives in, passed as
    /// `PYTHONPATH`.
    ///
    /// The child runs with `PYTHONNOUSERSITE=1`, so whatever a developer happens
    /// to have installed in their own user directory is invisible — which is the
    /// right default for a process that executes agent-authored code, and is
    /// also why numpy has to be *named* rather than found. This is the platform
    /// saying where its own stack is, not a hole in that rule: a package that is
    /// not in one of these directories or in the interpreter's own
    /// `site-packages` still does not load.
    ///
    /// Empty in deployment, where the container installs the stack into the
    /// interpreter's `site-packages` and there is no user directory at all.
    pub stack_path: Vec<std::path::PathBuf>,
}

impl RunnerConfig {
    /// Locate the runner relative to the workspace root.
    #[must_use]
    pub fn from_workspace(root: &std::path::Path) -> Self {
        Self {
            python: std::env::var("SKILL_RUNNER_PYTHON").unwrap_or_else(|_| "python".into()),
            script: root.join("apps/skill-runner/skill_runner.py"),
            limits: Limits::default(),
            stack_path: std::env::var("SKILL_RUNNER_STACK_PATH")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|s| std::env::split_paths(&s).collect())
                .unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
struct Job<'a> {
    source: &'a str,
    payload: &'a serde_json::Value,
    limits: JobLimits,
}

#[derive(Serialize)]
struct JobLimits {
    cpu_seconds: u64,
    memory_mb: u64,
    output_bytes: u64,
}

#[derive(Deserialize, Default)]
struct Envelope {
    outcome: String,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    peak_memory_bytes: Option<u64>,
}

/// One run's result: the record, plus whatever the skill returned.
#[derive(Clone, Debug)]
pub struct Completed {
    pub record: RunRecord,
    pub result: Option<serde_json::Value>,
}

/// Run a skill against a payload.
///
/// Checks the contract on both sides — before, so a bad payload never burns a
/// process, and after, so a skill that quietly stops returning a field is caught
/// by the platform rather than by whoever consumes the field three steps later.
///
/// Never returns `Err` for anything the skill did: a skill that raises, times
/// out or breaks its contract produces a [`RunRecord`] saying so, because those
/// are *results* and the library's whole value is in keeping them.
pub async fn run(
    skill: &Skill,
    payload: &serde_json::Value,
    config: &RunnerConfig,
) -> Completed {
    let started = Instant::now();

    if let Err(violation) = skill.contract().check_input(payload) {
        // Fails before spawning: a bad payload should not cost a process, and
        // it is not evidence about the skill either way.
        return finish(skill.hash(), violation.into(), started, None, String::new());
    }

    let job = Job {
        source: skill.source(),
        payload,
        limits: JobLimits {
            cpu_seconds: config.limits.cpu_seconds,
            memory_mb: config.limits.memory_mb,
            output_bytes: config.limits.output_bytes,
        },
    };
    let Ok(job_json) = serde_json::to_vec(&job) else {
        return finish(
            skill.hash(),
            Outcome::RunnerFailed { detail: "the job would not serialise".into() },
            started,
            None,
            String::new(),
        );
    };

    let mut command = tokio::process::Command::new(&config.python);
    command
        .arg(&config.script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A trimmed environment: the child gets nothing it was not given, so a
        // credential in the platform's environment is absent because it was
        // never passed rather than because something removed it.
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PYTHONIOENCODING", "utf-8")
        // No `.pyc` files in a directory the skill can see, and no site-packages
        // surprises from a user directory.
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONNOUSERSITE", "1");

    if !config.stack_path.is_empty() {
        if let Ok(joined) = std::env::join_paths(&config.stack_path) {
            command.env("PYTHONPATH", joined);
        }
    }

    #[cfg(windows)]
    {
        // `SystemRoot` is required for the Windows CRT to initialise; without it
        // Python will not start at all.
        if let Ok(v) = std::env::var("SystemRoot") {
            command.env("SystemRoot", v);
        }
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            return finish(
                skill.hash(),
                Outcome::RunnerFailed {
                    detail: format!("could not start {}: {e}", config.python),
                },
                started,
                None,
                String::new(),
            )
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&job_json).await;
        let _ = stdin.shutdown().await;
    }

    // Read the pipes concurrently with the wait, or a skill that fills a 64 KB
    // pipe buffer blocks forever and the "timeout" becomes a deadlock.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let collect = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = tokio::io::AsyncReadExt::read_to_end(p, &mut out).await;
        }
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = tokio::io::AsyncReadExt::read_to_end(p, &mut err).await;
        }
        (out, err)
    };

    let deadline = Duration::from_secs(config.limits.wall_seconds);
    let waited = tokio::time::timeout(deadline, async {
        let piped = collect.await;
        let status = child.wait().await;
        (piped, status)
    })
    .await;

    let output = match waited {
        Ok(((out, err), Ok(status))) => Output { status_text: status.to_string(), stdout: out, stderr: err },
        Ok((_, Err(e))) => {
            return finish(
                skill.hash(),
                Outcome::RunnerFailed { detail: e.to_string() },
                started,
                None,
                String::new(),
            )
        }
        Err(_) => {
            // **Kill, then reap.** Dropping the future does not stop the
            // process: the child keeps running, and a skill spinning in a tight
            // loop then burns a core until something else notices. An earlier
            // version of this used `wait_with_output`, which consumes the handle
            // and leaves nothing to kill with — the orphan it left took the test
            // harness down with it, which is how this was found.
            let _ = child.start_kill();
            let _ = child.wait().await;
            return finish(
                skill.hash(),
                Outcome::TimedOut { limit_secs: config.limits.wall_seconds },
                started,
                None,
                String::new(),
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Ok(envelope) = serde_json::from_str::<Envelope>(stdout.trim()) else {
        // No envelope means the interpreter died before the runner could write
        // one — an OOM kill, a segfault in a C extension, or a signal.
        let stderr = String::from_utf8_lossy(&output.stderr);
        return finish(
            skill.hash(),
            Outcome::Killed {
                reason: format!(
                    "the interpreter exited without a result envelope ({}). This is usually the \
                     memory limit or a crash inside a native extension. stderr: {}",
                    output.status_text,
                    stderr.chars().take(400).collect::<String>()
                ),
            },
            started,
            None,
            String::new(),
        );
    };

    let outcome = match envelope.outcome.as_str() {
        "ok" => match &envelope.result {
            Some(v) => match skill.contract().check_output(v) {
                Ok(()) => Outcome::Ok,
                Err(violation) => violation.into(),
            },
            None => Outcome::BrokeContract {
                detail: "the runner reported success with no result".into(),
            },
        },
        "raised" => Outcome::Raised { exception: envelope.detail.clone() },
        "bad_output" => Outcome::BrokeContract { detail: envelope.detail.clone() },
        "killed" => Outcome::Killed { reason: envelope.detail.clone() },
        other => Outcome::RunnerFailed {
            detail: format!("unknown outcome {other:?} from the runner"),
        },
    };

    let result = outcome.succeeded().then(|| envelope.result.clone()).flatten();
    let mut completed = finish(
        skill.hash(),
        outcome,
        started,
        envelope.peak_memory_bytes,
        envelope.stdout,
    );
    completed.result = result;
    completed
}

fn finish(
    hash: &SkillHash,
    outcome: Outcome,
    started: Instant,
    peak: Option<u64>,
    stdout: String,
) -> Completed {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut record = RunRecord::new(hash.clone(), outcome, ms);
    record.peak_memory_bytes = peak;
    record.stdout_note = stdout;
    Completed { record, result: None }
}

/// A contract for a skill that takes nothing and returns one key. Handy in
/// tests and for the examples the agent is shown.
#[must_use]
pub fn simple_contract(output: &str, summary: &str) -> Contract {
    let no_inputs: [&str; 0] = [];
    Contract::new(no_inputs, [output], summary).expect("one output is a valid contract")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workspace_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    fn config() -> RunnerConfig {
        RunnerConfig::from_workspace(&workspace_root())
    }

    fn skill_from(source: &str, contract: Contract) -> Skill {
        Skill::author(
            "test_skill",
            "A skill used by the runner tests, described at sufficient length.",
            source,
            contract,
            None,
        )
        .expect("authors")
    }

    #[test]
    fn the_runner_script_is_where_the_config_says_it_is() {
        assert!(
            config().script.is_file(),
            "apps/skill-runner/skill_runner.py must exist at {:?}",
            config().script
        );
    }

    /// The limits are deliberate numbers. A skill that needs more than this is a
    /// job, and jobs go through the job store where they can be cancelled.
    #[test]
    fn the_default_limits_are_bounded() {
        let l = Limits::default();
        assert!(l.wall_seconds <= 60);
        assert!(l.cpu_seconds < l.wall_seconds, "CPU must bind before the wall clock");
        assert!(l.memory_mb <= 2048);
    }

    /// A bad payload never costs a process, and it is not evidence about the
    /// skill either way.
    #[tokio::test]
    async fn a_payload_missing_a_declared_input_fails_without_spawning() {
        let contract = Contract::new(["values"], ["mean"], "Mean.").unwrap();
        let skill = skill_from("def run(p):\n    return {'mean': 0}\n", contract);

        let done = run(&skill, &json!({}), &config()).await;
        assert!(matches!(done.record.outcome, Outcome::BadInput { .. }));
        assert!(!done.record.outcome.is_about_the_skill());
        assert!(done.record.duration_ms < 1_000, "nothing should have been spawned");
    }

    #[test]
    fn a_simple_contract_needs_no_inputs() {
        let c = simple_contract("now", "Current time.");
        assert!(c.inputs().is_empty());
        assert!(c.outputs().contains("now"));
    }
}
