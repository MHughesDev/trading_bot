//! Concrete job workers (COMP-005 §12).
//!
//! Each worker is a pure function of its manifest: same manifest, same pinned data,
//! same result. That is what makes a job safe to re-queue after a lost lease.

use serde_json::{json, Value};

use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};

/// Data quality-control over stored bars (DATA-005 §7, `data_qc` kind).
///
/// Grades an instrument's bar coverage so that Experiments can be refused on bad
/// data before anyone spends a trial on it. The checks are deliberately about the
/// shape of the data rather than its values: gaps, duplicate timestamps, zero-range
/// bars and stale runs are the failures that silently produce beautiful backtests.
pub struct DataQcWorker {
    clickhouse_url: String,
    /// When present, the job also runs the eight-check battery
    /// (`dataplane::qc`) and persists a `data_qc_reports` row (Set P P-2.2).
    pg: Option<sqlx::PgPool>,
}

impl DataQcWorker {
    pub fn new(clickhouse_url: String) -> Self {
        Self {
            clickhouse_url,
            pg: None,
        }
    }

    /// Give the worker the database the battery's reports are written to.
    ///
    /// Without it the worker can only run the four-threshold summary below, whose
    /// letters are not the ones the admission gate will read; `main.rs` passes it.
    #[must_use]
    pub fn with_pool(mut self, pg: sqlx::PgPool) -> Self {
        self.pg = Some(pg);
        self
    }
}

/// A QC grade (DATA-005 §7).
///
/// The thresholds live in `data_admission` alongside the gate that acts on them
/// (DA-07). Keeping them in one place is the point: a grader and an admission rule
/// that drift apart produce a "C" that nothing refuses.
use crate::data_admission::grade;

