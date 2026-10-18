//! `compare_candidates` — SPEC §11.5 as one door (AGENT-013 §8.3; Set S task
//! S-10.10).
//!
//! # What was already here, and what was missing
//!
//! `backtest::stats::compare` holds all of it: paired replicates, anytime-valid
//! confidence sequences, the ROPE, the win-rate rule, Holm, the Multiple
//! Comparison Matrix, and `ComparisonPlan::judge` which cannot be called without
//! a gate profile. **Nothing outside that file called any of it.** The comparison
//! this platform actually shipped was `compare_backtests`, which puts headline
//! metrics side by side and lets a reader form an impression.
//!
//! So this module is the door, not the statistics. It sweeps the pairs, calls
//! `judge` on each, applies Holm across them, and assembles the matrix — and it
//! re-implements none of those things, because a second implementation of a
//! comparison is a second standard, and the looser one is the one a candidate
//! gets judged by.
//!
//! # The tool has no seed argument
//!
//! ADR-P2-02's technique. Nuisance sources are randomised from **platform-held**
//! seeds, and there is no parameter through which a caller can choose them —
//! because a caller that could would, eventually, choose the ones where its
//! candidate wins. [`replicate_grid`] takes a base seed the platform supplies and
//! this function never sees one.
//!
//! # Ranking is what the comparison produces, and it can produce none
//!
//! [`Ranking::Indistinguishable`] is a real answer and the common one. Two copies
//! of the same strategy must produce it, and a comparison that always ranked
//! would be a comparison that ranked noise.

use std::collections::BTreeMap;

use backtest::stats::compare::{holm_adjusted, ComparisonMatrix, ComparisonPlan, Measurement};
use serde::{Deserialize, Serialize};

/// One candidate's measured replicates.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    /// One measurement per replicate, in replicate order. The **same**
    /// replicates for every candidate: an unpaired difference estimates a
    /// different and much noisier quantity, and `judge` refuses one.
    pub measurements: Vec<Measurement>,
}

/// What the comparison concluded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ranking", rename_all = "snake_case")]
pub enum Ranking {
    /// Best first. Only candidates that beat something are ordered; the rest are
    /// reported as indistinguishable from each other.
    Ranked { order: Vec<String> },
    /// Nothing beat anything. The common answer, and a real one.
    Indistinguishable,
}

/// A sweep over every pair, with its evidence.
///
/// **Named `Sweep` and not `Comparison`** — `backtest::stats::compare::Comparison`
/// is the sealed one-pair verdict, and a static invariant greps for
/// `Comparison {` outside the protocol so that verdict cannot be constructed
/// without its context. A second type of that name here defeats a name-based
/// guard, which is the kind of guard that one is. The invariant caught it.
#[derive(Debug, Clone)]
pub struct Sweep {
    pub matrix: ComparisonMatrix,
    pub ranking: Ranking,
    /// `(a, b)` → Holm-adjusted descriptive p-value, in the order the pairs were
    /// judged. Descriptive: §11.5 is explicit that the Wilcoxon number is not a
    /// test anything turns on, and adjusting it does not make it one — it is
    /// adjusted so a reader comparing ten pairs is not reading ten unadjusted
    /// numbers.
    pub adjusted_p: BTreeMap<(String, String), f64>,
    /// Pairs that could not be judged, with why. Named rather than dropped: a
    /// matrix missing a cell for an unstated reason reads as a pair nobody
    /// tried.
    pub refused: Vec<(String, String, String)>,
}

/// Why a comparison could not run at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompareError {
    #[error("a comparison needs at least two candidates; got {0}")]
    TooFewCandidates(usize),
    #[error("candidate names must be distinct: `{0}` appears twice")]
    DuplicateName(String),
}

