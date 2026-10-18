//! `preview_signals` — what a strategy would have done, without buying a trial
//! (AGENT-011 §6; Set S task S-5.4).
//!
//! # Why this exists
//!
//! Before this, the only way to find out whether a definition fires at all was
//! to run a backtest, and a backtest costs a trial: a ledger row, a draw against
//! the family's `NEff`, a permanent entry in the multiple-testing arithmetic. So
//! the cheapest question a strategy author asks — *does this thing ever
//! trigger?* — was priced the same as the most expensive one, and the model paid
//! for typos.
//!
//! This answers that question from bars alone. It returns no P&L, no Sharpe and
//! no fills, and that is not a limitation to be lifted later — it is the reason
//! the tool can be free. A preview that reported a return would be a backtest
//! with the ledger switched off.
//!
//! # The window rule, which is the whole security story
//!
//! A tool that reads bars and spends no trial is the cheapest route around the
//! honesty stack in the platform, so the window is bounded twice:
//!
//! * **The research cutoff** clips, exactly as `GET /api/data/bars` clips
//!   (DA-03). A window ending past the horizon comes back shorter, with
//!   `cutoff_applied` saying so.
//! * **The sealed holdout refuses.** A window reaching into the holdout is
//!   rejected by name, never quietly truncated.
//!
//! Those two are different on purpose, and the difference is worth stating
//! because `Project::clip`'s own documentation argues the other way: refusing is
//! worse than clipping, it says, because it teaches nothing and invites the
//! caller to probe for the boundary. That is right about the cutoff and wrong
//! about the holdout, for two reasons.
//!
//! First, a clipped preview and the backtest that follows it would disagree —
//! the preview would report the entries from the window it was allowed, the
//! backtest would refuse the same request outright, and a model that had learned
//! the preview's number would conclude the tool lies. Agreement with the
//! simulator is the only property that makes a preview worth having.
//!
//! Second, the probing argument does not apply here: the refusal *states* the
//! boundary rather than hiding it. There is nothing to discover by bisection,
//! because the answer names the date.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::projects::{Project, ProjectError, ProjectStore};
use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewBody {
    /// The definition to preview. Not a stored strategy id: the point is to look
    /// at a draft, and requiring it to be saved first would make the cheap
    /// question cost a write.
    pub definition: Value,
    pub instrument_id: String,
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    /// For a `tbot.strategy/3` graph: run against the simulated venue (the
    /// default — decisions match a backtest exactly) or record orders without
    /// executing them (`intent_only`). Ignored for v1 definitions.
    #[serde(default)]
    pub venue: backtest::preview::PreviewVenue,
    /// Deliberately corrupted inputs (S-3.10, `future_perturbation_test`): one
    /// entry per feature column. Empty means an ordinary preview. v1 definitions
    /// only; a v3 graph with perturbations is refused rather than run unperturbed,
    /// because an unperturbed run wearing a perturbed label diffs to "no
    /// divergence".
    #[serde(default)]
    pub perturbations: Vec<PerturbationIn>,
}

/// The wire form of one [`backtest::perturb::Perturbation`].
#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum PerturbationIn {
    ShiftColumn {
        column: String,
        shift_bars: i32,
    },
    MaskColumn {
        column: String,
    },
    JitterColumn {
        column: String,
        perturbation_pct: f64,
        #[serde(default)]
        seed: u64,
    },
}

impl From<PerturbationIn> for backtest::perturb::Perturbation {
    fn from(p: PerturbationIn) -> Self {
        match p {
            PerturbationIn::ShiftColumn { column, shift_bars } => {
                Self::ShiftColumn { column, shift_bars }
            }
            PerturbationIn::MaskColumn { column } => Self::MaskColumn { column },
            PerturbationIn::JitterColumn {
                column,
                perturbation_pct,
                seed,
            } => Self::JitterColumn {
                column,
                perturbation_pct,
                seed,
            },
        }
    }
}

fn default_timeframe() -> String {
    "1m".into()
}

/// Why a window may not be previewed.
///
/// Separated from the handler so the rule can be tested without a database, a
/// ClickHouse or a definition — see the tests at the foot of this file. A
/// security property that can only be exercised through a live stack is a
/// security property nobody runs.
#[derive(Debug, PartialEq)]
pub enum WindowVerdict {
    /// Preview this window, ending here. `cutoff_applied` is `Some` when the
    /// research cutoff shortened it.
    Allowed {
        effective_end: DateTime<Utc>,
        cutoff_applied: Option<DateTime<Utc>>,
    },
    /// The window reaches into the sealed holdout.
    HoldoutOverlap {
        /// The first instant that belongs to the holdout.
        sealed_from: DateTime<Utc>,
    },
    /// Nothing is left to look at.
    Empty { effective_end: DateTime<Utc> },
}

