//! `/api/skills/*` — the skill registry's REST surface (Set R-a RA-4.1, RA-4.2;
//! Set R-b RB-2.1 to RB-2.3; ADR-0028).
//!
//! # Who may do what
//!
//! * **Read the glossary, search, read a skill, propose, vote** — any session that
//!   holds the matching research scope (`research:artifacts` to propose, `research:memory`
//!   to vote); a web login holds everything.
//! * **Verify, record a step, admit, quarantine, deprecate, promote, record a
//!   promotion condition** — `skills.admit`, which is in
//!   `auth::scopes::NEVER_FOR_AGENTS`: no agent session can hold it, so no agent can
//!   reach any route that changes what is admitted (SK-03). Each of these handlers
//!   begins with [`require_admit`] and nothing else, so the authority question is asked
//!   in one place by one name.
//!
//! # Search is lexical, and says so
//!
//! `GET /api/skills/search` ranks with Postgres full-text over the full `SKILL.md`
//! bodies. KM-07's store decision (ADR-RB-06: stay on pgvector, which the platform's
//! migrations already require; Milvus deferred) means a dense leg arrives with the
//! embedder wiring; until a skill body has an embedding the response says
//! `"mode": "lexical"`, never pretends otherwise.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use skills::lifecycle::{check_promotion, Condition};
use sqlx::Row;
use uuid::Uuid;

use crate::auth::scopes;
use crate::skills_store::{SkillStore, StoreError};
use crate::skills_verify::{self, ProposalInput};
use crate::{auth::BearerToken, state::AppState};

/// The scope that changes what the registry admits. Never an agent scope.
pub const ADMIT: &str = "skills.admit";

fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": code, "message": message.into() })),
    )
        .into_response()
}

fn store_err(e: StoreError) -> Response {
    match e {
        StoreError::Refused(m) => err(StatusCode::UNPROCESSABLE_ENTITY, "refused", m),
        StoreError::NotFound(m) => err(StatusCode::NOT_FOUND, "not_found", m),
        StoreError::Db(sqlx::Error::Database(d)) => err(
            StatusCode::CONFLICT,
            "registry_constraint",
            d.message().to_string(),
        ),
        StoreError::Db(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "database", e.to_string()),
    }
}

/// The one authority check every state-changing route begins with.
#[allow(clippy::result_large_err)]
fn require_admit(token: &BearerToken) -> Result<(), Response> {
    if token.permits(ADMIT) {
        Ok(())
    } else {
        Err(err(
            StatusCode::FORBIDDEN,
            "skills_admit_required",
            "changing what the registry admits needs the `skills.admit` scope, which no agent \
             session can hold; an agent proposes and the platform admits",
        ))
    }
}

#[allow(clippy::result_large_err)]
fn require(token: &BearerToken, scope: &str) -> Result<(), Response> {
    if token.permits(scope) {
        Ok(())
    } else {
        Err(err(
            StatusCode::FORBIDDEN,
            "missing_scope",
            format!("this route needs the `{scope}` scope"),
        ))
    }
}

fn row_json(s: &skills::materialiser::RegistrySkill) -> Value {
    json!({
        "name": s.name,
        "scope": s.scope,
        "status": s.status,
        "category": s.category,
        "description": s.description,
        "version": s.version,
        "pinned": s.pinned,
    })
}

#[derive(Deserialize)]
pub struct ProjectQuery {
    pub project: Option<Uuid>,
}

