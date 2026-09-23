//! Falsification and feasibility routes (EXTERNAL-REVIEW-2026-09 §3.4, §4).
//!
//! Four endpoints behind the four agent tools:
//!
//! - `POST /api/falsification/kills` — refute a candidate.
//! - `POST /api/falsification/defects` — the platform itself is wrong.
//! - `POST /api/research/tradeable-frontier` — is this horizon searchable.
//! - `POST /api/research/oracle-test` — is the cost model charging what it says.
//!
//! The first two construct the sealed types in [`ledger::falsification`] before
//! touching the database, so an unfalsifiable claim is refused with a reason the
//! agent can act on rather than a constraint violation it cannot parse. The
//! database refuses it again — a direct SQL client bypasses the type and not the
//! CHECK.
//!
//! The last two do not spend a trial. That is the whole point of asking them
//! before a search rather than after it disappoints.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use backtest::costs::{empirical_frontier, oracle, ExecutionStyle, FeeBook, FeeSchedule};
use ledger::falsification::{EvaluatorDefect, Kill, KillClass, Reproduction};

use crate::{auth::BearerToken, state::AppState};

/// Horizons the frontier is measured at, in bars.
///
/// Geometric rather than linear: the curve is `√h`-shaped, so evenly spaced
/// horizons spend most of their samples on the flat part where nothing changes.
const FRONTIER_HORIZONS: &[usize] = &[
    1, 2, 5, 10, 15, 30, 60, 120, 240, 480, 960, 1440, 2880, 10_080,
];

// ── shared request shapes ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReproductionBody {
    artifact_ref: String,
    seed: u64,
    expected: String,
    observed: String,
    steps: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct KillBody {
    target_trial_id: Uuid,
    defect_class: String,
    #[serde(flatten)]
    reproduction: ReproductionBody,
}

#[derive(Debug, Deserialize)]
pub struct DefectBody {
    component: String,
    claimed_guarantee: String,
    #[serde(flatten)]
    reproduction: ReproductionBody,
}

#[derive(Debug, Deserialize)]
pub struct FrontierBody {
    instrument_id: String,
    holding_bars: usize,
    #[serde(default = "default_timeframe")]
    timeframe: String,
    venue: Option<String>,
    fee_tier: Option<String>,
    #[serde(default = "default_style")]
    execution_style: String,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct OracleBody {
    instrument_id: String,
    #[serde(default = "default_timeframe")]
    timeframe: String,
    venue: Option<String>,
    fee_tier: Option<String>,
    #[serde(default = "default_style")]
    execution_style: String,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_timeframe() -> String {
    "1m".into()
}

/// `taker_both` — the only style honest against OHLCV alone, since a maker fill
/// is contingent on the book coming to you and minute bars cannot show that.
fn default_style() -> String {
    "taker_both".into()
}

fn parse_style(s: &str) -> Option<ExecutionStyle> {
    match s {
        "taker_both" => Some(ExecutionStyle::TakerBoth),
        "maker_both" => Some(ExecutionStyle::MakerBoth),
        "maker_entry_taker_exit" => Some(ExecutionStyle::MakerEntryTakerExit),
        _ => None,
    }
}

fn parse_class(s: &str) -> Option<KillClass> {
    ledger::falsification_store::class_from_code(s)
}

/// Boxed because an `axum::Response` is large and this sits in the `Err` arm of
/// a `Result` returned on every call; an unboxed one would widen the success
/// path to the size of the failure path.
type Refusal = Box<axum::response::Response>;

fn build_reproduction(b: ReproductionBody) -> Result<Reproduction, Refusal> {
    Reproduction::replayed(b.artifact_ref, b.seed, b.expected, b.observed, b.steps).map_err(|e| {
        Box::new(
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "not_falsifiable", "message": e.to_string() })),
            )
                .into_response(),
        )
    })
}

// ── kills ────────────────────────────────────────────────────────────────────

