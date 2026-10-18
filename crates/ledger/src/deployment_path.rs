//! The deployment path (Set T T-1.1, T-1.2): what a proposal to reach paper
//! must carry, and what a `paper_v1` pass can and cannot cause.
//!
//! This module performs no I/O. The tables it describes are migration 0180; the
//! routes that write them are `api::deployment_gate`. What lives here is the part
//! that is true whatever the transport: the shape of a proposal, the refusal of a
//! malformed or stale one, and the one value a venue class can take.
//!
//! # One venue class
//!
//! [`VenueClass`] has exactly one variant. Not "paper is the default": paper is
//! the only thing a deployment this platform authorises can be. Anything that
//! would add a capital variant has to edit this enum, which breaks every
//! exhaustive `match` on it (`api/tests/no_capital_path.rs` holds one), and edit
//! migration 0180's `CHECK (venue_class = 'paper')`. Enabling capital is a
//! reviewed change in three places, never a configuration discovery (T-3.4).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Where an authorised deployment may trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VenueClass {
    /// The platform's own paper venue (ADR-0048). The only class there is.
    Paper,
}

impl VenueClass {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
        }
    }
}

/// The detector's thresholds, declared at proposal time and never edited
/// (SPEC §12.6: a stopping rule chosen after watching the result is not one).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DivergenceThresholds {
    pub max_abs_return_divergence: f64,
    pub max_cum_return_divergence: f64,
    pub max_position_divergence: f64,
    pub min_signal_match: f64,
}

/// Why a proposal was refused. Every variant names the field to fix.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProposalRefusal {
    #[error("revision `{0}` is not a 64-character lowercase sha256")]
    BadRevision(String),
    #[error("the verdict was computed for revision `{verdict}`, not `{proposal}`: a verdict is about one revision and cannot be carried to another")]
    StaleVerdict { verdict: String, proposal: String },
    #[error("the strategy's head is now `{head}`; this proposal is for `{proposal}`. The verdict is stale: re-run the gates against the revision that would deploy")]
    StaleAgainstHead { head: String, proposal: String },
    #[error("a proposal carries all sixteen gate verdicts, each once; gate {0} is missing")]
    GateMissing(i32),
    #[error("gate {0} appears more than once in the verdict")]
    GateDuplicated(i32),
    #[error("gate {0} is not one of the sixteen")]
    GateOutOfRange(i32),
    #[error("a paper pass needs gates 1..=15 to have passed; gate {0} did not")]
    GateFailed(i32),
    #[error("threshold `{0}` must be a finite positive number (min_signal_match in (0, 1], max_position_divergence >= 0)")]
    BadThreshold(&'static str),
    #[error("starting capital must be a positive decimal")]
    BadCapital,
    #[error("the review period must be at least one day")]
    BadPeriod,
}

/// A proposal as submitted. No defaults anywhere: an omitted field is a refusal,
/// not a value somebody never chose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalDraft {
    pub strategy_name: String,
    pub revision_hash: String,
    /// One instrument, or `selection:<revision>`.
    pub instrument_id: String,
    pub profile_id: String,
    /// The revision the verdict was computed for.
    pub verdict_revision_hash: String,
    /// `(gate_no, passed)` for every one of the sixteen.
    pub verdict: Vec<GateVerdictEntry>,
    pub thresholds: DivergenceThresholds,
    /// Decimal string.
    pub starting_capital: String,
    pub review_period_days: i32,
}

/// One gate's entry in a proposal's verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateVerdictEntry {
    pub gate_no: i32,
    pub passed: bool,
    /// The statistic and detail the human reads. Optional: the pass/fail is
    /// what the platform acts on, the rest is what the human decides on.
    #[serde(default)]
    pub detail: Option<String>,
}

/// A proposal that satisfied every structural rule. Only [`ProposalDraft::validate`]
/// produces one, so a function that takes it knows the rules ran.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedProposal {
    draft: ProposalDraft,
    gates_passed: i32,
}

