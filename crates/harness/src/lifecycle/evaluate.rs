//! The platform writes the verdict (AGENT-013 §6.2, §6.9, P3; Set S tasks
//! S-10.4 and S-10.8).
//!
//! # Why this is a function and not a tool
//!
//! A run may *commit* to a falsifier, *bind* a design and *ask* for a
//! refinement. It may not decide whether its own hypothesis was supported.
//!
//! The reason is the one running through this whole system: a verdict the model
//! writes is a verdict the model can rewrite, and a run that can grade itself
//! will grade itself well. So `evaluate` reads the tickets. There is no argument
//! by which a model reaches it, and `record_finding` is corrected when it tries
//! — see [`claims_a_verdict`].
//!
//! # The falsifier is parsed, not matched
//!
//! A falsifier is `metric comparator threshold`, and the metric must be one the
//! platform computes. That is checked when the hypothesis is *stated* — by
//! `mcp_server_lib::tools::lifecycle::check_hypothesis`, before any trial — so by
//! the time evaluation runs the only remaining questions are what the number was
//! and whether it crossed.
//!
//! # `supported` is the expensive verdict
//!
//! §6.9: before `evaluate` may write `supported` it needs an out-of-sample
//! ticket, a cost-model ticket, a benchmark ticket **and** a reproducibility
//! receipt. Missing any one of them is `inconclusive: evidence_incomplete`, not
//! `supported with caveats` — because a caveat is a sentence somebody may skip
//! and a verdict is a field every reader sees.
//!
//! Note the asymmetry, which is deliberate: **refutation needs none of that.**
//! A falsifier that fired fired. Requiring the full battery before admitting a
//! failure would make failing expensive and passing cheap, which is precisely
//! backwards for a platform whose main output is supposed to be negative results.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a falsifier compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    AtMost,
    AtLeast,
    Below,
    Above,
}

impl Comparator {
    /// Whether the measurement satisfies the falsifier — that is, whether the
    /// hypothesis is **refuted**.
    #[must_use]
    pub fn fired(self, measured: f64, threshold: f64) -> bool {
        match self {
            Comparator::AtMost => measured <= threshold,
            Comparator::AtLeast => measured >= threshold,
            Comparator::Below => measured < threshold,
            Comparator::Above => measured > threshold,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Comparator::AtMost => "<=",
            Comparator::AtLeast => ">=",
            Comparator::Below => "<",
            Comparator::Above => ">",
        }
    }
}

/// A falsifier the platform can check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Falsifier {
    pub metric: String,
    pub comparator: Comparator,
    pub threshold: f64,
}

/// Why a falsifier could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("no metric in `{0}` is one this platform computes")]
    NoMetric(String),
    #[error("`{0}` names a metric but no direction: a metric alone cannot turn out false")]
    NoComparator(String),
    #[error("`{0}` names a direction but no number to compare against")]
    NoThreshold(String),
}

/// Read a falsifier written for one of the platform's metrics.
///
/// Tolerant of surrounding prose — `sharpe_after_costs <= 0 on the out-of-sample
/// window` is the shape a model actually writes — because the alternative is a
/// grammar the model gets wrong three times before it gets it right, and the
/// content is identical either way.
///
/// # Errors
/// [`ParseError`] naming the missing part, which is the part the model has to
/// add.
pub fn parse(falsifier: &str, vocabulary: &[&str]) -> Result<Falsifier, ParseError> {
    let text = falsifier.trim();
    // Longest match first: `sortino_after_costs` contains no other metric, but a
    // vocabulary that later gains `sharpe` would make `sharpe_after_costs` match
    // the shorter one and compare the wrong number.
    let mut candidates: Vec<&&str> = vocabulary.iter().filter(|m| text.contains(**m)).collect();
    candidates.sort_by_key(|m| std::cmp::Reverse(m.len()));
    let Some(metric) = candidates.first().map(|m| (**m).to_string()) else {
        return Err(ParseError::NoMetric(text.to_string()));
    };

    // The comparator has to come *after* the metric: `max_drawdown above 0.3` and
    // `above 0.3 max_drawdown` mean the same thing to a reader and only the
    // first is what anybody writes, so reading from the metric onward avoids
    // matching a comparator that belongs to a different clause.
    let after = text
        .find(&metric)
        .map_or(text, |i| &text[i + metric.len()..]);
    const FORMS: &[(&str, Comparator)] = &[
        ("<=", Comparator::AtMost),
        (">=", Comparator::AtLeast),
        ("=<", Comparator::AtMost),
        ("=>", Comparator::AtLeast),
        (" at most ", Comparator::AtMost),
        (" at least ", Comparator::AtLeast),
        (" below ", Comparator::Below),
        (" above ", Comparator::Above),
        ("<", Comparator::Below),
        (">", Comparator::Above),
    ];
    let hit = FORMS
        .iter()
        .filter_map(|(form, cmp)| after.find(form).map(|at| (at, form.len(), *cmp)))
        .min_by_key(|(at, _, _)| *at);
    let Some((at, len, comparator)) = hit else {
        return Err(ParseError::NoComparator(text.to_string()));
    };

    let tail = &after[at + len..];
    let Some(threshold) = first_number(tail) else {
        return Err(ParseError::NoThreshold(text.to_string()));
    };
    Ok(Falsifier {
        metric,
        comparator,
        threshold,
    })
}

