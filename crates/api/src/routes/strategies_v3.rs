//! One authoring API for people and the agent (Set U U-7.4; FEAT-007 §10;
//! ADR-0045 decision 4).
//!
//! The editor and the agent's tools call these routes and nothing else to
//! author a `tbot.strategy/3` strategy, so a person and a model get the same
//! answer to the same question:
//!
//! | route | does |
//! |---|---|
//! | `POST /api/v3/authoring/validate` | text, graph JSON or builder steps → the graph, its text and revision, or every finding |
//! | `POST /api/v3/authoring/compile` | … → the plan's summary: inputs, warm-up, model pins |
//! | `POST /api/v3/authoring/explain` | … → the strategy in plain words |
//! | `POST /api/v3/authoring/build` | builder steps (on a base, or from nothing) → the graph so far, valid or not |
//! | `POST /api/v3/strategies` | save: validate, then append a revision and move the head |
//! | `GET /api/v3/strategies/{id}` | the head revision, as graph and text |
//! | `GET /api/v3/catalogue` | operators, feature families and model contracts |
//!
//! Preview, backtest and selection runs already take a graph through their own
//! routes (`/api/agent/strategy/preview`, `/api/backtests`,
//! `/api/selections/run`); they are not served twice.

use std::collections::BTreeMap;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::strategy_def::params::ParamValues;
use domain::strategy_graph::StrategyGraph;
use serde::Deserialize;
use serde_json::{json, Value};
use strategy_engine::plan::NodeKind;
use strategy_lang::v3::{self, BuildOp, Finding, Source};

use crate::auth::BearerToken;
use crate::routes::strategies::{write_revision, RevisionWrite};
use crate::state::AppState;

/// A strategy, however it is written.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyInput {
    /// SLv3 text…
    #[serde(default)]
    pub text: Option<String>,
    /// …or graph JSON…
    #[serde(default)]
    pub graph: Option<Value>,
    /// …or a stored strategy's head…
    #[serde(default)]
    pub strategy_ref: Option<String>,
    /// …with builder steps applied to it (or, alone, from nothing).
    #[serde(default)]
    pub ops: Option<Vec<BuildOp>>,
    /// Parameter overrides; the declared defaults otherwise.
    #[serde(default)]
    pub params: Option<BTreeMap<String, Value>>,
    /// The window it will run over, in bars — checks the warm-up fits.
    #[serde(default)]
    pub window_bars: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveBody {
    pub strategy_id: String,
    /// Another strategy this one was derived from, on its first save.
    #[serde(default)]
    pub parent_strategy_id: Option<String>,
    #[serde(flatten)]
    pub input: StrategyInput,
}

fn findings_response(status: StatusCode, findings: &[Finding]) -> axum::response::Response {
    (status, Json(json!({"valid": false, "findings": findings}))).into_response()
}

fn one(code: &str, message: impl Into<String>, fix: &str) -> Vec<Finding> {
    vec![Finding {
        code: code.into(),
        at: "request".into(),
        message: message.into(),
        fix: fix.into(),
        example: None,
        see: Some("slv3#structure"),
    }]
}

async fn stored_head(state: &AppState, strategy_id: &str) -> Result<Value, Vec<Finding>> {
    let row: Option<(Value,)> =
        sqlx::query_as("SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1")
            .bind(strategy_id)
            .fetch_optional(&state.pg)
            .await
            .map_err(|e| one("storage", e.to_string(), "try again"))?;
    let Some((json,)) = row else {
        return Err(one(
            "unknown_strategy",
            format!("no strategy `{strategy_id}`"),
            "check the id; `GET /api/strategies` lists them",
        ));
    };
    Ok(json)
}

