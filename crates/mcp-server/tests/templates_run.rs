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
fn argument(tool: &str, name: &str) -> Option<Value> {
    // Per-tool first, and only where the fixture forces it. This is not an
    // escape hatch: `bar_boundary_check` compares two resolutions of one
    // instrument, so its `timeframe` is the *fast* leg and the shared default
    // would have it comparing 1h against 1h — which the template correctly
    // refuses, and which is how this override came to exist.
    if tool == "bar_boundary_check" && name == "timeframe" {
        return Some(json!("1m"));
    }
    Some(match name {
        "instrument_id" => json!("BTC-USD"),
        "secondary_instrument_id" => json!("ETH-USD"),
        "timeframe" => json!("1h"),
        // The fixture's window, less a margin at each end so a template that
        // trims has something to trim.
        "start" => json!("2024-01-08T00:00:00Z"),
        "end" => json!("2024-08-01T00:00:00Z"),
        "seed" => json!(7),
        // A representative event, not a rare one: the suite is checking that
        // the template runs and reproduces, and a threshold that yields four
        // events exercises the refusal path instead.
        "event" => json!("large_down_move"),
        // The evidentiary generator, so the suite exercises the path whose
        // output a run is allowed to cite.
        "generator" => json!("stationary_block_bootstrap"),
        // The timeframe pair: same instrument, two resolutions.
        "higher_timeframe" => json!("1h"),
        // A family fixture stands in for a real experiment id.
        "experiment_id" => json!("00000000-0000-0000-0000-000000000001"),
        _ => return None,
    })
}

