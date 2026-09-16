//! Running a templated tool (AGENT-006 §3.3; Set S tasks S-3.1, S-3.2).
//!
//! `crates/mcp-server` declares these tools and routes calls here; this is the
//! side that has a database, a filesystem and a Python interpreter. The division
//! is the point: a tool being a Python file must not become a way around the
//! things a tool goes through.
//!
//! # Materialisation, and the one read path
//!
//! A template has no network and no database, so its rows arrive as a parquet
//! file. Those rows are fetched **through `BarStore::as_of`**, the same
//! point-in-time read `GET /api/data/bars` uses, clipped by the same
//! `Project::clip`. There is no second query here and there is no "just for
//! analysis" path — a template asking for a window it may not see is refused
//! before a process is spawned.
//!
//! The cache key includes the effective `as_of` for the same reason. Two
//! projects with different horizons asking for one window are asking for
//! different bars, because `as_of` bounds which *revisions* are visible; a key
//! without it would hand the first project's file to the second, which is a
//! future leak with a cache in front of it.
//!
//! # Write-once
//!
//! A materialised window is written to a temporary name and renamed into place.
//! Rename is atomic on every filesystem this runs on, so a second caller either
//! sees no file or sees a complete one — never the half a concurrent writer had
//! got through, which would surface as a parquet parse error inside a statistic.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use skills::template::{DataKey, Template};
use uuid::Uuid;

use crate::projects::{Project, ProjectError, ProjectStore};
use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct RunBody {
    /// The tool name, which is also the template's file stem.
    pub template: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// Where materialised windows live.
///
/// Under the platform's data directory rather than a temp dir: these are
/// content-addressed and reusable, and throwing them away between restarts would
/// re-query ClickHouse for every repeated call.
fn cache_root() -> PathBuf {
    std::env::var("TEMPLATE_DATA_DIR").map_or_else(
        |_| {
            std::env::var("WORKSPACE_ROOT")
                .map_or_else(|_| PathBuf::from("."), PathBuf::from)
                .join("data")
                .join("template-cache")
        },
        PathBuf::from,
    )
}

fn bad(code: &str, message: String) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": code, "message": message })),
    )
        .into_response()
}

/// POST /api/agent/templates/run
pub async fn run_template(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<RunBody>,
) -> impl IntoResponse {
    let Some(template) = mcp_server_lib::tools::templated::find(&body.template) else {
        return bad(
            "unknown_template",
            format!("no templated tool is named {:?}", body.template),
        );
    };
    let args = if body.arguments.is_object() {
        body.arguments.clone()
    } else {
        json!({})
    };

    // Required arguments first: a missing one should cost nothing, not a
    // ClickHouse scan followed by a Python traceback.
    if let Some(required) = template
        .header()
        .input_schema
        .get("required")
        .and_then(Value::as_array)
    {
        for field in required.iter().filter_map(Value::as_str) {
            if args.get(field).is_none() {
                return bad(
                    "missing_field",
                    format!("`{}` requires `{field}`", template.header().name),
                );
            }
        }
    }

    let project = match resolve_project(&state, token.user_id(), body.project_id).await {
        Ok(p) => p,
        Err(e) => return project_error(&e),
    };

    let mut payload = args.clone();
    let mut data = serde_json::Map::new();
    let mut windows = Vec::new();

    for series in &template.header().series {
        let key = match data_key(&args, series, &project) {
            Ok(k) => k,
            Err(response) => return response,
        };
        let path = match materialise(&state.clickhouse_url, &key, &cache_root()).await {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": "materialisation_failed",
                        "message": e.to_string(),
                        "series": series.role,
                    })),
                )
                    .into_response()
            }
        };
        data.insert(series.role.clone(), json!(path.to_string_lossy()));
        windows.push(json!({
            "role": series.role,
            "instrument_id": key.instrument_id,
            "timeframe": key.timeframe,
            "start": key.start,
            "end": key.end,
            "as_of": key.as_of,
            "digest": key.digest(),
        }));
    }

    if let Some(map) = payload.as_object_mut() {
        map.insert("data".into(), Value::Object(data));
        // Seeded even when the schema does not require it. A template that draws
        // without a seed raises in the preamble, and a template that does not
        // draw ignores this.
        map.entry("seed".to_string()).or_insert(json!(0));
    }

    execute(template, &payload, windows).await
}

