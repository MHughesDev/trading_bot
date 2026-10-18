//! Skills at session start (Set R-a RA-1.5, RA-2.1 to RA-2.4, RA-4.5; Set R-b RB-2.5,
//! RB-2.6; AGENT-003 §6.2).
//!
//! Until this file the materialiser in `crates/skills` had no caller: the core skills
//! were a folder of files that no session read. [`session_start`] is the caller. It runs
//! once at the start of every agent run and:
//!
//! 1. loads the release's core skills from `skills/core` and syncs them into the
//!    registry (idempotent on content hash);
//! 2. settles provisional skills that have earned admission and archives skills that
//!    never earned a place (the lifecycle rules, evaluated where a session starts so no
//!    separate scheduler is needed to reach them);
//! 3. reads what this user may see, applies the model guard (SK-17), selects the
//!    resident set (at most 40, deterministic) and writes the read-only mount to
//!    `<workspace>/.claude/skills`, verifying every bundle's hash first;
//! 4. records one `exposed` row per resident skill;
//! 5. returns the text the charter carries: the resident descriptions and where the
//!    glossary is.
//!
//! # When the registry is not there
//!
//! A database without migration 0210 to 0212 must not stop an agent from running. The
//! fallback is the v0 mount (`skills::core::materialise`: every core skill and a
//! core-only glossary), and the returned [`SessionSkills::degraded`] says why, so the
//! charter tells the model it has the core set only rather than implying more.
//!
//! # Project scope
//!
//! A conversation has no project (`agent_conversations` carries none; `agent_projects`
//! is a separate table nothing links it to), so a session's viewer has no project and
//! project-scope skills are reachable through the API and the glossary route, not the
//! mount. When conversations gain a project this is the one place to pass it.

use std::path::{Path, PathBuf};

use harness::workspace::Workspace;
use skills::core::{load_core_dir, manifest, materialise, CoreSkill};
use skills::materialiser::{exposure_rows, resident_block, select, visible, Scope, Viewer};
use sqlx::PgPool;
use uuid::Uuid;

use crate::skills_store::{admit_after_exposures, SkillStore};

/// What a session knows about its skills.
#[derive(Clone, Debug, Default)]
pub struct SessionSkills {
    /// Names mounted, in resident order.
    pub resident: Vec<String>,
    /// Text for the charter. Empty when nothing could be mounted.
    pub hint: String,
    /// Why the registry was not used, when it was not.
    pub degraded: Option<String>,
    /// Skills left out of the resident set by the model guard or the cap.
    pub withheld: Vec<String>,
}

/// Where the release's core skills are: `TBOT_SKILLS_CORE_DIR`, else `skills/core`
/// beside the working directory, else the source tree this binary was built from.
#[must_use]
pub fn core_dir() -> Option<PathBuf> {
    let candidates = [
        std::env::var("TBOT_SKILLS_CORE_DIR").ok().map(PathBuf::from),
        Some(PathBuf::from("skills/core")),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/core")),
    ];
    candidates.into_iter().flatten().find(|p| p.is_dir())
}

/// The mount directory inside a workspace.
#[must_use]
pub fn mount_dir(ws: &Workspace) -> PathBuf {
    ws.root().join(super::workspace_tools::SKILLS_MOUNT)
}

fn charter_text(resident: &str, withheld: &[String], degraded: Option<&str>) -> String {
    let mut out = String::from(
        "Skills are procedures other sessions proved worth keeping. Each is a folder under \
         `.claude/skills/`; read its `SKILL.md` before doing the task it names. \
         `.claude/skills/skill-finder/GLOSSARY.md` lists every skill you can see, resident or not. \
         The mount is read-only: to keep a procedure of your own, propose it with `propose_skill`.\n\n",
    );
    if let Some(why) = degraded {
        out.push_str(&format!(
            "NOTE: the skill registry was not used this session ({why}); only the core skills are mounted.\n\n"
        ));
    }
    out.push_str("Resident skills (name, version, when to use):\n");
    out.push_str(resident);
    if !withheld.is_empty() {
        out.push_str(&format!(
            "\nWithheld this session (in the glossary, not resident): {}\n",
            withheld.join(", ")
        ));
    }
    out
}

fn v0_fallback(core: &[CoreSkill], dest: &Path, why: String) -> SessionSkills {
    // A mount written read-only by an earlier session has to be writable to be replaced.
    skills::mount::clear_readonly(dest);
    match materialise(core, &manifest(core), dest) {
        Ok(rep) => {
            let block: String = core
                .iter()
                .map(|s| format!("{}\t{}\t{}\n", s.name, &s.digest[..8], s.description))
                .collect();
            SessionSkills {
                resident: rep.resident,
                hint: charter_text(&block, &[], Some(&why)),
                degraded: Some(why),
                withheld: Vec::new(),
            }
        }
        Err(e) => SessionSkills {
            degraded: Some(format!("{why}; and the v0 mount failed too: {e}")),
            ..Default::default()
        },
    }
}

