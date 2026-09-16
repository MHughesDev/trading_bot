//! Every template actually runs, and twice is the same as once (S-3.1, S-3.2).
//!
//! # Why this is not `#[ignore]`d
//!
//! `crates/skills/tests/live_runner.rs` is ignored by default, and rightly: it
//! tests the *runner*, and a developer without Python still wants the rest of the
//! suite. This file is different. A template is the platform's own code and its
//! header is a catalogue entry a model will be offered — an unrun determinism
//! check is an invariant that is claimed and not enforced, which is the one thing
//! this project does not do. So it runs, and if the interpreter cannot satisfy
//! `apps/skill-runner/requirements.txt` it fails saying exactly that.
//!
//! # What "deterministic" has to mean here
//!
//! Byte-identical `detail` across two runs of the same seeded call. Not "close
//! to": a result that differs in the last ulp is a result whose hash differs, and
//! a trial ticket citing it cannot be checked against a re-run. The preamble does
//! the work that makes this true — single-threaded BLAS, sorted mappings, twelve
//! significant digits — and this is what proves it did.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use skills::runner::RunnerConfig;
use skills::{Outcome, Skill};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .to_path_buf()
}

fn python() -> String {
    std::env::var("SKILL_RUNNER_PYTHON").unwrap_or_else(|_| "python".into())
}

/// Where this machine keeps the analysis stack.
///
/// The runner sets `PYTHONNOUSERSITE=1`, so a developer install under
/// `~/.local` or `%APPDATA%\Python` is invisible to the child — deliberately,
/// because the child executes agent-authored code. In the deployment container
/// the stack is in the interpreter's own `site-packages` and this is empty; on a
/// developer machine it usually is not, so the interpreter is asked once where
/// numpy is and told to look there. A hard-coded guess would be wrong on every
/// second machine.
fn stack_path() -> Vec<PathBuf> {
    if let Ok(explicit) = std::env::var("SKILL_RUNNER_STACK_PATH") {
        if !explicit.trim().is_empty() {
            return std::env::split_paths(&explicit).collect();
        }
    }
    let out = Command::new(python())
        .args([
            "-c",
            "import numpy, os; print(os.path.dirname(os.path.dirname(numpy.__file__)))",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if dir.is_empty() {
                vec![]
            } else {
                vec![PathBuf::from(dir)]
            }
        }
        _ => panic!(
            "`{}` cannot import numpy. Templated tools are Python: install \
             apps/skill-runner/requirements.txt, or point SKILL_RUNNER_PYTHON at an interpreter \
             that has it.",
            python()
        ),
    }
}