/// The window rule.
///
/// For a research project, everything at or after the research cutoff is the
/// sealed holdout: `ProjectStore::create_research` derives the cutoff as
/// `now − holdout_len` precisely so that the two are the same boundary. So a
/// request whose `end` passes the cutoff is a request to look at held-out bars,
/// and it is refused rather than shortened.
///
/// The Desk has no cutoff and therefore no holdout — it reads up to now and
/// cannot make a confirmatory claim at all (`allow_confirmatory`). Its windows
/// are clipped to now and never refused, because there is no sealed partition to
/// overlap.
#[must_use]
pub fn judge_window(project: &Project, start: DateTime<Utc>, end: DateTime<Utc>) -> WindowVerdict {
    if let Some(cutoff) = project.research_cutoff {
        if end > cutoff {
            return WindowVerdict::HoldoutOverlap {
                sealed_from: cutoff,
            };
        }
    }
    let (effective_end, cutoff_applied) = project.clip(end);
    if start >= effective_end {
        return WindowVerdict::Empty { effective_end };
    }
    WindowVerdict::Allowed {
        effective_end,
        cutoff_applied,
    }
}

/// The fewest bars the preview will speak about, mirrored from
/// `backtest::preview` so the refusal can name a number before any query runs.
const MIN_BARS: usize = 30;

/// POST /api/agent/strategy/preview
pub async fn preview(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<PreviewBody>,
) -> impl IntoResponse {
    let store = ProjectStore::new(state.pg.clone());
    let project = match match body.project_id {
        Some(id) => store.get(id).await.and_then(|p| {
            if p.user_id == token.user_id() {
                Ok(p)
            } else {
                Err(ProjectError::NotFound(id.to_string()))
            }
        }),
        None => store.desk(token.user_id()).await,
    } {
        Ok(p) => p,
        Err(e) => return project_error(&e),
    };

    // A v3 graph previews as a short backtest through the one strategy engine
    // (Set U U-3.5); the window rule below applies to it unchanged.
    if backtest::runner::is_v3(&body.definition) {
        if !body.perturbations.is_empty() {
            return unprocessable(
                "perturbations_unsupported",
                "perturbations apply to v1 definitions only; a v3 preview would have run                  unperturbed and reported no divergence",
            );
        }
        return preview_v3(&state, &project, body).await;
    }

    let definition: StrategyDefinition = match serde_json::from_value(body.definition) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "invalid_definition",
                    "message": e.to_string(),
                    "fix": "the preview parses the same document the backtest does; \
                            fix the definition and the backtest would have rejected it too",
                })),
            )
                .into_response()
        }
    };

    // Parameters at their defaults, as a backtest runs them (`sim_executor`): the
    // preview reads feature names as written, so a swept `ema_{{fast}}` reached the
    // engine unresolved and the first draft the platform handed a model could not
    // be previewed (S-5.15).
    let definition =
        match domain::strategy_def::params::materialize(&definition, &Default::default()) {
            Ok(d) => d,
            Err(e) => return unprocessable("params_do_not_materialise", e.to_string()),
        };

    let timeframe = match <Timeframe as backtest::types::TimeframeExt>::from_key(&body.timeframe) {
        Some(tf) => tf,
        None => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "unknown_timeframe",
                    "timeframe": body.timeframe,
                })),
            )
                .into_response()
        }
    };

    let (effective_end, cutoff_applied) = match judge_window(&project, body.start, body.end) {
        WindowVerdict::Allowed {
            effective_end,
            cutoff_applied,
        } => (effective_end, cutoff_applied),
        WindowVerdict::HoldoutOverlap { sealed_from } => {
            // Named, not shortened. See the module docs: a silently clipped
            // preview would disagree with the backtest that follows it.
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "holdout_overlap",
                    "partition": "sealed_holdout",
                    "sealed_from": sealed_from,
                    "requested_end": body.end,
                    "message": format!(
                        "the window ends {} and this project's sealed holdout begins {}. \
                         The preview is refused rather than shortened: a truncated preview \
                         would disagree with the backtest you run next.",
                        body.end, sealed_from
                    ),
                    "fix": format!("request end <= {sealed_from}"),
                })),
            )
                .into_response();
        }
        WindowVerdict::Empty { effective_end } => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "empty_window",
                    "message": format!(
                        "start {} is at or after the effective end {effective_end}",
                        body.start
                    ),
                    "cutoff_applied": cutoff_applied_of(&project, body.end),
                })),
            )
                .into_response()
        }
    };

    // The same point-in-time read every other path uses. `as_of` is the
    // effective end, so a late correction to a bar inside the window is invisible
    // here exactly as it is to the backtest.
    let bar_store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(effective_end);
    let bars = match bar_store
        .load_bars(&body.instrument_id, timeframe, body.start, effective_end)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "bars_unavailable", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    if bars.len() < MIN_BARS {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "window_too_short",
                "have": bars.len(),
                "need": MIN_BARS,
                "message": "below this the warm-up dominates every answer, and \
                            \"it never fired\" would be about the window rather than \
                            about the strategy",
            })),
        )
            .into_response();
    }

    let previewed = if body.perturbations.is_empty() {
        backtest::preview::preview_signals(&definition, &bars, timeframe)
    } else {
        // Validated against the columns the strategy actually reads: a
        // perturbation of a column nothing reads cannot diverge, and that
        // negative would be about the request rather than the strategy.
        let known: Vec<String> =
            match backtest::requirements::derive_requirements(&definition, timeframe) {
                Ok(r) => r.features.iter().map(|f| f.name.clone()).collect(),
                Err(e) => return unprocessable("preview_failed", e.to_string()),
            };
        let items: Vec<backtest::perturb::Perturbation> =
            body.perturbations.into_iter().map(Into::into).collect();
        let set = match backtest::perturb::PerturbationSet::validate(items, &known, bars.len()) {
            Ok(s) => s,
            Err(e) => return unprocessable("invalid_perturbation", e.to_string()),
        };
        backtest::preview::preview_signals_perturbed(&definition, &bars, timeframe, &set)
    };
    match previewed {
        Ok(p) => {
            let mut out = serde_json::to_value(&p).unwrap_or_else(|_| json!({}));
            out["window"] = json!({
                "start": body.start,
                "end": effective_end,
                "as_of": effective_end,
                "cutoff_applied": cutoff_applied,
                "instrument_id": body.instrument_id,
                "timeframe": body.timeframe,
            });
            // Said in the answer, not only in the docs. A model reading a
            // preview needs to know what it did *not* buy.
            out["spends_trial"] = json!(false);
            out["note"] = json!(
                "signals only — no fills, no costs, no P&L, and no ledger row. \
                 The entry count and entry bars agree with the simulator; \
                 everything a backtest adds is priced in a trial."
            );
            Json(out).into_response()
        }
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "preview_failed",
                "message": e.to_string(),
            })),
        )
            .into_response(),
    }
}

