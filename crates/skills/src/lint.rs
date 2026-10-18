//! The static scan a skill bundle must pass before it is verified (Set R-a, RA-3.2;
//! AGENT-003 §6.3 step 1; SK-06, SK-08, SK-12).
//!
//! A skill is reusable procedure. What does not belong in one is anything that is
//! really a *fact about the world on a particular day*: an instrument, a date, a
//! tuned number. Those make a skill that works once and misleads forever, and they
//! are the cheapest thing to catch before a model spends a verification run on it.
//!
//! # Rules
//!
//! | rule | catches |
//! |---|---|
//! | `ticker` | instrument pairs (`BTC-USD`, `ETH/USDT`, `SOLUSDT`) and bare crypto tickers |
//! | `date` | an ISO or slash date |
//! | `tuned_constant` | a literal with three or more significant fractional digits |
//! | `capability` | frontmatter `allowed-tools` / `hooks`, a `!` shell line, a network or process import in a script, a credential assignment, a path that escapes the workspace |
//! | `description_tokens` | a description over 100 tokens |
//! | `body_lines` | a body over 500 lines |
//!
//! The scan is deliberately *lexical*. It will flag `0.0137` in a worked example
//! the author meant as an illustration, and it cannot recognise an equity ticker it
//! was not told about. The first is why a finding may be suppressed; the second is
//! why this is a floor and the Reviewer sub-agent (RA-3.3) still reads the bundle.
//!
//! # Suppression is a recorded decision, not an escape hatch
//!
//! A line may carry `lint: allow(<rule>) -- <reason>`. The finding moves to
//! [`LintReport::suppressed`] **with the reason**, which the Reviewer reads, and a
//! suppression with an empty reason does not suppress (ADR-RA-06). The `capability`
//! rule cannot be suppressed at all: SK-12 says a skill grants no capability, and a
//! reason is not a capability.

use std::collections::BTreeMap;

use crate::core::{
    body_lines_over, description_tokens_over, MAX_BODY_LINES, MAX_DESCRIPTION_TOKENS,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    /// 1-based; 0 for a finding about the whole file.
    pub line: usize,
    pub rule: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suppressed {
    pub finding: Finding,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintReport {
    pub findings: Vec<Finding>,
    pub suppressed: Vec<Suppressed>,
}

/// The rules about what a bundle *can do or how big it is*. The other three are
/// about what it *says*: a skill whose job is to teach them (`skill-authoring`)
/// has to name the forbidden thing to forbid it, so release-signed core content is
/// held to these and not to the content rules (ADR-RA-07).
pub const HARD_RULES: [&str; 3] = ["capability", "description_tokens", "body_lines"];

impl LintReport {
    #[must_use]
    pub fn passes(&self) -> bool {
        self.findings.is_empty()
    }

    /// Findings under [`HARD_RULES`] only.
    #[must_use]
    pub fn hard_findings(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| HARD_RULES.contains(&f.rule))
            .collect()
    }
}

const QUOTES: [&str; 5] = ["USD", "USDT", "USDC", "EUR", "GBP"];
const BARE_TICKERS: [&str; 9] = [
    "BTC", "ETH", "SOL", "XRP", "DOGE", "ADA", "BNB", "AVAX", "LINK",
];
const SCRIPT_IMPORTS: [&str; 14] = [
    "requests",
    "urllib",
    "socket",
    "http.client",
    "httpx",
    "aiohttp",
    "subprocess",
    "ftplib",
    "smtplib",
    "paramiko",
    "os.system",
    "shutil.rmtree",
    "ctypes",
    "multiprocessing",
];
const CREDENTIAL_WORDS: [&str; 5] = ["api_key", "apikey", "secret", "password", "bearer "];

fn is_pair(tok: &str) -> bool {
    // `AAA-USD`, `AAAA/USDT`, `AAAAUSDT`.
    let upper = |s: &str, lo: usize, hi: usize| {
        (lo..=hi).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_uppercase())
    };
    if let Some((a, b)) = tok.split_once(['-', '/']) {
        return upper(a, 2, 6) && QUOTES.contains(&b);
    }
    for q in QUOTES {
        if let Some(base) = tok.strip_suffix(q) {
            if upper(base, 2, 6) && q != "USD" {
                return true;
            }
        }
    }
    false
}

