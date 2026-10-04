//! Watched screeners run on their own schedule (Set U U-6.3; migration 0187).
//!
//! A selection bound to a deployment runs through its allocator. A screener
//! someone only watches is a `selection_watch` row: its pinned revision and its
//! `triggers.schedule`. Each minute, every running watch whose schedule fires
//! that minute is run as of that minute through the same
//! [`backtest::selection_run::run_selection`] the route and the allocator use,
//! and journaled with `requested_by = 'schedule'` — so a watched screener's
//! list is the list a person or the agent would get for that instant.

use std::time::Duration;

use backtest::selection_run::{self as selection, Watch};
use chrono::{DateTime, Timelike, Utc};
use strategy_engine::schedule::Cron;
use tracing::{info, warn};

pub fn spawn(
    pg: sqlx::PgPool,
    ch_url: String,
    models: Option<backtest::runner::ModelsFactory>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            // Wake a few seconds into each minute: the bar that closed on the
            // minute has had time to land.
            let now = Utc::now();
            let into = Duration::from_secs(u64::from(now.second()))
                + Duration::from_nanos(u64::from(now.nanosecond()));
            let wait = Duration::from_secs(65).saturating_sub(into);
            tokio::time::sleep(wait).await;
            run_once(&pg, &ch_url, models.as_ref(), Utc::now()).await;
        }
    })
}

/// The minute `t` falls in.
#[must_use]
pub fn minute_of(t: DateTime<Utc>) -> DateTime<Utc> {
    t.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(t)
}

/// The watches whose schedule fires at `minute`. A schedule that no longer
/// parses is never due (it was valid when the watch was made; it is reported).
#[must_use]
pub fn due(watches: &[Watch], minute: DateTime<Utc>) -> Vec<&Watch> {
    let at_ns = minute.timestamp_nanos_opt().unwrap_or(i64::MAX);
    watches
        .iter()
        .filter(|w| match Cron::parse(&w.schedule) {
            Ok(c) => c.fires_at(at_ns),
            Err(e) => {
                warn!(watch = %w.watch_id, schedule = %w.schedule, error = %e, "watched screener's schedule does not parse");
                false
            }
        })
        .collect()
}

/// Run every watch due this minute, once.
pub async fn run_once(
    pg: &sqlx::PgPool,
    ch_url: &str,
    models: Option<&backtest::runner::ModelsFactory>,
    now: DateTime<Utc>,
) {
    let minute = minute_of(now);
    let watches = match selection::running_watches(pg).await {
        Ok(w) => w,
        Err(e) => return warn!(error = %e, "watched screeners could not be read"),
    };
    for w in due(&watches, minute) {
        match selection::scheduled_run_since(pg, &w.revision, minute).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                warn!(watch = %w.watch_id, error = %e, "scheduled-run check failed; not running");
                continue;
            }
        }
        let report =
            match selection::run_selection(pg, ch_url, &w.revision, minute, models).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(watch = %w.watch_id, error = %e, "watched screener did not run");
                    continue;
                }
            };
        match selection::journal(pg, &report, selection::RequestedBy::Schedule).await {
            Ok(run) => info!(
                watch = %w.watch_id,
                run = %run,
                selected = report.run.selection.selected.len(),
                "watched screener ran on its schedule"
            ),
            Err(e) => warn!(watch = %w.watch_id, error = %e, "watched screener ran but was not journaled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn watch(schedule: &str) -> Watch {
        Watch {
            watch_id: uuid::Uuid::new_v4(),
            user_id: "u".into(),
            strategy_ref: "s".into(),
            revision: "r".into(),
            schedule: schedule.into(),
            created_at: Utc::now(),
            stopped_at: None,
        }
    }

    #[test]
    fn a_watch_is_due_exactly_when_its_schedule_fires() {
        let ws = vec![watch("30 14 * * 1-5"), watch("0 * * * *"), watch("not a cron")];
        // Monday 2026-10-05 14:30 UTC.
        let at = Utc.with_ymd_and_hms(2026, 10, 5, 14, 30, 0).unwrap();
        let d = due(&ws, at);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].schedule, "30 14 * * 1-5");
        let on_the_hour = Utc.with_ymd_and_hms(2026, 10, 5, 15, 0, 0).unwrap();
        assert_eq!(due(&ws, on_the_hour)[0].schedule, "0 * * * *");
        // Saturday: the weekday schedule is not due.
        let saturday = Utc.with_ymd_and_hms(2026, 10, 10, 14, 30, 0).unwrap();
        assert!(due(&ws, saturday).is_empty());
    }

    #[test]
    fn a_tick_late_in_the_minute_is_still_that_minute() {
        let late = Utc.with_ymd_and_hms(2026, 10, 5, 14, 30, 59).unwrap();
        assert_eq!(minute_of(late), Utc.with_ymd_and_hms(2026, 10, 5, 14, 30, 0).unwrap());
    }

    /// Registered at boot — a source scan, for the reason `fill_stream_tests`
    /// gives: a job nobody spawns is indistinguishable from one that runs.
    #[test]
    fn watched_screeners_are_spawned_at_boot() {
        let main = include_str!("main.rs");
        assert!(main.contains("\nmod screener_jobs;"));
        let at = main
            .find("screener_jobs::spawn(")
            .expect("main must spawn the watched-screener job");
        let line_start = main[..at].rfind('\n').map_or(0, |i| i + 1);
        assert!(!main[line_start..at].contains("//"), "not commented out");
    }
}
