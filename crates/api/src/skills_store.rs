//! The skill registry's Postgres adapter (Set R-a RA-1.2, RA-1.5, RA-2.1, RA-2.4,
//! RA-3.1, RA-3.3, RA-3.8, RA-4.5; Set R-b RB-2.1, RB-2.3, RB-2.6; ADR-0028).
//!
//! Every rule that *decides* is a pure function in the `skills` crate
//! ([`skills::registry`], [`skills::lifecycle`], [`skills::materialiser`]). This file
//! reads rows into them and writes their answers back. The only way a skill becomes
//! visible is [`SkillStore::admit`], which calls `skill_registry.admit()` and stores
//! the bundle in the same transaction.
//!
//! # `skills.admit` is not an agent scope
//!
//! Nothing here checks a token; the routes in `routes/skills.rs` do, and the agent's
//! scopes (`auth::scopes::RESEARCH_SCOPES`) do not contain `skills.admit`
//! (`crates/invariants/tests/skills_admit_is_never_an_agent_scope.rs`).

use std::collections::BTreeMap;

use serde_json::{json, Value};
use skills::lifecycle::{Condition, Pair, PromotionFile, Recorded};
use skills::materialiser::{RegistrySkill, Scope, Status};
use skills::registry::Draft;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("{0}")]
    Refused(String),
    #[error("not found: {0}")]
    NotFound(String),
}

fn scope_str(s: Scope) -> &'static str {
    match s {
        Scope::Core => "core",
        Scope::Global => "global",
        Scope::User => "user",
        Scope::Project => "project",
    }
}

fn parse_scope(s: &str) -> Option<Scope> {
    Some(match s {
        "core" => Scope::Core,
        "global" => Scope::Global,
        "user" => Scope::User,
        "project" => Scope::Project,
        _ => return None,
    })
}

fn parse_status(s: &str) -> Option<Status> {
    Some(match s {
        "provisional" => Status::Provisional,
        "admitted" => Status::Admitted,
        "quarantined" => Status::Quarantined,
        "deprecated" => Status::Deprecated,
        "archived" => Status::Archived,
        _ => return None,
    })
}

/// A bundle as the registry stores it: path to text.
pub fn files_to_json(files: &BTreeMap<String, Vec<u8>>) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in files {
        m.insert(
            k.clone(),
            Value::String(String::from_utf8_lossy(v).into_owned()),
        );
    }
    Value::Object(m)
}

pub fn files_from_json(v: &Value) -> BTreeMap<String, Vec<u8>> {
    v.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.as_bytes().to_vec())))
                .collect()
        })
        .unwrap_or_default()
}

