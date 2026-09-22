use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use domain::instrument::AssetClass;
use domain::strategy_def::StrategyDefinition;
use strategy_runtime::compatibility::{
    default_provided_lanes, is_compatible, InstrumentCapabilities,
};
use strategy_runtime::manifest::compile_manifest;
use strategy_validator::validate;

use crate::{auth::BearerToken, state::AppState};

// ── Create ────────────────────────────────────────────────────────────────────

/// Who the platform recorded as the author of a revision (ADR-D-22).
///
/// Migration 0058: `actor_kind` "distinguishes a human author from the research
/// agent, which is the difference between a hypothesis and a sample". That
/// difference is spent downstream in the multiple-testing accounting: a run the
/// agent generated is one draw from a search, and a run a person wrote is one
/// hypothesis. Recording an agent's revision as a person's understates the
/// search width, which flatters every result derived from it — the one direction
/// a misattribution is never harmless.
///
/// There is no `Default`, and no `Unknown` variant that gets written. A caller
/// the platform cannot classify is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActorKind {
    Human,
    Agent,
}

impl ActorKind {
    /// The string migration 0058's CHECK accepts.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    /// Classify a live session by `sessions.kind`.
    ///
    /// `kind` is the platform's own principal distinction (migration 0033): a
    /// `web` row is an interactive login and a `service` row is a minted token
    /// held by a headless client — the MCP server and the internal agent runs.
    /// Scopes are the wrong discriminator here even though they look like one:
    /// the login handler inserts a session without naming `scopes`, so a web
    /// login created after migration 0038 carries `'{}'` and would classify as
    /// a service token under any "has web:full" test.
    ///
    /// `kind` has a default and **no CHECK constraint**, so an unrecognised
    /// value is reachable. It returns `None`, and the caller refuses.
    fn from_session_kind(kind: &str) -> Option<Self> {
        match kind {
            "web" => Some(Self::Human),
            "service" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// Look up the authenticated principal's kind.
///
/// Separate from the `BearerToken` extractor deliberately: widening the
/// extractor would make every route in the crate carry a field only this one
/// reads, and this one has to fail closed in a way a shared extractor cannot
/// (a route that does not record an author has nothing to refuse over).
async fn actor_kind_for(
    tx: &mut sqlx::PgConnection,
    token: &BearerToken,
) -> Result<ActorKind, String> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM sessions WHERE token = $1")
            .bind(&token.token)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

    let Some(kind) = kind else {
        return Err("the session carries no kind row".into());
    };
    ActorKind::from_session_kind(&kind)
        .ok_or_else(|| format!("session kind {kind:?} is neither 'web' nor 'service'"))
}

/// Query parameters for the create route.
#[derive(Debug, Default, Deserialize)]
pub struct CreateParams {
    /// An explicit lineage: the slug this definition was derived from, when it
    /// was derived from a **different** lineage than its own (ADR-D-21).
    ///
    /// Optional, and never required. A first version genuinely has no parent,
    /// and a required lineage field is a field that gets filled with something.
    /// The ordinary case needs nothing here: re-saving under the same slug makes
    /// the previous head the parent, because `strategy_revision` is append-only
    /// and the pointer move does not discard what it pointed at.
    pub parent_strategy_id: Option<String>,
}

/// POST /api/strategies — validate then persist a strategy definition.
pub async fn create_strategy(
    State(state): State<AppState>,
    Query(params): Query<CreateParams>,
    token: BearerToken,
    Json(def): Json<StrategyDefinition>,
) -> impl IntoResponse {
    match validate(&def) {
        Err(errors) => {
            let formatted: Vec<serde_json::Value> = errors
                .iter()
                .map(|e| json!({ "path": e.path, "message": e.message }))
                .collect();
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "validation_failed", "errors": formatted })),
            )
                .into_response()
        }
        Ok(validated) => {
            let inner = validated.into_inner();
            let def_json = match serde_json::to_value(&inner) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": e.to_string() })),
                    )
                        .into_response()
                }
            };

            // ── append the revision, then move the pointer (ADR-P6-04) ──
            //
            // This used to be `ON CONFLICT (strategy_id) DO UPDATE`, which made
            // the authoring store the last mutable thing in the evidence path:
            // a re-save meant the definition a live instance was running could
            // not be reconstructed from any record, and two runs sharing a
            // `strategy_version` were only comparable if nobody had saved in
            // between.
            //
            // Now `strategy_id` is a pointer. The revision is content-addressed
            // and append-only, so re-saving an unchanged document is a no-op
            // rather than a new row, and re-saving a changed one *adds* rather
            // than replaces.
            //
            // Both writes are one transaction: a revision with nothing pointing
            // at it is harmless, but a pointer to a revision that was rolled
            // back is a dangling head.
            let revision = match domain::strategy_def::revision::RevisionHash::of(&inner) {
                Ok(h) => h,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "canonicalisation_failed", "message": e.to_string() })),
                    )
                        .into_response()
                }
            };
            let canonical = domain::strategy_def::revision::canonical_json(&def_json);

            let mut tx = match state.pg.begin().await {
                Ok(t) => t,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": e.to_string() })),
                    )
                        .into_response()
                }
            };

            // Who is writing this (ADR-D-22). Read before anything is inserted,
            // so an unclassifiable caller costs a rolled-back transaction rather
            // than a row attributed to a guess.
            let actor = match actor_kind_for(&mut tx, &token).await {
                Ok(a) => a,
                Err(why) => {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({
                            "error": "unclassifiable_actor",
                            "message": format!(
                                "the author of a revision is recorded, not defaulted: {why}. \
                                 Sign in with a web session or use a service token."
                            ),
                        })),
                    )
                        .into_response()
                }
            };

            // The parent is whatever this strategy currently points at. A first
            // save has none; a later one records what it was edited from, which
            // is what makes the revision history a chain rather than a pile.
            //
            // **Reusing the slug is the lineage mechanism** (ADR-D-21). The
            // advice to mint `my_strat_v2` when iterating was written as though
            // the pointer move discarded the previous definition; it does not,
            // because `strategy_revision` is append-only and holds every version
            // — which is the only reason `parent_hash` is derivable at all. A
            // new slug has no head, so that advice produced exactly the NULL
            // parent it was meant to avoid.
            //
            // The error is propagated rather than `.ok().flatten()`-ed away, as
            // it used to be. A failed lookup and a slug with no history return
            // the same `None` under that idiom, and the second is written to the
            // row as "this is a first version" — an absent measurement recorded
            // as a reading, in the one column whose whole job is to say what
            // came before.
            let own_head: Option<String> = match sqlx::query_scalar(
                "SELECT head_revision FROM strategy_definitions WHERE strategy_id = $1",
            )
            .bind(&inner.strategy_id)
            .fetch_optional(&mut *tx)
            .await
            {
                Ok(v) => v.flatten(),
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "parent_lookup_failed", "message": e.to_string() })),
                    )
                        .into_response()
                }
            };

            // The case the slug rule cannot express: a definition derived from a
            // *different* lineage. Resolved to that strategy's head, and refused
            // rather than silently dropped when it names nothing — a lineage
            // that was asked for and not recorded is worse than one never
            // claimed, because the record then reads as a first version.
            let parent: Option<String> = match params.parent_strategy_id.as_deref() {
                None => own_head,
                Some(slug) => {
                    let named: Option<String> = match sqlx::query_scalar(
                        "SELECT head_revision FROM strategy_definitions WHERE strategy_id = $1",
                    )
                    .bind(slug)
                    .fetch_optional(&mut *tx)
                    .await
                    {
                        Ok(v) => v.flatten(),
                        Err(e) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({ "error": e.to_string() })),
                            )
                                .into_response()
                        }
                    };
                    let Some(named) = named else {
                        return (
                            StatusCode::UNPROCESSABLE_ENTITY,
                            Json(json!({
                                "error": "unknown_parent_strategy",
                                "message": format!(
                                    "parent_strategy_id {slug:?} has no stored revision to \
                                     descend from. Omit it if this is a first version."
                                ),
                            })),
                        )
                            .into_response();
                    };
                    // Both a slug chain and a declared fork, disagreeing. Two
                    // answers is not a lineage, so neither is recorded.
                    if let Some(head) = &own_head {
                        if head != &named {
                            return (
                                StatusCode::UNPROCESSABLE_ENTITY,
                                Json(json!({
                                    "error": "ambiguous_parent",
                                    "message": format!(
                                        "{:?} already has a head revision, which is its parent. \
                                         Pass parent_strategy_id only on the first revision of \
                                         a slug derived from another lineage.",
                                        inner.strategy_id
                                    ),
                                })),
                            )
                                .into_response();
                        }
                    }
                    Some(named)
                }
            };

            // A revision cannot be its own parent: re-saving an unchanged
            // document is a no-op, not a self-referential link.
            let parent_recorded = parent.filter(|p| p != revision.as_str());

            // Re-saving the identical document is not a new revision. The
            // content hash is the identity, so ON CONFLICT DO NOTHING is the
            // correct no-op here and the only place a `DO NOTHING` belongs in
            // this path.
            if let Err(e) = sqlx::query(
                "INSERT INTO strategy_revision (
                     revision_hash, strategy_id, parent_hash, definition_version, asset_class,
                     definition_json, definition_bytes, actor_kind, actor_id,
                     seq, prev_hash, row_hash
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,'\\x'::bytea,'\\x'::bytea)
                 ON CONFLICT (revision_hash) DO NOTHING",
            )
            .bind(revision.as_str())
            .bind(&inner.strategy_id)
            .bind(parent_recorded.clone())
            .bind(&inner.definition_version)
            .bind(&inner.asset_class)
            .bind(&def_json)
            .bind(canonical.as_bytes())
            .bind(actor.as_str())
            .bind(token.user_id().to_string())
            .execute(&mut *tx)
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "revision_insert_failed", "message": e.to_string() })),
                )
                    .into_response();
            }

            // The pointer. `definition_json` here is a cache of the head
            // revision, and 0058's trigger refuses a head belonging to another
            // strategy, so the two cannot drift into naming different documents.
            if let Err(e) = sqlx::query(
                "INSERT INTO strategy_definitions \
                   (strategy_id, definition_version, asset_class, definition_json, head_revision) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (strategy_id) DO UPDATE SET \
                   definition_version = EXCLUDED.definition_version, \
                   definition_json = EXCLUDED.definition_json, \
                   head_revision   = EXCLUDED.head_revision, \
                   updated_at      = now()",
            )
            .bind(&inner.strategy_id)
            .bind(&inner.definition_version)
            .bind(&inner.asset_class)
            .bind(&def_json)
            .bind(revision.as_str())
            .execute(&mut *tx)
            .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }

            if let Err(e) = tx.commit().await {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": e.to_string() })),
                )
                    .into_response();
            }

            // Also keep in the in-memory store so the runtime can use it
            // without an extra DB round-trip (apply-list, start).
            {
                let id = Uuid::new_v4();
                let mut store = state
                    .strategy_store
                    .lock()
                    .expect("strategy_store lock poisoned");
                store.insert(id, inner.clone());
            }

            (
                StatusCode::CREATED,
                Json(json!({
                    "id": inner.strategy_id,
                    "strategy_id": inner.strategy_id,
                    "revision_hash": revision.as_str(),
                    // Echoed so a caller can see what was recorded rather than
                    // assume it. A tool that reports "created" and nothing else
                    // cannot tell a chained revision from a fresh lineage.
                    "actor_kind": actor.as_str(),
                    "parent_revision": parent_recorded,
                })),
            )
                .into_response()
        }
    }
}

