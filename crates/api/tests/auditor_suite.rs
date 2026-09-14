//! AE-02: the auditor suite runs in CI with 100% catch and 0 false rejections.
//!
//! No agent, no model, no credential, no network — which is the point. This is the
//! one part of AGENT-004 that can gate a pull request, so it has to be the part that
//! runs in seconds on a laptop with nothing else up.
//!
//! The two assertions at the bottom are the requirement verbatim. Everything above
//! them exists so that a failure says *which* guard stopped working and on which
//! fixture, rather than "assertion failed".

use std::path::PathBuf;

use api::auditor::{load_dir, run_suite, Check, Fixture};

fn fixtures_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is crates/api; the fixtures live at the repository root
    // so that they are editable by someone who is not reading Rust.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("evals")
        .join("auditor")
}

fn load() -> Vec<Fixture> {
    load_dir(&fixtures_dir()).expect("the auditor fixtures must load")
}

#[test]
fn the_auditor_suite_catches_every_violation_and_rejects_no_clean_twin() {
    let fixtures = load();
    let card = run_suite(&fixtures);

    // A detailed failure report, printed before the assertions so that a CI log
    // shows what broke rather than only that something did.
    for o in &card.outcomes {
        if o.passed() {
            continue;
        }
        eprintln!("FIXTURE FAILED: {} ({:?})", o.id, o.check);
        if !o.caught {
            eprintln!(
                "  the violating input was NOT caught. raised: {:?}; expected but missing: {:?}",
                o.violating_codes, o.missing_codes
            );
        }
        if !o.clean_accepted {
            eprintln!(
                "  the CLEAN twin was rejected with {:?} — a check that rejects honest \
                 work gets routed around, and then catches nothing at all",
                o.clean_codes
            );
        }
    }

    assert_eq!(
        card.caught,
        card.fixtures,
        "catch rate is {:.1}% ({} of {}); the target is 100%",
        card.catch_rate() * 100.0,
        card.caught,
        card.fixtures
    );
    assert_eq!(
        card.false_rejections, 0,
        "{} clean twin(s) were rejected; the target is 0",
        card.false_rejections
    );
    assert!(card.passed());
}

/// Every guard named in AGENT-004 §4 must have at least one fixture.
///
/// Without this, the suite passes trivially the moment a guard's fixtures are
/// deleted — which is exactly what someone does when a guard starts failing and the
/// deadline is today.
#[test]
fn every_check_is_exercised_by_at_least_one_fixture() {
    let fixtures = load();
    let required = [
        Check::Gate0,
        Check::FinalReport,
        Check::LeakageLint,
        Check::SkillLint,
        Check::DataGrade,
        Check::RevisionPinning,
        Check::PredictionOverlap,
        Check::Truncation,
    ];
    for check in required {
        assert!(
            fixtures.iter().any(|f| f.check == check),
            "{check:?} has no fixture; AGENT-004 §4 lists it as covered"
        );
    }
}

/// The suite must be big enough to mean something, and every fixture must say why
/// it exists. `why` is read by a human deciding whether the rule is still right.
#[test]
fn the_suite_is_substantive_and_self_documenting() {
    let fixtures = load();
    assert!(
        fixtures.len() >= 15,
        "only {} fixtures; AGENT-004 §4 lists nine violation families and each needs \
         a clean twin",
        fixtures.len()
    );
    let mut ids: Vec<&str> = fixtures.iter().map(|f| f.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate fixture ids");

    for f in &fixtures {
        assert!(
            f.why.len() >= 60,
            "fixture {} has no real rationale; say what goes wrong in production if \
             this guard misses",
            f.id
        );
        assert_ne!(
            f.violating.to_string(),
            f.clean.to_string(),
            "fixture {} has identical halves, so it proves nothing",
            f.id
        );
    }
}
