//! Adversarial test: normal market close does not alarm; true outage during
//! trading hours does.

use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use domain::instrument::{TradingSchedule, TradingSession};
use reconciliation::freshness::{check_freshness, FreshnessOutcome};
use risk::KillSwitch;

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

fn crypto_schedule() -> TradingSchedule {
    TradingSchedule::always_open()
}

fn ks() -> Arc<KillSwitch> {
    Arc::new(KillSwitch::new(false))
}

#[test]
fn normal_equity_close_does_not_alarm() {
    let ks = ks();
    // 18:00 UTC — outside 09:30–16:00 UTC session.
    let now = Utc.with_ymd_and_hms(2026, 6, 8, 18, 0, 0).unwrap();
    let last_event = now - Duration::seconds(7200); // 2 hours silent

    let result = check_freshness(
        "AAPL",
        "market.trades",
        last_event,
        now,
        &equity_schedule(),
        60,
        &ks,
    );

    assert_eq!(
        result,
        FreshnessOutcome::StaleOutsideHours,
        "normal market close must not alarm"
    );
    assert!(
        !ks.is_active(),
        "kill switch must not be tripped for a normal close"
    );
}

#[test]
fn outage_during_equity_session_alarms() {
    let ks = ks();
    // 13:00 UTC — within session.
    let now = Utc.with_ymd_and_hms(2026, 6, 8, 13, 0, 0).unwrap();
    let last_event = now - Duration::seconds(120); // 2 min silent during session

    let result = check_freshness(
        "AAPL",
        "market.trades",
        last_event,
        now,
        &equity_schedule(),
        60,
        &ks,
    );

    assert!(
        matches!(result, FreshnessOutcome::StaleAlarm { .. }),
        "true outage during session must alarm"
    );
    assert!(
        ks.is_active(),
        "kill switch must be tripped during real outage"
    );
}

#[test]
fn crypto_24_7_stale_feed_always_alarms() {
    let ks = ks();
    let now = Utc::now();
    let last_event = now - Duration::seconds(120);

    let result = check_freshness(
        "BTC-USD",
        "market.trades",
        last_event,
        now,
        &crypto_schedule(),
        60,
        &ks,
    );

    assert!(
        matches!(result, FreshnessOutcome::StaleAlarm { .. }),
        "crypto feed is 24/7 — any staleness is an alarm"
    );
    assert!(ks.is_active());
}

#[test]
fn fresh_feed_is_always_ok() {
    let ks = ks();
    let now = Utc::now();
    let last_event = now - Duration::seconds(5); // well within threshold

    let result = check_freshness(
        "BTC-USD",
        "market.trades",
        last_event,
        now,
        &crypto_schedule(),
        60,
        &ks,
    );

    assert_eq!(result, FreshnessOutcome::Fresh);
    assert!(!ks.is_active());
}
