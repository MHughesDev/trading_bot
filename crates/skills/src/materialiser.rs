//! The v1 materialiser's resolution and selection (Set R-a, AGENT-003 §6.2;
//! SK-01, SK-07, SK-09).
//!
//! [`crate::core`] is the v0 materialiser: it mounts every core skill and says it
//! has no scopes. This is the part of v1 that is *pure*: given the registry's
//! visible rows and a viewer, decide which skills a session may see and which of
//! them are resident, and nothing else. Reading the rows from Postgres and writing
//! the read-only mount are adapters around it (ADR-RA-02), because the claim that
//! matters, byte-identical output for two sessions with the same visible set, is
//! only testable if selection has no I/O.
//!
//! # Scopes
//!
//! `core` and `global` are visible to everyone; a `user` skill only to its owner; a
//! `project` skill only to its project. A lower scope may not shadow a core or global
//! name. The registry's unique index refuses that at admission; this refuses it again
//! rather than choosing a winner, because a defensive check that silently picks the
//! higher scope would hide the day the index was dropped.
//!
//! # The resident set
//!
//! At most [`MAX_RESIDENT`] skills: core first, then pinned global, then the project's
//! **frozen selection**, filled in that order. The output is sorted by tier then
//! name, so it does not depend on the order the rows arrived in. Over the cap the
//! lowest tier is cut and *the cut is reported* (ADR-RA-03); core is never silently
//! cut, and a core set that alone exceeds the cap is an error, because there is no
//! honest way to choose which core skill a model does without.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most skills whose descriptions a session carries (SK-07).
pub const MAX_RESIDENT: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Core,
    Global,
    User,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Provisional,
    Admitted,
    Quarantined,
    Deprecated,
    Archived,
}

impl Status {
    /// Only these appear in a glossary view. A quarantined skill disappears from
    /// every glossary at once, which is why visibility is a function of status.
    #[must_use]
    pub const fn visible(self) -> bool {
        matches!(self, Self::Provisional | Self::Admitted)
    }
}

/// One registry row, as the materialiser needs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySkill {
    pub name: String,
    pub scope: Scope,
    pub owner_user: Option<String>,
    pub project: Option<String>,
    pub category: String,
    pub status: Status,
    pub description: String,
    /// The admitted version's content hash.
    pub version: String,
    /// Pinned global skills are resident for everyone.
    pub pinned: bool,
}

/// Who the session is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewer {
    pub user: String,
    pub project: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MaterialiseError {
    #[error("two visible skills are named {name:?} ({a:?} and {b:?}); a lower scope may not shadow a higher one and the registry index should have refused it")]
    NameCollision { name: String, a: Scope, b: Scope },
    #[error("{core} core skills exceed the {MAX_RESIDENT}-skill resident cap; there is no honest way to choose which to drop")]
    CoreExceedsCap { core: usize },
    #[error("a {scope:?} skill {name:?} has no {missing}")]
    MissingOwner {
        name: String,
        scope: Scope,
        missing: &'static str,
    },
}

/// The skills this viewer may see, sorted by name.
///
/// # Errors
/// A name collision, or a user or project skill that names no owner.
pub fn visible<'a>(
    skills: &'a [RegistrySkill],
    viewer: &Viewer,
) -> Result<Vec<&'a RegistrySkill>, MaterialiseError> {
    let mut out: Vec<&RegistrySkill> = Vec::new();
    for s in skills {
        if !s.status.visible() {
            continue;
        }
        let ok = match s.scope {
            Scope::Core | Scope::Global => true,
            Scope::User => match &s.owner_user {
                Some(u) => *u == viewer.user,
                None => {
                    return Err(MaterialiseError::MissingOwner {
                        name: s.name.clone(),
                        scope: s.scope,
                        missing: "owner",
                    })
                }
            },
            Scope::Project => match &s.project {
                Some(p) => *p == viewer.project,
                None => {
                    return Err(MaterialiseError::MissingOwner {
                        name: s.name.clone(),
                        scope: s.scope,
                        missing: "project",
                    })
                }
            },
        };
        if ok {
            out.push(s);
        }
    }
    out.sort_by(|a, b| (&a.name, a.scope).cmp(&(&b.name, b.scope)));
    for w in out.windows(2) {
        if w[0].name == w[1].name {
            return Err(MaterialiseError::NameCollision {
                name: w[0].name.clone(),
                a: w[0].scope,
                b: w[1].scope,
            });
        }
    }
    Ok(out)
}

