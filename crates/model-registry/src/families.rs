//! The model family registry, as the platform sees it (Set N N-2.1, FEAT-006 section 4).
//!
//! The trainer owns the registry: `apps/model-trainer/app/families/registry.json` is
//! the one list of `(family, variant)` entries, and the trainer resolves each to a
//! train function. This module `include_str!`s that same file, so the platform and the
//! trainer cannot disagree about which families exist, and the platform refuses an
//! unknown `(family, variant)` *before* a run is dispatched rather than after the
//! sidecar has been asked and has failed.
//!
//! Three rules live here because the trainer cannot enforce them for the platform:
//!
//! * an unknown entry is refused with the list of known ones (never a silent fallback);
//! * a family that needs sequences refuses a declaration with no `sequence` (MD-07);
//! * a `foundation_baseline` is never a candidate (N-2.8): a zero-shot model that may
//!   have seen the evaluation window is a thing to beat, not a thing to ship.
//!
//! ADR-N-07: the registry is a JSON file shared by include, not a Rust table that a
//! parity test compares with a Python one. A second copy is a second answer.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The registry file, embedded at build time.
const REGISTRY_JSON: &str = include_str!("../../../apps/model-trainer/app/families/registry.json");

/// What a family entry is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Always trained beside a candidate, on the same folds.
    Baseline,
    Candidate,
    /// Zero-shot foundation model: a baseline only, with contamination metadata.
    FoundationBaseline,
}

/// One `(family, variant)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Family {
    pub family: String,
    pub variant: String,
    /// The legacy framework name that routes here, if any.
    #[serde(default)]
    pub framework: Option<String>,
    pub role: Role,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_sequence: bool,
    #[serde(default)]
    pub uses_gpu: bool,
    #[serde(default)]
    pub requires: Vec<String>,
    pub selectable_as_candidate: bool,
    pub entrypoint: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub contamination: Option<serde_json::Value>,
}

/// A family refusal, naming the reason.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FamilyError {
    #[error("unknown family ({family}, {variant}); known: {known}")]
    Unknown {
        family: String,
        variant: String,
        known: String,
    },
    #[error("({family}, {variant}) consumes sequence windows and the declaration has no `sequence`")]
    NeedsSequence { family: String, variant: String },
    #[error("({family}, {variant}) is a {role} and cannot be the candidate")]
    NotACandidate { family: String, variant: String, role: String },
    #[error("({family}, {variant}) does not support the target `{target}`; it supports {supported}")]
    UnsupportedTarget {
        family: String,
        variant: String,
        target: String,
        supported: String,
    },
}

/// Every registered family.
///
/// # Panics
/// If the embedded registry is not valid JSON of the declared shape: a build-time
/// defect that the unit test below reports first.
#[must_use]
pub fn all() -> &'static [Family] {
    static ALL: OnceLock<Vec<Family>> = OnceLock::new();
    ALL.get_or_init(|| {
        serde_json::from_str(REGISTRY_JSON).expect("registry.json parses as a list of families")
    })
}

fn known_list() -> String {
    all()
        .iter()
        .map(|f| format!("{}/{}", f.family, f.variant))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Look up `(family, variant)`.
///
/// # Errors
/// [`FamilyError::Unknown`], carrying the known list.
pub fn resolve(family: &str, variant: &str) -> Result<&'static Family, FamilyError> {
    all()
        .iter()
        .find(|f| f.family == family && f.variant == variant)
        .ok_or_else(|| FamilyError::Unknown {
            family: family.to_string(),
            variant: variant.to_string(),
            known: known_list(),
        })
}

