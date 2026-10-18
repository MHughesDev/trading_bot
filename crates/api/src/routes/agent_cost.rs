//! What a call will cost, before making it (AGENT-006 §2.4b; Set S task S-3.19).
//!
//! A model that can ask what a call will cost can plan inside a budget instead
//! of discovering it at the wall. This is that question, and it spends nothing
//! answering it: it reads the catalogue, the caller's own completed runs and a
//! closed form, and dispatches nothing.
//!
//! # The three numbers, and which of them is real
//!
//! * **`trials`** is exact. A tool's trial cost is its `ToolSpec::spends_trial`
//!   declaration, which is also what the call gate reads — so this cannot
//!   disagree with what actually happens.
//! * **`est_seconds`** is measured-then-extrapolated, and carries a band. The
//!   throughput comes from this user's own completed runs (rows per second of
//!   wall clock); with no completed runs it is `null` and labelled `unmeasured`,
//!   never defaulted to a constant somebody chose.
//! * **`est_usd`** is priced from the shipped price table (ADR-D-03, OQ-16
//!   option A): seconds times the hardware class's `usd_per_second`, from a
//!   **fresh verified row only**. The class is named by `TBOT_HARDWARE_CLASS`,
//!   because the platform cannot tell what its compute costs by looking at itself.
//!   With no class, no table or a stale row it is `null` and says which, never a
//!   default: `ask_above_usd` pauses for a human, so a rate that is too low never
//!   pauses and one that is too high pauses on everything, and both failures look
//!   like the gate working. The dollar band is the seconds band times the same rate.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimateBody {
    /// The tool being considered.
    pub tool: String,
    /// The arguments it would be called with.
    #[serde(default)]
    pub arguments: Value,
}

/// Below this many completed runs the estimate is reported as indicative.
///
/// Ten is the same floor `probability_of_overfit` uses, and for a related
/// reason: a ratio derived from three runs is dominated by whichever three they
/// were. It does not change the number, only what the number is called.
const CONFIDENT_RUNS: usize = 10;

fn refuse(code: &str, message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "refused": true, "error": code, "message": message.into() })),
    )
        .into_response()
}

/// Bars in the argument window, when the call names one.
///
/// `None` rather than a guess when the arguments carry no window: a tool called
/// without one is a tool whose size this cannot know, and an invented row count
/// would produce an estimate with a band around nothing.
fn rows_from_arguments(arguments: &Value) -> Option<u64> {
    let text = |k: &str| arguments.get(k).and_then(Value::as_str);
    let start = DateTime::parse_from_rfc3339(text("start")?)
        .ok()?
        .with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(text("end")?)
        .ok()?
        .with_timezone(&Utc);
    let timeframe = text("timeframe").unwrap_or("1h");
    let seconds = timeframe_seconds(timeframe)?;
    let span = (end - start).num_seconds();
    if span <= 0 {
        return None;
    }
    u64::try_from(span / seconds).ok()
}

fn timeframe_seconds(timeframe: &str) -> Option<i64> {
    let (size, unit) = timeframe.split_at(timeframe.len().checked_sub(1)?);
    let size: i64 = size.parse().ok()?;
    let unit = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    (size > 0).then_some(size * unit)
}

/// Dollars for a seconds estimate, or why there is no figure. Pure over its
/// inputs so the "no fresh row, no number" rule is tested without a filesystem.
fn price_seconds(
    seconds: Option<f64>,
    band: (Option<f64>, Option<f64>),
    class: Option<&str>,
    table: Option<&harness::adapter::PriceTable>,
    today: &str,
) -> Result<(f64, f64, f64), &'static str> {
    let Some(seconds) = seconds else {
        return Err("no seconds estimate to price");
    };
    let Some(class) = class else {
        return Err(
            "TBOT_HARDWARE_CLASS is not set, so the platform does not know what its compute costs",
        );
    };
    let Some(table) = table else {
        return Err("no price table could be read (TBOT_PRICES_PATH / config/prices.yaml)");
    };
    let Some(micro) = table.hardware_micro_usd_per_second(class, today) else {
        return Err("the price table has no fresh verified row for this hardware class");
    };
    #[allow(clippy::cast_precision_loss)]
    let per_second = micro as f64 / 1_000_000.0;
    let (lo, hi) = (band.0.unwrap_or(seconds), band.1.unwrap_or(seconds));
    Ok((seconds * per_second, lo * per_second, hi * per_second))
}

