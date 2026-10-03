//! Scheduled triggers: five-field cron, in UTC (ADR-0042; Set U U-6.3, U-6.4).
//!
//! `triggers.schedule` and a deployment's `rebalance.cron` are cron expressions.
//! Until now nothing in the platform could evaluate one — backtests refused
//! them by name. This is the one evaluator: pure, so a backtest asks it which
//! instants fire inside a window and a live scheduler asks it when the next one
//! is, and both get the same instants.
//!
//! Fields: minute (0–59), hour (0–23), day of month (1–31), month (1–12), day of
//! week (0–6, Sunday = 0; 7 is also Sunday). Each is `*`, a number, a range
//! `a-b`, a step `*/n` or `a-b/n`, or a comma list of those. As in classic cron,
//! when both day fields are restricted a day matches if *either* does.
//!
//! Instants are UTC. Venues with sessions need the exchange calendar on top of
//! this (FEAT-007 §7); crypto, the venue class this runs on today, has none.

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Timelike, Utc};

/// A parsed cron expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    weekdays: Vec<u32>,
    days_restricted: bool,
    weekdays_restricted: bool,
    source: String,
}

fn field(raw: &str, lo: u32, hi: u32, what: &str) -> Result<(Vec<u32>, bool), String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u32>()
                    .ok()
                    .filter(|s| *s > 0)
                    .ok_or_else(|| format!("{what}: step '{s}' is not a positive number"))?,
            ),
            None => (part, 1),
        };
        let (a, b) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            let p = |x: &str| {
                x.parse::<u32>()
                    .map_err(|_| format!("{what}: '{x}' is not a number"))
            };
            (p(a)?, p(b)?)
        } else {
            let x = range
                .parse::<u32>()
                .map_err(|_| format!("{what}: '{range}' is not a number"))?;
            // `5/15` means from 5 to the end, every 15.
            if part.contains('/') {
                (x, hi)
            } else {
                (x, x)
            }
        };
        if a < lo || b > hi || a > b {
            return Err(format!("{what}: {a}-{b} is outside {lo}-{hi}"));
        }
        out.extend((a..=b).step_by(step as usize));
    }
    out.sort_unstable();
    out.dedup();
    Ok((out, raw != "*"))
}

impl Cron {
    /// Parse a five-field expression.
    ///
    /// # Errors
    /// The wrong number of fields, or a field out of range — named.
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        let [m, h, dom, mon, dow] = parts.as_slice() else {
            return Err(format!(
                "'{expr}' has {} fields; a schedule is five: minute hour day-of-month month day-of-week",
                parts.len()
            ));
        };
        let (minutes, _) = field(m, 0, 59, "minute")?;
        let (hours, _) = field(h, 0, 23, "hour")?;
        let (days, days_restricted) = field(dom, 1, 31, "day of month")?;
        let (months, _) = field(mon, 1, 12, "month")?;
        let (mut weekdays, weekdays_restricted) = field(dow, 0, 7, "day of week")?;
        if weekdays.contains(&7) {
            weekdays.retain(|d| *d != 7);
            if !weekdays.contains(&0) {
                weekdays.insert(0, 0);
            }
        }
        Ok(Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            days_restricted,
            weekdays_restricted,
            source: expr.to_string(),
        })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        if !self.months.contains(&d.month()) {
            return false;
        }
        let dom = self.days.contains(&d.day());
        let dow = self.weekdays.contains(&d.weekday().num_days_from_sunday());
        match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }

    /// The first instant strictly after `after_ns` that the schedule fires, in
    /// Unix ns. `None` if it never fires within eight years (`0 0 30 2 *`).
    #[must_use]
    pub fn next_after(&self, after_ns: i64) -> Option<i64> {
        let after = DateTime::from_timestamp_nanos(after_ns);
        let mut day = after.date_naive();
        for _ in 0..(8 * 366) {
            if self.day_matches(day) {
                for &h in &self.hours {
                    for &m in &self.minutes {
                        let t = Utc.from_utc_datetime(&day.and_hms_opt(h, m, 0)?);
                        if t > after {
                            return t.timestamp_nanos_opt();
                        }
                    }
                }
            }
            day += Duration::days(1);
        }
        None
    }

    /// Every instant the schedule fires in `(from_ns, to_ns]`, in order.
    #[must_use]
    pub fn fires_between(&self, from_ns: i64, to_ns: i64) -> Vec<i64> {
        let mut out = Vec::new();
        let mut at = from_ns;
        while let Some(next) = self.next_after(at) {
            if next > to_ns {
                break;
            }
            out.push(next);
            at = next;
        }
        out
    }

    /// Whether the schedule fires at exactly `at_ns`.
    #[must_use]
    pub fn fires_at(&self, at_ns: i64) -> bool {
        let t = DateTime::from_timestamp_nanos(at_ns);
        t.second() == 0
            && t.nanosecond() == 0
            && self.minutes.contains(&t.minute())
            && self.hours.contains(&t.hour())
            && self.day_matches(t.date_naive())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap()
    }

    #[test]
    fn a_daily_schedule_fires_once_a_day_at_its_minute() {
        let c = Cron::parse("55 23 * * *").unwrap();
        assert_eq!(
            c.next_after(ns(2025, 1, 1, 0, 0)),
            Some(ns(2025, 1, 1, 23, 55))
        );
        assert_eq!(
            c.next_after(ns(2025, 1, 1, 23, 55)),
            Some(ns(2025, 1, 2, 23, 55)),
            "strictly after"
        );
        assert_eq!(
            c.fires_between(ns(2025, 1, 1, 0, 0), ns(2025, 1, 4, 0, 0))
                .len(),
            3
        );
        assert!(c.fires_at(ns(2025, 3, 9, 23, 55)));
        assert!(!c.fires_at(ns(2025, 3, 9, 23, 54)));
    }

    #[test]
    fn steps_ranges_lists_and_weekdays() {
        let c = Cron::parse("*/15 9-10 * * 1-5").unwrap();
        // 2025-01-04 is a Saturday: the next firing is Monday 09:00.
        assert_eq!(
            c.next_after(ns(2025, 1, 4, 12, 0)),
            Some(ns(2025, 1, 6, 9, 0))
        );
        assert_eq!(
            c.fires_between(ns(2025, 1, 6, 0, 0), ns(2025, 1, 6, 23, 0))
                .len(),
            8
        );
        let sundays = Cron::parse("0 0 * * 7").unwrap();
        assert_eq!(
            sundays.next_after(ns(2025, 1, 1, 0, 0)),
            Some(ns(2025, 1, 5, 0, 0))
        );
        let either = Cron::parse("0 0 1 * 1").unwrap();
        // The 1st of the month OR a Monday.
        assert_eq!(
            either.next_after(ns(2025, 1, 1, 1, 0)),
            Some(ns(2025, 1, 6, 0, 0))
        );
        let list = Cron::parse("0 0,12 * * *").unwrap();
        assert_eq!(
            list.fires_between(ns(2025, 1, 1, 0, 0), ns(2025, 1, 2, 0, 0))
                .len(),
            2
        );
    }

    #[test]
    fn a_malformed_schedule_is_named() {
        assert!(Cron::parse("* * * *").unwrap_err().contains("five"));
        assert!(Cron::parse("60 * * * *").unwrap_err().contains("minute"));
        assert!(Cron::parse("* * * * */0").unwrap_err().contains("step"));
        assert_eq!(Cron::parse("0 0 30 2 *").unwrap().next_after(0), None);
    }
}