fn unprocessable(error: &str, message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": error, "message": message.into() })),
    )
        .into_response()
}

/// The v3 preview: the backtest runner over the window, decisions only.
///
/// The same window rule as v1 — the cutoff clips, the holdout refuses — and the
/// same point-in-time bar read. Warm-up history before `start` is loaded so the
/// window's first decision is not about cold indicators; it ends at the same
/// `as_of`, so nothing past the effective end is read.
#[allow(clippy::too_many_lines)]
async fn preview_v3(
    state: &AppState,
    project: &Project,
    body: PreviewBody,
) -> axum::response::Response {
    use strategy_engine::plan::PRIMARY;
    let graph: domain::strategy_graph::StrategyGraph = match serde_json::from_value(body.definition)
    {
        Ok(g) => g,
        Err(e) => return unprocessable("invalid_definition", e.to_string()),
    };
    let plan = match backtest::runner::compile_for_run(
        &graph,
        &backtest::run::ParamMap::new(),
        &backtest::models::RegistrySnapshot::load_or_empty(&state.pg).await,
    ) {
        Ok(p) => p,
        Err(message) => return unprocessable("invalid_strategy", message),
    };
    if plan.timeframe.key() != body.timeframe {
        return unprocessable(
            "timeframe_mismatch",
            format!(
                "the strategy decides on {} bars; the preview asked for {}",
                plan.timeframe.key(),
                body.timeframe
            ),
        );
    }
    let (effective_end, cutoff_applied) = match judge_window(project, body.start, body.end) {
        WindowVerdict::Allowed {
            effective_end,
            cutoff_applied,
        } => (effective_end, cutoff_applied),
        WindowVerdict::HoldoutOverlap { sealed_from } => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "holdout_overlap",
                    "partition": "sealed_holdout",
                    "sealed_from": sealed_from,
                    "requested_end": body.end,
                    "fix": format!("request end <= {sealed_from}"),
                })),
            )
                .into_response();
        }
        WindowVerdict::Empty { effective_end } => {
            return unprocessable(
                "empty_window",
                format!(
                    "start {} is at or after the effective end {effective_end}",
                    body.start
                ),
            )
        }
    };
    let asset_class: domain::instrument::AssetClass =
        match serde_json::from_value(serde_json::Value::String(graph.asset_class.clone())) {
            Ok(a) => a,
            Err(_) => return unprocessable("unknown_asset_class", graph.asset_class.clone()),
        };
    let meta =
        match storage::postgres::instruments::fetch_by_id(&state.pg, &body.instrument_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                return unprocessable(
                    "instrument_metadata_missing",
                    format!(
                    "no instrument metadata for '{}': orders cannot be sized or rounded without \
                     its lot and tick, here as in a backtest",
                    body.instrument_id
                ),
                )
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "internal", "message": e.to_string() })),
                )
                    .into_response()
            }
        };
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(effective_end);
    let mut bars = Vec::with_capacity(plan.inputs.len());
    for (i, need) in plan.required_history() {
        let input = &plan.inputs[i];
        let id = match &input.instrument {
            domain::strategy_graph::InstrumentRef::Bound => body.instrument_id.clone(),
            domain::strategy_graph::InstrumentRef::Fixed { instrument } => instrument.clone(),
        };
        let lead = chrono::Duration::seconds(
            i64::from(need.saturating_add(1)) * i64::from(input.timeframe.seconds()),
        );
        match store
            .load_bars(
                &id,
                input.timeframe.timeframe(),
                body.start - lead,
                effective_end,
            )
            .await
        {
            Ok(b) if !b.is_empty() => bars.push(b),
            Ok(_) => {
                return unprocessable(
                    "bars_unavailable",
                    format!(
                        "no {} bars stored for {id} in the window",
                        input.timeframe.key()
                    ),
                )
            }
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": "bars_unavailable", "message": e.to_string() })),
                )
                    .into_response()
            }
        }
    }
    let in_window = bars[PRIMARY]
        .iter()
        .filter(|b| b.ts_ns > body.start.timestamp_nanos_opt().unwrap_or(0))
        .count();
    if in_window < MIN_BARS {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "window_too_short", "have": in_window, "need": MIN_BARS })),
        )
            .into_response();
    }
    // External rows the plan's macro and fundamentals indicators read (P-3.6),
    // as the store stood at the preview's horizon.
    let external = match backtest::runner::load_external(
        &state.pg,
        &state.clickhouse_url,
        &plan,
        body.start,
        effective_end,
    )
    .await
    {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "external_inputs_unavailable", "message": e })),
            )
                .into_response()
        }
    };
    let run = backtest::runner::V3Run {
        plan: std::sync::Arc::new(plan),
        instrument: body.instrument_id.clone(),
        asset_class,
        bars,
        start_ns: body.start.timestamp_nanos_opt().unwrap_or(0),
        spec: strategy_engine::InstrumentSpec {
            instrument: body.instrument_id.clone(),
            lot_size: meta.lot_size,
            tick_size: meta.tick_size,
            min_qty: meta.lot_size,
            caps: strategy_engine::VenueCaps::all(),
        },
        instrument_limits: risk::InstrumentLimits {
            tick_size: meta.tick_size,
            lot_size: meta.lot_size,
        },
        limits: risk::GlobalRiskLimits::default(),
        initial_balance: rust_decimal::Decimal::from(10_000),
        fill_policy: execution::sim_venue::FillPolicy::NextBarOpen,
        strategy_id: graph.name.clone(),
        sims: execution::paper::SimulatorSet::realistic(),
        external,
    };
    let venue = body.venue;
    let outcome = tokio::task::spawn_blocking(move || {
        backtest::preview::preview_v3(run, venue, &mut strategy_engine::env::NoModels)
    })
    .await;
    match outcome {
        Ok(Ok((preview, _traces))) => {
            let mut out = serde_json::to_value(&preview).unwrap_or_else(|_| json!({}));
            out["engine"] = json!("strategy-engine/v3");
            out["window"] = json!({
                "start": body.start,
                "end": effective_end,
                "as_of": effective_end,
                "cutoff_applied": cutoff_applied,
                "instrument_id": body.instrument_id,
                "timeframe": body.timeframe,
            });
            out["spends_trial"] = json!(false);
            out["note"] = json!(
                "decisions only — the backtest's own loop over this window, with no fills, \
                 costs or P&L reported and no ledger row. A backtest over the same window \
                 makes exactly these decisions."
            );
            Json(out).into_response()
        }
        Ok(Err(e)) => unprocessable("preview_failed", e.to_string()),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "internal", "message": e.to_string() })),
        )
            .into_response(),
    }
}

