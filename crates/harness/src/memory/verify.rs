//! `record_finding`, verified (AGENT-007 §3.1, P1; Set S task S-4.2).
//!
//! # What was wrong
//!
//! `drive.rs` accepted any two strings. The id check landed with Set S phase 0 —
//! `evidence` naming an id no tool returned is already refused — but the second
//! half of §3.1 did not: **a claim may still assert a number that appears in no
//! result it cites.**
//!
//! That is the failure that matters. A citation to a real experiment is exactly
//! what makes a wrong number credible, and a finding stored `established` becomes
//! a premise for every later run (§2.2.1). So the rule here is narrow and
//! absolute: a number the cited results do not carry does not make the finding a
//! lie, and does not refuse it — it makes it [`Confidence::Suggestive`], with
//! [`EvidenceClass::Derived`], **never `established`**.
//!
//! # Why a downgrade rather than a refusal
//!
//! Because the arithmetic is often right. "The two Sharpes differ by 0.31" is a
//! true statement derived from two results that each carry one Sharpe and neither
//! carries the difference. Refusing it would teach the model to stop doing
//! arithmetic, which is worse than recording that the arithmetic was the model's.
//! `derived` is exactly what AGENT-007 §2.2 means by an evidence class.

use std::collections::{BTreeMap, BTreeSet};

use super::{Confidence, EvidenceClass};

/// What verification decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingVerdict {
    /// Not storable. `correction` is model-facing text: it names what is wrong and
    /// what to do instead, because a correction the model cannot act on is how a
    /// healthy session gets fenced (LOCAL_TIER_FINDINGS §8, "one that is not a
    /// bug").
    Refused {
        code: &'static str,
        correction: String,
    },
    /// Storable, at the confidence the evidence actually supports.
    Accepted {
        confidence: Confidence,
        evidence_class: EvidenceClass,
        /// The ids parsed out of `evidence`, in the order they appeared.
        cited: Vec<String>,
        /// Numbers in the claim that no cited result carries. Empty on an
        /// `established` finding, by construction.
        unbacked: Vec<String>,
        /// One line, for the timeline and for the model.
        why: String,
    },
}

impl FindingVerdict {
    #[must_use]
    pub fn is_established(&self) -> bool {
        matches!(
            self,
            FindingVerdict::Accepted {
                confidence: Confidence::Established,
                ..
            }
        )
    }
}

/// Tokens that look like machine-generated handles rather than words.
///
/// The same rule `drive::opaque_ids` uses, duplicated here rather than shared
/// because this module must be callable without a `Loop`: length ≥ 8, identifier
/// characters, at least one digit. It never challenges an English word.
fn opaque_ids(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .filter(|t| t.len() >= 8 && t.chars().any(|c| c.is_ascii_digit()))
        .collect()
}

/// A numeric literal lifted out of prose, with the text it was written as.
///
/// The text is kept because precision is the whole comparison: `0.42` found in a
/// result that says `0.4231` is a match, and `0.43` is not. Comparing parsed
/// floats alone loses the number of decimals the claim committed to.
#[derive(Debug, Clone, PartialEq)]
struct Num {
    text: String,
    value: f64,
    decimals: u32,
}