/// `GET /api/skills/glossary` — every skill this caller can see, and which of them a
/// session would carry resident. Read from the `skill_glossary` view, so a quarantined
/// skill is absent in the statement that quarantined it.
pub async fn glossary(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<ProjectQuery>,
) -> Response {
    let store = SkillStore::new(state.pg.clone());
    let vis = match store.visible(token.user_id(), q.project).await {
        Ok(v) => v,
        Err(e) => return store_err(e),
    };
    let refs: Vec<&skills::materialiser::RegistrySkill> = vis.iter().collect();
    let frozen = store.frozen_selection(q.project).await.unwrap_or_default();
    let resident = skills::materialiser::select(&refs, &frozen)
        .map(|r| {
            (
                r.resident
                    .iter()
                    .map(|(_, s)| s.name.clone())
                    .collect::<Vec<_>>(),
                r.cut,
            )
        })
        .unwrap_or_default();
    // The ids the registry's other routes (`/item/{id}`, quarantine, promote) are addressed by.
    // `visible` returns the materialiser's rows, which carry no id, so they are read here
    // under the same visibility predicate and joined on (name, scope), which is unique among
    // what one viewer can see. Without them the page could list a skill and do nothing to it.
    let ids: std::collections::HashMap<(String, String), Uuid> = sqlx::query(
        "SELECT name, scope, skill_id FROM skill_registry.skill_glossary
          WHERE scope IN ('core', 'global')
             OR (scope = 'user' AND owner_user_id = $1)
             OR (scope = 'project' AND project_id = $2)",
    )
    .bind(token.user_id())
    .bind(q.project)
    .fetch_all(&state.pg)
    .await
    .map(|rows| {
        rows.iter()
            .map(|r| {
                (
                    (r.get::<String, _>(0), r.get::<String, _>(1)),
                    r.get::<Uuid, _>(2),
                )
            })
            .collect()
    })
    .unwrap_or_default();
    Json(json!({
        "skills": vis
            .iter()
            .map(|s| {
                let mut v = row_json(s);
                v["skill_id"] = json!(ids.get(&(s.name.clone(), v["scope"].as_str().unwrap_or_default().to_string())));
                v
            })
            .collect::<Vec<_>>(),
        "resident": resident.0,
        "cut": resident.1,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub project: Option<Uuid>,
    pub limit: Option<i64>,
}

/// `GET /api/skills/search?q=` — full-text over the whole `SKILL.md` body.
pub async fn search(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<SearchQuery>,
) -> Response {
    let words: Vec<String> =
        q.q.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .map(str::to_ascii_lowercase)
            .collect();
    if words.is_empty() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "empty_query",
            "give at least one word of two letters or more",
        );
    }
    let tsq = words.join(" | ");
    let rows = sqlx::query(
        "SELECT g.name, g.scope, g.status, g.description,
                ts_rank(to_tsvector('english', b.files ->> 'SKILL.md'), to_tsquery('english', $1)) AS rank, g.skill_id
           FROM skill_registry.skill_glossary g
           JOIN skill_registry.skill_bundle b ON b.version_hash = g.current_version
          WHERE (g.scope IN ('core', 'global')
                 OR (g.scope = 'user' AND g.owner_user_id = $2)
                 OR (g.scope = 'project' AND g.project_id = $3))
            AND to_tsvector('english', b.files ->> 'SKILL.md') @@ to_tsquery('english', $1)
          ORDER BY rank DESC, g.name
          LIMIT $4",
    )
    .bind(&tsq)
    .bind(token.user_id())
    .bind(q.project)
    .bind(q.limit.unwrap_or(10).clamp(1, 50))
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => Json(json!({
            "mode": "lexical",
            "embedder": false,
            "note": "ranked by full-text match over the whole skill body; no dense retrieval is wired to the registry yet",
            "results": rows.iter().map(|r| json!({
                "name": r.get::<String, _>(0),
                "scope": r.get::<String, _>(1),
                "status": r.get::<String, _>(2),
                "description": r.get::<String, _>(3),
                "rank": r.get::<f32, _>(4),
                "skill_id": r.get::<Uuid, _>(5),
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => store_err(e.into()),
    }
}

/// `GET /api/skills/item/{id}` — one skill, its versions and its usage statistics.
pub async fn get_skill(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> Response {
    let admin = token.permits(ADMIT);
    let row = sqlx::query(
        "SELECT name, scope, status, category, current_version, pinned, alias_of,
                owner_user_id::text, project_id::text
           FROM skill_registry.skill WHERE skill_id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await;
    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, "not_found", id.to_string()),
        Err(e) => return store_err(e.into()),
    };
    let (scope, status): (String, String) = (row.get(1), row.get(2));
    let owner: Option<String> = row.get(7);
    let visible_to_caller = admin
        || (matches!(status.as_str(), "provisional" | "admitted")
            && (matches!(scope.as_str(), "core" | "global")
                || owner.as_deref() == Some(&token.user_id().to_string())));
    if !visible_to_caller {
        return err(StatusCode::NOT_FOUND, "not_found", id.to_string());
    }
    let store = SkillStore::new(state.pg.clone());
    let stats = store.stats(id).await.unwrap_or_default();
    // A deprecated skill says what replaced it (RB-2.6).
    let resolves_to = if status == "deprecated" {
        store.resolve_alias(id).await.ok().flatten().map(
            |(sid, name, version)| json!({ "skill_id": sid, "name": name, "version": version }),
        )
    } else {
        None
    };
    let versions = sqlx::query(
        "SELECT version_hash, parent_version, description, created_by, created_at
           FROM skill_registry.skill_version WHERE skill_id = $1 ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();
    Json(json!({
        "skill_id": id,
        "name": row.get::<String, _>(0),
        "scope": scope,
        "status": status,
        "category": row.get::<String, _>(3),
        "current_version": row.get::<Option<String>, _>(4),
        "pinned": row.get::<bool, _>(5),
        "alias_of": row.get::<Option<Uuid>, _>(6),
        "resolves_to": resolves_to,
        "usage": { "exposures": stats.exposures, "activations": stats.activations, "balance": stats.balance },
        "versions": versions.iter().map(|v| json!({
            "version": v.get::<String, _>(0),
            "parent": v.get::<Option<String>, _>(1),
            "description": v.get::<String, _>(2),
            "created_by": v.get::<String, _>(3),
            "created_at": v.get::<chrono::DateTime<chrono::Utc>, _>(4),
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct ProposeBody {
    pub project_id: Uuid,
    pub name: String,
    pub category: String,
    pub reason: String,
    pub source_ref: String,
    #[serde(default)]
    pub sources: Vec<String>,
    /// Path to text. `SKILL.md` is required.
    pub files: std::collections::BTreeMap<String, String>,
}

/// `POST /api/skills/proposals` — the agent proposes; the platform admits. A proposal
/// with no cited trajectory is refused with nothing written; a near-duplicate becomes
/// a patch of the existing skill (`skills.patch_threshold`).
pub async fn propose(
    State(state): State<AppState>,
    token: BearerToken,
    Json(b): Json<ProposeBody>,
) -> Response {
    if let Err(r) = require(&token, scopes::ARTIFACTS) {
        return r;
    }
    let files = b
        .files
        .into_iter()
        .map(|(k, v)| (k, v.into_bytes()))
        .collect();
    let input = ProposalInput {
        user: token.user_id(),
        project: b.project_id,
        name: b.name,
        category: b.category,
        reason: b.reason,
        source_ref: b.source_ref,
        sources: b.sources,
        files,
    };
    match skills_verify::propose(&state.pg, input).await {
        Ok(o) => (
            StatusCode::CREATED,
            Json(json!({
                "proposal_id": o.proposal_id,
                "status": o.status,
                "disposition": match &o.disposition {
                    skills::registry::Disposition::New => json!({ "kind": "new" }),
                    skills::registry::Disposition::Patch { target, similarity } =>
                        json!({ "kind": "patch", "target": target, "similarity": similarity }),
                },
                "similarity": o.similarity,
                "verdicts": o.verdicts,
            })),
        )
            .into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

/// `GET /api/skills/proposals` — the promotion queue. A caller without `skills.admit`
/// sees only their own proposals.
pub async fn list_proposals(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<ListQuery>,
) -> Response {
    let admin = token.permits(ADMIT);
    let rows = sqlx::query(
        "SELECT proposal_id, proposed_name, project_id, proposer, reason, status, similarity, created_at
           FROM skill_registry.skill_proposal
          WHERE ($1::text IS NULL OR status = $1) AND ($2 OR proposer = $3)
          ORDER BY created_at DESC LIMIT 100",
    )
    .bind(q.status)
    .bind(admin)
    .bind(token.user_id().to_string())
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => Json(json!({ "proposals": rows.iter().map(|r| json!({
            "proposal_id": r.get::<Uuid, _>(0),
            "name": r.get::<String, _>(1),
            "project_id": r.get::<Uuid, _>(2),
            "proposer": r.get::<String, _>(3),
            "reason": r.get::<String, _>(4),
            "status": r.get::<String, _>(5),
            "similarity": r.get::<Option<f64>, _>(6),
            "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>(7),
        })).collect::<Vec<_>>() }))
        .into_response(),
        Err(e) => store_err(e.into()),
    }
}

#[derive(Deserialize)]
pub struct PairBody {
    pub kind: String,
    pub model: String,
    pub variant: String,
    pub activated: bool,
    pub with_success: bool,
    pub without_success: bool,
    pub with_cost: f64,
    pub without_cost: f64,
}

/// `POST /api/skills/proposals/{id}/pairs` — a paired replay outcome from the
/// evaluation harness. `skills.admit`: the numbers admission reads are never written
/// by the session that proposed.
pub async fn add_pair(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<PairBody>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    let pair = skills::lifecycle::Pair {
        activated: b.activated,
        with_success: b.with_success,
        without_success: b.without_success,
        with_cost: b.with_cost,
        without_cost: b.without_cost,
    };
    match SkillStore::new(state.pg.clone())
        .record_pair(Some(id), None, &b.kind, &b.model, &b.variant, &pair)
        .await
    {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct StepBody {
    pub step: String,
    pub passed: bool,
    #[serde(default)]
    pub detail: Value,
}

/// `POST /api/skills/proposals/{id}/steps` — record a step the platform cannot compute
/// itself (the Reviewer's verdict).
pub async fn add_step(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<StepBody>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    match skills_verify::record_step(&state.pg, id, &b.step, b.passed, b.detail).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => store_err(e),
    }
}

/// `POST /api/skills/proposals/{id}/verify` — run what the platform can run and
/// report which step is still unmet.
pub async fn verify(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    match skills_verify::verify(&state.pg, id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => store_err(e),
    }
}

/// `POST /api/skills/proposals/{id}/admit` — admit a verified proposal as provisional.
pub async fn admit(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    match skills_verify::admit_proposal(&state.pg, id, &token.user_id().to_string()).await {
        Ok(skill) => (
            StatusCode::CREATED,
            Json(json!({ "skill_id": skill, "status": "provisional" })),
        )
            .into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct VoteBody {
    pub session_id: Uuid,
    pub vote: i16,
}

/// `POST /api/skills/{id}/vote` — a helpfulness vote from a session that was exposed.
pub async fn vote(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<VoteBody>,
) -> Response {
    if let Err(r) = require(&token, scopes::MEMORY) {
        return r;
    }
    match SkillStore::new(state.pg.clone())
        .vote(id, b.session_id, b.vote)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct ReasonBody {
    pub reason: String,
    /// On `deprecate`: the skill that replaces this one. Lookups of the deprecated skill
    /// resolve through it.
    #[serde(default)]
    pub alias_of: Option<Uuid>,
}

async fn retire_with(
    state: AppState,
    token: BearerToken,
    id: Uuid,
    b: ReasonBody,
    status: &'static str,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    let store = SkillStore::new(state.pg.clone());
    if let Err(e) = store.retire(id, status, &b.reason).await {
        return store_err(e);
    }
    if let (Some(target), "deprecated") = (b.alias_of, status) {
        if let Err(e) = store.set_alias(id, target).await {
            return store_err(e);
        }
    }
    Json(json!({ "skill_id": id, "status": status, "alias_of": b.alias_of })).into_response()
}

/// `POST /api/skills/{id}/quarantine` — out of every glossary in one statement, with
/// a `skill_notice` for live sessions.
pub async fn quarantine(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<ReasonBody>,
) -> Response {
    retire_with(state, token, id, b, "quarantined").await
}

/// `POST /api/skills/{id}/deprecate`.
pub async fn deprecate(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<ReasonBody>,
) -> Response {
    retire_with(state, token, id, b, "deprecated").await
}

#[derive(Deserialize)]
pub struct ConditionBody {
    pub condition: String,
    pub passed: bool,
    #[serde(default)]
    pub evidence: Value,
}

/// `POST /api/skills/{id}/conditions` — record one of the six promotion conditions,
/// attributed to the recording user.
pub async fn record_condition(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<ConditionBody>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    let Some(c) = Condition::parse(&b.condition) else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_condition",
            format!(
                "{:?} is not one of {:?}",
                b.condition,
                Condition::ALL.map(Condition::as_str)
            ),
        );
    };
    match SkillStore::new(state.pg.clone())
        .record_condition(id, c, b.passed, &token.user_id().to_string(), &b.evidence)
        .await
    {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct PromoteBody {
    /// `user` or `global`.
    pub to: String,
}

/// `POST /api/skills/{id}/promote` — project to user is the owner's approval (audited
/// in `skill_promotion_log`); to global it needs all six recorded conditions, and the
/// refusal names each one that is not.
pub async fn promote(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<PromoteBody>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    let store = SkillStore::new(state.pg.clone());
    match b.to.as_str() {
        "user" => match store
            .move_scope(id, "user", token.user_id(), token.user_id())
            .await
        {
            Ok(()) => {
                Json(json!({ "skill_id": id, "scope": "user", "approved_by": token.user_id() }))
                    .into_response()
            }
            Err(e) => store_err(e),
        },
        "global" => {
            let file = match store.promotion_file(id).await {
                Ok(f) => f,
                Err(e) => return store_err(e),
            };
            if let Err(refused) = check_promotion(&file) {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "promotion_refused",
                        "unmet": refused.0.iter().map(|(c, why)| json!({ "condition": c.as_str(), "why": why })).collect::<Vec<_>>(),
                    })),
                )
                    .into_response();
            }
            match store
                .move_scope(id, "global", token.user_id(), token.user_id())
                .await
            {
                Ok(()) => Json(json!({ "skill_id": id, "scope": "global" })).into_response(),
                Err(e) => store_err(e),
            }
        }
        other => err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_target",
            format!("{other:?}: promote to `user` or `global`"),
        ),
    }
}

/// `GET /api/skills/notices?after=` — what a live session appends as system messages
/// when a resident skill changes standing (SK-14).
pub async fn notices(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(q): Query<NoticeQuery>,
) -> Response {
    match SkillStore::new(state.pg.clone())
        .notices_since(q.after.unwrap_or(0))
        .await
    {
        Ok(n) => Json(
            json!({ "notices": n.iter().map(|(id, name, kind, reason)| json!({
            "notice_id": id, "skill": name, "kind": kind, "reason": reason,
        })).collect::<Vec<_>>() }),
        )
        .into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct NoticeQuery {
    pub after: Option<i64>,
}

/// `GET /api/skills/runs/{session}/bill` — the skills a run had exposed and activated,
/// by version hash (RA-3.8).
pub async fn bill_of_materials(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(session): Path<Uuid>,
) -> Response {
    match SkillStore::new(state.pg.clone())
        .bill_of_materials(session)
        .await
    {
        Ok(b) => Json(json!({ "session_id": session, "skills": b })).into_response(),
        Err(e) => store_err(e),
    }
}

/// `POST /api/skills/curator/run` — run the curator over the registry's statistics and
/// store what it proposes as open items. It reads numbers and identifiers only, under a
/// token cap, and writes nothing to any skill: its items wait for a person.
pub async fn curator_run(State(state): State<AppState>, token: BearerToken) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    let store = SkillStore::new(state.pg.clone());
    let input = match store.curator_input(&token.user_id().to_string()).await {
        Ok(i) => i,
        Err(e) => return store_err(e),
    };
    let run = match harness::memory::curator::curate(
        &input,
        &harness::memory::curator::CuratorConfig::default(),
    ) {
        Ok(r) => r,
        Err(e) => {
            return err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "curator_input_refused",
                e.to_string(),
            )
        }
    };
    let run_id = Uuid::new_v4();
    match store.save_curator_items(run_id, &run.items).await {
        Ok(n) => Json(json!({ "run_id": run_id, "items": run.items, "stored": n, "cut": run.cut }))
            .into_response(),
        Err(e) => store_err(e),
    }
}

/// `GET /api/skills/curator/items` — open curator items.
pub async fn curator_items(State(state): State<AppState>, token: BearerToken) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    match SkillStore::new(state.pg.clone())
        .curator_items("open")
        .await
    {
        Ok(v) => Json(json!({ "items": v })).into_response(),
        Err(e) => store_err(e),
    }
}

#[derive(Deserialize)]
pub struct DecideBody {
    pub accept: bool,
}

/// `POST /api/skills/curator/items/{item_id}/decide`.
pub async fn curator_decide(
    State(state): State<AppState>,
    token: BearerToken,
    Path(item_id): Path<i64>,
    Json(b): Json<DecideBody>,
) -> Response {
    if let Err(r) = require_admit(&token) {
        return r;
    }
    match SkillStore::new(state.pg.clone())
        .decide_curator_item(item_id, b.accept, token.user_id())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_err(e),
    }
}

#[cfg(test)]
mod tests {
    /// Every handler that changes what the registry admits starts with `require_admit`.
    /// Read from the source so adding a handler without the check fails here, not in
    /// production: the list of handlers that are allowed not to is closed and short.
    #[test]
    fn every_state_changing_handler_starts_with_the_admit_check() {
        let src = include_str!("skills.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        // Handlers a non-admin may call, each guarded by a research scope or read-only.
        const OPEN: [&str; 10] = [
            "glossary",
            "search",
            "get_skill",
            "propose",
            "list_proposals",
            "vote",
            "notices",
            "bill_of_materials",
            "retire_with",
            "require",
        ];
        let mut checked = 0;
        for chunk in prod.split("pub async fn ").skip(1) {
            let name: String = chunk
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if OPEN.contains(&name.as_str()) {
                continue;
            }
            let body_start = chunk.find('{').unwrap_or(0);
            let head: String = chunk[body_start..].chars().take(400).collect();
            assert!(
                head.contains("require_admit(&token)") || head.contains("retire_with(state, token"),
                "{name} changes the registry but does not begin with require_admit"
            );
            checked += 1;
        }
        assert!(
            checked >= 8,
            "only {checked} admin handlers were found; the scan is not reading the file"
        );
    }
}
