//! The retention pass: the caller `dataplane::retention::retention_action` never had
//! (Set P P-0.6, clause 2).
//!
//! `retention_action` decided, correctly and under test, what to do when a forward
//! series approached its disk budget — and nothing in the workspace ever asked it.
//! A forward series would have reached its budget and nothing would have
//! downsampled it, while `over_budget_never_deletes` went on passing. This module
//! measures each registered series' disk use, asks the decision, and carries it out.
//!
//! # What "downsampled, not deleted" means here
//!
//! For `option_chain_snapshots`, the only table whose registry rows carry a
//! downsampling policy: beyond `downsample_after_days`, every contract keeps
//! **exactly one row per `downsample_to_secs` bucket** — the last observation in
//! that bucket, re-labelled `snapshot_cadence = 'daily'` — and the finer rows in the
//! same bucket are removed. The series stays continuous: every `(contract, bucket)`
//! that held an observation before the pass holds one after it, which is the
//! property [`DownsampleReport::continuity_held`] checks on the live table and the
//! test asserts. Nothing older than the budget is dropped wholesale; there is no
//! code path here that removes a bucket.
//!
//! The kept row is the bucket's **last** observation, not its first or an average:
//! it is a real observation the venue published (IV is a stored quantity, INV-07,
//! and an averaged IV is a number nobody quoted), and the end-of-bucket snapshot
//! is what a daily series means everywhere else in the platform.
//!
//! A series with policy `none` that crosses its high-water mark is **not touched**:
//! the decision comes back `BudgetExceededNeedsDecision`, the pass reports it, and a
//! human decides. That is `retention_action`'s rule, carried out rather than
//! restated.

use chrono::{DateTime, Duration, Utc};
use clickhouse::Row;
use dataplane::retention::{retention_action, ForwardSeriesSpec, RetentionAction, RetentionPolicy};
use serde::Deserialize;

use super::{connect, ChError};

/// The one table this pass knows how to coarsen.
pub const CHAIN_TABLE: &str = "option_chain_snapshots";

/// What one series' pass did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeriesOutcome {
    /// Under the high-water mark.
    Kept { series_id: String, used_bytes: u64 },
    /// Coarsened.
    Downsampled {
        series_id: String,
        used_bytes: u64,
        report: DownsampleReport,
    },
    /// Over budget with no rule that helps. Reported, never acted on.
    NeedsDecision {
        series_id: String,
        used_bytes: u64,
        budget_bytes: u64,
    },
    /// The policy asks to downsample a table this pass does not know how to
    /// coarsen. Reported rather than guessed at.
    Unsupported { series_id: String, table: String },
    /// Disabled in the registry.
    Disabled { series_id: String },
}

/// The effect of one downsampling run, measured on the table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DownsampleReport {
    /// Fine rows in scope before the pass.
    pub fine_rows_before: u64,
    /// `(contract, bucket)` cells in scope before the pass.
    pub buckets_before: u64,
    /// The same cells after the pass. Equal to `buckets_before`, or the pass
    /// broke the series.
    pub buckets_after: u64,
    /// Rows promoted to the coarse cadence.
    pub promoted: u64,
    /// Fine rows remaining in scope after the pass (should be 0).
    pub fine_rows_after: u64,
}

impl DownsampleReport {
    /// Every bucket that had an observation still has one.
    #[must_use]
    pub const fn continuity_held(&self) -> bool {
        self.buckets_before == self.buckets_after
    }
}

#[derive(Row, Deserialize)]
struct Count {
    n: u64,
}

