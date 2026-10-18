//! Core skills: the bundles every research session starts with (AGENT-003 §6.1,
//! Set L SK-01 / SK-09).
//!
//! These are *instruction* skills (a `SKILL.md` an agent reads), not the Python
//! transformations the rest of this crate runs. Sources live in
//! `skills/core/<category>/<name>/` at the repository root.
//!
//! This is the v0 materialiser: it mounts **every** core skill and a generated
//! glossary. It has no scopes and no per-project selection yet (that is the
//! Set R-a materialiser, AGENT-003 §6.2), and it says so rather than pretending:
//! [`MaterialiseReport::resident`] is the whole core set.
//!
//! Three properties are enforced here, by code and by test, not by convention:
//!
//! * **Byte-identical output.** Two users' sessions get the same bytes. Nothing
//!   in a bundle's rendering depends on time, user, or the order the filesystem
//!   listed a directory (everything is sorted before it is hashed or written).
//! * **Verified on the way out.** Every file is hashed after it is read and the
//!   bundle digest is checked against an expected digest when one is given; a
//!   mismatch aborts before anything is written (AGENT-003 §6.2 step 6).
//! * **A description is a trigger, not prose.** At most [`MAX_DESCRIPTION_TOKENS`]
//!   (estimated), and a body of at most [`MAX_BODY_LINES`] lines. A bundle that
//!   enables tools, hooks or shell execution is refused: core skills are
//!   read-only instructions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A skill description may not exceed this many tokens (AGENT-003 §6.3).
pub const MAX_DESCRIPTION_TOKENS: usize = 100;
/// A skill body may not exceed this many lines (AGENT-003 §6.3).
pub const MAX_BODY_LINES: usize = 500;
/// The skill that carries the glossary.
pub const FINDER_SKILL: &str = "skill-finder";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreSkillError {
    #[error("io: {0}")]
    Io(String),
    #[error("{skill}: {reason}")]
    Invalid { skill: String, reason: String },
    #[error("two core skills are named {0:?}; a name is unique across the whole set")]
    DuplicateName(String),
    #[error("{skill}: bundle digest {actual} does not match the expected {expected}")]
    DigestMismatch {
        skill: String,
        expected: String,
        actual: String,
    },
    #[error("the core set has no `{0}` skill, so there is nowhere to put the glossary")]
    NoFinder(&'static str),
}

fn io(e: &std::io::Error, what: &Path) -> CoreSkillError {
    CoreSkillError::Io(format!("{}: {e}", what.display()))
}

/// One core skill, loaded and validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreSkill {
    pub name: String,
    pub category: String,
    pub description: String,
    /// Every file in the bundle, by path relative to the skill directory, sorted.
    /// Line endings are normalised to `\n` so a bundle checked out on Windows is
    /// the same bundle.
    pub files: BTreeMap<String, Vec<u8>>,
    /// Hex SHA-256 over the sorted `(path, length, bytes)` of every file.
    pub digest: String,
}

/// What [`materialise`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialiseReport {
    /// Skills written, in the order they were written (sorted by category then name).
    pub resident: Vec<String>,
    pub files_written: usize,
    /// Digest of the glossary that was written.
    pub glossary_sha256: String,
}

fn normalise(bytes: &[u8]) -> Vec<u8> {
    match std::str::from_utf8(bytes) {
        Ok(text) if text.contains('\r') => text.replace("\r\n", "\n").into_bytes(),
        _ => bytes.to_vec(),
    }
}

