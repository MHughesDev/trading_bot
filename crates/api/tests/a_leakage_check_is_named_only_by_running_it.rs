//! `checks_run` may not name a check the leakage audit did not run.
//!
//! Gate 2 (`backtest::gates::evaluators::gate_2_leakage_suite`) reads that array
//! and refuses a run missing any required check, on the stated ground that "a
//! check that did not run is not a check that passed". The array is therefore
//! the gate's only evidence that a check happened, and a name in it that nothing
//! earned is a lie the gate cannot see through.
//!
//! The audit used to build the array by hand beside the calls, and the two had
//! drifted: `causal_access` was named by a `push` with no call anywhere in the
//! file, and `snapshot_reproducibility` was named before the loop that runs it.
//! `api::leakage_audit::pass` now makes the name a *product* of the run —
//! `checks_run` is private to that child module and `Tally::run` is the only
//! append site — and these tests hold that property from the outside.

use api::leakage_audit::{Pass, Tally};
use features::leakage::inject::{plant, Leak};
use features::leakage::Check;

// ───────────────────────────────────────────────────────────────────────────
// Fixtures: a clean frame, built through the same aligned builder the dataset
// manager uses, so the frame under test is shaped like a real one.
// ───────────────────────────────────────────────────────────────────────────

const STEP_NS: i64 = 60_000_000_000;
const PERMUTATIONS: usize = 8;

/// Full-period LCG, not `i % k`: a sawtooth is structure a linear probe can read
/// straight off, and this frame has to be clean for the pass/decline split to
/// mean anything.
fn noise(seed: u64) -> f64 {
    let mut h = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((h ^ (h >> 31)) % 10_007) as f64 / 10_007.0 - 0.5
}

fn bars(n: usize) -> Vec<features::BarObs> {
    (0..n)
        .map(|i| {
            let c = 100.0 * (1.0 + noise(i as u64 + 1) * 0.01);
            let ts = i as i64 * STEP_NS;
            features::BarObs {
                ts_ns: ts,
                knowledge_ns: ts,
                open: c,
                high: c * 1.001,
                low: c * 0.999,
                close: c,
                volume: 1.0,
                quality: dataplane::quality::QualityFlags::NONE,
            }
        })
        .collect()
}

fn feature_names() -> Vec<String> {
    vec!["close".to_string(), "ema_7".to_string()]
}

fn clean_frame(rows: usize) -> features::TrainingFrame {
    features::build_aligned_training_frame(&bars(rows), &feature_names(), 1, STEP_NS)
}

fn pipeline() -> dataplane::split::EmbargoInputs {
    features::walk_forward::embargo_inputs(&feature_names(), 1)
}

/// A geometry three expanding folds fit inside a ~2 000-row frame.
fn spec() -> domain::model_def::cv::WalkForwardSpec {
    domain::model_def::cv::WalkForwardSpec {
        mode: domain::model_def::cv::WindowMode::Expanding,
        folds: 3,
        train_bars: 1_000,
        cal_bars: 80,
        test_bars: 150,
        purge_bars: 2,
        embargo_bars: 2,
    }
}

fn random_label_pass<'a>(
    frame: &'a features::TrainingFrame,
    spec: &'a domain::model_def::cv::WalkForwardSpec,
    pipeline: &'a dataplane::split::EmbargoInputs,
) -> Pass<'a> {
    Pass::RandomLabel {
        frame,
        spec,
        pipeline,
        permutations: PERMUTATIONS,
        seed: 0xfeed_beef,
    }
}

// ───────────────────────────────────────────────────────────────────────────
// The property
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn a_tally_names_nothing_until_a_pass_runs() {
    let tally = Tally::default();
    assert!(
        tally.checks_run().is_empty(),
        "a tally that has run nothing claimed {:?}",
        tally.checks_run()
    );
    assert!(tally.report().findings.is_empty());
    assert!(tally.random_label().is_none());
}

