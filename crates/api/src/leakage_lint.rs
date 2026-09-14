//! Static leakage lint over agent-written feature and strategy code.
//!
//! The agent has bash and Python in a sandbox, so it can compute anything it likes.
//! What it cannot do is get a result *counted* without the code passing through
//! here. This lint is the cheap first line: a handful of constructs that are almost
//! always future-referencing, caught before a trial is spent rather than after a
//! beautiful backtest has already been written up.
//!
//! **What this is not.** It is a lint, not a proof. `df.shift(-1)` is unambiguous;
//! a leak hidden in a hand-rolled loop is not, and Gate 0's close-stamped scan and
//! the purged CV are what catch that. The value of a lint is that it is free and it
//! runs on every submission, so the obvious mistakes never reach the expensive
//! machinery.
//!
//! **Why the findings carry a `fix`.** A refusal an agent cannot act on becomes a
//! retry loop, and a retry loop against a static lint is an infinite one.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Stable machine code, e.g. `"leak.bfill"`.
    pub code: &'static str,
    /// 1-indexed line the match was on.
    pub line: usize,
    /// The offending source line, trimmed.
    pub excerpt: String,
    /// What to do instead.
    pub fix: &'static str,
}

/// One rule: a substring to look for and what it means.
///
/// Substrings rather than a parser on purpose. A Python parser would catch more and
/// would also have to be kept correct across two languages and every version of
/// pandas; a substring rule that produces a false positive costs the agent one
/// rename, and a rule that misses is backstopped by Gate 0.
struct Rule {
    code: &'static str,
    needles: &'static [&'static str],
    fix: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        code: "leak.bfill",
        needles: &[
            ".bfill(",
            ".backfill(",
            "method='bfill'",
            "method=\"bfill\"",
            "method='backfill'",
            "method=\"backfill\"",
            "limit_direction='backward'",
            "limit_direction=\"backward\"",
        ],
        fix: "back-filling copies a later value into an earlier row, which is the \
              future by definition; forward-fill, or leave the gap and drop the row",
    },
    Rule {
        code: "leak.centred_window",
        needles: &[
            "center=True",
            "center = True",
            "centered=True",
            "centred=True",
        ],
        fix: "a centred window averages bars on both sides of the point; use a \
              trailing window (the default, center=False)",
    },
    Rule {
        code: "leak.negative_shift",
        needles: &[".shift(-", "shift(periods=-", ".diff(-", ".pct_change(-"],
        fix: "a negative shift pulls a future row into the present; that is correct \
              only when building a *label*, which must be declared as the target and \
              purged, never used as a feature",
    },
    Rule {
        code: "leak.future_index",
        needles: &[
            ".iloc[i + 1",
            ".iloc[i+1",
            ".iloc[t + 1",
            ".iloc[t+1",
            "[i + 1]",
            "[i+1]",
        ],
        fix: "indexing one row ahead inside a feature loop reads the future; index \
              backwards from the current row instead",
    },
    Rule {
        code: "leak.full_sample_fit",
        needles: &[
            "fit_transform(df)",
            "fit_transform(X)",
            "fit(df)",
            "StandardScaler().fit(X)",
            "fit(X)\n",
        ],
        fix: "fitting a scaler or encoder on the whole sample leaks test-fold \
              statistics into training; fit inside each fold, on the training rows only",
    },
    Rule {
        code: "leak.interpolate_both_ways",
        needles: &["interpolate(", "resample(", "asfreq("],
        fix: "interpolation and resampling both fill a bar from its neighbours on \
              either side; check the direction and the label/closed convention, and \
              state it in the report",
    },
    Rule {
        code: "cost.disabled",
        needles: &[
            "costs=None",
            "costs = None",
            "fees=0",
            "fees = 0",
            "commission=0",
            "slippage=0",
            "apply_costs=False",
            "include_costs=False",
        ],
        fix: "a backtest without costs measures the market, not a strategy; use the \
              pessimistic cost model, and if a cost must be waived say so as a caveat",
    },
];

/// Rules that flag a *possible* problem rather than a certain one.
///
/// Kept separate because the auditor's target is 0 false rejections on clean twins,
/// and a rule that fires on legitimate code must not be able to reject anything.
/// These are reported as advisories: the agent should explain them, not avoid them.
const ADVISORY_CODES: &[&str] = &["leak.interpolate_both_ways"];