/// Which fixture file a declared role is fed from.
///
/// Named rather than positional, for the same reason `argument` is: a template
/// that adds a role should force a decision about what that role's
/// *representative* data is. The pairs are deliberate — `secondary` is a
/// different instrument (correlation, cointegration, spanning), `slow` is the
/// **same** instrument at a coarser resolution (bar-boundary), and conflating
/// the two would make a cointegration test pass against a series' own
/// aggregate.
fn fixture_file(role: &str) -> &'static str {
    match role {
        "primary" => "primary.parquet",
        "secondary" => "secondary.parquet",
        "fast" => "fast.parquet",
        "slow" => "slow.parquet",
        "family" => "family.parquet",
        other => panic!(
            "no fixture file for the series role `{other}`. Add one to `fixture_file` and, if              the fixture does not contain suitable data yet, to reference_dataset.py."
        ),
    }
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
        let value = argument(&h.name, &name).unwrap_or_else(|| {
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
        data.insert(
            series.role.clone(),
            json!(fixture.join(fixture_file(&series.role)).to_string_lossy()),
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

        // Presence, not non-nullness. A null is a legitimate answer — the
        // reference series is a random walk, so `stationarity` has no half-life
        // to report and correctly says so — and the renderer prints
        // "(n unknown)" for it. A *missing* key is the template having forgotten
        // the field, which renders identically and means something entirely
        // different.
        let mut non_null = 0;
        let slots = skills::template::slots(template);
        for slot in &slots {
            let value = detail.get(slot);
            assert!(
                value.is_some(),
                "`{}` declares `{slot}` in its summary and did not emit the field at all on the \
                 reference dataset. An absent field and a null field both render as \
                 \"(n unknown)\"; only one of them is deliberate.",
                h.name
            );
            if !value.unwrap().is_null() {
                non_null += 1;
            }
        }
        assert!(
            slots.is_empty() || non_null > 0,
            "every slot in `{}`'s summary is null on the reference dataset, so the one line a \
             flat-tier model reads first carries nothing at all",
            h.name
        );
    }
}

/// The fixture's spread is known exactly, so a cost estimator can be checked
/// rather than believed.
///
/// This is not a smoke test. It is the test that caught a real defect: the first
/// version of the reference dataset drew a proportional wick around
/// `max(open, close)` instead of simulating an intrabar path, and the mid of
/// that wick is not an estimate of the efficient price — so Abdi-Ranaldo came
/// out at half the true spread and EDGE at a third. Both would have shipped,
/// and a cost estimator that is wrong *low* makes every strategy look
/// profitable.
///
/// The tolerance is wide (±40%) on purpose. These estimators are biased by
/// construction and disagree with each other; the claim being made is "recovers
/// the right order of magnitude from data whose answer we know", which is the
/// claim a cost model actually needs. A tight tolerance here would be a tolerance
/// tuned to the current fixture.
#[tokio::test(flavor = "multi_thread")]
async fn the_cost_estimators_recover_the_fixtures_known_spread() {
    // `SPREAD_A` in apps/skill-runner/reference_dataset.py.
    const TRUE_SPREAD_BPS: f64 = 60.0;

    let fixture = fixture_dir();
    let Some(t) = mcp_server_lib::tools::templated::find("estimate_liquidity_cost") else {
        return;
    };
    let (outcome, result) = run_once(t, &payload_for(t, &fixture)).await;
    assert_eq!(outcome, Outcome::Ok, "{outcome:?}");
    let detail = result
        .as_ref()
        .and_then(|r| r.get("detail"))
        .expect("a detail");

    let consensus = detail
        .get("spread_bps")
        .and_then(Value::as_f64)
        .expect("a consensus spread");
    let error = (consensus - TRUE_SPREAD_BPS).abs() / TRUE_SPREAD_BPS;
    assert!(
        error < 0.4,
        "the consensus spread is {consensus:.1} bps against a true {TRUE_SPREAD_BPS} bps          ({:.0}% out). Per-estimator values: {}",
        error * 100.0,
        detail.get("estimators_bps").unwrap_or(&Value::Null)
    );

    // And every estimator that did not decline has to be in the same territory.
    // One that is out by a factor is one that should be declining instead.
    let per = detail
        .get("estimators_bps")
        .and_then(Value::as_object)
        .expect("per-estimator values");
    assert!(!per.is_empty(), "every estimator declined on the fixture");
    for (name, value) in per {
        let bps = value.as_f64().unwrap_or(f64::NAN);
        let out = (bps - TRUE_SPREAD_BPS).abs() / TRUE_SPREAD_BPS;
        assert!(
            out < 0.6,
            "`{name}` reports {bps:.1} bps against a true {TRUE_SPREAD_BPS}. An estimator that              cannot recover a spread from data built to contain it should return None, not a              number a cost model will use."
        );
    }
}

/// A planted lookahead is caught, and a clean pair is not flagged.
///
/// Both halves matter. A checker that flags everything would pass the first
/// assertion alone, and that is not a hypothetical failure mode — the first
/// version of `bar_boundary_check` reported 720 of 720 bars violating on the
/// clean fixture, because a parquet round trip had handed it `datetime64[us]`
/// and it divided by 1e9 to "get seconds". Every timestamp landed in 1970 and
/// every bucket became one bucket. A tool for detecting timestamp defects had
/// one, and only the clean half of this test says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_planted_bar_boundary_leak_is_caught_and_a_clean_pair_is_not() {
    let fixture = fixture_dir();
    let Some(t) = mcp_server_lib::tools::templated::find("bar_boundary_check") else {
        return;
    };

    let call = |slow_file: &str| {
        json!({
            "instrument_id": "BTC-USD",
            "timeframe": "1m",
            "higher_timeframe": "1h",
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-02-01T00:00:00Z",
            "seed": 7,
            "data": {
                "fast": fixture.join("fast.parquet").to_string_lossy(),
                "slow": fixture.join(slow_file).to_string_lossy(),
            }
        })
    };

    let detail_of = |result: Option<Value>| -> Value {
        result
            .and_then(|r| r.get("detail").cloned())
            .expect("a detail")
    };

    let (outcome, clean) = run_once(t, &call("slow.parquet")).await;
    assert_eq!(outcome, Outcome::Ok, "{outcome:?}");
    let clean = detail_of(clean);
    assert_eq!(
        clean.get("n_violations").and_then(Value::as_u64),
        Some(0),
        "the clean aggregate was flagged: {}",
        clean.get("verdict").and_then(Value::as_str).unwrap_or("")
    );
    assert!(
        clean.get("n_checked").and_then(Value::as_u64).unwrap_or(0) > 100,
        "too few bars were actually checked for the clean result to mean anything"
    );

    let (outcome, leaky) = run_once(t, &call("slow_leaky.parquet")).await;
    assert_eq!(outcome, Outcome::Ok, "{outcome:?}");
    let leaky = detail_of(leaky);
    let violations = leaky
        .get("n_violations")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(
        violations > 0,
        "the planted lookahead was not caught at all"
    );

    // The first violating timestamp, which is the acceptance criterion: "caught
    // with its first violating timestamp".
    let first = leaky
        .get("first_violation")
        .and_then(|v| v.get("bar_open"))
        .and_then(Value::as_str)
        .expect("the first violation names its bar");
    assert!(
        first.starts_with("2024-"),
        "the first violation's timestamp is {first}, which is not inside the fixture's window          — the epoch conversion is wrong again"
    );

    // And it recovers *how far early*, not merely that something is off.
    let early = leaky
        .get("first_violation")
        .and_then(|v| v.get("readable_bars_early"))
        .and_then(Value::as_u64)
        .expect("how many bars early");
    assert_eq!(
        early, 20,
        "the fixture plants a 20-minute-early write (LEAKY_MINUTES_EARLY); the tool recovered          {early}"
    );
}

