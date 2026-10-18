//! Read a model version's filled prediction series, and pair each answer with
//! what the market then did (S-3.16 `signal_half_life`, S-3.17
//! `conformal_calibrate`).
//!
//! # What the series is, and is not
//!
//! `model_prediction_series` (`clickhouse/09_model_prediction_series.sql`, Set U
//! U-5.5) holds every answer a model version gave, keyed by the instant it was
//! asked and a digest of its inputs. It holds **no realised outcome**: the model
//! was asked, it answered, and nothing recorded what happened next. So a label
//! is derived here from the bars, and the derivation is stated rather than
//! implied:
//!
//! * the decision at `at_ns` is made on the **last bar that had closed** by then
//!   (`open + timeframe <= at_ns`);
//! * the label over horizon `h` is `ln(close[i + h] / close[i])` for that bar `i`
//!   -- the return from the close the decision saw to the close `h` bars later;
//! * a label whose bar `i + h` is not in the bars handed in is `NaN`, never zero.
//!   The caller hands in bars clipped by the project's research cutoff, so a
//!   label past the cutoff is absent rather than peeked.
//!
//! This treats the model's scalar as a *forecast of the forward return*. A model
//! whose output means something else (a probability, a volatility) has no such
//! label, and the caller names the output it is treating as a return forecast.
//!
//! Only `status = 'answered'` rows with a finite number in the chosen output
//! enter; abstentions, timeouts and unavailable answers are counted, not
//! imputed.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One row of the series, reduced to what the arithmetic reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub at_ns: i64,
    pub y_hat: f64,
}

/// What was in the window, so an empty answer says why it is empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeriesCounts {
    pub rows: usize,
    pub answered: usize,
    /// Answered rows whose chosen output was not a finite number.
    pub not_numeric: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PredictionError {
    #[error("`{0}` must be `name@version` with a numeric version; an alias is not resolved here")]
    BadModel(String),
    #[error("the answered rows carry several numeric outputs ({0}); name one with `output`")]
    AmbiguousOutput(String),
    #[error("no answered row carries a numeric output named `{0}`")]
    NoSuchOutput(String),
    /// `model_prediction_series` does not exist in this database: `clickhouse/09`
    /// has not been applied here (it is applied at platform boot). Distinct from an
    /// empty read, which is a series nobody has filled.
    #[error("the `model_prediction_series` table does not exist in this ClickHouse database; apply clickhouse/09_model_prediction_series.sql (the platform does at boot)")]
    NoTable,
    #[error("clickhouse: {0}")]
    Store(String),
}

/// `name@version` to `(name, version)`.
///
/// # Errors
/// A reference without `@`, or whose version is not a number.
pub fn parse_model(reference: &str) -> Result<(String, u32), PredictionError> {
    let (name, version) = reference
        .split_once('@')
        .ok_or_else(|| PredictionError::BadModel(reference.to_string()))?;
    let v: u32 = version
        .parse()
        .map_err(|_| PredictionError::BadModel(reference.to_string()))?;
    if name.is_empty() {
        return Err(PredictionError::BadModel(reference.to_string()));
    }
    Ok((name.to_string(), v))
}

/// The numeric outputs of one stored `ModelOutcome` JSON document.
///
/// The stored shape is `{"outcome":"answered","outputs":{name:{"kind":"num","value":x}}}`
/// (`strategy_engine::models::ModelOutcome`, `domain::strategy_graph::Value`);
/// parsed from JSON rather than through those types so this crate need not
/// depend on the engine for a read.
#[must_use]
pub fn numeric_outputs(outcome_json: &str) -> Option<BTreeMap<String, f64>> {
    let v: serde_json::Value = serde_json::from_str(outcome_json).ok()?;
    if v.get("outcome")?.as_str()? != "answered" {
        return None;
    }
    let outputs = v.get("outputs")?.as_object()?;
    let mut out = BTreeMap::new();
    for (name, val) in outputs {
        if val.get("kind").and_then(|k| k.as_str()) == Some("num") {
            if let Some(x) = val.get("value").and_then(serde_json::Value::as_f64) {
                if x.is_finite() {
                    out.insert(name.clone(), x);
                }
            }
        }
    }
    Some(out)
}

/// Reduce raw `(at_ns, outcome_json)` rows to answers for one output. Pure.
///
/// # Errors
/// No numeric output of that name, or several and none chosen.
pub fn answers_from_rows(
    rows: &[(i64, String)],
    output: Option<&str>,
) -> Result<(Vec<Answer>, SeriesCounts), PredictionError> {
    let mut counts = SeriesCounts {
        rows: rows.len(),
        ..SeriesCounts::default()
    };
    let mut parsed: Vec<(i64, BTreeMap<String, f64>)> = Vec::new();
    for (at, json) in rows {
        let Some(outs) = numeric_outputs(json) else {
            continue;
        };
        counts.answered += 1;
        parsed.push((*at, outs));
    }
    let chosen: String = if let Some(o) = output {
        o.to_string()
    } else {
        let mut names: Vec<&String> = parsed.iter().flat_map(|(_, m)| m.keys()).collect();
        names.sort();
        names.dedup();
        match names.as_slice() {
            [only] => (*only).clone(),
            [] => return Err(PredictionError::NoSuchOutput("(any)".into())),
            many => {
                return Err(PredictionError::AmbiguousOutput(
                    many.iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ))
            }
        }
    };
    let mut answers = Vec::new();
    for (at, outs) in parsed {
        match outs.get(&chosen) {
            Some(x) => answers.push(Answer {
                at_ns: at,
                y_hat: *x,
            }),
            None => counts.not_numeric += 1,
        }
    }
    if answers.is_empty() {
        return Err(PredictionError::NoSuchOutput(chosen));
    }
    answers.sort_by_key(|a| a.at_ns);
    answers.dedup_by_key(|a| a.at_ns);
    Ok((answers, counts))
}