fn project_error(e: &ProjectError) -> axum::response::Response {
    let status = match e {
        ProjectError::NotFound(_) => StatusCode::NOT_FOUND,
        ProjectError::Sqlx(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::FORBIDDEN,
    };
    (
        status,
        Json(json!({ "error": "project", "message": e.to_string() })),
    )
        .into_response()
}

async fn resolve_project(
    state: &AppState,
    user_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Project, ProjectError> {
    let store = ProjectStore::new(state.pg.clone());
    match project_id {
        Some(id) => {
            let project = store.get(id).await?;
            if project.user_id != user_id {
                return Err(ProjectError::NotFound(id.to_string()));
            }
            Ok(project)
        }
        None => store.desk(user_id).await,
    }
}

/// Turn a call's arguments into the window it is actually allowed to see.
fn data_key(
    args: &Value,
    series: &skills::template::SeriesRequest,
    project: &Project,
) -> Result<DataKey, axum::response::Response> {
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);

    let instrument = text(&series.instrument_arg).ok_or_else(|| {
        bad(
            "missing_field",
            format!(
                "the `{}` series is named by `{}`, which this call did not set",
                series.role, series.instrument_arg
            ),
        )
    })?;
    let timeframe = text(&series.timeframe_arg).unwrap_or_else(|| "1h".to_string());

    let parse = |k: &str| -> Result<DateTime<Utc>, axum::response::Response> {
        let raw = text(k).ok_or_else(|| bad("missing_field", format!("`{k}` is required")))?;
        DateTime::parse_from_rfc3339(&raw)
            .map(|d| d.with_timezone(&Utc))
            .map_err(|e| bad("bad_timestamp", format!("`{k}` is not RFC 3339: {e}")))
    };
    let start = parse("start")?;
    let requested_end = parse("end")?;

    // The same two clips `GET /api/data/bars` applies, in the same order. `end`
    // bounds which bars and `as_of` bounds which revisions of them, and both go
    // through the project's horizon.
    let (end, cutoff_applied) = project.clip(requested_end);
    let (as_of, _) = project.clip(end);

    if start >= end {
        return Err(bad(
            "empty_window",
            format!(
                "start {start} is at or after the effective end {end}{}",
                if cutoff_applied.is_some() {
                    " — the window was clipped to this project's research cutoff"
                } else {
                    ""
                }
            ),
        ));
    }

    Ok(DataKey {
        instrument_id: instrument,
        timeframe,
        start: start.to_rfc3339(),
        end: end.to_rfc3339(),
        as_of: as_of.to_rfc3339(),
    })
}

/// Fetch the window point-in-time and write it once.
async fn materialise(
    clickhouse_url: &str,
    key: &DataKey,
    root: &Path,
) -> anyhow::Result<PathBuf> {
    let path = key.path(root);
    if path.is_file() {
        return Ok(path);
    }

    let as_of = DateTime::parse_from_rfc3339(&key.as_of)?.with_timezone(&Utc);
    let start = DateTime::parse_from_rfc3339(&key.start)?.with_timezone(&Utc);
    let end = DateTime::parse_from_rfc3339(&key.end)?.with_timezone(&Utc);

    let timeframe =
        <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(&key.timeframe)
            .ok_or_else(|| anyhow::anyhow!("unknown timeframe {:?}", key.timeframe))?;

    let store = backtest::store::BarStore::connect(clickhouse_url).as_of(as_of);
    let bars = store
        .load_bars(&key.instrument_id, timeframe, start, end)
        .await?;

    let bytes = encode_parquet(&bars)?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Atomic publish. A reader sees no file or a whole one, never the prefix a
    // concurrent writer had managed — which would present as a parquet parse
    // error from inside a statistic, three layers from its cause.
    let staging = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&staging, &bytes)?;
    match std::fs::rename(&staging, &path) {
        Ok(()) => {}
        Err(e) => {
            let _ = std::fs::remove_file(&staging);
            // Another process winning the race is success, not failure: the
            // content is addressed by its key, so their file and ours are the
            // same bytes.
            if !path.is_file() {
                return Err(e.into());
            }
        }
    }
    Ok(path)
}

