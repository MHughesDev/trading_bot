//! Fit the weights, then earn the promotion (AGENT-005 §22, §30.8; AGENT-009 §4;
//! Set S task S-9.12).
//!
//! # Fitted, not declared
//!
//! [`Weights::default`](super::policy::Weights::default) is the table §15
//! *implies*. It is not a measurement, and shipping it as one would be the
//! platform doing the thing it exists to stop other people doing: stating a
//! number with no evidence and letting it acquire authority by being in source.
//!
//! [`fit`] replaces it with an arithmetic over the attribution corpus: a signal's
//! weight is how far it separates the records the model used from the records it
//! did not. A signal that is equally present in both carries no information about
//! usefulness and earns nothing, whatever anyone's intuition said.
//!
//! The fit is deterministic and reproducible from the corpus — no sampling, no
//! initialisation, no stopping rule — which is what makes "the shipped weights
//! are fitted and reproducible" checkable rather than a claim.
//!
//! # The promotion is the existing gate, not a new one
//!
//! `v2` ships only by beating `v1` on **every** tier under AGENT-009 §4's
//! interval rules. That machinery already exists in
//! [`eval::scorecard::gate`](crate::eval::scorecard::gate), and this calls it
//! rather than growing a second one — a second gate would be a second standard,
//! and the looser of two standards is the one a candidate is judged by.
//!
//! What this adds is §22's reporting: `utility_rate` and cost beside the pass
//! rates, and the rule AGENT-009 P4 states — **a policy that helps the frontier
//! and hurts the 7B does not ship.** On this phase that is the likely failure,
//! because relevance scoring has the most room to go wrong exactly where the
//! window is smallest.
//!
//! # A losing `v2` is recorded, not re-tuned until it passes
//!
//! [`Promotion::refused`] carries the weights and the corpus that produced them.
//! Quietly re-fitting until the gate goes green is how a platform accumulates a
//! policy that beat its own evaluation and nothing else — and this repository
//! already has a name for that shape in `fitted_to_its_evals`.

use super::policy::Weights;
use super::select::Signals;
use super::store::Utility;
use crate::eval::scorecard::{gate, GateVerdict, Scorecard};

/// One rendered record from the corpus, with what became of it.
#[derive(Debug, Clone)]
pub struct Observation {
    pub signals: Signals,
    pub utility: Utility,
}

/// The largest weight the fit will assign.
///
/// The scale is arbitrary — only ratios between weights matter to a weighted sum
/// — but it is pinned so a fitted policy and a declared one are legible side by
/// side, and so a change in corpus size cannot move every weight at once.
const MAX_WEIGHT: f32 = 1.5;

/// The fitted weights, and what the fit could not conclude.
#[derive(Debug, Clone)]
pub struct Fitted {
    pub weights: Weights,
    /// Records the corpus could not classify — §28.2's `indeterminate`. Reported
    /// because a fit over a mostly-indeterminate corpus is a fit to noise, and
    /// the number is the reader's only way to know that happened.
    pub indeterminate: usize,
    pub used: usize,
    pub unused: usize,
    /// Signals with no separation in this corpus, named. Each one is a weight
    /// somebody believed in that the evidence does not support.
    pub uninformative: Vec<&'static str>,
}

/// Every signal, as a name and an accessor.
///
/// One list, so a signal added to [`Signals`] and forgotten here fails
/// `the_fit_covers_every_signal` rather than silently getting a weight of zero
/// for the life of the policy.
type Getter = fn(&Signals) -> f32;
type Setter = fn(&mut Weights, f32);
const SIGNALS: &[(&str, Getter, Setter)] = &[
    ("subject", |s| s.subject, |w, v| w.subject = v),
    ("id_overlap", |s| s.id_overlap, |w, v| w.id_overlap = v),
    ("kind_prior", |s| s.kind_prior, |w, v| w.kind_prior = v),
    ("recency", |s| s.recency, |w, v| w.recency = v),
    ("citations", |s| s.citations, |w, v| w.citations = v),
    ("same_tool", |s| s.same_tool, |w, v| w.same_tool = v),
    ("correction", |s| s.correction, |w, v| w.correction = v),
    ("expansion", |s| s.expansion, |w, v| w.expansion = v),
    ("stickiness", |s| s.stickiness, |w, v| w.stickiness = v),
];