async fn count(client: &clickhouse::Client, sql: &str) -> Result<u64, ChError> {
    client
        .query(sql)
        .fetch_one::<Count>()
        .await
        .map(|c| c.n)
        .map_err(|e| ChError::Client(e.to_string()))
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Estimated bytes on disk for one venue's rows of `table`.
///
/// `system.parts` reports a table, not a venue, and two registry series can share
/// one table (crypto and equity chains). The table's bytes are apportioned by row
/// count — an estimate, stated as one, and the direction of its error does not
/// matter much at an 80 % high-water mark.
///
/// # Errors
/// Backend failures.
pub async fn venue_bytes(url: &str, table: &str, venue: &str) -> Result<u64, ChError> {
    let client = connect(url);
    let total_bytes = count(
        &client,
        &format!(
            "SELECT toUInt64(sum(bytes_on_disk)) AS n FROM system.parts
              WHERE active AND database = currentDatabase() AND table = '{}'",
            esc(table)
        ),
    )
    .await?;
    if total_bytes == 0 {
        return Ok(0);
    }
    let total_rows = count(&client, &format!("SELECT count() AS n FROM {table}")).await?;
    if total_rows == 0 {
        return Ok(0);
    }
    let venue_rows = count(
        &client,
        &format!(
            "SELECT count() AS n FROM {table} WHERE venue = '{}'",
            esc(venue)
        ),
    )
    .await?;
    Ok(((u128::from(total_bytes) * u128::from(venue_rows)) / u128::from(total_rows)) as u64)
}

/// Coarsen one venue's chain snapshots older than `older_than` to one row per
/// contract per `to_secs` bucket.
///
/// Two mutations, run synchronously (`mutations_sync = 2`) so the report measures
/// the finished state: the last observation of each `(contract, bucket)` is
/// re-labelled coarse, then the remaining fine rows in scope are removed. The
/// order matters — label first, so that at no instant is a bucket empty.
///
/// # Errors
/// Backend failures, or a pass that would have broken continuity (checked after,
/// and reported as an error rather than a success with a bad number).
pub async fn downsample_chain_snapshots(
    url: &str,
    venue: &str,
    older_than: DateTime<Utc>,
    to_secs: u32,
) -> Result<DownsampleReport, ChError> {
    if to_secs == 0 {
        return Err(ChError::Client("to_secs must be positive".into()));
    }
    let client = connect(url);
    let v = esc(venue);
    let cutoff = older_than.timestamp();
    let scope = format!(
        "venue = '{v}' AND event_time < fromUnixTimestamp64Nano(toInt64({cutoff}) * 1000000000)"
    );
    let contract = "underlying, instrument, expiry, option_right, strike";
    let bucket = format!("toStartOfInterval(event_time, INTERVAL {to_secs} SECOND)");

    let fine_rows_before = count(
        &client,
        &format!("SELECT count() AS n FROM {CHAIN_TABLE} FINAL WHERE {scope} AND snapshot_cadence = 'hourly'"),
    )
    .await?;
    let cells = format!(
        "SELECT count() AS n FROM (SELECT {contract}, {bucket} AS b FROM {CHAIN_TABLE} FINAL WHERE {scope} GROUP BY {contract}, b)"
    );
    let buckets_before = count(&client, &cells).await?;
    if fine_rows_before == 0 {
        return Ok(DownsampleReport {
            buckets_before,
            buckets_after: buckets_before,
            ..DownsampleReport::default()
        });
    }

    // 1. Promote the last fine observation of each cell that has no coarse row yet.
    let promote = format!(
        "ALTER TABLE {CHAIN_TABLE} UPDATE snapshot_cadence = 'daily'
          WHERE {scope} AND snapshot_cadence = 'hourly'
            AND (underlying, instrument, expiry, option_right, strike, event_time) IN (
                SELECT {contract}, max(event_time) FROM {CHAIN_TABLE}
                 WHERE {scope}
                 GROUP BY {contract}, {bucket}
                HAVING countIf(snapshot_cadence != 'hourly') = 0)
          SETTINGS mutations_sync = 2"
    );
    let fine_before_promote = fine_rows_before;
    client
        .query(&promote)
        .execute()
        .await
        .map_err(|e| ChError::Client(format!("promote: {e}")))?;
    let fine_after_promote = count(
        &client,
        &format!(
            "SELECT count() AS n FROM {CHAIN_TABLE} WHERE {scope} AND snapshot_cadence = 'hourly'"
        ),
    )
    .await?;
    let promoted = fine_before_promote.saturating_sub(fine_after_promote);

    // 2. Remove the remaining fine rows in scope: every cell now has its coarse row.
    client
        .query(&format!(
            "ALTER TABLE {CHAIN_TABLE} DELETE WHERE {scope} AND snapshot_cadence = 'hourly'
              SETTINGS mutations_sync = 2"
        ))
        .execute()
        .await
        .map_err(|e| ChError::Client(format!("coarsen: {e}")))?;

    let buckets_after = count(&client, &cells).await?;
    let fine_rows_after = count(
        &client,
        &format!(
            "SELECT count() AS n FROM {CHAIN_TABLE} WHERE {scope} AND snapshot_cadence = 'hourly'"
        ),
    )
    .await?;
    let report = DownsampleReport {
        fine_rows_before,
        buckets_before,
        buckets_after,
        promoted,
        fine_rows_after,
    };
    if !report.continuity_held() {
        return Err(ChError::Client(format!(
            "downsampling broke continuity: {buckets_before} (contract, bucket) cells before, {buckets_after} after"
        )));
    }
    Ok(report)
}

/// Build the pure spec from a registry row.
///
/// # Errors
/// A registry row whose policy is not one `retention` knows, or is incomplete.
pub fn spec_of(r: &observability::silence::RegistryRow) -> Result<ForwardSeriesSpec, ChError> {
    let retention = match r.retention_policy.as_str() {
        "none" => RetentionPolicy::None,
        "downsample" => RetentionPolicy::Downsample {
            after_days: r
                .downsample_after_days
                .and_then(|d| u32::try_from(d).ok())
                .ok_or_else(|| {
                    ChError::Decode(format!("{}: downsample without after_days", r.series_id))
                })?,
            to_secs: r
                .downsample_to_secs
                .and_then(|d| u32::try_from(d).ok())
                .ok_or_else(|| {
                    ChError::Decode(format!("{}: downsample without to_secs", r.series_id))
                })?,
        },
        other => {
            return Err(ChError::Decode(format!(
                "{}: unknown retention policy {other:?}",
                r.series_id
            )))
        }
    };
    Ok(ForwardSeriesSpec {
        series_id: r.series_id.clone(),
        store_table: r.store_table.clone(),
        venue: r.venue.clone(),
        cadence_secs: u32::try_from(r.cadence_secs).unwrap_or(0),
        alarm_after_cadences: u32::try_from(r.alarm_after_cadences).unwrap_or(0),
        disk_budget_bytes: u64::try_from(r.disk_budget_bytes).unwrap_or(0),
        retention,
        enabled: r.enabled,
    })
}

/// One series, given its measured use: decide with `retention_action`, then act.
///
/// # Errors
/// Backend failures while acting.
pub async fn apply(
    url: &str,
    spec: &ForwardSeriesSpec,
    used_bytes: u64,
    now: DateTime<Utc>,
) -> Result<SeriesOutcome, ChError> {
    if !spec.enabled {
        return Ok(SeriesOutcome::Disabled {
            series_id: spec.series_id.clone(),
        });
    }
    match retention_action(spec, used_bytes) {
        RetentionAction::Keep => Ok(SeriesOutcome::Kept {
            series_id: spec.series_id.clone(),
            used_bytes,
        }),
        RetentionAction::BudgetExceededNeedsDecision {
            used_bytes,
            budget_bytes,
        } => Ok(SeriesOutcome::NeedsDecision {
            series_id: spec.series_id.clone(),
            used_bytes,
            budget_bytes,
        }),
        RetentionAction::Downsample {
            older_than_days,
            to_secs,
        } => {
            if spec.store_table != CHAIN_TABLE {
                return Ok(SeriesOutcome::Unsupported {
                    series_id: spec.series_id.clone(),
                    table: spec.store_table.clone(),
                });
            }
            let report = downsample_chain_snapshots(
                url,
                &spec.venue,
                now - Duration::days(i64::from(older_than_days)),
                to_secs,
            )
            .await?;
            Ok(SeriesOutcome::Downsampled {
                series_id: spec.series_id.clone(),
                used_bytes,
                report,
            })
        }
    }
}

/// The whole pass over the registry. Never returns `Err` for one series' failure:
/// each series' result stands alone, so one bad row cannot stop the others.
pub async fn retention_pass(
    pg: &sqlx::PgPool,
    url: &str,
    now: DateTime<Utc>,
) -> Result<Vec<Result<SeriesOutcome, ChError>>, ChError> {
    let rows = super::census::registry(pg).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let res = async {
            let spec = spec_of(r)?;
            let used = venue_bytes(url, &spec.store_table, &spec.venue).await?;
            apply(url, &spec, used, now).await
        }
        .await;
        out.push(res);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        policy: &str,
        after: Option<i32>,
        to: Option<i32>,
    ) -> observability::silence::RegistryRow {
        observability::silence::RegistryRow {
            series_id: "equity.option_chains".into(),
            store_table: CHAIN_TABLE.into(),
            venue: "tradier".into(),
            cadence_secs: 3600,
            alarm_after_cadences: 3,
            disk_budget_bytes: 1000,
            retention_policy: policy.into(),
            downsample_after_days: after,
            downsample_to_secs: to,
            enabled: true,
            arrival: None,
        }
    }

    #[test]
    fn a_registry_row_maps_to_the_spec_retention_decides_on() {
        let s = spec_of(&row("downsample", Some(90), Some(86_400))).unwrap();
        assert_eq!(
            s.retention,
            RetentionPolicy::Downsample {
                after_days: 90,
                to_secs: 86_400
            }
        );
        assert!(spec_of(&row("downsample", None, Some(86_400))).is_err());
        assert!(spec_of(&row("delete", None, None)).is_err());
    }

    /// Over budget with policy `none`: reported, nothing is touched. The URL is
    /// unreachable, so any attempt to act would error rather than pass.
    #[tokio::test]
    async fn over_budget_without_a_rule_is_a_decision_for_a_human() {
        let spec = spec_of(&row("none", None, None)).unwrap();
        let out = apply("http://127.0.0.1:1/none", &spec, 990, Utc::now())
            .await
            .unwrap();
        assert!(
            matches!(out, SeriesOutcome::NeedsDecision { .. }),
            "{out:?}"
        );
        let out = apply("http://127.0.0.1:1/none", &spec, 10, Utc::now())
            .await
            .unwrap();
        assert!(matches!(out, SeriesOutcome::Kept { .. }), "{out:?}");
    }

    #[tokio::test]
    async fn a_downsample_rule_on_an_unknown_table_is_reported_not_guessed() {
        let mut r = row("downsample", Some(90), Some(86_400));
        r.store_table = "funding_rates".into();
        let spec = spec_of(&r).unwrap();
        let out = apply("http://127.0.0.1:1/none", &spec, 990, Utc::now())
            .await
            .unwrap();
        assert!(matches!(out, SeriesOutcome::Unsupported { .. }), "{out:?}");
    }
}
