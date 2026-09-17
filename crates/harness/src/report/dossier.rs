//! The dossier — a report a human can read cold (AGENT-006 §5.8, AGENT-012 §5;
//! Set S task S-8.7).
//!
//! # The rule this module exists for
//!
//! **A section with nothing in it says "none found" rather than being omitted.**
//!
//! That sounds like formatting and is not. An experienced reviewer reading a
//! research write-up with no "Unresolved questions" heading does not conclude
//! there were none — they conclude the section was cut, and they are usually
//! right. The absence of a section and the absence of content look identical on
//! the page and mean opposite things, so the rendering has to distinguish them
//! and only the writer can.
//!
//! The same argument is why the reviewer's critique is mandatory. A dossier with
//! no critique section could mean the critic found nothing or that no critic
//! ran, and those are very different reports.
//!
//! # What is not here
//!
//! No prose generation. A dossier is assembled from parts the platform already
//! holds — the verified claims, the open questions, the critique, the trial ids
//! — and the one thing this module decides is what must be present for it to be
//! renderable at all.

use serde::{Deserialize, Serialize};

use super::claims::Extraction;

/// A section that must appear whether or not it has content.
///
/// Named rather than boolean-flagged, so "which sections are mandatory" is one
/// list that a renderer and a validator both read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    /// What the run could not settle (AGENT-013 §9).
    UnresolvedQuestions,
    /// What the critic said (S-8.6).
    ReviewerCritique,
}

impl Section {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnresolvedQuestions => "Unresolved questions",
            Self::ReviewerCritique => "Reviewer critique",
        }
    }

    /// What an empty one says.
    ///
    /// Per section, because "none found" means different things: no questions
    /// left open is a claim about the research, and no critique findings is a
    /// claim about the review. A shared string would make a reader read one as
    /// the other.
    #[must_use]
    pub fn when_empty(self) -> &'static str {
        match self {
            Self::UnresolvedQuestions => {
                "None found — this run closed everything it opened."
            }
            Self::ReviewerCritique => "None found — the reviewer raised nothing blocking.",
        }
    }

    pub const ALL: &'static [Section] = &[Section::UnresolvedQuestions, Section::ReviewerCritique];
}

/// Why a dossier cannot be compiled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DossierError {
    #[error(
        "a claim in this report is not supported by any evidence, so the dossier is not \
         compiled: {0:?}"
    )]
    UnverifiedClaims(Vec<String>),
    #[error("a dossier states which trial it rests on")]
    NoTicket,
    #[error(
        "the reviewer's critique is missing. A dossier with no critique section could mean the \
         critic found nothing or that no critic ran, and those are different reports — so the \
         absence has to be stated rather than left out"
    )]
    NoCritique,
}

/// One rendered section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rendered {
    pub section: Section,
    /// The lines. Never empty — an empty section renders
    /// [`Section::when_empty`].
    pub lines: Vec<String>,
    /// Whether the content is the explicit "none found" rather than findings.
    pub empty: bool,
}

/// What a compiled dossier holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dossier {
    /// The trial this rests on.
    pub ticket: String,
    pub claims: Extraction,
    pub sections: Vec<Rendered>,
}

impl Dossier {
    /// The section, if it is present. Every [`Section::ALL`] always is.
    #[must_use]
    pub fn section(&self, s: Section) -> Option<&Rendered> {
        self.sections.iter().find(|r| r.section == s)
    }
}

/// What the caller supplies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parts {
    pub ticket: String,
    pub unresolved_questions: Vec<String>,
    /// `None` means **no critique ran**, which is refused. `Some(vec![])` means
    /// the critic ran and found nothing, which renders "none found".
    ///
    /// An `Option<Vec<_>>` rather than a `Vec`, precisely so those two cannot
    /// collapse into the same value — which is the mistake the whole module is
    /// about, expressed in the type.
    pub critique: Option<Vec<String>>,
}

