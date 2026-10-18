//! Move memory with a confidence-gated veto (Set R-b RB-1.3, KM-04; AGENT-003 §3;
//! FEAT-004 §6).
//!
//! A structural move is a typed edit ([`crate::propose::Move`]). Most are cheap to try
//! and a few are known not to work: the same *shape* of edit, tried before, failed.
//! This remembers the shape and what happened, and before a move is submitted answers
//! one question: has this motif a record of failure, and how sure are we?
//!
//! # The motif
//!
//! [`motif`] is the move with everything instrument- and node-specific taken out: kind
//! plus the part of the payload that is the *idea*. Numerals are replaced by `#` and
//! node ids lose their digits, so "AND `rsi_14 < 30`" and "AND `rsi_21 < 25`" are one
//! motif. Two moves with one motif are the same experiment as far as memory is
//! concerned, which is the point.
//!
//! # The veto is a posterior, not a count
//!
//! With `f` failures in `n` tries and a uniform prior on the failure rate, the
//! probability that the true failure rate exceeds one half is the Beta(`f + 1`,
//! `n - f + 1`) tail, which for integers is a binomial sum and needs no special
//! function. A single failure gives 0.75, which is why one failure never blocks: it
//! warns. [`VetoPolicy`] holds the two confidences and the minimum failure count, and
//! says so in the verdict, so a refusal states the number it acted on.
//!
//! * below `warn_confidence`: [`Veto::Allow`];
//! * at or above `warn_confidence`: [`Veto::Warn`], advisory, the move proceeds;
//! * at or above `block_confidence` **and** at least `min_failures` failures:
//!   [`Veto::Block`], citing the records.
//!
//! A block can be overridden only by a human (the route that records an override is
//! admin-scoped); the veto itself is never silent about what it cited.

use serde::{Deserialize, Serialize};

use crate::propose::Move;

/// What happened when a move was tried.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The objective improved on the held comparison.
    Improved,
    /// It did not help, or hurt.
    Failed,
    /// Inconclusive; counts toward neither side.
    Neutral,
}

impl Outcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Improved => "improved",
            Self::Failed => "failed",
            Self::Neutral => "neutral",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "improved" => Self::Improved,
            "failed" => Self::Failed,
            "neutral" => Self::Neutral,
            _ => return None,
        })
    }
}

/// One remembered try.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveRecord {
    pub motif: String,
    pub outcome: Outcome,
    /// Where the evidence is: a campaign or run id the veto cites.
    pub run_ref: String,
}

fn abstract_numbers(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_num = false;
    for c in s.chars() {
        if c.is_ascii_digit() || (in_num && c == '.') {
            if !in_num {
                out.push('#');
                in_num = true;
            }
        } else {
            in_num = false;
            out.push(c);
        }
    }
    out
}

fn normal_expr(s: &str) -> String {
    abstract_numbers(&s.split_whitespace().collect::<Vec<_>>().join(" ")).to_ascii_lowercase()
}

/// The canonical motif of a move (see the module docs).
#[must_use]
pub fn motif(m: &Move) -> String {
    match m {
        Move::AddConditionFilter { expr, .. } => format!("add_condition_filter:{}", normal_expr(expr)),
        Move::RemoveNode { node } => format!("remove_node:{}", abstract_numbers(&node.to_ascii_lowercase())),
        Move::SwapFeature { from, to, .. } => format!(
            "swap_feature:{}->{}",
            abstract_numbers(&from.to_ascii_lowercase()),
            abstract_numbers(&to.to_ascii_lowercase())
        ),
        Move::ChangeExit { mode } => format!("change_exit:{mode:?}").to_ascii_lowercase(),
        Move::AddRegimeGate { regime } => format!("add_regime_gate:{}", regime.to_ascii_lowercase()),
        Move::ChangeSizing { mode, .. } => format!("change_sizing:{mode:?}").to_ascii_lowercase(),
        Move::Simplify => "simplify".to_string(),
        Move::Combine { how, .. } => format!("combine:{how:?}").to_ascii_lowercase(),
    }
}

