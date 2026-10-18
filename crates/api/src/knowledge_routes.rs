//! `/api/knowledge/*` — the knowledge model's read and write surface for Set R-b
//! (RB-1.2, RB-1.3, RB-1.7, RB-3.1 to RB-3.3, RB-4.3 to RB-4.5, RB-5.1 to RB-5.6).
//!
//! The rules are in `research` and `harness::memory`; this file reads rows into them
//! and returns what they decide. Two kinds of caller, two scopes:
//!
//! * **The agent** (`research:memory`): ask a question (is this move vetoed, what are
//!   the analogs of this window, what regime is this instrument in, how does my campaign
//!   compare with random, which tier answers) and tag where a hypothesis came from.
//! * **The platform** ([`KNOWLEDGE_WRITE`], held by a web login and by no agent token):
//!   record what a move did, what a regime-sliced study measured, and a campaign's
//!   scores. These are *measurements*. A record the agent could write about its own
//!   moves would let it dilute the failures that veto them, and a score list the agent
//!   supplied to `compare-to-random` would make the comparison whatever it liked.
//!
//! # Hand-offs (callers outside this set's territory)
//!
//! Each of these needs a call from a file this set does not own; the exact lines are in
//! the Set R-b report. They are listed here so the next reader of this file can see what
//! still has no production caller:
//!
//! * the campaign driver records each move's outcome with [`MoveStore::record`] and each
//!   trial's score with [`ArmStore::record_score`], and creates the arm with
//!   [`ArmStore::create`];
//! * Gate 3 calls [`arm_gate`] for a campaign candidate;
//! * the study worker calls [`PriorStore::update`] after a regime-sliced study.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use harness::memory::rules::{flag_repeat, refuse_findings_as_evidence, RepeatKey, Source};
use harness::memory::DataWindow;
use research::analog::{analogs as find_analogs, Episode, Query as AnalogQ};
use research::move_memory::{motif, veto_move, MoveRecord, Outcome, VetoPolicy};
use research::propose::Move;
use research::random_arm::{arm_seed, compare_to_random, gate_check, Arm, ArmError};
use research::regime_prior::{profile, resolve_regime_gate, top_families, PriorCell};
use research::tiers::{ladder, LedgerCounts};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::auth::scopes;
use crate::knowledge::{RegimeLabel, RegimeScopeStore, RegimeStore, ScopeResolution};
use crate::{auth::BearerToken, state::AppState};

/// The platform's write authority over knowledge records. A web login holds it (it holds
/// everything); no agent session can: `validate_agent_scopes` refuses any scope outside
/// `RESEARCH_SCOPES`.
pub const KNOWLEDGE_WRITE: &str = "knowledge.write";

fn err(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": code, "message": message.into() })),
    )
        .into_response()
}

fn db(e: &sqlx::Error) -> Response {
    err(StatusCode::INTERNAL_SERVER_ERROR, "database", e.to_string())
}