/// The pairing test, and the one that would have caught the original defect.
///
/// Each pass is given input that *forces* a finding, and the name it earned is
/// compared against the `check` the findings themselves carry. A pass wired to
/// the wrong function — which is what "push the name beside the call" degrades
/// into — produces findings under a different check than the name it returned,
/// and fails here.
#[test]
fn each_pass_is_named_by_the_check_whose_findings_it_produced() {
    let clean = clean_frame(2_000);
    assert!(
        clean.row_count() >= 32,
        "fixture too short for the frame screens: {} rows",
        clean.row_count()
    );

    let target_leak = plant(&clean, Leak::TargetInFeature);
    let scaling_leak = plant(&clean, Leak::FullSampleNormalization);

    let cases: Vec<(&str, Pass<'_>, Check)> = vec![
        (
            "snapshot reproducibility",
            Pass::SnapshotReproducibility {
                dataset_id: "ds_under_test",
                recorded: "sha256:1111",
                rebuilt: "sha256:2222",
            },
            Check::SnapshotReproducibility,
        ),
        (
            "target correlation",
            Pass::TargetCorrelation(&target_leak),
            Check::TargetCorrelation,
        ),
        (
            "full-sample normalization",
            Pass::FullSampleNormalization(&scaling_leak),
            Check::FullSampleNormalization,
        ),
    ];

    for (label, pass, expected) in cases {
        let passed = pass.run();
        assert_eq!(
            passed.earned(),
            Some(expected),
            "{label}: the pass earned the wrong name"
        );
        assert!(
            !passed.report().findings.is_empty(),
            "{label}: a planted leak produced no finding, so this case proves nothing \
             about the pairing — fix the fixture, not the assertion"
        );
        for f in &passed.report().findings {
            assert_eq!(
                f.check, expected,
                "{label}: the pass claimed `{expected}` but its findings are `{}` — \
                 the name and the work have drifted apart",
                f.check
            );
        }
    }
}

/// The random-label pass has no leak that can be planted in a frame — the leak
/// it detects lives in the harness — so its pairing is held by the statistics
/// instead: only that pass returns a `RandomLabelResult`, and the result carries
/// one permutation per permutation requested. An arm wired to some other check
/// cannot produce those.
#[test]
fn only_the_random_label_pass_carries_the_statistics_that_prove_it_permuted() {
    let frame = clean_frame(2_000);
    let (spec, pipeline) = (spec(), pipeline());

    let passed = random_label_pass(&frame, &spec, &pipeline).run();
    assert_eq!(passed.earned(), Some(Check::RandomLabel));
    let r = passed
        .random_label()
        .expect("the random-label pass returns its statistics");
    assert_eq!(
        r.permuted_sharpes.len(),
        PERMUTATIONS,
        "the pass claimed `random_label` on {} permutations",
        r.permuted_sharpes.len()
    );
    assert_eq!(r.permuted_t_stats.len(), PERMUTATIONS);
    assert!(
        r.permuted_max_abs_t.is_finite(),
        "an absent measurement is not a zero: the permuted maximum |t| is {}",
        r.permuted_max_abs_t
    );

    for other in [
        Pass::SnapshotReproducibility {
            dataset_id: "ds",
            recorded: "sha256:1111",
            rebuilt: "sha256:1111",
        },
        Pass::TargetCorrelation(&frame),
        Pass::FullSampleNormalization(&frame),
    ] {
        assert!(
            other.run().random_label().is_none(),
            "a pass that is not the random-label test returned random-label statistics"
        );
    }
}

/// A check that ran and declined earns no name. `random_label` says so itself by
/// permuting nothing when the frame cannot carry the fold geometry; naming it
/// anyway would be the same false claim one layer down.
#[test]
fn a_random_label_pass_that_could_not_build_folds_claims_nothing() {
    let short = clean_frame(64);
    let (spec, pipeline) = (spec(), pipeline());

    let passed = random_label_pass(&short, &spec, &pipeline).run();
    let r = passed
        .random_label()
        .expect("the pass still reports what it found");
    assert!(
        r.permuted_sharpes.is_empty(),
        "fixture is no longer short enough to make the fold geometry unbuildable"
    );
    assert_eq!(
        passed.earned(),
        None,
        "a random-label pass that permuted nothing claimed to have run"
    );
    assert!(
        !passed.report().findings.is_empty(),
        "declining is itself information and must still reach the report"
    );

    let mut tally = Tally::default();
    tally.run(random_label_pass(&short, &spec, &pipeline));
    assert!(
        tally.checks_run().is_empty(),
        "the tally recorded {:?} for a pass that measured nothing",
        tally.checks_run()
    );
}

