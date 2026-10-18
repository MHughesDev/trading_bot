//! Market-data tools: `get_bars`, `get_instrument`, `list_asset_classes`.
//!
//! `get_bars` is the agent's eyes on the data: real OHLCV history for any
//! instrument/interval, size-capped so a months-long window can't blow out the
//! context. Coarser intervals are aggregated server-side from stored 1m bars.

use serde_json::{json, Value};

use crate::ApiClient;

/// Hard cap on bars returned to the model in one call.
const MAX_BARS_CAP: usize = 2000;
const DEFAULT_MAX_BARS: usize = 300;

fn f64_of(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_str())
        .and_then(|s| s.parse::<f64>().ok())
}

/// `get_bars` — fetch OHLCV bars with summary stats and tail-truncation.
pub async fn get_bars(api: &ApiClient, params: &Value) -> Value {
    let instrument = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if instrument.is_empty() {
        return crate::tool_error::missing_field("instrument_id");
    }
    let start = params.get("start").and_then(|v| v.as_str()).unwrap_or("");
    let end = params.get("end").and_then(|v| v.as_str()).unwrap_or("");
    if start.is_empty() || end.is_empty() {
        return crate::tool_error::missing_field("start/end (RFC3339)");
    }
    let interval = params
        .get("interval_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600)
        .clamp(60, 86_400);
    let max_bars = params
        .get("max_bars")
        .and_then(|v| v.as_i64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_MAX_BARS)
        // A floor of 10 turned "the latest bar" (max_bars 1) into ten bars, and a
        // small model re-asked with a shifted window until fenced (2026-10-05).
        .clamp(1, MAX_BARS_CAP);

    let path = format!(
        "/assets/chart/bars?symbol={}&start={}&end={}&interval_seconds={}",
        urlencode(instrument),
        urlencode(start),
        urlencode(end),
        interval
    );
    let resp = match api.get(&path).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let bars = resp
        .get("bars")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Summary stats over the FULL window, before truncation.
    let closes: Vec<f64> = bars.iter().filter_map(|b| f64_of(b.get("c"))).collect();
    let highs: Vec<f64> = bars.iter().filter_map(|b| f64_of(b.get("h"))).collect();
    let lows: Vec<f64> = bars.iter().filter_map(|b| f64_of(b.get("l"))).collect();
    let summary = if closes.is_empty() {
        json!({ "bar_count": 0 })
    } else {
        let first = closes.first().copied().unwrap_or(0.0);
        let last = closes.last().copied().unwrap_or(0.0);
        json!({
            "bar_count": bars.len(),
            "first_close": first,
            "last_close": last,
            "return_pct": if first != 0.0 { (last - first) / first * 100.0 } else { 0.0 },
            "high": highs.iter().cloned().fold(f64::MIN, f64::max),
            "low": lows.iter().cloned().fold(f64::MAX, f64::min),
        })
    };

    let omitted = bars.len().saturating_sub(max_bars);
    let tail: Vec<Value> = bars.iter().skip(omitted).cloned().collect();

    json!({
        "instrument_id": instrument,
        "interval_seconds": interval,
        "summary": summary,
        "bars": tail,
        "omitted_earlier_bars": omitted,
        "note": if omitted > 0 {
            "bars is the most recent tail; summary covers the full window — narrow the range or raise interval_seconds for detail"
        } else { "" },
    })
}

/// `get_instrument` — one instrument's registry row.
pub async fn get_instrument(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return crate::tool_error::missing_field("instrument_id");
    }
    match api
        .get(&format!("/api/instruments/{}", urlencode(id)))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_asset_classes` — asset classes the platform knows.
pub async fn list_asset_classes(api: &ApiClient) -> Value {
    match api.get("/api/assets").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Minimal percent-encoding for query/path values (alnum plus -_.~ kept).
pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// ── S-3.15: funding history, point-in-time provenance, survivorship ──────────
//
// Thin clients over routes that already exist (`/api/data/funding`,
// `/api/data/bars`, `/api/data/universe/{name}`). Everything below is arithmetic
// over a response, which this crate is allowed (ADR-0010). The reading rules are
// pure functions so they are tested without a platform.

/// Largest gap, as a share of expected settlements, before a funding answer is
/// downgraded rather than trusted (AGENT-006 §5.6).
pub(crate) const FUNDING_GAP_DOWNGRADE_PCT: f64 = 10.0;

fn parse_ts(v: Option<&Value>) -> Option<f64> {
    let s = v?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp() as f64)
}

/// One funding settlement, reduced to what the arithmetic needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FundingPoint {
    pub t: f64,
    /// Fractional rate per settlement (`relative_rate`), when the venue gave one.
    pub rate: Option<f64>,
}

/// What a funding read turned into: the points, the cadence they settle at and
/// how much of the expected grid is missing. Never interpolated.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FundingGrid {
    pub points: Vec<FundingPoint>,
    pub cadence_s: f64,
    pub expected: usize,
    pub missing: usize,
    /// Grid instants with no settlement within half a cadence.
    pub missing_times: Vec<f64>,
}

impl FundingGrid {
    pub(crate) fn gap_pct(&self) -> f64 {
        if self.expected == 0 {
            0.0
        } else {
            self.missing as f64 / self.expected as f64 * 100.0
        }
    }
}

/// Read the `funding` array of `/api/data/funding` into a grid.
///
/// The cadence is the caller's `interval_hours` or, failing that, the median
/// spacing of the rows -- never a venue default, because the table this crate
/// would take it from has no verified source (OQ-17).
pub(crate) fn funding_grid(
    rows: &[Value],
    from: f64,
    to: f64,
    interval_hours: Option<f64>,
) -> Result<FundingGrid, Value> {
    let mut points: Vec<FundingPoint> = rows
        .iter()
        .filter_map(|r| {
            Some(FundingPoint {
                t: parse_ts(r.get("event_time"))?,
                rate: r
                    .get("relative_rate")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|x| x.is_finite()),
            })
        })
        .collect();
    points.sort_by(|a, b| a.t.total_cmp(&b.t));
    let cadence_s = match interval_hours {
        Some(h) if h > 0.0 => h * 3600.0,
        Some(h) => return Err(crate::tool_error::refuse_extra(
            "invalid_interval_hours",
            format!("interval_hours {h} is not a positive spacing."),
            "set interval_hours to the venue's settlement spacing in hours, above zero (e.g. 8)",
            json!({ "got": h }),
        )),
        None => {
            if points.len() < 3 {
                return Err(crate::tool_error::refuse(
                    "cadence_unknown",
                    "fewer than three settlements in the window, so the settlement spacing cannot be inferred, \
                     and this crate holds no verified venue default",
                    "pass interval_hours (e.g. 8 for an 8-hourly venue, 1 for hourly)",
                ));
            }
            let mut d: Vec<f64> = points.windows(2).map(|w| w[1].t - w[0].t).collect();
            d.sort_by(f64::total_cmp);
            d[d.len() / 2]
        }
    };
    if points.is_empty() {
        return Ok(FundingGrid {
            points,
            cadence_s,
            expected: 0,
            missing: 0,
            missing_times: vec![],
        });
    }
    // The grid is phased on the first observed settlement and extended both ways.
    let t0 = points[0].t;
    let first_k = ((from - t0) / cadence_s).ceil() as i64;
    let last_k = ((to - t0) / cadence_s).ceil() as i64 - 1;
    let mut expected = 0usize;
    let mut missing_times = Vec::new();
    for k in first_k..=last_k {
        let g = t0 + k as f64 * cadence_s;
        if g < from || g >= to {
            continue;
        }
        expected += 1;
        let covered = points.iter().any(|p| (p.t - g).abs() <= cadence_s / 2.0);
        if !covered {
            missing_times.push(g);
        }
    }
    Ok(FundingGrid {
        points,
        cadence_s,
        expected,
        missing: missing_times.len(),
        missing_times,
    })
}

fn window_of(resp: &Value) -> Option<(f64, f64)> {
    let w = resp.get("manifest")?.get("window")?;
    Some((parse_ts(w.get("from"))?, parse_ts(w.get("to"))?))
}

/// The funding response, reduced. Pure.
pub(crate) fn funding_history_result(resp: &Value, interval_hours: Option<f64>) -> Value {
    let manifest = resp.get("manifest").cloned().unwrap_or(Value::Null);
    let Some(rows) = resp.get("funding").and_then(Value::as_array) else {
        // `null`: the series is absent from the database. Not a rate of zero.
        return json!({
            "summary_fields": {"measured": false, "n_rows": 0, "data_gap_pct": "unknown", "annualized_mean_bps": "unknown"},
            "detail": {
                "measured": false,
                "unmeasured": manifest.get("unmeasured").cloned(),
                "annualized_mean_bps": null,
                "data_gap_pct": null,
                "message": "no funding series exists in this database. That is not a funding rate of zero: \
                            the carry over this window is unknown and a study that needs it must refuse.",
                "manifest": manifest,
            },
            "n": 0,
            "ids": {},
        });
    };
    let Some((from, to)) = window_of(resp) else {
        return crate::tool_error::refuse(
            "malformed_funding_response",
            "the funding response carried no manifest.window, so the grid cannot be placed.",
            "call again; if it repeats, report that the platform's funding history has no window",
        );
    };
    let grid = match funding_grid(rows, from, to, interval_hours) {
        Ok(g) => g,
        Err(e) => return e,
    };
    let n = grid.points.len();
    let with_rate: Vec<f64> = grid.points.iter().filter_map(|p| p.rate).collect();
    let periods_per_year = 365.0 * 24.0 * 3600.0 / grid.cadence_s;
    let annualized = if with_rate.is_empty() {
        None
    } else {
        Some(with_rate.iter().sum::<f64>() / with_rate.len() as f64 * periods_per_year * 1e4)
    };
    let gap = grid.gap_pct();
    let confidence = if n == 0 || gap > FUNDING_GAP_DOWNGRADE_PCT {
        "low"
    } else {
        "ok"
    };
    let tail: Vec<&Value> = rows
        .iter()
        .rev()
        .take(24)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    json!({
        "summary_fields": {
            "measured": true,
            "n_rows": n,
            "data_gap_pct": (gap * 10.0).round() / 10.0,
            "annualized_mean_bps": annualized.map_or(Value::Null, |a| json!((a * 10.0).round() / 10.0)),
        },
        "detail": {
            "measured": true,
            "n_rows": n,
            "cadence_hours": grid.cadence_s / 3600.0,
            "cadence_source": if interval_hours.is_some() { "caller" } else { "median spacing of the rows" },
            "expected_settlements": grid.expected,
            "missing_settlements": grid.missing,
            "data_gap_pct": gap,
            "confidence": confidence,
            "downgrade_note": if gap > FUNDING_GAP_DOWNGRADE_PCT {
                json!("more than 10% of expected settlements are missing; the answer is downgraded and nothing was interpolated")
            } else { Value::Null },
            "annualized_mean_bps": annualized,
            "annualized_note": if annualized.is_none() {
                json!("no row carried a relative_rate, and an absolute rate cannot be annualised without the price it is quoted against")
            } else { json!("mean relative_rate x settlements per year; positive means longs pay") },
            "rows_without_relative_rate": n - with_rate.len(),
            "forward_evidence_rows": manifest.get("forward_evidence_rows").cloned(),
            "latest_rows": tail,
            "manifest": manifest,
        },
        "n": n,
        "ids": {},
    })
}

/// `get_funding_history` -- realised funding for one perpetual, with its own gaps.
pub async fn get_funding_history(api: &ApiClient, params: &Value) -> Value {
    for f in ["instrument", "venue", "start", "end"] {
        if params
            .get(f)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return crate::tool_error::missing_field(f);
        }
    }
    let q = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("");
    let mut path = format!(
        "/api/data/funding?venue={}&instrument={}&from={}&to={}",
        urlencode(q("venue")),
        urlencode(q("instrument")),
        urlencode(q("start")),
        urlencode(q("end")),
    );
    if let Some(k) = params.get("kind").and_then(Value::as_str) {
        path.push_str(&format!("&kind={}", urlencode(k)));
    }
    let resp = match api.get(&path).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    funding_history_result(&resp, params.get("interval_hours").and_then(Value::as_f64))
}

/// Timeframe key to nanoseconds.
fn tf_ns(tf: &str) -> Option<i64> {
    let s: i64 = match tf {
        "1s" => 1,
        "1m" => 60,
        "5m" => 300,
        "15m" => 900,
        "1h" => 3600,
        "4h" => 14_400,
        "1d" => 86_400,
        _ => return None,
    };
    Some(s * 1_000_000_000)
}

/// Diff an as-of read against a latest read of the same window. Pure.
///
/// `ts_ns` on the bars route is the instant the bar **completed** (`LoadedBar::ts_ns`,
/// open + period), so a bar is visible at `as_of` only if `ts_ns <= as_of`.
///
/// Four findings, each a different defect: bars visible at `as_of` that had not
/// closed by then (the as-of filter is not filtering), bars whose values changed
/// between the reads (restatements), bars that exist later and not then (late
/// arrivals), and whether the early read hid anything at all.
pub(crate) fn pit_diff(early: &[Value], late: &[Value], as_of_ns: i64) -> Value {
    let key = |b: &Value| b.get("ts_ns").and_then(Value::as_i64);
    let sig = |b: &Value| {
        ["open", "high", "low", "close", "volume"]
            .iter()
            .map(|k| b.get(*k).and_then(Value::as_str).unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    let late_by: std::collections::BTreeMap<i64, Vec<String>> = late
        .iter()
        .filter_map(|b| Some((key(b)?, sig(b))))
        .collect();
    let early_keys: std::collections::BTreeSet<i64> = early.iter().filter_map(key).collect();

    let violations: Vec<i64> = early
        .iter()
        .filter_map(key)
        .filter(|t| *t > as_of_ns)
        .collect();
    let restated: Vec<i64> = early
        .iter()
        .filter_map(|b| {
            let t = key(b)?;
            let l = late_by.get(&t)?;
            (l != &sig(b)).then_some(t)
        })
        .collect();
    let late_arrivals: Vec<i64> = late_by
        .keys()
        .copied()
        .filter(|t| *t <= as_of_ns && !early_keys.contains(t))
        .collect();
    let hidden = late_by.keys().filter(|t| !early_keys.contains(t)).count();
    let bitemporal = if !violations.is_empty() {
        "no"
    } else if hidden > 0 {
        "yes"
    } else {
        "unknown"
    };
    let sample = |v: &[i64]| -> Vec<i64> { v.iter().copied().take(5).collect() };
    json!({
        "is_bitemporal": bitemporal,
        "restatement_count": restated.len(),
        "late_arrival_count": late_arrivals.len(),
        "violation_count": violations.len(),
        "sample_violations": sample(&violations),
        "sample_restatements": sample(&restated),
        "sample_late_arrivals": sample(&late_arrivals),
        "bars_early": early.len(),
        "bars_latest": late.len(),
        "bars_hidden_by_as_of": hidden,
        "worst_case_lag_days": null,
        "worst_case_lag_note": "unmeasured: the bars route does not serve knowledge_time, so a lag cannot be computed from here",
        "reading": match bitemporal {
            "no" => "bars that had not closed by the as-of instant were visible at it: the as-of filter is not filtering, and any backtest on this source can see the future",
            "yes" => "the as-of read hid bars the latest read shows, and showed none past its instant",
            _ => "the as-of read hid nothing, so this cannot tell a bitemporal store from one that ignores as_of; widen the window so it spans live ingestion",
        },
    })
}

/// Fold the bar-lag route's answer into the provenance detail. Pure.
///
/// `worst_case_lag_days` is the maximum `knowledge - close` over bars whose
/// knowledge time was received; with none received (every bar backfilled) it
/// stays null and says so, because a stamped knowledge time is not a lag.
pub(crate) fn apply_lag(detail: &mut Value, lag: &Value) {
    let max_s = lag.pointer("/lag_seconds/max").and_then(Value::as_f64);
    detail["backfilled_bars"] = lag.get("backfilled_bars").cloned().unwrap_or(Value::Null);
    detail["received_bars"] = lag.get("received_bars").cloned().unwrap_or(Value::Null);
    detail["lag_seconds"] = lag.get("lag_seconds").cloned().unwrap_or(Value::Null);
    match max_s {
        Some(s) => {
            detail["worst_case_lag_days"] = json!(s / 86_400.0);
            detail["worst_case_lag_note"] = json!(
                "maximum knowledge-minus-close over bars whose knowledge time was received; backfilled bars are counted separately"
            );
        }
        None => {
            detail["worst_case_lag_days"] = Value::Null;
            detail["worst_case_lag_note"] = json!(
                "unmeasured: no bar in the window carries a received knowledge time (all backfilled), and a backfill-stamped knowledge time is not a lag"
            );
        }
    }
}

/// `probe_pit_provenance` -- executes a representative as-of read and diffs it
/// against the latest read, rather than inspecting a schema.
pub async fn probe_pit_provenance(api: &ApiClient, params: &Value) -> Value {
    for f in ["instrument_id", "start", "end"] {
        if params
            .get(f)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return crate::tool_error::missing_field(f);
        }
    }
    let tf = params
        .get("timeframe")
        .and_then(Value::as_str)
        .unwrap_or("1h");
    if tf_ns(tf).is_none() {
        return crate::tool_error::refuse_extra(
            "unknown_timeframe",
            format!("timeframe `{tf}` is not one this tool reads."),
            "set timeframe to a bar size such as \"1m\", \"5m\", \"15m\", \"1h\", \"4h\" or \"1d\"",
            json!({ "timeframe": tf }),
        );
    }
    let q = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("");
    let (Some(s), Some(e)) = (
        chrono::DateTime::parse_from_rfc3339(q("start")).ok(),
        chrono::DateTime::parse_from_rfc3339(q("end")).ok(),
    ) else {
        return crate::tool_error::refuse(
            "invalid_window",
            "start or end is missing or not an RFC3339 instant.",
            "send start and end as RFC3339 instants, e.g. 2026-01-01T00:00:00Z",
        );
    };
    if e <= s {
        return crate::tool_error::refuse(
            "invalid_window",
            "end is not after start.",
            "make start the earlier instant and end the later one",
        );
    }
    let mid = s + (e - s) / 2;
    let base = format!(
        "/api/data/bars?instrument={}&tf={}&start={}&end={}&limit=50000",
        urlencode(q("instrument_id")),
        urlencode(tf),
        urlencode(q("start")),
        urlencode(q("end")),
    );
    let early = match api
        .get(&format!("{base}&as_of={}", urlencode(&mid.to_rfc3339())))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let late = match api.get(&base).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let rows = |v: &Value| {
        v.get("bars")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (early_rows, late_rows) = (rows(&early), rows(&late));
    if late_rows.is_empty() {
        return crate::tool_error::refuse_extra(
            "no_bars",
            "the window holds no bars, so there is nothing to probe",
            "widen start/end or pick an instrument and timeframe that have data",
            json!({ "instrument_id": q("instrument_id"), "timeframe": tf }),
        );
    }
    let as_of_ns = mid.timestamp_nanos_opt().unwrap_or(i64::MAX);
    let mut detail = pit_diff(&early_rows, &late_rows, as_of_ns);
    // The lag figure comes from the one place that can see knowledge_time. Failing
    // to get it leaves the field null with the reason, never zero.
    let lag_path = format!(
        "/api/agent/data/bar-lag?instrument={}&tf={}&start={}&end={}",
        urlencode(q("instrument_id")),
        urlencode(tf),
        urlencode(q("start")),
        urlencode(q("end")),
    );
    match api.get(&lag_path).await {
        Ok(l) => apply_lag(&mut detail, &l),
        Err(e) => {
            detail["worst_case_lag_note"] = json!(format!(
                "unmeasured: the bar-lag read failed ({})",
                e.to_tool_error()["error"]
            ));
        }
    }
    detail["as_of_probe"] = json!(mid.to_rfc3339());
    detail["window"] = json!({"start": q("start"), "end": q("end")});
    detail["truncated"] = json!(early_rows.len() >= 50_000 || late_rows.len() >= 50_000);
    json!({
        "summary_fields": {
            "is_bitemporal": detail["is_bitemporal"].clone(),
            "restatement_count": detail["restatement_count"].clone(),
            "violation_count": detail["violation_count"].clone(),
            "bars_latest": detail["bars_latest"].clone(),
        },
        "detail": detail,
        "n": late_rows.len(),
        "ids": {},
    })
}

/// One membership sample: the instant and its (instrument, status) members.
pub(crate) struct MembershipSample {
    pub as_of: String,
    pub members: Vec<(String, String)>,
}

/// Judge a universe from membership samples across a window. Pure.
///
/// An unauditable universe is reported as such. A membership that never changes
/// across the window is indistinguishable from a list of today's survivors back-
/// filled over history, so it is **not** reported clean.
pub(crate) fn survivorship_verdict(samples: &[MembershipSample]) -> Value {
    use std::collections::BTreeSet;
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        return crate::tool_error::refuse(
            "no_samples",
            "no membership samples were taken.",
            "widen start/end so at least one sample falls inside the window",
        );
    };
    fn set(s: &MembershipSample) -> BTreeSet<&str> {
        s.members.iter().map(|m| m.0.as_str()).collect()
    }
    let (a, b) = (set(first), set(last));
    let all: BTreeSet<&str> = samples
        .iter()
        .flat_map(|s| s.members.iter().map(|m| m.0.as_str()))
        .collect();
    let changed = samples.windows(2).any(|w| set(&w[0]) != set(&w[1]));
    let status_of = |inst: &str| -> String {
        samples
            .iter()
            .flat_map(|s| s.members.iter())
            .find(|m| m.0 == inst)
            .map_or("unknown".to_string(), |m| m.1.clone())
    };
    let dropped: Vec<Value> = a
        .difference(&b)
        .map(|i| json!({"instrument": i, "status": status_of(i)}))
        .collect();
    let added: Vec<&str> = b.difference(&a).copied().collect();
    let inactive_at_end: Vec<Value> = last
        .members
        .iter()
        .filter(|m| m.1 != "active")
        .map(|m| json!({"instrument": m.0, "status": m.1}))
        .collect();
    let unknown: Vec<&str> = all
        .iter()
        .copied()
        .filter(|i| status_of(i) == "unknown")
        .collect();
    let mut reasons: Vec<String> = Vec::new();
    if !changed {
        reasons.push(
            "membership is identical at every sampled instant. A static list cannot be told from \
             today's survivors written backwards over history, so survivorship cannot be audited \
             from it"
                .into(),
        );
    }
    if !unknown.is_empty() {
        reasons.push(format!(
            "{} member(s) have no listing status in the instrument registry: {}",
            unknown.len(),
            unknown.join(", ")
        ));
    }
    let verdict = if reasons.is_empty() {
        "audited"
    } else {
        "unauditable"
    };
    json!({
        "summary_fields": {
            "verdict": verdict,
            "assets_included": a.len(),
            "n_dropped": dropped.len(),
            "n_samples": samples.len(),
        },
        "detail": {
            "verdict": verdict,
            "unauditable_reasons": reasons,
            "assets_included": a.len(),
            "assets_at_end": b.len(),
            "delisted_missing": dropped,
            "added_during_window": added,
            "inactive_members_at_end": inactive_at_end,
            "sampled_as_of": samples.iter().map(|s| s.as_of.clone()).collect::<Vec<_>>(),
            "estimated_bias_bps": null,
            "estimated_bias_note": "not estimated: a bias figure needs the returns of the names that left, which this tool does not read",
        },
        "n": samples.len(),
        "ids": {},
    })
}

/// `audit_survivorship` -- is this universe's history free of survivors-only
/// selection, or can that not be told.
pub async fn audit_survivorship(api: &ApiClient, params: &Value) -> Value {
    for f in ["universe_id", "start", "end"] {
        if params
            .get(f)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return crate::tool_error::missing_field(f);
        }
    }
    let q = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("");
    let (Some(s), Some(e)) = (
        chrono::DateTime::parse_from_rfc3339(q("start")).ok(),
        chrono::DateTime::parse_from_rfc3339(q("end")).ok(),
    ) else {
        return crate::tool_error::refuse(
            "invalid_window",
            "start or end is missing or not an RFC3339 instant.",
            "send start and end as RFC3339 instants, e.g. 2026-01-01T00:00:00Z",
        );
    };
    if e <= s {
        return crate::tool_error::refuse(
            "invalid_window",
            "end is not after start.",
            "make start the earlier instant and end the later one",
        );
    }
    let points = [s, s + (e - s) / 4, s + (e - s) / 2, s + (e - s) * 3 / 4, e];
    let mut samples = Vec::new();
    for p in points {
        let as_of = p.to_rfc3339();
        let path = format!(
            "/api/data/universe/{}?as_of={}",
            urlencode(q("universe_id")),
            urlencode(&as_of)
        );
        let resp = match api.get(&path).await {
            Ok(v) => v,
            Err(err) => {
                // A universe the platform does not know is unauditable, said so.
                let mut out = err.to_tool_error();
                out["verdict"] = json!("unauditable");
                return out;
            }
        };
        let members = resp
            .get("members")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|m| {
                        (
                            m.get("instrument")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            m.get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        samples.push(MembershipSample { as_of, members });
    }
    survivorship_verdict(&samples)
}

/// Declarations for the three tools this module serves (AGENT-006 §2.3).
///
/// `get_bars` is `data`; the two registry lookups are `discovery`. The split is
/// the routing boundary, not the module boundary — a plan step that asks "what
/// exists" and one that asks "what did it do" want different slices.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    crate::result_shape::declared(vec![
        ToolSpec::new(
            "list_asset_classes",
            "discovery",
            Risk::Read,
            "List the asset classes the platform supports (crypto_spot_cex is the v1 focus).",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "get_instrument",
            "discovery",
            Risk::Read,
            "Fetch one instrument's registry row (venue, asset class, active flag).",
            json!({
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string", "description": "e.g. BTC-USD" }
                }
            }),
        )
        .internal()
        .evidence(&["instrument_id"]),
        ToolSpec::new(
            "get_bars",
            "data",
            Risk::Read,
            "Fetch real OHLCV bars for an instrument over a time range, with full-window summary \
             stats. Responses are capped: you get the summary plus the most recent max_bars bars.",
            json!({
                "type": "object",
                "required": ["instrument_id", "start", "end"],
                "properties": {
                    "instrument_id": { "type": "string", "description": "e.g. BTC-USD" },
                    "start": { "type": "string", "description": "RFC3339 UTC start" },
                    "end": { "type": "string", "description": "RFC3339 UTC end" },
                    "interval_seconds": { "type": "integer", "description": "Bar size in seconds (default 3600; 60=1m, 86400=1d). Aggregated from stored 1m bars." },
                    "max_bars": { "type": "integer", "description": "Max bars returned (default 300, cap 2000); earlier bars are summarized, not returned" }
                }
            }),
        )
        .internal()
        .notes("get_bars"),
        ToolSpec::new(
            "get_funding_history",
            "data",
            Risk::Read,
            "Realised funding for one perpetual over a window, with the share of expected \
             settlements that are missing and the annualised mean. An absent series answers \
             unmeasured, never a rate of zero.",
            json!({
                "type": "object",
                "required": ["instrument", "venue", "start", "end"],
                "properties": {
                    "instrument": { "type": "string" },
                    "venue": { "type": "string" },
                    "start": { "type": "string", "description": "RFC3339 UTC" },
                    "end": { "type": "string", "description": "RFC3339 UTC" },
                    "kind": { "type": "string", "enum": ["realized", "predicted"], "default": "realized" },
                    "interval_hours": { "type": "number", "description": "settlement spacing; omit to infer it from the rows" }
                }
            }),
        )
        .internal()
        .summary(
            "funding measured={measured}: {n_rows}, {data_gap_pct} missing, annualised mean {annualized_mean_bps}",
            &[("measured", ""), ("n_rows", "settlements"), ("data_gap_pct", "%"), ("annualized_mean_bps", "bps")],
        ),
        ToolSpec::new(
            "probe_pit_provenance",
            "data",
            Risk::Read,
            "Executes an as-of read of a bar series and diffs it against the latest read, to \
             find restatements, late arrivals and an as-of filter that does not filter. Run it \
             before trusting a source for a point-in-time claim.",
            json!({
                "type": "object",
                "required": ["instrument_id", "start", "end"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "start": { "type": "string", "description": "RFC3339 UTC" },
                    "end": { "type": "string", "description": "RFC3339 UTC" },
                    "timeframe": { "type": "string", "default": "1h" }
                }
            }),
        )
        .internal()
        .summary(
            "bitemporal: {is_bitemporal}; {restatement_count} restatements, {violation_count} future-visible bars over {bars_latest}",
            &[("is_bitemporal", ""), ("restatement_count", ""), ("violation_count", ""), ("bars_latest", "bars")],
        ),
        ToolSpec::new(
            "audit_survivorship",
            "data",
            Risk::Read,
            "Samples a universe's membership across a window and names the members that left. \
             A membership that never changes is reported unauditable, never clean.",
            json!({
                "type": "object",
                "required": ["universe_id", "start", "end"],
                "properties": {
                    "universe_id": { "type": "string" },
                    "start": { "type": "string", "description": "RFC3339 UTC" },
                    "end": { "type": "string", "description": "RFC3339 UTC" }
                }
            }),
        )
        .internal()
        .summary(
            "survivorship {verdict}: {assets_included} at start, {n_dropped} left, over {n_samples}",
            &[("verdict", ""), ("assets_included", "members"), ("n_dropped", "members"), ("n_samples", "samples")],
        ),
    ])
}

#[cfg(test)]
mod data_tool_tests {
    use super::*;

    fn rows(rates: &[(&str, &str)]) -> Vec<Value> {
        rates
            .iter()
            .map(|(t, r)| json!({"event_time": t, "relative_rate": r}))
            .collect()
    }

    fn resp(rows: Vec<Value>, from: &str, to: &str) -> Value {
        json!({"manifest": {"window": {"from": from, "to": to}, "forward_evidence_rows": 0}, "funding": rows})
    }

    /// The one property the route was built around, kept across the tool: an
    /// absent series is unknown, never a zero rate.
    #[test]
    fn an_absent_series_is_unmeasured_not_zero() {
        let out = funding_history_result(
            &json!({"manifest": {"unmeasured": {"reason": "no_such_series"}}, "funding": null}),
            None,
        );
        assert_eq!(out["detail"]["measured"], false);
        assert!(out["detail"]["annualized_mean_bps"].is_null());
        assert!(out["detail"]["data_gap_pct"].is_null());
    }

    #[test]
    fn a_complete_hourly_window_has_no_gap_and_annualises() {
        let r: Vec<(String, String)> = (0..24)
            .map(|h| (format!("2024-01-01T{h:02}:00:00Z"), "0.0001".to_string()))
            .collect();
        let rr: Vec<(&str, &str)> = r.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let out = funding_history_result(
            &resp(rows(&rr), "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z"),
            None,
        );
        assert_eq!(out["detail"]["data_gap_pct"], 0.0);
        // 1 bp per hour * 8760 hours = 8760 bps a year.
        let a = out["detail"]["annualized_mean_bps"].as_f64().unwrap();
        assert!((a - 8760.0).abs() < 1.0, "{a}");
        assert_eq!(out["detail"]["confidence"], "ok");
    }

    /// Missing settlements are counted against the grid and never filled.
    #[test]
    fn missing_settlements_downgrade_the_answer_and_are_not_interpolated() {
        let r: Vec<(String, String)> = (0..24)
            .filter(|h| h % 4 != 1) // drop six of 24
            .map(|h| (format!("2024-01-01T{h:02}:00:00Z"), "0.0001".to_string()))
            .collect();
        let rr: Vec<(&str, &str)> = r.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let out = funding_history_result(
            &resp(rows(&rr), "2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z"),
            Some(1.0),
        );
        assert_eq!(out["detail"]["missing_settlements"], 6);
        assert_eq!(out["detail"]["confidence"], "low");
        assert_eq!(out["detail"]["n_rows"], 18);
    }

    #[test]
    fn an_unknown_cadence_is_refused_not_defaulted() {
        let out = funding_history_result(
            &resp(
                rows(&[("2024-01-01T00:00:00Z", "0.0001")]),
                "2024-01-01T00:00:00Z",
                "2024-01-02T00:00:00Z",
            ),
            None,
        );
        assert_eq!(out["error"]["code"], "cadence_unknown");
    }

    fn bar(ts_ns: i64, close: &str) -> Value {
        json!({"ts_ns": ts_ns, "open": close, "high": close, "low": close, "close": close, "volume": "1"})
    }

    const H: i64 = 3_600_000_000_000;

    /// A source that ignores `as_of` shows bars that had not closed: the defect
    /// this probe exists to find.
    #[test]
    fn a_source_that_ignores_as_of_is_caught() {
        let late: Vec<Value> = (0..10).map(|i| bar(i * H, "1")).collect();
        let early = late.clone();
        let out = pit_diff(&early, &late, 5 * H);
        assert_eq!(out["is_bitemporal"], "no");
        assert!(out["violation_count"].as_u64().unwrap() > 0);
    }

    #[test]
    fn a_filtering_source_with_a_restatement_is_reported() {
        let late: Vec<Value> = (0..10)
            .map(|i| bar(i * H, if i == 2 { "9" } else { "1" }))
            .collect();
        let early: Vec<Value> = (0..5).map(|i| bar(i * H, "1")).collect();
        let out = pit_diff(&early, &late, 5 * H);
        assert_eq!(out["is_bitemporal"], "yes");
        assert_eq!(out["restatement_count"], 1);
        assert_eq!(out["violation_count"], 0);
    }

    /// Equal reads prove nothing about as-of filtering; it must not read "yes".
    #[test]
    fn the_lag_is_the_maximum_over_received_bars_and_null_when_none_were() {
        let mut d = json!({});
        apply_lag(
            &mut d,
            &json!({"backfilled_bars": 2, "received_bars": 3,
                                  "lag_seconds": {"max": 172800.0, "median": 60.0}}),
        );
        assert_eq!(d["worst_case_lag_days"], 2.0);
        let mut e = json!({});
        apply_lag(
            &mut e,
            &json!({"backfilled_bars": 5, "received_bars": 0,
                                  "lag_seconds": {"max": null}}),
        );
        assert!(e["worst_case_lag_days"].is_null());
        assert!(e["worst_case_lag_note"]
            .as_str()
            .unwrap()
            .contains("backfill"));
    }

    #[test]
    fn an_as_of_that_hides_nothing_is_unknown() {
        let late: Vec<Value> = (0..4).map(|i| bar(i * H, "1")).collect();
        let out = pit_diff(&late.clone(), &late, 100 * H);
        assert_eq!(out["is_bitemporal"], "unknown");
    }

    fn sample(as_of: &str, m: &[(&str, &str)]) -> MembershipSample {
        MembershipSample {
            as_of: as_of.into(),
            members: m.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect(),
        }
    }

    /// The acceptance: a static membership is unauditable, never a false clean.
    #[test]
    fn a_static_universe_is_unauditable_not_clean() {
        let s = vec![
            sample("a", &[("X", "active"), ("Y", "active")]),
            sample("b", &[("X", "active"), ("Y", "active")]),
        ];
        let out = survivorship_verdict(&s);
        assert_eq!(out["detail"]["verdict"], "unauditable");
        assert!(out["detail"]["estimated_bias_bps"].is_null());
    }

    #[test]
    fn a_universe_that_loses_a_delisted_member_is_audited_and_names_it() {
        let s = vec![
            sample("a", &[("X", "active"), ("Y", "delisted")]),
            sample("b", &[("X", "active")]),
        ];
        let out = survivorship_verdict(&s);
        assert_eq!(out["detail"]["verdict"], "audited");
        assert_eq!(out["detail"]["delisted_missing"][0]["instrument"], "Y");
        assert_eq!(out["detail"]["delisted_missing"][0]["status"], "delisted");
    }
}
