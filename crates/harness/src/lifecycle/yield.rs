//! What the run produced, as a number (AGENT-013 P5, §10; Set S task S-10.13).
//!
//! # The failure this measures
//!
//! A run can end having produced no knowledge while every dashboard shows
//! activity: steps taken, tools called, tokens spent, trials counted. None of
//! those is an output. A run that spent nine trials and wrote nothing down looks
//! identical, on every surface this platform had, to one that spent nine trials
//! and closed a question.
//!
//! So the platform computes a yield at run end, and a run with **trials and zero
//! knowledge units is barren** — reported per tier and per workflow rather than
//! averaged, because "12% barren" over everything is a number nobody can act on
//! and "`survey` on the 7B is barren 60% of the time" is.
//!
//! # What counts, and what is deliberately worth nothing
//!
//! The weighting is the whole design, because a yield metric is a thing runs will
//! be optimised against the moment it exists. Each nought below is there to close
//! a way of manufacturing units:
//!
//! * **A suggestive finding is worth nothing.** It is a lead, not knowledge, and
//!   counting it would make "record six weak findings" the cheapest way to look
//!   productive. `established` requires verified evidence; that is the bar.
//! * **An opened question is worth nothing.** Noticing an absence is not
//!   producing knowledge, and a run that opened four questions and answered none
//!   has left more work than it did. Counted and reported, never scored.
//! * **A proposal is worth nothing.** It is a request for a human decision, and
//!   the decision is where the value is.
//!
//! And what is worth *more* than one:
//!
//! * **A negative result whose class is new for the subject scores two.** It
//!   closes a door nobody had closed. Most strategies are noise, so proving one
//!   is noise is worth more than another backtest that settles nothing — and a
//!   platform whose yield metric paid only for positives would be a platform that
//!   quietly stopped reporting failures.

use serde::{Deserialize, Serialize};

use crate::memory::Confidence;

/// What one run produced (§10).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Yield {
    pub findings_established: u32,
    pub findings_suggestive: u32,
    /// A finding a later comparison refuted. **Knowledge**, and counted as such:
    /// knowing an idea is dead is the point of keeping it (`Confidence::weight`
    /// makes the same argument about retrieval), and a yield metric that paid
    /// nothing for it would be a metric that discouraged checking.
    pub findings_refuted: u32,
    pub negatives: u32,
    /// Negatives whose failure class had not been recorded for this subject
    /// before. The door nobody had closed.
    pub negatives_new_class: u32,
    pub questions_opened: u32,
    pub questions_closed: u32,
    /// Disagreements between two measurements, surfaced under P5.
    pub contradictions_surfaced: u32,
    pub proposals: u32,
    pub hypotheses_tested: u32,
    pub hypotheses_refuted: u32,
    pub hypotheses_supported: u32,
    pub trials_spent: u32,
    pub cost_usd: f64,
}

/// What a negative result is worth when its class is new for the subject.
pub const NEW_CLASS_BONUS: u32 = 1;

impl Yield {
    /// The run's knowledge, as a count.
    ///
    /// See the module header for why each nought is a nought. The short version:
    /// every one of them closes a way to manufacture units without learning
    /// anything, and a yield metric with an open one is a yield metric runs will
    /// find.
    #[must_use]
    pub fn knowledge_units(&self) -> u32 {
        self.findings_established
            + self.findings_refuted
            + self.negatives
            + self.negatives_new_class * NEW_CLASS_BONUS
            + self.questions_closed
            + self.contradictions_surfaced
    }

    /// A run that spent trials and produced nothing (§10).
    ///
    /// **Trials, not steps.** A run that spent no trial and produced nothing is
    /// idle, or answering a lookup, or refused — none of which is the failure
    /// this flags. Barren means the platform's scarcest resource was consumed
    /// and nothing came back.
    #[must_use]
    pub fn is_barren(&self) -> bool {
        self.trials_spent > 0 && self.knowledge_units() == 0
    }