/// Compare every pair (§11.5).
///
/// `plan` carries the gate profile, the pre-registered `delta_practical`, the
/// ROPE scale and the replicate cap — none of which this function chooses, and
/// all of which `judge` refuses to work without.
///
/// # Errors
/// [`CompareError`] when there is nothing to compare or the names collide.
pub fn compare(candidates: &[Candidate], plan: &ComparisonPlan) -> Result<Sweep, CompareError> {
    if candidates.len() < 2 {
        return Err(CompareError::TooFewCandidates(candidates.len()));
    }
    for (i, c) in candidates.iter().enumerate() {
        if candidates[..i].iter().any(|d| d.name == c.name) {
            return Err(CompareError::DuplicateName(c.name.clone()));
        }
    }

    let mut matrix = ComparisonMatrix::new();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut raw_p: Vec<f64> = Vec::new();
    let mut refused = Vec::new();
    // `a ≻ b` for each pair that reached the promotion bar.
    let mut beats: Vec<(String, String)> = Vec::new();

    for (i, a) in candidates.iter().enumerate() {
        for b in &candidates[i + 1..] {
            match plan.judge(&a.measurements, &b.measurements) {
                Ok(comparison) => {
                    let differences: Vec<f64> = a
                        .measurements
                        .iter()
                        .zip(b.measurements.iter())
                        .map(|(x, y)| x.value - y.value)
                        .collect();
                    matrix.insert(&a.name, &b.name, &comparison, &differences);
                    raw_p.push(
                        matrix
                            .get(&a.name, &b.name)
                            .map_or(1.0, |c| c.wilcoxon_p_descriptive),
                    );
                    pairs.push((a.name.clone(), b.name.clone()));
                    // **Only `Promote` establishes an edge.** `Reject` means A's
                    // advantage is smaller than the declared delta — §11.5 says
                    // in as many words that it is *not* "B is better" — so
                    // reading it as an edge for B would invent a result the
                    // protocol declined to state.
                    //
                    // Which is why the reverse direction is judged too: `b ≻ a`
                    // is a `Promote` on `judge(b, a)` and is reachable no other
                    // way. Cheap, because judging is arithmetic over
                    // measurements that already exist.
                    if comparison.verdict().promotes() {
                        beats.push((a.name.clone(), b.name.clone()));
                    } else if plan
                        .judge(&b.measurements, &a.measurements)
                        .is_ok_and(|reverse| reverse.verdict().promotes())
                    {
                        beats.push((b.name.clone(), a.name.clone()));
                    }
                }
                Err(e) => refused.push((a.name.clone(), b.name.clone(), e.to_string())),
            }
        }
    }

    let adjusted = holm_adjusted(&raw_p);
    let adjusted_p: BTreeMap<(String, String), f64> = pairs.into_iter().zip(adjusted).collect();

    Ok(Sweep {
        ranking: rank(candidates, &beats),
        matrix,
        adjusted_p,
        refused,
    })
}