// ── List ──────────────────────────────────────────────────────────────────────

/// GET /api/strategies — list all persisted strategy definitions.
pub async fn list_strategies(
    State(state): State<AppState>,
    _token: BearerToken,
) -> impl IntoResponse {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT strategy_id FROM strategy_definitions ORDER BY created_at DESC")
            .fetch_all(&state.pg)
            .await
            .unwrap_or_default();

    let list: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(sid,)| json!({ "id": sid, "strategy_id": sid }))
        .collect();

    Json(json!({ "strategies": list }))
}

// ── Get ───────────────────────────────────────────────────────────────────────

/// GET /api/strategies/:id/config — fetch a strategy definition by strategy_id.
pub async fn get_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(strategy_id): Path<String>,
) -> impl IntoResponse {
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1")
            .bind(&strategy_id)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();

    match row {
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "id": strategy_id })),
        )
            .into_response(),
        Some((def_json,)) => match serde_json::from_value::<StrategyDefinition>(def_json) {
            Ok(def) => Json(json!({ "id": strategy_id, "definition": def })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        },
    }
}

// ── Start ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StartRequest {
    pub user_id: String,
    pub instrument_id: String,
}

/// POST /api/strategies/:id/start — initialize a strategy instance on an instrument.
pub async fn start_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(strategy_id): Path<String>,
    Json(req): Json<StartRequest>,
) -> impl IntoResponse {
    // Try the in-memory cache first, then fall back to Postgres.
    let def_opt = {
        let store = state
            .strategy_store
            .lock()
            .expect("strategy_store lock poisoned");
        store
            .values()
            .find(|d| d.strategy_id == strategy_id)
            .cloned()
    };

    let def = match def_opt {
        Some(d) => d,
        None => {
            let row: Option<(serde_json::Value,)> = sqlx::query_as(
                "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
            )
            .bind(&strategy_id)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();

            match row {
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": "not_found", "id": strategy_id })),
                    )
                        .into_response()
                }
                Some((def_json,)) => match serde_json::from_value::<StrategyDefinition>(def_json) {
                    Ok(d) => d,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": e.to_string() })),
                        )
                            .into_response()
                    }
                },
            }
        }
    };

    // Live/paper instances run typed parameters (v1.2) on their declared
    // defaults; the runtime never sees `param()`/`{{}}` references.
    let def = match domain::strategy_def::params::materialize(
        &def,
        &domain::strategy_def::ParamValues::new(),
    ) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "invalid_parameters", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    // Ensure a live data pipeline is running for this instrument before the
    // strategy starts evaluating — it needs fresh bar/trade data to operate.
    state
        .ensure_pipeline_for_instrument(&req.instrument_id)
        .await;

    // Declare it live.  This is the registration the in-process pipeline's
    // stage 3 actually reads (ADR-0038).  Until this line existed, starting a
    // strategy inserted it into `instance_manager` — which nothing on the hot
    // path ever consulted — and returned `201`, so the call reported success
    // and the strategy evaluated nothing, for ever, silently.
    let live = strategy_runtime::live::global();
    let spec = strategy_runtime::live::LiveStrategySpec {
        user_id: req.user_id.clone(),
        instrument_id: req.instrument_id.clone(),
        definition: def.clone(),
        source: strategy_runtime::live::LiveSource::Strategy {
            strategy_id: strategy_id.clone(),
        },
    };
    if let Err(e) = live.register(spec) {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "already_running", "detail": e.to_string() })),
        )
            .into_response();
    }

    // The instance manager still owns lane demand (one pipeline per lane, no
    // matter how many strategies want it).  A failure here must not leave a
    // strategy declared live with no data behind it, so it unwinds.
    let clock: Arc<dyn strategy_runtime::StrategyClock> = state.clock.clone();
    let mut manager = match state.instance_manager.lock() {
        Ok(m) => m,
        Err(_) => {
            live.deregister_all_for(&req.user_id, &req.instrument_id);
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "instance manager unavailable" })),
            )
                .into_response();
        }
    };

    match manager.initialize(&req.user_id, &req.instrument_id, def, &clock) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({
                "strategy_id": strategy_id,
                "user_id": req.user_id,
                "instrument_id": req.instrument_id,
                "status": "running",
                "live_generation": live.generation(),
            })),
        )
            .into_response(),
        Err(strategy_runtime::RuntimeError::AlreadyRunning {
            user_id,
            instrument_id,
        }) => {
            live.deregister_all_for(&user_id, &instrument_id);
            (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "already_running",
                    "user_id": user_id,
                    "instrument_id": instrument_id
                })),
            )
                .into_response()
        }
    }
}