impl ValidatedProposal {
    #[must_use]
    pub fn draft(&self) -> &ProposalDraft {
        &self.draft
    }
    #[must_use]
    pub fn gates_passed(&self) -> i32 {
        self.gates_passed
    }
    #[must_use]
    pub fn venue_class(&self) -> VenueClass {
        VenueClass::Paper
    }
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn positive_decimal(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && t.matches('.').count() <= 1
        && t.bytes().any(|b| (b'1'..=b'9').contains(&b))
}

impl ProposalDraft {
    /// The structural rules of T-1.1. What this cannot check is staleness against
    /// the strategy's head, which needs a read: [`check_not_stale`] does that.
    ///
    /// # Errors
    /// The first rule the draft breaks.
    pub fn validate(self) -> Result<ValidatedProposal, ProposalRefusal> {
        if !is_sha256(&self.revision_hash) {
            return Err(ProposalRefusal::BadRevision(self.revision_hash));
        }
        if self.verdict_revision_hash != self.revision_hash {
            return Err(ProposalRefusal::StaleVerdict {
                verdict: self.verdict_revision_hash,
                proposal: self.revision_hash,
            });
        }
        let mut seen = BTreeSet::new();
        let mut passed = 0;
        for g in &self.verdict {
            if !(1..=16).contains(&g.gate_no) {
                return Err(ProposalRefusal::GateOutOfRange(g.gate_no));
            }
            if !seen.insert(g.gate_no) {
                return Err(ProposalRefusal::GateDuplicated(g.gate_no));
            }
            if g.passed {
                passed += 1;
            }
        }
        for n in 1..=16 {
            if !seen.contains(&n) {
                return Err(ProposalRefusal::GateMissing(n));
            }
        }
        let t = &self.thresholds;
        for (name, v, lo_open) in [
            (
                "max_abs_return_divergence",
                t.max_abs_return_divergence,
                true,
            ),
            (
                "max_cum_return_divergence",
                t.max_cum_return_divergence,
                true,
            ),
            ("max_position_divergence", t.max_position_divergence, false),
        ] {
            if !v.is_finite() || v < 0.0 || (lo_open && v == 0.0) {
                return Err(ProposalRefusal::BadThreshold(name));
            }
        }
        if !(t.min_signal_match.is_finite()
            && t.min_signal_match > 0.0
            && t.min_signal_match <= 1.0)
        {
            return Err(ProposalRefusal::BadThreshold("min_signal_match"));
        }
        if !positive_decimal(&self.starting_capital) {
            return Err(ProposalRefusal::BadCapital);
        }
        if self.review_period_days < 1 {
            return Err(ProposalRefusal::BadPeriod);
        }
        Ok(ValidatedProposal {
            draft: self,
            gates_passed: passed,
        })
    }
}

/// The verdict is about a revision; the strategy's head may have moved since.
///
/// # Errors
/// [`ProposalRefusal::StaleAgainstHead`] when they differ.
pub fn check_not_stale(
    proposal_revision: &str,
    head_revision: &str,
) -> Result<(), ProposalRefusal> {
    if proposal_revision == head_revision {
        Ok(())
    } else {
        Err(ProposalRefusal::StaleAgainstHead {
            head: head_revision.to_owned(),
            proposal: proposal_revision.to_owned(),
        })
    }
}

/// Proof that a verdict clears paper and says nothing about capital.
///
/// `paper_v1` is `strict_v1` minus Gate 9's calendar floor and minus Gate 16
/// (ADR-P2-14), so a paper pass is gates 1..=15. The type has a venue class
/// ([`VenueClass::Paper`]) and no way to become anything else: there is no
/// `into_capital`, and the only path to capital is
/// [`crate::capital::authorise`], which refuses a profile that does not
/// authorise it (AT-65) and is not reachable from here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperAuthority {
    profile_id: String,
}

