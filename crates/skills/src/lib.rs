//! The agent's Python skill library — code it writes, keeps, and gets better at.
//!
//! ## What a skill is
//!
//! A Python file with a declared contract: it reads one JSON object from stdin
//! and writes one to stdout. That is the whole interface. No imports from the
//! platform, no database handle, no network — a skill is a **pure data
//! transformation**, and everything it needs arrives in its input.
//!
//! The narrowness is the point. A skill that could reach the ledger would be a
//! second, unaudited path to the ledger; a skill that could reach the network
//! would be a way around [`webfetch::egress`], which is the one place every
//! outbound decision is supposed to be made. So a skill gets data and returns
//! data, and the caller decides what that data was and what happens to the
//! answer.
//!
//! ## Why this is the shape of "getting better"
//!
//! The external review's arithmetic: alpha feedback is roughly twenty-four noisy
//! labels a year on a ninety-day lag, and process feedback is thousands of
//! events resolved in seconds. Anything the agent can actually learn has to be
//! scored on the dense signal, and *"did this transformation run and produce
//! what it said it would"* is about as dense as feedback gets.
//!
//! So a skill accumulates a record. [`SkillStats`] counts runs, failures and
//! contract violations, and a skill that keeps failing is visibly worse than one
//! that does not — without anyone training anything. That is a much smaller
//! claim than "the agent learns", and it is a claim this platform can actually
//! support.
//!
//! ## Immutability, for the same reason as everywhere else
//!
//! A skill revision is content-addressed and append-only. Editing a skill in
//! place would mean a run recorded last week names code that no longer exists —
//! the same defect the strategy store had before ADR-P6-04, in a place where it
//! would be even harder to notice.

pub mod contract;
pub mod core;
pub mod lifecycle;
pub mod lint;
pub mod materialiser;
pub mod mount;
pub mod quality;
pub mod registry;
pub mod run;
pub mod runner;
pub mod signing;
pub mod template;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use contract::{Contract, ContractError};
pub use run::{Outcome, RunRecord, SkillStats};
pub use runner::{Completed, Limits, RunnerConfig};
pub use template::{DataKey, SeriesKind, Template, TemplateError, TemplateHeader};

/// The content address of one skill revision: lowercase hex SHA-256 of the
/// source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SkillHash(String);

impl SkillHash {
    /// Hash a skill's source. The only way to mint one from code.
    #[must_use]
    pub fn of(source: &str) -> Self {
        // Line endings normalised first: the same skill written on Windows and
        // on Linux is the same skill, and migration 0036 is the standing
        // reminder of what happens when a hash disagrees with that.
        let normalised = source.replace("\r\n", "\n");
        Self(hex::encode(Sha256::digest(normalised.as_bytes())))
    }

