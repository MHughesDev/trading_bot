//! The research-process model ladder and its computed triggers (Set R-b RB-5.1 to
//! RB-5.6; ADR-P4-01).
//!
//! Every learned model in the ladder sits behind a **ledger-size trigger**, and until
//! the trigger holds the rule tier answers. A learned model fitted on a ledger too
//! small to train it is confidently wrong, which is worse than a median rule that
//! knows it is a median rule (ADR-RB-02). So "not yet" has to be a *computed fact*:
//! [`decide`] takes the ledger's counts and returns which tier answers and which
//! numbers fell short, and a count the ledger cannot supply is `None`, which is
//! **not met**, never assumed.
//!
//! | model | rule tier | learned tier trigger |
//! |---|---|---|
//! | M3 learning curve | median stopping rule | at least 1 000 partial curves, at least 20% run to completion |
//! | M4 surrogate | the searcher's own model | at least 1 500 runs across at least 30 tasks |
//! | M5 gate pre-screener | monotone margin rule at 99% recall | at least 500 gated candidates and at least 100 passes |
//! | M8 family recommender | global family win rates | at least 100 tasks, per tenant always |
//! | M9 proposal ranker | surrogate mean + k sigma | at least 3 000 ranked items in at least 100 groups |
//! | M13 step critic | rules + judge triage | at least 3 000 labelled steps |
//!
//! The thresholds are the ones PHASE-2-5-PLAN §4 states; they are constants here so
//! the trigger a reader sees is the trigger that is checked.
//!
//! # What this does not build
//!
//! The learned models themselves. Each needs data this ledger has not accumulated;
//! the trigger says by how much. When one crosses, the learned tier is built, runs
//! **shadowed** first (it logs what it would have done beside the rule tier's answer,
//! with `decision_tier` on every decision) and only then answers.

use serde::{Deserialize, Serialize};

/// What the ledger holds, as counts. `None` is "not measured here", never zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LedgerCounts {
    pub curves: Option<u64>,
    /// Share of partial curves whose run went to completion, in `[0, 1]`.
    pub curve_completion_share: Option<f64>,
    pub runs: Option<u64>,
    pub tasks: Option<u64>,
    pub gated_candidates: Option<u64>,
    pub gate_passes: Option<u64>,
    pub ranked_items: Option<u64>,
    pub ranked_groups: Option<u64>,
    pub labelled_steps: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Model {
    M3,
    M4,
    M5,
    M8,
    M9,
    M13,
}