fn tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| {
        !(c.is_ascii_alphanumeric() || c == '-' || c == '/' || c == '_' || c == '.')
    })
    .filter(|t| !t.is_empty())
}

fn has_date(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let mut i = 0;
    while i + 10 <= b.len() {
        let w = &b[i..i + 10];
        let sep = w[4];
        let ok = (sep == b'-' || sep == b'/')
            && w[7] == sep
            && w[..4].iter().all(u8::is_ascii_digit)
            && w[5..7].iter().all(u8::is_ascii_digit)
            && w[8..10].iter().all(u8::is_ascii_digit)
            && (w[..2] == *b"19" || w[..2] == *b"20");
        if ok {
            let month = (w[5] - b'0') * 10 + (w[6] - b'0');
            let day = (w[8] - b'0') * 10 + (w[9] - b'0');
            let left_ok = i == 0 || !b[i - 1].is_ascii_alphanumeric();
            let right_ok = i + 10 == b.len() || !b[i + 10].is_ascii_digit();
            if (1..=12).contains(&month) && (1..=31).contains(&day) && left_ok && right_ok {
                return Some(String::from_utf8_lossy(w).into_owned());
            }
        }
        i += 1;
    }
    None
}

/// A literal with three or more significant fractional digits.
fn tuned_constant(line: &str) -> Option<String> {
    for tok in line.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        // A sentence's full stop is not a decimal point.
        let tok = tok.trim_end_matches('.');
        if tok.matches('.').count() != 1 {
            continue; // not a decimal, or a version string like 1.2.3
        }
        let (int, frac) = tok.split_once('.')?;
        if int.is_empty() || frac.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if frac.trim_end_matches('0').len() >= 3 {
            return Some(tok.to_string());
        }
    }
    None
}

/// Parse `lint: allow(rule) -- reason` from a line.
fn suppression(line: &str) -> Option<(String, String)> {
    let rest = &line[line.find("lint: allow(")? + "lint: allow(".len()..];
    let (rule, after) = rest.split_once(')')?;
    let reason = after
        .trim_start()
        .strip_prefix("--")
        .unwrap_or("")
        .trim()
        .to_string();
    Some((rule.trim().to_string(), reason))
}

fn split_frontmatter(text: &str) -> (Vec<(usize, &str)>, usize) {
    // Returns the frontmatter's (line_no, line) pairs and the 0-based index of the first body line.
    let mut lines = text.lines().enumerate();
    if text.lines().next() != Some("---") {
        return (vec![], 0);
    }
    lines.next();
    let mut front = Vec::new();
    for (i, l) in lines {
        if l == "---" {
            return (front, i + 1);
        }
        front.push((i + 1, l));
    }
    (front, text.lines().count())
}