/// Whether a finding is a hard rejection rather than an advisory.
#[must_use]
pub fn is_rejection(code: &str) -> bool {
    !ADVISORY_CODES.contains(&code)
}

/// Lints a source file. Returns every finding, in line order.
///
/// Comment lines are skipped. A rule name written in a comment explaining why the
/// rule exists must not trip the rule — this module's own documentation would
/// otherwise fail its own lint.
#[must_use]
pub fn lint_code(source: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        for rule in RULES {
            if rule.needles.iter().any(|n| line.contains(n)) {
                findings.push(Finding {
                    code: rule.code,
                    line: idx + 1,
                    excerpt: raw.trim().to_string(),
                    fix: rule.fix,
                });
            }
        }
    }
    findings
}

/// Everything after an unquoted `#` or `//` on a line.
///
/// Quote tracking is deliberate: `df.query("a # b")` is not a comment, and a lint
/// that silently stopped reading a line at the first `#` inside a string would miss
/// exactly the code someone was trying to hide.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b'#' || (c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/') {
                    // Python and Rust comment starts, handled together: both mean
                    // "the rest of this line is prose".
                    return &line[..i];
                }
            }
        }
        i += 1;
    }
    line
}

// ── Skill lint ───────────────────────────────────────────────────────────────

/// A tuned constant found in a skill body.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillFinding {
    pub code: &'static str,
    pub line: usize,
    pub excerpt: String,
    pub fix: &'static str,
}

/// Numbers that appear in honest skill code and mean nothing about a market.
///
/// A skill is *procedure*, and a procedure containing `if sharpe > 1.37` has stopped
/// being a procedure and become a result — one that was fitted somewhere, by
/// someone, on data nobody is citing (ADR-0028). But `range(0, 2)` and `* 100` are
/// arithmetic, and rejecting those would make honest skills unwritable.
const INNOCENT_NUMBERS: &[&str] = &[
    "0", "1", "2", "3", "4", "-1", "100", "1000", "60", "24", "365", "252", "0.0", "1.0", "0.5",
    "2.0", "1e-8", "1e-9", "1e-6",
];

/// Lints a skill body for tuned constants (ADR-0028, AGENT-004 §4).
///
/// The rule is deliberately blunt: a decimal literal that is not obviously
/// structural, appearing in a comparison or an assignment, is a threshold someone
/// chose. A skill may still ship with one — it just has to be declared as a
/// parameter with a stated provenance rather than baked into the body.
#[must_use]
pub fn lint_skill(source: &str) -> Vec<SkillFinding> {
    let mut findings = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line = strip_comment(raw);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        for token in numeric_tokens(line) {
            if INNOCENT_NUMBERS.contains(&token.as_str()) {
                continue;
            }
            // A bare integer is usually a window length or an index; a decimal is
            // usually a threshold. Window lengths are tuned too, but flagging every
            // `rolling(20)` would make the lint useless, and a window is at least
            // visible in the strategy's own parameter space.
            if !token.contains('.') {
                continue;
            }
            findings.push(SkillFinding {
                code: "skill.tuned_constant",
                line: idx + 1,
                excerpt: trimmed.to_string(),
                fix: "a skill is a procedure, not a result: move the number to a \
                      declared parameter with a default and say where it came from",
            });
            break;
        }
    }
    findings
}

