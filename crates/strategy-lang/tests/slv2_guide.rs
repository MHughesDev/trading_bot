//! The SLv2 guide is small enough to fetch a section at a time, and every anchor
//! the platform points at resolves (S-5.13; AGENT-011 §6, §8.8, AC-8).
//!
//! The acceptance, in its own terms: *"each SLv2 section is ≤ 4 KB (so a 7B
//! fetches one at a time) and every `see` anchor from S-5.3 resolves."*
//!
//! The three things that go wrong with a guide are tested instead of promised:
//! a section grows past what a small tier can read, a diagnostic points at a
//! section that was renamed, and the worked examples stop compiling. The last is
//! the one that does the damage — a guide's example is read as the language.
//!
//! Regenerate the half that is rendered from the registries with
//! `SLV2_GUIDE_WRITE=1 cargo test -p strategy-lang --test slv2_guide`.

use std::collections::BTreeSet;
use std::path::PathBuf;

use strategy_lang::guide::{self, MAX_SECTION_BYTES};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(crate_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn the_generated_half_is_the_registries_and_the_skeletons() {
    let path = crate_dir().join("docs/slv2-guide.md");
    let on_disk = std::fs::read_to_string(&path).expect("the guide is in the repository");
    let fresh = guide::with_generated(&on_disk);
    if std::env::var_os("SLV2_GUIDE_WRITE").is_some() {
        std::fs::write(&path, &fresh).expect("write the guide");
        return;
    }
    assert!(
        on_disk == fresh,
        "docs/slv2-guide.md is out of date with the feature registry or the skeletons; \
         regenerate with SLV2_GUIDE_WRITE=1"
    );
}

/// The acceptance's first clause. Heading included: that is what is served.
#[test]
fn every_section_fits_in_four_kilobytes() {
    let sections = guide::sections();
    assert!(sections.len() >= 20, "only {} sections", sections.len());
    let too_big: Vec<(String, usize)> = sections
        .iter()
        .map(|s| (s.anchor.clone(), s.text().len()))
        .filter(|(_, n)| *n > MAX_SECTION_BYTES)
        .collect();
    assert!(
        too_big.is_empty(),
        "sections over {MAX_SECTION_BYTES} bytes: {too_big:?}"
    );
}

#[test]
fn no_anchor_names_two_sections() {
    let mut seen = BTreeSet::new();
    for s in guide::sections() {
        for a in s.anchors() {
            assert!(seen.insert(a.to_string()), "`{a}` names two sections");
        }
    }
}

/// Every anchor `needle` introduces in `text`, up to the closing quote.
fn scan(out: &mut BTreeSet<String>, text: &str, needle: &str) {
    let mut rest = text;
    while let Some(i) = rest.find(needle) {
        let after = &rest[i + needle.len()..];
        let end = after.find('"').unwrap_or(after.len());
        let anchor = &after[..end];
        if !anchor.is_empty() {
            out.insert(anchor.to_string());
        }
        rest = &after[end..];
    }
}

/// Every `guide#…` anchor the language, the validator and the tools can emit.
fn emitted_anchors() -> BTreeSet<String> {
    let mut out = BTreeSet::new();

    // SLv2's own diagnostics: `.with_see("guide#…")`. `src/v3` is SLv3's, with
    // its own guide, and is not a directory this scan enters.
    for entry in std::fs::read_dir(crate_dir().join("src")).expect("src") {
        let p = entry.expect("entry").path();
        if p.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&p).expect("read");
            // The scan reads string literals, so a comment that names a section
            // it cannot reach is counted too — which is right: that is a stale
            // comment.
            scan(&mut out, &text, "with_see(\"guide#");
            scan(&mut out, &text, "see: Some(\"guide#");
        }
    }
    // The validator's teaching table, as data and as source.
    for t in strategy_validator::teach::TEACHINGS {
        if let Some(see) = t.see {
            out.insert(see.trim_start_matches("guide#").to_string());
        }
    }
    let validator_src =
        std::fs::read_dir(crate_dir().join("../strategy-validator/src")).expect("validator src");
    for entry in validator_src {
        let p = entry.expect("entry").path();
        if p.extension().is_some_and(|e| e == "rs") {
            scan(
                &mut out,
                &std::fs::read_to_string(&p).expect("read"),
                "see: Some(\"guide#",
            );
        }
    }
    // The tools that point a caller at the guide.
    scan(
        &mut out,
        &read("../mcp-server/src/tools/strategy_lang.rs"),
        "\"guide#",
    );
    out
}

