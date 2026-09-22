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

// ── Spend: two states, because one of them would be a lie ───────────────────

/// What a phrase like "not priced" costs a reader who believed the zero.
///
/// Stated once, here, so the three surfaces that already say it and the ones
/// that will say it next say the same thing.
pub const NOT_PRICED: &str = "not priced — this platform records no per-run USD";

/// What a run cost, or the fact that nobody measured it.
///
/// # Why this is not an `f64`
///
/// It was one, and nothing in the workspace ever assigned it. Every dossier
/// this platform has ever compiled therefore carried the `Default`, `0.0`,
/// which the reader rendered as **`$0.00`** — a measurement, in the one
/// document a human opens to decide whether a strategy is worth money. An
/// absent measurement is never a zero, and a type with no way to say "absent"
/// gives the absence nowhere to go but into the number.
///
/// The platform already knew better everywhere else: `session_driver.rs`,
/// `routes/desk_running.rs` and `routes/agent_sessions.rs` each refuse to
/// price a run, because `llm_usage.cost_usd` keys on an **auth session**, not
/// on a run. The dossier was the last surface believing the zero.
///
/// # Why not `Option<f64>`
///
/// Because `Option<f64>` has `unwrap_or`, and the shortest path from "nobody
/// priced this run" to a rendered `$0.00` is one method call that reads as
/// defensive programming — the exact collapse that put the zero in the
/// document. Same argument [`Met`](crate::state::Met) makes about
/// `Option<bool>`, applied to the thing that turns into money.
///
/// # The compile-time part
///
/// There is deliberately **no** bare float anywhere on this type: no
/// `From<Spend>` conversion, no `is_priced`, no accessor handing back
/// `Option<f64>`. The only way out is [`Spend::fold`], which takes one
/// argument per state, so "render an unpriced run as a number" cannot be
/// written by omission — a caller has to type the unpriced arm and choose what
/// it says. `spend_never_grows_a_float_escape_hatch` in
/// `tests/acceptance_and_budget_are_honest.rs` keeps it that way.
///
/// # What this deliberately does not do
///
/// It does not invent a price. ADR-D-03 decided that a static price table is
/// the answer and that the estimate is `null` for any model not in it; that
/// table does not exist yet. Until it does, [`Spend::NotPriced`] is the true
/// value for every run this platform executes, and the job of this type is to
/// stop the lie rather than to build the pricing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Spend {
    /// Nobody measured it. The honest default, and the state of every run this
    /// platform has ever executed. **Not** a claim that the run was free.
    #[default]
    NotPriced,
    /// Measured, in US dollars. **Measured** is the load-bearing word: this is
    /// a figure something computed from a price, never the absence of one.
    Usd(f64),
}

impl Spend {
    /// Collapse to one value per state. Two arguments, no default, no bare
    /// float falling out.
    ///
    /// The priced arm takes the figure; the unpriced arm is a value, because
    /// there is nothing to hand it.
    pub fn fold<T>(self, priced: impl FnOnce(f64) -> T, not_priced: T) -> T {
        match self {
            Spend::Usd(usd) => priced(usd),
            Spend::NotPriced => not_priced,
        }
    }

    /// How a card says it.
    ///
    /// Words rather than a blank, because a blank beside a column of dollar
    /// figures reads as zero, and that is the reading this type exists to
    /// prevent.
    #[must_use]
    pub fn mark(self) -> String {
        self.fold(|usd| format!("${usd:.2}"), NOT_PRICED.to_string())
    }
}

/// `cost_usd: <number> | null` on the wire. `null` is [`Spend::NotPriced`], and
/// it is sent rather than skipped: an absent key lets a consumer's `??` or
/// `COALESCE` invent the zero that a missing measurement does not have.
impl Serialize for Spend {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Spend::NotPriced => s.serialize_none(),
            Spend::Usd(usd) => s.serialize_some(usd),
        }
    }
}