/// Numeric literals in a line, as written.
fn numeric_tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit() {
            // Not a number if it is part of an identifier (`sma_200`, `x2`).
            let preceded_by_ident =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            // Not a number if an identifier continues straight after it.
            let followed_by_ident =
                i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_');
            if !preceded_by_ident && !followed_by_ident {
                out.push(line[start..i].trim_end_matches('.').to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|x| x.code).collect()
    }

    #[test]
    fn bfill_in_every_spelling_is_caught() {
        for src in [
            "df = df.bfill()",
            "df = df.fillna(method='bfill')",
            "df = df.fillna(method=\"backfill\")",
            "df = df.interpolate(limit_direction='backward')",
        ] {
            let f = lint_code(src);
            assert!(
                f.iter().any(|x| x.code == "leak.bfill"),
                "not caught: {src}"
            );
        }
    }

    #[test]
    fn forward_fill_is_not_flagged() {
        let f = lint_code("df = df.ffill()\ndf = df.fillna(method='ffill')");
        assert!(
            !f.iter().any(|x| x.code == "leak.bfill"),
            "forward-fill is legitimate and must not be rejected"
        );
    }

    #[test]
    fn a_centred_window_is_caught_and_a_trailing_one_is_not() {
        assert!(codes(&lint_code("df.rolling(20, center=True).mean()"))
            .contains(&"leak.centred_window"));
        assert!(lint_code("df.rolling(20).mean()").is_empty());
        assert!(lint_code("df.rolling(20, center=False).mean()").is_empty());
    }

    #[test]
    fn a_negative_shift_is_caught_and_a_positive_one_is_not() {
        assert!(codes(&lint_code("x = df.close.shift(-1)")).contains(&"leak.negative_shift"));
        assert!(lint_code("x = df.close.shift(1)").is_empty());
    }

    /// The lint reads code, and code has comments. A rule name written in a comment
    /// explaining why the rule exists must not trip the rule — otherwise the only
    /// way to document a leak is to commit one.
    #[test]
    fn a_rule_named_in_a_comment_does_not_trip_it() {
        let src = "# never use .bfill() here, it reads the future\ndf = df.ffill()";
        assert!(lint_code(src).is_empty());
        let src = "// center=True would be a leak\nx = roll(df, 20);";
        assert!(lint_code(src).is_empty());
    }

    /// ...but a `#` inside a string is not a comment, and a lint that stopped
    /// reading there would miss code hidden behind one.
    #[test]
    fn a_hash_inside_a_string_does_not_hide_the_rest_of_the_line() {
        let src = "df = df.query(\"a # b\").bfill()";
        assert!(codes(&lint_code(src)).contains(&"leak.bfill"));
    }

    #[test]
    fn disabled_costs_are_caught() {
        assert!(codes(&lint_code("run(cfg, costs=None)")).contains(&"cost.disabled"));
        assert!(codes(&lint_code("run(cfg, slippage=0)")).contains(&"cost.disabled"));
        assert!(lint_code("run(cfg, costs=pessimistic())").is_empty());
    }

    #[test]
    fn advisories_are_not_rejections() {
        assert!(!is_rejection("leak.interpolate_both_ways"));
        assert!(is_rejection("leak.bfill"));
        for r in RULES {
            // Every rule is either a rejection or an advisory; there is no third
            // state, and a rule with no category would silently do nothing.
            assert!(is_rejection(r.code) || ADVISORY_CODES.contains(&r.code));
        }
    }

    #[test]
    fn every_finding_carries_a_fix() {
        let src = "df = df.bfill()\ndf.rolling(5, center=True)\nrun(costs=None)\nx=df.shift(-1)";
        let f = lint_code(src);
        assert_eq!(f.len(), 4);
        for finding in &f {
            assert!(!finding.fix.is_empty(), "{} has no fix", finding.code);
            assert!(finding.line >= 1);
        }
    }

    // ── Skill lint ──────────────────────────────────────────────────────────

    #[test]
    fn a_tuned_threshold_in_a_skill_is_caught() {
        let f = lint_skill("if sharpe > 1.37:\n    return 'promote'");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].code, "skill.tuned_constant");
    }

    #[test]
    fn ordinary_skill_arithmetic_is_not_flagged() {
        let src = "\
for i in range(0, 2):
    pct = value * 100
    ann = daily * 252
    eps = 1e-9
    name = 'sma_200'
";
        assert!(lint_skill(src).is_empty(), "flagged: {:?}", lint_skill(src));
    }

    #[test]
    fn a_window_length_is_not_a_tuned_constant() {
        // Integers are left alone: flagging every rolling(20) would make the lint
        // useless, and a window is at least visible in the parameter space.
        assert!(lint_skill("x = df.rolling(20).mean()").is_empty());
    }

    #[test]
    fn a_number_inside_an_identifier_is_not_a_literal() {
        assert!(lint_skill("sma_20_slope = 1").is_empty());
        assert!(lint_skill("v2_0_threshold = 1").is_empty());
    }

    #[test]
    fn a_declared_parameter_is_the_documented_way_out() {
        // The same threshold, declared rather than baked in. The lint reads the
        // body, so a default in a signature still shows up — which is correct: the
        // registry checks provenance, and the lint's job is to make the number
        // visible in the first place.
        let f = lint_skill("def promote(sharpe, threshold):\n    return sharpe > threshold");
        assert!(f.is_empty());
    }
}