#[allow(clippy::result_large_err)]
fn need(token: &BearerToken, scope: &str) -> Result<(), Response> {
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

// ── retrieval metadata (RB-1.1) ─────────────────────────────────────────────

/// An insight's staleness horizon in days: the same 90 the finding kind uses.
const INSIGHT_STALE_DAYS: i64 = 90;

/// Attach age, staleness and capability flags to each retrieved insight (KM-02).
///
/// The timestamps are read in one tenant-scoped query. A failure to read them leaves the
/// flags out of the row rather than inventing them: an absent `age_days` says "not
/// measured".
pub async fn annotate_insights(
    pg: &PgPool,
    tenant: &str,
    insights: &[crate::knowledge_jobs::Insight],
    have: Option<&[String]>,
) -> Vec<Value> {
    let ids: Vec<Uuid> = insights.iter().map(|i| i.insight_id).collect();
    let mut when: std::collections::HashMap<Uuid, (DateTime<Utc>, Option<DateTime<Utc>>)> =
        std::collections::HashMap::new();
    if let Ok(mut tx) = ledger::pg::tenant_tx(pg, tenant).await {
        if let Ok(rows) = sqlx::query(
            "SELECT insight_id, created_at, last_confirmed_at FROM knowledge.insight WHERE insight_id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        {
            for r in rows {
                when.insert(r.get(0), (r.get(1), r.get(2)));
            }
        }
        let _ = tx.commit().await;
    }
    let today = Utc::now().date_naive().to_string();
    insights
        .iter()
        .map(|i| {
            let mut v = serde_json::to_value(i).unwrap_or(Value::Null);
            let requires: Vec<String> = i
                .scope
                .get("requires")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if let (Some(obj), Some((created, confirmed))) =
                (v.as_object_mut(), when.get(&i.insight_id))
            {
                let last = confirmed.unwrap_or(*created).date_naive().to_string();
                let a = harness::memory::rules::annotate(
                    &last,
                    &today,
                    Some(INSIGHT_STALE_DAYS),
                    false,
                    &requires,
                    have,
                );
                obj.insert("age_days".into(), json!(a.age_days));
                obj.insert("stale".into(), json!(a.stale));
                obj.insert("capability_mismatch".into(), json!(a.capability_mismatch));
                obj.insert("missing_capabilities".into(), json!(a.missing_capabilities));
                obj.insert("contested".into(), json!(i.contradicted_n > i.support_n));
            }
            v
        })
        .collect()
}

/// Whether a retrieval should issue a semantic (embedding) query: only when the caller
/// asked a question and the structured filter left something to rank (KM-02). An embedding
/// call that cannot change the answer is cost, latency and a request the caller did not need.
#[must_use]
pub fn should_issue_semantic_query(metadata_hits: usize, q: Option<&str>) -> bool {
    metadata_hits > 0 && q.is_some_and(|q| !q.trim().is_empty())
}

// ── move memory (RB-1.3) ────────────────────────────────────────────────────

pub struct MoveStore {
    pg: PgPool,
}

impl MoveStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Record what a move did. The campaign driver calls this with a measured outcome.
    ///
    /// # Errors
    /// The database.
    pub async fn record(
        &self,
        user: Uuid,
        instrument: &str,
        m: &Move,
        outcome: Outcome,
        run_ref: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO agent_memory.move_record (user_id, instrument, motif, outcome, run_ref) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user)
        .bind(instrument)
        .bind(motif(m))
        .bind(outcome.as_str())
        .bind(run_ref)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Every record of one motif, oldest first.
    ///
    /// # Errors
    /// The database.
    pub async fn history(
        &self,
        user: Uuid,
        motif_key: &str,
    ) -> Result<Vec<MoveRecord>, sqlx::Error> {
        Ok(sqlx::query(
            "SELECT motif, outcome, run_ref FROM agent_memory.move_record WHERE user_id = $1 AND motif = $2 ORDER BY record_id",
        )
        .bind(user)
        .bind(motif_key)
        .fetch_all(&self.pg)
        .await?
        .iter()
        .filter_map(|r| {
            Some(MoveRecord {
                motif: r.get(0),
                outcome: Outcome::parse(r.get::<String, _>(1).as_str())?,
                run_ref: r.get(2),
            })
        })
        .collect())
    }

    /// Whether a person has overridden the veto on this motif.
    ///
    /// # Errors
    /// The database.
    pub async fn overridden(&self, user: Uuid, motif_key: &str) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query("SELECT EXISTS (SELECT 1 FROM agent_memory.move_veto_override WHERE user_id = $1 AND motif = $2)")
            .bind(user)
            .bind(motif_key)
            .fetch_one(&self.pg)
            .await?
            .get(0))
    }
}

#[derive(Deserialize)]
pub struct VetoBody {
    pub candidate: Move,
}

/// `POST /api/knowledge/moves/veto` — has this shape of move a record of failure, and
/// how sure are we? Advisory below the policy's confidence, blocking above it, citing
/// the records either way. A block a person has overridden says so and does not stop.
pub async fn veto(
    State(state): State<AppState>,
    token: BearerToken,
    Json(b): Json<VetoBody>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let store = MoveStore::new(state.pg.clone());
    let key = motif(&b.candidate);
    let records = match store.history(token.user_id(), &key).await {
        Ok(r) => r,
        Err(e) => return db(&e),
    };
    let verdict = veto_move(&b.candidate, &records, &VetoPolicy::default());
    let overridden = verdict.blocks()
        && store
            .overridden(token.user_id(), &key)
            .await
            .unwrap_or(false);
    Json(json!({ "motif": key, "veto": verdict, "overridden": overridden, "may_proceed": !verdict.blocks() || overridden })).into_response()
}

#[derive(Deserialize)]
pub struct MoveRecordBody {
    pub instrument: String,
    pub candidate: Move,
    pub outcome: String,
    pub run_ref: String,
}

/// `POST /api/knowledge/moves/records` — record what a move did. Platform authority.
pub async fn record_move(
    State(state): State<AppState>,
    token: BearerToken,
    Json(b): Json<MoveRecordBody>,
) -> Response {
    if let Err(r) = need(&token, KNOWLEDGE_WRITE) {
        return r;
    }
    let Some(outcome) = Outcome::parse(&b.outcome) else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_outcome",
            "outcome is improved, failed or neutral",
        );
    };
    match MoveStore::new(state.pg.clone())
        .record(
            token.user_id(),
            &b.instrument,
            &b.candidate,
            outcome,
            &b.run_ref,
        )
        .await
    {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => db(&e),
    }
}

#[derive(Deserialize)]
pub struct OverrideBody {
    pub motif: String,
    pub reason: String,
}

/// `POST /api/knowledge/moves/overrides` — a person proceeds past a blocking veto.
pub async fn override_veto(
    State(state): State<AppState>,
    token: BearerToken,
    Json(b): Json<OverrideBody>,
) -> Response {
    if let Err(r) = need(&token, KNOWLEDGE_WRITE) {
        return r;
    }
    match sqlx::query(
        "INSERT INTO agent_memory.move_veto_override (user_id, motif, reason) VALUES ($1, $2, $3)",
    )
    .bind(token.user_id())
    .bind(&b.motif)
    .bind(&b.reason)
    .execute(&state.pg)
    .await
    {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(e) => db(&e),
    }
}