// ── Stop ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StopRequest {
    pub user_id: String,
    pub instrument_id: String,
}

/// POST /api/strategies/:id/stop — stop a running strategy instance.
pub async fn stop_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(_id): Path<Uuid>,
    Json(req): Json<StopRequest>,
) -> impl IntoResponse {
    // Stop evaluating first, then release lane demand.  In that order a stop
    // cannot leave a window where the strategy is still live but its data feed
    // has been torn down.
    let removed = strategy_runtime::live::global()
        .deregister_all_for(&req.user_id, &req.instrument_id);

    if let Ok(mut manager) = state.instance_manager.lock() {
        manager.stop(&req.user_id, &req.instrument_id);
    } else {
        tracing::error!(
            user_id = %req.user_id,
            instrument_id = %req.instrument_id,
            "instance manager lock poisoned during stop — the strategy is deregistered \
             from live evaluation but its lane demand was not released"
        );
    }

    Json(json!({
        "user_id": req.user_id,
        "instrument_id": req.instrument_id,
        "status": "stopped",
        "deregistered": removed,
    }))
}

// ── Apply-list (P3-T03) ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApplyListParams {
    /// Instrument identifier (e.g. `"BTC-USD"`).
    pub instrument: Option<String>,
    /// Asset class slug (e.g. `"crypto_spot_cex"`).  When supplied the provided
    /// lane set is derived from the asset class; otherwise all lanes are assumed.
    pub asset_class: Option<String>,
}

