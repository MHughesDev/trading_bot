//! Per-lane data freshness watchdog.
//!
//! Monitors how long ago the last event arrived on a lane.  If the gap
//! exceeds the staleness threshold **during expected trading hours**, the
//! watchdog trips the kill switch.
//!
//! A normal exchange close (outside trading hours) does **not** alarm.
//! Only a true feed outage during active hours does.

use chrono::{DateTime, NaiveTime, Utc};
use std::sync::Arc;
use tracing::{info, warn};

use domain::instrument::TradingSchedule;
use risk::KillSwitch;

/// Threshold after which silence is considered a staleness alarm.
pub const DEFAULT_STALENESS_SECONDS: i64 = 60;

/// Outcome of a freshness check.
#[derive(Debug, PartialEq, Eq)]
pub enum FreshnessOutcome {
    /// Data is fresh (last event within threshold).
    Fresh,
    /// Feed is stale but outside trading hours — no alarm.
    StaleOutsideHours,
    /// Feed is stale during active trading hours — kill switch tripped.
    StaleAlarm { seconds_silent: i64 },
}

/// Check whether the lane is fresh given:
/// - `last_event_at`: when the last event arrived
/// - `now`: current time
/// - `schedule`: the instrument's trading schedule
/// - `threshold_secs`: how many seconds of silence trigger an alarm
pub fn check_freshness(
    instrument_id: &str,
    lane: &str,
    last_event_at: DateTime<Utc>,
    now: DateTime<Utc>,
    schedule: &TradingSchedule,
    threshold_secs: i64,
    kill_switch: &Arc<KillSwitch>,
) -> FreshnessOutcome {
    let silence = (now - last_event_at).num_seconds();

    if silence <= threshold_secs {
        return FreshnessOutcome::Fresh;
    }

    // Silence exceeds threshold — but is it within trading hours?
    if is_within_trading_hours(now, schedule) {
        warn!(
            %instrument_id,
            %lane,
            %silence,
            "data staleness alarm — tripping kill switch"
        );
        kill_switch.trip();
        FreshnessOutcome::StaleAlarm {
            seconds_silent: silence,
        }
    } else {
        info!(
            %instrument_id,
            %lane,
            %silence,
            "data feed silent but outside trading hours — no alarm"
        );
        FreshnessOutcome::StaleOutsideHours
    }
}

/// Returns `true` if `now` (UTC) falls within any of the schedule's sessions.
/// A 24/7 instrument is always within hours.
fn is_within_trading_hours(now: DateTime<Utc>, schedule: &TradingSchedule) -> bool {
    if schedule.is_24_7() {
        return true;
    }

    // Convert `now` to the schedule's timezone for time comparison.
    // We use a simplified UTC-only comparison here since full tz support
    // would require the `chrono-tz` crate (out of scope for Phase 2).
    // For equities (America/New_York), the caller should pass UTC-adjusted times
    // or pre-convert.  This implementation does a UTC-based session check.
    let time = now.time();
    schedule.sessions.iter().any(|session| {
        if let (Ok(open), Ok(close)) = (
            NaiveTime::parse_from_str(&session.open, "%H:%M"),
            NaiveTime::parse_from_str(&session.close, "%H:%M"),
        ) {
            time >= open && time < close
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use domain::instrument::{TradingSchedule, TradingSession};

    fn crypto_schedule() -> TradingSchedule {
        TradingSchedule::always_open()
    }

    fn equity_schedule() -> TradingSchedule {
        TradingSchedule {
            timezone: "UTC".to_owned(),
            sessions: vec![TradingSession {
                open: "09:30".to_owned(),
                close: "16:00".to_owned(),
            }],
            has_pre_market: false,
            has_post_market: false,
        }
    }

    fn ks() -> Arc<KillSwitch> {
        Arc::new(KillSwitch::new(false))
    }

    #[test]
    fn fresh_data_does_not_alarm() {
        let ks = ks();
        let now = Utc::now();
        let last = now - Duration::seconds(5);
        let result = check_freshness(
            "BTC-USD",
            "market.trades",
            last,
            now,
            &crypto_schedule(),
            60,
            &ks,
        );
        assert_eq!(result, FreshnessOutcome::Fresh);
        assert!(!ks.is_active());
    }

    #[test]
    fn stale_crypto_during_24_7_trips_switch() {
        let ks = ks();
        let now = Utc::now();
        let last = now - Duration::seconds(120);
        let result = check_freshness(
            "BTC-USD",
            "market.trades",
            last,
            now,
            &crypto_schedule(),
            60,
            &ks,
        );
        assert!(
            matches!(result, FreshnessOutcome::StaleAlarm { seconds_silent } if seconds_silent >= 120)
        );
        assert!(ks.is_active());
    }

    #[test]
    fn stale_equity_outside_session_does_not_alarm() {
        let ks = ks();
        // 18:00 UTC — outside 09:30–16:00 UTC session.
        let now = Utc.with_ymd_and_hms(2026, 6, 8, 18, 0, 0).unwrap();
        let last = now - Duration::seconds(3600); // 1 hour silent
        let result = check_freshness(
            "AAPL",
            "market.trades",
            last,
            now,
            &equity_schedule(),
            60,
            &ks,
        );
        assert_eq!(result, FreshnessOutcome::StaleOutsideHours);
        assert!(
            !ks.is_active(),
            "normal market close must not trip kill switch"
        );
    }

    #[test]
    fn stale_equity_during_session_trips_switch() {
        let ks = ks();
        // 12:00 UTC — within 09:30–16:00 session.
        let now = Utc.with_ymd_and_hms(2026, 6, 8, 12, 0, 0).unwrap();
        let last = now - Duration::seconds(120);
        let result = check_freshness(
            "AAPL",
            "market.trades",
            last,
            now,
            &equity_schedule(),
            60,
            &ks,
        );
        assert!(matches!(result, FreshnessOutcome::StaleAlarm { .. }));
        assert!(ks.is_active());
    }
}
