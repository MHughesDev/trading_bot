//! Claims, extracted from what the model wrote rather than from what it says it
//! cited (AGENT-006 §5.8, AGENT-008 §3.4 rungs 6–7; Set S task S-8.5).
//!
//! # Why not ask the model
//!
//! `api::report_validator` already checks a `final_report` the model submits with
//! its own `evidence` array per claim, and that check is worth having. It rests
//! on the model having attached the right ids.
//!
//! The research this task comes from says that is the half models are worst at:
//! they find relevant documents well and **pinpoint the supporting span badly**.
//! So a self-cited claim is usually citing something *related* and often not the
//! thing that supports the number. Post-hoc alignment asks the other question —
//! given this sentence and this evidence, does the evidence actually contain
//! what the sentence says — and answers it without the model's help.
//!
//! # `direct_stat` is narrow on purpose
//!
//! A claim is [`Method::DirectStat`] only when every number in it appears
//! **exactly** in the evidence. "Sharpe was 1.4" over an evidence value of
//! `1.43` is a true sentence and a *derivation*, and the distinction is the
//! whole point — a reader who sees `direct_stat` should be able to find that
//! number in the evidence by looking.
//!
//! Which makes the middle case load-bearing. Without
//! [`is_rounding_of`] every rounded number would be unsupported, and since
//! reports round almost every number the block would fire on almost every
//! report — and a check that always fires is a check people turn off. So a
//! number the evidence rounds to, at the precision the claim states, is
//! `Derived`: checkable, and not blocking. A number that is neither present nor
//! a rounding of anything present is `Unverified`, and that is the case the
//! layer exists for.
//!
//! The relation is deliberately **asymmetric**. Stating fewer digits than the
//! evidence is a summary; stating more is a fabrication. So `1.4` is a rounding
//! of `1.43` and `1.432` is not a rounding of `1.4`.
//!
//! # What this cannot catch, stated rather than discovered
//!
//! **A claim is a sentence containing a number.** Alignment needs something to
//! align, so a purely qualitative overclaim — "the edge is robust", "this
//! generalises" — is not extracted at all and is not blocked here. That is a real
//! hole and the honest place for it is the critic (S-8.6) and the caveat rules,
//! not a regex that guesses which adjectives are claims.
//!
//! And it checks a number against *whatever bundle it is handed*, which puts a
//! requirement on the caller: *the pre-registration belongs in the bundle*. A
//! report compares measurements to the targets it committed to, and a target is
//! a number that lives in the registration rather than in any result — leave it
//! out and every comparison to a threshold blocks as an invention.
//!
//! It also reads only the *rendered* evidence it is given. A number that exists
//! in a tool's JSON and never reached the text is invisible, which is why the
//! caller passes what the model actually saw.

use serde::{Deserialize, Serialize};

/// How a claim could be checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// A number in the claim appears exactly in the cited evidence.
    DirectStat,
    /// The claim rests on evidence that does not contain its numbers verbatim —
    /// a rounding, a ratio, a difference. Checkable, but only by re-deriving.
    Derived,
    /// Nothing in the evidence supports it.
    ///
    /// **This blocks the report** ([`Extraction::blocks`]). Not a warning: an
    /// unsupported number in a research report is the failure the whole layer
    /// exists to prevent, and a warning is a thing people scroll past.
    Unverified,
}

impl Method {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectStat => "direct_stat",
            Self::Derived => "derived",
            Self::Unverified => "unverified",
        }
    }
}

/// One piece of evidence, as the model actually saw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// The citable id — an experiment, a job, an artifact, a finding.
    pub id: String,
    /// The rendered text. Not the JSON: a number that never reached the text is
    /// a number the model did not read.
    pub text: String,
}

/// One extracted assertion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    /// The sentence, as written.
    pub text: String,
    /// The numbers it asserts, in order.
    pub numbers: Vec<String>,
    /// Evidence ids that account for at least one of those numbers — verbatim or
    /// as a faithful rounding.
    ///
    /// Derived by alignment, **never** taken from the model. Empty only when
    /// nothing in the bundle accounts for any of them.
    pub supported_by: Vec<String>,
    pub method: Method,
}

/// What an extraction found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    pub claims: Vec<Claim>,
}

impl Extraction {
    /// The claims that must be re-derived before the report can be finalised.
    #[must_use]
    pub fn unverified(&self) -> Vec<&Claim> {
        self.claims
            .iter()
            .filter(|c| c.method == Method::Unverified)
            .collect()
    }

    /// Whether this blocks finalisation.
    ///
    /// One unsupported claim is enough. A report is read as a whole and a reader
    /// who finds one invented number stops trusting the rest — which is the
    /// correct response, and the reason this is a block rather than a note.
    #[must_use]
    pub fn blocks(&self) -> bool {
        self.claims.iter().any(|c| c.method == Method::Unverified)
    }

