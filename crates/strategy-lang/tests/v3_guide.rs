//! Set U U-7.6: the authoring guide is generated from the registries, and
//! every section a finding points at exists.
//!
//! Run with `SLV3_GUIDE_WRITE=1` to regenerate the tables after an operator or
//! feature family is added.

use strategy_lang::v3::catalogue::{guide_block, GUIDE_BEGIN, GUIDE_END};
use strategy_lang::v3::validate::TEACH;

fn guide_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/agent/slv3-guide.md")
}

#[test]
fn the_guide_tables_are_the_registries() {
    let path = guide_path();
    let text = std::fs::read_to_string(&path).expect("docs/agent/slv3-guide.md exists");
    let start = text
        .find(GUIDE_BEGIN)
        .expect("the guide has the generated block");
    let end = text.find(GUIDE_END).expect("…and its end") + GUIDE_END.len();
    let current = &text[start..end];
    let want = guide_block();
    if current != want {
        if std::env::var("SLV3_GUIDE_WRITE").is_ok() {
            let updated = format!("{}{}{}", &text[..start], want, &text[end..]);
            std::fs::write(&path, updated).expect("write the guide");
            return;
        }
        panic!("docs/agent/slv3-guide.md is stale: rerun with SLV3_GUIDE_WRITE=1");
    }
}

#[test]
fn every_section_a_finding_names_is_in_the_guide() {
    let text = std::fs::read_to_string(guide_path()).unwrap();
    let headings: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("## "))
        .map(|h| h.trim().to_lowercase())
        .collect();
    for (code, section) in TEACH {
        let anchor = section.strip_prefix("slv3#").unwrap();
        assert!(
            headings.iter().any(|h| h == anchor),
            "`{code}` points at #{anchor}, which the guide lacks"
        );
    }
}