// ── hypothesis sources (RB-1.2) ─────────────────────────────────────────────

/// The one refusal every route that is offered evidence gives when a *finding* is cited as
/// it (KM-03): a finding says where an idea came from, never what a new experiment rests on.
/// `None` when no id is a finding. The hypothesis and experiment routes call this, so the
/// rule has one body and one status.
#[must_use]
pub fn finding_evidence_refusal(evidence_ids: &[String]) -> Option<Response> {
    let bad = refuse_findings_as_evidence(evidence_ids).err()?;
    Some(
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "finding_is_not_evidence",
                "findings": bad,
                "message": "a finding says where an idea came from, never what the new experiment rests on; cite the experiment, job or artifact that measured it",
            })),
        )
            .into_response(),
    )
}

#[derive(Deserialize)]
pub struct SourceBody {
    pub hypothesis_id: Uuid,
    /// `memory:<finding id>` when a stored finding suggested it.
    pub source: String,
    pub instrument: String,
    pub window_start: NaiveDate,
    pub window_end: NaiveDate,
    /// The evidence the experiment rests on. Findings are refused.
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}

/// `POST /api/knowledge/hypothesis-sources` — say where a hypothesis came from. A
/// finding cited as evidence is refused; a hypothesis that repeats an earlier one (the
/// same finding, instrument and an overlapping window) is recorded as a repeated trial
/// and the response says how many times.
pub async fn hypothesis_source(
    State(state): State<AppState>,
    token: BearerToken,
    Json(b): Json<SourceBody>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    if let Some(refusal) = finding_evidence_refusal(&b.evidence_ids) {
        return refusal;
    }
    let user = token.user_id();
    let src = Source::parse(&b.source);
    let (kind, finding) = match &src {
        Source::Memory(f) => ("memory", Some(f.clone())),
        Source::Other(_) => ("other", None),
    };
    let mut repeats = 0usize;
    if let Some(f) = &finding {
        let rows = sqlx::query(
            "SELECT finding_id, instrument, window_start::text, window_end::text
               FROM agent_memory.hypothesis_source
              WHERE user_id = $1 AND finding_id = $2 AND instrument = $3",
        )
        .bind(user)
        .bind(f)
        .bind(&b.instrument)
        .fetch_all(&state.pg)
        .await;
        let earlier: Vec<RepeatKey> = match rows {
            Ok(rows) => rows
                .iter()
                .map(|r| RepeatKey {
                    finding: r.get(0),
                    instrument: r.get(1),
                    window: DataWindow::new(r.get::<String, _>(2), r.get::<String, _>(3)),
                })
                .collect(),
            Err(e) => return db(&e),
        };
        repeats = flag_repeat(
            &RepeatKey {
                finding: f.clone(),
                instrument: b.instrument.clone(),
                window: DataWindow::new(b.window_start.to_string(), b.window_end.to_string()),
            },
            &earlier,
        );
    }
    let done = sqlx::query(
        "INSERT INTO agent_memory.hypothesis_source
           (hypothesis_id, user_id, source_kind, finding_id, source_text, instrument, window_start, window_end, repeats)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(b.hypothesis_id)
    .bind(user)
    .bind(kind)
    .bind(finding)
    .bind(&b.source)
    .bind(&b.instrument)
    .bind(b.window_start)
    .bind(b.window_end)
    .bind(i32::try_from(repeats).unwrap_or(i32::MAX))
    .execute(&state.pg)
    .await;
    match done {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({ "hypothesis_id": b.hypothesis_id, "repeated_trial": repeats > 0, "repeats": repeats })),
        )
            .into_response(),
        Err(e) => db(&e),
    }
}

// ── analog search (RB-1.7) ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AnalogParams {
    pub instrument_id: i64,
    pub venue_id: i32,
    /// Analogs and the query are drawn only from fingerprints known by this time.
    pub as_of: Option<DateTime<Utc>>,
    pub k: Option<usize>,
    pub window_spec: Option<String>,
}

fn window_days(spec: &str) -> i64 {
    spec.split('d')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(21)
}

#[derive(Debug)]
pub enum AnalogError {
    NoFingerprint,
    Db(sqlx::Error),
}

