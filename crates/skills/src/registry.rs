//! Admission: what a bundle must be before the registry will hold it (Set R-a
//! RA-1.2, RA-1.4, RA-3.1, RA-4.4; ADR-0028; AGENT-003 §6.3).
//!
//! Everything here is pure. The Postgres half (`skill_registry.admit()`, the bundle
//! store, the routes) lives in `crates/api` and calls these functions, so the rule
//! that decides is one definition and the transaction that records it is another.
//!
//! # One gate, not two
//!
//! [`Draft::check`] runs, in order: [`crate::core::validate`] (the same function
//! core loading uses, so a user skill cannot exceed what a core skill may: RA-1.4),
//! [`crate::lint::lint_bundle`] (every rule for an agent-authored skill; only the
//! hard rules for release-signed core, ADR-RA-07), and, for a bundle that declares
//! `kind: procedure`, [`check_procedure`] (RA-4.4).
//!
//! # The version hash is the bundle digest
//!
//! A version is content-addressed: its hash *is* [`crate::core::bundle_digest`] of
//! the bundle. Two admissions of the same bytes are the same version, and a changed
//! `SKILL.md` can only ever be a new one (RA-1.5: "never an edit").

use std::collections::{BTreeMap, BTreeSet};

use crate::core::{self, split_skill_md, CoreSkillError};
use crate::lint::{lint_bundle, Finding, Suppressed};
use crate::materialiser::Scope;

/// `skills.patch_threshold`'s default (RA-3.1). A *default*: the API reads the
/// setting and passes it to [`decide`], so the number is configuration.
pub const DEFAULT_PATCH_THRESHOLD: f64 = 0.85;

/// Exposures a provisional skill needs before it can become admitted (ADR-RA-04).
pub const DEFAULT_ADMIT_AFTER_EXPOSURES: u32 = 20;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AdmissionError {
    #[error("{0}")]
    Invalid(String),
    #[error("the lint refused the bundle: {}", fmt_findings(.0))]
    Lint(Vec<Finding>),
    #[error("the procedure is not admissible: {}", .0.join("; "))]
    Procedure(Vec<String>),
}

fn fmt_findings(f: &[Finding]) -> String {
    f.iter()
        .map(|x| format!("{}:{} [{}] {}", x.file, x.line, x.rule, x.text))
        .collect::<Vec<_>>()
        .join("; ")
}

impl From<CoreSkillError> for AdmissionError {
    fn from(e: CoreSkillError) -> Self {
        Self::Invalid(e.to_string())
    }
}

/// The tools a procedure may name, and which of them reach the network.
#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    tools: BTreeMap<String, bool>,
}

impl Catalogue {
    /// `(tool name, reaches the network)`.
    pub fn new(tools: impl IntoIterator<Item = (String, bool)>) -> Self {
        Self {
            tools: tools.into_iter().collect(),
        }
    }
    #[must_use]
    pub fn contains(&self, tool: &str) -> bool {
        self.tools.contains_key(tool)
    }
    #[must_use]
    pub fn reaches_network(&self, tool: &str) -> bool {
        self.tools.get(tool).copied().unwrap_or(false)
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// A bundle that passed every static rule, ready for `skill_registry.admit()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub name: String,
    pub category: String,
    pub scope: Scope,
    pub description: String,
    pub description_tokens: i32,
    pub body_lines: i32,
    /// Equal to the bundle digest.
    pub version_hash: String,
    pub files: BTreeMap<String, Vec<u8>>,
    /// Findings an author suppressed with a reason; the Reviewer reads them.
    pub suppressed: Vec<Suppressed>,
}

impl Draft {
    /// Check a bundle for admission at `scope`.
    ///
    /// # Errors
    /// [`AdmissionError`] naming the first gate the bundle failed.
    pub fn check(
        scope: Scope,
        category: &str,
        name: &str,
        files: BTreeMap<String, Vec<u8>>,
        catalogue: &Catalogue,
    ) -> Result<Self, AdmissionError> {
        let skill = core::validate(category, name, files)?;
        let report = lint_bundle(&skill.files);
        let findings: Vec<Finding> = if scope == Scope::Core {
            report.hard_findings().into_iter().cloned().collect()
        } else {
            report.findings.clone()
        };
        if !findings.is_empty() {
            return Err(AdmissionError::Lint(findings));
        }
        check_procedure(&skill.files, catalogue)?;
        let text = std::str::from_utf8(&skill.files["SKILL.md"])
            .map_err(|_| AdmissionError::Invalid("SKILL.md is not UTF-8".into()))?;
        let (_, body) = split_skill_md(name, text)?;
        Ok(Self {
            name: skill.name,
            category: skill.category,
            scope,
            description_tokens: i32::try_from(core::estimate_tokens(&skill.description))
                .unwrap_or(i32::MAX)
                .max(1),
            description: skill.description,
            body_lines: i32::try_from(body.lines().count()).unwrap_or(i32::MAX),
            version_hash: skill.digest,
            files: skill.files,
            suppressed: report.suppressed,
        })
    }

