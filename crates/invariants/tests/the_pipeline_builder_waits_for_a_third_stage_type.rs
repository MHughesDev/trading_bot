//! The visual pipeline builder is deferred until a third pipeline-stage type exists
//! (Set R-b RB-5.10; ADR-P5-03). This is what keeps "not yet" a *measured* fact instead of
//! a sentence in a plan.
//!
//! ```bash
//! cargo test -j 2 -p invariants --test the_pipeline_builder_waits_for_a_third_stage_type
//! ```
//!
//! ADR-P5-03 defers the builder because the strategy builder and guided mode cover the
//! need while there are fewer than three kinds of stage to compose. A task closed as "not
//! yet" goes stale the day a third kind appears and nobody is looking. So this counts the
//! stage-type identifiers in the source trees that would hold one, and fails the build when
//! the count reaches three, naming the files: that is the signal to re-open RB-5.10.
//!
//! On 2026-10-04 the count is **0**: no `pipeline_stage_type`, `PipelineStageKind` or the
//! like exists in `crates/`, `apps/` (Python), `frontend/src` or `migrations/` (the 2026-09
//! Meridian rebuild removed the old Pipelines page, and the `/mlops` suite composes models,
//! not stages).

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if matches!(name.as_str(), "target" | "node_modules" | "dist" | ".git" | "__pycache__" | "tests") {
                continue;
            }
            walk(&p, out);
        } else if matches!(p.extension().and_then(|x| x.to_str()), Some("rs" | "py" | "ts" | "tsx" | "sql")) {
            out.push(p);
        }
    }
}

/// Distinct stage-type identifiers: `PipelineStageKind`, `pipeline_stage_type`, `StageType`
/// in a pipeline context. Distinctness is by the identifier's normalised spelling, so one
/// type used in fifty places counts once.
fn stage_type_identifiers() -> Vec<(String, String)> {
    let re = Regex::new(r"(?i)\b(pipeline_?stage_?(?:type|kind)s?|stage_?(?:type|kind)s?_of_pipeline)\b").unwrap();
    let mut files = Vec::new();
    for d in ["crates", "apps", "frontend/src", "migrations"] {
        walk(&root().join(d), &mut files);
    }
    let mut found = Vec::new();
    for f in files {
        // This file names the identifiers in its own docs.
        if f.ends_with("the_pipeline_builder_waits_for_a_third_stage_type.rs") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&f) else { continue };
        for m in re.find_iter(&text) {
            found.push((m.as_str().to_ascii_lowercase().replace('_', ""), f.display().to_string()));
        }
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn the_builder_stays_deferred_until_three_stage_types_exist() {
    let found = stage_type_identifiers();
    assert!(
        found.len() < 3,
        "{} pipeline-stage type identifiers now exist, so ADR-P5-03's trigger may have fired; re-open RB-5.10 \
         and decide whether the visual builder is now worth building: {found:#?}",
        found.len()
    );
}

#[test]
fn the_scan_reads_the_trees_it_claims_to() {
    // A scan that found no files would pass the test above for the wrong reason.
    let mut files = Vec::new();
    for d in ["crates", "apps", "frontend/src", "migrations"] {
        walk(&root().join(d), &mut files);
    }
    assert!(files.len() > 500, "only {} source files found; the walk has broken", files.len());
    assert!(files.iter().any(|f| f.ends_with("crates/api/src/lib.rs") || f.ends_with("crates\\api\\src\\lib.rs")));
}