/// Validate a request's strategy. `Err` carries the status and every finding.
async fn validated(
    state: &AppState,
    input: &StrategyInput,
) -> Result<v3::Valid, (StatusCode, Vec<Finding>)> {
    let bad = |f: Vec<Finding>| (StatusCode::UNPROCESSABLE_ENTITY, f);
    let given = usize::from(input.text.is_some())
        + usize::from(input.graph.is_some())
        + usize::from(input.strategy_ref.is_some());
    if given > 1 {
        return Err(bad(one(
            "ambiguous_input",
            "give one of `text`, `graph` or `strategy_ref`",
            "send the strategy one way",
        )));
    }
    if given == 0 && input.ops.is_none() {
        return Err(bad(one(
            "no_strategy",
            "the request carries no strategy",
            "send `text`, `graph`, `strategy_ref` or builder `ops`",
        )));
    }
    let registry = backtest::models::RegistrySnapshot::load_or_empty(&state.pg).await;
    let params: ParamValues = input
        .params
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let stored = match &input.strategy_ref {
        Some(id) => Some(stored_head(state, id).await.map_err(bad)?),
        None => None,
    };
    let source = match (
        &input.ops,
        &input.text,
        input.graph.as_ref().or(stored.as_ref()),
    ) {
        (None, Some(t), _) => Source::Text(t),
        (None, None, Some(g)) => Source::Json(g),
        (Some(ops), text, graph) => {
            let base: Option<StrategyGraph> = match (text, graph) {
                (Some(t), _) => Some(v3::parse(t).map_err(|e| {
                    bad(vec![Finding {
                        code: "syntax".into(),
                        at: format!("line {}, column {}", e.line, e.col),
                        message: e.message,
                        fix: e.fix,
                        example: None,
                        see: Some("slv3#syntax"),
                    }])
                })?),
                (None, Some(g)) => Some(StrategyGraph::from_json(g).map_err(|e| {
                    bad(one(
                        "malformed",
                        e.to_string(),
                        "fix the graph before building on it",
                    ))
                })?),
                (None, None) => None,
            };
            Source::Graph(v3::build(base.as_ref(), ops).map_err(|f| bad(vec![f]))?)
        }
        (None, None, None) => unreachable!("checked above"),
    };
    v3::validate(source, &registry, &params, input.window_bars).map_err(bad)
}

fn summary(v: &v3::Valid) -> Value {
    let p = &v.plan;
    json!({
        "kind": p.kind,
        "timeframe": p.timeframe,
        "warmup_ticks": p.warmup_ticks,
        "nodes": p.nodes.len(),
        "inputs": p.inputs.iter().map(|i| json!({"id": i.id, "timeframe": i.timeframe, "indicators": i.columns})).collect::<Vec<_>>(),
        "model_pins": p.nodes.iter().filter_map(|n| match &n.kind {
            NodeKind::Model(c) => Some(json!({"node": n.id, "model": c.model, "version": c.version})),
            _ => None,
        }).collect::<Vec<_>>(),
        "triggers": p.triggers,
    })
}

/// `POST /api/v3/authoring/validate`
pub async fn validate(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(input): Json<StrategyInput>,
) -> impl IntoResponse {
    match validated(&state, &input).await {
        Ok(v) => {
            Json(json!({"valid": true, "revision": v.revision, "text": v.text, "graph": v.graph}))
                .into_response()
        }
        Err((s, f)) => findings_response(s, &f),
    }
}

/// `POST /api/v3/authoring/compile`
pub async fn compile(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(input): Json<StrategyInput>,
) -> impl IntoResponse {
    match validated(&state, &input).await {
        Ok(v) => Json(json!({"valid": true, "revision": v.revision, "plan": summary(&v)}))
            .into_response(),
        Err((s, f)) => findings_response(s, &f),
    }
}

/// `POST /api/v3/authoring/explain`
pub async fn explain(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(input): Json<StrategyInput>,
) -> impl IntoResponse {
    match validated(&state, &input).await {
        Ok(v) => Json(json!({"valid": true, "revision": v.revision, "text": v.text, "explanation": v3::explain::explain(&v.graph)})).into_response(),
        Err((s, f)) => findings_response(s, &f),
    }
}