    /// The `SKILL.md` body, for similarity and search.
    #[must_use]
    pub fn body(&self) -> String {
        body_of(&self.files)
    }
}

/// A bundle's `SKILL.md` body (empty when it has none).
#[must_use]
pub fn body_of(files: &BTreeMap<String, Vec<u8>>) -> String {
    files
        .get("SKILL.md")
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|t| split_skill_md("skill", t).ok())
        .map(|(_, body)| body)
        .unwrap_or_default()
}

// ── Procedure skills (RA-4.4) ───────────────────────────────────────────────

/// Frontmatter keys a procedure may never carry: each one asks for a capability.
const CAPABILITY_KEYS: [&str; 7] = [
    "allowed-tools",
    "allowed_tools",
    "hooks",
    "shell",
    "network",
    "permissions",
    "env",
];

/// The sections a procedure skill must have, as heading prefixes.
pub const PROCEDURE_SECTIONS: [&str; 3] = ["## Steps", "## Evidence", "## Failure"];

/// The tool references (`[tool:name]`) in a body, in order of appearance.
#[must_use]
pub fn tool_refs(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("[tool:") {
        let after = &rest[i + "[tool:".len()..];
        match after.find(']') {
            Some(j) => {
                out.push(after[..j].trim().to_string());
                rest = &after[j + 1..];
            }
            None => break,
        }
    }
    out
}

/// Whether a bundle declares itself a procedure.
#[must_use]
pub fn is_procedure(files: &BTreeMap<String, Vec<u8>>) -> bool {
    files
        .get("SKILL.md")
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|t| split_skill_md("skill", t).ok())
        .is_some_and(|(f, _)| f.get("kind").map(String::as_str) == Some("procedure"))
}