    /// One line per unverified claim, for the refusal the model is handed.
    #[must_use]
    pub fn why_blocked(&self) -> Vec<String> {
        self.unverified()
            .iter()
            .map(|c| {
                format!(
                    "`{}` asserts {}, and no evidence contains it or rounds to it",
                    c.text.trim(),
                    if c.numbers.is_empty() {
                        "a value".to_string()
                    } else {
                        c.numbers.join(", ")
                    }
                )
            })
            .collect()
    }
}

/// Numeric literals in a string, as they were written.
///
/// Returned as **text**, not `f64`, because the comparison this module makes is
/// "does this appear in the evidence" and `1.40` and `1.4` are different
/// appearances of the same value. Parsing to a float would silently make a
/// rounded claim `direct_stat`, which is exactly the distinction being drawn.
#[must_use]
pub fn numbers_in(s: &str) -> Vec<String> {
    let bytes: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // A leading `-` counts only when it starts the token, so "a-b" is not a
        // negative number and "-0.2" is.
        let negative = c == '-'
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_alphanumeric());
        if !(c.is_ascii_digit() || negative) {
            i += 1;
            continue;
        }
        let start = i;
        if negative {
            i += 1;
        }
        let mut seen_dot = false;
        while i < bytes.len() {
            let d = bytes[i];
            if d.is_ascii_digit() {
                i += 1;
            } else if d == '.' && !seen_dot && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()
            {
                seen_dot = true;
                i += 1;
            } else {
                break;
            }
        }
        out.push(bytes[start..i].iter().collect());
    }
    out
}

/// Split prose into sentences.
///
/// Deliberately simple: `.`, `!`, `?` followed by whitespace, plus newlines. A
/// sentence splitter that handled abbreviations would be a second natural
/// language problem inside a module whose job is arithmetic, and over-splitting
/// costs nothing here — a fragment carrying a number is still checked against
/// the same evidence.
#[must_use]
pub fn sentences(prose: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = prose.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let ends =
            matches!(c, '.' | '!' | '?' | '\n') && chars.peek().is_none_or(|n| n.is_whitespace());
        if ends {
            let t = current.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
            current.clear();
        }
    }
    let t = current.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