/// Compile one, or say why not.
///
/// # Errors
/// [`DossierError`] — an unverified claim, a missing ticket, or a critique that
/// never ran.
pub fn compile(parts: Parts, claims: Extraction) -> Result<Dossier, DossierError> {
    if parts.ticket.trim().is_empty() {
        return Err(DossierError::NoTicket);
    }
    // The claim check first: a dossier assembled around an invented number is
    // worse than no dossier, and the other two failures are recoverable.
    if claims.blocks() {
        return Err(DossierError::UnverifiedClaims(claims.why_blocked()));
    }
    let Some(critique) = parts.critique else {
        return Err(DossierError::NoCritique);
    };

    let render = |section: Section, lines: Vec<String>| {
        let empty = lines.is_empty();
        Rendered {
            section,
            lines: if empty {
                vec![section.when_empty().to_string()]
            } else {
                lines
            },
            empty,
        }
    };

    Ok(Dossier {
        ticket: parts.ticket,
        claims,
        sections: vec![
            render(Section::UnresolvedQuestions, parts.unresolved_questions),
            render(Section::ReviewerCritique, critique),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::claims::{extract, Evidence};

    fn ev() -> Vec<Evidence> {
        vec![Evidence {
            id: "exp_7".into(),
            text: "sharpe 1.43".into(),
        }]
    }

    fn parts() -> Parts {
        Parts {
            ticket: "t_1".into(),
            unresolved_questions: vec![],
            critique: Some(vec![]),
        }
    }

    /// The rule the module exists for.
    #[test]
    fn an_empty_section_still_renders_and_says_so() {
        let d = compile(parts(), extract("Sharpe 1.43.", &ev())).expect("compiles");
        for s in Section::ALL {
            let r = d.section(*s).unwrap_or_else(|| panic!("{s:?} is missing"));
            assert!(r.empty);
            assert_eq!(r.lines.len(), 1);
            assert!(
                r.lines[0].contains("None found"),
                "{:?} renders {:?}",
                s,
                r.lines
            );
        }
    }

    /// The two "none found" lines differ, because they are claims about
    /// different things.
    #[test]
    fn each_empty_section_says_what_was_empty() {
        assert_ne!(
            Section::UnresolvedQuestions.when_empty(),
            Section::ReviewerCritique.when_empty()
        );
        assert!(Section::UnresolvedQuestions.when_empty().contains("closed"));
        assert!(Section::ReviewerCritique.when_empty().contains("reviewer"));
    }

    /// "The critic ran and found nothing" and "no critic ran" are different
    /// reports, and the type makes them different values.
    #[test]
    fn a_critique_that_never_ran_is_not_an_empty_critique() {
        let mut p = parts();
        p.critique = None;
        let err = compile(p, extract("Sharpe 1.43.", &ev())).expect_err("no critic ran");
        assert_eq!(err, DossierError::NoCritique);

        // Whereas an empty one compiles and renders.
        let d = compile(parts(), extract("Sharpe 1.43.", &ev())).expect("compiles");
        assert!(d.section(Section::ReviewerCritique).expect("present").empty);
    }

    /// An unverified claim stops the dossier rather than annotating it.
    #[test]
    fn an_unsupported_claim_blocks_compilation() {
        let err = compile(parts(), extract("Sharpe 2.9.", &ev())).expect_err("unsupported");
        match err {
            DossierError::UnverifiedClaims(why) => {
                assert_eq!(why.len(), 1);
                assert!(why[0].contains("2.9"), "{why:?}");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn a_dossier_names_the_trial_it_rests_on() {
        let mut p = parts();
        p.ticket = "  ".into();
        assert_eq!(
            compile(p, extract("Sharpe 1.43.", &ev())).expect_err("no ticket"),
            DossierError::NoTicket
        );
    }

    /// Content renders as content, and the empty flag is off.
    #[test]
    fn a_section_with_findings_renders_them() {
        let mut p = parts();
        p.unresolved_questions = vec!["does the funding leg survive costs".into()];
        p.critique = Some(vec!["the null family was chosen after the result".into()]);
        let d = compile(p, extract("Sharpe 1.43.", &ev())).expect("compiles");
        let q = d.section(Section::UnresolvedQuestions).expect("present");
        assert!(!q.empty);
        assert!(q.lines[0].contains("funding leg"));
        let c = d.section(Section::ReviewerCritique).expect("present");
        assert!(!c.empty);
        assert!(c.lines[0].contains("null family"));
    }

    /// Every mandatory section is present on every compiled dossier, whatever
    /// the caller passed.
    #[test]
    fn every_mandatory_section_is_always_present() {
        let d = compile(parts(), extract("Sharpe 1.43.", &ev())).expect("compiles");
        assert_eq!(d.sections.len(), Section::ALL.len());
        for s in Section::ALL {
            assert!(d.section(*s).is_some(), "{s:?}");
        }
    }
}