/// Nearest windows over the explicit fingerprint vector as of `as_of` (RB-1.7). The whole
/// query, so a test can run it against a scratch database without the HTTP layer.
///
/// # Errors
/// [`AnalogError::NoFingerprint`] when the instrument has no complete fingerprint known by
/// `as_of`.
pub async fn analog_search(
    pg: &PgPool,
    instrument_id: i64,
    venue_id: i32,
    as_of: DateTime<Utc>,
    k: usize,
    spec: &str,
) -> Result<Value, AnalogError> {
    let rows = sqlx::query(
        "SELECT instrument_id, venue_id, knowledge_time, tier1, dimension_order
           FROM knowledge.instrument_fingerprint
          WHERE window_spec = $1 AND knowledge_time <= $2
          ORDER BY knowledge_time DESC LIMIT 20000",
    )
    .bind(spec)
    .bind(as_of)
    .fetch_all(pg)
    .await
    .map_err(AnalogError::Db)?;
    let horizon = window_days(spec);
    let mut dims: Vec<String> = Vec::new();
    let mut pool: Vec<Episode> = Vec::new();
    let mut query: Option<AnalogQ> = None;
    for r in &rows {
        let tier1: Vec<Option<f32>> = r.get(3);
        if tier1.iter().any(Option::is_none) {
            continue; // a coordinate that was not measured is not a coordinate of zero
        }
        let v: Vec<f64> = tier1.iter().flatten().map(|x| f64::from(*x)).collect();
        if dims.is_empty() {
            dims = r.get(4);
        }
        let (iid, vid): (i64, i32) = (r.get(0), r.get(1));
        let kt: DateTime<Utc> = r.get(2);
        let end = kt.timestamp().div_euclid(86_400);
        let ep = Episode {
            id: format!("{iid}:{vid}:{end}"),
            instrument: format!("{iid}:{vid}"),
            start: end - horizon,
            end,
            vector: v,
        };
        if iid == instrument_id && vid == venue_id && query.is_none() {
            // Rows are newest first, so the first match is the query's own latest window.
            query = Some(AnalogQ {
                id: Some(ep.id.clone()),
                instrument: ep.instrument.clone(),
                start: ep.start,
                end: ep.end,
                vector: ep.vector.clone(),
            });
        }
        pool.push(ep);
    }
    let q = query.ok_or(AnalogError::NoFingerprint)?;
    let out = find_analogs(&q, &pool, k.clamp(1, 50), horizon);
    Ok(json!({
        "vector": "explicit_standardised_fingerprint",
        "dimensions": dims,
        "window_spec": spec,
        "exclusion_zone_days": horizon,
        "as_of": as_of,
        "analogs": out,
    }))
}

/// `GET /api/knowledge/analogs` — nearest windows over the **explicit** fingerprint
/// vector, excluding the query's own episode and anything on the same instrument within
/// one window length of it, and nothing known after `as_of`.
pub async fn analogs(
    State(state): State<AppState>,
    token: BearerToken,
    Query(p): Query<AnalogParams>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let spec = p.window_spec.unwrap_or_else(|| "21d@1m".to_string());
    match analog_search(
        &state.pg,
        p.instrument_id,
        p.venue_id,
        p.as_of.unwrap_or_else(Utc::now),
        p.k.unwrap_or(5),
        &spec,
    )
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(AnalogError::NoFingerprint) => err(
            StatusCode::NOT_FOUND,
            "no_fingerprint",
            "that instrument has no complete fingerprint at or before as_of",
        ),
        Err(AnalogError::Db(e)) => db(&e),
    }
}

// ── regimes (RB-4.3 to RB-4.5) ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegimeParams {
    pub instrument: String,
    pub asset_class: String,
    pub regime: Option<String>,
    pub k: Option<usize>,
}

async fn resolve_scope(pg: &PgPool, p: &RegimeParams) -> Result<(String, String), Response> {
    match RegimeScopeStore::new(pg.clone()).resolve(&p.asset_class, Some(&p.instrument)).await {
        Ok(ScopeResolution::Scope(s)) => {
            let version: Option<String> = sqlx::query(
                "SELECT model_version FROM regime_causal.regime_state WHERE market_scope = $1
                  GROUP BY model_version ORDER BY max(event_time) DESC, model_version DESC LIMIT 1",
            )
            .bind(&s.market_scope)
            .fetch_optional(pg)
            .await
            .map_err(|e| db(&e))?
            .map(|r| r.get(0));
            version.map(|v| (s.market_scope, v)).ok_or_else(|| {
                err(StatusCode::UNPROCESSABLE_ENTITY, "no_labels", format!("{} has no regime labels yet; the labeller has not run for its scope", p.instrument))
            })
        }
        Ok(ScopeResolution::Ambiguous(names)) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "scope_ambiguous", "scopes": names })),
        )
            .into_response()),
        Ok(ScopeResolution::Unconfigured) => Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "scope_unconfigured",
            "no regime scope covers this instrument; a scope names a market and records why (ADR-P3-07)",
        )),
        Err(e) => Err(db(&e)),
    }
}

/// `GET /api/knowledge/regimes/profile` — occupancy, transitions and the current label.
/// An instrument with no labels is a typed refusal, never an empty profile.
pub async fn regime_profile(
    State(state): State<AppState>,
    token: BearerToken,
    Query(p): Query<RegimeParams>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let (scope, version) = match resolve_scope(&state.pg, &p).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let now = Utc::now();
    let hist = match RegimeStore::new(state.pg.clone())
        .history(&scope, &version, now - Duration::days(900), now)
        .await
    {
        Ok(h) => h,
        Err(e) => return db(&e),
    };
    let labels: Vec<String> = hist.iter().map(|o| o.label.as_str().to_string()).collect();
    match profile(&p.instrument, &labels) {
        Ok(pr) => {
            Json(json!({ "scope": scope, "model_version": version, "profile": pr })).into_response()
        }
        Err(e) => err(StatusCode::UNPROCESSABLE_ENTITY, "no_labels", e.to_string()),
    }
}

