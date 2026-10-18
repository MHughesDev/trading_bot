//! The read-only mount a session starts with (Set R-a RA-2.3, RA-2.5, SK-14; AGENT-003
//! §6.2 steps 3 and 6).
//!
//! [`crate::materialiser`] decides *which* skills are resident, purely. This writes
//! them: the I/O half of ADR-RA-02. It is deliberately small and takes the bundles as
//! an argument, so the registry read (an adapter in `crates/api`) and this write can
//! each be tested alone.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::core::{bundle_digest, CoreSkill, FINDER_SKILL};
use crate::materialiser::{glossary_md, RegistrySkill, Resident, Scope, Status};

/// The bundles a mount is built from: skill name to its files.
pub type Bundles = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MountError {
    #[error("resident skill {0:?} has no bundle to mount")]
    MissingBundle(String),
    #[error("{skill}: bundle digest {actual} does not match the admitted version {expected}; nothing was written")]
    DigestMismatch {
        skill: String,
        expected: String,
        actual: String,
    },
    #[error("the resident set has no `{0}` skill, so there is nowhere to put the glossary")]
    NoFinder(&'static str),
    #[error("io: {0}")]
    Io(String),
}

/// What [`write_mount`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountReport {
    /// Skills written, sorted by tier then name.
    pub resident: Vec<String>,
    /// Skill directories that were in the mount and are not resident any more (a
    /// quarantined, deprecated or archived skill leaves at the next session start).
    pub removed: Vec<String>,
    pub files_written: usize,
    pub glossary_sha256: String,
}

fn io_err(e: &std::io::Error, p: &Path) -> MountError {
    MountError::Io(format!("{}: {e}", p.display()))
}

/// Make a path (and, for a directory, everything under it) writable so it can be
/// replaced. Best effort: a failure surfaces as the write or remove that follows.
pub fn clear_readonly(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perm = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        let _ = std::fs::set_permissions(path, perm);
        if meta.is_dir() {
            if let Ok(rd) = std::fs::read_dir(path) {
                for e in rd.flatten() {
                    clear_readonly(&e.path());
                }
            }
        }
    }
}

/// Write a session's resident skills and `skill-finder/GLOSSARY.md` under `dest`
/// (normally `<workspace>/.claude/skills`).
///
/// Every bundle's digest is checked against its admitted version **before anything
/// is written** (AGENT-003 §6.2 step 6): a tampered bundle aborts the mount and the
/// previous mount is left as it was. Files are written read-only, and a skill
/// directory the resident set no longer contains is removed, which is how a
/// revocation takes effect at the next session start while the live session is told
/// by a notice instead (SK-14).
///
/// # Errors
/// [`MountError`].
pub fn write_mount(
    dest: &Path,
    resident: &Resident,
    visible: &[&RegistrySkill],
    bundles: &Bundles,
) -> Result<MountReport, MountError> {
    for (_, s) in &resident.resident {
        let files = bundles
            .get(&s.name)
            .ok_or_else(|| MountError::MissingBundle(s.name.clone()))?;
        let actual = bundle_digest(files);
        if actual != s.version {
            return Err(MountError::DigestMismatch {
                skill: s.name.clone(),
                expected: s.version.clone(),
                actual,
            });
        }
    }
    if !resident.resident.iter().any(|(_, s)| s.name == FINDER_SKILL) {
        return Err(MountError::NoFinder(FINDER_SKILL));
    }
    std::fs::create_dir_all(dest).map_err(|e| io_err(&e, dest))?;

    let keep: Vec<&str> = resident
        .resident
        .iter()
        .map(|(_, s)| s.name.as_str())
        .collect();
    let mut removed = Vec::new();
    for entry in std::fs::read_dir(dest)
        .map_err(|e| io_err(&e, dest))?
        .flatten()
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() && !keep.contains(&name.as_str()) {
            clear_readonly(&path);
            std::fs::remove_dir_all(&path).map_err(|e| io_err(&e, &path))?;
            removed.push(name);
        }
    }
    removed.sort();

    let write_file = |path: &Path, bytes: &[u8]| -> Result<(), MountError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(&e, parent))?;
        }
        if path.exists() {
            clear_readonly(path);
        }
        std::fs::write(path, bytes).map_err(|e| io_err(&e, path))?;
        let mut perm = std::fs::metadata(path)
            .map_err(|e| io_err(&e, path))?
            .permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(path, perm).map_err(|e| io_err(&e, path))
    };

    let mut files_written = 0;
    for (_, s) in &resident.resident {
        let files = &bundles[&s.name];
        // A file the new version dropped must not survive from the old one.
        let skill_dir = dest.join(&s.name);
        if skill_dir.exists() {
            clear_readonly(&skill_dir);
            std::fs::remove_dir_all(&skill_dir).map_err(|e| io_err(&e, &skill_dir))?;
        }
        for (rel, bytes) in files {
            write_file(&skill_dir.join(rel), bytes)?;
            files_written += 1;
        }
    }
    let text = glossary_md(visible);
    let gpath = dest.join(FINDER_SKILL).join("GLOSSARY.md");
    write_file(&gpath, text.as_bytes())?;
    files_written += 1;
    Ok(MountReport {
        resident: resident
            .resident
            .iter()
            .map(|(_, s)| s.name.clone())
            .collect(),
        removed,
        files_written,
        glossary_sha256: hex::encode(Sha256::digest(text.as_bytes())),
    })
}