/// A procedure skill is an instruction bundle: ordered steps, the tools it uses, the
/// evidence it produces and the failure it was written for. It grants no capability
/// (SK-12), so it may not ask for shell, hooks or network, and it may only *name*
/// tools that exist in the catalogue, none of which reaches the network.
///
/// A bundle that is not `kind: procedure` is not checked here.
///
/// # Errors
/// [`AdmissionError::Procedure`] listing every problem found, not just the first.
pub fn check_procedure(
    files: &BTreeMap<String, Vec<u8>>,
    catalogue: &Catalogue,
) -> Result<(), AdmissionError> {
    if !is_procedure(files) {
        return Ok(());
    }
    let text = files
        .get("SKILL.md")
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or_default();
    let Ok((fields, body)) = split_skill_md("skill", text) else {
        return Ok(()); // core::validate already refused a malformed SKILL.md
    };
    let mut problems = Vec::new();
    for key in CAPABILITY_KEYS {
        if fields.contains_key(key) {
            problems.push(format!(
                "frontmatter `{key}` requests a capability; a procedure grants none"
            ));
        }
    }
    for section in PROCEDURE_SECTIONS {
        if !body.lines().any(|l| l.trim_start().starts_with(section)) {
            problems.push(format!("the procedure has no `{section}` section"));
        }
    }
    let mut seen = BTreeSet::new();
    for tool in tool_refs(&body) {
        if !seen.insert(tool.clone()) {
            continue;
        }
        if !catalogue.contains(&tool) {
            problems.push(format!("`[tool:{tool}]` is not in the tool catalogue"));
        } else if catalogue.reaches_network(&tool) {
            problems.push(format!(
                "`[tool:{tool}]` reaches the network; a procedure may not depend on it"
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(AdmissionError::Procedure(problems))
    }
}

// ── Near-duplicates (RA-3.1) ────────────────────────────────────────────────

fn term_counts(text: &str) -> BTreeMap<String, f64> {
    let mut m: BTreeMap<String, f64> = BTreeMap::new();
    for w in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
    {
        *m.entry(w.to_ascii_lowercase()).or_default() += 1.0;
    }
    m
}

/// Cosine similarity of two bodies over lowercase word counts, in `[0, 1]`.
///
/// This is the **lexical** similarity: the honest v1 while no embedder is wired to
/// the registry (RA-4.1 says so in its response). It is a pure function, so the
/// patch decision is reproducible; a dense score replaces it behind the same
/// signature when the embedder is present.
#[must_use]
pub fn similarity(a: &str, b: &str) -> f64 {
    let (ca, cb) = (term_counts(a), term_counts(b));
    let dot: f64 = ca
        .iter()
        .filter_map(|(k, v)| cb.get(k).map(|w| v * w))
        .sum();
    let na = ca.values().map(|v| v * v).sum::<f64>().sqrt();
    let nb = cb.values().map(|v| v * v).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        (dot / (na * nb)).clamp(0.0, 1.0)
    }
}

/// What a proposal becomes.
#[derive(Clone, Debug, PartialEq)]
pub enum Disposition {
    /// No existing skill is close enough: a new skill.
    New,
    /// At or above the threshold: a patch of the nearest existing skill.
    Patch { target: String, similarity: f64 },
}

/// Nearest-neighbour search over existing bodies and the patch decision.
///
/// `existing` is `(skill name, full body)`. The threshold is the caller's
/// configuration (`skills.patch_threshold`), never a constant in here.
#[must_use]
pub fn decide(body: &str, existing: &[(String, String)], threshold: f64) -> (Disposition, f64) {
    let best = existing
        .iter()
        .map(|(n, b)| (n, similarity(body, b)))
        .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(a.0)));
    match best {
        Some((name, s)) if s >= threshold => (
            Disposition::Patch {
                target: name.clone(),
                similarity: s,
            },
            s,
        ),
        Some((_, s)) => (Disposition::New, s),
        None => (Disposition::New, 0.0),
    }
}

// ── Proposals (RA-3.1, RA-3.7) ──────────────────────────────────────────────

/// Why a skill is being proposed (AGENT-003 §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Repetition.
    T1,
    /// Hard-won and platform-verified.
    T2,
    /// A pitfall a rejection taught.
    T3,
    /// A human asked.
    T4,
}

impl Reason {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "T1" => Self::T1,
            "T2" => Self::T2,
            "T3" => Self::T3,
            "T4" => Self::T4,
            _ => return None,
        })
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T1 => "T1",
            Self::T2 => "T2",
            Self::T3 => "T3",
            Self::T4 => "T4",
        }
    }
}