impl PaperAuthority {
    /// # Errors
    /// The first of gates 1..=15 that did not pass, or that is absent.
    pub fn from_pass(
        profile_id: &str,
        verdict: &[GateVerdictEntry],
    ) -> Result<Self, ProposalRefusal> {
        for n in 1..=15 {
            match verdict.iter().find(|g| g.gate_no == n) {
                None => return Err(ProposalRefusal::GateMissing(n)),
                Some(g) if !g.passed => return Err(ProposalRefusal::GateFailed(n)),
                Some(_) => {}
            }
        }
        Ok(Self {
            profile_id: profile_id.to_owned(),
        })
    }

    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    #[must_use]
    pub fn venue_class(&self) -> VenueClass {
        VenueClass::Paper
    }
}

/// The hash a decision's envelope cites as the evidence it was made on: the
/// revision, the profile, every verdict, and the thresholds, in a fixed order.
/// Recomputing it from the proposal row reproduces it, so "on what evidence" is a
/// checkable claim rather than a sentence.
#[must_use]
pub fn evidence_hash(p: &ProposalDraft) -> String {
    let mut verdict: Vec<&GateVerdictEntry> = p.verdict.iter().collect();
    verdict.sort_by_key(|g| g.gate_no);
    let canonical = json!({
        "revision": p.revision_hash,
        "profile": p.profile_id,
        "verdict": verdict.iter().map(|g| json!([g.gate_no, g.passed])).collect::<Vec<_>>(),
        "thresholds": [
            p.thresholds.max_abs_return_divergence,
            p.thresholds.max_cum_return_divergence,
            p.thresholds.max_position_divergence,
            p.thresholds.min_signal_match,
        ],
        "capital": p.starting_capital,
        "period_days": p.review_period_days,
    });
    hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
}

/// Migration 0054's envelope shape, for a deployment decision: who decided,
/// when, on what evidence, within what limits. Migration 0180 requires each key.
#[must_use]
pub fn envelope(p: &ProposalDraft, decided_by: &str, decided_at: &str) -> Value {
    json!({
        "decided_by": decided_by,
        "decided_at": decided_at,
        "evidence_hash": evidence_hash(p),
        "starting_capital": p.starting_capital,
        "review_period_days": p.review_period_days,
        "thresholds": p.thresholds,
        "profile_id": p.profile_id,
        "venue_class": VenueClass::Paper.as_str(),
    })
}

/// The proposal's sixteen is the capital ramp's sixteen: changing one without
/// the other is a compile error.
const _: () = assert!(crate::capital::GATE_COUNT == 16);

#[cfg(test)]
mod tests {
    use super::*;

    const REV: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn gates(all: bool) -> Vec<GateVerdictEntry> {
        (1..=16)
            .map(|n| GateVerdictEntry {
                gate_no: n,
                passed: all || n != 7,
                detail: None,
            })
            .collect()
    }

    pub(crate) fn draft() -> ProposalDraft {
        ProposalDraft {
            strategy_name: "ema-cross".into(),
            revision_hash: REV.into(),
            instrument_id: "BTC-USD".into(),
            profile_id: "paper_v1".into(),
            verdict_revision_hash: REV.into(),
            verdict: gates(true),
            thresholds: DivergenceThresholds {
                max_abs_return_divergence: 0.02,
                max_cum_return_divergence: 0.05,
                max_position_divergence: 0.0,
                min_signal_match: 0.9,
            },
            starting_capital: "10000".into(),
            review_period_days: 14,
        }
    }

    #[test]
    fn a_complete_proposal_validates_and_is_paper() {
        let v = draft().validate().expect("valid");
        assert_eq!(v.gates_passed(), 16);
        assert_eq!(v.venue_class(), VenueClass::Paper);
    }

    #[test]
    fn a_verdict_for_another_revision_is_stale() {
        let mut d = draft();
        d.verdict_revision_hash = "b".repeat(64);
        assert!(matches!(
            d.validate(),
            Err(ProposalRefusal::StaleVerdict { .. })
        ));
        // And against the head: the strategy moved after the verdict was made.
        assert!(check_not_stale(REV, REV).is_ok());
        assert!(matches!(
            check_not_stale(REV, &"c".repeat(64)),
            Err(ProposalRefusal::StaleAgainstHead { .. })
        ));
    }