/// The acceptance's second clause. A diagnostic that names a section that does
/// not exist sends a small model to `get_slv2_guide` for an error.
#[test]
fn every_see_anchor_resolves_to_a_section() {
    let anchors = emitted_anchors();
    assert!(
        anchors.len() >= 25,
        "only {} anchors found, so the scan is no longer finding what it scans for: {anchors:?}",
        anchors.len()
    );
    let missing: Vec<&String> = anchors
        .iter()
        .filter(|a| guide::section(a).is_none())
        .collect();
    assert!(missing.is_empty(), "no section answers to: {missing:?}");
}

/// The first line of the file is what a diagnostic's `see` is for, so the six
/// the acceptance names (AGENT-011 §5) are checked by name as well.
#[test]
fn the_anchors_the_validator_teaches_are_there() {
    for a in [
        "features",
        "exits.units",
        "nodes",
        "risk",
        "warmup",
        "preview_signals",
        "expressions",
        "exits",
    ] {
        assert!(guide::section(a).is_some(), "`{a}`");
    }
}

/// A full strategy in a code fence compiles. The guide's examples are read as
/// the language, so one that does not compile teaches something false.
#[test]
fn every_worked_strategy_in_the_guide_compiles() {
    let mut n = 0;
    for s in guide::sections() {
        let mut in_fence = false;
        let mut block = String::new();
        for line in s.body.lines() {
            if line.starts_with("```") {
                // A document opens with its header, possibly after comments.
                let opens_with_header = block
                    .lines()
                    .find(|l| !l.trim_start().starts_with('#'))
                    .is_some_and(|l| l.starts_with("strategy "));
                if in_fence && opens_with_header {
                    n += 1;
                    strategy_lang::text_to_definition(&block)
                        .unwrap_or_else(|e| panic!("`{}` example: {e:#?}\n{block}", s.anchor));
                }
                in_fence = !in_fence;
                block.clear();
            } else if in_fence {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    // The first-page example and the four skeletons.
    assert!(n >= 5, "only {n} worked strategies were found in the guide");
}

/// A claim in prose that a form compiles is a claim, so it is compiled.
#[test]
fn the_tight_band_the_crossing_section_suggests_is_a_strategy() {
    let text = "\
strategy band on $bound_at_init timeframe 1h
input fast_ma = ema(close, 7)
input slow_ma = ema(close, 21)
entry long when fast_ma > slow_ma and fast_ma < slow_ma * 1.002
size fixed 0.01
exit time 24 bars
";
    strategy_lang::text_to_definition(text).unwrap_or_else(|e| panic!("{e:#?}"));
}

/// The reference is the registry: every canonical family appears, by the
/// example a caller can paste.
#[test]
fn every_family_the_engine_computes_is_in_the_reference() {
    let reference: String = guide::sections()
        .iter()
        .filter(|s| s.anchor.starts_with("features.reference."))
        .map(|s| s.body.clone())
        .collect();
    for f in features::registry::families() {
        if f.alias_of.is_some() {
            continue;
        }
        assert!(
            reference.contains(&format!("`{}`", f.example())),
            "`{}` is in the registry and not in the reference",
            f.name
        );
    }
}

/// Refusals the guide describes are the refusals the compiler gives, so the
/// prose cannot drift from the code it describes.
#[test]
fn the_refusals_the_guide_names_are_the_compilers() {
    let body = guide::section("functions").expect("functions").body;
    for (clause, code) in [
        ("entry long when abs(close) > 1", "needs_operator"),
        ("entry long when flat", "needs_position_state"),
        (
            "entry long when crossed_above(ema(close, 7), ema(close, 21))",
            "needs_history",
        ),
    ] {
        assert!(body.contains(code), "the guide does not name `{code}`");
        let text = format!(
            "strategy s on $bound_at_init timeframe 1h\n{clause}\nsize fixed 1\nexit none\n"
        );
        let errs = strategy_lang::text_to_definition(&text).expect_err("refused");
        assert!(
            errs.iter().any(|d| d.code.as_str() == code),
            "`{clause}` was not refused with `{code}`: {errs:#?}"
        );
    }
}
