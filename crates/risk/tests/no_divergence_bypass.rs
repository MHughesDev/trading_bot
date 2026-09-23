//! T-0.3's static check: **no flag exists that continues on divergence.**
//!
//! Set T MASTER §5 is explicit — "Do not add a flag that continues; if one is
//! requested, it is an ADR and a conversation, not a config key." A sentence in
//! a design document does not survive the first urgent Friday. A test does.
//!
//! What this asserts, over source rather than behaviour, because behaviour tests
//! can only cover the flags that exist:
//!
//! 1. no identifier anywhere in the risk or reconciliation crates, or in the
//!    platform's configuration, reads as "carry on despite the divergence";
//! 2. the risk gate consults the halt registry unconditionally — not behind a
//!    `cfg`, a feature, or an `Option` that could be `None` in production;
//! 3. the only way out of a halt takes a named human and a stated reason.
//!
//! Comment lines are stripped before scanning, so this file and the modules it
//! polices can go on *describing* the flags that must not exist.

use std::path::{Path, PathBuf};

/// Substrings that would each be a way to keep trading through a divergence.
const FORBIDDEN: &[&str] = &[
    "allow_divergence",
    "ignore_divergence",
    "continue_on_divergence",
    "skip_divergence",
    "divergence_enabled",
    "divergence_disabled",
    "disable_divergence",
    "bypass_halt",
    "disable_halt",
    "skip_halt",
    "halt_enabled",
    "halts_enabled",
    "override_halt",
    "force_submit",
    // `fn unhalt`, not bare `unhalt`: a *test* named
    // `clearing_an_unhalted_strategy_is_an_error` is the opposite of a bypass,
    // and a check that cannot tell those apart teaches people to rename their
    // tests rather than to keep the rule.
    "fn unhalt",
    "clear_all_halts",
];

fn repo_root() -> PathBuf {
    // crates/risk → crates → repo root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/risk sits two levels below the repo root")
        .to_path_buf()
}

/// Every file under `dir` with one of `exts`, recursively.
fn files_under(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path, exts));
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path);
        }
    }
    out
}

/// The file's content with comment-only lines removed, lowercased.
///
/// Prose about the flags that must not exist is not itself such a flag, and a
/// check that could not tell the difference would force these modules to stop
/// explaining themselves.
fn code_only(path: &Path) -> String {
    // `#` starts a comment in YAML and TOML and an *attribute* in Rust, so the
    // rule has to differ by language. Stripping `#[...]` lines from Rust would
    // blind the `#[cfg(feature` assertion below to the exact thing it checks.
    let rust = path.extension().and_then(|e| e.to_str()) == Some("rs");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            if t.starts_with("//") || t.starts_with("--") {
                return false;
            }
            rust || !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

#[test]
fn no_identifier_offers_a_way_to_continue_on_divergence() {
    let root = repo_root();
    let mut scanned = 0_usize;
    let mut violations: Vec<String> = Vec::new();

    let mut targets = files_under(&root.join("crates/risk/src"), &["rs"]);
    targets.extend(files_under(
        &root.join("crates/reconciliation/src"),
        &["rs"],
    ));
    // A config key is the likeliest shape this would take, so the platform's
    // configuration is scanned too — read-only; nothing here edits it.
    targets.extend(files_under(&root.join("crates/config/src"), &["rs"]));
    targets.extend(files_under(&root.join("config"), &["yaml", "yml", "toml"]));

    for path in targets {
        let body = code_only(&path);
        scanned += 1;
        for needle in FORBIDDEN {
            if body.contains(needle) {
                violations.push(format!("{} contains {needle:?}", path.display()));
            }
        }
    }

    assert!(
        scanned > 5,
        "the scan found almost nothing ({scanned} files) — it is pointed at the wrong place, \
         and a check that scans nothing passes for the wrong reason"
    );
    assert!(
        violations.is_empty(),
        "a way to continue on divergence has appeared. Set T MASTER §5: that is an ADR and a \
         conversation, not a config key.\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_gate_consults_the_halt_registry_unconditionally() {
    let gate = code_only(&repo_root().join("crates/risk/src/gate.rs"));

    assert!(
        gate.contains("self.halts.admit("),
        "RiskGate::check must consult the divergence-halt registry"
    );
    assert_eq!(
        gate.matches("self.halts.admit(").count(),
        1,
        "exactly one consultation — two would mean two chances for one to be removed"
    );
    assert!(
        !gate.contains("halts: option<"),
        "the registry must not be optional: a `None` in production is a gate that never halts"
    );
    assert!(
        !gate.contains("#[cfg(feature") && !gate.contains("cfg!(feature"),
        "no part of the gate may be feature-gated"
    );

    // The consultation has to come before the idempotency cache returns a prior
    // approval. A cached approval redelivered after the divergence was found is
    // precisely the order this task exists to stop.
    let admit = gate.find("self.halts.admit(").expect("checked above");
    let cache = gate
        .find("cacheddecision::approved =>")
        .expect("the cached-approval fast path");
    assert!(
        admit < cache,
        "the halt must be checked before the cached-approval fast path, or a redelivered \
         intent rides out after the divergence"
    );
}

#[test]
fn a_halt_is_lifted_only_by_a_named_human_with_a_stated_reason() {
    let src = code_only(&repo_root().join("crates/risk/src/strategy_halt.rs"));

    // `clear` is the only thing that removes an entry.
    assert_eq!(
        src.matches(".remove(").count(),
        1,
        "exactly one place removes a halt; a second remover is a second way out"
    );
    assert!(
        src.contains("pub fn clear(") && src.contains("cleared_by: humanactor"),
        "clearing must demand a HumanActor"
    );
    assert!(
        src.contains("clearerror::noreason"),
        "clearing must demand a non-empty reason"
    );
    assert!(
        !src.contains("pub fn reset(") && !src.contains("pub fn disable("),
        "no blanket reset: the kill switch has one because it is global, and a per-strategy \
         divergence halt that could be reset in bulk would be cleared in bulk"
    );
}

/// The registry has no public constructor path that starts out ignoring halts,
/// and `RiskGate::new` builds one unconditionally — so there is no wiring
/// mistake that silently disables divergence halting.
#[test]
fn every_risk_gate_has_a_halt_registry() {
    let gate = code_only(&repo_root().join("crates/risk/src/gate.rs"));
    assert!(
        gate.contains("halts: arc::new(strategyhaltregistry::new())"),
        "RiskGate::new must always construct a registry"
    );
    assert_eq!(
        gate.matches("pub fn new(").count(),
        1,
        "one constructor; a second could omit the registry"
    );
}