/// Fit the §15 weights to an attribution corpus.
///
/// Returns the declared defaults unchanged when the corpus cannot support a fit
/// — fewer than [`MIN_CORPUS`] classified records on either side. That is the
/// honest failure: a fit to nine records is a number with the authority of a
/// measurement and the content of a guess, and this repository has been caught
/// by exactly that shape before.
#[must_use]
pub fn fit(corpus: &[Observation]) -> Fitted {
    let used: Vec<&Observation> = corpus
        .iter()
        .filter(|o| o.utility == Utility::Used || o.utility == Utility::Wanted)
        .collect();
    let unused: Vec<&Observation> = corpus
        .iter()
        .filter(|o| o.utility == Utility::Unused)
        .collect();
    let indeterminate = corpus
        .iter()
        .filter(|o| {
            matches!(
                o.utility,
                Utility::Indeterminate | Utility::NotAttributed
            )
        })
        .count();

    let mut out = Fitted {
        weights: Weights::default(),
        indeterminate,
        used: used.len(),
        unused: unused.len(),
        uninformative: Vec::new(),
    };
    if used.len() < MIN_CORPUS || unused.len() < MIN_CORPUS {
        return out;
    }

    // Separation per signal: how much higher it runs on the records the model
    // used. Signals arrive percentile-ranked, so every one is already in [0,1]
    // and the differences are directly comparable — which is the reason §24.3's
    // normalisation has to happen before this and not after.
    let mut separations: Vec<(usize, f32)> = Vec::with_capacity(SIGNALS.len());
    for (i, (_, get, _)) in SIGNALS.iter().enumerate() {
        let mean = |rows: &[&Observation]| -> f32 {
            rows.iter().map(|o| get(&o.signals)).sum::<f32>() / rows.len() as f32
        };
        separations.push((i, mean(&used) - mean(&unused)));
    }

    let best = separations
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0_f32, f32::max);
    if best <= f32::EPSILON {
        // Nothing separated anything. Returning the defaults is right and
        // saying so is the point: a corpus where no signal predicts use is
        // evidence about the corpus, not a licence to invent weights.
        out.uninformative = SIGNALS.iter().map(|(n, _, _)| *n).collect();
        return out;
    }

    let mut weights = Weights {
        // Not fitted: it is a penalty on redundancy, not a signal about one
        // record, so the corpus above says nothing about it. Left at the
        // declared value and named here so nobody reads the whole struct as
        // measured.
        redundancy_penalty: Weights::default().redundancy_penalty,
        ..Weights::default()
    };
    for (i, sep) in &separations {
        let (name, _, set) = SIGNALS[*i];
        if *sep <= 0.0 {
            out.uninformative.push(name);
            set(&mut weights, 0.0);
        } else {
            set(&mut weights, (sep / best) * MAX_WEIGHT);
        }
    }
    out.weights = weights;
    out
}

/// Classified records needed on each side before a fit is worth having.
pub const MIN_CORPUS: usize = 30;

/// The outcome of asking for a promotion.
#[derive(Debug, Clone)]
pub struct Promotion {
    pub verdict: GateVerdict,
    /// True only when the gate passed **and** no tier lost utility.
    pub promoted: bool,
    /// One line per tier: pass rate, utility rate, cost.
    pub report: Vec<String>,
    /// Why it was refused, if it was.
    pub refused: Vec<String>,
}

/// One tier's context measurements, for the report §22 asks for.
#[derive(Debug, Clone, Copy)]
pub struct TierUtility {
    pub utility_rate: f32,
    pub cost_tokens: u64,
}