#[async_trait::async_trait]
impl Worker for DataQcWorker {
    fn kind(&self) -> JobKind {
        JobKind::DataQc
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        // A handful of aggregate queries over one instrument.
        Cost {
            compute_s: Some(5.0),
            gpu_s: None,
            cost_usd: Some(0.0),
        }
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let instrument = manifest
            .get("instrument_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "invalid_manifest",
                    "data_qc needs an instrument_id in its manifest",
                )
            })?;
        let timeframe = manifest
            .get("timeframe")
            .and_then(Value::as_str)
            .unwrap_or("1m");

        ctx.progress(Progress {
            pct: Some(0.1),
            stage: Some("querying".into()),
            message: None,
        })
        .await;

        // Through the single PIT reader: QC sees exactly what a strategy could.
        let tf = <domain::payloads::bar::Timeframe as backtest::TimeframeExt>::from_key(timeframe)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "unknown_timeframe",
                    format!("no known period for timeframe {timeframe:?}"),
                )
            })?;
        let profile = backtest::BarStore::connect(&self.clickhouse_url)
            .quality_profile(instrument, tf)
            .await
            .map_err(|e| JobError {
                code: "clickhouse_query_failed".into(),
                terminal: ledger::TerminalReason::DependencyFailure,
                field: None,
                rule: None,
                fix: Some(format!("{e}")),
                detail_ref: None,
                retryable: true,
            })?;
        let backtest::store::QualityProfile {
            rows,
            distinct_bars,
            first_s,
            last_s,
            flat_bars,
            backfilled,
            flagged,
        } = profile;

        if distinct_bars == 0 {
            return Err(JobError::logic(
                // Nothing is wrong with the request; the bars are not there.
                ledger::TerminalReason::DataError,
                "no_data",
                format!("no bars stored for {instrument} at {timeframe}"),
            ));
        }

        ctx.progress(Progress {
            pct: Some(0.7),
            stage: Some("grading".into()),
            message: None,
        })
        .await;

        let period_s = timeframe_seconds(timeframe).ok_or_else(|| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "unknown_timeframe",
                format!("no known period for timeframe {timeframe:?}"),
            )
        })?;

        let span_s = (last_s - first_s).max(0) as f64;
        let expected = if span_s > 0.0 {
            (span_s / period_s as f64) + 1.0
        } else {
            1.0
        };
        let coverage_pct = (distinct_bars as f64 / expected * 100.0).min(100.0);
        let gap_pct = (100.0 - coverage_pct).max(0.0);
        let flat_pct = flat_bars as f64 / distinct_bars as f64 * 100.0;
        // Extra rows for the same bar: late revisions, plus any re-collection not
        // yet merged away. Not corruption (v2 keeps each revision as its own row),
        // but a large number means the same range is being re-collected.
        let repeat_rows = rows.saturating_sub(distinct_bars);

        let legacy_assessment = grade(coverage_pct, gap_pct, flat_pct);

        // The battery (Set P P-2.2): the grade a manifest carries and an
        // Experiment is admitted on. Needs a `venue` in the manifest because
        // a persisted report is per series, and a pool to persist it to.
        // `expected_bars` is the continuous count over the window, so a series
        // missing its ends is graded as missing them; a session venue's closed
        // hours would read as gaps, which is why the manifest may narrow
        // the window with `from` / `to`.
        let battery = match (&self.pg, manifest.get("venue").and_then(Value::as_str)) {
            (Some(pg), Some(venue)) => {
                let secs = |k: &str| manifest.get(k).and_then(Value::as_i64);
                let from = chrono::DateTime::from_timestamp(secs("from").unwrap_or(first_s), 0);
                let to = chrono::DateTime::from_timestamp(
                    secs("to").unwrap_or(last_s + period_s),
                    0,
                );
                let (Some(from), Some(to)) = (from, to) else {
                    return Err(JobError::logic(
                        ledger::TerminalReason::IntegrityRejected,
                        "invalid_manifest",
                        "data_qc `from` / `to` are not valid instants",
                    ));
                };
                let expected = u64::try_from(((to - from).num_seconds() / period_s).max(0))
                    .unwrap_or(0);
                let (id, report) = storage::clickhouse::qc_reports::grade_and_persist(
                    pg,
                    &self.clickhouse_url,
                    instrument,
                    venue,
                    timeframe,
                    u32::try_from(period_s).unwrap_or(0),
                    from,
                    to,
                    expected,
                    &dataplane::qc::QcConfig::default(),
                )
                .await
                .map_err(|e| JobError {
                    code: "qc_report_failed".into(),
                    terminal: ledger::TerminalReason::DependencyFailure,
                    field: None,
                    rule: None,
                    fix: Some(format!("{e}")),
                    detail_ref: None,
                    retryable: true,
                })?;
                Some((id, report))
            }
            _ => None,
        };
        let assessment = battery
            .as_ref()
            .map_or(legacy_assessment, |(_, r)| r.graded.grade.as_str());

        // ASCII in the summary: a model reads it, and so do developer terminals,
        // some of which are cp1252 and raise on an em dash.
        let summary = format!(
            "{instrument} {timeframe}: grade {assessment} - {distinct_bars} bars, \
             {coverage_pct:.1}% coverage, {flat_pct:.1}% flat, {repeat_rows} repeat rows"
        );

        Ok(JobOutput {
            summary: Some(summary),
            result: json!({
                "instrument_id": instrument,
                "timeframe": timeframe,
                "grade": assessment,
                "legacy_grade": legacy_assessment,
                "report_id": battery.as_ref().map(|(id, _)| id.to_string()),
                "grade_source": if battery.is_some() { "battery" } else { "legacy_thresholds" },
                "backfilled_knowledge_bars": backfilled,
                "quality_flagged_bars": flagged,
                "bars": distinct_bars,
                "rows": rows,
                "repeat_rows": repeat_rows,
                "coverage_pct": coverage_pct,
                "gap_pct": gap_pct,
                "flat_pct": flat_pct,
                "first_event_time": first_s,
                "last_event_time": last_s,
            }),
            artifacts: vec![],
            actual: Cost {
                compute_s: Some(1.0),
                gpu_s: None,
                cost_usd: Some(0.0),
            },
        })
    }
}

/// Bar period in seconds.
///
/// Mirrors `backtest::types::TimeframeExt`; an unknown timeframe is an error rather
/// than a guess, because a wrong period silently turns into a wrong coverage figure
/// and therefore a wrong grade.
fn timeframe_seconds(key: &str) -> Option<i64> {
    match key {
        "1s" => Some(1),
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "1h" => Some(3_600),
        "4h" => Some(14_400),
        "1d" => Some(86_400),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_run_from_a_to_d() {
        assert_eq!(grade(99.9, 0.1, 0.0), "A");
        assert_eq!(grade(97.0, 3.0, 0.0), "B");
        assert_eq!(grade(85.0, 15.0, 0.0), "C");
        assert_eq!(grade(40.0, 60.0, 0.0), "D");
    }

    #[test]
    fn flat_bars_alone_can_cost_a_grade() {
        // A long run of bars where high == low is usually a dead feed rather than a
        // quiet market, and it is exactly the shape that produces a flawless-looking
        // backtest with no trades to lose on.
        assert_eq!(grade(99.9, 0.1, 25.0), "C");
        assert_eq!(grade(99.9, 0.1, 6.0), "B");
    }

    #[test]
    fn unknown_timeframes_have_no_period() {
        assert_eq!(timeframe_seconds("1m"), Some(60));
        assert_eq!(timeframe_seconds("2m"), None);
    }
}