/// POST /api/falsification/kills
pub async fn report_kill(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<KillBody>,
) -> impl IntoResponse {
    let Some(class) = parse_class(&body.defect_class) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "unknown_defect_class",
                "got": body.defect_class,
                "allowed": ["lookahead","data_artifact","below_cost_floor",
                            "below_tradeable_frontier","fill_model_exploit",
                            "duplicate_of_existing","invalid_metric"],
                "fix": "there is no class for 'underperforms' on purpose: a low score is a \
                        result, not a defect"
            })),
        )
            .into_response();
    };

    let target = body.target_trial_id;
    let reproduction = match build_reproduction(body.reproduction) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };

    let kill = Kill::reported(target.to_string(), class, reproduction);
    let tenant = token.user_id().to_string();
    match ledger::falsification_store::insert_kill(
        &state.pg,
        &tenant,
        (&tenant, "agent"),
        target,
        &kill,
    )
    .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({
                "falsification_id": id,
                "verification": "unverified",
                "score": "zero",
                "note": "filed. A kill scores nothing until somebody re-runs the reproduction \
                         and it holds — that is what stops the degenerate policy of killing \
                         everything."
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── evaluator defects ────────────────────────────────────────────────────────

/// POST /api/falsification/defects
pub async fn report_defect(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<DefectBody>,
) -> impl IntoResponse {
    if body.claimed_guarantee.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "missing_guarantee",
                "fix": "quote what this component claims to guarantee. A defect is a gap \
                        between a claim and a behaviour; without the claim there is only a \
                        behaviour, and behaviours are not defects."
            })),
        )
            .into_response();
    }

    let (component, guarantee) = (body.component, body.claimed_guarantee);
    let reproduction = match build_reproduction(body.reproduction) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };

    let defect = EvaluatorDefect::reported(component, guarantee, reproduction);
    let tenant = token.user_id().to_string();
    match ledger::falsification_store::insert_defect(
        &state.pg,
        &tenant,
        (&tenant, "agent"),
        &defect,
    )
    .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({
                "falsification_id": id,
                "verification": "unverified",
                "note": "filed. Once verified this outranks a promotion — reporting a defect in \
                         the evaluator is worth more than any strategy it would have passed."
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── the tradeable frontier ───────────────────────────────────────────────────

/// POST /api/research/tradeable-frontier
pub async fn tradeable_frontier(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<FrontierBody>,
) -> impl IntoResponse {
    let Some(style) = parse_style(&body.execution_style) else {
        return bad_style(&body.execution_style);
    };
    let schedule = resolve_schedule(body.venue.as_deref(), body.fee_tier.as_deref());
    let round_trip = schedule.round_trip_bps(style);

    let closes = match load_closes(
        &state,
        &body.instrument_id,
        &body.timeframe,
        body.start,
        body.end,
    )
    .await
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let Some(frontier) = empirical_frontier(&closes, FRONTIER_HORIZONS, round_trip) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "insufficient_bars",
                "bars": closes.len(),
                "fix": "the frontier needs enough bars to measure the shortest horizon asked \
                        for; widen the window rather than reading a frontier computed from a \
                        handful of bars"
            })),
        )
            .into_response();
    };

    let admission = frontier.admits(body.holding_bars);
    (
        StatusCode::OK,
        Json(json!({
            "instrument_id": body.instrument_id,
            "timeframe": body.timeframe,
            "fee_schedule": {
                "venue": schedule.venue,
                "tier": schedule.tier,
                "maker_bps": schedule.maker_bps,
                "taker_bps": schedule.taker_bps,
                "half_spread_bps": schedule.half_spread_bps,
            },
            "execution_style": body.execution_style,
            "requires_fill_model": style.requires_fill_model(),
            "round_trip_bps": round_trip,
            "breakeven_bars": frontier.breakeven_bars,
            "curve": frontier.curve.iter()
                .map(|(h, m)| json!({ "horizon_bars": h, "mean_abs_move_bps": m }))
                .collect::<Vec<_>>(),
            "sample_bars": frontier.sample_bars,
            "admission": admission,
        })),
    )
        .into_response()
}

// ── the oracle test ──────────────────────────────────────────────────────────