/// What a source the proposal rests on is.
pub const UNTRUSTED_SOURCES: [&str; 3] = ["reader_extraction", "web_text", "fetched_page"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProposalError {
    #[error("a proposal must cite the trajectory it was distilled from (`source_ref`); a speculative skill is refused")]
    NoSource,
    #[error("`{0}` is not a proposal reason; use T1, T2, T3 or T4")]
    BadReason(String),
    #[error("every source this proposal rests on is untrusted text ({0:?}); a skill distilled only from fetched text is an injection route")]
    OnlyUntrusted(Vec<String>),
}

/// Validate a proposal's provenance. Pure; the route calls it before it writes.
///
/// `sources` are the kinds of evidence the session consumed: at least one must not be
/// in [`UNTRUSTED_SOURCES`]. An empty list means "only the cited trajectory", which
/// is trusted.
///
/// # Errors
/// [`ProposalError`].
pub fn check_proposal(
    reason: &str,
    source_ref: &str,
    sources: &[String],
) -> Result<Reason, ProposalError> {
    if source_ref.trim().is_empty() {
        return Err(ProposalError::NoSource);
    }
    let reason = Reason::parse(reason).ok_or_else(|| ProposalError::BadReason(reason.into()))?;
    if !sources.is_empty()
        && sources
            .iter()
            .all(|s| UNTRUSTED_SOURCES.contains(&s.as_str()))
    {
        return Err(ProposalError::OnlyUntrusted(sources.to_vec()));
    }
    Ok(reason)
}

// ── Verification (RA-3.3) ───────────────────────────────────────────────────

/// The steps a proposal must pass before admission. Each is a row in
/// `skill_registry.skill_verdict`; a step with no row has not passed.
pub const REQUIRED_STEPS: [&str; 6] = [
    "lint",
    "procedure",
    "triggers",
    "coexistence",
    "replay",
    "reviewer",
];

/// Whether a bundle declares `kind: analysis-code`: a skill that is a series transform
/// (`scripts/run.py`) rather than instructions, and so owes the quality tests of
/// [`crate::quality`] before it is admitted (RA-3.5).
#[must_use]
pub fn is_analysis_code(files: &BTreeMap<String, Vec<u8>>) -> bool {
    files
        .get("SKILL.md")
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|t| split_skill_md("skill", t).ok())
        .is_some_and(|(f, _)| f.get("kind").map(String::as_str) == Some("analysis-code"))
}

/// Whether a bundle declares `claims: savings`: it says it makes work cheaper, so it owes
/// the T5 condition (at least 20% fewer tokens or tool calls at equal success, RA-3.6).
#[must_use]
pub fn claims_savings(files: &BTreeMap<String, Vec<u8>>) -> bool {
    files
        .get("SKILL.md")
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|t| split_skill_md("skill", t).ok())
        .is_some_and(|(f, _)| f.get("claims").map(String::as_str) == Some("savings"))
}

/// The steps this bundle must pass: the six every proposal owes, plus `tests` for
/// analysis code and `savings` for a skill that claims to save work.
#[must_use]
pub fn required_steps(files: &BTreeMap<String, Vec<u8>>) -> Vec<&'static str> {
    let mut v = REQUIRED_STEPS.to_vec();
    if is_analysis_code(files) {
        v.push("tests");
    }
    if claims_savings(files) {
        v.push("savings");
    }
    v
}

/// [`first_unmet_step`] over an explicit list of required steps.
#[must_use]
pub fn first_unmet_in<'a>(required: &[&'a str], latest: &BTreeMap<String, bool>) -> Option<&'a str> {
    required.iter().copied().find(|s| latest.get(*s) != Some(&true))
}

/// The first required step that has not passed, given each step's latest verdict
/// (`Some(true)` passed, `Some(false)` failed, `None` never recorded).
#[must_use]
pub fn first_unmet_step(latest: &BTreeMap<String, bool>) -> Option<&'static str> {
    REQUIRED_STEPS
        .into_iter()
        .find(|s| latest.get(*s) != Some(&true))
}

