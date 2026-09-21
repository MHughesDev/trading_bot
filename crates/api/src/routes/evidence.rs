//! Evidence routes behind the forensic tools (ADR-P7-05).
//!
//! These exist because the falsification sprint shipped `report_kill` with a
//! defect vocabulary the agent had no way to gather evidence for. Each route
//! answers one defect class with a number, and none of them returns a verdict —
//! a concentration of 94% is a fact, and whether it is an artifact depends on
//! whether those bars are real, which is a second question with a second route.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;

use backtest::forensics::{self, BarSlice, TradeSlice};

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct TradesBody {
    run_id: String,
    #[serde(default = "default_top_n")]
    top_n: usize,
}

const fn default_top_n() -> usize {
    6
}

/// POST /api/evidence/trades — the trade list, with its concentration measured.
///
/// Returns both together on purpose. A trade list alone invites the agent to
/// eyeball it; the concentration is the number that makes "94% of P&L is six
/// bars" writable in a kill report.
pub async fn trades(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<TradesBody>,
) -> impl IntoResponse {
    // The run store this reads is `backtest_jobs`. `0026_backtest_runs.sql`
    // renamed the job tracker created by `0011` to that name and gave the freed
    // name to the Set-J content-addressed store, which has `run_id TEXT` /
    // `result_json` and no `user_id` — and which nothing in this workspace
    // writes to (`backtest::run::store` is in-memory only). So the table with
    // run results in it is `backtest_jobs`, and it is the one `BacktestManager`
    // persists to and `list_backtests` reads back.
    let Some(run_uuid) = body.run_id.parse::<uuid::Uuid>().ok() else {
        // Not "no such run". A caller holding a Set-J `sha256:…` id or a typo
        // gets told which id space this route reads, rather than being told the
        // run does not exist — which would be a claim about the database that
        // this route never made.
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "run_id_not_a_uuid",
                "run_id": body.run_id,
                "why": "this route reads the backtest job store, whose ids are UUIDs. A \
                        `sha256:…` id addresses the Set-J content-addressed run store, which \
                        holds no rows on this deployment and which this route does not read.",
                "fix": "list_backtests shows the ids this route accepts",
            })),
        )
            .into_response();
    };

    let tenant = token.user_id().to_string();
    let Ok(mut tx) = ledger::pg::tenant_tx(&state.pg, &tenant).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "no_tx" })))
            .into_response();
    };

    // `backtest_jobs` is scoped by `user_id` rather than by the mlops RLS
    // policy, so the ownership check is in the WHERE clause. Leaving it out
    // would let any caller read any run's trade list by guessing a UUID.
    let row = sqlx::query(
        "SELECT result, status, error FROM backtest_jobs WHERE id = $1 AND user_id = $2",
    )
    .bind(run_uuid)
    .bind(token.user_id())
    .fetch_optional(&mut *tx)
    .await;
    let _ = tx.commit().await;

    // Three outcomes, not two. The previous `let Ok(Some(row)) = row else …`
    // collapsed a query that could not execute into "no such run", which is how
    // a route reading a table renamed out from under it reported a clean 404
    // for every id for as long as it was wrong.
    let row = match row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "no_such_run",
                    "run_id": body.run_id,
                    "fix": "list_backtests shows what exists",
                })),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "run_lookup_failed",
                    "run_id": body.run_id,
                    "table": "backtest_jobs",
                    "message": e.to_string(),
                    "why": "the trade list could not be read, so nothing is known about this \
                            run's trades. This is not `no_such_run` and not an empty list — \
                            Postgres names the relation or column it could not resolve in \
                            `message`.",
                })),
            )
                .into_response()
        }
    };

    let status: String = row.try_get("status").unwrap_or_default();
    let run_error: Option<String> = row.try_get("error").unwrap_or_default();

    // `result` is nullable: a run that failed before producing one stores no
    // document at all. An empty trade list would say it traded nothing, which is
    // a statement about a run that never got far enough to trade.
    let Ok(Some(result)) = row.try_get::<Option<serde_json::Value>, _>("result") else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_result_document",
                "run_id": body.run_id,
                "status": status,
                "run_error": run_error,
                "why": "this run stored no result document, so its trade list is unknown — not \
                        empty. A run with no result did not trade zero times; it did not report.",
            })),
        )
            .into_response();
    };

    // A missing key and a recorded empty list are different facts, and only one
    // of them licenses "this run did not trade". The engine that wrote the rows
    // on this deployment stores `total_orders` / `total_positions` / `summary`
    // and no `trades` key at all, so the fallback-to-`[]` this replaced would
    // have reported "no trades" for a run that placed 58 orders.
    let raw = match result.get("trades") {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(other) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "trades_not_an_array",
                    "run_id": body.run_id,
                    "got_type": json_type(other),
                    "why": "the result document has a `trades` key that is not a list, so the \
                            trade list cannot be read and is not empty.",
                })),
            )
                .into_response()
        }
        None => {
            let orders = result.get("total_orders").cloned();
            let positions = result.get("total_positions").cloned();
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "trades_not_recorded",
                    "run_id": body.run_id,
                    "status": status,
                    "result_keys": result.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
                    "orders_recorded": orders,
                    "positions_recorded": positions,
                    "why": "this run's result document has no `trades` key. That is a recording \
                            gap in the engine that produced it, not a run that took no trades — \
                            `orders_recorded` says how many orders it placed. Nothing here \
                            supports a statement about concentration, path risk or costs.",
                    "fix": "re-run this configuration on an engine that records per-trade rows, \
                            or read the per-trade series directly if the run wrote one.",
                })),
            )
                .into_response()
        }
    };

    let slices: Vec<TradeSlice> = raw
        .iter()
        .enumerate()
        .map(|(i, t)| TradeSlice {
            entry_index: i,
            exit_index: i,
            pnl: t.get("pnl").and_then(as_number).unwrap_or(0.0),
        })
        .collect();

    let conc = forensics::concentration(&slices, 0.9, body.top_n);

    (
        StatusCode::OK,
        Json(json!({
            "run_id": body.run_id,
            "trades": raw,
            "concentration": conc.as_ref().map(|c| json!({
                "top": c.top,
                "top_n_share": c.top_n_share,
                "trades_for_90pct": c.trades_for_target,
                "total_trades": c.total_trades,
                "net_pnl": c.net_pnl,
                "gross_gain": c.gross_gain,
                "net_without_top_n": c.net_without_top(body.top_n),
            })),
            // The list was present in the result document, so an empty one here
            // is a run that recorded zero trades — the one case in which
            // "it did not trade" is a thing this route may say.
            "trades_recorded": true,
            "note": conc.as_ref().map_or(
                if raw.is_empty() {
                    "This run recorded a trade list and it is empty: it took no trades. That is \
                     a measured zero, not a missing measurement."
                } else {
                    "No winning trades, so there is no concentration to measure."
                },
                |c| if c.net_without_top(body.top_n) < 0.0 {
                    "Removing the top trades turns this result negative. A strategy whose edge \
                     disappears without its best few bars did not have an edge; it had those \
                     bars. Check whether they are real with probe_data_quality before concluding \
                     anything."
                } else {
                    "The result survives removing its best trades, so it is not resting on a \
                     handful of prints."
                },
            ),
        })),
    )
        .into_response()
}