/// POST /api/research/oracle-test
pub async fn oracle_test(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<OracleBody>,
) -> impl IntoResponse {
    let Some(style) = parse_style(&body.execution_style) else {
        return bad_style(&body.execution_style);
    };
    let schedule = resolve_schedule(body.venue.as_deref(), body.fee_tier.as_deref());
    let round_trip = schedule.round_trip_bps(style);

    let closes = match load_closes(
        &state,
        &body.instrument_id,
        &body.timeframe,
        body.start,
        body.end,
    )
    .await
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let Some(o) = oracle(&closes, round_trip) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "insufficient_bars", "bars": closes.len() })),
        )
            .into_response();
    };

    let sane = o.cost_model_is_sane();
    (
        StatusCode::OK,
        Json(json!({
            "instrument_id": body.instrument_id,
            "timeframe": body.timeframe,
            "round_trip_bps": round_trip,
            "always_in_gross_bps": o.always_in_gross_bps,
            "always_in_net_bps": o.always_in_net_bps,
            "selective_net_bps": o.selective_net_bps,
            "selective_trade_fraction": o.selective_trade_fraction,
            "sample_bars": o.sample_bars,
            "cost_model_is_sane": sane,
            "verdict": if sane {
                "The cost model passes its own test: a perfect-foresight strategy trading every \
                 bar loses money at this horizon, which is what the algebra requires."
            } else {
                "FAILURE. A perfect-foresight strategy trading every bar is PROFITABLE at this \
                 horizon. It can earn at most the mean absolute move and must pay the round trip \
                 on every flip, so this means the cost model is not charging what it claims. \
                 Every backtest that used it is suspect. Report this with \
                 report_evaluator_defect."
            },
            "selectivity_note": format!(
                "A selective oracle — one that trades only when it knows the move clears costs \
                 — could trade on {:.3}% of bars. With perfect foresight that is the ceiling on \
                 how often any strategy can profitably trade at this horizon.",
                o.selective_trade_fraction * 100.0
            ),
        })),
    )
        .into_response()
}

// ── reading the ledger ───────────────────────────────────────────────────────

/// GET /api/falsification — the tenant's reports, newest first.
///
/// Returns the score the **database** assigned, read from
/// `mlops.falsification_scored` rather than recomputed here. If the SQL function
/// and the Rust type ever disagree, a surface that recomputed would hide it.
pub async fn list_reports(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let mut tx = match ledger::pg::tenant_tx(&state.pg, &tenant).await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let rows = sqlx::query(
        "SELECT falsification_id, report_kind, target_trial_id, component, claimed_guarantee,
                defect_class, artifact_ref, seed, expected, observed, steps,
                verification, verified_by, regression_test, reported_by, reporter_kind,
                reported_at, score, is_discharged
           FROM mlops.falsification_scored
          WHERE tenant_id = $1
          ORDER BY reported_at DESC
          LIMIT 200",
    )
    .bind(&tenant)
    .fetch_all(&mut *tx)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let reports: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            json!({
                "falsification_id": r.get::<Uuid, _>("falsification_id"),
                "report_kind":      r.get::<String, _>("report_kind"),
                "target_trial_id":  r.get::<Option<Uuid>, _>("target_trial_id"),
                "component":        r.get::<Option<String>, _>("component"),
                "claimed_guarantee": r.get::<Option<String>, _>("claimed_guarantee"),
                "defect_class":     r.get::<Option<String>, _>("defect_class"),
                "artifact_ref":     r.get::<String, _>("artifact_ref"),
                "seed":             r.get::<i64, _>("seed"),
                "expected":         r.get::<String, _>("expected"),
                "observed":         r.get::<String, _>("observed"),
                "steps":            r.get::<Vec<String>, _>("steps"),
                "verification":     r.get::<String, _>("verification"),
                "verified_by":      r.get::<Option<String>, _>("verified_by"),
                "regression_test":  r.get::<Option<String>, _>("regression_test"),
                "reported_by":      r.get::<String, _>("reported_by"),
                "reporter_kind":    r.get::<String, _>("reporter_kind"),
                "reported_at":      r.get::<chrono::DateTime<chrono::Utc>, _>("reported_at"),
                "score":            r.get::<i32, _>("score"),
                "is_discharged":    r.get::<bool, _>("is_discharged"),
            })
        })
        .collect();

    // Counted separately because they mean different things. A verified defect
    // with no regression test is the work queue; an unverified report is a claim
    // nobody has checked, and rolling the two together would let the queue look
    // empty while it is merely unexamined.
    let undischarged = reports
        .iter()
        .filter(|r| r["report_kind"] == "evaluator_defect" && r["is_discharged"] == false)
        .count();
    let unverified = reports
        .iter()
        .filter(|r| r["verification"] == "unverified")
        .count();

    (
        StatusCode::OK,
        Json(json!({
            "reports": reports,
            "counts": {
                "total": reports.len(),
                "unverified": unverified,
                "undischarged_defects": undischarged,
            },
        })),
    )
        .into_response()
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn bad_style(got: &str) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({
            "error": "unknown_execution_style",
            "got": got,
            "allowed": ["taker_both", "maker_both", "maker_entry_taker_exit"],
        })),
    )
        .into_response()
}