/// `POST /api/v3/authoring/build` — the graph so far, and what is still wrong
/// with it. A half-built strategy is the normal state of a builder session, so
/// findings are reported beside the graph rather than instead of it.
pub async fn build(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(input): Json<StrategyInput>,
) -> impl IntoResponse {
    let Some(ops) = &input.ops else {
        return findings_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &one(
                "no_ops",
                "build takes builder `ops`",
                "send `ops`, and optionally a base `text`, `graph` or `strategy_ref`",
            ),
        );
    };
    let base: Option<StrategyGraph> = match (&input.text, &input.graph, &input.strategy_ref) {
        (Some(t), _, _) => match v3::parse(t) {
            Ok(g) => Some(g),
            Err(e) => {
                return findings_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &one("syntax", e.to_string(), &e.fix),
                )
            }
        },
        (None, Some(g), _) => match StrategyGraph::from_json(g) {
            Ok(g) => Some(g),
            Err(e) => {
                return findings_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &one("malformed", e.to_string(), "fix the base graph"),
                )
            }
        },
        (None, None, Some(id)) => match stored_head(&state, id).await.and_then(|j| {
            StrategyGraph::from_json(&j).map_err(|e| {
                one(
                    "malformed",
                    e.to_string(),
                    "the stored head is not a v3 graph",
                )
            })
        }) {
            Ok(g) => Some(g),
            Err(f) => return findings_response(StatusCode::UNPROCESSABLE_ENTITY, &f),
        },
        (None, None, None) => None,
    };
    let graph = match v3::build(base.as_ref(), ops) {
        Ok(g) => g,
        Err(f) => return findings_response(StatusCode::UNPROCESSABLE_ENTITY, &[f]),
    };
    let registry = backtest::models::RegistrySnapshot::load_or_empty(&state.pg).await;
    let params: ParamValues = input
        .params
        .clone()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let findings = v3::validate(
        Source::Graph(graph.clone()),
        &registry,
        &params,
        input.window_bars,
    )
    .err()
    .unwrap_or_default();
    Json(json!({
        "valid": findings.is_empty(),
        "graph": graph,
        "text": v3::print(&graph),
        "revision": graph.revision().map(|r| r.as_str().to_string()).ok(),
        "findings": findings,
    }))
    .into_response()
}

/// `POST /api/v3/strategies` — the editor's save and the agent's save, one call.
pub async fn save(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<SaveBody>,
) -> impl IntoResponse {
    let v = match validated(&state, &body.input).await {
        Ok(v) => v,
        Err((s, f)) => return findings_response(s, &f),
    };
    let def_json = match serde_json::to_value(&v.graph) {
        Ok(j) => j,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    let canonical = match v.graph.canonical_json() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    let written = match write_revision(
        &state,
        &token,
        RevisionWrite {
            strategy_id: &body.strategy_id,
            definition_version: "3",
            asset_class: &v.graph.asset_class,
            def_json: &def_json,
            revision: &v.revision,
            canonical: &canonical,
            parent_strategy_id: body.parent_strategy_id.as_deref(),
        },
    )
    .await
    {
        Ok(w) => w,
        Err(r) => return r,
    };
    (
        StatusCode::CREATED,
        Json(json!({
            "strategy_id": body.strategy_id,
            "revision_hash": v.revision,
            "actor_kind": written.actor.as_str(),
            "parent_revision": written.parent,
            "text": v.text,
        })),
    )
        .into_response()
}

/// `GET /api/v3/strategies/{id}`
pub async fn get(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let json = match stored_head(&state, &id).await {
        Ok(j) => j,
        Err(f) => return findings_response(StatusCode::NOT_FOUND, &f),
    };
    match StrategyGraph::from_json(&json) {
        Ok(g) => Json(json!({
            "strategy_id": id,
            "revision": g.revision().map(|r| r.as_str().to_string()).ok(),
            "graph": g,
            "text": v3::print(&g),
        }))
        .into_response(),
        Err(_) => findings_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            &one(
                "wrong_format",
                format!("`{id}` is a v1 definition"),
                "translate it to a v3 graph first",
            ),
        ),
    }
}

/// `GET /api/v3/catalogue`
pub async fn catalogue(State(state): State<AppState>, _token: BearerToken) -> impl IntoResponse {
    let registry = backtest::models::RegistrySnapshot::load_or_empty(&state.pg).await;
    Json(v3::catalogue::catalogue(models_of(&registry)))
}

/// Every callable model version in the registry, with its contract.
#[must_use]
pub fn models_of(registry: &backtest::models::RegistrySnapshot) -> Vec<v3::catalogue::ModelEntry> {
    registry
        .entries()
        .filter_map(|e| {
            e.contract
                .clone()
                .map(|contract| v3::catalogue::ModelEntry {
                    reference: format!("{}@{}", e.model, e.version),
                    kind: e.kind.clone(),
                    contract,
                })
        })
        .collect()
}
