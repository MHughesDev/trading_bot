//! Set T T-3.4 — no path exists to a capital deployment today, and adding one is
//! a deliberate, reviewed change rather than a configuration discovery.
//!
//! The written half is `docs/governance/capital-gate.md`. This is the half the
//! compiler and the build enforce:
//!
//! 1. the venue class has one value, held by an exhaustive `match` that stops
//!    compiling the day a second one is added;
//! 2. every migration that gives a deployment table a venue says `paper`, and none
//!    says anything else;
//! 3. every place that records or declares a deployment is on a reviewed list, and
//!    the one recording statement hardcodes the paper venue, so no caller can pass
//!    one in;
//! 4. the deployment record has no venue field for a caller to fill.
//!
//! A new entry point fails (3) until somebody reads it and adds it to the list —
//! which is the review.

use std::fs;
use std::path::{Path, PathBuf};

use ledger::deployment_path::VenueClass;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if matches!(name.as_str(), "target" | "tests" | "node_modules" | ".git") {
                continue;
            }
            rust_sources(&p, out);
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

/// 1. The tripwire. Adding a variant makes this fail to compile: handle it here
///    only after the checklist in `docs/governance/capital-gate.md` holds.
#[test]
fn the_only_venue_class_is_paper_and_a_second_one_stops_this_compiling() {
    fn describe(v: VenueClass) -> &'static str {
        match v {
            VenueClass::Paper => "paper",
        }
    }
    assert_eq!(describe(VenueClass::Paper), "paper");
}

/// 2. The database half: every venue constraint on a deployment table names paper
///    and nothing else, and nothing later in the migration history widens them.
#[test]
fn every_deployment_table_constrains_its_venue_to_paper() {
    let d = read("migrations/0142_strategy_deployment.sql");
    assert!(d.contains("environment       TEXT NOT NULL CHECK (environment IN ('paper'))"));
    assert!(d.contains("venue             TEXT NOT NULL CHECK (venue IN ('platform_paper'))"));
    let p = read("migrations/0180_deployment_path.sql");
    assert_eq!(
        p.matches("CHECK (venue_class = 'paper')").count(),
        2,
        "the proposal and the authorisation each name the paper class"
    );

    for entry in fs::read_dir(root().join("migrations")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".sql") {
            continue;
        }
        let sql = fs::read_to_string(entry.path()).unwrap();
        for table in [
            "strategy_deployment",
            "deployment_proposal",
            "deployment_authorisation",
        ] {
            if !sql.contains(table) {
                continue;
            }
            for (i, line) in sql.lines().enumerate() {
                let l = line.to_lowercase();
                let widens = (l.contains("venue") || l.contains("environment"))
                    && (l.contains("'live'") || l.contains("'real'") || l.contains("'capital'"));
                assert!(
                    !widens,
                    "{name}:{} widens a deployment venue past paper: {line}. Opening capital is \
                     a reviewed change that starts at docs/governance/capital-gate.md",
                    i + 1
                );
            }
        }
    }
}

/// Count calls with a leading boundary, so `deregister_v3(` is not `register_v3(`.
fn calls(src: &str, needle: &str) -> usize {
    src.match_indices(needle)
        .filter(|(i, _)| {
            let before = src[..*i].chars().last();
            needle.starts_with('.') || !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
        .count()
}

/// 3. Every deployment entry point is on a list somebody reviewed.
#[test]
fn every_deployment_entry_point_is_reviewed_and_records_a_paper_deployment() {
    // (file, why it is paper)
    const REVIEWED: &[(&str, &str)] = &[
        (
            "crates/api/src/routes/deployments.rs",
            "POST /api/deployments: records through storage::postgres::deployments::start, whose SQL hardcodes paper",
        ),
        (
            "apps/platform/src/allocator.rs",
            "a selection's member deployments: recorded through the same start(), under the parent's authority",
        ),
        (
            "apps/platform/src/live_deployments.rs",
            "resumes recorded deployments at boot: only rows the paper-only table holds",
        ),
        (
            "apps/platform/src/live_runner.rs",
            "the runner for a registered deployment; it records nothing and has no venue to choose",
        ),
        (
            "crates/strategy-runtime/src/live.rs",
            "the in-process registry; it names no venue, and the runner it feeds is the paper one",
        ),
    ];
    let mut files = Vec::new();
    rust_sources(&root().join("crates"), &mut files);
    rust_sources(&root().join("apps"), &mut files);
    let mut found = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // This module's siblings stop deployments and read them; they start none.
        let src = fs::read_to_string(&f).unwrap_or_default();
        let n = calls(&src, "deployments::start(")
            + calls(&src, ".register_v3(")
            + calls(&src, ".register_selection(");
        if n > 0 {
            found.push(rel);
        }
    }
    found.sort();
    for f in &found {
        assert!(
            REVIEWED.iter().any(|(r, _)| r == f),
            "{f} starts or declares a deployment and is not on the reviewed list in \
             crates/api/tests/no_capital_path.rs. Read it; if it can only reach a paper venue, \
             add it with the reason. If it cannot, it is a capital path: see \
             docs/governance/capital-gate.md"
        );
    }
    assert!(
        found.iter().any(|f| f.ends_with("routes/deployments.rs")),
        "the scan found nothing in the deployments route; the test is no longer looking"
    );

    // The one recording statement fixes the venue; there is no parameter for it.
    let storage = read("crates/storage/src/postgres/deployments.rs");
    assert!(
        storage
            .contains("VALUES ($1,$2,$3,$4,$5,$6,'paper','platform_paper',$7,$8,$9,$10,$11,$12)"),
        "the recording statement no longer hardcodes paper / platform_paper"
    );
}

/// 4. A caller has no field to put a venue in.
#[test]
fn the_deployment_record_has_no_venue_for_a_caller_to_choose() {
    let storage = read("crates/storage/src/postgres/deployments.rs");
    let start = storage
        .find("pub struct NewDeployment")
        .expect("NewDeployment");
    let body = &storage[start..start + storage[start..].find("\n}").unwrap()];
    assert!(
        !body.contains("venue") && !body.contains("environment"),
        "NewDeployment grew a venue or environment field:\n{body}"
    );
}