/// Materialise the session's skills. Never fails: a problem becomes
/// [`SessionSkills::degraded`], because an agent without skills is a worse session and
/// an agent that cannot start is no session.
pub async fn session_start(
    pg: &PgPool,
    ws: &Workspace,
    user: Uuid,
    session: Uuid,
    model: &str,
) -> SessionSkills {
    let dest = mount_dir(ws);
    let Some(dir) = core_dir() else {
        return SessionSkills {
            degraded: Some("no skills/core directory found; set TBOT_SKILLS_CORE_DIR".into()),
            ..Default::default()
        };
    };
    let core = match load_core_dir(&dir) {
        Ok(c) => c,
        Err(e) => {
            return SessionSkills {
                degraded: Some(format!("core skills do not load: {e}")),
                ..Default::default()
            }
        }
    };
    // RA-2.5: with a release signing key configured, an unsigned or tampered core set is
    // refused outright; with none, nothing is required and nothing is claimed.
    if let Err(e) = skills::signing::check_core_from_env(&core, &dir) {
        return SessionSkills {
            degraded: Some(format!("the core skills were refused: {e}")),
            ..Default::default()
        };
    }
    let store = SkillStore::new(pg.clone());
    if let Err(e) = store.sync_core(&core).await {
        return v0_fallback(&core, &dest, format!("registry unavailable: {e}"));
    }
    let n = admit_after_exposures();
    if let Ok((promoted, _flagged)) = store.settle_provisional(n).await {
        if !promoted.is_empty() {
            tracing::info!(?promoted, "provisional skills admitted after enough exposures");
        }
    }
    if let Ok(archived) = store.sweep_retirements(n).await {
        if !archived.is_empty() {
            tracing::info!(?archived, "skills archived: no activation or no contribution");
        }
    }

    let mut rows = match store.visible(user, None).await {
        Ok(r) => r,
        Err(e) => return v0_fallback(&core, &dest, format!("registry unavailable: {e}")),
    };

    // SK-17: a library that has not been re-run on this model is not trusted on it.
    // Only non-core skills are withheld; core is the release's and is tested with it.
    let mut withheld: Vec<String> = Vec::new();
    let non_core: Vec<String> = rows
        .iter()
        .filter(|s| s.scope != Scope::Core)
        .map(|s| s.name.clone())
        .collect();
    if !non_core.is_empty() {
        let records = store.reevaluations(model).await.unwrap_or_default();
        match skills::lifecycle::check_model_change(model, &records) {
            Ok(failing) => {
                withheld.extend(failing.iter().filter(|f| non_core.contains(f)).cloned());
            }
            Err(_) => withheld.extend(non_core.iter().cloned()),
        }
        rows.retain(|s| s.scope == Scope::Core || !withheld.contains(&s.name));
    }

    let viewer = Viewer {
        user: user.to_string(),
        project: String::new(),
    };
    let vis = match visible(&rows, &viewer) {
        Ok(v) => v,
        Err(e) => return v0_fallback(&core, &dest, format!("registry inconsistent: {e}")),
    };
    let frozen = store.frozen_selection(None).await.unwrap_or_default();
    let resident = match select(&vis, &frozen) {
        Ok(r) => r,
        Err(e) => return v0_fallback(&core, &dest, format!("selection failed: {e}")),
    };
    let resident_refs: Vec<&skills::materialiser::RegistrySkill> =
        resident.resident.iter().map(|(_, s)| s).collect();
    let bundles = match store.bundles_for(&resident_refs).await {
        Ok(b) => b,
        Err(e) => return v0_fallback(&core, &dest, format!("registry unavailable: {e}")),
    };
    let report = match skills::mount::write_mount(&dest, &resident, &vis, &bundles) {
        Ok(r) => r,
        Err(e) => return v0_fallback(&core, &dest, format!("mount refused: {e}")),
    };
    if let Err(e) = store
        .record_exposures(&exposure_rows(&resident), session, None)
        .await
    {
        tracing::warn!(error = %e, "could not record skill exposures");
    }
    let mut left_out = withheld;
    left_out.extend(resident.cut.iter().cloned());
    SessionSkills {
        hint: charter_text(&resident_block(&resident), &left_out, None),
        resident: report.resident,
        degraded: None,
        withheld: left_out,
    }
}

/// Add the skills text to a charter. Kept here so both drivers' call sites are one line.
#[must_use]
pub fn with_skills(charter: &str, skills: &SessionSkills) -> String {
    if skills.hint.is_empty() {
        charter.to_string()
    } else {
        format!("{charter}\n\n\u{2500}\u{2500} SKILLS \u{2500}\u{2500}\n\n{}", skills.hint)
    }
}

/// The `skill_notice` rows a live session has not yet been told about (RA-2.6),
/// rendered as one system message each. The system prompt is never edited: the caller
/// appends these to the conversation.
pub async fn pending_notices(pg: &PgPool, after: i64, resident: &[String]) -> (i64, Vec<String>) {
    let store = SkillStore::new(pg.clone());
    let Ok(rows) = store.notices_since(after).await else {
        return (after, Vec::new());
    };
    let mut last = after;
    let mut msgs = Vec::new();
    for (id, name, kind, reason) in rows {
        last = last.max(id);
        if resident.contains(&name) {
            msgs.push(format!(
                "skill_notice: the skill `{name}` was {kind} ({reason}). Do not rely on it; it leaves your mount at the next session start."
            ));
        }
    }
    (last, msgs)
}