/// Order the candidates by how many others each one beat.
///
/// A count rather than a tournament resolution: `A ≻ B` and `B ≻ C` does not
/// give `A ≻ C` under a paired comparison, and inventing the transitive edge
/// would state evidence nobody produced. Candidates that beat nothing and were
/// beaten by nothing are simply absent from the order, which is what
/// "indistinguishable from the field" looks like.
fn rank(candidates: &[Candidate], beats: &[(String, String)]) -> Ranking {
    if beats.is_empty() {
        return Ranking::Indistinguishable;
    }
    let mut wins: BTreeMap<&str, usize> = BTreeMap::new();
    for c in candidates {
        wins.insert(c.name.as_str(), 0);
    }
    for (winner, _) in beats {
        *wins.entry(winner.as_str()).or_insert(0) += 1;
    }
    let mut ordered: Vec<(&str, usize)> = wins.into_iter().filter(|(_, n)| *n > 0).collect();
    // Ties break on the name, so the order is reproducible from the matrix
    // rather than depending on iteration.
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    Ranking::Ranked {
        order: ordered.into_iter().map(|(n, _)| n.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backtest::stats::compare::{ComparisonFlags, RopeScale};

    fn plan(delta: f64) -> ComparisonPlan {
        ComparisonPlan {
            profile_id: "strict_v1".into(),
            n_eff: 12.0,
            delta_practical: delta,
            scale: RopeScale::new(2.0).expect("a bound on a paired difference"),
            trials_a: 1,
            trials_b: 1,
            k_max: 40,
            flags: ComparisonFlags::default(),
            alpha: 0.05,
        }
    }

    /// 29 paired replicates, which is what §11.5 asks for.
    const K: usize = 29;

    fn candidate(name: &str, values: impl Fn(usize) -> f64) -> Candidate {
        candidate_at(name, K, values)
    }

    fn candidate_at(name: &str, k: usize, values: impl Fn(usize) -> f64) -> Candidate {
        Candidate {
            name: name.to_string(),
            measurements: backtest::stats::compare::replicate_grid(k, 0x5eed_0000_0000_0001)
                .into_iter()
                .enumerate()
                .map(|(i, replicate)| Measurement {
                    replicate,
                    value: values(i),
                })
                .collect(),
        }
    }

    /// Two copies of the same strategy are indistinguishable.
    ///
    /// The acceptance, and the one a comparison that always ranked would fail:
    /// a comparison that always ranks is a comparison that ranks noise.
    #[test]
    fn two_copies_of_one_strategy_are_indistinguishable() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.1;
        let a = candidate("a", noise);
        let b = candidate("b", noise);
        let c = compare(&[a, b], &plan(0.5)).expect("two candidates");
        assert_eq!(c.ranking, Ranking::Indistinguishable);
        assert!(c.refused.is_empty(), "{:?}", c.refused);
        assert!(c.matrix.get("a", "b").is_some(), "the cell still exists");
    }

    /// A planted dominant candidate is ranked first.
    ///
    /// **At more than 29 replicates, and that is the finding rather than a
    /// fixture convenience.** The anytime-valid sequence's width depends on the
    /// *declared* ROPE scale, not on the observed spread: with a half-width of
    /// 2.0 and a difference of 1.0, thirty replicates leave the lower bound just
    /// below zero and the protocol says `KeepSampling` — correctly. Declaring a
    /// loose bound costs replicates, which is a real property of the protocol
    /// and one a caller should know before declaring one.
    #[test]
    fn a_candidate_that_wins_every_replicate_is_ranked_first() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.05;
        let k = 400;
        let weak = candidate_at("weak", k, noise);
        let strong = candidate_at("strong", k, move |i| noise(i) + 1.0);
        let mut p = plan(0.5);
        p.k_max = k;
        let c = compare(&[weak, strong], &p).expect("two candidates");
        match &c.ranking {
            Ranking::Ranked { order } => {
                assert_eq!(order.first().map(String::as_str), Some("strong"))
            }
            other => panic!("expected a ranking, got {other:?}"),
        }
    }

    /// `Reject` is not an edge for the other side.
    ///
    /// §11.5 says in as many words that `Reject` means "A's advantage is smaller
    /// than the declared delta", not "B is better". Reading it as an edge for B
    /// would state a result the protocol declined to state — which is why the
    /// reverse direction is judged rather than inferred.
    #[test]
    fn a_reject_is_not_read_as_a_win_for_the_other_side() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.05;
        // A real but sub-delta advantage: `b` is better by 0.2 against a delta
        // of 0.5, so neither direction promotes and neither is ranked.
        let c = compare(
            &[
                candidate("a", noise),
                candidate("b", move |i| noise(i) + 0.2),
            ],
            &plan(0.5),
        )
        .expect("two candidates");
        assert_eq!(
            c.ranking,
            Ranking::Indistinguishable,
            "not by enough is not the same as the other one winning"
        );
    }

    /// The matrix stores both directions, so a reader never has to remember
    /// which way round a pair was computed.
    #[test]
    fn every_pair_appears_both_ways_round() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.05;
        let c = compare(
            &[
                candidate("a", noise),
                candidate("b", move |i| noise(i) + 1.0),
                candidate("c", move |i| noise(i) + 0.5),
            ],
            &plan(0.3),
        )
        .expect("three candidates");
        for (x, y) in [("a", "b"), ("b", "c"), ("a", "c")] {
            assert!(c.matrix.get(x, y).is_some(), "{x} vs {y}");
            assert!(c.matrix.get(y, x).is_some(), "{y} vs {x}");
        }
        assert_eq!(c.matrix.names().len(), 3);
    }

    /// Holm is applied across the pairs (§11.5).
    #[test]
    fn holm_runs_across_every_pair() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.05;
        let c = compare(
            &[
                candidate("a", noise),
                candidate("b", move |i| noise(i) + 1.0),
                candidate("c", move |i| noise(i) + 0.5),
            ],
            &plan(0.3),
        )
        .expect("three candidates");
        assert_eq!(c.adjusted_p.len(), 3, "three pairs");
        for ((x, y), adjusted) in &c.adjusted_p {
            let raw = c
                .matrix
                .get(x, y)
                .expect("the cell exists")
                .wilcoxon_p_descriptive;
            assert!(
                *adjusted >= raw - 1e-12,
                "Holm is more conservative, never less: {x}/{y} raw {raw} adjusted {adjusted}"
            );
            assert!((0.0..=1.0).contains(adjusted));
        }
    }

    /// A pair that cannot be judged is named, never dropped.
    ///
    /// A matrix missing a cell for an unstated reason reads as a pair nobody
    /// tried, which is the one reading that is never right.
    #[test]
    fn an_unjudgeable_pair_is_reported_rather_than_omitted() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.05;
        let mut short = candidate("short", noise);
        short.measurements.truncate(3);
        let c =
            compare(&[candidate("full", noise), short], &plan(0.5)).expect("the comparison runs");
        assert_eq!(c.refused.len(), 1);
        let (a, b, why) = &c.refused[0];
        assert_eq!((a.as_str(), b.as_str()), ("full", "short"));
        assert!(!why.is_empty(), "the reason travels with the pair");
        assert_eq!(c.ranking, Ranking::Indistinguishable, "nothing was judged");
    }

    /// Nothing to compare, and colliding names, are refused up front.
    #[test]
    fn a_comparison_needs_two_distinctly_named_candidates() {
        let noise = |i: usize| ((i as f64) * 0.37).sin() * 0.05;
        assert_eq!(
            compare(&[candidate("a", noise)], &plan(0.5)).err(),
            Some(CompareError::TooFewCandidates(1))
        );
        assert_eq!(
            compare(&[candidate("a", noise), candidate("a", noise)], &plan(0.5)).err(),
            Some(CompareError::DuplicateName("a".into()))
        );
    }

    /// The ranking is a win count, not a tournament resolution.
    ///
    /// `A ≻ B` and `B ≻ C` does not give `A ≻ C` under a paired comparison, and
    /// inventing the transitive edge would state evidence nobody produced.
    #[test]
    fn the_ranking_does_not_invent_a_transitive_edge() {
        let cands = [
            candidate("a", |_| 0.0),
            candidate("b", |_| 0.0),
            candidate("c", |_| 0.0),
        ];
        let beats = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "c".to_string()),
        ];
        let Ranking::Ranked { order } = rank(&cands, &beats) else {
            panic!("something beat something");
        };
        // `a` and `b` each won once, so they tie on count and break on name.
        // Nothing here claims `a ≻ c`.
        assert_eq!(order, vec!["a".to_string(), "b".to_string()]);
        assert!(!order.contains(&"c".to_string()), "c beat nothing");
    }

    /// This module chooses no seed and has nowhere to put one.
    ///
    /// ADR-P2-02's technique: a caller that could choose the nuisance seeds
    /// would, eventually, choose the ones where its candidate wins.
    #[test]
    fn there_is_no_seed_anywhere_in_this_door() {
        let source = include_str!("compare.rs");
        let body = source
            .split("#[cfg(test)]")
            .next()
            .expect("the module has a non-test half");
        assert!(
            !body.contains("seed:") && !body.contains("seed ="),
            "a seed argument here is a seed a caller can choose"
        );
    }
}