/// `GET /api/knowledge/regimes/prior` — the top families per regime, each with its
/// observation count. With `regime` set, that regime only.
pub async fn regime_prior(
    State(state): State<AppState>,
    token: BearerToken,
    Query(p): Query<RegimeParams>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let store = PriorStore::new(state.pg.clone());
    let mut out = serde_json::Map::new();
    for l in RegimeLabel::ALL {
        if p.regime.as_deref().is_some_and(|r| r != l.as_str()) {
            continue;
        }
        let cells = match store
            .cells(&token.user_id().to_string(), &p.instrument, l.as_str())
            .await
        {
            Ok(c) => c,
            Err(e) => return db(&e),
        };
        let top: Vec<Value> = top_families(&cells, p.k.unwrap_or(5), 0.5)
            .into_iter()
            .map(|(f, ucb, n)| json!({ "family": f, "ucb": ucb, "n_obs": n, "mean": cells[&f].mean() }))
            .collect();
        out.insert(l.as_str().to_string(), Value::Array(top));
    }
    Json(json!({ "instrument": p.instrument, "prior": out, "kind": "contextual_bandit_table" }))
        .into_response()
}

pub struct PriorStore {
    pg: PgPool,
}

impl PriorStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Fold one regime-sliced study into its cell. The study worker calls this.
    ///
    /// # Errors
    /// The database.
    pub async fn update(
        &self,
        tenant: &str,
        instrument: &str,
        regime: &str,
        family: &str,
        score: f64,
        success: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO regime_causal.regime_strategy_prior
               (tenant_id, instrument, regime, strategy_family, n_obs, n_success, score_sum, score_sumsq)
             VALUES ($1, $2, $3, $4, 1, $5, $6, $7)
             ON CONFLICT (tenant_id, instrument, regime, strategy_family) DO UPDATE SET
               n_obs = regime_strategy_prior.n_obs + 1,
               n_success = regime_strategy_prior.n_success + EXCLUDED.n_success,
               score_sum = regime_strategy_prior.score_sum + EXCLUDED.score_sum,
               score_sumsq = regime_strategy_prior.score_sumsq + EXCLUDED.score_sumsq,
               updated_at = now()",
        )
        .bind(tenant)
        .bind(instrument)
        .bind(regime)
        .bind(family)
        .bind(i32::from(success))
        .bind(score)
        .bind(score * score)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// The cells of one `(tenant, instrument, regime)`, by family.
    ///
    /// # Errors
    /// The database.
    pub async fn cells(
        &self,
        tenant: &str,
        instrument: &str,
        regime: &str,
    ) -> Result<std::collections::BTreeMap<String, PriorCell>, sqlx::Error> {
        Ok(sqlx::query(
            "SELECT strategy_family, n_obs, n_success, score_sum, score_sumsq
               FROM regime_causal.regime_strategy_prior WHERE tenant_id = $1 AND instrument = $2 AND regime = $3",
        )
        .bind(tenant)
        .bind(instrument)
        .bind(regime)
        .fetch_all(&self.pg)
        .await?
        .iter()
        .map(|r| {
            (
                r.get::<String, _>(0),
                PriorCell {
                    n_obs: u32::try_from(r.get::<i32, _>(1)).unwrap_or(0),
                    n_success: u32::try_from(r.get::<i32, _>(2)).unwrap_or(0),
                    sum: r.get(3),
                    sumsq: r.get(4),
                },
            )
        })
        .collect())
    }
}

#[derive(Deserialize)]
pub struct PriorUpdateBody {
    pub instrument: String,
    pub regime: String,
    pub family: String,
    pub score: f64,
    pub success: bool,
}

/// `POST /api/knowledge/regimes/prior` — fold one regime-sliced study into the table.
/// Platform authority: the numbers are measurements.
pub async fn update_regime_prior(
    State(state): State<AppState>,
    token: BearerToken,
    Json(b): Json<PriorUpdateBody>,
) -> Response {
    if let Err(r) = need(&token, KNOWLEDGE_WRITE) {
        return r;
    }
    if RegimeLabel::from_code(&b.regime).is_none() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_regime",
            "regime is low_vol, mid_vol or high_vol",
        );
    }
    if !b.score.is_finite() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_score",
            "score must be finite",
        );
    }
    match PriorStore::new(state.pg.clone())
        .update(
            &token.user_id().to_string(),
            &b.instrument,
            &b.regime,
            &b.family,
            b.score,
            b.success,
        )
        .await
    {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => db(&e),
    }
}