/// GET /api/strategies/apply-list — returns strategies compatible with the
/// given instrument/asset-class combination.
///
/// Incompatible strategies are **omitted** from the response, never returned
/// with a flag (C-113/C-117).
pub async fn apply_list(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(params): Query<ApplyListParams>,
) -> impl IntoResponse {
    let caps = match params.asset_class.as_deref() {
        Some(slug) => {
            // Parse the slug to an AssetClass; fall back to full lane set on
            // an unknown slug so new asset classes degrade gracefully.
            let ac = slug_to_asset_class(slug);
            InstrumentCapabilities {
                provided_lanes: match ac {
                    Some(ac) => default_provided_lanes(ac),
                    None => all_lanes(),
                },
            }
        }
        None => InstrumentCapabilities {
            provided_lanes: all_lanes(),
        },
    };

    // Read definitions from Postgres so the list survives restarts.
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT strategy_id, definition_json FROM strategy_definitions ORDER BY created_at DESC",
    )
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();

    let compatible: Vec<serde_json::Value> = rows
        .into_iter()
        .filter_map(|(sid, def_json)| {
            let def = serde_json::from_value::<StrategyDefinition>(def_json).ok()?;
            let manifest = compile_manifest(&def);
            if is_compatible(&manifest, &caps) {
                Some(json!({
                    "id": sid,
                    "strategy_id": def.strategy_id,
                    "strategy_kind": manifest.strategy_kind.as_str(),
                    "evaluation_trigger": manifest.evaluation_trigger.as_str(),
                    "required_lanes": manifest.required_lanes
                        .iter()
                        .map(|dt| dt.as_key())
                        .collect::<Vec<_>>(),
                }))
            } else {
                None
            }
        })
        .collect();

    Json(json!({ "strategies": compatible }))
}