/// Hex SHA-256 over the sorted files of a bundle.
#[must_use]
pub fn bundle_digest(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut h = Sha256::new();
    for (path, bytes) in files {
        h.update(path.as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    hex::encode(h.finalize())
}

/// A rough token count: a token is about four characters. Deliberately an
/// estimate, and deliberately on the high side for prose.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// The one definition of the description limit (SK-08, RA-1.4): `Some(tokens)` when a
/// description is over it. Core loading, admission and the lint all call this, so a
/// user skill cannot exceed what a core skill may and the three cannot disagree.
#[must_use]
pub fn description_tokens_over(description: &str) -> Option<usize> {
    let n = estimate_tokens(description);
    (n > MAX_DESCRIPTION_TOKENS).then_some(n)
}

/// The one definition of the body limit: `Some(lines)` when a body is over it.
#[must_use]
pub fn body_lines_over(lines: usize) -> Option<usize> {
    (lines > MAX_BODY_LINES).then_some(lines)
}

/// Split `SKILL.md` into its frontmatter fields and its body.
///
/// # Errors
/// A `SKILL.md` that does not open with a closed `---` block of `key: value` lines.
pub fn split_skill_md(
    skill: &str,
    text: &str,
) -> Result<(BTreeMap<String, String>, String), CoreSkillError> {
    parse_skill_md(skill, text)
}

fn parse_skill_md(
    skill: &str,
    text: &str,
) -> Result<(BTreeMap<String, String>, String), CoreSkillError> {
    let bad = |reason: &str| CoreSkillError::Invalid {
        skill: skill.to_string(),
        reason: reason.to_string(),
    };
    let rest = text
        .strip_prefix("---\n")
        .ok_or_else(|| bad("SKILL.md must open with a `---` frontmatter block"))?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| bad("the frontmatter block is never closed with `---`"))?;
    let (front, body) = (&rest[..end], &rest[end + 5..]);
    let mut fields = BTreeMap::new();
    for line in front.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (k, v) = line
            .split_once(':')
            .ok_or_else(|| bad("a frontmatter line is not `key: value`"))?;
        fields.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok((fields, body.to_string()))
}

/// Validate one bundle's files into a [`CoreSkill`].
///
/// # Errors
/// A missing or malformed `SKILL.md`, a name that does not match its directory, an
/// over-long description or body, or a field that would let the skill run code.
pub fn validate(
    category: &str,
    dir_name: &str,
    files: BTreeMap<String, Vec<u8>>,
) -> Result<CoreSkill, CoreSkillError> {
    let bad = |reason: String| CoreSkillError::Invalid {
        skill: dir_name.to_string(),
        reason,
    };
    let md = files
        .get("SKILL.md")
        .ok_or_else(|| bad("the bundle has no SKILL.md".into()))?;
    let text = std::str::from_utf8(md).map_err(|_| bad("SKILL.md is not UTF-8".into()))?;
    let (fields, body) = parse_skill_md(dir_name, text)?;

    let name = fields.get("name").cloned().unwrap_or_default();
    if name != dir_name {
        return Err(bad(format!(
            "frontmatter name {name:?} must equal the directory name {dir_name:?}"
        )));
    }
    let description = fields.get("description").cloned().unwrap_or_default();
    if description.is_empty() {
        return Err(bad("a skill needs a description: it is the trigger".into()));
    }
    if let Some(n) = description_tokens_over(&description) {
        return Err(bad(format!(
            "description is about {n} tokens; the limit is {MAX_DESCRIPTION_TOKENS}"
        )));
    }
    if let Some(n) = body_lines_over(body.lines().count()) {
        return Err(bad(format!(
            "body is {n} lines; the limit is {MAX_BODY_LINES}"
        )));
    }
    // Core skills are read-only instructions: nothing that enables a tool, a hook
    // or a shell, and no `!` command-substitution lines.
    for forbidden in ["allowed-tools", "hooks", "allowed_tools"] {
        if fields.contains_key(forbidden) {
            return Err(bad(format!(
                "frontmatter field `{forbidden}` is not allowed on a core skill"
            )));
        }
    }
    if body.lines().any(|l| l.trim_start().starts_with("!`")) {
        return Err(bad(
            "a `!` shell-substitution line is not allowed on a core skill".into(),
        ));
    }
    let digest = bundle_digest(&files);
    Ok(CoreSkill {
        name,
        category: category.to_string(),
        description,
        files,
        digest,
    })
}

fn read_bundle(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, CoreSkillError> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<(), CoreSkillError> {
        for entry in std::fs::read_dir(dir).map_err(|e| io(&e, dir))? {
            let path = entry.map_err(|e| io(&e, dir))?.path();
            if path.is_dir() {
                walk(root, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let bytes = std::fs::read(&path).map_err(|e| io(&e, &path))?;
                out.insert(rel, normalise(&bytes));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out)?;
    Ok(out)
}

/// Load every core skill under `root` (`root/<category>/<name>/SKILL.md`).
/// Sorted by category, then name.
///
/// # Errors
/// Any bundle that fails [`validate`], or two bundles sharing a name.
pub fn load_core_dir(root: &Path) -> Result<Vec<CoreSkill>, CoreSkillError> {
    let mut skills: Vec<CoreSkill> = Vec::new();
    let mut categories: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| io(&e, root))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    categories.sort();
    for cat in categories {
        let category = cat
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&cat)
            .map_err(|e| io(&e, &cat))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let skill = validate(&category, &name, read_bundle(&dir)?)?;
            if skills.iter().any(|s| s.name == skill.name) {
                return Err(CoreSkillError::DuplicateName(skill.name));
            }
            skills.push(skill);
        }
    }
    Ok(skills)
}

/// The glossary: one line per skill, grouped by category. Pure function of the
/// skills, so every session renders the same bytes.
#[must_use]
pub fn glossary(skills: &[CoreSkill]) -> String {
    let mut sorted: Vec<&CoreSkill> = skills.iter().collect();
    sorted.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    let mut out = String::from(
        "# Skill glossary\n\nEvery skill visible to this session. Core skills are always resident.\n",
    );
    let mut current = "";
    for s in sorted {
        if s.category != current {
            out.push_str(&format!("\n## {}\n\n", s.category));
            current = &s.category;
        }
        out.push_str(&format!(
            "- `{}` (core, admitted): {}\n",
            s.name, s.description
        ));
    }
    out
}

/// Write every core skill and the glossary under `dest` (normally
/// `.claude/skills/`), refusing on any digest mismatch **before** writing.
///
/// `expected` maps a skill name to the digest the release signed; a skill listed
/// there must match it exactly. Pass an empty map to skip the check (tests, dev).
///
/// # Errors
/// A digest mismatch, no `skill-finder` skill to carry the glossary, or the
/// filesystem.
pub fn materialise(
    skills: &[CoreSkill],
    expected: &BTreeMap<String, String>,
    dest: &Path,
) -> Result<MaterialiseReport, CoreSkillError> {
    for s in skills {
        let actual = bundle_digest(&s.files);
        if actual != s.digest {
            return Err(CoreSkillError::DigestMismatch {
                skill: s.name.clone(),
                expected: s.digest.clone(),
                actual,
            });
        }
        if let Some(want) = expected.get(&s.name) {
            if *want != actual {
                return Err(CoreSkillError::DigestMismatch {
                    skill: s.name.clone(),
                    expected: want.clone(),
                    actual,
                });
            }
        }
    }
    if !skills.iter().any(|s| s.name == FINDER_SKILL) {
        return Err(CoreSkillError::NoFinder(FINDER_SKILL));
    }

    let mut sorted: Vec<&CoreSkill> = skills.iter().collect();
    sorted.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));

    let mut files_written = 0;
    let mut resident = Vec::new();
    for s in &sorted {
        for (rel, bytes) in &s.files {
            let path = dest.join(&s.name).join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io(&e, parent))?;
            }
            std::fs::write(&path, bytes).map_err(|e| io(&e, &path))?;
            files_written += 1;
        }
        resident.push(s.name.clone());
    }
    let text = glossary(skills);
    let gpath = dest.join(FINDER_SKILL).join("GLOSSARY.md");
    std::fs::write(&gpath, text.as_bytes()).map_err(|e| io(&e, &gpath))?;
    files_written += 1;
    Ok(MaterialiseReport {
        resident,
        files_written,
        glossary_sha256: hex::encode(Sha256::digest(text.as_bytes())),
    })
}