/// Rewrite a bundle's frontmatter `name:` line, so a proposal that becomes a patch of
/// an existing skill carries that skill's name.
#[must_use]
pub fn rename_bundle(
    files: &BTreeMap<String, Vec<u8>>,
    new_name: &str,
) -> BTreeMap<String, Vec<u8>> {
    let mut out = files.clone();
    if let Some(text) = files.get("SKILL.md").and_then(|b| std::str::from_utf8(b).ok()) {
        let mut in_front = false;
        let mut done = false;
        let mut lines: Vec<String> = Vec::new();
        for (i, l) in text.split('\n').enumerate() {
            if i == 0 && l == "---" {
                in_front = true;
            } else if in_front && l == "---" {
                in_front = false;
            }
            if in_front && !done && l.trim_start().starts_with("name:") {
                lines.push(format!("name: {new_name}"));
                done = true;
            } else {
                lines.push(l.to_string());
            }
        }
        out.insert("SKILL.md".to_string(), lines.join("
").into_bytes());
    }
    out
}

/// What the isolated Reviewer is shown (AGENT-003 §6.3: "the bundle, the tests and the
/// task descriptions"). Built from exactly three inputs, so there is nowhere for a
/// trajectory, a finding or a transcript to get in: the Reviewer judges the skill, not
/// the session that produced it.
#[must_use]
pub fn reviewer_context(
    files: &BTreeMap<String, Vec<u8>>,
    tests: &[String],
    task_descriptions: &[String],
) -> String {
    let mut out = String::from(
        "You are reviewing a proposed skill. Judge whether it is a reusable procedure that          grants no capability and contains no finding. You have only what follows.
",
    );
    out.push_str("
## Bundle
");
    for (path, bytes) in files {
        out.push_str(&format!("
### {path}
"));
        out.push_str(&String::from_utf8_lossy(bytes));
        out.push('\n');
    }
    out.push_str("
## Tests
");
    for t in tests {
        out.push_str(&format!("- {t}
"));
    }
    out.push_str("
## Tasks it was evaluated on
");
    for t in task_descriptions {
        out.push_str(&format!("- {t}
"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(name: &str, extra_front: &str, body: &str) -> BTreeMap<String, Vec<u8>> {
        let mut f = BTreeMap::new();
        f.insert(
            "SKILL.md".to_string(),
            format!("---\nname: {name}\ndescription: use when checking a thing\n{extra_front}---\n{body}")
                .into_bytes(),
        );
        f
    }

    fn cat() -> Catalogue {
        Catalogue::new([
            ("get_bars".to_string(), false),
            ("run_backtest".to_string(), false),
            ("fetch_url".to_string(), true),
        ])
    }

    const PROC: &str = "## Steps\n1. Read the bars [tool:get_bars].\n\n## Evidence\nA backtest id.\n\n## Failure\nThe bars were empty.\n";

    #[test]
    fn a_clean_skill_is_admissible_and_its_hash_is_its_digest() {
        let files = bundle("clean-skill", "", "Do the plain thing.\n");
        let d = Draft::check(Scope::Project, "research", "clean-skill", files.clone(), &cat())
            .unwrap();
        assert_eq!(d.version_hash, core::bundle_digest(&files));
        assert_eq!(d.description_tokens, 7);
        assert_eq!(d.body_lines, 1);
    }

    #[test]
    fn a_101_token_description_and_a_501_line_body_are_refused_with_the_count() {
        let mut f = BTreeMap::new();
        let long = "word ".repeat(81); // 405 chars -> 102 tokens
        f.insert(
            "SKILL.md".into(),
            format!("---\nname: big\ndescription: {long}\n---\nbody\n").into_bytes(),
        );
        let e = Draft::check(Scope::User, "c", "big", f, &cat()).unwrap_err();
        assert!(e.to_string().contains("tokens"), "{e}");
        let big = "line\n".repeat(core::MAX_BODY_LINES + 1);
        let e = Draft::check(Scope::User, "c", "big", bundle("big", "", &big), &cat()).unwrap_err();
        assert!(e.to_string().contains("501"), "{e}");
        // Exactly at the limits is fine.
        let ok = "line\n".repeat(core::MAX_BODY_LINES);
        assert!(Draft::check(Scope::User, "c", "big", bundle("big", "", &ok), &cat()).is_ok());
    }

    /// RA-1.4's "one definition": the limits are compared in exactly one place.
    #[test]
    fn the_limits_have_one_definition() {
        let mut desc = 0;
        let mut body = 0;
        for (name, src) in [
            ("core.rs", include_str!("core.rs")),
            ("lint.rs", include_str!("lint.rs")),
            ("registry.rs", include_str!("registry.rs")),
            ("materialiser.rs", include_str!("materialiser.rs")),
        ] {
            let prod = src.split("#[cfg(test)]").next().unwrap();
            desc += prod.matches("> MAX_DESCRIPTION_TOKENS").count();
            body += prod.matches("> MAX_BODY_LINES").count();
            let _ = name;
        }
        assert_eq!((desc, body), (1, 1), "a second copy of a limit can drift");
    }

    #[test]
    fn core_is_held_to_the_hard_rules_and_an_agent_skill_to_all() {
        let named = bundle("names-a-pair", "", "Never write BTC-USD in a skill.\n");
        assert!(matches!(
            Draft::check(Scope::Project, "c", "names-a-pair", named.clone(), &cat()),
            Err(AdmissionError::Lint(_))
        ));
        assert!(Draft::check(Scope::Core, "c", "names-a-pair", named, &cat()).is_ok());
    }

    #[test]
    fn a_procedure_naming_an_unknown_tool_is_refused() {
        let body = PROC.replace("get_bars", "get_unicorns");
        let f = bundle("proc-a", "kind: procedure\n", &body);
        let e = Draft::check(Scope::Project, "c", "proc-a", f, &cat()).unwrap_err();
        assert!(e.to_string().contains("get_unicorns"), "{e}");
    }

    #[test]
    fn a_procedure_requesting_shell_hooks_or_network_is_refused() {
        // Frontmatter capability keys. `allowed-tools` and `hooks` are caught by the
        // core gate first; `shell` and `network` only by the procedure check.
        for key in ["shell: true\n", "network: true\n", "permissions: all\n"] {
            let f = bundle("proc-b", &format!("kind: procedure\n{key}"), PROC);
            assert!(
                matches!(
                    Draft::check(Scope::Project, "c", "proc-b", f, &cat()),
                    Err(AdmissionError::Procedure(_))
                ),
                "{key}"
            );
        }
        let f = bundle("proc-c", "kind: procedure\nallowed-tools: Bash\n", PROC);
        assert!(Draft::check(Scope::Project, "c", "proc-c", f, &cat()).is_err());
        // A step that depends on a network tool.
        let body = PROC.replace("get_bars", "fetch_url");
        let f = bundle("proc-d", "kind: procedure\n", &body);
        let e = Draft::check(Scope::Project, "c", "proc-d", f, &cat()).unwrap_err();
        assert!(e.to_string().contains("network"), "{e}");
    }

    #[test]
    fn a_clean_procedure_is_admitted_and_a_missing_section_is_not() {
        let f = bundle("proc-e", "kind: procedure\n", PROC);
        assert!(Draft::check(Scope::Project, "c", "proc-e", f, &cat()).is_ok());
        let f = bundle(
            "proc-f",
            "kind: procedure\n",
            "## Steps\n1. [tool:get_bars]\n",
        );
        let e = Draft::check(Scope::Project, "c", "proc-f", f, &cat()).unwrap_err();
        assert!(e.to_string().contains("## Evidence"), "{e}");
        assert!(e.to_string().contains("## Failure"), "{e}");
    }

    #[test]
    fn a_non_procedure_is_not_held_to_the_procedure_shape() {
        let f = bundle("plain-a", "", "No sections at all [tool:nonsense].\n");
        assert!(Draft::check(Scope::Project, "c", "plain-a", f, &cat()).is_ok());
    }

    #[test]
    fn a_near_duplicate_becomes_a_patch_and_the_threshold_is_an_argument() {
        let existing = vec![(
            "stationarity-check".to_string(),
            "run the augmented dickey fuller test on the returns then difference the series until stationary".to_string(),
        )];
        let body = "run the augmented dickey fuller test on the returns then difference the series until it is stationary";
        let (d, s) = decide(body, &existing, DEFAULT_PATCH_THRESHOLD);
        assert!(s > 0.85, "{s}");
        assert!(matches!(d, Disposition::Patch { ref target, .. } if target == "stationarity-check"));
        // The same body against a threshold above any score is a new skill: it is configuration.
        let (d, _) = decide(body, &existing, 1.01);
        assert_eq!(d, Disposition::New);
        // An unrelated body is new.
        let (d, _) = decide("size a position from a volatility target", &existing, 0.85);
        assert_eq!(d, Disposition::New);
        // No existing skills: new, similarity zero.
        assert_eq!(decide(body, &[], 0.85), (Disposition::New, 0.0));
    }

    #[test]
    fn similarity_is_symmetric_bounded_and_one_for_identical_text() {
        let (a, b) = ("alpha beta gamma delta", "alpha beta epsilon");
        assert!((similarity(a, b) - similarity(b, a)).abs() < 1e-12);
        assert!((similarity(a, a) - 1.0).abs() < 1e-12);
        assert_eq!(similarity("", a), 0.0);
    }

    #[test]
    fn a_proposal_needs_a_trajectory_a_reason_and_one_trusted_source() {
        assert_eq!(check_proposal("T2", "  ", &[]), Err(ProposalError::NoSource));
        assert!(matches!(
            check_proposal("T9", "run:1", &[]),
            Err(ProposalError::BadReason(_))
        ));
        assert!(check_proposal("T2", "run:1", &[]).is_ok());
        let only = vec!["web_text".to_string(), "reader_extraction".to_string()];
        assert!(matches!(
            check_proposal("T3", "run:1", &only),
            Err(ProposalError::OnlyUntrusted(_))
        ));
        let mixed = vec!["web_text".to_string(), "trajectory".to_string()];
        assert!(check_proposal("T3", "run:1", &mixed).is_ok());
    }

    #[test]
    fn tool_refs_are_read_in_order() {
        assert_eq!(
            tool_refs("a [tool:x] b [tool: y ] c [tool:z"),
            vec!["x".to_string(), "y".to_string()]
        );
    }

    #[test]
    fn a_proposal_is_admissible_only_when_every_required_step_passed() {
        let mut m: BTreeMap<String, bool> = REQUIRED_STEPS.iter().map(|s| (s.to_string(), true)).collect();
        assert_eq!(first_unmet_step(&m), None);
        m.insert("replay".into(), false);
        assert_eq!(first_unmet_step(&m), Some("replay"));
        m.remove("reviewer");
        m.insert("replay".into(), true);
        assert_eq!(first_unmet_step(&m), Some("reviewer"), "a step never recorded has not passed");
    }

    #[test]
    fn renaming_a_bundle_changes_only_the_frontmatter_name() {
        let f = bundle("old-name", "", "name: not-me
body
");
        let r = rename_bundle(&f, "new-name");
        let t = String::from_utf8(r["SKILL.md"].clone()).unwrap();
        assert!(t.contains("
name: new-name
"));
        assert!(t.contains("body"));
        assert!(t.contains("name: not-me"), "only the frontmatter line moves");
        assert!(Draft::check(Scope::Project, "c", "new-name", r, &cat()).is_ok());
    }

    #[test]
    fn the_reviewer_sees_the_bundle_tests_and_tasks_and_nothing_else() {
        let f = bundle("rev", "", "Do the thing.
");
        let ctx = reviewer_context(&f, &["unit: parses".to_string()], &["task one".to_string()]);
        assert!(ctx.contains("Do the thing."));
        assert!(ctx.contains("unit: parses"));
        assert!(ctx.contains("task one"));
        // The signature has no parameter a trajectory could arrive through; the
        // assembled text carries only its three sections.
        let headings: Vec<&str> = ctx.lines().filter(|l| l.starts_with("## ")).collect();
        assert_eq!(headings, vec!["## Bundle", "## Tests", "## Tasks it was evaluated on"]);
    }

    #[test]
    fn analysis_code_owes_tests_and_a_savings_claim_owes_the_savings_condition() {
        let plain = bundle("plain-b", "", "Steps.\n");
        assert_eq!(required_steps(&plain), REQUIRED_STEPS.to_vec());
        let analysis = bundle("series-a", "kind: analysis-code\n", "Steps.\n");
        assert!(is_analysis_code(&analysis));
        assert_eq!(required_steps(&analysis).last(), Some(&"tests"));
        let cheap = bundle("cheap-a", "claims: savings\n", "Steps.\n");
        assert!(claims_savings(&cheap));
        assert_eq!(required_steps(&cheap).last(), Some(&"savings"));
        let mut m: BTreeMap<String, bool> = REQUIRED_STEPS.iter().map(|s| ((*s).to_string(), true)).collect();
        assert_eq!(first_unmet_in(&required_steps(&analysis), &m), Some("tests"), "a never-recorded step has not passed");
        m.insert("tests".into(), true);
        assert_eq!(first_unmet_in(&required_steps(&analysis), &m), None);
    }
}