/// Scan a bundle's files (`SKILL.md`, `scripts/*`, `reference/*`).
#[must_use]
pub fn lint_bundle(files: &BTreeMap<String, Vec<u8>>) -> LintReport {
    let mut report = LintReport::default();
    let push = |report: &mut LintReport, f: Finding, line_text: &str| {
        if f.rule != "capability" {
            if let Some((rule, reason)) = suppression(line_text) {
                if rule == f.rule && !reason.is_empty() {
                    report.suppressed.push(Suppressed { finding: f, reason });
                    return;
                }
            }
        }
        report.findings.push(f);
    };

    for (path, bytes) in files {
        let Ok(text) = std::str::from_utf8(bytes) else {
            push(
                &mut report,
                Finding {
                    file: path.clone(),
                    line: 0,
                    rule: "capability",
                    text: "not UTF-8: a skill is text".into(),
                },
                "",
            );
            continue;
        };
        let is_script = path.starts_with("scripts/");
        let is_skill_md = path == "SKILL.md";
        let (front, body_start) = if is_skill_md {
            split_frontmatter(text)
        } else {
            (vec![], 0)
        };

        if is_skill_md {
            for (no, l) in &front {
                let key = l.split(':').next().unwrap_or("").trim();
                if matches!(key, "allowed-tools" | "allowed_tools" | "hooks") {
                    report.findings.push(Finding {
                        file: path.clone(),
                        line: *no,
                        rule: "capability",
                        text: format!(
                            "frontmatter `{key}` grants a capability; a skill grants none"
                        ),
                    });
                }
                if key == "description" {
                    let d = l.split_once(':').map_or("", |x| x.1).trim();
                    if let Some(n) = description_tokens_over(d) {
                        report.findings.push(Finding {
                            file: path.clone(),
                            line: *no,
                            rule: "description_tokens",
                            text: format!(
                                "about {n} tokens; the limit is {MAX_DESCRIPTION_TOKENS}"
                            ),
                        });
                    }
                }
            }
            let body_lines = text.lines().count().saturating_sub(body_start);
            if body_lines_over(body_lines).is_some() {
                report.findings.push(Finding {
                    file: path.clone(),
                    line: 0,
                    rule: "body_lines",
                    text: format!("{body_lines} lines; the limit is {MAX_BODY_LINES}"),
                });
            }
        }

        for (idx, line) in text.lines().enumerate() {
            let no = idx + 1;
            let in_front = is_skill_md && no <= body_start;
            let find = |rule: &'static str, text: String| Finding {
                file: path.clone(),
                line: no,
                rule,
                text,
            };
            if is_skill_md && !in_front && line.trim_start().starts_with("!`") {
                report.findings.push(find(
                    "capability",
                    "a `!` shell-substitution line runs code".into(),
                ));
            }
            if is_script {
                for imp in SCRIPT_IMPORTS {
                    if line.contains(imp)
                        && (line.contains("import") || line.contains(&format!("{imp}(")))
                    {
                        report.findings.push(find(
                            "capability",
                            format!("`{imp}` reaches the network, a process or the filesystem; a script is a pure transform"),
                        ));
                    }
                }
                let lower = line.to_ascii_lowercase();
                for w in CREDENTIAL_WORDS {
                    if lower.contains(w) && (lower.contains('=') || lower.contains(':')) {
                        report.findings.push(find(
                            "capability",
                            format!("`{}` looks like a credential", w.trim()),
                        ));
                        break;
                    }
                }
                if (line.contains("open(") || line.contains("Path("))
                    && (line.contains("..")
                        || line.contains("\"/")
                        || line.contains("'/")
                        || line.contains(":\\"))
                {
                    report.findings.push(find(
                        "capability",
                        "a path that leaves the workspace".into(),
                    ));
                }
            }
            for tok in tokens(line) {
                let t = tok.trim_matches(|c| c == '.' || c == '-' || c == '/');
                if is_pair(t) {
                    push(
                        &mut report,
                        find("ticker", format!("instrument `{t}`")),
                        line,
                    );
                } else if BARE_TICKERS.contains(&t) {
                    push(&mut report, find("ticker", format!("ticker `{t}`")), line);
                }
            }
            if !in_front {
                if let Some(d) = has_date(line) {
                    push(&mut report, find("date", format!("date `{d}`")), line);
                }
                if let Some(c) = tuned_constant(line) {
                    push(
                        &mut report,
                        find("tuned_constant", format!("constant `{c}`")),
                        line,
                    );
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(skill_md: &str) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([("SKILL.md".to_string(), skill_md.as_bytes().to_vec())])
    }

    fn md(body: &str) -> String {
        format!("---\nname: x\ndescription: use when testing\n---\n{body}")
    }

    fn rules(r: &LintReport) -> Vec<&'static str> {
        r.findings.iter().map(|f| f.rule).collect()
    }

    #[test]
    fn a_clean_skill_passes() {
        let r = lint_bundle(&bundle(&md(
            "1. Read the series.\n2. State the null.\n3. Use alpha 0.05.\nVersion 1.2.3.\n",
        )));
        assert!(r.passes(), "{:?}", r.findings);
        assert!(r.suppressed.is_empty());
    }

    #[test]
    fn instruments_are_caught_in_each_spelling_and_the_line_is_named() {
        for body in [
            "Trade BTC-USD hourly.",
            "ETH/USDT works",
            "buy SOLUSDT now",
            "a BTC example",
        ] {
            let r = lint_bundle(&bundle(&md(&format!("ok\n{body}\n"))));
            assert_eq!(rules(&r), vec!["ticker"], "{body}");
            assert_eq!(
                r.findings[0].line, 6,
                "{body}: the second body line is line 6"
            );
        }
        // Words that merely contain letters are not tickers.
        assert!(lint_bundle(&bundle(&md("the USD leg, a LINKED list, BTCfoo\n"))).passes());
    }

    #[test]
    fn dates_are_caught_and_version_strings_are_not() {
        let r = lint_bundle(&bundle(&md("Data until 2024-05-17 only.\n")));
        assert_eq!(rules(&r), vec!["date"]);
        assert_eq!(
            lint_bundle(&bundle(&md("on 2023/01/31\n"))).findings.len(),
            1
        );
        assert!(lint_bundle(&bundle(&md(
            "release 2024-13-45 and 1.2.3 and 20240517123\n"
        )))
        .passes());
    }

    #[test]
    fn a_fitted_looking_constant_fails_and_round_numbers_do_not() {
        let r = lint_bundle(&bundle(&md("scale by 0.0137 here\n")));
        assert_eq!(rules(&r), vec!["tuned_constant"]);
        assert!(r.findings[0].text.contains("0.0137"));
        assert!(lint_bundle(&bundle(&md("use 0.5, 0.05, 2.0, 1.25 and 10.100\n"))).passes());
        assert_eq!(lint_bundle(&bundle(&md("x = 3.14159\n"))).findings.len(), 1);
    }

    #[test]
    fn a_ticker_and_a_fitted_constant_fail_together_as_the_spec_acceptance_names() {
        // AGENT-003 acceptance 4: "A skill with a ticker and a fitted constant fails lint."
        let r = lint_bundle(&bundle(&md("For BTC-USD multiply by 0.0173.\n")));
        let mut got = rules(&r);
        got.sort_unstable();
        assert_eq!(got, vec!["ticker", "tuned_constant"]);
    }

    #[test]
    fn capabilities_cannot_be_granted_by_a_skill() {
        let r = lint_bundle(&bundle(
            "---\nname: x\ndescription: d\nallowed-tools: Bash\nhooks: pre\n---\nbody\n",
        ));
        assert_eq!(rules(&r), vec!["capability", "capability"]);
        assert_eq!(r.findings[0].line, 4);
        let r = lint_bundle(&bundle(&md("!`rm -rf /`\n")));
        assert_eq!(rules(&r), vec!["capability"]);
    }

    #[test]
    fn scripts_may_not_reach_the_network_a_process_or_a_credential() {
        let mut f = bundle(&md("ok\n"));
        f.insert("scripts/a.py".into(), b"import requests\nx = 1\n".to_vec());
        f.insert("scripts/b.py".into(), b"import subprocess\n".to_vec());
        f.insert("scripts/c.py".into(), b"api_key = 'abc'\n".to_vec());
        f.insert(
            "scripts/d.py".into(),
            b"data = open('../../etc/passwd')\n".to_vec(),
        );
        f.insert(
            "scripts/ok.py".into(),
            b"import math\nprint(math.sqrt(2))\n".to_vec(),
        );
        let r = lint_bundle(&f);
        let files: Vec<&str> = r.findings.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(
            files,
            vec![
                "scripts/a.py",
                "scripts/b.py",
                "scripts/c.py",
                "scripts/d.py"
            ]
        );
        assert!(r.findings.iter().all(|f| f.rule == "capability"));
    }

    #[test]
    fn a_capability_finding_cannot_be_suppressed_but_a_ticker_can_with_a_reason() {
        let mut f = bundle(&md("ok\n"));
        f.insert(
            "scripts/a.py".into(),
            b"import requests  # lint: allow(capability) -- it is fine\n".to_vec(),
        );
        assert_eq!(
            rules(&lint_bundle(&f)),
            vec!["capability"],
            "SK-12: a reason is not a capability"
        );

        let with = lint_bundle(&bundle(&md("Illustration: BTC-USD <!-- lint: allow(ticker) -- worked example of the naming format -->\n")));
        assert!(with.passes());
        assert_eq!(with.suppressed.len(), 1);
        assert_eq!(
            with.suppressed[0].reason,
            "worked example of the naming format -->"
        );

        // No reason, no suppression.
        let bare = lint_bundle(&bundle(&md("BTC-USD lint: allow(ticker)\n")));
        assert_eq!(rules(&bare), vec!["ticker"]);
        let empty = lint_bundle(&bundle(&md("BTC-USD lint: allow(ticker) --   \n")));
        assert_eq!(rules(&empty), vec!["ticker"]);
        // A suppression names its rule; another rule's does not apply.
        let wrong = lint_bundle(&bundle(&md("BTC-USD lint: allow(date) -- because\n")));
        assert_eq!(rules(&wrong), vec!["ticker"]);
    }

    #[test]
    fn size_limits_are_enforced() {
        let long_desc = "word ".repeat(120);
        let r = lint_bundle(&bundle(&format!(
            "---\nname: x\ndescription: {long_desc}\n---\nb\n"
        )));
        assert_eq!(rules(&r), vec!["description_tokens"]);
        let big = "line\n".repeat(MAX_BODY_LINES + 1);
        let r = lint_bundle(&bundle(&md(&big)));
        assert_eq!(rules(&r), vec!["body_lines"]);
        assert!(lint_bundle(&bundle(&md(&"line\n".repeat(MAX_BODY_LINES)))).passes());
    }

    #[test]
    fn the_shipped_core_skills_pass_the_hard_rules_and_only_the_teaching_skill_names_a_forbidden_thing(
    ) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/core");
        let skills = crate::core::load_core_dir(&root).expect("the shipped core loads");
        let mut hard = Vec::new();
        let mut soft_outside_the_teacher = Vec::new();
        for s in &skills {
            let r = lint_bundle(&s.files);
            for f in r.hard_findings() {
                hard.push(format!("{}:{}:{} {}", s.name, f.file, f.line, f.text));
            }
            if s.name != "skill-authoring" {
                for f in r.findings.iter().filter(|f| !HARD_RULES.contains(&f.rule)) {
                    soft_outside_the_teacher
                        .push(format!("{}:{}:{} {}", s.name, f.file, f.line, f.text));
                }
            }
        }
        assert!(
            hard.is_empty(),
            "core skills breaking a hard rule: {hard:#?}"
        );
        assert!(
            soft_outside_the_teacher.is_empty(),
            "a core skill other than `skill-authoring` names an instrument, date or fitted constant:              {soft_outside_the_teacher:#?}"
        );
    }

    #[test]
    fn hard_findings_are_the_capability_and_size_rules_only() {
        let r = lint_bundle(&bundle(&md("BTC-USD at 0.0137
!`ls`
")));
        assert_eq!(r.findings.len(), 3);
        let hard = r.hard_findings();
        assert_eq!(hard.len(), 1);
        assert_eq!(hard[0].rule, "capability");
    }
}
