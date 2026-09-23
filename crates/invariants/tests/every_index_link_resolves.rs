//! Every relative link in the navigation documents resolves (Set S task S-0.5).
//!
//! # Why a test and not an audit
//!
//! S-0.5 asks that "no index row points at a missing file". That is a list
//! maintained by looking, and a list maintained by looking is wrong by exactly
//! the amount nobody looked — which is the argument
//! `the_desk_calls_routes_that_exist` and `every_client_path_is_served` already
//! make for the two API surfaces. A broken index link is the same defect in the
//! documentation: a reader follows it, finds nothing, and concludes the thing
//! does not exist.
//!
//! It was **clean** when this was written (84 of 84). That is the point of
//! adding it now rather than when it breaks: the cheap moment to install a check
//! is while it passes, because installing one that already fails means fixing
//! and installing at once and never learning which change did it.
//!
//! # What it does not check
//!
//! Anchors (`#section`) are stripped rather than resolved — a heading rename is
//! a real break and finding it needs a Markdown parse this does not do, so
//! claiming otherwise would be worse than the gap.
//!
//! And it checks the *navigation* documents, not every file in `docs/`. A link
//! inside a spec is between two things a reader is already holding; an index row
//! is the thing they use to find anything at all, so it is where a break costs
//! most.

use std::path::{Path, PathBuf};

/// The documents a reader navigates from.
const INDEXES: &[&str] = &[
    "docs/plans/plan-sets/set-S/MASTER.md",
    "docs/plans/README.md",
    "docs/adr/README.md",
    "docs/FOLDER_MAP.md",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Relative link targets in a Markdown document, anchors stripped.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        // `](` opens a target. Written by hand rather than with a regex crate:
        // this crate has `regex` as a dev-dependency, but the grammar here is
        // two characters and a bracket depth of zero.
        if bytes[i] == ']' && i + 1 < bytes.len() && bytes[i + 1] == '(' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && bytes[j] != ')' {
                j += 1;
            }
            if j < bytes.len() {
                let target: String = bytes[start..j].iter().collect();
                let target = target
                    .split('#')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !target.is_empty() {
                    out.push(target);
                }
                i = j;
            }
        }
        i += 1;
    }
    out
}

fn is_external(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://") || target.starts_with("mailto:")
}

/// Resolve a link the way a reader's viewer would: relative to the document.
fn resolve(doc: &Path, target: &str) -> PathBuf {
    let decoded = target.replace("%20", " ");
    doc.parent().unwrap_or_else(|| Path::new(".")).join(decoded)
}

#[test]
fn every_relative_link_in_the_navigation_documents_resolves() {
    let root = repo_root();
    let mut checked = 0usize;
    let mut broken: Vec<String> = Vec::new();

    for index in INDEXES {
        let doc = root.join(index);
        assert!(
            doc.exists(),
            "`{index}` is itself missing, so the list of documents a reader navigates from is \
             wrong before any link in it is"
        );
        let text = std::fs::read_to_string(&doc).expect("a readable index");
        for target in links(&text) {
            if is_external(&target) {
                continue;
            }
            checked += 1;
            let path = resolve(&doc, &target);
            if !path.exists() {
                broken.push(format!("{index} -> {target}"));
            }
        }
    }

    assert!(
        checked > 50,
        "the link scan found only {checked} links; it has stopped reading these documents"
    );
    assert!(
        broken.is_empty(),
        "index rows pointing at nothing — a reader follows one of these and concludes the thing \
         does not exist: {broken:#?}"
    );
}

/// The scanner finds what it is meant to find.
///
/// A scan that silently matched nothing would pass the test above for the wrong
/// reason, which is the failure mode every source-scanning check in this
/// repository has to guard against separately.
#[test]
fn the_scanner_reads_the_shapes_these_documents_use() {
    let found = links(
        "see [the plan](./plan-sets/set-S/MASTER.md) and [an adr](../adr/0034-x.md#section) \
         and [the web](https://example.com) and [spaced](a%20b.md)",
    );
    assert_eq!(
        found,
        vec![
            "./plan-sets/set-S/MASTER.md",
            "../adr/0034-x.md",
            "https://example.com",
            "a%20b.md",
        ]
    );
    // The anchor is stripped rather than resolved, and the test says so where a
    // reader of the failure would look.
    assert!(!found[1].contains('#'));
    assert!(is_external(&found[2]));
}
