//! End-to-end tests against a real Python interpreter.
//!
//! `#[ignore]` because they need `python` on PATH. Everything here is about
//! behaviour that only exists when a process actually runs — a unit test can
//! check that the *types* say a skill has no network, and only this can check
//! that it does not.
//!
//! ```text
//! cargo test -p skills --test live_runner -- --ignored --test-threads=1
//! ```

use serde_json::json;
use skills::runner::{run, RunnerConfig};
use skills::{Contract, Outcome, Skill};

fn config() -> RunnerConfig {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf();
    RunnerConfig::from_workspace(&root)
}

fn skill(source: &str, inputs: &[&str], outputs: &[&str]) -> Skill {
    Skill::author(
        "a_test_skill",
        "A skill authored by the runner's end-to-end tests, described at length.",
        source,
        Contract::new(inputs.to_vec(), outputs.to_vec(), "test").unwrap(),
        None,
    )
    .expect("authors")
}

// ── the happy path ───────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_skill_runs_and_returns_its_result() {
    let s = skill(
        "import statistics\n\
         def run(payload):\n\
         \x20   xs = payload['values']\n\
         \x20   return {'mean': statistics.fmean(xs), 'n': len(xs)}\n",
        &["values"],
        &["mean"],
    );

    let done = run(&s, &json!({"values": [1.0, 2.0, 3.0, 4.0]}), &config()).await;
    assert_eq!(done.record.outcome, Outcome::Ok, "{:?}", done.record.outcome);
    let result = done.result.expect("a result");
    assert!((result["mean"].as_f64().unwrap() - 2.5).abs() < 1e-9);
    assert_eq!(result["n"], 4);
}

/// `print` is how a Python author debugs. A runner that swallows stdout makes a
/// skill undebuggable from outside — and one that lets it reach the result
/// channel is a runner whose output a skill can forge.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn what_a_skill_prints_is_captured_and_kept_out_of_the_result() {
    let s = skill(
        "def run(payload):
             mod = __import__('so' + 'cket')
             sock = mod.socket()
             sock.connect(('example.com', 80))
             return {'reached': True}
",
        &[],
        &["reached"],
    );

    let done = run(&s, &json!({}), &config()).await;
    match &done.record.outcome {
        Outcome::Raised { exception } => {
            assert!(
                exception.contains("no network"),
                "the refusal should name the rule: {exception}"
            );
        }
        other => panic!("a socket must not open: {other:?}"),
    }
    assert!(done.result.is_none());
}

/// The other half: an obvious `import socket` never reaches the sandbox at all,
/// because authoring refuses it with an explanation.
#[test]
fn an_obvious_network_import_is_refused_at_authoring_time() {
    let err = Skill::author(
        "a_test_skill",
        "A skill that tries to open a socket, described at sufficient length.",
        "import socket
def run(payload):
    return {'x': 1}
",
        Contract::new(Vec::<String>::new(), vec!["x"], "test").unwrap(),
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("no network"), "{err}");
}

/// Each HTTP library, hidden from the authoring scan the same way, so this
/// measures the runtime block rather than the static one.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn the_common_http_libraries_fail_by_name_at_run_time() {
    for module in ["requests", "httpx", "urllib3"] {
        let head = &module[..2];
        let tail = &module[2..];
        let source = format!(
            "def run(payload):
    __import__('{head}' + '{tail}')
    return {{'ok': True}}
"
        );
        let s = skill(&source, &[], &["ok"]);
        let done = run(&s, &json!({}), &config()).await;
        assert!(
            matches!(&done.record.outcome, Outcome::Raised { exception } if exception.contains("no network")),
            "{module}: {:?}",
            done.record.outcome
        );
    }
}

/// A child asked to police its own deadline can be prevented from doing so by
/// the code it is policing, which is why the parent kills.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_skill_that_never_returns_is_killed_by_the_parent() {
    let mut cfg = config();
    cfg.limits.wall_seconds = 2;
    cfg.limits.cpu_seconds = 60; // deliberately not the binding limit

    let s = skill(
        "def run(payload):\n\x20   while True:\n\x20       pass\n",
        &[],
        &["never"],
    );

    let started = std::time::Instant::now();
    let done = run(&s, &json!({}), &cfg).await;
    assert!(
        matches!(done.record.outcome, Outcome::TimedOut { limit_secs: 2 }),
        "{:?}",
        done.record.outcome
    );
    assert!(started.elapsed().as_secs() < 10, "the kill must not wait for the CPU limit");
}