/// Which tier a skill is resident in. Lower is kept first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Tier {
    Core,
    PinnedGlobal,
    Selected,
}

/// The session's resident set and what was left out of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resident {
    /// Sorted by tier, then name.
    pub resident: Vec<(Tier, RegistrySkill)>,
    /// Frozen-selection or pinned skills dropped because the cap was reached,
    /// in the order they were considered. Never a core skill.
    pub cut: Vec<String>,
    /// Visible and not resident: reachable through the glossary and `skills search`.
    pub on_demand: Vec<String>,
}

/// Choose the resident set.
///
/// `frozen_selection` is the project's selection by name, in rank order (best
/// first), as recorded at the last project-version bump. It only ever *adds* skills
/// beyond core and pinned global, and a name in it that is not visible is ignored.
///
/// # Errors
/// More core skills than the cap.
pub fn select(
    visible: &[&RegistrySkill],
    frozen_selection: &[String],
) -> Result<Resident, MaterialiseError> {
    let core: Vec<&RegistrySkill> = visible
        .iter()
        .copied()
        .filter(|s| s.scope == Scope::Core)
        .collect();
    if core.len() > MAX_RESIDENT {
        return Err(MaterialiseError::CoreExceedsCap { core: core.len() });
    }
    let by_name: BTreeMap<&str, &RegistrySkill> =
        visible.iter().map(|s| (s.name.as_str(), *s)).collect();

    let mut chosen: Vec<(Tier, &RegistrySkill)> = core.iter().map(|s| (Tier::Core, *s)).collect();
    let mut cut: Vec<String> = Vec::new();

    let mut pinned: Vec<&RegistrySkill> = visible
        .iter()
        .copied()
        .filter(|s| s.scope == Scope::Global && s.pinned)
        .collect();
    pinned.sort_by(|a, b| a.name.cmp(&b.name));
    for s in pinned {
        if chosen.len() < MAX_RESIDENT {
            chosen.push((Tier::PinnedGlobal, s));
        } else {
            cut.push(s.name.clone());
        }
    }
    for name in frozen_selection {
        let Some(s) = by_name.get(name.as_str()) else {
            continue;
        };
        if chosen.iter().any(|(_, c)| c.name == s.name) {
            continue;
        }
        if chosen.len() < MAX_RESIDENT {
            chosen.push((Tier::Selected, s));
        } else {
            cut.push(s.name.clone());
        }
    }

    chosen.sort_by(|a, b| (a.0, &a.1.name).cmp(&(b.0, &b.1.name)));
    let resident_names: Vec<&str> = chosen.iter().map(|(_, s)| s.name.as_str()).collect();
    let on_demand: Vec<String> = visible
        .iter()
        .filter(|s| !resident_names.contains(&s.name.as_str()))
        .map(|s| s.name.clone())
        .collect();
    Ok(Resident {
        resident: chosen.into_iter().map(|(t, s)| (t, s.clone())).collect(),
        cut,
        on_demand,
    })
}

/// The bytes a session carries for its resident descriptions: one line per skill,
/// `name<TAB>version<TAB>description`, in resident order, with no time, no user and
/// no counters (SK-07: no dynamic content).
#[must_use]
pub fn resident_block(resident: &Resident) -> String {
    let mut out = String::new();
    for (_, s) in &resident.resident {
        out.push_str(&s.name);
        out.push('\t');
        out.push_str(&s.version);
        out.push('\t');
        out.push_str(&s.description.replace(['\n', '\t'], " "));
        out.push('\n');
    }
    out
}

/// `GLOSSARY.md` for the visible set: every visible skill, resident or not, with
/// scope, status and version (SK-09). Sorted by category then name.
#[must_use]
pub fn glossary_md(visible: &[&RegistrySkill]) -> String {
    let mut sorted: Vec<&&RegistrySkill> = visible.iter().collect();
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
            "- `{}` ({:?}, {:?}, v{}): {}\n",
            s.name,
            s.scope,
            s.status,
            &s.version[..s.version.len().min(8)],
            s.description
        ));
    }
    out
}

/// One `skill_run` row to write with `state = 'exposed'` per resident skill (RA-2.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExposureRow {
    pub skill: String,
    pub version: String,
}