/// The first number in a string, sign and decimal point included.
fn first_number(s: &str) -> Option<f64> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let starts = c.is_ascii_digit()
            || ((c == b'-' || c == b'+' || c == b'.')
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit));
        if starts {
            let mut j = i + 1;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit() || bytes[j] == b'.' || bytes[j] == b'e'
                    || bytes[j] == b'E'
                    || ((bytes[j] == b'-' || bytes[j] == b'+')
                        && matches!(bytes[j - 1], b'e' | b'E')))
            {
                j += 1;
            }
            if let Ok(v) = s[i..j].parse::<f64>() {
                return Some(v);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    None
}

/// What a hypothesis is for (AGENT-013 §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Primary,
    Secondary,
    Exploratory,
}

impl Role {
    /// The highest confidence a finding from this hypothesis may carry.
    ///
    /// AGENT-007 §3.1 step 2, extended: only a primary hypothesis can produce an
    /// established finding. A secondary result is a result the run went looking
    /// for after the fact, and an exploratory one is a result it went looking
    /// *for*; neither was the thing pre-registered, so neither earns the word
    /// that means "this platform stands behind it".
    #[must_use]
    pub fn max_confidence(self) -> crate::memory::Confidence {
        match self {
            Role::Primary => crate::memory::Confidence::Established,
            Role::Secondary | Role::Exploratory => crate::memory::Confidence::Suggestive,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Primary => "primary",
            Role::Secondary => "secondary",
            Role::Exploratory => "exploratory",
        }
    }
}

/// Why the platform could not decide.
///
/// A closed list, and every `inconclusive` carries one. "We could not tell" with
/// no reason is the answer a run reaches for when it has nothing, and it is
/// exactly the one that must cost something to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// Not enough observations to say anything.
    InsufficientData,
    /// The run rewrote its protocol until it stopped being one.
    ProtocolUnstable,
    /// Two measurements of the same quantity disagree beyond their error.
    EvidenceConflict,
    /// The run ran out of trials before the question was answered.
    BudgetExhausted,
    /// §6.9's battery is not complete, so `supported` is not available.
    EvidenceIncomplete,
}

impl Why {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Why::InsufficientData => "insufficient_data",
            Why::ProtocolUnstable => "protocol_unstable",
            Why::EvidenceConflict => "evidence_conflict",
            Why::BudgetExhausted => "budget_exhausted",
            Why::EvidenceIncomplete => "evidence_incomplete",
        }
    }

    pub const ALL: &'static [Why] = &[
        Why::InsufficientData,
        Why::ProtocolUnstable,
        Why::EvidenceConflict,
        Why::BudgetExhausted,
        Why::EvidenceIncomplete,
    ];
}

/// What the platform concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Verdict {
    Supported,
    Refuted,
    Inconclusive { why: Why },
}

impl Verdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Supported => "supported",
            Verdict::Refuted => "refuted",
            Verdict::Inconclusive { .. } => "inconclusive",
        }
    }

    #[must_use]
    pub fn why(self) -> Option<Why> {
        match self {
            Verdict::Inconclusive { why } => Some(why),
            _ => None,
        }
    }
}