/// What a train request asks of a family: the chosen entry, the declared target name
/// (`forward_return`, `realized_vol`, ...) if any, and whether a sequence window was declared.
///
/// # Errors
/// An unknown entry, a foundation baseline or baseline chosen as the candidate when
/// `candidate` is true, a sequence family without a window, or a target the family
/// does not list.
pub fn check_for_training(
    family: &str,
    variant: &str,
    target: Option<&str>,
    has_sequence: bool,
    candidate: bool,
) -> Result<&'static Family, FamilyError> {
    let f = resolve(family, variant)?;
    if candidate && !f.selectable_as_candidate {
        return Err(FamilyError::NotACandidate {
            family: f.family.clone(),
            variant: f.variant.clone(),
            role: match f.role {
                Role::Baseline => "baseline",
                Role::Candidate => "candidate-ineligible entry",
                Role::FoundationBaseline => "foundation baseline",
            }
            .to_string(),
        });
    }
    if f.needs_sequence && !has_sequence {
        return Err(FamilyError::NeedsSequence {
            family: f.family.clone(),
            variant: f.variant.clone(),
        });
    }
    if let Some(t) = target {
        if !f.targets.is_empty() && !f.targets.iter().any(|x| x == t) {
            return Err(FamilyError::UnsupportedTarget {
                family: f.family.clone(),
                variant: f.variant.clone(),
                target: t.to_string(),
                supported: f.targets.join(", "),
            });
        }
    }
    Ok(f)
}

/// The entry a legacy `framework` string routes to (first match), so a model that
/// predates the registry still resolves.
#[must_use]
pub fn for_framework(framework: &str) -> Option<&'static Family> {
    let fw = framework.to_ascii_lowercase();
    all()
        .iter()
        .find(|f| f.framework.as_deref().is_some_and(|x| x == fw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_registry_parses_and_has_no_duplicate_entries() {
        let all = all();
        assert!(all.len() >= 20, "registry has {} entries", all.len());
        let mut keys: Vec<_> = all.iter().map(|f| (&f.family, &f.variant)).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "a (family, variant) is listed twice");
        // Every entry names a module:function the trainer resolves.
        assert!(all.iter().all(|f| f.entrypoint.contains(':')));
    }

    #[test]
    fn an_unknown_entry_is_refused_with_the_known_list() {
        let e = resolve("gbm", "no_such_variant").unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("unknown family (gbm, no_such_variant)"));
        assert!(msg.contains("gbm/lightgbm"), "the known list is named: {msg}");
    }

    #[test]
    fn every_legacy_framework_is_reachable_through_the_registry() {
        for fw in ["lightgbm", "xgboost", "sklearn", "torch", "garch"] {
            assert!(for_framework(fw).is_some(), "framework `{fw}` has no registry entry");
        }
    }

    #[test]
    fn a_sequence_family_refuses_a_declaration_without_a_window() {
        let seq = all()
            .iter()
            .find(|f| f.needs_sequence)
            .expect("the registry lists at least one sequence family");
        let refused = check_for_training(&seq.family, &seq.variant, None, false, true);
        assert!(matches!(refused, Err(FamilyError::NeedsSequence { .. })));
        assert!(check_for_training(&seq.family, &seq.variant, None, true, true).is_ok());
    }

    #[test]
    fn a_foundation_baseline_is_never_the_candidate() {
        let fb = all()
            .iter()
            .find(|f| f.role == Role::FoundationBaseline)
            .expect("the registry lists a foundation baseline");
        assert!(!fb.selectable_as_candidate);
        let refused = check_for_training(&fb.family, &fb.variant, None, true, true);
        assert!(matches!(refused, Err(FamilyError::NotACandidate { .. })), "{refused:?}");
        // As a baseline (candidate = false) it is allowed.
        assert!(check_for_training(&fb.family, &fb.variant, None, true, false).is_ok());
    }

    #[test]
    fn a_target_the_family_does_not_list_is_refused() {
        let f = for_framework("lightgbm").unwrap();
        let refused = check_for_training(&f.family, &f.variant, Some("realized_vol"), false, true);
        assert!(matches!(refused, Err(FamilyError::UnsupportedTarget { .. })), "{refused:?}");
    }
}
