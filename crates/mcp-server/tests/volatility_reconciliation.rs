//! One engine, proven: the Python templates and `crates/features` compute the
//! same volatility (Set S task S-5.14; ADR-0029).
//!
//! # The drift this closes
//!
//! Phase 3 implemented Parkinson, Garman-Klass and Rogers-Satchell in
//! `apps/skill-runner/templates/analysis/estimate_volatility.py`, against numpy,
//! because the skill runner has no Rust (ADR-S3-01). `crates/features` computes
//! the same three families in Rust, because the strategy engine has no Python.
//! Both are correct decisions and together they are the thing the one-engine rule
//! exists to prevent: **two implementations of one estimator with nothing
//! comparing them.**
//!
//! Nobody would notice a divergence. A model asks `estimate_volatility` how
//! volatile an instrument is, gets a number, writes a strategy whose stop is
//! sized in `parkinson_vol_20`, and the two numbers describe the same window
//! through different arithmetic. The failure is silent, it is in the sizing, and
//! it would be debugged anywhere but here.
//!
//! # The one real difference, and why it is not a bug
//!
//! The template reports **variance per bar**, under a `variance_per_bar` key,
//! and annualises with `sqrt(v · bars_per_year)`. The feature engine reports a
//! per-bar **standard deviation** — the family is named `parkinson_vol_N` and
//! returns `sqrt(mean(...))`. Both are internally consistent and correctly
//! labelled, so this asserts `engine² == template`, which is the actual claim.
//!
//! Asserting it is the point. The relationship is currently true by coincidence
//! of two independent authorings; after this it is true by test, and a change to
//! either side that breaks it has to be argued for rather than discovered in a
//! position size.
//!
//! # Why it reads the fixture through the platform's own loader
//!
//! The bars come back from `preamble.load` — the same function the template
//! calls — rather than from a parquet reader written here. That matters: `load`
//! sorts and drops duplicate timestamps, and a reconciliation that read the file
//! differently would compare two estimators over two different sets of bars and
//! call the difference a formula error.

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

/// See `templates_run.rs` for why this is asked for rather than guessed.
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
             apps/skill-runner/requirements.txt, or point SKILL_RUNNER_PYTHON at an \
             interpreter that has it.",
            python()
        ),
    }
}