impl<'de> Deserialize<'de> for Spend {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Option::<f64>::deserialize(d)? {
            None => Spend::NotPriced,
            Some(usd) => Spend::Usd(usd),
        })
    }
}

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
    /// What the run cost, or the fact that nobody priced it — see [`Spend`].
    ///
    /// Named `spend` and not `cost_usd` because the field is no longer a number
    /// of dollars, and a name that promises one is how the zero got read as a
    /// measurement. It still **serialises** as `cost_usd`, now carrying `null`
    /// instead of `0.0`, so a consumer that reads the key gets the absence
    /// rather than losing the key and defaulting.
    #[serde(rename = "cost_usd", default)]
    pub spend: Spend,
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

    /// Dollars per unit of knowledge — three states, and two of them are
    /// absences of different things.
    ///
    /// * `None` — the run produced no units, so there is no *per unit* figure
    ///   at all. `None` rather than infinity, and rather than the cost itself:
    ///   reporting the raw cost there would put a finite number in a column
    ///   readers average.
    /// * `Some(Spend::NotPriced)` — it produced units, and nobody priced the
    ///   run. Distinct from the line above, because "learned nothing" and "we
    ///   do not know what it cost" are different facts about a run and a reader
    ///   acts on them differently.
    /// * `Some(Spend::Usd(_))` — both were measured.
    ///
    /// Folding the second case into the first would be the cheaper signature
    /// and would lose the distinction; folding it into a number would be the
    /// bug this module was rewritten to close.
    #[must_use]
    pub fn cost_per_knowledge_unit(&self) -> Option<Spend> {
        let units = self.knowledge_units();
        (units > 0).then(|| {
            self.spend
                .fold(|usd| Spend::Usd(usd / f64::from(units)), Spend::NotPriced)
        })
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
    ///
    /// The fixture prices the run deliberately: the assertion is about the
    /// *units* being absent, so the cost has to be present for it to mean
    /// anything. A run priced at `Spend::NotPriced` here would pass for the
    /// wrong reason.
    #[test]
    fn a_barren_run_has_no_cost_per_unit() {
        let y = Yield {
            trials_spent: 3,
            spend: Spend::Usd(4.20),
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
        let usd = per.fold(|usd| usd, f64::NAN);
        assert!((usd - 2.10).abs() < 1e-9, "{usd}");
    }

    /// The whole defect, as an assertion: a run nobody priced is not a free run.
    ///
    /// `Yield::default()` is what every run on this platform carries, because
    /// nothing in the workspace assigns a price. It used to be `0.0`, and it
    /// reached a stored dossier and printed `$0.00`.
    #[test]
    fn a_run_nobody_priced_is_not_a_run_that_cost_nothing() {
        let y = Yield {
            trials_spent: 3,
            findings_established: 2,
            ..Yield::default()
        };
        assert_eq!(
            y.spend,
            Spend::NotPriced,
            "nothing in this workspace assigns a price, so the default is the live value"
        );
        assert_eq!(
            y.cost_per_knowledge_unit(),
            Some(Spend::NotPriced),
            "two units and no price is not two units at zero dollars each"
        );
        assert_eq!(y.spend.mark(), NOT_PRICED);
        assert!(
            !y.spend.mark().contains('$'),
            "an unpriced run must not render with a currency figure: {}",
            y.spend.mark()
        );
    }

    /// `null` on the wire, not a missing key and not a zero.
    ///
    /// A missing key is what lets a consumer's `??` or a `COALESCE` invent the
    /// zero back; a present `null` is a statement.
    #[test]
    fn an_unpriced_run_serialises_as_null_under_the_key_a_reader_looks_for() {
        let wire = serde_json::to_value(Yield::default()).expect("serialises");
        assert_eq!(
            wire.get("cost_usd"),
            Some(&serde_json::Value::Null),
            "the key is sent, carrying null: {wire}"
        );

        let back: Yield = serde_json::from_value(wire).expect("round-trips");
        assert_eq!(back.spend, Spend::NotPriced);

        let priced = serde_json::to_value(Yield {
            spend: Spend::Usd(4.20),
            ..Yield::default()
        })
        .expect("serialises");
        assert_eq!(priced["cost_usd"], serde_json::json!(4.20));
        let back: Yield = serde_json::from_value(priced).expect("round-trips");
        assert_eq!(back.spend, Spend::Usd(4.20));
    }

    /// A row that omits the key entirely still deserialises to the absence.
    ///
    /// Rows were persisted before this field existed. They carry no price, so
    /// they read back as carrying no price — not as free runs.
    #[test]
    fn a_row_written_before_the_field_existed_is_unpriced_not_free() {
        let y: Yield = serde_json::from_value(serde_json::json!({
            "findings_established": 1,
            "findings_suggestive": 0,
            "findings_refuted": 0,
            "negatives": 0,
            "negatives_new_class": 0,
            "questions_opened": 0,
            "questions_closed": 0,
            "contradictions_surfaced": 0,
            "proposals": 0,
            "hypotheses_tested": 0,
            "hypotheses_refuted": 0,
            "hypotheses_supported": 0,
            "trials_spent": 2
        }))
        .expect("the old shape still reads");
        assert_eq!(y.spend, Spend::NotPriced);
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