/// One bar, as `(open_ns, close)`.
pub type BarClose = (i64, f64);

/// Forward log returns for each answer and horizon. Pure.
///
/// `bars` must be in time order. Row `i` of the result is the answer's labels,
/// one per horizon, `NaN` where the bar `h` ahead is not present.
#[must_use]
pub fn forward_returns(
    answers: &[Answer],
    bars: &[BarClose],
    bar_ns: i64,
    horizons: &[usize],
) -> Vec<Vec<f64>> {
    answers
        .iter()
        .map(|a| {
            // Last bar that had closed by the decision instant.
            let i = bars.partition_point(|(open, _)| open + bar_ns <= a.at_ns);
            horizons
                .iter()
                .map(|&h| {
                    if i == 0 {
                        return f64::NAN;
                    }
                    let (entry, exit) = (bars.get(i - 1), bars.get(i - 1 + h));
                    match (entry, exit) {
                        (Some((_, c0)), Some((_, c1))) if *c0 > 0.0 && *c1 > 0.0 => (c1 / c0).ln(),
                        _ => f64::NAN,
                    }
                })
                .collect()
        })
        .collect()
}

#[derive(clickhouse::Row, Deserialize)]
struct SeriesRow {
    at_ns: i64,
    outcome: String,
}

/// Read the answered rows of one model version in `[from_ns, to_ns)`.
///
/// `FINAL` because the table is a `ReplacingMergeTree`: a recomputed row
/// replaces the old one, and an unmerged duplicate would be double-counted.
///
/// # Errors
/// Any ClickHouse error.
pub async fn read_series(
    clickhouse_url: &str,
    model_id: &str,
    version: u32,
    from_ns: i64,
    to_ns: i64,
) -> Result<Vec<(i64, String)>, PredictionError> {
    let client = storage::clickhouse::connect(clickhouse_url);
    let rows: Vec<SeriesRow> = client
        .query(
            "SELECT at_ns, outcome FROM model_prediction_series FINAL \
             WHERE model_id = ? AND version = ? AND at_ns >= ? AND at_ns < ? \
             ORDER BY at_ns",
        )
        .bind(model_id)
        .bind(version)
        .bind(from_ns)
        .bind(to_ns)
        .fetch_all()
        .await
        .map_err(|e| {
            let m = e.to_string();
            if m.contains("UNKNOWN_TABLE") || m.contains("Unknown table") {
                PredictionError::NoTable
            } else {
                PredictionError::Store(m)
            }
        })?;
    Ok(rows.into_iter().map(|r| (r.at_ns, r.outcome)).collect())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000_000_000;

    fn row(at: i64, name: &str, x: f64) -> (i64, String) {
        (
            at,
            format!(
                r#"{{"outcome":"answered","outputs":{{"{name}":{{"kind":"num","value":{x}}}}}}}"#
            ),
        )
    }

    #[test]
    fn a_model_reference_names_a_numeric_version() {
        assert_eq!(parse_model("vol@3"), Ok(("vol".into(), 3)));
        assert!(parse_model("vol").is_err());
        assert!(parse_model("vol@latest").is_err());
        assert!(parse_model("@3").is_err());
    }

    #[test]
    fn abstentions_are_counted_and_never_imputed() {
        let rows = vec![
            row(1, "p", 0.5),
            (2, r#"{"outcome":"abstained","detail":"no"}"#.to_string()),
            row(3, "p", -0.5),
        ];
        let (a, c) = answers_from_rows(&rows, None).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!((c.rows, c.answered), (3, 2));
    }

    #[test]
    fn several_outputs_demand_a_choice() {
        let rows = vec![(
            1,
            r#"{"outcome":"answered","outputs":{"a":{"kind":"num","value":1.0},"b":{"kind":"num","value":2.0}}}"#.to_string(),
        )];
        assert!(matches!(
            answers_from_rows(&rows, None),
            Err(PredictionError::AmbiguousOutput(_))
        ));
        assert_eq!(answers_from_rows(&rows, Some("b")).unwrap().0[0].y_hat, 2.0);
        assert!(matches!(
            answers_from_rows(&rows, Some("zz")),
            Err(PredictionError::NoSuchOutput(_))
        ));
    }

    /// The decision sees the last *closed* bar, and the label runs from its close.
    #[test]
    fn a_label_runs_from_the_last_closed_bar_and_is_nan_past_the_data() {
        // Bars open at 0,1,2,3 hours with closes 100, 110, 121, 133.1.
        let bars: Vec<BarClose> = vec![(0, 100.0), (H, 110.0), (2 * H, 121.0), (3 * H, 133.1)];
        // Decision at t=2h: bars 0 and 1 have closed (open+1h <= 2h); the last is bar 1.
        let a = vec![Answer {
            at_ns: 2 * H,
            y_hat: 0.0,
        }];
        let f = forward_returns(&a, &bars, H, &[1, 2, 5]);
        assert!((f[0][0] - (121.0f64 / 110.0).ln()).abs() < 1e-12);
        assert!((f[0][1] - (133.1f64 / 110.0).ln()).abs() < 1e-12);
        assert!(
            f[0][2].is_nan(),
            "a horizon past the bars is absent, not zero"
        );
    }

    #[test]
    fn a_decision_before_any_bar_closed_has_no_label() {
        let bars: Vec<BarClose> = vec![(H, 100.0), (2 * H, 101.0)];
        let a = vec![Answer {
            at_ns: 0,
            y_hat: 0.0,
        }];
        assert!(forward_returns(&a, &bars, H, &[1])[0][0].is_nan());
    }
}