    /// Dollars per unit of knowledge, or `None` when there are none.
    ///
    /// `None` rather than infinity, and rather than the cost itself: a barren run
    /// has no cost *per unit* because it produced no units, and reporting the raw
    /// cost there would put a finite number in a column readers average.
    #[must_use]
    pub fn cost_per_knowledge_unit(&self) -> Option<f64> {
        let units = self.knowledge_units();
        (units > 0).then(|| self.cost_usd / f64::from(units))
    }

    /// Count one finding at its verified confidence.
    pub fn record_finding(&mut self, confidence: Confidence) {
        match confidence {
            Confidence::Established => self.findings_established += 1,
            Confidence::Suggestive => self.findings_suggestive += 1,
            Confidence::Refuted => self.findings_refuted += 1,
        }
    }

    /// Count one negative result, saying whether its class is new for the
    /// subject.
    ///
    /// The caller decides "new", because it is a question about the memory store
    /// and this crate is sans-IO — and because a run cannot be trusted to answer
    /// it about itself.
    pub fn record_negative(&mut self, class_is_new_for_subject: bool) {
        self.negatives += 1;
        if class_is_new_for_subject {
            self.negatives_new_class += 1;
        }
    }
}

/// One row of the barren report (AGENT-009 §4, amended).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarrenRate {
    pub tier: String,
    pub workflow: String,
    pub runs: u32,
    pub barren: u32,
}

impl BarrenRate {
    #[must_use]
    pub fn rate(&self) -> Option<f64> {
        (self.runs > 0).then(|| f64::from(self.barren) / f64::from(self.runs))
    }
}