/// Whether `needle` appears in `haystack` as a whole numeric token.
///
/// Substring matching would make `1.4` match `31.42`, which is how a claim about
/// one number gets "verified" by an unrelated one. The neighbours of a match must
/// not be digits or a decimal point.
#[must_use]
pub fn contains_number(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let h: Vec<char> = haystack.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    if n.len() > h.len() {
        return false;
    }
    for start in 0..=(h.len() - n.len()) {
        if h[start..start + n.len()] != n[..] {
            continue;
        }
        let before_ok = start == 0 || !(h[start - 1].is_ascii_digit() || h[start - 1] == '.');
        let end = start + n.len();
        let after_ok = end == h.len() || !(h[end].is_ascii_digit() || h[end] == '.');
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Whether `claim` is what `evidence` becomes when rounded to the claim's own
/// precision.
///
/// The relation that separates a *derivation* from an *invention*. Without it
/// every rounded number is unsupported, and since reports round almost every
/// number, the block would fire on almost every report — a check that always
/// fires is a check people turn off.
///
/// Rounded to the claim's decimals, not to a fixed tolerance: "1.4" claims one
/// decimal and `1.43` rounds to it, while "1.43" claims two and `1.4` does not
/// round to that. A tolerance would make the check symmetric, and it is not —
/// stating fewer digits is a summary and stating more is a fabrication.
#[must_use]
pub fn is_rounding_of(claim: &str, evidence: f64) -> bool {
    let Ok(stated) = claim.parse::<f64>() else {
        return false;
    };
    let decimals = claim.split_once('.').map_or(0, |(_, frac)| {
        frac.chars().filter(char::is_ascii_digit).count()
    });
    let Ok(decimals) = i32::try_from(decimals) else {
        return false;
    };
    let scale = 10_f64.powi(decimals);
    let rounded = (evidence * scale).round() / scale;
    // Compared at the claim's own precision, so the float error of the scaling
    // cannot decide it. Half a unit in the last place stated.
    (rounded - stated).abs() < 0.5 / scale
}

/// Every numeric literal in a text, parsed.
fn values_in(s: &str) -> Vec<f64> {
    numbers_in(s)
        .iter()
        .filter_map(|n| n.parse::<f64>().ok())
        .collect()
}

/// Extract the claims in `prose` and align each against `evidence`.
///
/// A claim is a sentence containing a number — see the module header for why,
/// and for what that cannot catch.
#[must_use]
pub fn extract(prose: &str, evidence: &[Evidence]) -> Extraction {
    let mut claims = Vec::new();
    for sentence in sentences(prose) {
        let numbers = numbers_in(&sentence);
        if numbers.is_empty() {
            continue;
        }
        // Two relations, in order of strength. Verbatim: the number is in the
        // evidence as written. Rounded: the evidence rounds to it at the
        // precision the claim states.
        let verbatim = |n: &String| evidence.iter().any(|e| contains_number(&e.text, n));
        let rounded = |n: &String| {
            evidence
                .iter()
                .any(|e| values_in(&e.text).iter().any(|v| is_rounding_of(n, *v)))
        };

        let mut supported_by: Vec<String> = Vec::new();
        for e in evidence {
            let vals = values_in(&e.text);
            let supports = numbers
                .iter()
                .any(|n| contains_number(&e.text, n) || vals.iter().any(|v| is_rounding_of(n, *v)));
            if supports {
                supported_by.push(e.id.clone());
            }
        }

        // Every number, or the claim is not direct. A sentence asserting two
        // numbers of which the evidence contains one is **not** direct: the
        // other is the one a reader would want to check, and calling the pair
        // `direct_stat` points them at the half that was already fine.
        let method = if numbers.iter().all(verbatim) {
            Method::DirectStat
        } else if numbers.iter().all(|n| verbatim(n) || rounded(n)) {
            // Every number is either there or is a faithful rounding of
            // something that is. Checkable by re-deriving, which is what
            // `Derived` means.
            Method::Derived
        } else {
            // At least one number is neither in the evidence nor a rounding of
            // anything in it. That is the case this layer exists for.
            Method::Unverified
        };
        claims.push(Claim {
            text: sentence,
            numbers,
            supported_by,
            method,
        });
    }
    Extraction { claims }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, text: &str) -> Evidence {
        Evidence {
            id: id.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_number_that_appears_verbatim_is_direct() {
        let e = [ev(
            "exp_7",
            "sharpe_after_costs 1.43 over 2020-01-01 to 2024-12-31",
        )];
        let x = extract("The strategy reached a sharpe of 1.43.", &e);
        assert_eq!(x.claims.len(), 1);
        assert_eq!(x.claims[0].method, Method::DirectStat);
        assert_eq!(x.claims[0].supported_by, vec!["exp_7"]);
        assert!(!x.blocks());
    }

    /// The distinction the whole module is for. "1.4" is a true sentence about
    /// an evidence value of 1.43, and it is a derivation — a reader told
    /// `direct_stat` should be able to find the number by looking.
    #[test]
    fn a_rounded_number_is_derived_and_not_direct() {
        let e = [ev("exp_7", "sharpe 1.43")];
        let x = extract("Sharpe was about 1.4.", &e);
        assert_eq!(x.claims[0].method, Method::Derived);
        // And it does not block: a faithful rounding is checkable by
        // re-deriving. Before `is_rounding_of` existed this was `Unverified`,
        // which would have blocked almost every report — reports round.
        assert!(!x.blocks());
        assert_eq!(x.claims[0].supported_by, vec!["exp_7"]);
    }

    /// The relation is asymmetric: fewer digits is a summary, more is a
    /// fabrication.
    #[test]
    fn stating_more_digits_than_the_evidence_is_not_a_rounding() {
        assert!(is_rounding_of("1.4", 1.43));
        assert!(is_rounding_of("1", 1.43));
        assert!(is_rounding_of("1.43", 1.43));
        // 1.4 does not round to 1.432 at two decimals.
        assert!(!is_rounding_of("1.432", 1.4));
        // Nor does an unrelated number.
        assert!(!is_rounding_of("2.9", 1.43));
        // Rounding is half-up at the stated precision, so 1.45 → 1.5 not 1.4.
        assert!(is_rounding_of("1.5", 1.45));
        assert!(!is_rounding_of("1.4", 1.45));
        // Negative values behave the same way.
        assert!(is_rounding_of("-0.2", -0.23));
        assert!(!is_rounding_of("-0.2", 0.23));
    }

    /// A number nothing supports blocks the report, and the refusal names it.
    #[test]
    fn an_unsupported_number_blocks_finalisation() {
        let e = [ev("exp_7", "sharpe 1.43")];
        let x = extract("The strategy reached a sharpe of 2.9.", &e);
        assert_eq!(x.claims[0].method, Method::Unverified);
        assert!(x.blocks());
        let why = x.why_blocked();
        assert_eq!(why.len(), 1);
        assert!(why[0].contains("2.9"), "{}", why[0]);
    }

    /// Substring matching is how a claim about one number gets verified by
    /// another. `1.4` must not be found inside `31.42`.
    #[test]
    fn a_number_is_not_supported_by_a_longer_one_containing_it() {
        assert!(!contains_number("turnover 31.42", "1.4"));
        assert!(!contains_number("sharpe 1.432", "1.43"));
        assert!(contains_number("sharpe 1.43 net", "1.43"));
        assert!(contains_number("(1.43)", "1.43"));
        let e = [ev("exp_7", "turnover 31.42")];
        assert!(extract("Sharpe was 1.4.", &e).blocks());
    }

    /// A sentence asserting two numbers is direct only if the evidence has both.
    /// Otherwise it points a reader at the half that was already fine.
    #[test]
    fn a_pair_is_direct_only_when_both_appear() {
        let both = [ev("exp_7", "sharpe 1.43 max_drawdown 0.12")];
        assert_eq!(
            extract("Sharpe 1.43 at a drawdown of 0.12.", &both).claims[0].method,
            Method::DirectStat
        );

        // One verbatim, one a faithful rounding: a derivation, not a block.
        let rounded = [ev("exp_7", "sharpe 1.43 max_drawdown 0.123")];
        let x = extract("Sharpe 1.43 at a drawdown of 0.12.", &rounded);
        assert_eq!(x.claims[0].method, Method::Derived);
        assert!(!x.blocks());

        // One verbatim and one the evidence cannot account for **blocks**, even
        // though half the sentence checks out. The unaccounted number is the one
        // a reader would want, and a `Derived` here would tell them the sentence
        // was fine.
        let half = [ev("exp_7", "sharpe 1.43")];
        let x = extract("Sharpe 1.43 at a drawdown of 0.12.", &half);
        assert_eq!(x.claims[0].method, Method::Unverified);
        assert!(x.blocks());
    }

    /// Prose without numbers is not a claim, which is this module's stated hole
    /// rather than an oversight.
    #[test]
    fn a_sentence_with_no_number_is_not_extracted() {
        let e = [ev("exp_7", "sharpe 1.43")];
        let x = extract("I looked at BTC-USD and the edge seems robust.", &e);
        assert!(x.claims.is_empty());
        assert!(!x.blocks(), "a qualitative overclaim is not caught here");
    }

    #[test]
    fn numbers_are_read_as_written() {
        assert_eq!(numbers_in("sharpe 1.43 and -0.2"), vec!["1.43", "-0.2"]);
        // A hyphen between words is not a sign.
        assert_eq!(numbers_in("2020-01-01"), vec!["2020", "01", "01"]);
        // A trailing dot is a full stop, not a decimal point.
        assert_eq!(numbers_in("it was 3."), vec!["3"]);
        assert!(numbers_in("no digits here").is_empty());
    }

    #[test]
    fn sentences_split_on_terminators_and_newlines() {
        let s = sentences("One 1. Two 2!\nThree 3");
        assert_eq!(s.len(), 3);
        assert!(s[2].contains('3'));
    }

    /// Several claims, mixed methods, and the block comes from the one that
    /// deserves it.
    ///
    /// The registration is in the bundle, which is not decoration: a report
    /// compares measurements to the targets it committed to, and a target is a
    /// number that exists in the *registration* rather than in any result. Leave
    /// it out and every comparison to a threshold blocks — see the module
    /// header.
    #[test]
    fn a_report_blocks_on_its_worst_claim() {
        let e = [
            ev("exp_7", "sharpe 1.43 max_drawdown 0.123"),
            ev("prereg_2", "delta_practical 0.10 objective maximize sharpe"),
        ];
        let x = extract(
            "Sharpe was 1.43. Drawdown 0.12 against a target of 0.10. Turnover hit 44.",
            &e,
        );
        assert_eq!(x.claims.len(), 3);
        assert_eq!(x.claims[0].method, Method::DirectStat);
        // 0.12 rounds from 0.123 and 0.10 is in the registration verbatim.
        assert_eq!(x.claims[1].method, Method::Derived);
        // 44 is in nothing and rounds from nothing.
        assert_eq!(x.claims[2].method, Method::Unverified);
        assert!(x.blocks());
        assert_eq!(x.unverified().len(), 1);
        assert!(x.why_blocked()[0].contains("44"));
    }

    /// Alignment never reads the model's own citation, because there is nowhere
    /// to put one. The support list is an output.
    #[test]
    fn support_is_derived_and_cannot_be_supplied() {
        let e = [ev("exp_1", "nothing numeric"), ev("exp_2", "sharpe 1.43")];
        let x = extract("Sharpe 1.43.", &e);
        // The evidence that actually contains it, not the one listed first.
        assert_eq!(x.claims[0].supported_by, vec!["exp_2"]);
    }
}
