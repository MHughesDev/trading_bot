//! Trading-session calendars for paper execution.
//!
//! Live venues reject orders when the market is closed; the paper half must
//! do the same or paper results overstate what live would have allowed.
//! Calendars are pure functions of `(asset_class, utc_now)` — no I/O, no
//! timezone database.  US Eastern daylight-saving rules are computed
//! directly (DST: second Sunday of March → first Sunday of November).
//!
//! | Calendar        | Classes                  | Hours (US Eastern)          |
//! |-----------------|--------------------------|-----------------------------|
//! | `Always`        | crypto CEX/DEX, perps, NFT, prediction | 24/7          |
//! | `UsEquityRth`   | equity, ETF, option      | Mon–Fri 09:30–16:00         |
//! | `UsBondCash`    | bond                     | Mon–Fri 08:00–17:00         |
//! | `UsFuturesGlobex` | expiring futures       | Sun 18:00 → Fri 17:00, daily 17:00–18:00 break |
//! | `FxWeek`        | FX                       | Sun 17:00 → Fri 17:00       |

use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc, Weekday};
use domain::instrument::AssetClass;

/// Which session calendar an asset class trades on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionCalendar {
    Always,
    UsEquityRth,
    UsBondCash,
    UsFuturesGlobex,
    FxWeek,
}

/// Calendar for an asset class.
pub fn calendar_for(asset_class: AssetClass) -> SessionCalendar {
    match asset_class {
        AssetClass::CryptoSpotCex
        | AssetClass::CryptoSpotDex
        | AssetClass::PerpetualSwap
        | AssetClass::Nft
        | AssetClass::PredictionMarket => SessionCalendar::Always,
        AssetClass::Equity | AssetClass::Etf | AssetClass::Option => SessionCalendar::UsEquityRth,
        AssetClass::Bond => SessionCalendar::UsBondCash,
        AssetClass::FuturesExpiring => SessionCalendar::UsFuturesGlobex,
        AssetClass::Fx => SessionCalendar::FxWeek,
    }
}

/// Is the market for `asset_class` open at `now`?
pub fn is_open(asset_class: AssetClass, now: DateTime<Utc>) -> bool {
    let et = now + Duration::hours(eastern_offset_hours(now));
    let weekday = et.weekday();
    let minutes = (et.hour() * 60 + et.minute()) as i32;

    match calendar_for(asset_class) {
        SessionCalendar::Always => true,
        SessionCalendar::UsEquityRth => {
            is_weekday(weekday) && (9 * 60 + 30..16 * 60).contains(&minutes)
        }
        SessionCalendar::UsBondCash => {
            is_weekday(weekday) && (8 * 60..17 * 60).contains(&minutes)
        }
        SessionCalendar::UsFuturesGlobex => match weekday {
            Weekday::Sat => false,
            Weekday::Sun => minutes >= 18 * 60,
            Weekday::Fri => minutes < 17 * 60,
            // Mon–Thu: open except the 17:00–18:00 maintenance break.
            _ => !(17 * 60..18 * 60).contains(&minutes),
        },
        SessionCalendar::FxWeek => match weekday {
            Weekday::Sat => false,
            Weekday::Sun => minutes >= 17 * 60,
            Weekday::Fri => minutes < 17 * 60,
            _ => true,
        },
    }
}

fn is_weekday(w: Weekday) -> bool {
    !matches!(w, Weekday::Sat | Weekday::Sun)
}

/// UTC→Eastern offset in hours (−4 during DST, −5 otherwise).
///
/// DST runs from 2:00 local on the second Sunday of March to 2:00 local on
/// the first Sunday of November.  The one-hour transition windows are
/// approximated at the UTC day boundary of those Sundays — a deliberate,
/// documented simplification (transitions happen at 07:00/06:00 UTC).
fn eastern_offset_hours(now: DateTime<Utc>) -> i64 {
    let date = now.date_naive();
    let year = date.year();
    let dst_start = nth_weekday(year, 3, Weekday::Sun, 2);
    let dst_end = nth_weekday(year, 11, Weekday::Sun, 1);
    if date > dst_start && date < dst_end {
        return -4;
    }
    if date == dst_start {
        // Spring forward at 2:00 EST = 07:00 UTC.
        return if now.hour() >= 7 { -4 } else { -5 };
    }
    if date == dst_end {
        // Fall back at 2:00 EDT = 06:00 UTC.
        return if now.hour() >= 6 { -5 } else { -4 };
    }
    -5
}

