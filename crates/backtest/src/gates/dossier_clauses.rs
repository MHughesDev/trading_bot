//! What a dossier must carry, as a check (Set Q Q-2.3 and Q-2.5; BS-007 11 EV-03, EV-06, T18).
//!
//! Three quantities have to sit beside every result so a reader can judge how much searching
//! produced it, and one prior has to sit beside every in-sample number so nobody sizes it as if
//! it were a live one:
//!
//! * the **specification count**: how many specifications were tried (EV-03);
//! * the **exploration ledger** reference: what was looked at before the hypothesis (EV-03);
//! * **`post_hoc`**: whether the hypothesis was written after the data was seen (EV-03);
//! * the **expected live haircut** and its source (T18, EV-06): a prior, not a measurement.
//!
//! [`validate`] returns one violation per missing quantity, each with the fix, so a report
//! validator can refuse a dossier that omits one and say which. It is the pure core; the API's
//! `report_validator` calls it (the hook is in the Q-2.3 / Q-2.5 task notes).
//!
//! # The haircut
//!
//! McLean and Pontiff (2016): about 26% lower out of sample than in sample, and about 58% lower
//! after publication. They are US-equity anomaly figures, and the output says so. They agree
//! with `apps/skill-runner/templates/attribution/expected_decay.py`, and
//! `the_haircuts_agree_with_the_template` reads that file so the two cannot drift.

use serde::{Deserialize, Serialize};

pub const OUT_OF_SAMPLE_PCT: f64 = 26.0;
pub const POST_PUBLICATION_PCT: f64 = 58.0;
pub const CITATION: &str = "McLean and Pontiff (2016), Journal of Finance 71(1): 5-32";

/// Where an idea came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeaSource {
    /// A published anomaly: selected and read.
    Published,
    /// In-house: still selected for looking good on the data that found it.
    InHouse,
    /// Not stated: treated as in-house, and says so.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpectedHaircut {
    pub pct: f64,
    pub source: IdeaSource,
    pub citation: String,
    /// Always `a prior`: the line a reader should not skip.
    pub basis: String,
    pub caveat: String,
}

/// The haircut prior for an idea source.
#[must_use]
pub fn expected_haircut(source: IdeaSource) -> ExpectedHaircut {
    let pct = if source == IdeaSource::Published {
        POST_PUBLICATION_PCT
    } else {
        OUT_OF_SAMPLE_PCT
    };
    ExpectedHaircut {
        pct,
        source,
        citation: CITATION.into(),
        basis: "a prior, to be replaced by the measured out-of-sample and forward results as they arrive".into(),
        caveat: "estimated on US equity anomalies; not estimated on crypto, on intraday horizons or on this platform".into(),
    }
}

/// What a dossier states about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DossierEvidence {
    pub specification_count: Option<u64>,
    pub exploration_ledger_ref: Option<String>,
    pub post_hoc: Option<bool>,
    pub expected_haircut: Option<ExpectedHaircut>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub field: &'static str,
    pub rule: &'static str,
    pub fix: &'static str,
}

/// One violation per missing quantity. An empty list is a dossier that carries all four.
#[must_use]
pub fn validate(ev: &DossierEvidence) -> Vec<Violation> {
    let mut v = Vec::new();
    if ev.specification_count.is_none_or(|n| n == 0) {
        v.push(Violation {
            field: "specification_count",
            rule: "missing_specification_count",
            fix: "state how many specifications were tried (the platform trial count for the experiment); zero is not a count of a search that ran",
        });
    }
    if ev
        .exploration_ledger_ref
        .as_deref()
        .is_none_or(|s| s.trim().is_empty())
    {
        v.push(Violation {
            field: "exploration_ledger_ref",
            rule: "missing_exploration_ledger",
            fix: "attach the exploration_summary artifact for the session",
        });
    }
    if ev.post_hoc.is_none() {
        v.push(Violation {
            field: "post_hoc",
            rule: "missing_post_hoc",
            fix: "state whether the hypothesis was registered before the data was seen; unstated is not false",
        });
    }
    match &ev.expected_haircut {
        None => v.push(Violation {
            field: "expected_haircut",
            rule: "missing_expected_haircut",
            fix: "state the expected live haircut and its source (`expected_decay`)",
        }),
        Some(h) if h.citation.trim().is_empty() => v.push(Violation {
            field: "expected_haircut",
            rule: "haircut_without_source",
            fix: "a haircut states its citation",
        }),
        Some(_) => {}
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete() -> DossierEvidence {
        DossierEvidence {
            specification_count: Some(214),
            exploration_ledger_ref: Some("art_ledger".into()),
            post_hoc: Some(false),
            expected_haircut: Some(expected_haircut(IdeaSource::InHouse)),
        }
    }

    #[test]
    fn a_dossier_that_omits_any_one_quantity_is_refused_naming_it() {
        assert!(validate(&complete()).is_empty());
        let cases: [(&str, Box<dyn Fn(&mut DossierEvidence)>); 4] = [
            (
                "missing_specification_count",
                Box::new(|e| e.specification_count = None),
            ),
            (
                "missing_exploration_ledger",
                Box::new(|e| e.exploration_ledger_ref = None),
            ),
            ("missing_post_hoc", Box::new(|e| e.post_hoc = None)),
            (
                "missing_expected_haircut",
                Box::new(|e| e.expected_haircut = None),
            ),
        ];
        for (rule, strip) in cases {
            let mut e = complete();
            strip(&mut e);
            let v = validate(&e);
            assert_eq!(v.len(), 1, "{rule}: {v:?}");
            assert_eq!(v[0].rule, rule);
        }
        assert_eq!(validate(&DossierEvidence::default()).len(), 4);
    }

    #[test]
    fn a_zero_specification_count_and_a_blank_ledger_ref_do_not_count_as_stated() {
        let mut e = complete();
        e.specification_count = Some(0);
        e.exploration_ledger_ref = Some("  ".into());
        let rules: Vec<_> = validate(&e).iter().map(|v| v.rule).collect();
        assert_eq!(
            rules,
            vec!["missing_specification_count", "missing_exploration_ledger"]
        );
        let mut e = complete();
        e.post_hoc = Some(true);
        assert!(
            validate(&e).is_empty(),
            "post_hoc = true is a stated answer, and is reported as such"
        );
    }

    #[allow(clippy::float_cmp)] // exact constants
    #[test]
    fn a_literature_idea_shows_the_larger_haircut_with_its_source() {
        let published = expected_haircut(IdeaSource::Published);
        let in_house = expected_haircut(IdeaSource::InHouse);
        assert!(published.pct > in_house.pct);
        assert_eq!((published.pct, in_house.pct), (58.0, 26.0));
        assert_eq!(expected_haircut(IdeaSource::Unknown).pct, 26.0);
        assert!(published.citation.contains("McLean") && published.basis.starts_with("a prior"));
    }

    #[test]
    fn a_haircut_with_no_citation_is_refused() {
        let mut e = complete();
        e.expected_haircut.as_mut().unwrap().citation = String::new();
        assert_eq!(validate(&e)[0].rule, "haircut_without_source");
    }

    #[test]
    fn the_haircuts_agree_with_the_template() {
        let src =
            include_str!("../../../../apps/skill-runner/templates/attribution/expected_decay.py");
        assert!(src.contains("OOS = 26.0") && src.contains("POST_PUBLICATION = 58.0"));
        assert!(src.contains(CITATION));
    }
}