/// The fee schedule to price against.
///
/// Falls back to the platform's configured default tier rather than to zero.
/// A cost model that defaults to free is one that eventually reports a free
/// strategy, and the direction of that error always flatters the candidate.
fn resolve_schedule(venue: Option<&str>, tier: Option<&str>) -> FeeSchedule {
    FeeBook::platform_default().resolve(venue, tier)
}

/// Point-in-time closes, clipped the same way `/api/data/bars` clips them.
async fn load_closes(
    state: &AppState,
    instrument_id: &str,
    timeframe: &str,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<f64>, axum::response::Response> {
    let Some(tf) =
        <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(timeframe)
    else {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "unknown_timeframe", "got": timeframe })),
        )
            .into_response());
    };

    let end = end.unwrap_or_else(chrono::Utc::now);
    let start = start.unwrap_or(end - chrono::Duration::days(30));
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(end);

    match store.load_bars(instrument_id, tf, start, end).await {
        // Decimal to f64 once, here, rather than in three call sites. The loss
        // is irrelevant at bp scale and the alternative is a Decimal
        // implementation of every statistic in `costs`.
        Ok(bars) => Ok(bars
            .into_iter()
            .filter_map(|b| b.close.to_string().parse::<f64>().ok())
            .collect()),
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "bar_load_failed", "message": e.to_string() })),
        )
            .into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_execution_style_parses_and_nothing_else_does() {
        assert_eq!(parse_style("taker_both"), Some(ExecutionStyle::TakerBoth));
        assert_eq!(parse_style("maker_both"), Some(ExecutionStyle::MakerBoth));
        assert_eq!(
            parse_style("maker_entry_taker_exit"),
            Some(ExecutionStyle::MakerEntryTakerExit)
        );
        assert_eq!(parse_style("free"), None);
        assert_eq!(parse_style(""), None);
    }

    /// The honest default. A request that omits the style must not silently
    /// price against maker fills it cannot demonstrate.
    #[test]
    fn the_default_style_is_the_one_ohlcv_can_support() {
        assert_eq!(
            parse_style(&default_style()),
            Some(ExecutionStyle::TakerBoth)
        );
        assert!(!ExecutionStyle::TakerBoth.requires_fill_model());
    }

    #[test]
    fn every_kill_class_parses_and_underperformance_does_not() {
        for c in [
            "lookahead",
            "data_artifact",
            "below_cost_floor",
            "below_tradeable_frontier",
            "fill_model_exploit",
            "duplicate_of_existing",
            "invalid_metric",
        ] {
            assert!(parse_class(c).is_some(), "{c}");
        }
        assert!(parse_class("underperforms").is_none());
        assert!(parse_class("low_sharpe").is_none());
    }

    /// Geometric spacing: the curve is √h-shaped, so linear horizons would spend
    /// most of their samples where nothing changes.
    #[test]
    fn the_frontier_horizons_are_ascending_and_span_a_minute_to_a_week() {
        assert!(FRONTIER_HORIZONS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(FRONTIER_HORIZONS[0], 1);
        assert_eq!(
            *FRONTIER_HORIZONS.last().unwrap(),
            10_080,
            "one week of minutes"
        );
    }

    /// The pre-flight the agent sees. A claim that refutes nothing is refused
    /// with a reason, not a constraint violation.
    #[test]
    fn a_reproduction_that_refutes_nothing_is_rejected_before_the_database() {
        let body = ReproductionBody {
            artifact_ref: "run_1".into(),
            seed: 1,
            expected: "sharpe 0.4".into(),
            observed: "sharpe 0.4".into(),
            steps: vec!["look".into()],
        };
        assert!(build_reproduction(body).is_err());
    }

    #[test]
    fn a_complete_reproduction_builds() {
        let body = ReproductionBody {
            artifact_ref: "run_1".into(),
            seed: 7,
            expected: "unchanged under a one-bar shift".into(),
            observed: "inverts".into(),
            steps: vec!["shift".into(), "compare".into()],
        };
        let r = build_reproduction(body).expect("valid");
        assert_eq!(r.seed(), 7);
        assert_eq!(r.steps().len(), 2);
    }
}