/// Pull numeric literals out of text, after removing identifiers.
///
/// Identifiers are stripped first, or `exp_20260401` donates `20260401` to the
/// claim and the finding is downgraded for citing its own evidence id. That is
/// not a hypothetical: every id this platform mints has digits in it.
fn numbers(text: &str) -> Vec<Num> {
    let mut cleaned = text.to_string();
    for id in opaque_ids(text) {
        cleaned = cleaned.replace(id, " ");
    }
    let bytes: Vec<char> = cleaned.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // Walk back over a leading minus, but only when it is not a hyphen
        // joining two words (`BTC-USD`) or a range (`2026-04`).
        let start = if i > 0
            && bytes[i - 1] == '-'
            && (i == 1 || !bytes[i - 2].is_ascii_alphanumeric())
        {
            i - 1
        } else {
            i
        };
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        let mut decimals = 0u32;
        if j + 1 < bytes.len() && bytes[j] == '.' && bytes[j + 1].is_ascii_digit() {
            j += 1;
            let frac = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            decimals = u32::try_from(j - frac).unwrap_or(0);
        }
        // A digit glued to a letter is a token, not a measurement: `1h`, `1d`,
        // `5m`, `v3`, `BTC2`. Reading the `1` out of "BTC-USD 1h" as a claimed
        // number downgraded every finding that named a timeframe — which is
        // nearly all of them — and the downgrade would have been permanent and
        // silent.
        let glued_after = j < bytes.len() && bytes[j].is_alphabetic();
        let glued_before = start > 0 && bytes[start - 1].is_alphabetic();
        if !glued_after && !glued_before {
            let text: String = bytes[start..j].iter().collect();
            if let Ok(value) = text.parse::<f64>() {
                out.push(Num {
                    text,
                    value,
                    decimals,
                });
            }
        }
        i = j;
    }
    out
}

/// Whether `claimed` is `found` to the precision `claimed` was written at.
///
/// "within rounding" in §3.1 step 2, made a rule: a claim of `0.42` is backed by
/// a result of `0.4231`, and a claim of `0.4231` is *not* backed by a result of
/// `0.42` — asserting more precision than the evidence carries is exactly the
/// thing being caught.
fn backs(claimed: &Num, found: &Num) -> bool {
    if claimed.decimals > found.decimals && found.decimals > 0 {
        // The claim is more precise than the result. Only an exact value passes.
        return (claimed.value - found.value).abs() < f64::EPSILON;
    }
    let scale = 10f64.powi(i32::try_from(claimed.decimals).unwrap_or(0));
    let rounded = (found.value * scale).round() / scale;
    (rounded - claimed.value).abs() <= 1e-9 * scale.max(1.0)
}