    #[test]
    fn fifteen_gates_is_not_a_verdict() {
        let mut d = draft();
        d.verdict.pop();
        assert_eq!(d.validate(), Err(ProposalRefusal::GateMissing(16)));
        let mut d = draft();
        d.verdict[3].gate_no = 5;
        assert_eq!(d.validate(), Err(ProposalRefusal::GateDuplicated(5)));
        let mut d = draft();
        d.verdict[0].gate_no = 17;
        assert_eq!(d.validate(), Err(ProposalRefusal::GateOutOfRange(17)));
    }

    #[test]
    fn thresholds_capital_and_period_are_each_required_to_mean_something() {
        for (mutate, expect) in [
            (
                (|d: &mut ProposalDraft| d.thresholds.min_signal_match = 0.0)
                    as fn(&mut ProposalDraft),
                ProposalRefusal::BadThreshold("min_signal_match"),
            ),
            (
                |d| d.thresholds.max_abs_return_divergence = f64::NAN,
                ProposalRefusal::BadThreshold("max_abs_return_divergence"),
            ),
            (
                |d| d.starting_capital = "0".into(),
                ProposalRefusal::BadCapital,
            ),
            (
                |d| d.starting_capital = "-5".into(),
                ProposalRefusal::BadCapital,
            ),
            (|d| d.review_period_days = 0, ProposalRefusal::BadPeriod),
        ] {
            let mut d = draft();
            mutate(&mut d);
            assert_eq!(d.validate(), Err(expect));
        }
    }

    #[test]
    fn a_paper_pass_needs_gates_one_to_fifteen_and_only_those() {
        let ok = PaperAuthority::from_pass("paper_v1", &gates(true)).unwrap();
        assert_eq!(ok.venue_class(), VenueClass::Paper);
        // Gate 16 is not applicable under paper_v1: failing it does not block paper.
        let mut v = gates(true);
        v[15].passed = false;
        assert!(PaperAuthority::from_pass("paper_v1", &v).is_ok());
        // Gate 7 failing does.
        assert_eq!(
            PaperAuthority::from_pass("paper_v1", &gates(false)),
            Err(ProposalRefusal::GateFailed(7))
        );
    }

    /// T-1.2 / AT-65 — sixteen genuine passes under `paper_v1` produce a paper
    /// authority and **no** capital authority. The refusal is about the profile.
    #[test]
    fn a_paper_v1_pass_cannot_be_converted_into_capital() {
        let all: Vec<(i32, bool)> = (1..=16).map(|g| (g, true)).collect();
        assert!(PaperAuthority::from_pass("paper_v1", &gates(true)).is_ok());
        let refused = crate::capital::authorise("paper_v1", false, &all).unwrap_err();
        assert!(matches!(
            refused,
            crate::capital::RampRefusal::ProfileDoesNotAuthorise(_)
        ));
    }

    /// The exhaustive match is the tripwire: adding a capital variant to
    /// [`VenueClass`] stops this compiling, which is the review T-3.4 asks for.
    #[test]
    fn the_only_venue_class_is_paper() {
        let v = VenueClass::Paper;
        match v {
            VenueClass::Paper => assert_eq!(v.as_str(), "paper"),
        }
    }

    #[test]
    fn the_evidence_hash_is_reproducible_and_changes_with_the_evidence() {
        let a = evidence_hash(&draft());
        assert_eq!(a, evidence_hash(&draft()));
        let mut d = draft();
        d.verdict[4].passed = false;
        assert_ne!(a, evidence_hash(&d));
        let mut d = draft();
        d.thresholds.min_signal_match = 0.5;
        assert_ne!(a, evidence_hash(&d));
        let e = envelope(&draft(), "mason", "2026-10-03T00:00:00Z");
        for key in [
            "decided_by",
            "decided_at",
            "evidence_hash",
            "starting_capital",
            "review_period_days",
            "thresholds",
            "profile_id",
        ] {
            assert!(e.get(key).is_some(), "{key}");
        }
    }
}