/// Findings can arrive without a name; a name cannot arrive without findings'
/// worth of work. The fold-geometry screen is the real instance: it runs, its
/// findings are recorded, and it has no `Check` of its own to claim.
#[test]
fn findings_recorded_outside_a_pass_name_no_check() {
    let mut tally = Tally::default();
    tally.add_findings(features::leakage::fold_geometry(
        &features::leakage::inject::overlapping_folds(400, 3),
    ));
    assert!(
        !tally.report().findings.is_empty(),
        "overlapping folds are a finding"
    );
    assert!(
        tally.checks_run().is_empty(),
        "work with no name of its own claimed {:?}",
        tally.checks_run()
    );
}

/// The tally holds exactly the names its passes earned, once each, in order.
#[test]
fn a_tally_names_exactly_the_passes_that_earned_a_name() {
    let frame = clean_frame(2_000);
    let (spec, pipeline) = (spec(), pipeline());

    let mut tally = Tally::default();
    assert!(tally.checks_run().is_empty());

    tally.run(Pass::SnapshotReproducibility {
        dataset_id: "ds_a",
        recorded: "sha256:1111",
        rebuilt: "sha256:1111",
    });
    assert_eq!(tally.checks_run(), ["snapshot_reproducibility"]);

    // The snapshot screen runs once per dataset; the array is a set of names,
    // not a tally of calls.
    tally.run(Pass::SnapshotReproducibility {
        dataset_id: "ds_b",
        recorded: "sha256:3333",
        rebuilt: "sha256:3333",
    });
    assert_eq!(tally.checks_run(), ["snapshot_reproducibility"]);

    tally.run(Pass::TargetCorrelation(&frame));
    tally.run(Pass::FullSampleNormalization(&frame));
    tally.run(random_label_pass(&frame, &spec, &pipeline));

    assert_eq!(
        tally.checks_run(),
        [
            "snapshot_reproducibility",
            "target_correlation",
            "full_sample_normalization",
            "random_label",
        ]
    );
    assert!(tally.random_label().is_some());
}

/// The regression itself, stated as a fact about the gate.
///
/// `features::leakage::causal_access` needs the dataset's OHLCV rows and
/// `DatasetManager` exposes only the assembled frame, so this audit cannot run
/// the causal guard and therefore does not name it. Gate 2 sees it missing,
/// which is the truth.
///
/// **Ordering constraint for whoever closes `cv_wf_gap` and
/// `synthetic_injection`.** Those two are genuinely absent and are what holds
/// Gate 2 at inconclusive today. Adding them *first* would leave `causal_access`
/// as the only missing name — and if the old hand-written `push` were ever
/// restored, Gate 2 would pass on a leakage suite in which the causal guard
/// never ran. Wire `causal_access` to a real call before adding either of the
/// other two. This assertion is the tripwire: it fails the moment that ordering
/// is broken in either direction, so read it before editing it.
#[test]
fn gate_2_is_told_the_causal_guard_did_not_run_because_it_did_not() {
    let frame = clean_frame(2_000);
    let (spec, pipeline) = (spec(), pipeline());

    let mut tally = Tally::default();
    tally.run(Pass::SnapshotReproducibility {
        dataset_id: "ds",
        recorded: "sha256:1111",
        rebuilt: "sha256:1111",
    });
    tally.run(Pass::TargetCorrelation(&frame));
    tally.run(Pass::FullSampleNormalization(&frame));
    tally.run(random_label_pass(&frame, &spec, &pipeline));

    let claimed: Vec<&str> = tally.checks_run().iter().map(String::as_str).collect();
    assert!(
        !claimed.contains(&"causal_access"),
        "the audit claimed the causal guard; nothing here calls \
         `features::leakage::causal_access`"
    );

    let missing: Vec<&str> = api::gate_worker::LEAKAGE_CHECKS
        .iter()
        .copied()
        .filter(|c| !claimed.contains(c))
        .collect();
    assert_eq!(
        missing,
        ["causal_access", "cv_wf_gap", "synthetic_injection"],
        "the set of checks Gate 2 will report missing has changed; see this test's \
         doc comment before updating it — `causal_access` must not be the last one left"
    );
}