/// One measured number and what it cost to get.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub value: f64,
    /// The trial that produced it. A number with no ticket is not evidence: it
    /// is a number somebody typed.
    pub ticket_id: String,
    pub se: Option<f64>,
}

/// What the run has, when evaluation runs.
///
/// Assembled by the caller from the ledger, never from the model's prose — which
/// is the whole reason `evaluate` is a platform function.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evidence {
    /// Measurements by metric name. More than one entry per metric is a
    /// disagreement, which §6.7 handles rather than resolving.
    pub measurements: BTreeMap<String, Vec<Measurement>>,
    /// §6.9's battery: the three tickets and the receipt.
    pub out_of_sample_ticket: Option<String>,
    pub cost_model_ticket: Option<String>,
    pub benchmark_ticket: Option<String>,
    pub repro_spec: Option<String>,
    /// Observations behind the measurement.
    pub n: u32,
    /// The run amended its pre-registration to the cap.
    pub protocol_unstable: bool,
    /// The run ran out of trials.
    pub budget_exhausted: bool,
}

/// Observations below which nothing is concluded either way.
///
/// Three, because two numbers have no spread and one has no comparison. It is a
/// floor on *saying something*, not an estimate of adequacy — the interval rules
/// in `eval::scorecard` do that job, and this stops the degenerate case reaching
/// them at all.
pub const MIN_OBSERVATIONS: u32 = 3;

/// Decide, from the tickets (§6.2, §6.9).
#[must_use]
pub fn evaluate(falsifier: &Falsifier, role: Role, evidence: &Evidence) -> Verdict {
    // A rewritten protocol first: whatever the numbers say, they were not
    // measured against one commitment, so neither verdict means what it says.
    if evidence.protocol_unstable {
        return Verdict::Inconclusive {
            why: Why::ProtocolUnstable,
        };
    }

    let Some(measurements) = evidence.measurements.get(&falsifier.metric) else {
        // Nothing measured the thing the falsifier names. Not a failure of the
        // hypothesis — a failure to test it.
        return Verdict::Inconclusive {
            why: if evidence.budget_exhausted {
                Why::BudgetExhausted
            } else {
                Why::InsufficientData
            },
        };
    };
    if measurements.is_empty() || evidence.n < MIN_OBSERVATIONS {
        return Verdict::Inconclusive {
            why: if evidence.budget_exhausted {
                Why::BudgetExhausted
            } else {
                Why::InsufficientData
            },
        };
    }

    // §6.7: two measurements of one quantity that disagree beyond their stated
    // error are a conflict, and the platform surfaces it rather than picking.
    // Same rule as AGENT-007 P5 and AGENT-005 §24.8, in the third place it
    // applies — and the reason is identical every time: the disagreement is the
    // finding.
    if disagree(measurements) {
        return Verdict::Inconclusive {
            why: Why::EvidenceConflict,
        };
    }

    // Did the falsifier fire? If **any** honest measurement satisfies it, it
    // fired. Requiring all of them would let a run bury a refutation under
    // repeats, which is the shape the trial ledger exists to count.
    if measurements
        .iter()
        .any(|m| falsifier.comparator.fired(m.value, falsifier.threshold))
    {
        return Verdict::Refuted;
    }

    // It did not fire, so `supported` is on the table — and it is the expensive
    // verdict (§6.9).
    let complete = evidence.out_of_sample_ticket.is_some()
        && evidence.cost_model_ticket.is_some()
        && evidence.benchmark_ticket.is_some()
        && evidence.repro_spec.is_some();
    if !complete {
        return Verdict::Inconclusive {
            why: Why::EvidenceIncomplete,
        };
    }
    let _ = role;
    Verdict::Supported
}

/// Whether any two of these disagree beyond their stated error.
///
/// Both must state one. A missing `se` is "cannot tell", never zero — the same
/// rule `harness::context::store::Measurement` applies, because a silent zero
/// would call every pair of unequal numbers a conflict and turn every repeated
/// measurement into an `inconclusive`.
fn disagree(ms: &[Measurement]) -> bool {
    for (i, a) in ms.iter().enumerate() {
        for b in &ms[i + 1..] {
            if let (Some(sa), Some(sb)) = (a.se, b.se) {
                if (a.value - b.value).abs() > sa.abs() + sb.abs() {
                    return true;
                }
            }
        }
    }
    false
}