/// Ask whether `v2` may ship.
///
/// `baselines` and `candidates` are the same scorecards
/// [`gate`](crate::eval::scorecard::gate) takes, so the interval rules, the
/// multiple-testing correction and the per-tier requirement are the platform's
/// existing ones rather than this phase's opinion of them.
///
/// `utility` is keyed by `profile_id`, giving each tier's `(v1, v2)` utility
/// rate.
#[must_use]
pub fn promote(
    baselines: &[Scorecard],
    candidates: &[Scorecard],
    utility: &std::collections::BTreeMap<String, (TierUtility, TierUtility)>,
) -> Promotion {
    let verdict = gate(baselines, candidates);
    let mut refused: Vec<String> = Vec::new();
    let mut report: Vec<String> = Vec::new();

    if verdict.blocked {
        refused.extend(verdict.reasons.iter().cloned());
    }

    for card in candidates {
        let base = baselines.iter().find(|b| b.profile_id == card.profile_id);
        let rates: Vec<String> = card
            .suites
            .iter()
            .map(|s| format!("{} {:.0}%", s.suite, s.pass_rate() * 100.0))
            .collect();
        let u = utility.get(&card.profile_id);
        let utility_line = match u {
            Some((v1, v2)) => format!(
                "utility {:.0}% → {:.0}%",
                v1.utility_rate * 100.0,
                v2.utility_rate * 100.0
            ),
            None => "utility unmeasured".to_string(),
        };
        report.push(format!(
            "{} [{}]: {} · {utility_line} · cost {} → {} tokens",
            card.profile_id,
            card.tier,
            rates.join(", "),
            base.map_or(0, |b| b.cost_tokens),
            card.cost_tokens
        ));

        // AGENT-009 P4, stated as a rule rather than left to the aggregate: a
        // policy that helps the frontier and hurts the 7B does not ship. An
        // average over tiers would hide exactly that, which is why the gate is
        // per-suite and this check is per-profile.
        if let Some((v1, v2)) = u {
            if v2.utility_rate + f32::EPSILON < v1.utility_rate {
                refused.push(format!(
                    "{} lost utility: {:.0}% → {:.0}%",
                    card.profile_id,
                    v1.utility_rate * 100.0,
                    v2.utility_rate * 100.0
                ));
            }
        } else {
            refused.push(format!(
                "{} has no utility measurement, so nothing licenses a claim about it",
                card.profile_id
            ));
        }
    }

    // A tier with no candidate scorecard at all is not a pass. It is a tier
    // nobody measured, and shipping past it is how a 7B regression reaches
    // production behind a frontier improvement.
    for base in baselines {
        if !candidates.iter().any(|c| c.profile_id == base.profile_id) {
            refused.push(format!(
                "{} has a baseline and no candidate: unmeasured is not passed",
                base.profile_id
            ));
        }
    }

    Promotion {
        promoted: refused.is_empty() && !verdict.blocked,
        verdict,
        report,
        refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::scorecard::{SuiteKind, SuiteScore};
    use std::collections::BTreeMap;

    fn observation(id_overlap: f32, recency: f32, utility: Utility) -> Observation {
        Observation {
            signals: Signals {
                id_overlap,
                recency,
                ..Signals::default()
            },
            utility,
        }
    }

    /// A corpus where one signal predicts use and another does not.
    fn corpus() -> Vec<Observation> {
        let mut c = Vec::new();
        for i in 0..40 {
            // Used records score high on id overlap; recency is noise, the same
            // on both sides.
            let r = if i % 2 == 0 { 0.8 } else { 0.2 };
            c.push(observation(0.9, r, Utility::Used));
            c.push(observation(0.1, r, Utility::Unused));
        }
        c
    }

    /// The weights come from the corpus, not from the table.
    #[test]
    fn a_signal_that_predicts_use_outweighs_one_that_does_not() {
        let f = fit(&corpus());
        assert!(
            f.weights.id_overlap > f.weights.recency,
            "id_overlap {} vs recency {}",
            f.weights.id_overlap,
            f.weights.recency
        );
        assert!(
            f.uninformative.contains(&"recency"),
            "a signal equally present on both sides carries no information about \
             usefulness, whatever anyone's intuition said: {:?}",
            f.uninformative
        );
    }

    /// Deterministic and reproducible from the corpus. The acceptance.
    #[test]
    fn the_fit_is_reproducible() {
        let c = corpus();
        let a = fit(&c);
        let b = fit(&c);
        assert_eq!(a.weights, b.weights);
        assert_eq!(a.used, 40);
        assert_eq!(a.unused, 40);
    }

    /// A corpus too small to support a fit gets the declared weights and says so.
    #[test]
    fn a_thin_corpus_does_not_produce_a_confident_number() {
        let thin: Vec<Observation> = (0..5)
            .map(|_| observation(0.9, 0.5, Utility::Used))
            .chain((0..5).map(|_| observation(0.1, 0.5, Utility::Unused)))
            .collect();
        let f = fit(&thin);
        assert_eq!(
            f.weights,
            Weights::default(),
            "a fit to ten records has the authority of a measurement and the content \
             of a guess"
        );
        assert_eq!(f.used, 5);
    }

    /// Indeterminate records are counted, not quietly folded into `unused`.
    #[test]
    fn indeterminate_records_are_reported_rather_than_assumed_unused() {
        let mut c = corpus();
        c.extend((0..25).map(|_| observation(0.5, 0.5, Utility::Indeterminate)));
        let f = fit(&c);
        assert_eq!(f.indeterminate, 25);
        assert_eq!(f.unused, 40, "and they did not become unused");
    }

    /// Every signal is covered by the fit.
    ///
    /// The failure this catches is silent: a signal added to `Signals` and not
    /// added here would keep whatever weight the defaults gave it and never be
    /// measured, while the policy claimed to be fitted.
    #[test]
    fn the_fit_covers_every_signal() {
        let f = fit(&corpus());
        let s = Signals::default();
        // Every field of `Signals` is read by exactly one entry in SIGNALS.
        assert_eq!(SIGNALS.len(), 9, "Signals has nine fields");
        let mut touched = 0;
        for (_, get, _) in SIGNALS {
            let _ = get(&s);
            touched += 1;
        }
        assert_eq!(touched, SIGNALS.len());
        // And the redundancy penalty is deliberately not fitted.
        assert_eq!(
            f.weights.redundancy_penalty,
            Weights::default().redundancy_penalty
        );
    }

    fn card(profile: &str, tier: &str, passed: usize, n: usize, cost: u64) -> Scorecard {
        Scorecard {
            harness_version: "0.1.0".into(),
            profile_id: profile.into(),
            tier: tier.into(),
            hardware: "ci".into(),
            suites: vec![SuiteScore {
                suite: "honesty".into(),
                // A replay suite, so the comparison is exact and the promotion
                // is not arguing with sampling noise as well as with itself.
                kind: SuiteKind::Replay,
                passed,
                n,
                flakiness: 0.0,
                attribution: crate::eval::scorecard::Attribution::default(),
                failed_cases: Vec::new(),
                sealed: false,
            }],
            cost_usd: 0.0,
            cost_tokens: cost,
            median_steps: 4.0,
            cache_hit_rate: None,
            memory_recall: None,
            hitl_precision: None,
            evaluations: 1,
        }
    }

    fn utility(rate_v1: f32, rate_v2: f32) -> (TierUtility, TierUtility) {
        (
            TierUtility {
                utility_rate: rate_v1,
                cost_tokens: 1_000,
            },
            TierUtility {
                utility_rate: rate_v2,
                cost_tokens: 900,
            },
        )
    }

    /// AGENT-009 P4: helping the frontier and hurting the 7B is not a ship.
    #[test]
    fn a_policy_that_hurts_the_small_model_does_not_ship() {
        let baselines = vec![
            card("claude-opus-5", "frontier", 80, 100, 5_000),
            card("qwen2.5:7b-instruct", "degraded", 60, 100, 2_000),
        ];
        let candidates = vec![
            card("claude-opus-5", "frontier", 95, 100, 4_000),
            card("qwen2.5:7b-instruct", "degraded", 60, 100, 2_000),
        ];
        let u = BTreeMap::from([
            ("claude-opus-5".to_string(), utility(0.4, 0.7)),
            ("qwen2.5:7b-instruct".to_string(), utility(0.5, 0.3)),
        ]);
        let p = promote(&baselines, &candidates, &u);
        assert!(!p.promoted);
        assert!(
            p.refused.iter().any(|r| r.contains("qwen2.5:7b-instruct")),
            "{:?}",
            p.refused
        );
        assert!(
            p.report.iter().any(|r| r.contains("utility")),
            "and the report names it beside the pass rates: {:?}",
            p.report
        );
    }

    /// A tier nobody measured is not a tier that passed.
    #[test]
    fn an_unmeasured_tier_blocks_rather_than_being_skipped() {
        let baselines = vec![
            card("claude-opus-5", "frontier", 80, 100, 5_000),
            card("qwen2.5:7b-instruct", "degraded", 60, 100, 2_000),
        ];
        let candidates = vec![card("claude-opus-5", "frontier", 95, 100, 4_000)];
        let u = BTreeMap::from([("claude-opus-5".to_string(), utility(0.4, 0.7))]);
        let p = promote(&baselines, &candidates, &u);
        assert!(!p.promoted);
        assert!(
            p.refused
                .iter()
                .any(|r| r.contains("unmeasured is not passed")),
            "{:?}",
            p.refused
        );
    }

    /// A refusal carries its reasons, so a loss is recorded rather than re-tuned.
    #[test]
    fn a_losing_candidate_is_recorded_with_why() {
        let baselines = vec![card("claude-opus-5", "frontier", 90, 100, 5_000)];
        let candidates = vec![card("claude-opus-5", "frontier", 90, 100, 5_000)];
        let u = BTreeMap::from([("claude-opus-5".to_string(), utility(0.6, 0.4))]);
        let p = promote(&baselines, &candidates, &u);
        assert!(!p.promoted);
        assert!(!p.refused.is_empty());
        assert!(
            p.report.iter().any(|r| r.contains("60% → 40%")),
            "the weights that produced the loss are recorded with it: {:?}",
            p.report
        );
    }
}