fn load_price_table() -> Option<harness::adapter::PriceTable> {
    let path = std::env::var("TBOT_PRICES_PATH").unwrap_or_else(|_| "config/prices.yaml".into());
    let text = std::fs::read_to_string(path).ok()?;
    harness::adapter::PriceTable::from_yaml(&text).ok()
}

/// POST /api/agent/cost/estimate
pub async fn estimate(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<EstimateBody>,
) -> impl IntoResponse {
    let Some(spec) = mcp_server_lib::spec::find(&body.tool) else {
        return refuse(
            "unknown_tool",
            format!(
                "no tool is named {:?}; ask about one in the catalogue",
                body.tool
            ),
        );
    };

    // Exact, because it is the same declaration the call gate reads.
    let (trials, trial_kind) = match spec.spends_trial {
        mcp_server_lib::spec::TrialKind::None => (0u32, "none"),
        mcp_server_lib::spec::TrialKind::Search => (1, "search"),
        mcp_server_lib::spec::TrialKind::Falsification => (1, "falsification"),
    };

    let (throughput, runs) = state.suite.observed_throughput(token.user_id());
    let rows = rows_from_arguments(&body.arguments);

    let (seconds, low, high, confidence, note) = match (rows, throughput > 0.0) {
        (Some(rows), true) => {
            let answer = jobs::models::estimate_cost(
                jobs::models::CostInputs {
                    rows,
                    // One pass over the bars. `features` and `steps` exist for
                    // training jobs, where the same formula reads a manifest;
                    // a catalogue tool over a window is one pass, and claiming
                    // otherwise would scale the estimate by a number nobody set.
                    features: 1,
                    steps: 1,
                    throughput,
                },
                i64::try_from(runs).unwrap_or(i64::MAX),
            );
            let (lo, hi) = answer.value.interval();
            let confidence = if runs >= CONFIDENT_RUNS {
                "measured"
            } else {
                "indicative"
            };
            (
                Some(answer.value.seconds),
                Some(lo),
                Some(hi),
                confidence,
                format!(
                    "throughput measured at {throughput:.0} rows/s over {runs} completed run(s); \
                     the band is a factor of {:.0} either way and models arithmetic only — not \
                     I/O, not contention, not warm-up",
                    answer.value.band
                ),
            )
        }
        (None, _) => (
            None,
            None,
            None,
            "unmeasured",
            "these arguments name no window, so there is no row count to estimate from".to_string(),
        ),
        (Some(_), false) => (
            None,
            None,
            None,
            "unmeasured",
            "no completed run has been recorded for this user, so there is no measured \
             throughput. A constant would be a number nobody measured, and it would be \
             indistinguishable from one that was."
                .to_string(),
        ),
    };

    let class = std::env::var("TBOT_HARDWARE_CLASS").ok();
    let table = load_price_table();
    let today = Utc::now().date_naive().to_string();
    let usd = price_seconds(
        seconds,
        (low, high),
        class.as_deref(),
        table.as_ref(),
        &today,
    );
    let (est_usd, usd_low, usd_high, usd_note) = match usd {
        Ok((m, lo, hi)) => (Some(m), Some(lo), Some(hi), "seconds x the hardware class's verified price per second".to_string()),
        Err(why) => (None, None, None, format!("no dollar figure: {why}. A rate nobody verified would make `ask_above_usd` look like it was working whichever way it was wrong.")),
    };

    Json(json!({
        "summary_fields": {
            "tool": spec.name,
            "trials": trials,
            "est_seconds": seconds,
            "est_usd": est_usd,
            "confidence": confidence,
        },
        "detail": {
            "tool": spec.name,
            "trials": trials,
            "trial_kind": trial_kind,
            "requires_approval": spec.envelope.requires_approval(),
            "approval_envelope": spec.envelope.as_str(),
            "max_calls_per_run": spec.max_calls_per_run,
            "est_rows": rows,
            "est_seconds": seconds,
            "est_seconds_low": low,
            "est_seconds_high": high,
            "throughput_rows_per_second": (throughput > 0.0).then_some(throughput),
            "measured_from_runs": runs,
            "confidence": confidence,
            "note": note,
            "est_usd": est_usd,
            "est_usd_low": usd_low,
            "est_usd_high": usd_high,
            "est_usd_basis": usd_note,
        },
        "ids": {},
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_becomes_a_row_count() {
        let args = json!({
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-01-02T00:00:00Z",
            "timeframe": "1h",
        });
        assert_eq!(rows_from_arguments(&args), Some(24));

        let minutes = json!({
            "start": "2024-01-01T00:00:00Z",
            "end": "2024-01-01T01:00:00Z",
            "timeframe": "1m",
        });
        assert_eq!(rows_from_arguments(&minutes), Some(60));
    }

    /// No window, no estimate — rather than a default that would give the band
    /// something to be a band around.
    #[test]
    fn arguments_without_a_window_yield_no_row_count() {
        assert_eq!(rows_from_arguments(&json!({})), None);
        assert_eq!(
            rows_from_arguments(&json!({"start": "2024-01-01T00:00:00Z"})),
            None
        );
        // Backwards is not a small window.
        assert_eq!(
            rows_from_arguments(&json!({
                "start": "2024-02-01T00:00:00Z",
                "end": "2024-01-01T00:00:00Z",
                "timeframe": "1h",
            })),
            None
        );
        // An unrecognised timeframe is not silently treated as hours.
        assert_eq!(
            rows_from_arguments(&json!({
                "start": "2024-01-01T00:00:00Z",
                "end": "2024-01-02T00:00:00Z",
                "timeframe": "1fortnight",
            })),
            None
        );
    }

    fn table(verified: &str) -> harness::adapter::PriceTable {
        harness::adapter::PriceTable::from_yaml(&format!(
            "hardware:
  - hardware_class: gtx1080ti
    usd_per_second: \"0.0001\"
    source_url: x
    verified_at: \"{verified}\"
"
        ))
        .expect("parses")
    }

    /// A fresh verified row prices the seconds and the band.
    #[test]
    fn a_fresh_row_prices_seconds_and_the_band() {
        let t = table("2026-10-01");
        let (m, lo, hi) = price_seconds(
            Some(100.0),
            (Some(33.0), Some(300.0)),
            Some("gtx1080ti"),
            Some(&t),
            "2026-10-03",
        )
        .unwrap();
        assert!((m - 0.01).abs() < 1e-12 && lo < m && hi > m);
    }

    /// No class, no table, a stale row or an unknown class is no figure -- never
    /// a default and never zero.
    #[test]
    fn no_fresh_row_is_no_number() {
        let t = table("2026-10-01");
        assert!(price_seconds(Some(1.0), (None, None), None, Some(&t), "2026-10-03").is_err());
        assert!(price_seconds(
            Some(1.0),
            (None, None),
            Some("gtx1080ti"),
            None,
            "2026-10-03"
        )
        .is_err());
        assert!(price_seconds(
            Some(1.0),
            (None, None),
            Some("other"),
            Some(&t),
            "2026-10-03"
        )
        .is_err());
        let stale = table("2025-01-01");
        assert!(price_seconds(
            Some(1.0),
            (None, None),
            Some("gtx1080ti"),
            Some(&stale),
            "2026-10-03"
        )
        .is_err());
        assert!(price_seconds(
            None,
            (None, None),
            Some("gtx1080ti"),
            Some(&t),
            "2026-10-03"
        )
        .is_err());
    }
}