/// `ts, open, high, low, close, volume, trade_count, quality_flags`.
///
/// Prices are `Decimal` in the store and `f64` here, and that is a deliberate
/// narrowing rather than an oversight: these columns feed statistics, not money.
/// Nothing downstream of a template settles, sizes or books anything — the
/// tier is read-only by construction — and pandas has no decimal path that any
/// of numpy, scipy or the statistics themselves would preserve.
fn encode_parquet(bars: &[backtest::store::LoadedBar]) -> anyhow::Result<Vec<u8>> {
    use arrow::array::{ArrayRef, Float64Array, Int64Array, TimestampNanosecondArray, UInt32Array};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use rust_decimal::prelude::ToPrimitive;

    let schema = Arc::new(Schema::new(vec![
        Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
            false,
        ),
        Field::new("open_ns", DataType::Int64, false),
        Field::new("open", DataType::Float64, false),
        Field::new("high", DataType::Float64, false),
        Field::new("low", DataType::Float64, false),
        Field::new("close", DataType::Float64, false),
        Field::new("volume", DataType::Float64, false),
        Field::new("trade_count", DataType::Int64, false),
        Field::new("quality_flags", DataType::UInt32, false),
    ]));

    let f = |pick: fn(&backtest::store::LoadedBar) -> rust_decimal::Decimal| -> Vec<f64> {
        bars.iter()
            .map(|b| pick(b).to_f64().unwrap_or(f64::NAN))
            .collect()
    };

    let arrays: Vec<ArrayRef> = vec![
        Arc::new(
            TimestampNanosecondArray::from(bars.iter().map(|b| b.ts_ns).collect::<Vec<_>>())
                .with_timezone("UTC"),
        ),
        Arc::new(Int64Array::from(
            bars.iter().map(|b| b.open_ns).collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(f(|b| b.open))),
        Arc::new(Float64Array::from(f(|b| b.high))),
        Arc::new(Float64Array::from(f(|b| b.low))),
        Arc::new(Float64Array::from(f(|b| b.close))),
        Arc::new(Float64Array::from(f(|b| b.volume))),
        Arc::new(Int64Array::from(
            bars.iter()
                .map(|b| i64::try_from(b.trade_count).unwrap_or(i64::MAX))
                .collect::<Vec<_>>(),
        )),
        Arc::new(UInt32Array::from(
            bars.iter().map(|b| b.quality_flags.0).collect::<Vec<_>>(),
        )),
    ];

    let batch = RecordBatch::try_new(schema.clone(), arrays)?;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None)?;
        writer.write(&batch)?;
        writer.close()?;
    }
    Ok(buf)
}