/// Registry rows for a core set, so the file loader's output flows through the same
/// selection as every other skill.
#[must_use]
pub fn core_as_registry(skills: &[CoreSkill]) -> (Vec<RegistrySkill>, Bundles) {
    let mut rows = Vec::new();
    let mut bundles = Bundles::new();
    for s in skills {
        rows.push(RegistrySkill {
            name: s.name.clone(),
            scope: Scope::Core,
            owner_user: None,
            project: None,
            category: s.category.clone(),
            status: Status::Admitted,
            description: s.description.clone(),
            version: s.digest.clone(),
            pinned: false,
        });
        bundles.insert(s.name.clone(), s.files.clone());
    }
    (rows, bundles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materialiser::{select, visible, Viewer};

    fn scratch(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("mount_{tag}_{}", std::process::id()));
        clear_readonly(&p);
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn core_set() -> (Vec<RegistrySkill>, Bundles) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/core");
        core_as_registry(&crate::core::load_core_dir(&root).unwrap())
    }

    fn viewer() -> Viewer {
        Viewer {
            user: "u1".into(),
            project: String::new(),
        }
    }

    #[test]
    fn two_sessions_get_byte_identical_mounts_that_are_read_only_and_list_every_skill() {
        let (rows, bundles) = core_set();
        let vis: Vec<&RegistrySkill> = rows.iter().collect();
        let res = select(&vis, &[]).unwrap();
        let (a, b) = (scratch("a"), scratch("b"));
        let ra = write_mount(&a, &res, &vis, &bundles).unwrap();
        let rb = write_mount(&b, &res, &vis, &bundles).unwrap();
        assert_eq!(ra, rb);
        for (name, files) in &bundles {
            for rel in files.keys() {
                assert_eq!(
                    std::fs::read(a.join(name).join(rel)).unwrap(),
                    std::fs::read(b.join(name).join(rel)).unwrap()
                );
            }
        }
        let g = std::fs::read_to_string(a.join("skill-finder/GLOSSARY.md")).unwrap();
        assert!(g.contains("`playbook`"));
        assert!(std::fs::metadata(a.join("playbook/SKILL.md"))
            .unwrap()
            .permissions()
            .readonly());
        // Mounting again over a read-only mount works.
        write_mount(&a, &res, &vis, &bundles).unwrap();
        clear_readonly(&a);
        clear_readonly(&b);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn a_tampered_bundle_aborts_the_mount_before_anything_is_written() {
        let (rows, mut bundles) = core_set();
        let vis: Vec<&RegistrySkill> = rows.iter().collect();
        let res = select(&vis, &[]).unwrap();
        bundles
            .get_mut("playbook")
            .unwrap()
            .insert("SKILL.md".into(), b"changed".to_vec());
        let dest = scratch("tamper");
        let err = write_mount(&dest, &res, &vis, &bundles).unwrap_err();
        assert!(matches!(err, MountError::DigestMismatch { ref skill, .. } if skill == "playbook"));
        assert!(!dest.exists(), "nothing may be written when verification fails");
    }

    #[test]
    fn a_quarantined_skill_is_removed_at_the_next_mount_and_leaves_the_glossary() {
        let (mut rows, mut bundles) = core_set();
        let mut files = BTreeMap::new();
        files.insert(
            "SKILL.md".to_string(),
            b"---\nname: extra\ndescription: use when extra\n---\nbody\n".to_vec(),
        );
        rows.push(RegistrySkill {
            name: "extra".into(),
            scope: Scope::Global,
            owner_user: None,
            project: None,
            category: "research".into(),
            status: Status::Admitted,
            description: "use when extra".into(),
            version: bundle_digest(&files),
            pinned: true,
        });
        bundles.insert("extra".into(), files);
        let dest = scratch("revoke");
        let vis = visible(&rows, &viewer()).unwrap();
        let res = select(&vis, &[]).unwrap();
        write_mount(&dest, &res, &vis, &bundles).unwrap();
        assert!(dest.join("extra/SKILL.md").exists());

        rows.last_mut().unwrap().status = Status::Quarantined;
        let vis2 = visible(&rows, &viewer()).unwrap();
        let res2 = select(&vis2, &[]).unwrap();
        let r = write_mount(&dest, &res2, &vis2, &bundles).unwrap();
        assert_eq!(r.removed, vec!["extra".to_string()]);
        assert!(!dest.join("extra").exists());
        let g = std::fs::read_to_string(dest.join("skill-finder/GLOSSARY.md")).unwrap();
        assert!(!g.contains("`extra`"));
        clear_readonly(&dest);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_mount_without_the_finder_is_refused() {
        let (rows, bundles) = core_set();
        let vis: Vec<&RegistrySkill> = rows.iter().filter(|s| s.name != "skill-finder").collect();
        let res = select(&vis, &[]).unwrap();
        assert!(matches!(
            write_mount(&scratch("nofinder"), &res, &vis, &bundles),
            Err(MountError::NoFinder(_))
        ));
    }

    #[test]
    fn a_missing_bundle_is_refused() {
        let (rows, mut bundles) = core_set();
        bundles.remove("tbot-howto");
        let vis: Vec<&RegistrySkill> = rows.iter().collect();
        let res = select(&vis, &[]).unwrap();
        assert!(matches!(
            write_mount(&scratch("missing"), &res, &vis, &bundles),
            Err(MountError::MissingBundle(_))
        ));
    }
}