    /// Accept an existing hash, checking its shape.
    ///
    /// # Errors
    /// When `s` is not 64 lowercase hex characters.
    pub fn parse(s: impl Into<String>) -> Result<Self, SkillError> {
        let s = s.into();
        let ok = s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then_some(Self(s.clone()))
            .ok_or(SkillError::MalformedHash(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn short(&self) -> &str {
        &self.0[..12]
    }
}

impl std::fmt::Display for SkillHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<SkillHash> for String {
    fn from(h: SkillHash) -> Self {
        h.0
    }
}

impl TryFrom<String> for SkillHash {
    type Error = SkillError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

/// One immutable version of a skill.
///
/// No public constructor: [`Skill::author`] is the only route, and it validates
/// the source before a revision exists. A skill that cannot be described by its
/// own contract should never reach a hash, let alone the database.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Skill {
    hash: SkillHash,
    /// Stable slug. A *pointer*, like `strategy_id`: it names the latest
    /// revision and is not an identity.
    name: String,
    /// What it does, in the author's words. Read by the agent when choosing a
    /// skill, so it is the difference between a library and a pile of files.
    description: String,
    source: String,
    contract: Contract,
    /// The revision this was edited from, when it was edited from one.
    parent: Option<SkillHash>,
}

impl Skill {
    /// Author a new revision.
    ///
    /// # Errors
    /// [`SkillError::Invalid`] when the name, description or source fails the
    /// checks in [`validate_source`].
    pub fn author(
        name: impl Into<String>,
        description: impl Into<String>,
        source: impl Into<String>,
        contract: Contract,
        parent: Option<SkillHash>,
    ) -> Result<Self, SkillError> {
        let name = name.into();
        let description = description.into();
        let source = source.into();

        if !is_valid_name(&name) {
            return Err(SkillError::Invalid(format!(
                "{name:?} is not a usable skill name: use lowercase letters, digits and \
                 underscores, 3 to 64 characters. The name is how you will call it later."
            )));
        }
        if description.trim().len() < 20 {
            return Err(SkillError::Invalid(
                "a skill needs a description of at least 20 characters. You will be choosing \
                 between these later from the description alone, and 'helper' will not be enough \
                 to tell two of them apart."
                    .into(),
            ));
        }
        validate_source(&source)?;

        Ok(Self {
            hash: SkillHash::of(&source),
            name,
            description,
            source,
            contract,
            parent,
        })
    }

    #[must_use]
    pub const fn hash(&self) -> &SkillHash {
        &self.hash
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    #[must_use]
    pub const fn contract(&self) -> &Contract {
        &self.contract
    }
    #[must_use]
    pub const fn parent(&self) -> Option<&SkillHash> {
        self.parent.as_ref()
    }
}

/// A skill name: lowercase, underscore-separated, 3–64 characters.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    (3..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
}

/// Imports a skill may not use.
///
/// **This is not the sandbox.** The sandbox is the runner, which blocks the
/// network at the socket layer and caps CPU and memory; a static scan of source
/// text can be defeated by `__import__("so"+"cket")` and anyone who wants to get
/// around it will.
///
/// It exists to turn a *mistake* into a clear message at authoring time instead
/// of a confusing failure at run time. An agent that writes `import requests`
/// should be told "skills have no network, here is why" while it is still
/// writing, not handed a connection error three minutes later.
const DISCOURAGED_IMPORTS: &[(&str, &str)] = &[
    (
        "socket",
        "skills run with no network; the fetch tools are the only way out",
    ),
    (
        "requests",
        "skills run with no network; use the web_search and fetch_url tools instead",
    ),
    ("urllib.request", "skills run with no network"),
    ("httpx", "skills run with no network"),
    ("aiohttp", "skills run with no network"),
    (
        "psycopg",
        "skills never touch the database; pass the rows in as input",
    ),
    (
        "psycopg2",
        "skills never touch the database; pass the rows in as input",
    ),
    (
        "sqlalchemy",
        "skills never touch the database; pass the rows in as input",
    ),
    (
        "clickhouse_driver",
        "skills never touch the database; pass the rows in as input",
    ),
    (
        "subprocess",
        "a skill is one transformation, not a process launcher",
    ),
    (
        "ctypes",
        "a skill is Python, not a way to call into the host",
    ),
    ("multiprocessing", "the runner owns the process model"),
];

/// Structural checks on a skill's source.
///
/// # Errors
/// [`SkillError::Invalid`] with a message saying what to do instead.
pub fn validate_source(source: &str) -> Result<(), SkillError> {
    if source.trim().is_empty() {
        return Err(SkillError::Invalid("a skill needs a body".into()));
    }
    if source.len() > 256 * 1024 {
        return Err(SkillError::Invalid(
            "a skill over 256 KB is a program, not a skill. Split it.".into(),
        ));
    }
    if !source.contains("def run(") {
        return Err(SkillError::Invalid(
            "a skill must define `def run(payload: dict) -> dict`. The runner calls it with the \
             parsed input and serialises what it returns; everything else in the file is \
             support code."
                .into(),
        ));
    }

    // Comment-stripped, so a line explaining why a skill avoids `requests` does
    // not trip the check that skills avoid `requests`.
    let code: String = source
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");

    for (module, why) in DISCOURAGED_IMPORTS {
        let patterns = [format!("import {module}"), format!("from {module}")];
        if patterns.iter().any(|p| mentions(&code, p)) {
            return Err(SkillError::Invalid(format!(
                "this skill imports `{module}`, which will not work: {why}. The runner blocks it \
                 at the sandbox boundary, so this is a message now instead of a confusing \
                 failure later."
            )));
        }
    }
    Ok(())
}

/// Whether `code` contains `needle` as a whole module path.
///
/// Boundary-aware, so `import psycopg2` is not reported as `psycopg`: a message
/// naming a module the author did not write sends them looking for a line that
/// is not there.
fn mentions(code: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = code[from..].find(needle) {
        let end = from + rel + needle.len();
        let next_is_part_of_the_name = code[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !next_is_part_of_the_name {
            return true;
        }
        from = end;
    }
    false
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillError {
    #[error("{0}")]
    Invalid(String),
    #[error("'{0}' is not a skill hash: expected 64 lowercase hex characters")]
    MalformedHash(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_source() -> &'static str {
        "import statistics\n\ndef run(payload):\n    xs = payload['values']\n    return {'mean': statistics.fmean(xs)}\n"
    }

    fn contract() -> Contract {
        Contract::new(["values"], ["mean"], "Mean of a list of numbers.").unwrap()
    }

    fn skill() -> Skill {
        Skill::author(
            "mean_of_values",
            "Computes the arithmetic mean of a list of numbers supplied as `values`.",
            good_source(),
            contract(),
            None,
        )
        .unwrap()
    }

    // ── identity ─────────────────────────────────────────────────────────────

    #[test]
    fn the_same_source_hashes_the_same_way() {
        assert_eq!(SkillHash::of(good_source()), SkillHash::of(good_source()));
    }

    /// Migration 0036 is the standing reminder of what happens when a hash
    /// disagrees about line endings.
    #[test]
    fn line_endings_do_not_change_a_skills_identity() {
        let lf = "def run(p):\n    return p\n";
        let crlf = "def run(p):\r\n    return p\r\n";
        assert_eq!(SkillHash::of(lf), SkillHash::of(crlf));
    }

    #[test]
    fn changing_the_source_changes_the_hash() {
        let a = SkillHash::of(good_source());
        let b = SkillHash::of(&format!("{}\n# tweak\n", good_source()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_malformed_hash_is_refused() {
        assert!(SkillHash::parse("a".repeat(64)).is_ok());
        assert!(SkillHash::parse("a".repeat(63)).is_err());
        assert!(SkillHash::parse("A".repeat(64)).is_err());
        assert!(SkillHash::parse("zz").is_err());
    }

    // ── authoring ────────────────────────────────────────────────────────────

    #[test]
    fn a_well_formed_skill_authors() {
        let s = skill();
        assert_eq!(s.name(), "mean_of_values");
        assert_eq!(s.hash().as_str().len(), 64);
        assert!(s.parent().is_none());
    }

    #[test]
    fn a_revision_records_what_it_was_edited_from() {
        let first = skill();
        let second = Skill::author(
            "mean_of_values",
            "Computes the arithmetic mean, ignoring non-numeric entries in `values`.",
            "def run(payload):\n    xs = [x for x in payload['values'] if isinstance(x, (int, float))]\n    return {'mean': sum(xs) / len(xs)}\n",
            contract(),
            Some(first.hash().clone()),
        )
        .unwrap();
        assert_eq!(second.parent(), Some(first.hash()));
        assert_ne!(second.hash(), first.hash());
    }

    /// The agent chooses between skills from the description alone. "helper"
    /// cannot tell two of them apart.
    #[test]
    fn a_skill_needs_a_description_worth_reading() {
        let err = Skill::author("x_skill", "helper", good_source(), contract(), None).unwrap_err();
        assert!(err.to_string().contains("20 characters"), "{err}");
    }

    #[test]
    fn a_skill_name_must_be_a_usable_identifier() {
        for bad in [
            "A_Skill",
            "has space",
            "hy-phen",
            "ab",
            "1starts_with_digit",
            &"x".repeat(65),
        ] {
            assert!(
                Skill::author(
                    bad,
                    "A description long enough to pass.",
                    good_source(),
                    contract(),
                    None
                )
                .is_err(),
                "{bad:?} should be refused"
            );
        }
        for good in ["ok", "a_b", "skill_2", "compute_rolling_sharpe"]
            .iter()
            .skip(1)
        {
            assert!(is_valid_name(good), "{good:?} should be accepted");
        }
    }

    // ── the source checks ────────────────────────────────────────────────────

    #[test]
    fn a_skill_without_a_run_function_is_refused_with_the_signature() {
        let err = validate_source("x = 1\n").unwrap_err();
        assert!(
            err.to_string().contains("def run(payload: dict) -> dict"),
            "{err}"
        );
    }

    /// Not the sandbox — a message at authoring time instead of a confusing
    /// failure three minutes into a run.
    #[test]
    fn network_and_database_imports_are_refused_with_the_reason() {
        for (module, _) in DISCOURAGED_IMPORTS {
            let src = format!("import {module}\ndef run(p):\n    return p\n");
            let err = validate_source(&src).unwrap_err();
            assert!(err.to_string().contains(module), "{module}: {err}");
        }
        let err =
            validate_source("from requests import get\ndef run(p):\n    return p\n").unwrap_err();
        assert!(err.to_string().contains("no network"), "{err}");
    }

    /// A comment explaining why a skill avoids `requests` must not trip the
    /// check that skills avoid `requests`.
    #[test]
    fn a_comment_mentioning_a_blocked_module_does_not_trip_the_check() {
        let src = "# deliberately no `import requests` here: skills have no network\ndef run(p):\n    return p\n";
        assert!(validate_source(src).is_ok());
    }

    #[test]
    fn an_empty_or_enormous_skill_is_refused() {
        assert!(validate_source("   \n").is_err());
        let huge = format!("def run(p):\n    return p\n# {}", "x".repeat(300_000));
        assert!(validate_source(&huge)
            .unwrap_err()
            .to_string()
            .contains("Split it"));
    }

    #[test]
    fn a_skill_round_trips_through_json() {
        let s = skill();
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Skill>(&j).unwrap(), s);
    }
}