/// Spawn the template and shape what comes back into a `ToolResult`.
async fn execute(
    template: &'static Template,
    payload: &Value,
    windows: Vec<Value>,
) -> axum::response::Response {
    let header = template.header();
    let skill = match skills::Skill::author(
        header.name.clone(),
        header.description.clone(),
        template.runnable_source(),
        template.contract(),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "bad_template", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    let root = std::env::var("WORKSPACE_ROOT").map_or_else(|_| PathBuf::from("."), PathBuf::from);
    let mut config = skills::RunnerConfig::from_workspace(&root);
    config.limits = template.limits();

    let done = skills::runner::run(&skill, payload, &config).await;
    let Some(result) = done.result else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "template_failed",
                "template": header.name,
                "outcome": format!("{:?}", done.record.outcome),
                "windows": windows,
            })),
        )
            .into_response();
    };

    // A refusal is an answer. It is passed through rather than turned into an
    // error, because "I will not compute a Sharpe on a series with no finite
    // variance" is information the model should act on, and an HTTP error is
    // something it retries.
    if result.get("refused").and_then(Value::as_bool) == Some(true) {
        return Json(json!({
            "refused": true,
            "reason": result.get("reason").cloned().unwrap_or(Value::Null),
            "detail": result.get("detail").cloned().unwrap_or(Value::Null),
            "windows": windows,
        }))
        .into_response();
    }

    let detail = result.get("detail").cloned().unwrap_or_else(|| json!({}));
    let summary = match &header.summary_template {
        Some(t) => {
            let declared: Vec<(&str, &str)> = header
                .detail_fields
                .iter()
                .map(|(f, u)| (f.as_str(), u.as_str()))
                .collect();
            mcp_server_lib::spec::render_summary(t, &declared, &detail)
                .unwrap_or_else(|e| format!("{} completed ({e:?})", header.name))
        }
        None => format!("{} completed", header.name),
    };

    Json(json!({
        "summary": summary,
        "n": result.get("n").cloned().unwrap_or(Value::Null),
        "se": result.get("se").cloned().unwrap_or(Value::Null),
        "ids": result.get("ids").cloned().unwrap_or_else(|| json!({})),
        "detail": detail,
        "windows": windows,
        "duration_ms": done.record.duration_ms,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectKind;
    use chrono::Duration;

    fn project_with(cutoff: Option<DateTime<Utc>>) -> Project {
        Project {
            project_id: Uuid::nil(),
            user_id: Uuid::nil(),
            kind: if cutoff.is_some() {
                ProjectKind::Research
            } else {
                ProjectKind::Desk
            },
            name: "t".into(),
            goal: None,
            instruments: vec![],
            research_cutoff: cutoff,
            status: "active".into(),
            created_at: Utc::now(),
        }
    }

    fn series() -> skills::template::SeriesRequest {
        skills::template::SeriesRequest {
            role: "primary".into(),
            instrument_arg: "instrument_id".into(),
            timeframe_arg: "timeframe".into(),
        }
    }

    fn args(start: &str, end: &str) -> Value {
        json!({
            "instrument_id": "BTC-USD",
            "timeframe": "1h",
            "start": start,
            "end": end,
        })
    }

    /// The invariant the whole tier rests on: a template cannot see past its
    /// project's horizon, and it cannot get there by asking.
    #[test]
    fn a_window_past_the_cutoff_is_clipped_not_served() {
        let cutoff = Utc::now() - Duration::days(45);
        let p = project_with(Some(cutoff));
        let key = data_key(
            &args("2024-01-01T00:00:00Z", "2099-01-01T00:00:00Z"),
            &series(),
            &p,
        )
        .map_err(|_| "refused")
        .expect("a window starting before the cutoff is answerable");

        let end = DateTime::parse_from_rfc3339(&key.end).unwrap().with_timezone(&Utc);
        let as_of = DateTime::parse_from_rfc3339(&key.as_of).unwrap().with_timezone(&Utc);
        assert!(end <= cutoff, "end {end} is past the cutoff {cutoff}");
        assert!(as_of <= cutoff, "as_of {as_of} is past the cutoff {cutoff}");
    }

    /// And a window entirely past the cutoff is a refusal rather than an empty
    /// frame: a template handed zero bars reports "n=0" and the model reads that
    /// as a finding about the market.
    #[test]
    fn a_window_entirely_past_the_cutoff_is_refused() {
        let p = project_with(Some(Utc::now() - Duration::days(45)));
        assert!(data_key(
            &args("2098-01-01T00:00:00Z", "2099-01-01T00:00:00Z"),
            &series(),
            &p
        )
        .is_err());
    }

    /// Two horizons over one window are two different files. This is the cache
    /// bug that would be a future leak: without `as_of` in the key, whichever
    /// project asked first would decide what the other one saw.
    #[test]
    fn two_horizons_do_not_share_a_materialised_file() {
        let older = project_with(Some(Utc::now() - Duration::days(400)));
        let newer = project_with(Some(Utc::now() - Duration::days(45)));
        let a = args("2020-01-01T00:00:00Z", "2099-01-01T00:00:00Z");
        let ka = data_key(&a, &series(), &older).map_err(|_| "refused").unwrap();
        let kb = data_key(&a, &series(), &newer).map_err(|_| "refused").unwrap();
        assert_ne!(ka.digest(), kb.digest());
    }

    /// The parquet the templates read is written by this function, so its
    /// columns are a contract with every template's `load()`.
    #[test]
    fn the_materialised_schema_is_the_one_templates_read() {
        let bytes = encode_parquet(&[]).expect("an empty window still has a schema");
        let text = String::from_utf8_lossy(&bytes);
        for column in ["ts", "open", "high", "low", "close", "volume"] {
            assert!(
                text.contains(column),
                "the materialised parquet has no `{column}` column"
            );
        }
    }
}