/// The `n`-th `weekday` of `month` in `year` (n is 1-based).
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month start");
    let shift = (7 + weekday.num_days_from_monday() as i64
        - first.weekday().num_days_from_monday() as i64)
        % 7;
    first + Duration::days(shift + 7 * (n as i64 - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn crypto_and_perps_trade_around_the_clock() {
        // Saturday 03:00 UTC.
        let t = utc(2026, 6, 13, 3, 0);
        assert!(is_open(AssetClass::CryptoSpotCex, t));
        assert!(is_open(AssetClass::PerpetualSwap, t));
        assert!(is_open(AssetClass::PredictionMarket, t));
    }

    #[test]
    fn equities_open_only_during_rth() {
        // Friday 2026-06-12 14:00 ET (18:00 UTC, EDT) — open.
        assert!(is_open(AssetClass::Equity, utc(2026, 6, 12, 18, 0)));
        // Friday 08:00 ET — pre-market, closed.
        assert!(!is_open(AssetClass::Equity, utc(2026, 6, 12, 12, 0)));
        // Friday 16:00 ET sharp — closed (close is exclusive).
        assert!(!is_open(AssetClass::Equity, utc(2026, 6, 12, 20, 0)));
        // Saturday — closed.
        assert!(!is_open(AssetClass::Equity, utc(2026, 6, 13, 18, 0)));
        // Options follow the equity calendar.
        assert!(!is_open(AssetClass::Option, utc(2026, 6, 13, 18, 0)));
    }

    #[test]
    fn dst_offset_shifts_session_boundaries() {
        // 2026-01-15 is EST (UTC−5): 09:30 ET = 14:30 UTC.
        assert!(!is_open(AssetClass::Equity, utc(2026, 1, 15, 14, 0)));
        assert!(is_open(AssetClass::Equity, utc(2026, 1, 15, 14, 30)));
        // 2026-06-15 is EDT (UTC−4): 09:30 ET = 13:30 UTC.
        assert!(is_open(AssetClass::Equity, utc(2026, 6, 15, 13, 30)));
    }

    #[test]
    fn dst_calendar_dates_are_correct() {
        // 2026: DST starts Sun Mar 8, ends Sun Nov 1.
        assert_eq!(
            nth_weekday(2026, 3, Weekday::Sun, 2),
            NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()
        );
        assert_eq!(
            nth_weekday(2026, 11, Weekday::Sun, 1),
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
    }

    #[test]
    fn futures_break_and_weekend_are_closed() {
        // Tuesday 17:30 ET (21:30 UTC EDT) — maintenance break.
        assert!(!is_open(
            AssetClass::FuturesExpiring,
            utc(2026, 6, 16, 21, 30)
        ));
        // Tuesday 18:30 ET — reopened.
        assert!(is_open(
            AssetClass::FuturesExpiring,
            utc(2026, 6, 16, 22, 30)
        ));
        // Saturday — closed; Sunday 18:30 ET — open.
        assert!(!is_open(
            AssetClass::FuturesExpiring,
            utc(2026, 6, 13, 22, 30)
        ));
        assert!(is_open(
            AssetClass::FuturesExpiring,
            utc(2026, 6, 14, 22, 30)
        ));
    }

    #[test]
    fn fx_closes_friday_evening_reopens_sunday() {
        // Friday 17:30 ET — closed.
        assert!(!is_open(AssetClass::Fx, utc(2026, 6, 12, 21, 30)));
        // Sunday 17:30 ET — open.
        assert!(is_open(AssetClass::Fx, utc(2026, 6, 14, 21, 30)));
        // Wednesday overnight (03:00 ET) — open.
        assert!(is_open(AssetClass::Fx, utc(2026, 6, 17, 7, 0)));
    }
}