fn as_number(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// Names the JSON type a field actually holds, so a refusal can say what was
/// there instead of only what was expected.
const fn json_type(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[derive(Debug, Deserialize)]
pub struct QualityBody {
    instrument_id: String,
    #[serde(default = "default_timeframe")]
    timeframe: String,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_wick")]
    wick_multiple: f64,
    #[serde(default = "default_sigma")]
    spike_sigma: f64,
}

fn default_timeframe() -> String {
    "1m".into()
}
const fn default_wick() -> f64 {
    3.0
}
const fn default_sigma() -> f64 {
    8.0
}

/// POST /api/evidence/data-quality — scan bars for the defects that make a
/// backtest lie.
pub async fn data_quality(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(body): Json<QualityBody>,
) -> impl IntoResponse {
    let Some(tf) = <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(
        &body.timeframe,
    ) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "unknown_timeframe", "got": body.timeframe })),
        )
            .into_response();
    };

    let end = body.end.unwrap_or_else(chrono::Utc::now);
    let start = body.start.unwrap_or(end - chrono::Duration::days(7));
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(end);

    // Resolve first, through the same point-in-time store, so the three ways
    // this can yield no bars stay three different answers: the backend could not
    // be read, the symbol maps to no instrument, or the instrument has no bars
    // in the window. Only the third is a fact about data; the first two are the
    // absence of a measurement, and reporting any of them as "no defects found"
    // is a clean bill of health nobody took.
    match store.resolve_instrument(&body.instrument_id).await {
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "instrument_lookup_failed",
                    "instrument_id": body.instrument_id,
                    "message": e.to_string(),
                    "why": "the symbol could not be resolved, so no bars were scanned and \
                            nothing is known about this instrument's data quality. ClickHouse \
                            names the table it could not read in `message`.",
                })),
            )
                .into_response()
        }
        Ok(None) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "instrument_not_resolved",
                    "instrument_id": body.instrument_id,
                    "as_of": end,
                    "why": "this symbol maps to no instrument/venue as of the window's end, so \
                            there were no bars to scan. Zero bars scanned is not zero defects.",
                    "fix": "list the instruments the platform has bars for and pass one of those",
                })),
            )
                .into_response()
        }
        Ok(Some(_)) => {}
    }

    let bars = match store.load_bars(&body.instrument_id, tf, start, end).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "bar_load_failed",
                    "instrument_id": body.instrument_id,
                    "timeframe": body.timeframe,
                    "start": start,
                    "end": end,
                    "message": e.to_string(),
                    "why": "the scan did not run, so this window has no defect count — neither \
                            zero nor any other number. ClickHouse names the relation or column \
                            it could not resolve in `message`.",
                })),
            )
                .into_response()
        }
    };

    if bars.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_bars_in_window",
                "instrument_id": body.instrument_id,
                "timeframe": body.timeframe,
                "start": start,
                "end": end,
                "why": "the instrument resolves but has no bars whose close falls in this \
                        window as of its end, so nothing was scanned. A scan of nothing is not \
                        a clean scan.",
                "fix": "widen the window, or pick a timeframe the platform has stored for this \
                        instrument",
            })),
        )
            .into_response();
    }

    let slices: Vec<BarSlice> = bars
        .iter()
        .map(|b| BarSlice {
            open: dec_f64(b.open),
            high: dec_f64(b.high),
            low: dec_f64(b.low),
            close: dec_f64(b.close),
            volume: dec_f64(b.volume),
        })
        .collect();

    let found = forensics::suspicious_bars(&slices, body.wick_multiple, body.spike_sigma);

    // Timestamps, not indices. An index into a window the agent did not choose
    // is not something it can quote in a reproduction.
    let flagged: Vec<_> = found
        .iter()
        .filter_map(|s| {
            bars.get(s.index).map(|b| {
                json!({
                    "index": s.index,
                    "ts": chrono::DateTime::from_timestamp_nanos(b.ts_ns),
                    "defect": s.defect,
                    "magnitude": s.magnitude,
                    "bar": { "o": b.open, "h": b.high, "l": b.low, "c": b.close, "v": b.volume },
                })
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "instrument_id": body.instrument_id,
            "timeframe": body.timeframe,
            "bars_scanned": slices.len(),
            "flagged": flagged,
            "note": if flagged.is_empty() {
                "No defects found in this window. That is not the same as clean data — this \
                 checks for impossible bars, prints with no volume, stale republished bars and \
                 spikes that fully revert, and a plausible-looking wrong price passes all four."
            } else {
                "Each flagged bar is a fact about the data, not yet a fact about a strategy. To \
                 file a data_artifact kill, show that the strategy's P&L depends on these bars — \
                 the trades route measures that."
            },
        })),
    )
        .into_response()
}