#[must_use]
pub fn exposure_rows(resident: &Resident) -> Vec<ExposureRow> {
    resident
        .resident
        .iter()
        .map(|(_, s)| ExposureRow {
            skill: s.name.clone(),
            version: s.version.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sk(name: &str, scope: Scope) -> RegistrySkill {
        RegistrySkill {
            name: name.into(),
            scope,
            owner_user: (scope == Scope::User).then(|| "u1".to_string()),
            project: (scope == Scope::Project).then(|| "p1".to_string()),
            category: "research".into(),
            status: Status::Admitted,
            description: format!("use when {name}"),
            version: format!("{:0>64}", name.len()),
            pinned: false,
        }
    }

    fn viewer() -> Viewer {
        Viewer {
            user: "u1".into(),
            project: "p1".into(),
        }
    }

    fn all_visible(skills: &[RegistrySkill]) -> Vec<&RegistrySkill> {
        visible(skills, &viewer()).unwrap()
    }

    #[test]
    fn scopes_decide_who_sees_what() {
        let mut other_user = sk("theirs", Scope::User);
        other_user.owner_user = Some("u2".into());
        let mut other_project = sk("elsewhere", Scope::Project);
        other_project.project = Some("p2".into());
        let skills = vec![
            sk("core-a", Scope::Core),
            sk("glob", Scope::Global),
            sk("mine", Scope::User),
            sk("ours", Scope::Project),
            other_user,
            other_project,
        ];
        let names: Vec<&str> = all_visible(&skills)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["core-a", "glob", "mine", "ours"]);
    }

    #[test]
    fn a_quarantined_or_archived_skill_is_invisible_to_everyone() {
        let mut q = sk("bad", Scope::Global);
        q.status = Status::Quarantined;
        let mut a = sk("old", Scope::Global);
        a.status = Status::Archived;
        let mut d = sk("dep", Scope::Global);
        d.status = Status::Deprecated;
        let skills = vec![q, a, d, sk("fine", Scope::Global), {
            let mut p = sk("prov", Scope::Project);
            p.status = Status::Provisional;
            p
        }];
        let names: Vec<&str> = all_visible(&skills)
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["fine", "prov"],
            "provisional is visible, the rest are not"
        );
    }

    #[test]
    fn a_lower_scope_cannot_shadow_a_core_or_global_name() {
        let skills = vec![sk("playbook", Scope::Core), sk("playbook", Scope::Project)];
        assert_eq!(
            visible(&skills, &viewer()),
            Err(MaterialiseError::NameCollision {
                name: "playbook".into(),
                a: Scope::Core,
                b: Scope::Project
            })
        );
        // Another user's skill of the same name is not visible, so it is not a collision.
        let mut theirs = sk("playbook", Scope::User);
        theirs.owner_user = Some("u2".into());
        assert!(visible(&[sk("playbook", Scope::Core), theirs], &viewer()).is_ok());
    }

    #[test]
    fn a_user_skill_with_no_owner_is_an_error_not_a_public_skill() {
        let mut s = sk("x", Scope::User);
        s.owner_user = None;
        assert!(matches!(
            visible(&[s], &viewer()),
            Err(MaterialiseError::MissingOwner {
                missing: "owner",
                ..
            })
        ));
    }

    fn fleet(core: usize, pinned: usize, project: usize) -> Vec<RegistrySkill> {
        let mut v = Vec::new();
        for i in 0..core {
            v.push(sk(&format!("c{i:02}"), Scope::Core));
        }
        for i in 0..pinned {
            let mut g = sk(&format!("g{i:02}"), Scope::Global);
            g.pinned = true;
            v.push(g);
        }
        for i in 0..project {
            v.push(sk(&format!("p{i:02}"), Scope::Project));
        }
        v
    }

    #[test]
    fn the_resident_set_is_capped_at_forty_and_reports_the_cut() {
        let skills = fleet(5, 10, 40); // 55 candidates
        let vis = all_visible(&skills);
        let frozen: Vec<String> = (0..40).map(|i| format!("p{i:02}")).collect();
        let r = select(&vis, &frozen).unwrap();
        assert_eq!(r.resident.len(), MAX_RESIDENT);
        // 5 core + 10 pinned + 25 of the frozen selection; the last 15 are cut.
        assert_eq!(r.cut.len(), 15);
        assert_eq!(
            r.cut[0], "p25",
            "cut in rank order: the worst-ranked go first"
        );
        assert_eq!(r.cut.last().map(String::as_str), Some("p39"));
        // Every core and pinned skill is resident.
        for i in 0..5 {
            assert!(r
                .resident
                .iter()
                .any(|(t, s)| *t == Tier::Core && s.name == format!("c{i:02}")));
        }
        // Visible, not resident: reachable on demand.
        assert_eq!(r.on_demand.len(), 15);
    }

    #[test]
    fn the_output_does_not_depend_on_the_order_rows_arrive_in() {
        let skills = fleet(4, 3, 6);
        let frozen: Vec<String> = vec!["p03".into(), "p01".into(), "p05".into()];
        let a = {
            let v = all_visible(&skills);
            resident_block(&select(&v, &frozen).unwrap())
        };
        let mut shuffled = skills.clone();
        shuffled.reverse();
        shuffled.swap(2, 7);
        let b = {
            let v = all_visible(&shuffled);
            resident_block(&select(&v, &frozen).unwrap())
        };
        assert_eq!(a, b, "same set, same bytes");
        // Sorted by tier then name, not by selection rank.
        let names: Vec<&str> = a.lines().map(|l| l.split('\t').next().unwrap()).collect();
        let expected = [
            "c00", "c01", "c02", "c03", "g00", "g01", "g02", "p01", "p03", "p05",
        ];
        assert_eq!(names, expected);
    }

    #[test]
    fn two_users_with_the_same_visible_set_get_identical_resident_bytes() {
        let skills = fleet(3, 2, 0);
        let v1 = visible(
            &skills,
            &Viewer {
                user: "alice".into(),
                project: "x".into(),
            },
        )
        .unwrap();
        let v2 = visible(
            &skills,
            &Viewer {
                user: "bob".into(),
                project: "y".into(),
            },
        )
        .unwrap();
        assert_eq!(
            resident_block(&select(&v1, &[]).unwrap()),
            resident_block(&select(&v2, &[]).unwrap())
        );
        assert_eq!(glossary_md(&v1), glossary_md(&v2));
    }

    #[test]
    fn core_is_never_cut_and_a_core_set_over_the_cap_is_an_error() {
        let skills = fleet(MAX_RESIDENT, 3, 0);
        let v = all_visible(&skills);
        let r = select(&v, &[]).unwrap();
        assert_eq!(r.resident.len(), MAX_RESIDENT);
        assert_eq!(
            r.cut,
            vec!["g00", "g01", "g02"],
            "the pinned globals are what give way"
        );
        let too_many = fleet(MAX_RESIDENT + 1, 0, 0);
        let v = all_visible(&too_many);
        assert_eq!(
            select(&v, &[]),
            Err(MaterialiseError::CoreExceedsCap {
                core: MAX_RESIDENT + 1
            })
        );
    }

    #[test]
    fn a_selection_naming_an_invisible_or_duplicate_skill_is_ignored() {
        let skills = fleet(1, 0, 2);
        let v = all_visible(&skills);
        let r = select(
            &v,
            &["nope".into(), "p00".into(), "p00".into(), "c00".into()],
        )
        .unwrap();
        let names: Vec<&str> = r.resident.iter().map(|(_, s)| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["c00", "p00"],
            "no duplicate, and core was already in"
        );
        assert_eq!(r.on_demand, vec!["p01"]);
    }

    #[test]
    fn the_glossary_lists_every_visible_skill_resident_or_not() {
        let skills = fleet(2, 0, 3);
        let v = all_visible(&skills);
        let r = select(&v, &[]).unwrap();
        assert_eq!(r.resident.len(), 2);
        let g = glossary_md(&v);
        for s in &skills {
            assert!(
                g.contains(&format!("`{}`", s.name)),
                "glossary omits {}",
                s.name
            );
        }
        assert!(g.contains("Project") && g.contains("Core"));
    }

    #[test]
    fn exposure_rows_are_one_per_resident_skill_and_none_for_the_rest() {
        let skills = fleet(2, 0, 3);
        let v = all_visible(&skills);
        let r = select(&v, &["p01".into()]).unwrap();
        let rows = exposure_rows(&r);
        let names: Vec<&str> = rows.iter().map(|e| e.skill.as_str()).collect();
        assert_eq!(names, vec!["c00", "c01", "p01"]);
        assert!(rows.iter().all(|e| e.version.len() == 64));
    }

    #[test]
    fn a_description_cannot_inject_a_line_into_the_resident_block() {
        let mut s = sk("a", Scope::Core);
        s.description = "line one\nfake\tfield".into();
        let skills = vec![s];
        let v = all_visible(&skills);
        let block = resident_block(&select(&v, &[]).unwrap());
        assert_eq!(block.lines().count(), 1);
        assert_eq!(block.matches('\t').count(), 2);
    }
}