/// What is missing before `supported` is available (§6.9).
///
/// Named rather than counted: "evidence incomplete" is not something a run can
/// act on, and "no out-of-sample ticket" is.
#[must_use]
pub fn missing_evidence(evidence: &Evidence) -> Vec<&'static str> {
    let mut out = Vec::new();
    if evidence.out_of_sample_ticket.is_none() {
        out.push("an out-of-sample ticket");
    }
    if evidence.cost_model_ticket.is_none() {
        out.push("a cost-model ticket");
    }
    if evidence.benchmark_ticket.is_none() {
        out.push("a benchmark ticket");
    }
    if evidence.repro_spec.is_none() {
        out.push("a reproducibility receipt");
    }
    out
}

/// Whether a finding is trying to write a verdict the platform owns (§6.2).
///
/// Deliberately narrow. "Supported" appears in ordinary prose — *"the result is
/// supported by the funding data"* is a perfectly good finding — so this fires
/// only when the sentence is about **a hypothesis** and reaches for a verdict
/// word. A broad match would correct honest findings, and a model corrected for
/// writing English learns to write worse English rather than to stop claiming
/// verdicts.
#[must_use]
pub fn claims_a_verdict(finding: &str) -> Option<&'static str> {
    let text = finding.to_lowercase();
    if !text.contains("hypothesis") {
        return None;
    }
    const WORDS: &[&str] = &[
        "supported",
        "refuted",
        "confirmed",
        "disproven",
        "proven",
        "validated",
    ];
    WORDS.iter().find(|w| text.contains(**w)).copied()
}