/// Barren rate per `(tier, workflow)`.
///
/// **Never averaged across tiers.** "12% barren" over everything is a number
/// nobody can act on; "`survey` on the 7B is barren 60% of the time" names a
/// workflow and a tier and is therefore something somebody can change. Same
/// argument AGENT-009 §4 makes about pass rates, applied to the thing that
/// measures whether a pass was worth having.
#[must_use]
pub fn barren_rates(runs: &[(String, String, Yield)]) -> Vec<BarrenRate> {
    let mut rows: std::collections::BTreeMap<(String, String), (u32, u32)> =
        std::collections::BTreeMap::new();
    for (tier, workflow, y) in runs {
        let e = rows.entry((tier.clone(), workflow.clone())).or_insert((0, 0));
        e.0 += 1;
        if y.is_barren() {
            e.1 += 1;
        }
    }
    rows.into_iter()
        .map(|((tier, workflow), (runs, barren))| BarrenRate {
            tier,
            workflow,
            runs,
            barren,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_that_spent_trials_and_learned_nothing_is_barren() {
        let y = Yield {
            trials_spent: 9,
            ..Yield::default()
        };
        assert!(y.is_barren());
        assert_eq!(y.knowledge_units(), 0);
        assert_eq!(y.cost_per_knowledge_unit(), None);
    }

    /// Spending no trial is not barren. It is idle, or a lookup, or a refusal.
    #[test]
    fn a_run_that_spent_nothing_is_not_barren() {
        let y = Yield::default();
        assert!(
            !y.is_barren(),
            "barren means the scarcest resource was consumed and nothing came back"
        );
    }

    /// A negative result is knowledge, and a new class for the subject is more.
    #[test]
    fn proving_something_is_noise_counts() {
        let mut y = Yield {
            trials_spent: 3,
            ..Yield::default()
        };
        y.record_negative(false);
        assert_eq!(y.knowledge_units(), 1);
        assert!(!y.is_barren());

        let mut y = Yield {
            trials_spent: 3,
            ..Yield::default()
        };
        y.record_negative(true);
        assert_eq!(
            y.knowledge_units(),
            2,
            "a class nobody had recorded for this subject closes a door nobody had closed"
        );
    }

    /// A suggestive finding is a lead, not knowledge.
    ///
    /// Counting it would make "record six weak findings" the cheapest way to
    /// look productive, and a yield metric with an open route like that is one
    /// runs will find.
    #[test]
    fn weak_findings_cannot_buy_a_yield() {
        let mut y = Yield {
            trials_spent: 4,
            ..Yield::default()
        };
        for _ in 0..6 {
            y.record_finding(Confidence::Suggestive);
        }
        assert_eq!(y.findings_suggestive, 6, "they are counted");
        assert_eq!(y.knowledge_units(), 0, "and they are not scored");
        assert!(y.is_barren());

        y.record_finding(Confidence::Established);
        assert_eq!(y.knowledge_units(), 1);
        assert!(!y.is_barren());
    }

    /// A refuted finding is knowledge, not a failure to produce any.
    ///
    /// The same argument `Confidence::weight` makes about retrieval: knowing an
    /// idea is dead is the point of keeping it. A metric that paid nothing for a
    /// refutation would be a metric that discouraged checking.
    #[test]
    fn a_refuted_finding_is_something_the_platform_learned() {
        let mut y = Yield {
            trials_spent: 2,
            ..Yield::default()
        };
        y.record_finding(Confidence::Refuted);
        assert_eq!(y.knowledge_units(), 1);
        assert!(!y.is_barren());
    }

    /// Opening a question is not producing knowledge.
    #[test]
    fn a_run_that_only_opened_questions_has_left_more_work_than_it_did() {
        let y = Yield {
            trials_spent: 2,
            questions_opened: 4,
            ..Yield::default()
        };
        assert!(y.is_barren());

        let y = Yield {
            questions_closed: 1,
            trials_spent: 2,
            ..y
        };
        assert_eq!(y.knowledge_units(), 1);
    }

    /// A proposal is a request for a decision, not a result.
    #[test]
    fn a_proposal_is_not_a_result() {
        let y = Yield {
            trials_spent: 5,
            proposals: 2,
            ..Yield::default()
        };
        assert!(
            y.is_barren(),
            "the decision is where the value is, and it has not been made"
        );
    }

    /// A surfaced contradiction is a finding under P5.
    #[test]
    fn a_disagreement_the_run_surfaced_is_knowledge() {
        let y = Yield {
            trials_spent: 1,
            contradictions_surfaced: 1,
            ..Yield::default()
        };
        assert_eq!(y.knowledge_units(), 1);
        assert!(!y.is_barren());
    }

    /// Cost per unit is `None` with no units, not infinity and not the raw cost.
    #[test]
    fn a_barren_run_has_no_cost_per_unit() {
        let y = Yield {
            trials_spent: 3,
            cost_usd: 4.20,
            ..Yield::default()
        };
        assert_eq!(
            y.cost_per_knowledge_unit(),
            None,
            "a finite number here lands in a column readers average"
        );

        let y = Yield {
            findings_established: 2,
            ..y
        };
        let per = y.cost_per_knowledge_unit().expect("two units");
        assert!((per - 2.10).abs() < 1e-9, "{per}");
    }

    /// Barren rate is per (tier, workflow) and never averaged across them.
    #[test]
    fn the_barren_rate_names_a_tier_and_a_workflow() {
        let barren = Yield {
            trials_spent: 3,
            ..Yield::default()
        };
        let productive = Yield {
            trials_spent: 3,
            findings_established: 1,
            ..Yield::default()
        };
        let rows = barren_rates(&[
            ("degraded".into(), "survey".into(), barren.clone()),
            ("degraded".into(), "survey".into(), barren.clone()),
            ("degraded".into(), "investigate".into(), productive.clone()),
            ("frontier".into(), "survey".into(), productive),
        ]);
        assert_eq!(rows.len(), 3, "three (tier, workflow) pairs");
        let survey_7b = rows
            .iter()
            .find(|r| r.tier == "degraded" && r.workflow == "survey")
            .expect("the pair that is failing");
        assert_eq!(survey_7b.runs, 2);
        assert_eq!(survey_7b.barren, 2);
        assert!((survey_7b.rate().unwrap() - 1.0).abs() < f64::EPSILON);

        let frontier = rows.iter().find(|r| r.tier == "frontier").unwrap();
        assert_eq!(frontier.barren, 0, "and it is not smeared across the tiers");
    }

    /// A pair with no runs reports no rate rather than zero.
    #[test]
    fn a_pair_nobody_ran_has_no_rate() {
        let row = BarrenRate {
            tier: "frontier".into(),
            workflow: "optimise".into(),
            runs: 0,
            barren: 0,
        };
        assert_eq!(
            row.rate(),
            None,
            "0% barren over zero runs reads as a tier that works"
        );
    }
}
