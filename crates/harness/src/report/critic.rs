//! The checklist critic (AGENT-008 §3.4 rung 8, AGENT-006 §5.8; Set S task
//! S-8.6).
//!
//! # Two critics, and why the cheap one is not a fallback
//!
//! §3.4 rung 8 wants a critic in a **fresh session** — the evidence bundle only,
//! read-only tools, never the producer's reasoning. Separated *sessions* rather
//! than separated prompts, because same-context self-critique measurably
//! underperforms: a model asked to attack its own argument in the window that
//! contains the argument mostly restates it.
//!
//! That critic is a sub-run, and the mechanism for it exists —
//! [`Projection::Critic`](crate::context::projection::Projection::Critic)
//! excludes the producer's reasoning **by construction**, and
//! [`Delegation::check`](crate::subrun::Delegation::check) refuses a critic
//! dispatched under any other projection. What is in this file is the other
//! half: the deterministic checklist smaller tiers get instead.
//!
//! It is not a degraded version of the model critic. It checks four things a
//! model is *bad* at checking — whether every acceptance criterion carries an
//! evidence id, whether every trial is in a family, whether a `suggestive`
//! finding is being presented as established, whether the null comparison ran —
//! and it checks them exactly, every time, at no cost. A frontier tier should
//! run both.
//!
//! # `blocking` sends the draft back
//!
//! A critique that annotates is a critique that gets shipped with the
//! annotations still in it. [`Critique::blocks`] is the difference between a
//! review and a comment.

use serde::{Deserialize, Serialize};

/// One thing the checklist looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Every acceptance criterion the report claims to have met names the
    /// evidence that met it.
    CriteriaCiteEvidence,
    /// Every trial is inside a registered family.
    ///
    /// A trial outside one is a look that no multiple-comparison correction
    /// covers, which is how a family-wise adjustment comes to be computed over
    /// the wrong denominator.
    TrialsAreInAFamily,
    /// No `suggestive` finding is written up as established.
    ///
    /// The most common overclaim and the hardest to see in prose, because the
    /// sentence is usually true — it is the *confidence word* that is wrong.
    SuggestiveNotPresentedAsEstablished,
    /// The null comparison ran.
    ///
    /// "Better than nothing" is not a result. A report with no null is a report
    /// comparing a strategy to itself.
    NullComparisonRan,
}

impl Check {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CriteriaCiteEvidence => "criteria_cite_evidence",
            Self::TrialsAreInAFamily => "trials_are_in_a_family",
            Self::SuggestiveNotPresentedAsEstablished => "suggestive_not_presented_as_established",
            Self::NullComparisonRan => "null_comparison_ran",
        }
    }

    pub const ALL: &'static [Check] = &[
        Check::CriteriaCiteEvidence,
        Check::TrialsAreInAFamily,
        Check::SuggestiveNotPresentedAsEstablished,
        Check::NullComparisonRan,
    ];
}

/// One acceptance criterion the report says it met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    pub text: String,
    /// The evidence that met it. Empty is the finding.
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

/// One trial the report rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    pub ticket_id: String,
    /// `None` is a look outside every family.
    #[serde(default)]
    pub family_id: Option<String>,
}

/// One finding, and how the report presents it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presented {
    pub id: String,
    /// What the platform labelled it: `established`, `suggestive`, `refuted`.
    pub confidence: String,
    /// Whether the report presents it as settled.
    pub presented_as_established: bool,
}

/// What the critic is given.
///
/// Deliberately **not** the transcript. The checklist reads the same evidence
/// bundle the model critic gets, and giving it more would make the two critics
/// answer different questions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub criteria: Vec<Criterion>,
    pub trials: Vec<Trial>,
    pub findings: Vec<Presented>,
    /// Whether a null comparison ran, and which.
    pub null_family: Option<String>,
}

/// One thing the critic found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub check: Check,
    /// What is wrong, naming the thing.
    pub detail: String,
    /// Whether this sends the draft back.
    pub blocking: bool,
}

/// What the critic concluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Critique {
    pub findings: Vec<Finding>,
}

impl Critique {
    /// Whether the draft goes back.
    #[must_use]
    pub fn blocks(&self) -> bool {
        self.findings.iter().any(|f| f.blocking)
    }