impl Model {
    pub const ALL: [Model; 6] = [Model::M3, Model::M4, Model::M5, Model::M8, Model::M9, Model::M13];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::M3 => "learning_curve_extrapolator",
            Self::M4 => "config_to_outcome_surrogate",
            Self::M5 => "gate_pre_screener",
            Self::M8 => "family_recommender",
            Self::M9 => "proposal_ranker",
            Self::M13 => "step_critic",
        }
    }

    #[must_use]
    pub const fn rule_tier(self) -> &'static str {
        match self {
            Self::M3 => "median_stopping_rule",
            Self::M4 => "searcher_internal_model",
            Self::M5 => "monotone_margin_rule_99_recall",
            Self::M8 => "global_family_win_rates",
            Self::M9 => "surrogate_mean_plus_kappa_sigma",
            Self::M13 => "rules_plus_judge_triage",
        }
    }

    /// M8 is the one that would broadcast an edge from one tenant to another, so it is
    /// per-tenant whatever the counts say (ADR-P4-01).
    #[must_use]
    pub const fn per_tenant_only(self) -> bool {
        matches!(self, Self::M8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Rule,
    /// The trigger holds. The learned tier runs shadowed before it answers.
    LearnedShadow,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Decision {
    pub model: Model,
    pub decision_tier: Tier,
    pub rule_tier: &'static str,
    /// Each clause that fell short, with the number it needed and the number found.
    pub shortfalls: Vec<String>,
}

fn need(shortfalls: &mut Vec<String>, label: &str, have: Option<u64>, want: u64) {
    match have {
        None => shortfalls.push(format!("{label}: not measured (needs {want})")),
        Some(h) if h < want => shortfalls.push(format!("{label}: {h} of {want}")),
        Some(_) => {}
    }
}

/// Which tier answers for `model` given the ledger's counts.
#[must_use]
pub fn decide(model: Model, c: &LedgerCounts) -> Decision {
    let mut s = Vec::new();
    match model {
        Model::M3 => {
            need(&mut s, "partial curves", c.curves, 1_000);
            match c.curve_completion_share {
                None => s.push("curve completion share: not measured (needs 0.20)".into()),
                Some(x) if x < 0.20 => s.push(format!("curve completion share: {x:.2} of 0.20")),
                Some(_) => {}
            }
        }
        Model::M4 => {
            need(&mut s, "runs", c.runs, 1_500);
            need(&mut s, "tasks", c.tasks, 30);
        }
        Model::M5 => {
            need(&mut s, "gated candidates", c.gated_candidates, 500);
            need(&mut s, "gate passes", c.gate_passes, 100);
        }
        Model::M8 => need(&mut s, "tasks", c.tasks, 100),
        Model::M9 => {
            need(&mut s, "ranked items", c.ranked_items, 3_000);
            need(&mut s, "ranked groups", c.ranked_groups, 100);
        }
        Model::M13 => need(&mut s, "labelled steps", c.labelled_steps, 3_000),
    }
    Decision {
        model,
        decision_tier: if s.is_empty() { Tier::LearnedShadow } else { Tier::Rule },
        rule_tier: model.rule_tier(),
        shortfalls: s,
    }
}

/// The whole ladder at once.
#[must_use]
pub fn ladder(c: &LedgerCounts) -> Vec<Decision> {
    Model::ALL.iter().map(|m| decide(*m, c)).collect()
}

/// M5's safety property, checkable on a held-out set before the pre-screener may
/// reject anything: it never rejects a candidate a gate would pass, to the stated
/// recall. `rejected_by_screener` and `passed_by_gate` are parallel booleans over the
/// held-out candidates.
///
/// # Errors
/// The recall found, when it is below `min_recall`.
pub fn screener_recall(rejected_by_screener: &[bool], passed_by_gate: &[bool], min_recall: f64) -> Result<f64, f64> {
    let passes: Vec<bool> = rejected_by_screener
        .iter()
        .zip(passed_by_gate)
        .filter(|(_, p)| **p)
        .map(|(r, _)| *r)
        .collect();
    if passes.is_empty() {
        return Err(0.0);
    }
    let kept = passes.iter().filter(|r| !**r).count() as f64 / passes.len() as f64;
    if kept + 1e-12 >= min_recall {
        Ok(kept)
    } else {
        Err(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> LedgerCounts {
        LedgerCounts {
            curves: Some(1_000),
            curve_completion_share: Some(0.2),
            runs: Some(1_500),
            tasks: Some(100),
            gated_candidates: Some(500),
            gate_passes: Some(100),
            ranked_items: Some(3_000),
            ranked_groups: Some(100),
            labelled_steps: Some(3_000),
        }
    }

    #[test]
    fn an_empty_ledger_leaves_every_tier_on_its_rule_and_says_what_is_missing() {
        for d in ladder(&LedgerCounts::default()) {
            assert_eq!(d.decision_tier, Tier::Rule, "{:?}", d.model);
            assert!(!d.shortfalls.is_empty());
            assert!(d.shortfalls.iter().all(|s| s.contains("not measured")), "{:?}", d.shortfalls);
        }
    }

    #[test]
    fn at_exactly_the_stated_counts_every_trigger_holds() {
        for d in ladder(&full()) {
            assert_eq!(d.decision_tier, Tier::LearnedShadow, "{:?}: {:?}", d.model, d.shortfalls);
        }
    }

    #[test]
    fn one_short_of_any_clause_keeps_the_rule_tier() {
        let mut c = full();
        c.gate_passes = Some(99);
        assert_eq!(decide(Model::M5, &c).decision_tier, Tier::Rule);
        assert!(decide(Model::M5, &c).shortfalls[0].contains("99 of 100"));
        let mut c = full();
        c.curve_completion_share = Some(0.19);
        assert_eq!(decide(Model::M3, &c).decision_tier, Tier::Rule);
        let mut c = full();
        c.tasks = Some(29);
        assert_eq!(decide(Model::M4, &c).decision_tier, Tier::Rule);
        c.tasks = Some(30);
        assert_eq!(decide(Model::M4, &c).decision_tier, Tier::LearnedShadow);
        assert_eq!(decide(Model::M8, &c).decision_tier, Tier::Rule, "M8 needs 100 tasks, not 30");
        c.tasks = Some(99);
        assert_eq!(decide(Model::M8, &c).decision_tier, Tier::Rule);
    }

    #[test]
    fn only_the_family_recommender_is_per_tenant_always() {
        assert!(Model::M8.per_tenant_only());
        assert!(Model::ALL.iter().filter(|m| m.per_tenant_only()).count() == 1);
    }

    #[test]
    fn the_prescreener_may_not_reject_what_a_gate_would_pass() {
        // 100 gate passes, the screener would reject one: recall 0.99 exactly.
        let passed = vec![true; 100];
        let mut rejected = vec![false; 100];
        rejected[3] = true;
        assert!(screener_recall(&rejected, &passed, 0.99).is_ok());
        rejected[4] = true;
        assert_eq!(screener_recall(&rejected, &passed, 0.99), Err(0.98));
        assert!(screener_recall(&[true], &[false], 0.99).is_err(), "no passes to measure against is not met");
    }
}