fn slug_to_asset_class(slug: &str) -> Option<AssetClass> {
    match slug {
        "crypto_spot_cex" => Some(AssetClass::CryptoSpotCex),
        "equity" => Some(AssetClass::Equity),
        "fx" => Some(AssetClass::Fx),
        "prediction_market" => Some(AssetClass::PredictionMarket),
        "option" => Some(AssetClass::Option),
        "crypto_spot_dex" => Some(AssetClass::CryptoSpotDex),
        "perpetual_swap" => Some(AssetClass::PerpetualSwap),
        "futures_expiring" => Some(AssetClass::FuturesExpiring),
        _ => None,
    }
}

fn all_lanes() -> Vec<domain::data_type::DataType> {
    use domain::data_type::DataType;
    vec![
        DataType::MarketOhlcv,
        DataType::MarketTrade,
        DataType::MarketQuote,
        DataType::MarketFundingRate,
        DataType::MarketOpenInterest,
        DataType::PredictionMarketPrice,
        DataType::DexQuote,
        DataType::SocialPost,
        DataType::WebPageSnapshot,
        DataType::NewsArticle,
    ]
}

// ── Response shape (shared by other handlers) ─────────────────────────────────

#[derive(Debug, Serialize)]
pub struct StrategyListItem {
    pub id: String,
    pub strategy_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-D-22. The route bound `actor_kind = "human"` unconditionally while
    /// migration 0058 defined that column as the thing that tells a person's
    /// hypothesis from a search the agent ran. The mapping below is the whole
    /// of the classification, asserted one principal at a time.
    #[test]
    fn a_web_session_is_a_human_and_a_service_token_is_an_agent() {
        assert_eq!(ActorKind::from_session_kind("web"), Some(ActorKind::Human));
        assert_eq!(
            ActorKind::from_session_kind("service"),
            Some(ActorKind::Agent)
        );
    }