    /// The lines a dossier's "Reviewer critique" section renders.
    ///
    /// Empty when nothing was found — and the dossier renders *that* as "none
    /// found" rather than omitting the section, which is the distinction
    /// [`super::dossier`] exists for.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.findings.iter().map(|f| f.detail.clone()).collect()
    }
}

/// Run the checklist.
///
/// Every check runs; the critique is not short-circuited on the first failure.
/// A writer who has to resubmit four times to learn four things burns four
/// rounds, which is the same argument `report_validator` makes.
#[must_use]
pub fn review(draft: &Draft) -> Critique {
    let mut findings = Vec::new();

    for c in &draft.criteria {
        if c.evidence_ids.is_empty() {
            findings.push(Finding {
                check: Check::CriteriaCiteEvidence,
                detail: format!(
                    "the criterion `{}` names no evidence, so nothing shows it was met",
                    c.text.trim()
                ),
                // Blocking. An acceptance criterion with no evidence is the
                // report asserting its own conclusion.
                blocking: true,
            });
        }
    }

    for t in &draft.trials {
        if t.family_id.as_ref().is_none_or(|f| f.trim().is_empty()) {
            findings.push(Finding {
                check: Check::TrialsAreInAFamily,
                detail: format!(
                    "trial `{}` is in no family, so no multiple-comparison correction covers it",
                    t.ticket_id
                ),
                blocking: true,
            });
        }
    }

    for f in &draft.findings {
        if f.presented_as_established && f.confidence != "established" {
            findings.push(Finding {
                check: Check::SuggestiveNotPresentedAsEstablished,
                detail: format!(
                    "finding `{}` is labelled `{}` and is presented as established",
                    f.id, f.confidence
                ),
                blocking: true,
            });
        }
    }

    if draft
        .null_family
        .as_ref()
        .is_none_or(|n| n.trim().is_empty())
    {
        findings.push(Finding {
            check: Check::NullComparisonRan,
            detail: "no null comparison ran, so the result is not better than anything"
                .to_string(),
            blocking: true,
        });
    }

    Critique { findings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> Draft {
        Draft {
            criteria: vec![Criterion {
                text: "sharpe after costs exceeds the benchmark".into(),
                evidence_ids: vec!["exp_7".into()],
            }],
            trials: vec![Trial {
                ticket_id: "t_1".into(),
                family_id: Some("fam_3".into()),
            }],
            findings: vec![Presented {
                id: "fnd_1".into(),
                confidence: "established".into(),
                presented_as_established: true,
            }],
            null_family: Some("block_bootstrap".into()),
        }
    }

    #[test]
    fn a_clean_draft_passes_and_finds_nothing() {
        let c = review(&clean());
        assert!(c.findings.is_empty());
        assert!(!c.blocks());
        // Which the dossier renders as "none found" rather than omitting.
        assert!(c.lines().is_empty());
    }

    #[test]
    fn a_criterion_with_no_evidence_blocks() {
        let mut d = clean();
        d.criteria[0].evidence_ids.clear();
        let c = review(&d);
        assert!(c.blocks());
        assert_eq!(c.findings[0].check, Check::CriteriaCiteEvidence);
        assert!(c.findings[0].detail.contains("sharpe after costs"));
    }

    /// A trial outside a family is a look no correction covers, which is how a
    /// family-wise adjustment ends up over the wrong denominator.
    #[test]
    fn a_trial_in_no_family_blocks() {
        let mut d = clean();
        d.trials[0].family_id = None;
        assert!(review(&d).blocks());
        // An empty string is not a family either.
        d.trials[0].family_id = Some("  ".into());
        let c = review(&d);
        assert!(c.blocks());
        assert!(c.findings[0].detail.contains("t_1"));
    }

    /// The overclaim that is hardest to see in prose: the sentence is true and
    /// the confidence word is wrong.
    #[test]
    fn a_suggestive_finding_presented_as_established_blocks() {
        let mut d = clean();
        d.findings[0].confidence = "suggestive".into();
        let c = review(&d);
        assert!(c.blocks());
        assert_eq!(
            c.findings[0].check,
            Check::SuggestiveNotPresentedAsEstablished
        );

        // Presenting it honestly is fine.
        d.findings[0].presented_as_established = false;
        assert!(!review(&d).blocks());
    }

    /// A refuted finding presented as established is caught by the same rule,
    /// which is worth asserting because it is the worse case.
    #[test]
    fn a_refuted_finding_presented_as_established_blocks() {
        let mut d = clean();
        d.findings[0].confidence = "refuted".into();
        assert!(review(&d).blocks());
    }

    #[test]
    fn a_report_with_no_null_blocks() {
        let mut d = clean();
        d.null_family = None;
        let c = review(&d);
        assert!(c.blocks());
        assert_eq!(c.findings[0].check, Check::NullComparisonRan);
    }

    /// Every check runs; the critique is not short-circuited. A writer who has
    /// to resubmit four times to learn four things burns four rounds.
    #[test]
    fn every_failure_is_reported_not_just_the_first() {
        let d = Draft {
            criteria: vec![Criterion {
                text: "it works".into(),
                evidence_ids: vec![],
            }],
            trials: vec![Trial {
                ticket_id: "t_1".into(),
                family_id: None,
            }],
            findings: vec![Presented {
                id: "fnd_1".into(),
                confidence: "suggestive".into(),
                presented_as_established: true,
            }],
            null_family: None,
        };
        let c = review(&d);
        assert_eq!(c.findings.len(), 4);
        let checks: Vec<Check> = c.findings.iter().map(|f| f.check).collect();
        for check in Check::ALL {
            assert!(checks.contains(check), "{check:?} did not run");
        }
    }

    /// An empty draft still demands a null. Nothing to check is not the same as
    /// everything checking out.
    #[test]
    fn an_empty_draft_is_not_a_clean_one() {
        let c = review(&Draft::default());
        assert!(c.blocks());
        assert_eq!(c.findings.len(), 1);
        assert_eq!(c.findings[0].check, Check::NullComparisonRan);
    }

    /// S-8.6's acceptance, asserted structurally rather than behaviourally: the
    /// model critic cannot read the producer's reasoning.
    ///
    /// Not a property of this file — the checklist reads a `Draft` and has no
    /// transcript to read — but of the sub-run the frontier tier dispatches. It
    /// is asserted here because this is the module the acceptance names, and a
    /// guarantee nobody points at is one somebody re-implements without.
    ///
    /// Two mechanisms, and neither is an instruction to the model: the
    /// `Critic` projection does not *filter* the producer's reasoning out of the
    /// prompt, it leaves those records out of the store the critic selects from;
    /// and a critic dispatched under any other projection is refused before a
    /// model is called.
    #[test]
    fn the_model_critic_cannot_read_the_producers_reasoning() {
        use crate::context::body::Body;
        use crate::context::projection::{project, Projection};
        use crate::context::store::{Draft as Record, RecordKind, RunStore};
        use crate::subrun::{Delegation, DelegationError, Role};

        let mut producer = RunStore::new();
        producer.insert(Record::new(
            1,
            RecordKind::ToolResult,
            "run_backtest",
            Body::text("sharpe 1.43"),
        ));
        producer.insert(Record::new(
            2,
            RecordKind::Decision,
            "harness",
            Body::text("I will present this as established"),
        ));
        producer.insert(Record::new(
            3,
            RecordKind::PlanChange,
            "harness",
            Body::text("dropped the null comparison to save a trial"),
        ));

        let projected = project(&producer, &Projection::Critic);
        for r in projected.store.all() {
            assert!(
                !matches!(
                    r.kind,
                    RecordKind::Decision | RecordKind::PlanChange | RecordKind::Correction
                ),
                "{:?} reached the critic",
                r.kind
            );
        }
        // The evidence did reach it, which is the half that makes a critic
        // useful rather than merely blind.
        assert_eq!(projected.store.len(), 1);
        assert_eq!(projected.withheld.len(), 2);

        // And a critic dispatched under a wider projection is refused before a
        // model is called.
        let wrong = Delegation {
            role: Role::Critic,
            projection: Some("full".into()),
            brief_digest: "sha256:abc".into(),
            withheld: 0,
            depth: 1,
        };
        assert!(matches!(
            wrong.check(1),
            Err(DelegationError::WrongProjection { .. })
        ));
    }
}