fn dec_f64(d: rust_decimal::Decimal) -> f64 {
    d.to_string().parse().unwrap_or(0.0)
}

#[derive(Debug, Deserialize)]
pub struct SimilarBody {
    #[serde(default)]
    config_hash: Option<String>,
    #[serde(default)]
    experiment_id: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}

const fn default_limit() -> i64 {
    20
}

/// POST /api/evidence/similar-trials — what the ledger already knows.
///
/// The agent could not query its own history, so every session started cold and
/// nothing stopped it re-running a trial it had already run. `config_hash` finds
/// exact repeats; `experiment_id` finds neighbours.
pub async fn similar_trials(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<SimilarBody>,
) -> impl IntoResponse {
    if body.config_hash.is_none() && body.experiment_id.is_none() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_selector",
                "fix": "supply `config_hash` to find exact repeats, or `experiment_id` to find \
                        neighbours in the same line of work",
            })),
        )
            .into_response();
    }

    let tenant = token.user_id().to_string();
    let Ok(mut tx) = ledger::pg::tenant_tx(&state.pg, &tenant).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "no_tx" })))
            .into_response();
    };

    let rows = sqlx::query(
        "SELECT trial_id, config_hash, experiment_id, dataset_id, registered_at,
                actor_kind, actor_id, exploration_flag, delta_practical
           FROM mlops.trial
          WHERE tenant_id = $1
            AND ($2::text IS NULL OR config_hash = $2)
            AND ($3::text IS NULL OR experiment_id = $3)
          ORDER BY registered_at DESC
          LIMIT $4",
    )
    .bind(&tenant)
    .bind(&body.config_hash)
    .bind(&body.experiment_id)
    .bind(body.limit.clamp(1, 200))
    .fetch_all(&mut *tx)
    .await;
    let _ = tx.commit().await;

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

    let trials: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "trial_id": r.get::<uuid::Uuid, _>("trial_id"),
                "config_hash": r.get::<String, _>("config_hash"),
                "experiment_id": r.get::<Option<String>, _>("experiment_id"),
                "dataset_id": r.get::<String, _>("dataset_id"),
                "registered_at": r.get::<chrono::DateTime<chrono::Utc>, _>("registered_at"),
                "actor": format!(
                    "{}:{}",
                    r.get::<String, _>("actor_kind"),
                    r.get::<String, _>("actor_id")
                ),
                "exploration": r.get::<bool, _>("exploration_flag"),
            })
        })
        .collect();

    let exact = body.config_hash.is_some();
    (
        StatusCode::OK,
        Json(json!({
            "trials": trials,
            "count": trials.len(),
            "note": if trials.is_empty() {
                "Nothing matching in the ledger."
            } else if exact {
                "These are trials with the SAME config hash — this exact configuration has been \
                 evaluated before. Re-running it produces no new market evidence and still \
                 counts against the trial budget, which is how N_eff is inflated by repetition."
            } else {
                "Neighbours in the same experiment. Look at what has already been tried before \
                 proposing something adjacent to it."
            },
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pnl_field_parses_from_a_number_or_a_decimal_string() {
        // The ledger stores money as exact decimal strings (ADR-0002), and a
        // run result written by an older path may carry a JSON number. Reading
        // only one of the two silently measures concentration over zeros.
        assert_eq!(as_number(&json!(1.5)), Some(1.5));
        assert_eq!(as_number(&json!("2.25")), Some(2.25));
        assert_eq!(as_number(&json!("-0.5")), Some(-0.5));
        assert_eq!(as_number(&json!("not a number")), None);
        assert_eq!(as_number(&json!(null)), None);
    }

    #[test]
    fn the_defaults_are_the_thresholds_the_forensics_module_documents() {
        assert!((default_wick() - 3.0).abs() < f64::EPSILON);
        assert!((default_sigma() - 8.0).abs() < f64::EPSILON);
        assert_eq!(default_top_n(), 6);
    }

    #[test]
    fn a_missing_trade_list_and_an_empty_one_are_different_documents() {
        // The route branches on exactly this, and the branch is the whole
        // point: `path_risk` prints "An empty trade list is not a result with
        // zero risk; it is a run that did not trade", which is true of the
        // second document and false of the first. Every result document this
        // deployment has stored is the first kind — `total_orders`, `summary`,
        // and no `trades` key — including one whose run placed 58 orders.
        let absent = json!({ "total_orders": 58, "summary": { "orders.total": "58" } });
        let empty = json!({ "trades": [] });
        assert!(absent.get("trades").is_none());
        assert_eq!(empty.get("trades"), Some(&json!([])));
        // The fallback this replaced collapsed them into one answer.
        let collapse = |v: &serde_json::Value| {
            v.get("trades")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(collapse(&absent), collapse(&empty));
    }

    #[test]
    fn a_refusal_can_name_the_type_it_actually_found() {
        assert_eq!(json_type(&json!(null)), "null");
        assert_eq!(json_type(&json!([])), "array");
        assert_eq!(json_type(&json!({})), "object");
        assert_eq!(json_type(&json!("x")), "string");
        assert_eq!(json_type(&json!(1)), "number");
        assert_eq!(json_type(&json!(true)), "bool");
    }

    #[test]
    fn a_decimal_converts_without_losing_the_magnitude() {
        use rust_decimal_macros::dec;
        assert!((dec_f64(dec!(123.456)) - 123.456).abs() < 1e-9);
        assert!((dec_f64(dec!(-0.00000001)) + 0.00000001).abs() < 1e-12);
        assert!(dec_f64(rust_decimal::Decimal::ZERO).abs() < f64::EPSILON);
    }
}