/// `POST /api/knowledge/regimes/gate` — the `add_regime_gate` move's machine check: an
/// unknown regime label is refused, and a known one returns the regime source it cites.
pub async fn regime_gate(
    State(state): State<AppState>,
    token: BearerToken,
    Query(p): Query<RegimeParams>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let Some(regime) = p.regime.clone() else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "missing_field", "regime");
    };
    let (_scope, version) = match resolve_scope(&state.pg, &p).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let known: Vec<&str> = RegimeLabel::ALL.iter().map(|l| l.as_str()).collect();
    match resolve_regime_gate(&Move::AddRegimeGate { regime }, &known, &version) {
        Ok(g) => Json(json!({ "gate": g })).into_response(),
        Err(e) => err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "regime_gate_refused",
            e.to_string(),
        ),
    }
}

// ── the tier ladder (RB-5.1 to RB-5.6, RB-4.6) ──────────────────────────────

/// Count what the ledger holds. A count whose table is missing or unreadable is `None`
/// (not measured), never zero.
pub async fn ledger_counts(pg: &PgPool, tenant: &str) -> LedgerCounts {
    let mut tx = match ledger::pg::tenant_tx(pg, tenant).await {
        Ok(t) => t,
        Err(_) => return LedgerCounts::default(),
    };
    async fn one(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, sql: &str) -> Option<u64> {
        let r = sqlx::query(sql).fetch_one(&mut **tx).await.ok()?;
        u64::try_from(r.try_get::<i64, _>(0).ok()?).ok()
    }
    let runs = one(&mut tx, "SELECT count(*)::bigint FROM mlops.trial").await;
    let tasks = one(&mut tx, "SELECT count(DISTINCT experiment_id)::bigint FROM mlops.trial WHERE experiment_id IS NOT NULL").await;
    let gated = one(&mut tx, "SELECT count(DISTINCT COALESCE(trial_id::text, experiment_id))::bigint FROM mlops.gate_verdict").await;
    let passes = one(
        &mut tx,
        "SELECT count(*)::bigint FROM (SELECT bool_and(passed) AS p FROM mlops.gate_verdict
          GROUP BY COALESCE(trial_id::text, experiment_id)) t WHERE p",
    )
    .await;
    let steps = one(
        &mut tx,
        "SELECT count(*)::bigint FROM mlops.trajectory_label",
    )
    .await;
    let curves = one(
        &mut tx,
        "SELECT count(DISTINCT run_id)::bigint FROM mlops.training_metric",
    )
    .await;
    let _ = tx.commit().await;
    LedgerCounts {
        curves,
        // No table records whether a partial curve's run went on to complete, so the
        // share is not measured, and M3's trigger says so rather than assuming it.
        curve_completion_share: None,
        runs,
        tasks,
        gated_candidates: gated,
        gate_passes: passes,
        // No ranking log exists yet: M9 stays on its rule tier and says why.
        ranked_items: None,
        ranked_groups: None,
        labelled_steps: steps,
    }
}

/// `GET /api/knowledge/tiers` — which tier answers for each learned model, from counts
/// computed over the ledger now.
pub async fn tiers(State(state): State<AppState>, token: BearerToken) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let counts = ledger_counts(&state.pg, &token.user_id().to_string()).await;
    // The separation verdict lives in the regime *analysis* module's schema, which no other
    // module may name (AT-42). Until that module exposes it, the regime tier reports "never
    // run", which keeps the rule tier answering and says so.
    let separation: Option<bool> = None;
    let regime = research::regime_learned::LearnedGate {
        separation_passed: separation,
    }
    .answer();
    Json(json!({
        "counts": counts,
        "ladder": ladder(&counts),
        "regime": { "model": "M7", "answer": regime },
        "note": "a learned tier answers only when its trigger is computed true; None means not measured, which is not met",
    }))
    .into_response()
}

// ── the random arm (RB-3.1 to RB-3.3) ───────────────────────────────────────

pub struct ArmStore {
    pg: PgPool,
}

impl ArmStore {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    /// Create a campaign's arm with the agent arm's budget and the platform's seed.
    ///
    /// # Errors
    /// The database (a second arm for one campaign is refused by the primary key).
    pub async fn create(
        &self,
        campaign: Uuid,
        user: Uuid,
        budget: i32,
    ) -> Result<u64, sqlx::Error> {
        let seed = arm_seed(&campaign.to_string());
        sqlx::query("INSERT INTO agent_memory.campaign_arm (campaign_id, user_id, budget, seed) VALUES ($1, $2, $3, $4)")
            .bind(campaign)
            .bind(user)
            .bind(budget)
            .bind(i64::from_ne_bytes(seed.to_ne_bytes()))
            .execute(&self.pg)
            .await?;
        Ok(seed)
    }