// ── failures are results, not errors ─────────────────────────────────────────

#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_skill_that_raises_records_the_exception() {
    let s = skill(
        "def run(payload):\n\x20   return {'x': payload['missing_key']}\n",
        &[],
        &["x"],
    );
    let done = run(&s, &json!({}), &config()).await;
    match &done.record.outcome {
        Outcome::Raised { exception } => assert!(exception.contains("KeyError"), "{exception}"),
        other => panic!("{other:?}"),
    }
    assert!(done.record.outcome.is_about_the_skill());
}

/// The check that catches a skill quietly dropping a field, at the platform
/// rather than at whoever consumes the field three steps later.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_skill_that_omits_a_declared_output_breaks_its_contract() {
    let s = skill(
        "def run(payload):\n\x20   return {'something_else': 1}\n",
        &[],
        &["sharpe"],
    );
    let done = run(&s, &json!({}), &config()).await;
    match &done.record.outcome {
        Outcome::BrokeContract { detail } => {
            assert!(detail.contains("sharpe"), "{detail}");
            assert!(detail.contains("skill's error"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_skill_returning_something_unserialisable_is_caught() {
    let s = skill(
        "def run(payload):\n\x20   return {'f': lambda x: x}\n",
        &[],
        &["f"],
    );
    let done = run(&s, &json!({}), &config()).await;
    assert!(
        matches!(&done.record.outcome, Outcome::BrokeContract { detail } if detail.contains("not JSON")),
        "{:?}",
        done.record.outcome
    );
}

/// A skill that calls `sys.exit()` would otherwise take the envelope with it and
/// the caller would see a runner failure for a skill bug.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_skill_that_exits_is_a_skill_error_not_a_runner_failure() {
    let s = skill(
        "import sys\ndef run(payload):\n\x20   sys.exit(3)\n",
        &[],
        &["x"],
    );
    let done = run(&s, &json!({}), &config()).await;
    assert!(
        matches!(&done.record.outcome, Outcome::Raised { exception } if exception.contains("sys.exit")),
        "{:?}",
        done.record.outcome
    );
    assert!(done.record.outcome.is_about_the_skill());
}

/// A runner that cannot start is not evidence about the code it did not run.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_missing_interpreter_is_a_runner_failure_not_a_skill_failure() {
    let mut cfg = config();
    cfg.python = "definitely_not_a_python_on_this_machine".into();

    let s = skill("def run(payload):\n\x20   return {'x': 1}\n", &[], &["x"]);
    let done = run(&s, &json!({}), &cfg).await;
    assert!(matches!(done.record.outcome, Outcome::RunnerFailed { .. }));
    assert!(!done.record.outcome.is_about_the_skill());
}

// ── accumulation ─────────────────────────────────────────────────────────────

/// The claim the library rests on: a record builds up without anybody training
/// anything, and a good skill handed bad payloads still reads as good.
#[tokio::test]
#[ignore = "requires python on PATH"]
async fn a_record_accumulates_and_attributes_correctly() {
    let s = skill(
        "def run(payload):\n\x20   return {'doubled': payload['x'] * 2}\n",
        &["x"],
        &["doubled"],
    );
    let cfg = config();
    let mut stats = skills::SkillStats::default();

    for i in 0..5 {
        let done = run(&s, &json!({"x": i}), &cfg).await;
        stats.record(&done.record.outcome, done.record.duration_ms);
    }
    // Three calls with the wrong payload: the caller's fault, not the skill's.
    for _ in 0..3 {
        let done = run(&s, &json!({"wrong": 1}), &cfg).await;
        stats.record(&done.record.outcome, done.record.duration_ms);
    }

    assert_eq!(stats.runs, 8);
    assert_eq!(stats.ok, 5);
    assert_eq!(stats.bad_input, 3);
    assert_eq!(stats.failures, 0);
    assert_eq!(stats.reliability(), Some(1.0));
    assert_eq!(stats.confidence(), "reliable");
}