/// The release manifest of a core set: skill name to bundle digest. Written at
/// build time and checked at materialisation.
#[must_use]
pub fn manifest(skills: &[CoreSkill]) -> BTreeMap<String, String> {
    skills
        .iter()
        .map(|s| (s.name.clone(), s.digest.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(name: &str, description: &str, body: &str) -> BTreeMap<String, Vec<u8>> {
        let mut f = BTreeMap::new();
        f.insert(
            "SKILL.md".to_string(),
            format!("---\nname: {name}\ndescription: {description}\n---\n{body}").into_bytes(),
        );
        f
    }

    fn core_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/core")
    }

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("core_skills_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn the_shipped_core_set_loads_and_is_what_the_spec_names() {
        let skills = load_core_dir(&core_root()).expect("the shipped core skills are valid");
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        for want in [
            "qc-rules-and-pitfalls",
            "tbot-howto",
            "skill-finder",
            "skill-authoring",
            "playbook",
        ] {
            assert!(
                names.contains(&want),
                "core set is missing {want}: {names:?}"
            );
        }
    }

    #[test]
    fn two_sessions_materialise_byte_identical_resident_blocks_and_the_glossary_lists_every_skill()
    {
        let skills = load_core_dir(&core_root()).unwrap();
        let (a, b) = (scratch("a"), scratch("b"));
        let ra = materialise(&skills, &manifest(&skills), &a).unwrap();
        let rb = materialise(&skills, &manifest(&skills), &b).unwrap();
        assert_eq!(ra, rb);
        for s in &skills {
            for rel in s.files.keys() {
                assert_eq!(
                    std::fs::read(a.join(&s.name).join(rel)).unwrap(),
                    std::fs::read(b.join(&s.name).join(rel)).unwrap(),
                    "{}/{rel} differs between sessions",
                    s.name
                );
            }
        }
        let g = std::fs::read_to_string(a.join(FINDER_SKILL).join("GLOSSARY.md")).unwrap();
        for s in &skills {
            assert!(
                g.contains(&format!("`{}`", s.name)),
                "glossary omits {}",
                s.name
            );
        }
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn a_tampered_bundle_aborts_before_anything_is_written() {
        let skills = load_core_dir(&core_root()).unwrap();
        let mut expected = manifest(&skills);
        expected.insert("playbook".into(), "0".repeat(64));
        let dest = scratch("tamper");
        let err = materialise(&skills, &expected, &dest).unwrap_err();
        assert!(
            matches!(err, CoreSkillError::DigestMismatch { ref skill, .. } if skill == "playbook")
        );
        assert!(
            !dest.exists(),
            "nothing may be written when verification fails"
        );

        // A bundle whose bytes changed after it was hashed fails too.
        let mut tampered = skills.clone();
        tampered[0]
            .files
            .insert("SKILL.md".into(), b"changed".to_vec());
        assert!(matches!(
            materialise(&tampered, &BTreeMap::new(), &dest).unwrap_err(),
            CoreSkillError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn validation_refuses_what_a_core_skill_may_not_be() {
        assert!(validate("c", "x", bundle("x", "ok", "body\n")).is_ok());
        // Name must match its directory.
        assert!(validate("c", "x", bundle("y", "ok", "body\n")).is_err());
        // Description is a trigger, capped.
        let long = "word ".repeat(200);
        assert!(validate("c", "x", bundle("x", &long, "body\n")).is_err());
        // Body capped.
        let big = "line\n".repeat(MAX_BODY_LINES + 1);
        assert!(validate("c", "x", bundle("x", "ok", &big)).is_err());
        // No code execution surface.
        let mut f = bundle("x", "ok", "body\n");
        f.insert(
            "SKILL.md".into(),
            b"---\nname: x\ndescription: ok\nallowed-tools: Bash\n---\nbody\n".to_vec(),
        );
        assert!(validate("c", "x", f).is_err());
        assert!(validate("c", "x", bundle("x", "ok", "!`rm -rf /`\n")).is_err());
        // No SKILL.md.
        assert!(validate("c", "x", BTreeMap::new()).is_err());
    }

    #[test]
    fn line_endings_do_not_change_a_bundle() {
        let unix = normalise(b"a\nb\n");
        let windows = normalise(b"a\r\nb\r\n");
        assert_eq!(unix, windows);
    }

    #[test]
    fn a_set_without_a_finder_cannot_carry_the_glossary() {
        let s = validate("c", "x", bundle("x", "ok", "b\n")).unwrap();
        let err = materialise(&[s], &BTreeMap::new(), &scratch("nofinder")).unwrap_err();
        assert_eq!(err, CoreSkillError::NoFinder(FINDER_SKILL));
    }
}