    /// Record one trial's sealed-metric score.
    ///
    /// # Errors
    /// The database (no arm for the campaign, or a repeated `seq`).
    pub async fn record_score(
        &self,
        campaign: Uuid,
        arm: &str,
        seq: i32,
        score: f64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO agent_memory.campaign_arm_score (campaign_id, arm, seq, score) VALUES ($1, $2, $3, $4)")
            .bind(campaign)
            .bind(arm)
            .bind(seq)
            .bind(score)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    /// Record a whole arm's scores at once, one per trial in draw order from `seq` 0,
    /// **idempotently**: a `seq` already recorded is left as it was (the table is
    /// append-only and a score is never rewritten). A job retried after a partial write
    /// therefore completes the arm and never doubles it. Returns how many were new.
    ///
    /// # Errors
    /// The database (no arm for the campaign).
    pub async fn record_scores(
        &self,
        campaign: Uuid,
        arm: &str,
        scores: &[f64],
    ) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "INSERT INTO agent_memory.campaign_arm_score (campaign_id, arm, seq, score)
             SELECT $1, $2, (t.ord - 1)::int, t.s FROM unnest($3::float8[]) WITH ORDINALITY AS t(s, ord)
             ON CONFLICT (campaign_id, arm, seq) DO NOTHING",
        )
        .bind(campaign)
        .bind(arm)
        .bind(scores)
        .execute(&self.pg)
        .await?
        .rows_affected())
    }

    /// The campaign's arm and both score lists, if it has an arm.
    ///
    /// # Errors
    /// The database.
    pub async fn load(&self, campaign: Uuid) -> Result<Option<(Arm, Vec<f64>)>, sqlx::Error> {
        let Some(a) = sqlx::query(
            "SELECT budget, seed FROM agent_memory.campaign_arm WHERE campaign_id = $1",
        )
        .bind(campaign)
        .fetch_optional(&self.pg)
        .await?
        else {
            return Ok(None);
        };
        let scores = |arm: &'static str| {
            sqlx::query("SELECT score FROM agent_memory.campaign_arm_score WHERE campaign_id = $1 AND arm = $2 ORDER BY seq")
                .bind(campaign)
                .bind(arm)
        };
        let random: Vec<f64> = scores("random")
            .fetch_all(&self.pg)
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        let agent: Vec<f64> = scores("agent")
            .fetch_all(&self.pg)
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        Ok(Some((
            Arm {
                campaign_id: campaign.to_string(),
                budget: usize::try_from(a.get::<i32, _>(0)).unwrap_or(0),
                seed: u64::from_ne_bytes(a.get::<i64, _>(1).to_ne_bytes()),
                scores: random,
            },
            agent,
        )))
    }
}

/// The predicate Gate 3 calls for a campaign candidate (RB-3.3): the arm exists and has
/// completed. The error names the arm.
///
/// # Errors
/// [`ArmError`], or the database as a string.
pub async fn arm_gate(pg: &PgPool, campaign: Uuid) -> Result<(), String> {
    let loaded = ArmStore::new(pg.clone())
        .load(campaign)
        .await
        .map_err(|e| e.to_string())?;
    gate_check(loaded.as_ref().map(|(a, _)| a)).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct ArmBody {
    pub budget: i32,
}

/// `POST /api/knowledge/campaigns/{id}/random-arm` — create the campaign's arm. Platform
/// authority; the seed is derived from the campaign id and is not a parameter.
pub async fn create_arm(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<ArmBody>,
) -> Response {
    if let Err(r) = need(&token, KNOWLEDGE_WRITE) {
        return r;
    }
    if b.budget <= 0 {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_budget",
            "budget must be positive",
        );
    }
    match ArmStore::new(state.pg.clone())
        .create(id, token.user_id(), b.budget)
        .await
    {
        Ok(seed) => (
            StatusCode::CREATED,
            Json(json!({ "campaign_id": id, "budget": b.budget, "seed": seed })),
        )
            .into_response(),
        Err(sqlx::Error::Database(d)) if d.is_unique_violation() => err(
            StatusCode::CONFLICT,
            "arm_exists",
            "this campaign already has its random arm",
        ),
        Err(e) => db(&e),
    }
}

#[derive(Deserialize)]
pub struct ScoreBody {
    pub arm: String,
    pub seq: i32,
    pub score: f64,
}

/// `POST /api/knowledge/campaigns/{id}/scores` — record one trial's sealed-metric score
/// for either arm. Platform authority.
pub async fn record_arm_score(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(b): Json<ScoreBody>,
) -> Response {
    if let Err(r) = need(&token, KNOWLEDGE_WRITE) {
        return r;
    }
    if !matches!(b.arm.as_str(), "agent" | "random") || !b.score.is_finite() {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_score",
            "arm is agent or random and the score is finite",
        );
    }
    match ArmStore::new(state.pg.clone())
        .record_score(id, &b.arm, b.seq, b.score)
        .await
    {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(e) => db(&e),
    }
}