fn cutoff_applied_of(project: &Project, requested_end: DateTime<Utc>) -> Option<DateTime<Utc>> {
    project.clip(requested_end).1
}

fn project_error(e: &ProjectError) -> axum::response::Response {
    let (status, code) = match e {
        ProjectError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        ProjectError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
        ProjectError::LiveDataDeskOnly(_) => (StatusCode::FORBIDDEN, "live_data_desk_only"),
        ProjectError::DeskExploratoryOnly(_) => (StatusCode::FORBIDDEN, "desk_exploratory_only"),
        ProjectError::Sqlx(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    (
        status,
        Json(json!({ "error": code, "message": e.to_string() })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectKind;
    use chrono::Duration;

    fn research(cutoff: DateTime<Utc>) -> Project {
        Project {
            project_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            kind: ProjectKind::Research,
            name: "p".into(),
            goal: None,
            instruments: vec![],
            research_cutoff: Some(cutoff),
            status: "active".into(),
            created_at: Utc::now(),
        }
    }

    fn desk() -> Project {
        Project {
            project_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            kind: ProjectKind::Desk,
            name: "desk".into(),
            goal: None,
            instruments: vec![],
            research_cutoff: None,
            status: "active".into(),
            created_at: Utc::now(),
        }
    }

    /// The property the whole route exists to hold.
    ///
    /// A free tool that reads bars is the cheapest way around the honesty stack,
    /// so a window reaching past the cutoff must not come back as a shorter
    /// answer that looks like a real one.
    #[test]
    fn a_window_reaching_into_the_holdout_is_refused_not_shortened() {
        let cutoff = Utc::now() - Duration::days(90);
        let p = research(cutoff);
        let verdict = judge_window(&p, cutoff - Duration::days(30), cutoff + Duration::days(1));
        assert_eq!(
            verdict,
            WindowVerdict::HoldoutOverlap {
                sealed_from: cutoff
            },
            "a preview that clipped here would report entries from a window the backtest \
             would refuse, and the two answers would disagree"
        );
    }

    /// One second past is past. Boundaries are where this kind of rule fails.
    #[test]
    fn the_boundary_is_the_cutoff_itself() {
        let cutoff = Utc::now() - Duration::days(90);
        let p = research(cutoff);

        // Ending exactly at the cutoff is inside: the holdout begins there.
        assert!(matches!(
            judge_window(&p, cutoff - Duration::days(30), cutoff),
            WindowVerdict::Allowed { .. }
        ));
        // One second later is not.
        assert!(matches!(
            judge_window(
                &p,
                cutoff - Duration::days(30),
                cutoff + Duration::seconds(1)
            ),
            WindowVerdict::HoldoutOverlap { .. }
        ));
    }

    /// The refusal carries the date, which is why it does not invite probing.
    #[test]
    fn the_refusal_names_the_partition_boundary() {
        let cutoff = Utc::now() - Duration::days(90);
        match judge_window(&research(cutoff), cutoff - Duration::days(1), Utc::now()) {
            WindowVerdict::HoldoutOverlap { sealed_from } => assert_eq!(sealed_from, cutoff),
            other => panic!("expected a refusal naming the boundary, got {other:?}"),
        }
    }

    /// The Desk has no holdout to overlap, so it is clipped rather than refused —
    /// and it cannot make a confirmatory claim in the first place.
    #[test]
    fn the_desk_is_clipped_and_never_refused() {
        let p = desk();
        let verdict = judge_window(
            &p,
            Utc::now() - Duration::days(30),
            Utc::now() + Duration::days(7),
        );
        match verdict {
            WindowVerdict::Allowed {
                effective_end,
                cutoff_applied,
            } => {
                assert!(effective_end <= Utc::now() + Duration::seconds(1));
                assert!(
                    cutoff_applied.is_none(),
                    "the Desk clips to now, and calling that a research cutoff would be a \
                     different claim"
                );
            }
            other => panic!("the Desk has no sealed partition: {other:?}"),
        }
    }

    /// A window entirely inside the research period is untouched.
    #[test]
    fn a_window_inside_the_research_period_is_allowed_whole() {
        let cutoff = Utc::now() - Duration::days(90);
        let end = cutoff - Duration::days(10);
        match judge_window(&research(cutoff), cutoff - Duration::days(40), end) {
            WindowVerdict::Allowed {
                effective_end,
                cutoff_applied,
            } => {
                assert_eq!(effective_end, end);
                assert_eq!(cutoff_applied, None);
            }
            other => panic!("expected the whole window: {other:?}"),
        }
    }
}