/// The `skills.patch_threshold` setting: the environment variable
/// `SKILLS_PATCH_THRESHOLD`, else the spec default.
#[must_use]
pub fn patch_threshold() -> f64 {
    std::env::var("SKILLS_PATCH_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(skills::registry::DEFAULT_PATCH_THRESHOLD)
}

/// `skills.admit_after_exposures` (AGENT-003 open question 3, ADR-RA-04).
#[must_use]
pub fn admit_after_exposures() -> u32 {
    std::env::var("SKILLS_ADMIT_AFTER_EXPOSURES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(skills::registry::DEFAULT_ADMIT_AFTER_EXPOSURES)
}

/// The tool catalogue a procedure skill may name: every declared tool, and whether it
/// is one that reaches the network.
#[must_use]
pub fn tool_catalogue() -> skills::registry::Catalogue {
    skills::registry::Catalogue::new(mcp_server_lib::spec::catalog().iter().map(|s| {
        (
            s.name.to_string(),
            matches!(s.risk, mcp_server_lib::taxonomy::Risk::Outbound),
        )
    }))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub admitted: usize,
    pub patched: usize,
    pub unchanged: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub exposures: i64,
    pub activations: i64,
    pub balance: i64,
}

#[derive(Clone)]
pub struct SkillStore {
    pg: PgPool,
}

impl SkillStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    // ── admission ───────────────────────────────────────────────────────────

    /// The one path to a visible skill: `skill_registry.admit()` and the bundle, in
    /// one transaction. `parent` makes it a patch of an existing skill.
    ///
    /// # Errors
    /// The registry's CHECKs and triggers (a shadowed name, a duplicate version).
    pub async fn admit(
        &self,
        draft: &Draft,
        status: Status,
        owner: Option<Uuid>,
        project: Option<Uuid>,
        pinned: bool,
        parent: Option<&str>,
        created_by: &str,
    ) -> Result<Uuid, StoreError> {
        let mut tx = self.pg.begin().await?;
        let id: Uuid = sqlx::query(
            "SELECT skill_registry.admit($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(&draft.name)
        .bind(scope_str(draft.scope))
        .bind(owner)
        .bind(project)
        .bind(&draft.category)
        .bind(match status {
            Status::Provisional => "provisional",
            _ => "admitted",
        })
        .bind(pinned)
        .bind(&draft.version_hash)
        .bind(parent)
        .bind(&draft.description)
        .bind(draft.description_tokens)
        .bind(draft.body_lines)
        .bind(&draft.version_hash)
        .bind(created_by)
        .fetch_one(&mut *tx)
        .await?
        .get(0);
        sqlx::query(
            "INSERT INTO skill_registry.skill_bundle (version_hash, files) VALUES ($1, $2)
             ON CONFLICT (version_hash) DO NOTHING",
        )
        .bind(&draft.version_hash)
        .bind(files_to_json(&draft.files))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Load the release's core skills into the registry (RA-1.5). Idempotent on
    /// content hash: the same bytes are a no-op, changed bytes are a new version
    /// whose parent is the old one, and the old one stays readable.
    ///
    /// # Errors
    /// A core skill that fails admission, or the database.
    pub async fn sync_core(
        &self,
        core: &[skills::core::CoreSkill],
    ) -> Result<SyncReport, StoreError> {
        let catalogue = tool_catalogue();
        let mut report = SyncReport::default();
        for c in core {
            let draft = Draft::check(
                Scope::Core,
                &c.category,
                &c.name,
                c.files.clone(),
                &catalogue,
            )
            .map_err(|e| StoreError::Refused(format!("core skill {}: {e}", c.name)))?;
            let current: Option<String> = sqlx::query(
                "SELECT current_version FROM skill_registry.skill WHERE name = $1 AND scope = 'core'",
            )
            .bind(&c.name)
            .fetch_optional(&self.pg)
            .await?
            .and_then(|r| r.get::<Option<String>, _>(0));
            match current {
                Some(v) if v == draft.version_hash => report.unchanged += 1,
                Some(v) => {
                    // The registry's current version is not this release's. Either this
                    // release is newer (a new version on top of the current one) or the
                    // registry has moved past it, with an admitted delta (KM-05) sitting
                    // above a version this release already shipped: then the release
                    // must not overwrite it.
                    let shipped: bool = sqlx::query(
                        "SELECT EXISTS (SELECT 1 FROM skill_registry.skill_version WHERE version_hash = $1)",
                    )
                    .bind(&draft.version_hash)
                    .fetch_one(&self.pg)
                    .await?
                    .get(0);
                    if shipped {
                        report.unchanged += 1;
                    } else {
                        self.admit(
                            &draft,
                            Status::Admitted,
                            None,
                            None,
                            false,
                            Some(&v),
                            "release",
                        )
                        .await?;
                        report.patched += 1;
                    }
                }
                None => {
                    self.admit(&draft, Status::Admitted, None, None, false, None, "release")
                        .await?;
                    report.admitted += 1;
                }
            }
        }
        Ok(report)
    }

    // ── reads ───────────────────────────────────────────────────────────────

    /// The glossary rows this viewer may see (`skill_glossary` is the only source).
    ///
    /// # Errors
    /// The database.
    pub async fn visible(
        &self,
        user: Uuid,
        project: Option<Uuid>,
    ) -> Result<Vec<RegistrySkill>, StoreError> {
        let rows = sqlx::query(
            "SELECT name, scope, owner_user_id::text, project_id::text, category, status, description,
                    current_version, pinned
               FROM skill_registry.skill_glossary
              WHERE scope IN ('core', 'global')
                 OR (scope = 'user' AND owner_user_id = $1)
                 OR (scope = 'project' AND project_id = $2)
              ORDER BY name",
        )
        .bind(user)
        .bind(project)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(RegistrySkill {
                    name: r.get(0),
                    scope: parse_scope(r.get::<String, _>(1).as_str())?,
                    owner_user: r.get(2),
                    project: r.get(3),
                    category: r.get(4),
                    status: parse_status(r.get::<String, _>(5).as_str())?,
                    description: r.get(6),
                    version: r.get(7),
                    pinned: r.get(8),
                })
            })
            .collect())
    }

    /// The project's selection as of its latest version, best first.
    ///
    /// # Errors
    /// The database.
    pub async fn frozen_selection(&self, project: Option<Uuid>) -> Result<Vec<String>, StoreError> {
        let Some(project) = project else {
            return Ok(Vec::new());
        };
        Ok(sqlx::query(
            "SELECT skill_name FROM skill_registry.project_skill_selection
              WHERE project_id = $1
                AND project_version = (SELECT max(project_version)
                                         FROM skill_registry.project_skill_selection WHERE project_id = $1)
              ORDER BY rank",
        )
        .bind(project)
        .fetch_all(&self.pg)
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect())
    }

    /// Freeze a new project version whose selection is the old one plus `name`.
    ///
    /// # Errors
    /// The database.
    pub async fn freeze_with(&self, project: Uuid, name: &str) -> Result<i32, StoreError> {
        let mut names = self.frozen_selection(Some(project)).await?;
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        let next: i32 = sqlx::query(
            "SELECT COALESCE(max(project_version), -1) + 1 FROM skill_registry.project_skill_selection WHERE project_id = $1",
        )
        .bind(project)
        .fetch_one(&self.pg)
        .await?
        .get(0);
        let mut tx = self.pg.begin().await?;
        for (rank, n) in names.iter().enumerate() {
            sqlx::query(
                "INSERT INTO skill_registry.project_skill_selection (project_id, project_version, skill_name, rank)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(project)
            .bind(next)
            .bind(n)
            .bind(i32::try_from(rank).unwrap_or(i32::MAX))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(next)
    }

    /// The bundle files of one version.
    ///
    /// # Errors
    /// The database.
    pub async fn bundle(
        &self,
        version: &str,
    ) -> Result<Option<BTreeMap<String, Vec<u8>>>, StoreError> {
        Ok(
            sqlx::query("SELECT files FROM skill_registry.skill_bundle WHERE version_hash = $1")
                .bind(version)
                .fetch_optional(&self.pg)
                .await?
                .map(|r| files_from_json(&r.get::<Value, _>(0))),
        )
    }

    /// Bundles for a set of registry rows, keyed by skill name.
    ///
    /// # Errors
    /// The database.
    pub async fn bundles_for(
        &self,
        rows: &[&RegistrySkill],
    ) -> Result<skills::mount::Bundles, StoreError> {
        let mut out = skills::mount::Bundles::new();
        for s in rows {
            if let Some(f) = self.bundle(&s.version).await? {
                out.insert(s.name.clone(), f);
            }
        }
        Ok(out)
    }

    // ── runs, exposure, votes (SK-13) ───────────────────────────────────────

    /// One `exposed` row per resident skill (RA-2.4): exactly the resident set, none
    /// for a skill that is only in the glossary.
    ///
    /// # Errors
    /// The database.
    pub async fn record_exposures(
        &self,
        rows: &[skills::materialiser::ExposureRow],
        session: Uuid,
        project: Option<Uuid>,
    ) -> Result<usize, StoreError> {
        let mut n = 0;
        for r in rows {
            let done = sqlx::query(
                "INSERT INTO skill_registry.skill_run (skill_id, version_hash, project_id, session_id, state)
                 SELECT s.skill_id, $2, $3, $4, 'exposed'
                   FROM skill_registry.skill s
                  WHERE s.current_version = $2 AND s.name = $1
                    AND NOT EXISTS (SELECT 1 FROM skill_registry.skill_run x
                                     WHERE x.session_id = $4 AND x.skill_id = s.skill_id AND x.state = 'exposed')",
            )
            .bind(&r.skill)
            .bind(&r.version)
            .bind(project)
            .bind(session)
            .execute(&self.pg)
            .await?;
            n += usize::try_from(done.rows_affected()).unwrap_or(0);
        }
        Ok(n)
    }

    /// The model read a resident skill: `activated`, once per session and skill.
    ///
    /// # Errors
    /// The database.
    pub async fn record_activation(
        &self,
        session: Uuid,
        skill_name: &str,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "INSERT INTO skill_registry.skill_run (skill_id, version_hash, project_id, session_id, state)
             SELECT x.skill_id, x.version_hash, x.project_id, x.session_id, 'activated'
               FROM skill_registry.skill_run x
               JOIN skill_registry.skill s ON s.skill_id = x.skill_id
              WHERE x.session_id = $1 AND s.name = $2 AND x.state = 'exposed'
                AND NOT EXISTS (SELECT 1 FROM skill_registry.skill_run y
                                 WHERE y.session_id = $1 AND y.skill_id = x.skill_id AND y.state = 'activated')",
        )
        .bind(session)
        .bind(skill_name)
        .execute(&self.pg)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// A helpfulness vote on a skill the session was exposed to. A vote on a skill
    /// the session never saw is refused: it would be a number about nothing.
    ///
    /// # Errors
    /// [`StoreError::Refused`] with no exposure; the database.
    pub async fn vote(&self, skill: Uuid, session: Uuid, vote: i16) -> Result<(), StoreError> {
        if !(-1..=1).contains(&vote) {
            return Err(StoreError::Refused("a vote is -1, 0 or 1".into()));
        }
        let done = sqlx::query(
            "UPDATE skill_registry.skill_run SET vote = $3
              WHERE skill_id = $1 AND session_id = $2 AND state = 'exposed'",
        )
        .bind(skill)
        .bind(session)
        .bind(vote)
        .execute(&self.pg)
        .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::Refused(
                "this session was never exposed to that skill, so it has nothing to vote on".into(),
            ));
        }
        Ok(())
    }

    /// Usage statistics. They live here and nowhere else (SK-13).
    ///
    /// # Errors
    /// The database.
    pub async fn stats(&self, skill: Uuid) -> Result<Stats, StoreError> {
        let r = sqlx::query(
            "SELECT count(*) FILTER (WHERE state = 'exposed'),
                    count(*) FILTER (WHERE state = 'activated'),
                    COALESCE(sum(vote), 0)
               FROM skill_registry.skill_run WHERE skill_id = $1",
        )
        .bind(skill)
        .fetch_one(&self.pg)
        .await?;
        Ok(Stats {
            exposures: r.get(0),
            activations: r.get(1),
            balance: r.get::<i64, _>(2),
        })
    }

    /// A run's skill bill of materials (RA-3.8): the skills and versions it had
    /// exposed and activated, read from `skill_run` and nowhere else.
    ///
    /// # Errors
    /// The database.
    pub async fn bill_of_materials(&self, session: Uuid) -> Result<Vec<Value>, StoreError> {
        Ok(sqlx::query(
            "SELECT s.name, r.version_hash,
                    bool_or(r.state = 'activated') AS activated
               FROM skill_registry.skill_run r
               JOIN skill_registry.skill s ON s.skill_id = r.skill_id
              WHERE r.session_id = $1
              GROUP BY s.name, r.version_hash
              ORDER BY s.name",
        )
        .bind(session)
        .fetch_all(&self.pg)
        .await?
        .iter()
        .map(|r| {
            json!({
                "name": r.get::<String, _>(0),
                "version": r.get::<String, _>(1),
                "activated": r.get::<bool, _>(2),
            })
        })
        .collect())
    }

    /// Provisional skills whose exposures and balance say they are ready (RA-4.5),
    /// moved to admitted. Returns `(promoted, flagged)` names.
    ///
    /// # Errors
    /// The database.
    pub async fn settle_provisional(
        &self,
        n: u32,
    ) -> Result<(Vec<String>, Vec<String>), StoreError> {
        let rows = sqlx::query(
            "SELECT skill_id, name FROM skill_registry.skill WHERE status = 'provisional'",
        )
        .fetch_all(&self.pg)
        .await?;
        let (mut promoted, mut flagged) = (Vec::new(), Vec::new());
        for r in rows {
            let (id, name): (Uuid, String) = (r.get(0), r.get(1));
            let s = self.stats(id).await?;
            match skills::lifecycle::provisional_verdict(
                u32::try_from(s.exposures).unwrap_or(u32::MAX),
                s.balance,
                n,
            ) {
                skills::lifecycle::ProvisionalVerdict::Promote => {
                    sqlx::query("SELECT skill_registry.set_admitted($1)")
                        .bind(id)
                        .execute(&self.pg)
                        .await?;
                    promoted.push(name);
                }
                skills::lifecycle::ProvisionalVerdict::Flagged { .. } => flagged.push(name),
                skills::lifecycle::ProvisionalVerdict::Stays { .. } => {}
            }
        }
        Ok((promoted, flagged))
    }

    // ── standing changes ────────────────────────────────────────────────────

    /// Quarantine, deprecate or archive: out of every glossary in the statement that
    /// changes the status, with the notice written in the same one (SK-14).
    ///
    /// # Errors
    /// The database.
    pub async fn retire(&self, skill: Uuid, status: &str, reason: &str) -> Result<(), StoreError> {
        if reason.trim().is_empty() {
            return Err(StoreError::Refused("a reason is required".into()));
        }
        sqlx::query("SELECT skill_registry.retire($1, $2, $3)")
            .bind(skill)
            .bind(status)
            .bind(reason)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    /// Point a deprecated skill at its replacement (RB-2.6).
    ///
    /// # Errors
    /// The database refuses an alias on a skill that is not deprecated.
    pub async fn set_alias(&self, skill: Uuid, target: Uuid) -> Result<(), StoreError> {
        sqlx::query("SELECT skill_registry.set_alias($1, $2)")
            .bind(skill)
            .bind(target)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    /// The live skill a name or id resolves to, following `alias_of` through deprecated
    /// skills: `Some((skill_id, name, current_version))`, `None` when the chain ends
    /// in nothing visible.
    ///
    /// # Errors
    /// The database.
    pub async fn resolve_alias(
        &self,
        skill: Uuid,
    ) -> Result<Option<(Uuid, String, String)>, StoreError> {
        Ok(sqlx::query(
            "SELECT s.skill_id, s.name, s.current_version FROM skill_registry.skill s
              WHERE s.skill_id = skill_registry.resolve_alias($1)",
        )
        .bind(skill)
        .fetch_optional(&self.pg)
        .await?
        .map(|r| (r.get(0), r.get(1), r.get(2))))
    }

    /// Notices since `after`, for a live session to append as system messages.
    ///
    /// # Errors
    /// The database.
    pub async fn notices_since(
        &self,
        after: i64,
    ) -> Result<Vec<(i64, String, String, String)>, StoreError> {
        Ok(sqlx::query(
            "SELECT notice_id, skill_name, kind, reason FROM skill_registry.skill_notice
              WHERE notice_id > $1 ORDER BY notice_id",
        )
        .bind(after)
        .fetch_all(&self.pg)
        .await?
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
        .collect())
    }

    /// The latest notice id, so a session can start counting from "now".
    ///
    /// # Errors
    /// The database.
    pub async fn latest_notice(&self) -> Result<i64, StoreError> {
        Ok(
            sqlx::query("SELECT COALESCE(max(notice_id), 0) FROM skill_registry.skill_notice")
                .fetch_one(&self.pg)
                .await?
                .get(0),
        )
    }

    // ── promotion (RB-2.1) ──────────────────────────────────────────────────

    /// Record one of the six conditions. Append-only: the latest row of a condition
    /// is the one promotion reads.
    ///
    /// # Errors
    /// The database.
    pub async fn record_condition(
        &self,
        skill: Uuid,
        condition: Condition,
        passed: bool,
        by: &str,
        evidence: &Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO skill_registry.skill_promotion_condition (skill_id, condition, passed, recorded_by, evidence)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(skill)
        .bind(condition.as_str())
        .bind(passed)
        .bind(by)
        .bind(evidence)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Cite a finding in a skill's provenance. The `tainted` mark is **read from the
    /// memory store**, not supplied by the caller: whoever asks for a promotion cannot
    /// decide that a finding from a fetched page is clean (KM-10, SK-11).
    ///
    /// # Errors
    /// [`StoreError::Refused`] when the finding does not exist for this user; the database.
    pub async fn cite_finding(
        &self,
        skill: Uuid,
        user: Uuid,
        finding: &str,
    ) -> Result<bool, StoreError> {
        let tainted: bool = sqlx::query(
            "SELECT COALESCE((extra ->> 'tainted')::boolean, false)
               FROM agent_memory.entry WHERE entry_id = $1 AND user_id = $2",
        )
        .bind(finding)
        .bind(user)
        .fetch_optional(&self.pg)
        .await?
        .map(|r| r.get(0))
        .ok_or_else(|| StoreError::Refused(format!("no finding {finding:?} to cite")))?;
        sqlx::query(
            "INSERT INTO skill_registry.skill_provenance (skill_id, finding_id, tainted) VALUES ($1, $2, $3)
             ON CONFLICT (skill_id, finding_id) DO UPDATE SET tainted = skill_registry.skill_provenance.tainted OR EXCLUDED.tainted",
        )
        .bind(skill)
        .bind(finding)
        .bind(tainted)
        .execute(&self.pg)
        .await?;
        Ok(tainted)
    }

    /// Everything promotion needs, read from the rows.
    ///
    /// # Errors
    /// The database.
    pub async fn promotion_file(&self, skill: Uuid) -> Result<PromotionFile, StoreError> {
        let author: String = sqlx::query(
            "SELECT v.created_by FROM skill_registry.skill_version v
               JOIN skill_registry.skill s ON s.skill_id = v.skill_id
              WHERE s.skill_id = $1 ORDER BY v.created_at ASC LIMIT 1",
        )
        .bind(skill)
        .fetch_optional(&self.pg)
        .await?
        .map(|r| r.get(0))
        .ok_or_else(|| StoreError::NotFound(skill.to_string()))?;
        let mut file = PromotionFile {
            author,
            ..Default::default()
        };
        let rows = sqlx::query(
            "SELECT DISTINCT ON (condition) condition, passed, recorded_by
               FROM skill_registry.skill_promotion_condition
              WHERE skill_id = $1 ORDER BY condition, record_id DESC",
        )
        .bind(skill)
        .fetch_all(&self.pg)
        .await?;
        for r in rows {
            if let Some(c) = Condition::parse(r.get::<String, _>(0).as_str()) {
                file.recorded.insert(
                    c,
                    Recorded {
                        passed: r.get(1),
                        by: r.get(2),
                    },
                );
            }
        }
        file.provenance_tainted = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM skill_registry.skill_provenance WHERE skill_id = $1 AND tainted)",
        )
        .bind(skill)
        .fetch_one(&self.pg)
        .await?
        .get(0);
        Ok(file)
    }

    /// Move a skill to a wider scope, logging who approved it.
    ///
    /// # Errors
    /// The registry's shadowing trigger, or the database.
    pub async fn move_scope(
        &self,
        skill: Uuid,
        to: &str,
        owner: Uuid,
        approved_by: Uuid,
    ) -> Result<(), StoreError> {
        let from: String =
            sqlx::query("SELECT scope FROM skill_registry.skill WHERE skill_id = $1")
                .bind(skill)
                .fetch_optional(&self.pg)
                .await?
                .map(|r| r.get(0))
                .ok_or_else(|| StoreError::NotFound(skill.to_string()))?;
        let mut tx = self.pg.begin().await?;
        sqlx::query("SELECT skill_registry.move_scope($1, $2, $3)")
            .bind(skill)
            .bind(to)
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO skill_registry.skill_promotion_log (skill_id, from_scope, to_scope, approved_by)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(skill)
        .bind(&from)
        .bind(to)
        .bind(approved_by)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    // ── evaluation pairs (RA-3.4, RB-2.4) ───────────────────────────────────

    /// Record one paired outcome for a proposal or an admitted skill.
    ///
    /// # Errors
    /// The database (the CHECK refuses a pair with no subject or two).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_pair(
        &self,
        proposal: Option<Uuid>,
        skill: Option<Uuid>,
        kind: &str,
        model: &str,
        variant: &str,
        pair: &Pair,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO skill_registry.skill_eval_pair
               (proposal_id, skill_id, version_hash, kind, model, variant, activated,
                with_success, without_success, with_cost, without_cost)
             VALUES ($1, $2, (SELECT current_version FROM skill_registry.skill WHERE skill_id = $2),
                     $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(proposal)
        .bind(skill)
        .bind(kind)
        .bind(model)
        .bind(variant)
        .bind(pair.activated)
        .bind(pair.with_success)
        .bind(pair.without_success)
        .bind(pair.with_cost)
        .bind(pair.without_cost)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// The pairs of a proposal or skill of one kind, in arrival order, and how many
    /// distinct variants they cover.
    ///
    /// # Errors
    /// The database.
    pub async fn pairs(
        &self,
        proposal: Option<Uuid>,
        skill: Option<Uuid>,
        kind: &str,
    ) -> Result<(Vec<Pair>, usize), StoreError> {
        let rows = sqlx::query(
            "SELECT activated, with_success, without_success, with_cost, without_cost, variant
               FROM skill_registry.skill_eval_pair
              WHERE proposal_id IS NOT DISTINCT FROM $1 AND skill_id IS NOT DISTINCT FROM $2 AND kind = $3
              ORDER BY pair_id",
        )
        .bind(proposal)
        .bind(skill)
        .bind(kind)
        .fetch_all(&self.pg)
        .await?;
        let variants: std::collections::BTreeSet<String> = rows.iter().map(|r| r.get(5)).collect();
        Ok((
            rows.iter()
                .map(|r| Pair {
                    activated: r.get(0),
                    with_success: r.get(1),
                    without_success: r.get(2),
                    with_cost: r.get(3),
                    without_cost: r.get(4),
                })
                .collect(),
            variants.len(),
        ))
    }

    // ── retirement and re-evaluation (RB-2.5, RB-2.6) ───────────────────────

    /// Leave-one-out contribution: how much better the with-skill arm did than the
    /// arm with this skill removed. `None` with no leave-one-out pairs.
    ///
    /// # Errors
    /// The database.
    pub async fn loo_contribution(&self, skill: Uuid) -> Result<Option<f64>, StoreError> {
        Ok(sqlx::query(
            "SELECT avg(with_success::int)::float8 - avg(without_success::int)::float8
               FROM skill_registry.skill_eval_pair WHERE skill_id = $1 AND kind = 'leave_one_out'",
        )
        .bind(skill)
        .fetch_one(&self.pg)
        .await?
        .get::<Option<f64>, _>(0))
    }

    /// Archive every non-core skill that has had `n` exposures and either was never
    /// activated or contributes nothing leave-one-out (RB-2.6). Core is release-managed
    /// and is never archived here. Returns `(name, why)`.
    ///
    /// # Errors
    /// The database.
    pub async fn sweep_retirements(
        &self,
        n: u32,
    ) -> Result<Vec<(String, &'static str)>, StoreError> {
        let rows = sqlx::query(
            "SELECT skill_id, name FROM skill_registry.skill
              WHERE status IN ('provisional', 'admitted') AND scope <> 'core'",
        )
        .fetch_all(&self.pg)
        .await?;
        let mut out = Vec::new();
        for r in rows {
            let (id, name): (Uuid, String) = (r.get(0), r.get(1));
            let st = self.stats(id).await?;
            let loo = self.loo_contribution(id).await?;
            if let skills::lifecycle::Retirement::Archive(why) = skills::lifecycle::retire_decision(
                u32::try_from(st.exposures).unwrap_or(u32::MAX),
                u32::try_from(st.activations).unwrap_or(u32::MAX),
                loo,
                n,
            ) {
                self.retire(id, "archived", why).await?;
                out.push((name, why));
            }
        }
        Ok(out)
    }

    /// The library re-evaluation records for a model, oldest first.
    ///
    /// # Errors
    /// The database.
    pub async fn reevaluations(
        &self,
        model: &str,
    ) -> Result<Vec<skills::lifecycle::ReEvaluation>, StoreError> {
        Ok(sqlx::query(
            "SELECT model, completed, failing FROM skill_registry.skill_library_reeval
              WHERE model = $1 ORDER BY reeval_id",
        )
        .bind(model)
        .fetch_all(&self.pg)
        .await?
        .iter()
        .map(|r| skills::lifecycle::ReEvaluation {
            model: r.get(0),
            completed: r.get(1),
            failing: r
                .get::<Value, _>(2)
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
    }

    /// Record a completed (or abandoned) run of the whole library on a model.
    ///
    /// # Errors
    /// The database.
    pub async fn record_reevaluation(
        &self,
        model: &str,
        completed: bool,
        failing: &[String],
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO skill_registry.skill_library_reeval (model, completed, failing) VALUES ($1, $2, $3)")
            .bind(model)
            .bind(completed)
            .bind(json!(failing))
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    // ── the curator (RB-1.8) ────────────────────────────────────────────────

    /// What the curator is shown: registry statistics, look-alike pairs and family
    /// pass counts, all numbers and identifiers. Names are normalised to the
    /// identifier alphabet here, so a name that is not already one cannot reach the
    /// curator as prose.
    ///
    /// # Errors
    /// The database.
    pub async fn curator_input(
        &self,
        tenant: &str,
    ) -> Result<harness::memory::curator::CuratorInput, StoreError> {
        use harness::memory::curator::{CuratorInput, FamilyRow, SimilarPair, SkillRow};
        fn ident(s: &str) -> String {
            let t: String = s
                .to_ascii_lowercase()
                .chars()
                .map(|c| {
                    if c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || matches!(c, '_' | '-' | '.' | ':')
                    {
                        c
                    } else {
                        '_'
                    }
                })
                .take(64)
                .collect();
            if t.is_empty() {
                "unnamed".to_string()
            } else {
                t
            }
        }
        let rows = sqlx::query(
            "SELECT skill_id, name, current_version FROM skill_registry.skill
              WHERE status IN ('provisional', 'admitted') AND scope <> 'core' ORDER BY name",
        )
        .fetch_all(&self.pg)
        .await?;
        let mut skills = Vec::new();
        let mut bodies: Vec<(String, String)> = Vec::new();
        for r in &rows {
            let (id, name, ver): (Uuid, String, Option<String>) = (r.get(0), r.get(1), r.get(2));
            let st = self.stats(id).await?;
            skills.push(SkillRow {
                name: ident(&name),
                exposures: u32::try_from(st.exposures).unwrap_or(u32::MAX),
                activations: u32::try_from(st.activations).unwrap_or(u32::MAX),
                balance: st.balance,
                loo_contribution: self.loo_contribution(id).await?,
            });
            if let Some(v) = ver {
                if let Some(files) = self.bundle(&v).await? {
                    bodies.push((ident(&name), skills::registry::body_of(&files)));
                }
            }
        }
        let mut pairs = Vec::new();
        for i in 0..bodies.len() {
            for j in (i + 1)..bodies.len() {
                let sim = skills::registry::similarity(&bodies[i].1, &bodies[j].1);
                if sim >= 0.5 {
                    pairs.push(SimilarPair {
                        a: bodies[i].0.clone(),
                        b: bodies[j].0.clone(),
                        similarity: sim,
                    });
                }
            }
        }
        let families = sqlx::query(
            "SELECT strategy_family, sum(n_obs)::bigint, sum(n_success)::bigint
               FROM regime_causal.regime_strategy_prior WHERE tenant_id = $1 GROUP BY strategy_family",
        )
        .bind(tenant)
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| FamilyRow {
            family: ident(&r.get::<String, _>(0)),
            tried: u32::try_from(r.get::<i64, _>(1)).unwrap_or(u32::MAX),
            passed: u32::try_from(r.get::<i64, _>(2)).unwrap_or(u32::MAX),
        })
        .collect();
        Ok(CuratorInput {
            skills,
            pairs,
            families,
        })
    }

    /// Store a curator run's items as open proposals. Nothing here changes a skill.
    ///
    /// # Errors
    /// The database.
    pub async fn save_curator_items(
        &self,
        run: Uuid,
        items: &[harness::memory::curator::CuratorItem],
    ) -> Result<usize, StoreError> {
        for item in items {
            let v = serde_json::to_value(item).unwrap_or(Value::Null);
            let kind = v
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("deprecate")
                .to_string();
            sqlx::query("INSERT INTO skill_registry.curator_item (run_id, kind, detail) VALUES ($1, $2, $3)")
                .bind(run)
                .bind(kind)
                .bind(v)
                .execute(&self.pg)
                .await?;
        }
        Ok(items.len())
    }

    /// Curator items with one status, oldest first.
    ///
    /// # Errors
    /// The database.
    pub async fn curator_items(&self, status: &str) -> Result<Vec<Value>, StoreError> {
        Ok(sqlx::query("SELECT item_id, run_id, kind, detail, status FROM skill_registry.curator_item WHERE status = $1 ORDER BY item_id LIMIT 200")
            .bind(status)
            .fetch_all(&self.pg)
            .await?
            .iter()
            .map(|r| json!({
                "item_id": r.get::<i64, _>(0),
                "run_id": r.get::<Uuid, _>(1),
                "kind": r.get::<String, _>(2),
                "detail": r.get::<Value, _>(3),
                "status": r.get::<String, _>(4),
            }))
            .collect())
    }

    /// Accept or dismiss an item. Accepting records the decision; the change itself
    /// still goes through the proposal path.
    ///
    /// # Errors
    /// The database; a decided or missing item.
    pub async fn decide_curator_item(
        &self,
        item: i64,
        accept: bool,
        by: Uuid,
    ) -> Result<(), StoreError> {
        let done = sqlx::query(
            "UPDATE skill_registry.curator_item SET status = $2, decided_by = $3, decided_at = now()
              WHERE item_id = $1 AND status = 'open'",
        )
        .bind(item)
        .bind(if accept { "accepted" } else { "dismissed" })
        .bind(by)
        .execute(&self.pg)
        .await?;
        if done.rows_affected() == 0 {
            return Err(StoreError::NotFound(format!("open curator item {item}")));
        }
        Ok(())
    }
}