    /// **An unclassifiable actor is never a default.**
    ///
    /// `sessions.kind` is `TEXT NOT NULL DEFAULT 'web'` with **no CHECK**, so a
    /// value outside the pair is reachable by anything that writes the row
    /// directly. The honest answer is no answer; the caller turns this into a
    /// refusal rather than picking the friendlier of the two.
    #[test]
    fn an_unrecognised_session_kind_classifies_as_nothing() {
        for kind in ["", "Web", "SERVICE", "scheduler", "import", "bot", "unknown"] {
            assert_eq!(
                ActorKind::from_session_kind(kind),
                None,
                "{kind:?} must not resolve to an author"
            );
        }
    }

    /// The two spellings are exactly the ones migration 0058's CHECK accepts,
    /// so a classified actor can never fail the insert.
    #[test]
    fn both_spellings_are_ones_the_check_constraint_accepts() {
        // 0058: CHECK (actor_kind IN ('human','agent','scheduler','import'))
        let allowed = ["human", "agent", "scheduler", "import"];
        for actor in [ActorKind::Human, ActorKind::Agent] {
            assert!(
                allowed.contains(&actor.as_str()),
                "{:?} would be refused by the column",
                actor
            );
        }
    }

    /// Scopes look like a principal discriminator and are not one.
    ///
    /// This is the trap the decision brief pointed at: `permits()` treats
    /// `web:full` as a web login, but `POST /auth/login` inserts a session
    /// without naming `scopes`, so the column takes its `'{}'` default. Migration
    /// 0038's `UPDATE … WHERE kind = 'web'` backfilled the rows that existed
    /// then and nothing keeps new ones filled. A "holds web:full ⇒ human" test
    /// would therefore record every login since as an agent.
    #[test]
    fn classification_is_by_session_kind_and_not_by_scopes() {
        let file = include_str!("strategies.rs");
        let body = file
            .split("mod tests")
            .next()
            .expect("the module above the tests");
        assert!(
            body.contains("SELECT kind FROM sessions"),
            "the author must be read from sessions.kind"
        );
        for reader in ["token.scopes", ".permits(", "scopes.contains"] {
            assert!(
                !body.contains(reader),
                "{reader} classifies by scopes, and a web login created after 0038 carries none"
            );
        }
    }

    /// No literal author survives in the insert.
    ///
    /// Written as a scan rather than a behavioural assertion because the failure
    /// it guards is textual: the bug was a hard-coded `"human"` sitting in a
    /// bind list, and it is reintroduced by typing one, not by reasoning wrongly.
    #[test]
    fn the_revision_insert_binds_no_literal_actor() {
        let file = include_str!("strategies.rs");
        let body = file.split("mod tests").next().expect("the module");
        for literal in ["\"human\"", "\"agent\"", "\"scheduler\"", "\"import\""] {
            assert!(
                !body.contains(&format!(".bind({literal})")),
                "{literal} is bound as an author literal; the actor is read, not asserted"
            );
        }
        assert!(
            body.contains(".bind(actor.as_str())"),
            "the insert must bind the classified actor"
        );
    }

    /// The refusal exists and is a refusal, not a fallback.
    #[test]
    fn an_unclassifiable_caller_is_refused_rather_than_recorded() {
        let file = include_str!("strategies.rs");
        let body = file.split("mod tests").next().expect("the module");
        assert!(body.contains("unclassifiable_actor"));
        assert!(
            body.contains("StatusCode::FORBIDDEN"),
            "the unclassifiable caller must leave with a status, not a row"
        );
    }

    /// ADR-D-21. `parent_strategy_id` is optional at the type level, which is
    /// the only place "optional" can be enforced.
    #[test]
    fn the_explicit_lineage_is_optional() {
        let params = CreateParams::default();
        assert!(
            params.parent_strategy_id.is_none(),
            "a first version genuinely has no parent"
        );
    }
}