/// `GET /api/knowledge/campaigns/{id}/compare-to-random` — both arms' sealed
/// distributions and the agent's percentile in the random arm's best-of-budget
/// distribution. A campaign with no arm, an incomplete one, or unequal budgets refuses.
pub async fn compare_random(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> Response {
    if let Err(r) = need(&token, scopes::MEMORY) {
        return r;
    }
    let loaded = match ArmStore::new(state.pg.clone()).load(id).await {
        Ok(l) => l,
        Err(e) => return db(&e),
    };
    let (arm, agent) = match &loaded {
        Some((a, g)) => (Some(a), g.as_slice()),
        None => (None, &[][..]),
    };
    match compare_to_random(agent, arm) {
        Ok(c) => Json(json!({ "campaign_id": id, "comparison": c })).into_response(),
        Err(
            e @ (ArmError::NoArm | ArmError::Incomplete { .. } | ArmError::UnequalBudget { .. }),
        ) => err(StatusCode::CONFLICT, "no_matched_arm", e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_write_scope_is_not_an_agent_scope() {
        assert!(!crate::auth::scopes::RESEARCH_SCOPES.contains(&super::KNOWLEDGE_WRITE));
        assert!(
            crate::auth::scopes::validate_agent_scopes(&[super::KNOWLEDGE_WRITE.to_string()])
                .is_err()
        );
        // A web login holds everything, which is how a person (and the platform's own
        // tooling) reaches the platform-write routes.
        assert!(crate::auth::scopes::permits(
            &["web:full".to_string()],
            super::KNOWLEDGE_WRITE
        ));
    }

    /// Every platform-write handler asks for the write scope first. Read from the source
    /// so a new handler of this kind cannot be added without the check.
    #[test]
    fn every_platform_write_handler_begins_with_the_write_scope_check() {
        let src = include_str!("knowledge_routes.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        let writers = [
            "record_move",
            "override_veto",
            "update_regime_prior",
            "create_arm",
            "record_arm_score",
        ];
        for w in writers {
            let at = prod
                .find(&format!("pub async fn {w}("))
                .unwrap_or_else(|| panic!("{w} missing"));
            let head: String = prod[at..].chars().take(420).collect();
            assert!(
                head.contains("need(&token, KNOWLEDGE_WRITE)"),
                "{w} does not begin with the write-scope check"
            );
        }
    }

    /// KM-03: one refusal, one body, and every route that is offered evidence calls it.
    #[test]
    fn a_finding_offered_as_evidence_is_refused_by_every_route_that_takes_evidence() {
        let refused = super::finding_evidence_refusal(&["fnd_01".to_string(), "job-7".to_string()])
            .expect("a finding id is refused");
        assert_eq!(
            refused.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
        assert!(super::finding_evidence_refusal(&[
            "job-7".to_string(),
            "artifact:abc".to_string()
        ])
        .is_none());
        assert!(super::finding_evidence_refusal(&[]).is_none());
        // The three routes that take evidence all go through it (read from the source so a
        // new copy of the rule, or a route that stops calling it, fails here).
        let routes = [
            ("lifecycle.rs", include_str!("routes/lifecycle.rs")),
            ("experiments.rs", include_str!("routes/experiments.rs")),
            ("knowledge_routes.rs", include_str!("knowledge_routes.rs")),
        ];
        for (name, src) in routes {
            assert!(
                src.contains("finding_evidence_refusal("),
                "{name} does not refuse a finding offered as evidence"
            );
        }
    }

    /// RB-5.9: sweeps are given `sweep_slots(...)` of the backtest slots, never the literal they
    /// were given before, and never all of them.
    #[test]
    fn sweeps_are_given_the_slots_that_leave_one_free_for_interactive_backtests() {
        let state = include_str!("state.rs");
        assert!(
            state.contains("research::refit::sweep_slots("),
            "ResearchManager is not given sweep_slots"
        );
        assert!(state.contains("backtest::manager::MAX_CONCURRENT_RUNS"));
        let slots = research::refit::sweep_slots(
            backtest::manager::MAX_CONCURRENT_RUNS,
            research::refit::sweep_max_parallel(),
        );
        assert!(
            (1..backtest::manager::MAX_CONCURRENT_RUNS).contains(&slots),
            "{slots} slots would starve an interactive backtest"
        );
    }

    #[test]
    fn a_window_spec_names_its_length() {
        assert_eq!(super::window_days("21d@1m"), 21);
        assert_eq!(super::window_days("90d@1h"), 90);
        assert_eq!(super::window_days("weird"), 21);
    }

    #[test]
    fn a_filter_that_excludes_everything_issues_no_semantic_query() {
        assert!(
            !super::should_issue_semantic_query(0, Some("momentum in btc")),
            "nothing to rank"
        );
        assert!(!super::should_issue_semantic_query(5, None), "no question");
        assert!(
            !super::should_issue_semantic_query(5, Some("   ")),
            "a blank question"
        );
        assert!(super::should_issue_semantic_query(
            5,
            Some("momentum in btc")
        ));
    }

    #[test]
    fn the_insight_handler_asks_before_it_embeds() {
        // The handler runs the metadata search and consults the decision *before* the
        // embedder is constructed. Read from the source so a refactor that embeds first fails.
        let src = include_str!("knowledge_jobs.rs");
        let h = src.split("pub async fn search_insights").nth(1).unwrap();
        let h: String = h.chars().take(3500).collect();
        let meta = h
            .find("store.search(&tenant, params.tier, limit)")
            .expect("metadata search");
        let embed = h.find("OllamaEmbedder::from_env()").expect("embedder");
        assert!(
            meta < embed,
            "the metadata filter must run before any embedding request"
        );
        assert!(h.contains("semantic_skipped"));
    }
}