fn fixture_dir() -> PathBuf {
    let dir = workspace_root().join("target").join("template-fixture");
    if dir.join("primary.parquet").is_file() {
        return dir;
    }
    let script = workspace_root().join("apps/skill-runner/reference_dataset.py");
    let out = Command::new(python())
        .arg(&script)
        .arg(&dir)
        .output()
        .unwrap_or_else(|e| panic!("could not build the reference dataset: {e}"));
    assert!(
        out.status.success(),
        "the reference dataset would not build:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

/// The fixture's bars, read through `preamble.load` — the template's own loader.
fn bars(fixture: &Path) -> Vec<features::FeatureRow> {
    let path = fixture.join("primary.parquet");
    // The paths travel as argv rather than interpolated into the source. A
    // Windows path is full of backslashes, and quoting one into a Python literal
    // is an escaping bug waiting for the first directory whose name starts with
    // a `t` or an `n`. One line, for the same reason: no continuations to lose.
    const SCRIPT: &str = "import sys, json; sys.path.insert(0, sys.argv[1]); from preamble import load; f = load({'data': {'primary': sys.argv[2]}}, 'primary'); print(json.dumps({c: [float(x) for x in f[c]] for c in ('open','high','low','close','volume')}))";
    let out = Command::new(python())
        .arg("-c")
        .arg(SCRIPT)
        .arg(workspace_root().join("apps/skill-runner"))
        .arg(&path)
        .output()
        .unwrap_or_else(|e| panic!("could not read the fixture through `preamble.load`: {e}"));
    assert!(
        out.status.success(),
        "reading the fixture through `preamble.load` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cols: Value =
        serde_json::from_slice(&out.stdout).expect("the loader helper printed valid JSON");
    let col = |name: &str| -> Vec<f64> {
        cols[name]
            .as_array()
            .unwrap_or_else(|| panic!("the loader helper omitted `{name}`"))
            .iter()
            .filter_map(Value::as_f64)
            .collect()
    };
    let (o, h, l, c, v) = (
        col("open"),
        col("high"),
        col("low"),
        col("close"),
        col("volume"),
    );
    (0..c.len())
        .map(|i| features::FeatureRow {
            // The estimators are functions of OHLC alone, so the timestamp is
            // an index rather than a value under test.
            ts_ns: i as i64 * 3_600_000_000_000,
            open: o[i],
            high: h[i],
            low: l[i],
            close: c[i],
            volume: v[i],
        })
        .collect()
}

async fn run_template(name: &str, fixture: &Path) -> Value {
    let t = mcp_server_lib::tools::templated::templates()
        .iter()
        .find(|t| t.header().name == name)
        .unwrap_or_else(|| panic!("`{name}` is not a compiled-in template"));

    let payload = json!({
        "instrument_id": "BTC-USD",
        "timeframe": "1h",
        "start": "2024-01-08T00:00:00Z",
        "end": "2024-08-01T00:00:00Z",
        "seed": 7,
        "data": { "primary": fixture.join("primary.parquet").to_string_lossy() },
    });

    let skill = Skill::author(
        t.header().name.clone(),
        t.header().description.clone(),
        t.runnable_source(),
        t.contract(),
        None,
    )
    .unwrap_or_else(|e| panic!("`{name}` is not a runnable skill: {e}"));
    let config = RunnerConfig {
        python: python(),
        script: workspace_root().join("apps/skill-runner/skill_runner.py"),
        limits: t.limits(),
        stack_path: stack_path(),
    };
    let done = skills::runner::run(&skill, &payload, &config).await;
    assert_eq!(
        done.record.outcome,
        Outcome::Ok,
        "`{name}` did not complete: {:?}",
        done.record.outcome
    );
    done.result
        .unwrap_or_else(|| panic!("`{name}` returned no result"))
}

/// The engine's value for one family over the whole window.
fn engine(family: &str, n: usize, rows: &[features::FeatureRow]) -> f64 {
    let name = format!("{family}_{n}");
    let f = features::runtime::feature(&name)
        .unwrap_or_else(|e| panic!("the engine does not resolve `{name}`: {e}"));
    features::runtime::value_at(f.as_ref(), rows, rows.len() - 1)
        .unwrap_or_else(|| panic!("`{name}` produced no value over {n} bars"))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_templates_and_the_engine_compute_the_same_volatility() {
    let fixture = fixture_dir();
    let rows = bars(&fixture);
    let result = run_template("estimate_volatility", &fixture).await;

    let detail = result.get("detail").expect("a detail block");
    let per_bar = detail
        .get("variance_per_bar")
        .expect("the template reports variance per bar");

    // The two must be looking at the same bars. `load` sorts and de-duplicates,
    // so a mismatch here means the loader saw something this test did not — and
    // comparing estimators over different windows would report a formula error
    // that is really a windowing one.
    let template_bars = detail
        .get("n_bars")
        .and_then(Value::as_u64)
        .expect("the template reports how many bars it used") as usize;
    assert_eq!(
        template_bars + 1,
        rows.len(),
        "the template computed over {template_bars} returns and this test loaded {} bars. \
         The comparison below would be between two different windows.",
        rows.len()
    );

    // `bars_true`-style tolerance would hide the thing this test is for. These
    // are the same formula over the same f64 inputs in two languages, so the
    // only legitimate difference is floating-point summation order: numpy pairwise
    // against a serial Rust loop over 6,000 terms. A relative 1e-9 is far tighter
    // than any real divergence in a formula and far looser than that.
    const TOL: f64 = 1e-9;

    for (family, key) in [
        ("parkinson_vol", "parkinson"),
        ("garman_klass_vol", "garman_klass"),
        ("rogers_satchell_vol", "rogers_satchell"),
    ] {
        let template_variance = per_bar
            .get(key)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("the template did not report `{key}`"));
        let engine_stdev = engine(family, rows.len(), &rows);
        let engine_variance = engine_stdev * engine_stdev;

        assert!(
            template_variance > 0.0,
            "`{key}` came back at {template_variance} on the reference fixture, which has real \
             ranges; a zero or negative variance means the estimator is not being exercised"
        );
        let rel = (engine_variance - template_variance).abs() / template_variance;
        assert!(
            rel < TOL,
            "`{family}` and the `{key}` estimator in estimate_volatility.py disagree: the engine \
             says {engine_stdev} (variance {engine_variance}), the template says \
             {template_variance} — a relative difference of {rel:.3e}.\n\n\
             These are two implementations of one estimator, in two languages, and this test is \
             the only thing comparing them. A model reads the template's number to judge a \
             regime and the engine's number to size a stop; if they disagree, one of the two is \
             wrong and neither says which."
        );
    }
}

/// The relationship the test above rests on, stated separately so a failure says
/// which of the two claims broke.
///
/// If the engine ever starts reporting a variance, or the template a standard
/// deviation, the squaring above would silently compare the wrong quantities and
/// the reconciliation would fail looking like a formula error. This asserts the
/// *shape* — the engine's value is the square root of the template's — on a
/// family where the arithmetic is simple enough to see by eye.
#[tokio::test(flavor = "multi_thread")]
async fn the_engine_reports_a_deviation_and_the_template_a_variance() {
    let fixture = fixture_dir();
    let rows = bars(&fixture);
    let result = run_template("estimate_volatility", &fixture).await;
    let template_variance = result["detail"]["variance_per_bar"]["parkinson"]
        .as_f64()
        .expect("the template reports Parkinson");
    let engine_stdev = engine("parkinson_vol", rows.len(), &rows);

    assert!(
        engine_stdev > template_variance,
        "per-bar volatility is well under 1, so its square is smaller than itself: the engine's \
         {engine_stdev} should exceed the template's variance {template_variance}. If it does \
         not, one of the two has changed which quantity it reports and the reconciliation test \
         beside this one is comparing the wrong things."
    );
}