/// Build the reference parquet once and reuse it.
///
/// Under `target/` rather than a temp dir so that a failing template can be
/// re-run by hand against the exact bytes the test used.
fn fixture_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("template-fixture");
    if dir.join("primary.parquet").is_file() && dir.join("secondary.parquet").is_file() {
        return dir;
    }
    let script = workspace_root().join("apps/skill-runner/reference_dataset.py");
    let out = Command::new(python())
        .arg(&script)
        .arg(&dir)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "could not run {} with `{}`: {e}. Templated tools are Python, and this suite \
                 needs an interpreter with apps/skill-runner/requirements.txt installed.",
                script.display(),
                python()
            )
        });
    assert!(
        out.status.success(),
        "the reference dataset would not build:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

/// A value for every argument a template can require.
///
/// Keyed by argument name rather than inferred from the schema, and a required
/// argument with no entry here fails the test. That is deliberate: adding a
/// template with a new argument should force a decision about what a
/// *representative* value for it is, rather than letting the test invent `0` and
/// quietly exercise the degenerate path.
fn argument(name: &str) -> Option<Value> {
    Some(match name {
        "instrument_id" => json!("BTC-USD"),
        "secondary_instrument_id" => json!("ETH-USD"),
        "timeframe" => json!("1h"),
        // The fixture's window, less a margin at each end so a template that
        // trims has something to trim.
        "start" => json!("2024-01-08T00:00:00Z"),
        "end" => json!("2024-08-01T00:00:00Z"),
        "seed" => json!(7),
        _ => return None,
    })
}

fn payload_for(t: &skills::Template, fixture: &Path) -> Value {
    let h = t.header();
    let mut map = serde_json::Map::new();
    let required: Vec<String> = h
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    for name in required {
        let value = argument(&name).unwrap_or_else(|| {
            panic!(
                "`{}` requires the argument `{name}`, and this suite has no representative value \
                 for it. Add one to `argument()` rather than letting the test guess.",
                h.name
            )
        });
        map.insert(name, value);
    }
    // Always seeded. A template that draws without declaring `seed` required
    // would otherwise raise, which is the preamble's intent.
    map.entry("seed".to_string()).or_insert(json!(7));

    let mut data = serde_json::Map::new();
    for series in &h.series {
        let file = if series.role == "primary" {
            "primary.parquet"
        } else {
            "secondary.parquet"
        };
        data.insert(
            series.role.clone(),
            json!(fixture.join(file).to_string_lossy()),
        );
    }
    map.insert("data".into(), Value::Object(data));
    Value::Object(map)
}

async fn run_once(t: &skills::Template, payload: &Value) -> (Outcome, Option<Value>) {
    let skill = Skill::author(
        t.header().name.clone(),
        t.header().description.clone(),
        t.runnable_source(),
        t.contract(),
        None,
    )
    .unwrap_or_else(|e| panic!("`{}` is not a runnable skill: {e}", t.header().name));

    let config = RunnerConfig {
        python: python(),
        script: workspace_root().join("apps/skill-runner/skill_runner.py"),
        limits: t.limits(),
        stack_path: stack_path(),
    };
    let done = skills::runner::run(&skill, payload, &config).await;
    (done.record.outcome, done.result)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_template_runs_on_the_reference_dataset_and_is_reproducible() {
    let fixture = fixture_dir();
    let templates = mcp_server_lib::tools::templated::templates();
    assert!(
        !templates.is_empty(),
        "no templates were compiled in; `build.rs` found nothing under \
         apps/skill-runner/templates"
    );

    for t in templates {
        let name = &t.header().name;
        let payload = payload_for(t, &fixture);

        let (outcome, first) = run_once(t, &payload).await;
        assert_eq!(
            outcome,
            Outcome::Ok,
            "`{name}` did not complete: {outcome:?}"
        );
        let first = first.unwrap_or_else(|| panic!("`{name}` returned no result"));

        // A refusal is a legitimate answer, but not on the reference dataset:
        // the fixture is built to be adequate for every template, so a refusal
        // here means either the fixture or the guard is wrong.
        assert!(
            first.get("refused").is_none(),
            "`{name}` refused the reference dataset: {}",
            first
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("(no reason)")
        );

        let detail = first
            .get("detail")
            .unwrap_or_else(|| panic!("`{name}` returned no `detail`"));
        assert!(
            detail.get("provenance").is_some(),
            "`{name}` did not stamp its provenance; a number nobody can re-derive is not evidence"
        );

        let (outcome2, second) = run_once(t, &payload).await;
        assert_eq!(outcome2, Outcome::Ok, "`{name}` failed on its second run");
        let second = second.expect("a result");
        assert_eq!(
            serde_json::to_string(detail).unwrap(),
            serde_json::to_string(second.get("detail").unwrap()).unwrap(),
            "`{name}` is not reproducible: two runs of the same seeded call differ. The usual \
             causes are an unsorted dict, an unseeded generator, and a BLAS reduction over more \
             than one thread."
        );
    }
}

/// The summary a model reads is rendered from the template's own declared
/// fields, so a template that stops emitting one renders "(n unknown)" in the
/// one line a flat-tier model reads first.
#[tokio::test(flavor = "multi_thread")]
async fn every_declared_summary_field_is_actually_emitted() {
    let fixture = fixture_dir();
    for t in mcp_server_lib::tools::templated::templates() {
        let h = t.header();
        let Some(template) = &h.summary_template else {
            continue;
        };
        let payload = payload_for(t, &fixture);
        let (_, result) = run_once(t, &payload).await;
        let detail = result
            .as_ref()
            .and_then(|r| r.get("detail"))
            .unwrap_or_else(|| panic!("`{}` returned no detail", h.name));

        for slot in skills::template::slots(template) {
            let value = detail.get(&slot);
            assert!(
                value.is_some() && !value.unwrap().is_null(),
                "`{}` declares `{slot}` in its summary and did not emit it on the reference \
                 dataset",
                h.name
            );
        }
    }
}