/// The correction a verdict-claiming finding gets.
#[must_use]
pub fn verdict_correction(word: &str) -> String {
    format!(
        "this finding says a hypothesis is `{word}`. The platform writes that, not you: \
         `evaluate` reads the tickets when the funnel returns and sets the status. Record \
         what you measured and what it showed, and let the verdict follow from it."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOCAB: &[&str] = &[
        "sharpe_after_costs",
        "sortino_after_costs",
        "max_drawdown",
        "hit_rate",
    ];

    fn falsifier() -> Falsifier {
        parse("sharpe_after_costs <= 0 on the out-of-sample window", VOCAB).expect("parses")
    }

    fn complete(value: f64) -> Evidence {
        Evidence {
            measurements: BTreeMap::from([(
                "sharpe_after_costs".to_string(),
                vec![Measurement {
                    value,
                    ticket_id: "t1".into(),
                    se: Some(0.05),
                }],
            )]),
            out_of_sample_ticket: Some("t_oos".into()),
            cost_model_ticket: Some("t_cost".into()),
            benchmark_ticket: Some("t_bench".into()),
            repro_spec: Some("receipt_v1".into()),
            n: 512,
            protocol_unstable: false,
            budget_exhausted: false,
        }
    }

    // ── Reading the falsifier ───────────────────────────────────────────────

    #[test]
    fn a_falsifier_is_read_out_of_the_prose_a_model_actually_writes() {
        let f = falsifier();
        assert_eq!(f.metric, "sharpe_after_costs");
        assert_eq!(f.comparator, Comparator::AtMost);
        assert!((f.threshold - 0.0).abs() < f64::EPSILON);

        let g = parse("max_drawdown above 0.35 on the holdout", VOCAB).unwrap();
        assert_eq!(g.comparator, Comparator::Above);
        assert!((g.threshold - 0.35).abs() < f64::EPSILON);
    }

    /// The longest metric wins.
    ///
    /// A vocabulary that later gains `sharpe` would otherwise make
    /// `sharpe_after_costs` match the shorter name and compare a different
    /// number — a failure that changes an answer and breaks nothing.
    #[test]
    fn the_longest_matching_metric_is_the_one_meant() {
        let vocab = &["sharpe", "sharpe_after_costs"];
        let f = parse("sharpe_after_costs < 0.2", vocab).unwrap();
        assert_eq!(f.metric, "sharpe_after_costs");
    }

    #[test]
    fn a_falsifier_with_no_direction_or_no_number_is_refused_by_name() {
        assert!(matches!(
            parse("it does not work", VOCAB),
            Err(ParseError::NoMetric(_))
        ));
        assert!(matches!(
            parse("sharpe_after_costs is bad", VOCAB),
            Err(ParseError::NoComparator(_))
        ));
        assert!(matches!(
            parse("sharpe_after_costs <= zero", VOCAB),
            Err(ParseError::NoThreshold(_))
        ));
    }

    #[test]
    fn a_negative_threshold_reads_as_a_negative_number() {
        let f = parse("sharpe_after_costs <= -0.25", VOCAB).unwrap();
        assert!((f.threshold + 0.25).abs() < f64::EPSILON);
    }

    // ── The verdict ─────────────────────────────────────────────────────────

    /// The falsifier fired, so the hypothesis is refuted. No battery required.
    ///
    /// The asymmetry is deliberate: requiring the full evidence battery before
    /// admitting a failure would make failing expensive and passing cheap, which
    /// is backwards for a platform whose main output is meant to be negative
    /// results.
    #[test]
    fn a_refutation_costs_nothing_extra() {
        let mut e = complete(-0.4);
        e.out_of_sample_ticket = None;
        e.cost_model_ticket = None;
        e.benchmark_ticket = None;
        e.repro_spec = None;
        assert_eq!(evaluate(&falsifier(), Role::Primary, &e), Verdict::Refuted);
    }

    /// `supported` needs all four (§6.9), and removing any one is enough.
    #[test]
    fn supported_needs_the_whole_battery() {
        let f = falsifier();
        assert_eq!(
            evaluate(&f, Role::Primary, &complete(1.4)),
            Verdict::Supported
        );

        for (name, drop) in [
            ("out of sample", 0),
            ("cost model", 1),
            ("benchmark", 2),
            ("receipt", 3),
        ] {
            let mut e = complete(1.4);
            match drop {
                0 => e.out_of_sample_ticket = None,
                1 => e.cost_model_ticket = None,
                2 => e.benchmark_ticket = None,
                _ => e.repro_spec = None,
            }
            assert_eq!(
                evaluate(&f, Role::Primary, &e),
                Verdict::Inconclusive {
                    why: Why::EvidenceIncomplete
                },
                "dropping the {name} ticket must not leave `supported` available"
            );
            assert!(
                !missing_evidence(&e).is_empty(),
                "and what is missing is named, because `evidence_incomplete` is not \
                 something a run can act on"
            );
        }
    }

    /// An exploratory hypothesis cannot yield an established finding.
    ///
    /// The *verdict* may still be `supported` — the falsifier did not fire, and
    /// that is a fact about the number rather than about the role. What the role
    /// caps is the confidence the finding carries, which is where "this platform
    /// stands behind it" is actually said.
    #[test]
    fn an_exploratory_hypothesis_cannot_yield_established() {
        let e = complete(1.4);
        assert_eq!(
            evaluate(&falsifier(), Role::Exploratory, &e),
            Verdict::Supported
        );
        assert_eq!(
            Role::Exploratory.max_confidence(),
            crate::memory::Confidence::Suggestive
        );
        assert_eq!(
            Role::Secondary.max_confidence(),
            crate::memory::Confidence::Suggestive
        );
        assert_eq!(
            Role::Primary.max_confidence(),
            crate::memory::Confidence::Established
        );
    }

    /// Nothing measured the metric the falsifier names.
    #[test]
    fn an_untested_hypothesis_is_inconclusive_rather_than_supported() {
        let mut e = complete(1.4);
        e.measurements.clear();
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e).why(),
            Some(Why::InsufficientData),
            "not testing a hypothesis is not evidence for it"
        );
        e.budget_exhausted = true;
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e).why(),
            Some(Why::BudgetExhausted),
            "and a run that ran out says so rather than saying it had too little data"
        );
    }

    /// Too few observations decides nothing in either direction.
    #[test]
    fn a_handful_of_observations_decides_nothing() {
        let mut e = complete(-0.4);
        e.n = MIN_OBSERVATIONS - 1;
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e).why(),
            Some(Why::InsufficientData),
            "including a refutation: two points cannot refute anything either"
        );
    }

    /// A rewritten protocol invalidates both verdicts, not just the flattering one.
    #[test]
    fn an_unstable_protocol_decides_nothing() {
        let mut e = complete(1.4);
        e.protocol_unstable = true;
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e).why(),
            Some(Why::ProtocolUnstable)
        );
        let mut e = complete(-0.4);
        e.protocol_unstable = true;
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e).why(),
            Some(Why::ProtocolUnstable),
            "the numbers were not measured against one commitment either way"
        );
    }

    /// Two measurements that disagree are surfaced, never resolved.
    #[test]
    fn a_disagreement_is_a_conflict_not_a_verdict() {
        let mut e = complete(1.4);
        e.measurements.get_mut("sharpe_after_costs").unwrap().push(Measurement {
            value: -0.9,
            ticket_id: "t2".into(),
            se: Some(0.05),
        });
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e).why(),
            Some(Why::EvidenceConflict),
            "the disagreement is the finding — the same rule as AGENT-007 P5"
        );
    }

    /// A missing error bar is "cannot tell", never zero.
    #[test]
    fn two_numbers_with_no_stated_error_are_not_a_conflict() {
        let mut e = complete(1.4);
        e.measurements.get_mut("sharpe_after_costs").unwrap()[0].se = None;
        e.measurements.get_mut("sharpe_after_costs").unwrap().push(Measurement {
            value: 1.1,
            ticket_id: "t2".into(),
            se: None,
        });
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e),
            Verdict::Supported,
            "a silent zero would turn every repeated measurement into an inconclusive"
        );
    }

    /// One firing measurement refutes, however many did not.
    #[test]
    fn a_refutation_cannot_be_buried_under_repeats() {
        let mut e = complete(1.4);
        let ms = e.measurements.get_mut("sharpe_after_costs").unwrap();
        // No stated errors, so this is not a *conflict* — it is a run that
        // measured the same thing repeatedly and got one bad answer.
        ms[0].se = None;
        for (i, v) in [1.3, 1.5, -0.2].into_iter().enumerate() {
            ms.push(Measurement {
                value: v,
                ticket_id: format!("t{i}"),
                se: None,
            });
        }
        assert_eq!(
            evaluate(&falsifier(), Role::Primary, &e),
            Verdict::Refuted,
            "requiring every measurement to fire would let a run bury a refutation under \
             repeats, which is the shape the trial ledger exists to count"
        );
    }

    /// Every `inconclusive` carries a reason, and the reason is from the list.
    #[test]
    fn every_inconclusive_says_why() {
        let cases = [
            {
                let mut e = complete(1.4);
                e.protocol_unstable = true;
                e
            },
            {
                let mut e = complete(1.4);
                e.measurements.clear();
                e
            },
            {
                let mut e = complete(1.4);
                e.repro_spec = None;
                e
            },
        ];
        for e in cases {
            let v = evaluate(&falsifier(), Role::Primary, &e);
            let why = v.why().expect("inconclusive carries a reason");
            assert!(Why::ALL.contains(&why));
        }
    }

    // ── The model cannot write one ──────────────────────────────────────────

    #[test]
    fn a_finding_that_grades_its_own_hypothesis_is_corrected() {
        assert_eq!(
            claims_a_verdict("the hypothesis is supported by the out-of-sample run"),
            Some("supported")
        );
        assert_eq!(
            claims_a_verdict("HYPOTHESIS REFUTED: costs eat the edge"),
            Some("refuted")
        );
        let fix = verdict_correction("supported");
        assert!(fix.contains("evaluate"), "{fix}");
        assert!(
            fix.contains("Record what you measured"),
            "the correction has to say what to do instead: {fix}"
        );
    }

    /// The check is narrow on purpose.
    ///
    /// A model corrected for writing English learns to write worse English
    /// rather than to stop claiming verdicts.
    #[test]
    fn an_ordinary_finding_using_the_word_supported_is_not_corrected() {
        assert_eq!(
            claims_a_verdict("the 1h reading is supported by the funding data"),
            None
        );
        assert_eq!(
            claims_a_verdict("BTC-USD momentum is weak after costs"),
            None
        );
    }
}
