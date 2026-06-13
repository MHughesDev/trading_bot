//! Per-lane data freshness watchdog.
//!
//! Monitors how long ago the last event arrived on a lane.  If the gap
//! exceeds the staleness threshold **during expected trading hours**, the
//! watchdog trips the kill switch.
//!
//! A normal exchange close (outside trading hours) does **not** alarm.
//! Only a true feed outage during active hours does.

use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
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

/// Returns `true` if `now` (UTC) falls within any of the schedule's sessions,
/// correctly accounting for the instrument's IANA timezone (e.g. America/New_York).
fn is_within_trading_hours(now: DateTime<Utc>, schedule: &TradingSchedule) -> bool {
    if schedule.is_24_7() {
        return true;
    }

    // Parse the IANA timezone. Fall back to UTC with a warning on unrecognized strings.
    let tz: Tz = match schedule.timezone.parse() {
        Ok(tz) => tz,
        Err(_) => {
            warn!(
                timezone = %schedule.timezone,
                "unrecognized IANA timezone in TradingSchedule; falling back to UTC"
            );
            chrono_tz::UTC
        }
    };

    // Convert `now` to the venue's local time for accurate session comparison.
    let local_time = now.with_timezone(&tz).time();

    schedule.sessions.iter().any(|session| {
        if let (Ok(open), Ok(close)) = (
            NaiveTime::parse_from_str(&session.open, "%H:%M"),
            NaiveTime::parse_from_str(&session.close, "%H:%M"),
        ) {
            // Handle midnight-crossing sessions (e.g. 22:00–06:00 for overnight FX).
            if close < open {
                local_time >= open || local_time < close
            } else {
                local_time >= open && local_time < close
            }
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

    /// NYSE session in the real America/New_York timezone.
    /// 09:30–16:00 ET = 13:30–20:00 UTC (EDT, summer) or 14:30–21:00 UTC (EST, winter).
    fn ny_equity_schedule() -> TradingSchedule {
        TradingSchedule {
            timezone: "America/New_York".to_owned(),
            sessions: vec![TradingSession {
                open: "09:30".to_owned(),
                close: "16:00".to_owned(),
            }],
            has_pre_market: false,
            has_post_market: false,
        }
    }

    /// Schedule with an unrecognized timezone — should fall back to UTC without panicking.
    fn bad_tz_schedule() -> TradingSchedule {
        TradingSchedule {
            timezone: "Not/ATimezone".to_owned(),
            sessions: vec![TradingSession {
                open: "09:00".to_owned(),
                close: "17:00".to_owned(),
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

    /// 14:00 UTC on a June weekday = 10:00 EDT — inside the NYSE session.
    /// This was the exact case that was previously broken: the old UTC-only
    /// check compared 14:00 UTC against "09:30–16:00" and returned
    /// `StaleOutsideHours`, silently failing to trip the kill switch.
    #[test]
    fn stale_equity_at_14_utc_trips_switch_ny_tz() {
        let ks = ks();
        // 14:00 UTC on 2026-06-08 (summer, EDT = UTC-4) → 10:00 EDT (in session)
        let now = Utc.with_ymd_and_hms(2026, 6, 8, 14, 0, 0).unwrap();
        let last = now - Duration::seconds(120);
        let result = check_freshness(
            "AAPL",
            "market.trades",
            last,
            now,
            &ny_equity_schedule(),
            60,
            &ks,
        );
        assert!(
            matches!(result, FreshnessOutcome::StaleAlarm { .. }),
            "14:00 UTC (10:00 EDT) is inside the NYSE session and must trip the switch"
        );
        assert!(ks.is_active());
    }

    /// 23:00 UTC on a June weekday = 19:00 EDT — after the NYSE close.
    #[test]
    fn stale_equity_at_23_utc_does_not_alarm_ny_tz() {
        let ks = ks();
        // 23:00 UTC → 19:00 EDT — outside session
        let now = Utc.with_ymd_and_hms(2026, 6, 8, 23, 0, 0).unwrap();
        let last = now - Duration::seconds(3600);
        let result = check_freshness(
            "AAPL",
            "market.trades",
            last,
            now,
            &ny_equity_schedule(),
            60,
            &ks,
        );
        assert_eq!(
            result,
            FreshnessOutcome::StaleOutsideHours,
            "23:00 UTC (19:00 EDT) is after the NYSE close and must not alarm"
        );
        assert!(!ks.is_active());
    }

    /// Unrecognized timezone must not panic; falls back to UTC.
    #[test]
    fn bad_timezone_falls_back_to_utc_without_panic() {
        let ks = ks();
        // 12:00 UTC — within the 09:00–17:00 UTC fallback session
        let now = Utc.with_ymd_and_hms(2026, 6, 8, 12, 0, 0).unwrap();
        let last = now - Duration::seconds(120);
        // Should not panic; may trip or not depending on UTC time
        let _ = check_freshness("AAPL", "market.trades", last, now, &bad_tz_schedule(), 60, &ks);
    }

    /// Midnight-crossing FX session (e.g. 22:00–06:00): a time inside the
    /// wrap-around window must count as in-session.
    #[test]
    fn midnight_crossing_session_detected() {
        let schedule = TradingSchedule {
            timezone: "UTC".to_owned(),
            sessions: vec![TradingSession {
                open: "22:00".to_owned(),
                close: "06:00".to_owned(),
            }],
            has_pre_market: false,
            has_post_market: false,
        };
        let ks = ks();
        // 02:00 UTC — inside the 22:00–06:00 window
        let now = Utc.with_ymd_and_hms(2026, 6, 9, 2, 0, 0).unwrap();
        let last = now - Duration::seconds(120);
        let result = check_freshness("EUR-USD", "market.trades", last, now, &schedule, 60, &ks);
        assert!(
            matches!(result, FreshnessOutcome::StaleAlarm { .. }),
            "02:00 UTC is inside the midnight-crossing 22:00–06:00 FX session"
        );
    }
}