/// `P(failure rate > 1/2)` after `f` failures in `n` tries, uniform prior.
///
/// `P(p > 1/2 | Beta(f+1, n-f+1)) = P(Binomial(n+1, 1/2) <= f)`.
#[must_use]
pub fn failure_confidence(f: u32, n: u32) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let m = n + 1;
    let mut log_c = 0.0_f64; // ln C(m, 0)
    let mut total = 0.0;
    for k in 0..=f.min(m) {
        total += (log_c + f64::from(m) * 0.5_f64.ln()).exp();
        // ln C(m, k+1) = ln C(m, k) + ln((m-k)/(k+1))
        log_c += (f64::from(m - k) / f64::from(k + 1)).ln();
    }
    total.min(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VetoPolicy {
    pub warn_confidence: f64,
    pub block_confidence: f64,
    pub min_failures: u32,
}

impl Default for VetoPolicy {
    fn default() -> Self {
        Self {
            warn_confidence: 0.70,
            block_confidence: 0.95,
            min_failures: 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Veto {
    Allow {
        tries: u32,
        failures: u32,
    },
    Warn {
        tries: u32,
        failures: u32,
        confidence: f64,
        cites: Vec<String>,
        why: String,
    },
    Block {
        tries: u32,
        failures: u32,
        confidence: f64,
        cites: Vec<String>,
        why: String,
    },
}

impl Veto {
    #[must_use]
    pub fn blocks(&self) -> bool {
        matches!(self, Self::Block { .. })
    }
}

/// Decide for one motif over its records. Records of other motifs are ignored.
#[must_use]
pub fn check(motif_key: &str, records: &[MoveRecord], policy: &VetoPolicy) -> Veto {
    let mine: Vec<&MoveRecord> = records.iter().filter(|r| r.motif == motif_key).collect();
    let tries = u32::try_from(mine.iter().filter(|r| r.outcome != Outcome::Neutral).count()).unwrap_or(u32::MAX);
    let failures = u32::try_from(mine.iter().filter(|r| r.outcome == Outcome::Failed).count()).unwrap_or(u32::MAX);
    let confidence = failure_confidence(failures, tries);
    let cites: Vec<String> = mine
        .iter()
        .filter(|r| r.outcome == Outcome::Failed)
        .map(|r| r.run_ref.clone())
        .collect();
    let why = format!(
        "motif `{motif_key}` failed {failures} of {tries} tries; P(failure rate > 0.5) = {confidence:.3}"
    );
    if confidence >= policy.block_confidence && failures >= policy.min_failures {
        Veto::Block { tries, failures, confidence, cites, why }
    } else if confidence >= policy.warn_confidence && failures > 0 {
        Veto::Warn { tries, failures, confidence, cites, why }
    } else {
        Veto::Allow { tries, failures }
    }
}

/// Check a move against its history. The one-liner a submission path calls.
#[must_use]
pub fn veto_move(m: &Move, records: &[MoveRecord], policy: &VetoPolicy) -> Veto {
    check(&motif(m), records, policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::propose::ExitMode;

    fn rec(m: &str, o: Outcome, i: usize) -> MoveRecord {
        MoveRecord {
            motif: m.into(),
            outcome: o,
            run_ref: format!("camp:{i}"),
        }
    }

    #[test]
    fn instrument_specific_numbers_and_node_digits_are_not_part_of_the_motif() {
        let a = Move::AddConditionFilter { node: "sig1".into(), expr: "rsi_14 < 30".into() };
        let b = Move::AddConditionFilter { node: "sig2".into(), expr: "RSI_21  <  25.5".into() };
        assert_eq!(motif(&a), motif(&b));
        let c = Move::AddConditionFilter { node: "sig1".into(), expr: "adx_14 > 20".into() };
        assert_ne!(motif(&a), motif(&c));
        assert_eq!(motif(&Move::ChangeExit { mode: ExitMode::Trail }), "change_exit:trail");
    }

    #[test]
    fn the_posterior_matches_the_binomial_identity() {
        // f=0, n=0: no evidence.
        assert_eq!(failure_confidence(0, 0), 0.0);
        // 1 failure in 1 try: Beta(2,1): P(p>.5) = 0.75.
        assert!((failure_confidence(1, 1) - 0.75).abs() < 1e-12);
        // 3 of 3: Beta(4,1): 1 - 0.5^4 = 0.9375.
        assert!((failure_confidence(3, 3) - 0.9375).abs() < 1e-12);
        // 0 of 5: Beta(1,6): 0.5^6 = 0.015625.
        assert!((failure_confidence(0, 5) - 0.015625).abs() < 1e-12);
        // monotone in failures
        assert!(failure_confidence(4, 6) < failure_confidence(5, 6));
    }

    #[test]
    fn enough_failures_veto_the_move_and_cite_the_records() {
        let m = "add_condition_filter:rsi_# < #";
        let recs: Vec<MoveRecord> = (0..6).map(|i| rec(m, Outcome::Failed, i)).collect();
        let v = check(m, &recs, &VetoPolicy::default());
        match v {
            Veto::Block { failures, cites, why, confidence, .. } => {
                assert_eq!(failures, 6);
                assert_eq!(cites.len(), 6);
                assert!(why.contains("failed 6 of 6"));
                assert!(confidence >= 0.95);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn below_the_confidence_it_only_warns_and_a_success_history_allows() {
        let m = "simplify";
        // One failure: 0.75 confidence, one failure: advisory.
        let v = check(m, &[rec(m, Outcome::Failed, 1)], &VetoPolicy::default());
        assert!(matches!(v, Veto::Warn { .. }), "{v:?}");
        assert!(!v.blocks());
        // Three failures are 0.9375, short of 0.95 and short of min_failures: still a warning.
        let three: Vec<MoveRecord> = (0..3).map(|i| rec(m, Outcome::Failed, i)).collect();
        assert!(!check(m, &three, &VetoPolicy::default()).blocks());
        // Mostly improvements: allowed.
        let mixed: Vec<MoveRecord> = (0..5)
            .map(|i| rec(m, if i == 0 { Outcome::Failed } else { Outcome::Improved }, i))
            .collect();
        assert!(matches!(check(m, &mixed, &VetoPolicy::default()), Veto::Allow { tries: 5, failures: 1 }));
    }

    #[test]
    fn other_motifs_and_neutral_tries_do_not_count() {
        let recs = vec![
            rec("a", Outcome::Failed, 1),
            rec("b", Outcome::Failed, 2),
            rec("b", Outcome::Neutral, 3),
        ];
        assert!(matches!(check("c", &recs, &VetoPolicy::default()), Veto::Allow { tries: 0, failures: 0 }));
        match check("b", &recs, &VetoPolicy::default()) {
            Veto::Warn { tries, failures, .. } => assert_eq!((tries, failures), (1, 1)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stricter_policy_blocks_sooner_and_the_policy_is_an_argument() {
        let m = "x";
        let recs: Vec<MoveRecord> = (0..2).map(|i| rec(m, Outcome::Failed, i)).collect();
        let strict = VetoPolicy { warn_confidence: 0.5, block_confidence: 0.8, min_failures: 2 };
        assert!(check(m, &recs, &strict).blocks());
        assert!(!check(m, &recs, &VetoPolicy::default()).blocks());
    }
}