/// The noise band finds the factor that is there, and does not find factors that
/// are not.
///
/// The fixture plants exactly one common factor across twelve strategies with
/// uneven loadings. A covariance tool that reported every eigenvalue as a factor
/// would pass a "does it run" test and be useless; one that reported none would
/// too. Marchenko-Pastur is what separates them, and this asserts the separation
/// rather than the arithmetic.
///
/// It is also the end-to-end check on `SeriesKind::TrialReturns`: the long-format
/// family reaches a template, is pivoted, and the two deliberately short
/// strategies cost periods rather than silently becoming padded zeros.
#[tokio::test(flavor = "multi_thread")]
async fn the_covariance_finds_exactly_the_factor_the_fixture_plants() {
    let fixture = fixture_dir();
    let Some(t) = mcp_server_lib::tools::templated::find("shrunk_covariance") else {
        return;
    };
    let (outcome, result) = run_once(t, &payload_for(t, &fixture)).await;
    assert_eq!(outcome, Outcome::Ok, "{outcome:?}");
    let detail = result
        .as_ref()
        .and_then(|r| r.get("detail"))
        .expect("a detail");

    // Every member arrived. This is not padding out the test: the preamble's
    // `load` sets a timestamp index and drops duplicate timestamps, which is
    // right for bars and catastrophic for a family — every member shares the
    // same timestamps, so it keeps the first strategy and discards the rest,
    // silently. `load_family` exists because of that, and this is the assertion
    // that says whether it is being used.
    assert_eq!(
        detail.get("n_strategies").and_then(Value::as_u64),
        Some(12),
        "the fixture has twelve strategies; a smaller number means the family was collapsed on \
         the way in"
    );

    let significant = detail
        .get("n_significant")
        .and_then(Value::as_u64)
        .expect("a factor count");
    assert_eq!(
        significant, 1,
        "the fixture has one common factor; the tool found {significant}. Reporting every \
         eigenvalue as a factor and reporting none are both failures this catches."
    );

    // Ragged handled by common periods, and said out loud rather than padded.
    let dropped = detail
        .get("n_periods_dropped")
        .and_then(Value::as_u64)
        .expect("a dropped count");
    assert!(
        dropped > 0,
        "two of the fixture's strategies are half length; a run that dropped no periods padded \
         them instead"
    );

    // Raw is reported beside shrunk — AGENT-006 §5.7's requirement, and the
    // reason is that shrinkage biases toward historical structure.
    assert!(detail.get("covariance_raw").is_some());
    assert!(detail.get("covariance_shrunk").is_some());
    let shrinkage = detail
        .get("shrinkage")
        .and_then(Value::as_f64)
        .expect("an intensity");
    assert!(
        (0.0..=1.0).contains(&shrinkage),
        "shrinkage {shrinkage} is outside [0, 1]"
    );
}

/// The weight cap holds against an input built to break it.
///
/// AGENT-006 §5.7's acceptance is "`allocate_book` cannot exceed the weight cap
/// by any input", and the well-behaved family does not test that: its largest
/// weight is 0.15 against a cap of 0.25, so the clip is a branch no run takes.
/// `family_concentrated.parquet` gives one strategy a hundredth of the variance
/// of the rest, which is what a barely-trading strategy actually looks like and
/// is exactly what a variance-minimising optimiser wants all of.
///
/// The property under test is not "the number is 0.25". It is that the cap is
/// applied *after* the objective and so cannot be negotiated with: an optimiser
/// handed a target it cannot reach under a constraint finds the corner of the
/// feasible set, and the corner is the cap on the most in-sample-flattering
/// strategy.
#[tokio::test(flavor = "multi_thread")]
async fn the_weight_cap_survives_a_family_built_to_concentrate() {
    let fixture = fixture_dir();
    let Some(t) = mcp_server_lib::tools::templated::find("allocate_book") else {
        return;
    };

    let payload = json!({
        "experiment_id": "00000000-0000-0000-0000-000000000001",
        "objective": "minimum_variance",
        "seed": 7,
        "data": {
            "family": fixture.join("family_concentrated.parquet").to_string_lossy(),
        }
    });

    let (outcome, result) = run_once(t, &payload).await;
    assert_eq!(outcome, Outcome::Ok, "{outcome:?}");
    let detail = result
        .as_ref()
        .and_then(|r| r.get("detail"))
        .expect("a detail");

    let cap = detail.get("cap").and_then(Value::as_f64).expect("a cap");
    let max = detail
        .get("max_weight")
        .and_then(Value::as_f64)
        .expect("a max weight");
    assert!(
        max <= cap + 1e-9,
        "a weight of {max} exceeded the cap of {cap} — the cap was negotiable"
    );
    assert!(
        detail.get("n_capped").and_then(Value::as_u64).unwrap_or(0) > 0,
        "this family is built so the objective wants a concentrated book; nothing was capped, \
         so the clip did not run and this test proves nothing"
    );

    // Every individual weight, not just the reported maximum — a maximum is a
    // number the template computes, and this checks the weights themselves.
    let weights = detail
        .get("weights")
        .and_then(Value::as_object)
        .expect("the weights");
    for (name, w) in weights {
        let w = w.as_f64().unwrap_or(f64::NAN);
        assert!(
            w <= cap + 1e-9,
            "`{name}` holds {w}, above the cap of {cap}"
        );
    }
    let total: f64 = weights.values().filter_map(Value::as_f64).sum();
    assert!(
        (total - 1.0).abs() < 1e-6,
        "the capped weights sum to {total}, not one — the redistribution lost or created book"
    );
}