/// AGENT-007 §3.1, all four steps.
///
/// * `seen_ids` — every opaque id a tool returned this run.
/// * `results_by_id` — the text of the result each id came in. A claim's number
///   must appear in the result of an id the claim cites; a number found in some
///   *other* result is not evidence for this claim.
#[must_use]
pub fn verify_finding(
    claim: &str,
    evidence: &str,
    seen_ids: &BTreeSet<String>,
    results_by_id: &BTreeMap<String, String>,
) -> FindingVerdict {
    if claim.trim().is_empty() {
        return FindingVerdict::Refused {
            code: "memory.finding_empty",
            correction: "record_finding needs a non-empty `finding`: say what you now know."
                .to_string(),
        };
    }
    if evidence.trim().is_empty() {
        return FindingVerdict::Refused {
            code: "memory.finding_evidence_missing",
            correction: "`evidence` is required: name what you actually read this session — a \
                         tool you called, or an id it returned."
                .to_string(),
        };
    }

    // Step 1 — every cited id must have been returned by a tool in this run.
    let cited: Vec<String> = opaque_ids(evidence)
        .into_iter()
        .map(str::to_string)
        .collect();
    let invented: Vec<&String> = cited.iter().filter(|id| !seen_ids.contains(*id)).collect();
    if !invented.is_empty() {
        let names: Vec<&str> = invented.iter().map(|s| s.as_str()).collect();
        return FindingVerdict::Refused {
            code: "memory.finding_evidence_unattributable",
            correction: format!(
                "`evidence` cites {} — no tool in this session returned that. Cite something \
                 you actually read, or do the work first.",
                names.join(", ")
            ),
        };
    }

    // Step 2 — every number in the claim must be in a result the claim cites.
    //
    // A claim with no cited ids is not refused here: a lookup goal produces no
    // ids at all (LOCAL_TIER_FINDINGS §8), and refusing it would repeat the
    // corrective-message bug that fenced a healthy session. It can only ever be
    // `suggestive`, which is the honest answer.
    let claimed = numbers(claim);
    let mut unbacked: Vec<String> = Vec::new();
    for c in &claimed {
        let backed = cited.iter().any(|id| {
            results_by_id
                .get(id)
                .map(|r| numbers(r).iter().any(|f| backs(c, f)))
                .unwrap_or(false)
        });
        if !backed {
            unbacked.push(c.text.clone());
        }
    }

    if cited.is_empty() {
        return FindingVerdict::Accepted {
            confidence: Confidence::Suggestive,
            evidence_class: EvidenceClass::ModelSummary,
            cited,
            unbacked,
            why: "no ids cited; this run produced none, so the claim is the model's summary"
                .to_string(),
        };
    }
    if unbacked.is_empty() {
        FindingVerdict::Accepted {
            confidence: Confidence::Established,
            evidence_class: EvidenceClass::DirectStat,
            why: format!("every number is in a cited result ({})", cited.join(", ")),
            cited,
            unbacked,
        }
    } else {
        FindingVerdict::Accepted {
            confidence: Confidence::Suggestive,
            evidence_class: EvidenceClass::Derived,
            why: format!(
                "{} is not in any cited result; stored as derived, not established",
                unbacked.join(", ")
            ),
            cited,
            unbacked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_string()).collect()
    }

    fn results(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// AGENT-007 §8.1, first half. The adversarial case: a plausible id that no
    /// tool produced.
    #[test]
    fn a_finding_citing_an_id_no_tool_returned_is_refused() {
        let v = verify_finding(
            "BTC-USD 1h has a momentum edge",
            "from experiment exp_9f3c1201",
            &seen(&["exp_11112222"]),
            &results(&[("exp_11112222", "sharpe 0.41")]),
        );
        match v {
            FindingVerdict::Refused { code, correction } => {
                assert_eq!(code, "memory.finding_evidence_unattributable");
                assert!(correction.contains("exp_9f3c1201"), "{correction}");
            }
            other => panic!("a fabricated id must be refused, got {other:?}"),
        }
    }

    /// AGENT-007 §8.1, second half. The number is the thing being checked, and
    /// the citation is real — which is what makes this the dangerous case.
    #[test]
    fn a_number_absent_from_the_cited_result_is_stored_suggestive() {
        let v = verify_finding(
            "BTC-USD 1h momentum has Sharpe 1.90",
            "exp_11112222",
            &seen(&["exp_11112222"]),
            &results(&[("exp_11112222", "sharpe 0.41, n 812")]),
        );
        match &v {
            FindingVerdict::Accepted {
                confidence,
                evidence_class,
                unbacked,
                ..
            } => {
                assert_eq!(*confidence, Confidence::Suggestive);
                assert_eq!(*evidence_class, EvidenceClass::Derived);
                assert_eq!(*unbacked, vec!["1.90".to_string()]);
            }
            other => panic!("expected a downgrade, not {other:?}"),
        }
        assert!(!v.is_established());
    }

    #[test]
    fn a_number_present_in_the_cited_result_is_established() {
        let v = verify_finding(
            "BTC-USD 1h momentum has Sharpe 0.41 over 812 bars",
            "exp_11112222",
            &seen(&["exp_11112222"]),
            &results(&[("exp_11112222", "{\"sharpe\": 0.41, \"n\": 812}")]),
        );
        assert!(v.is_established(), "{v:?}");
    }

    /// The claim's precision is what it is held to. `0.42` is backed by
    /// `0.4231`; the reverse is a claim to precision the evidence does not have.
    #[test]
    fn rounding_is_allowed_downwards_and_not_upwards() {
        let ok = verify_finding(
            "residual t is 0.42",
            "exp_11112222",
            &seen(&["exp_11112222"]),
            &results(&[("exp_11112222", "residual_t = 0.4231")]),
        );
        assert!(ok.is_established(), "{ok:?}");

        let not_ok = verify_finding(
            "residual t is 0.4231",
            "exp_11112222",
            &seen(&["exp_11112222"]),
            &results(&[("exp_11112222", "residual_t = 0.42")]),
        );
        assert!(!not_ok.is_established(), "{not_ok:?}");
    }

    /// A number found in some *other* result is not evidence for this claim.
    /// Without this the check is "does this number exist anywhere in the run",
    /// which every number does.
    #[test]
    fn a_number_from_an_uncited_result_does_not_back_the_claim() {
        let v = verify_finding(
            "the edge is 2.75",
            "exp_11112222",
            &seen(&["exp_11112222", "exp_33334444"]),
            &results(&[
                ("exp_11112222", "sharpe 0.41"),
                ("exp_33334444", "sharpe 2.75"),
            ]),
        );
        assert!(!v.is_established(), "{v:?}");
    }

    /// The id itself must not donate its digits to the claim. Every id this
    /// platform mints has digits in it.
    #[test]
    fn an_id_in_the_claim_is_not_read_as_a_number() {
        let v = verify_finding(
            "exp_20260401 shows no edge",
            "exp_20260401",
            &seen(&["exp_20260401"]),
            &results(&[("exp_20260401", "no edge detected")]),
        );
        assert!(
            v.is_established(),
            "the id's digits must not be read as an unbacked claim: {v:?}"
        );
    }

    /// LOCAL_TIER_FINDINGS §8's "one that is not a bug", as a rule: a lookup goal
    /// produces no ids, and demanding one fences a healthy session. It is stored,
    /// and stored honestly.
    #[test]
    fn a_claim_with_no_ids_is_accepted_but_can_never_be_established() {
        let v = verify_finding(
            "the platform has 1m bars for BTC-USD",
            "list_instruments",
            &seen(&[]),
            &results(&[]),
        );
        match v {
            FindingVerdict::Accepted {
                confidence,
                evidence_class,
                ..
            } => {
                assert_eq!(confidence, Confidence::Suggestive);
                assert_eq!(evidence_class, EvidenceClass::ModelSummary);
            }
            other => panic!("a lookup finding must not be refused: {other:?}"),
        }
    }

    #[test]
    fn empty_claim_and_empty_evidence_are_both_refused() {
        assert!(matches!(
            verify_finding("  ", "exp_11112222", &seen(&[]), &results(&[])),
            FindingVerdict::Refused {
                code: "memory.finding_empty",
                ..
            }
        ));
        assert!(matches!(
            verify_finding("something", "", &seen(&[]), &results(&[])),
            FindingVerdict::Refused {
                code: "memory.finding_evidence_missing",
                ..
            }
        ));
    }

    /// The regression that this rule exists for: "BTC-USD 1h" donated a `1`
    /// to every claim that named a timeframe, and the downgrade was silent.
    #[test]
    fn a_timeframe_suffix_is_not_a_claimed_number() {
        let texts: Vec<String> = numbers("BTC-USD 1h momentum over 5m bars, v3")
            .into_iter()
            .map(|n| n.text)
            .collect();
        assert!(texts.is_empty(), "{texts:?}");
        let v = verify_finding(
            "BTC-USD 1h momentum has Sharpe 0.41",
            "exp_11112222",
            &seen(&["exp_11112222"]),
            &results(&[("exp_11112222", "sharpe 0.41")]),
        );
        assert!(v.is_established(), "{v:?}");
    }

    #[test]
    fn negative_numbers_parse_but_hyphenated_names_do_not() {
        let ns = numbers("BTC-USD drifted -0.15 over 2026-04");
        let texts: Vec<&str> = ns.iter().map(|n| n.text.as_str()).collect();
        assert!(texts.contains(&"-0.15"), "{texts:?}");
        assert!(
            !texts.contains(&"-04"),
            "a date component must not read as a negative number: {texts:?}"
        );
    }
}
