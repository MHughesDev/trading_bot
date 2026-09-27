//! The calendar behind a session's schedule windows (AGENT-014 §2.1, §7 rows
//! 1–2; Set S task S-11.6).
//!
//! [`crate::charter::Schedule`] is what the user writes: a list of
//! `{days, start, end, tz}` and a concurrency. This is the part that *reads* it
//! — whether the session may claim now, when it may claim next, and when the
//! child running now has to drain. Nothing here talks to a job service or a
//! clock of its own: every entry point takes the instant it is asked about, so
//! a Tuesday in November is a fixture rather than a wait.
//!
//! # An unknown timezone is refused, never quietly made UTC
//!
//! [`ScheduleError::UnknownTimezone`] is the whole reason [`OpenHours::parse`]
//! is fallible. A session that ran outside its window because `Amercia/New_York`
//! became UTC is a session that spent money the operator thought was fenced, and
//! the fence would have looked fine on the charter form. The tz string is
//! resolved once, at the boundary, and a [`ParsedWindow`] cannot exist without a
//! zone the platform actually knows.
//!
//! # No windows means always open
//!
//! [`crate::charter::Schedule::default`] has an empty window list and that means
//! *always open*, exactly as an empty [`crate::charter::Scope`] means
//! everything. Reading it the other way round would silently stop every session
//! whose user never opened the schedule grid.
//!
//! # What a window means across a DST transition
//!
//! A window is a statement about the **local clock**: the session may run while
//! the wall clock in `tz` reads inside `[start, end)` on one of `days`. Two
//! consequences, both tested rather than described:
//!
//! * on the night an hour repeats, a window covering that hour is open for two
//!   hours of real time — the clock reads 01:30 twice and both are inside it;
//! * on the morning an hour never happens, a window inside the gap never opens
//!   that day, and one that straddles it opens the instant the clock passes its
//!   start. [`OpenHours::next_open_after`] never returns an instant
//!   [`OpenHours::is_open_at`] would call closed, which is what stops a driver
//!   re-arming onto a window that is not there.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::charter::{Schedule, Window};

/// Minutes in a day. Also the value `24:00` parses to — see [`parse_clock`].
pub const MINUTES_PER_DAY: u32 = 24 * 60;

/// How far past a local time that does not exist the resolver looks for the
/// instant the clock jumps to. Three hours covers every transition in the tz
/// database, which are at most two.
const GAP_SCAN_MINUTES: i64 = 180;

/// How many days ahead [`OpenHours::next_open_after`] looks.
///
/// Fifteen rather than eight so that **every** weekday falls in the horizon
/// twice. A window's only day of the week can be swallowed whole by a DST gap
/// (`02:00–02:30` on the morning the clock jumps from 02:00 to 03:00); the
/// following week is the honest answer, and it is only reachable if the search
/// looks past the first occurrence.
pub const SEARCH_HORIZON_DAYS: i64 = 15;

/// How many overlapping windows [`OpenHours::closes_after`] will chain through
/// before it gives up. Windows that hand over to each other all day are
/// ordinary; a chain longer than this is not a schedule anybody wrote.
const MAX_OVERLAP_CHAIN: usize = 64;

/// Why a schedule could not be read.
///
/// Every variant is a refusal rather than a repair. There is no default zone,
/// no assumed day list and no wrapped window, because each of those would turn
/// a typo into a session that runs at a time nobody agreed to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error(
        "`{tz}` is not a timezone this platform knows; a window is refused rather than run in UTC, \
         because a fence that silently moved is not a fence"
    )]
    UnknownTimezone { tz: String },
    #[error("`{got}` is not an `HH:MM` time for a window's {field}")]
    BadClock { field: &'static str, got: String },
    #[error("{got} is not an ISO weekday: Monday is 1 and Sunday is 7")]
    BadDay { got: u8 },
    #[error(
        "a window with no days never opens; a schedule that means \"never run\" is an empty window \
         list away from meaning \"always run\", so neither is guessed at"
    )]
    NoDays,
    #[error(
        "a window from {start} to {end} never opens: write the two halves as two windows if it was \
         meant to cross midnight, so the days it names stay the days it runs on"
    )]
    NeverOpens { start: String, end: String },
}

/// Which side of a repeated local hour an instant is taken from.
///
/// On the night the clock goes back, a local time happens twice. A window
/// *opens* at the first of them and *closes* at the last, which is what makes
/// the window cover the whole of the repeated hour rather than half of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Earliest,
    Latest,
}

/// `HH:MM` as minutes from local midnight.
///
/// `24:00` is `1440` and is the **only** way to say "to the end of the day":
/// `00:00` would parse to zero, which reads as ending before it starts, and
/// [`ScheduleError::NeverOpens`] is what a charter written that way gets.
///
/// # Errors
///
/// Refuses anything that is not two `:`-separated numbers inside `00:00..=24:00`.
pub fn parse_clock(field: &'static str, s: &str) -> Result<u32, ScheduleError> {
    let bad = || ScheduleError::BadClock {
        field,
        got: s.to_string(),
    };
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    let m: u32 = m.trim().parse().map_err(|_| bad())?;
    if h > 24 || m > 59 {
        return Err(bad());
    }
    let total = h * 60 + m;
    if total > MINUTES_PER_DAY {
        return Err(bad());
    }
    Ok(total)
}

/// The zone that string names, or a refusal that carries the string.
///
/// The one place a tz is resolved, so there is one place to look for the
/// fallback that is not there.
fn parse_tz(tz: &str) -> Result<Tz, ScheduleError> {
    tz.parse()
        .map_err(|_| ScheduleError::UnknownTimezone { tz: tz.to_string() })
}

/// One window of [`crate::charter::Schedule`], resolved against the tz database.
///
/// The fields are private because every one of them is an invariant somebody
/// could otherwise break after the fact: the days are ISO and non-empty, the
/// zone is one chrono-tz knows, and `start < end <= 1440`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedWindow {
    days: Vec<u8>,
    start: u32,
    end: u32,
    tz: Tz,
}

impl ParsedWindow {
    /// Read one window off a charter.
    ///
    /// # Errors
    ///
    /// [`ScheduleError::UnknownTimezone`] for a zone the platform does not know,
    /// [`ScheduleError::BadClock`] for a time that is not `HH:MM`,
    /// [`ScheduleError::BadDay`] for a weekday outside `1..=7`,
    /// [`ScheduleError::NoDays`] for an empty day list, and
    /// [`ScheduleError::NeverOpens`] when `end` is not after `start`.
    pub fn parse(w: &Window) -> Result<Self, ScheduleError> {
        let tz = parse_tz(&w.tz)?;
        let start = parse_clock("start", &w.start)?;
        let end = parse_clock("end", &w.end)?;
        if end <= start {
            return Err(ScheduleError::NeverOpens {
                start: w.start.clone(),
                end: w.end.clone(),
            });
        }
        let mut days = w.days.clone();
        days.sort_unstable();
        days.dedup();
        if days.is_empty() {
            return Err(ScheduleError::NoDays);
        }
        if let Some(bad) = days.iter().find(|d| !(1..=7).contains(*d)) {
            return Err(ScheduleError::BadDay { got: *bad });
        }
        Ok(Self {
            days,
            start,
            end,
            tz,
        })
    }

    /// ISO weekdays, Monday = 1, sorted and deduplicated.
    #[must_use]
    pub fn days(&self) -> &[u8] {
        &self.days
    }

    /// Minutes from local midnight, inclusive.
    #[must_use]
    pub fn start_minutes(&self) -> u32 {
        self.start
    }

    /// Minutes from local midnight, exclusive. [`MINUTES_PER_DAY`] for a window
    /// written `24:00`.
    #[must_use]
    pub fn end_minutes(&self) -> u32 {
        self.end
    }

    #[must_use]
    pub fn tz(&self) -> Tz {
        self.tz
    }

    /// Whether the local clock in this window's zone reads inside it.
    ///
    /// This is the whole definition of "open", and every other function here is
    /// held to agree with it.
    #[must_use]
    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        let local = at.with_timezone(&self.tz);
        let day = u8::try_from(local.weekday().number_from_monday()).unwrap_or(0);
        if !self.days.contains(&day) {
            return false;
        }
        let minutes = local.hour() * 60 + local.minute();
        minutes >= self.start && minutes < self.end
    }

    /// When this window opens on a given local date, or `None` when it does not
    /// open that day at all — either because the date is not one of its days, or
    /// because a DST gap swallowed the whole of it.
    #[must_use]
    pub fn opens_on(&self, date: NaiveDate) -> Option<DateTime<Utc>> {
        if !self.days.contains(&iso_weekday(date)) {
            return None;
        }
        let t = self.resolve(date, self.start, Edge::Earliest)?;
        // The resolver answers "when does the clock first read at or past
        // `start`", which after a gap can be past `end` as well. Asking
        // `contains` is what keeps `next_open_after` from handing the driver an
        // instant `is_open_at` calls closed.
        self.contains(t).then_some(t)
    }

    /// The first instant at or after `from` at which this window is closed,
    /// given that it is open at `from`.
    fn closes_after(&self, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let date = from.with_timezone(&self.tz).date_naive();
        [Edge::Earliest, Edge::Latest]
            .into_iter()
            .filter_map(|edge| self.resolve(date, self.end, edge))
            .filter(|t| *t > from && !self.contains(*t))
            .min()
    }

    /// The earliest instant at which the local clock on `date` reads at or past
    /// `minutes`.
    ///
    /// Three cases, and the third is the one worth reading. When that local time
    /// exists once, that is the instant. When it happens twice, `edge` picks
    /// which. When it never happens — the clock jumped over it — the scan walks
    /// forward a minute at a time to the far side of the gap, so the answer is
    /// the instant the clock *passed* the time asked about. That is the only
    /// reading under which a window straddling a spring-forward opens at all.
    fn resolve(&self, date: NaiveDate, minutes: u32, edge: Edge) -> Option<DateTime<Utc>> {
        let (date, minutes) = if minutes >= MINUTES_PER_DAY {
            (date.succ_opt()?, minutes - MINUTES_PER_DAY)
        } else {
            (date, minutes)
        };
        let naive = date.and_hms_opt(minutes / 60, minutes % 60, 0)?;
        for step in 0..=GAP_SCAN_MINUTES {
            let candidate = naive.checked_add_signed(Duration::minutes(step))?;
            match self.tz.from_local_datetime(&candidate) {
                LocalResult::Single(t) => return Some(t.with_timezone(&Utc)),
                LocalResult::Ambiguous(first, last) => {
                    return Some(
                        match edge {
                            Edge::Earliest => first,
                            Edge::Latest => last,
                        }
                        .with_timezone(&Utc),
                    )
                }
                LocalResult::None => {}
            }
        }
        None
    }
}

/// When a session may claim, read rather than written (AGENT-014 §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHours {
    windows: Vec<ParsedWindow>,
    max_concurrent_runs: i16,
}

impl OpenHours {
    /// Read a charter's schedule.
    ///
    /// # Errors
    ///
    /// Whatever [`ParsedWindow::parse`] refuses, with the offending window's own
    /// strings in it. One bad window refuses the schedule: a session that ran on
    /// the windows that happened to parse would be running on a schedule nobody
    /// wrote.
    pub fn parse(schedule: &Schedule) -> Result<Self, ScheduleError> {
        let windows = schedule
            .windows
            .iter()
            .map(ParsedWindow::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            windows,
            max_concurrent_runs: schedule.max_concurrent_runs,
        })
    }

    /// An always-open schedule, which is what no windows means.
    #[must_use]
    pub fn always_open() -> Self {
        Self {
            windows: Vec::new(),
            max_concurrent_runs: 1,
        }
    }

    #[must_use]
    pub fn windows(&self) -> &[ParsedWindow] {
        &self.windows
    }

    #[must_use]
    pub fn max_concurrent_runs(&self) -> i16 {
        self.max_concurrent_runs
    }

    /// Whether this schedule names no windows, and is therefore always open.
    #[must_use]
    pub fn is_always_open(&self) -> bool {
        self.windows.is_empty()
    }

    /// Whether the session may claim at `at`.
    ///
    /// Total, and the definition every other answer here is checked against.
    #[must_use]
    pub fn is_open_at(&self, at: DateTime<Utc>) -> bool {
        self.is_always_open() || self.windows.iter().any(|w| w.contains(at))
    }

    /// The next instant a window opens, strictly after `at`.
    ///
    /// This is what the driver re-arms on when it finds itself outside a window
    /// (§7 row 1). `None` for an always-open schedule — there is no boundary to
    /// wait for — and `None` when nothing opens inside [`SEARCH_HORIZON_DAYS`],
    /// which a weekly window cannot manage and which is therefore a reason to
    /// tell a human rather than to guess an instant.
    ///
    /// Every instant it returns is one [`Self::is_open_at`] calls open.
    #[must_use]
    pub fn next_open_after(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.next_open(at, 0)
    }

    /// The next instant a window opens on a **later local day** than `at`.
    ///
    /// §7 row 2: a session that has spent its day budget resumes at "next day's
    /// window", not at the next window — a second window later the same day is
    /// the same day's budget. The local day is each window's own, so a schedule
    /// that mixes zones answers per window and takes the earliest.
    ///
    /// `None` for an always-open schedule: there is no window to wait for, and
    /// when the budget day itself rolls over is the budget's business rather
    /// than the calendar's.
    #[must_use]
    pub fn next_open_on_a_later_day(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.next_open(at, 1)
    }

    fn next_open(&self, at: DateTime<Utc>, first_offset: i64) -> Option<DateTime<Utc>> {
        let mut best: Option<DateTime<Utc>> = None;
        for w in &self.windows {
            let today = at.with_timezone(&w.tz).date_naive();
            for k in first_offset..=SEARCH_HORIZON_DAYS {
                let Some(date) = today.checked_add_signed(Duration::days(k)) else {
                    break;
                };
                if let Some(t) = w.opens_on(date) {
                    if t > at {
                        // Later dates for this window open later, so the first
                        // one past `at` is this window's answer.
                        best = Some(best.map_or(t, |b: DateTime<Utc>| b.min(t)));
                        break;
                    }
                }
            }
        }
        best
    }

    /// The first instant at or after `at` at which the session is closed — the
    /// boundary a live child drains to (§7 row 1).
    ///
    /// `None` when the session is not open at `at` in the first place, when it
    /// is always open, and when the chain of overlapping windows runs longer
    /// than [`MAX_OVERLAP_CHAIN`]. Callers that need those three told apart read
    /// [`Self::openness`] instead.
    #[must_use]
    pub fn closes_after(&self, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.is_always_open() || !self.is_open_at(at) {
            return None;
        }
        let mut t = at;
        for _ in 0..MAX_OVERLAP_CHAIN {
            // The session is open while *any* window contains it, so the
            // boundary is the last of the ones covering `t` to close. Two
            // windows that hand over — 09:00–12:00 and 11:00–15:00 — are one
            // stretch of open time and one drain, not two.
            let close = self
                .windows
                .iter()
                .filter(|w| w.contains(t))
                .filter_map(|w| w.closes_after(t))
                .max()?;
            if !self.is_open_at(close) {
                return Some(close);
            }
            t = close;
        }
        None
    }

    /// Everything the driver asks the calendar in one answer.
    #[must_use]
    pub fn openness(&self, at: DateTime<Utc>) -> Openness {
        if self.is_always_open() {
            return Openness::AlwaysOpen;
        }
        if self.is_open_at(at) {
            self.closes_after(at)
                .map_or(Openness::Indeterminate, |closes_at| Openness::Open {
                    closes_at,
                })
        } else {
            self.next_open_after(at)
                .map_or(Openness::Indeterminate, |opens_at| Openness::Closed {
                    opens_at,
                })
        }
    }
}

/// Where an instant stands against a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Openness {
    /// No windows. The session claims whenever it has budget, and never
    /// re-arms on a boundary because there is none.
    AlwaysOpen,
    /// Inside a window. A child may start, and the one running drains at
    /// `closes_at`.
    Open { closes_at: DateTime<Utc> },
    /// Outside every window. Nothing is claimed; the driver re-arms on
    /// `opens_at`.
    Closed { opens_at: DateTime<Utc> },
    /// The schedule parses but its next boundary is beyond
    /// [`SEARCH_HORIZON_DAYS`].
    ///
    /// Unreachable for a window that recurs weekly, which is every window the
    /// charter can express. It exists so that a caller is handed a word it can
    /// pause on rather than an instant somebody guessed: a driver that treated
    /// "I could not find the boundary" as "open" would be a driver that ran
    /// outside its fence at the one moment the calendar was confused.
    Indeterminate,
}

impl Openness {
    /// Whether a session may claim now.
    #[must_use]
    pub fn may_claim(self) -> bool {
        matches!(self, Openness::AlwaysOpen | Openness::Open { .. })
    }

    /// The instant the driver re-arms on, if there is one.
    #[must_use]
    pub fn re_arms_at(self) -> Option<DateTime<Utc>> {
        match self {
            Openness::Open { closes_at } => Some(closes_at),
            Openness::Closed { opens_at } => Some(opens_at),
            Openness::AlwaysOpen | Openness::Indeterminate => None,
        }
    }

    /// The word the `window_closed` event carries.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Openness::AlwaysOpen => "always_open",
            Openness::Open { .. } => "open",
            Openness::Closed { .. } => "closed",
            Openness::Indeterminate => "indeterminate",
        }
    }
}

/// ISO weekday, Monday = 1, as [`crate::charter::Window::days`] numbers them.
fn iso_weekday(date: NaiveDate) -> u8 {
    u8::try_from(date.weekday().number_from_monday()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-01-05 is a Monday; New York is on EST that week.
    const MON_JAN_5: (i32, u32, u32) = (2026, 1, 5);

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("a real UTC instant")
    }

    fn window(days: &[u8], start: &str, end: &str, tz: &str) -> Window {
        Window {
            days: days.to_vec(),
            start: start.to_string(),
            end: end.to_string(),
            tz: tz.to_string(),
        }
    }

    fn hours(windows: Vec<Window>) -> OpenHours {
        OpenHours::parse(&Schedule {
            windows,
            max_concurrent_runs: 1,
        })
        .expect("these windows parse")
    }

    /// Weekdays 09:00–17:00 New York — the schedule a charter form produces by
    /// default when somebody touches the grid at all.
    fn weekdays_nine_to_five() -> OpenHours {
        hours(vec![window(
            &[1, 2, 3, 4, 5],
            "09:00",
            "17:00",
            "America/New_York",
        )])
    }

    #[test]
    fn no_windows_means_always_open() {
        let h = OpenHours::parse(&Schedule::default()).expect("the default schedule parses");
        assert!(h.is_always_open());
        // Every hour a session that was *meant* to be fenced would be stopped.
        for (d, hh) in [(5, 3), (5, 14), (10, 3), (10, 23), (11, 0)] {
            let t = utc(2026, 1, d, hh, 0);
            assert!(
                h.is_open_at(t),
                "an empty window list stopped the session at {t}: it means always open, the same \
                 way an empty scope means everything"
            );
        }
        assert_eq!(h.openness(utc(2026, 1, 10, 3, 0)), Openness::AlwaysOpen);
        assert!(h.openness(utc(2026, 1, 10, 3, 0)).may_claim());
        assert_eq!(h.next_open_after(utc(2026, 1, 5, 9, 0)), None);
        assert_eq!(h.closes_after(utc(2026, 1, 5, 9, 0)), None);
    }

    #[test]
    fn a_window_is_read_in_its_own_timezone() {
        let h = weekdays_nine_to_five();
        let (y, m, d) = MON_JAN_5;
        // 09:00 EST is 14:00 UTC. Reading these as UTC would have the session
        // open from 09:00 UTC, five hours early.
        assert!(
            !h.is_open_at(utc(y, m, d, 9, 0)),
            "09:00 UTC is 04:00 in NY"
        );
        assert!(!h.is_open_at(utc(y, m, d, 13, 59)));
        assert!(h.is_open_at(utc(y, m, d, 14, 0)), "09:00 EST");
        assert!(h.is_open_at(utc(y, m, d, 21, 59)), "16:59 EST");
        assert!(
            !h.is_open_at(utc(y, m, d, 22, 0)),
            "17:00 EST, and end is exclusive"
        );
        // Saturday is not one of its days.
        assert!(!h.is_open_at(utc(2026, 1, 10, 14, 0)));
    }

    #[test]
    fn the_next_window_is_the_next_day_it_names() {
        let h = weekdays_nine_to_five();
        // Monday evening → Tuesday morning.
        assert_eq!(
            h.next_open_after(utc(2026, 1, 5, 23, 0)),
            Some(utc(2026, 1, 6, 14, 0))
        );
        // Inside Monday's window the *next* opening is still Tuesday's: the one
        // it is inside already opened.
        assert_eq!(
            h.next_open_after(utc(2026, 1, 5, 15, 0)),
            Some(utc(2026, 1, 6, 14, 0))
        );
        // Friday night → Monday, over a weekend it does not name.
        assert_eq!(
            h.next_open_after(utc(2026, 1, 9, 23, 0)),
            Some(utc(2026, 1, 12, 14, 0))
        );
    }

    #[test]
    fn the_openness_of_an_instant_says_when_the_child_drains() {
        let h = weekdays_nine_to_five();
        assert_eq!(
            h.openness(utc(2026, 1, 5, 15, 0)),
            Openness::Open {
                closes_at: utc(2026, 1, 5, 22, 0)
            }
        );
        assert_eq!(
            h.openness(utc(2026, 1, 5, 23, 0)),
            Openness::Closed {
                opens_at: utc(2026, 1, 6, 14, 0)
            }
        );
        assert!(h.openness(utc(2026, 1, 5, 15, 0)).may_claim());
        assert!(!h.openness(utc(2026, 1, 5, 23, 0)).may_claim());
        assert_eq!(h.openness(utc(2026, 1, 5, 15, 0)).as_str(), "open");
    }

    #[test]
    fn two_windows_that_hand_over_are_one_drain() {
        let h = hours(vec![
            window(&[1], "09:00", "12:00", "UTC"),
            window(&[1], "11:00", "15:00", "UTC"),
        ]);
        let (y, m, d) = MON_JAN_5;
        assert!(h.is_open_at(utc(y, m, d, 11, 30)));
        assert_eq!(
            h.closes_after(utc(y, m, d, 9, 30)),
            Some(utc(y, m, d, 15, 0)),
            "a child that drained at noon would drain again at 12:01"
        );
    }

    #[test]
    fn a_window_ending_at_twenty_four_hundred_runs_to_the_end_of_the_day() {
        let h = hours(vec![window(&[1], "22:00", "24:00", "UTC")]);
        let (y, m, d) = MON_JAN_5;
        assert!(h.is_open_at(utc(y, m, d, 23, 59)));
        assert!(!h.is_open_at(utc(y, m, d, 21, 59)));
        assert!(
            !h.is_open_at(utc(y, m, d + 1, 0, 0)),
            "the day boundary is where it ends, and Tuesday is not one of its days"
        );
        assert_eq!(
            h.closes_after(utc(y, m, d, 23, 0)),
            Some(utc(y, m, d + 1, 0, 0))
        );

        // And `00:00` is not a second spelling of it: that window never opens,
        // and the refusal says so rather than running the session for 0 minutes
        // or for 24 hours.
        assert_eq!(
            OpenHours::parse(&Schedule {
                windows: vec![window(&[1], "22:00", "00:00", "UTC")],
                max_concurrent_runs: 1,
            }),
            Err(ScheduleError::NeverOpens {
                start: "22:00".into(),
                end: "00:00".into()
            })
        );
    }

    #[test]
    fn an_end_before_its_start_is_refused_rather_than_wrapped() {
        for (start, end) in [("22:00", "06:00"), ("09:00", "09:00"), ("24:00", "24:00")] {
            assert_eq!(
                ParsedWindow::parse(&window(&[1], start, end, "UTC")),
                Err(ScheduleError::NeverOpens {
                    start: start.into(),
                    end: end.into()
                }),
                "{start}–{end} was accepted"
            );
        }
        // The remedy the error names does work, and it keeps the days honest:
        // the small hours belong to Tuesday, which is what a wrapped window
        // would have hidden.
        let h = hours(vec![
            window(&[1], "22:00", "24:00", "UTC"),
            window(&[2], "00:00", "06:00", "UTC"),
        ]);
        assert!(h.is_open_at(utc(2026, 1, 5, 23, 0)));
        assert!(h.is_open_at(utc(2026, 1, 6, 2, 0)));
        assert!(!h.is_open_at(utc(2026, 1, 6, 7, 0)));
    }

    #[test]
    fn an_unknown_timezone_is_refused_by_name_and_never_becomes_utc() {
        let typo = "Amercia/New_York";
        assert_eq!(
            OpenHours::parse(&Schedule {
                windows: vec![window(&[1, 2, 3, 4, 5], "09:00", "17:00", typo)],
                max_concurrent_runs: 1,
            }),
            Err(ScheduleError::UnknownTimezone { tz: typo.into() }),
        );
        assert!(
            ScheduleError::UnknownTimezone { tz: typo.into() }
                .to_string()
                .contains(typo),
            "the message has to name the string somebody has to go and fix"
        );

        // The refusal is worth something because the fallback it prevents is
        // visible: read as UTC, that window is open at an instant the operator
        // fenced off, five hours before New York opens.
        let as_utc = hours(vec![window(&[1, 2, 3, 4, 5], "09:00", "17:00", "UTC")]);
        assert!(
            as_utc.is_open_at(utc(2026, 1, 5, 9, 30)),
            "if this is not open then the silent-UTC fallback would have been harmless and this \
             test proves nothing"
        );
        assert!(!weekdays_nine_to_five().is_open_at(utc(2026, 1, 5, 9, 30)));

        // One bad window refuses the whole schedule, so a session never runs on
        // the half of a grid that happened to parse.
        assert!(OpenHours::parse(&Schedule {
            windows: vec![
                window(&[1], "09:00", "17:00", "UTC"),
                window(&[2], "09:00", "17:00", typo),
            ],
            max_concurrent_runs: 1,
        })
        .is_err());
    }

    #[test]
    fn a_local_hour_that_happens_twice_is_inside_the_window_both_times() {
        // 2026-11-01, America/New_York: at 06:00 UTC the clock reads 02:00 EDT
        // and goes back to 01:00 EST, so 01:30 happens at 05:30 UTC and again
        // at 06:30 UTC.
        let h = hours(vec![window(&[7], "01:00", "02:00", "America/New_York")]);
        assert!(h.is_open_at(utc(2026, 11, 1, 5, 30)), "01:30 EDT");
        assert!(
            h.is_open_at(utc(2026, 11, 1, 6, 30)),
            "01:30 EST, the second pass"
        );
        assert!(!h.is_open_at(utc(2026, 11, 1, 7, 0)), "02:00 EST");
        assert_eq!(
            h.closes_after(utc(2026, 11, 1, 5, 30)),
            Some(utc(2026, 11, 1, 7, 0)),
            "the window covers two hours of real time on this night, and the drain is at the end \
             of the second"
        );
        // It opens at the first pass of 01:00, not the second.
        assert_eq!(
            h.next_open_after(utc(2026, 10, 31, 12, 0)),
            Some(utc(2026, 11, 1, 5, 0))
        );
    }

    #[test]
    fn a_local_hour_that_never_happens_does_not_open_a_window() {
        // 2026-03-08, America/New_York: at 07:00 UTC the clock jumps from 02:00
        // EST to 03:00 EDT. Local 02:00–03:00 does not exist that morning.
        let swallowed = hours(vec![window(&[7], "02:00", "02:30", "America/New_York")]);
        for minute in 0..120 {
            let t = utc(2026, 3, 8, 5, 0) + Duration::minutes(minute);
            assert!(
                !swallowed.is_open_at(t),
                "{t} was called open, but the clock never reads 02:00–02:30 that morning"
            );
        }
        assert_eq!(
            swallowed.next_open_after(utc(2026, 3, 7, 12, 0)),
            Some(utc(2026, 3, 15, 6, 0)),
            "the honest answer is the following Sunday, at 02:00 EDT"
        );

        // A window that straddles the gap opens the instant the clock passes
        // its start, which is the jump itself.
        let straddling = hours(vec![window(&[7], "02:00", "04:00", "America/New_York")]);
        assert_eq!(
            straddling.next_open_after(utc(2026, 3, 7, 12, 0)),
            Some(utc(2026, 3, 8, 7, 0)),
            "03:00 EDT, the far side of the gap"
        );
        assert!(straddling.is_open_at(utc(2026, 3, 8, 7, 0)));
        assert_eq!(
            straddling.closes_after(utc(2026, 3, 8, 7, 0)),
            Some(utc(2026, 3, 8, 8, 0)),
            "04:00 EDT"
        );
    }

    #[test]
    fn every_instant_the_calendar_re_arms_on_is_one_it_calls_open() {
        // The invariant that stops a driver looping: re-arm, wake, find itself
        // closed, re-arm on the same instant. Swept across the two transitions
        // and a fortnight of ordinary days.
        let schedules = [
            weekdays_nine_to_five(),
            hours(vec![window(&[7], "02:00", "02:30", "America/New_York")]),
            hours(vec![window(&[7], "01:00", "02:00", "America/New_York")]),
            hours(vec![window(
                &[1, 2, 3, 4, 5, 6, 7],
                "22:00",
                "24:00",
                "UTC",
            )]),
            hours(vec![
                window(&[1], "09:00", "12:00", "Europe/London"),
                window(&[1], "11:00", "15:00", "Asia/Tokyo"),
            ]),
        ];
        let starts = [
            utc(2026, 1, 5, 0, 0),
            utc(2026, 3, 7, 12, 0),
            utc(2026, 3, 8, 6, 30),
            utc(2026, 10, 31, 12, 0),
            utc(2026, 11, 1, 5, 45),
        ];
        let mut checked = 0_u32;
        for h in &schedules {
            for start in starts {
                for hour in 0..48 {
                    let at = start + Duration::hours(hour);
                    if let Some(opens) = h.next_open_after(at) {
                        assert!(opens > at, "{opens} is not after {at}");
                        assert!(
                            h.is_open_at(opens),
                            "the driver would wake at {opens} and find itself closed"
                        );
                        checked += 1;
                    }
                    if let Some(closes) = h.closes_after(at) {
                        assert!(h.is_open_at(at));
                        assert!(!h.is_open_at(closes), "{closes} is not a boundary");
                        assert!(closes > at);
                        checked += 1;
                    }
                }
            }
        }
        assert!(
            checked > 500,
            "the sweep asserted on {checked} instants, which is not enough of them to be evidence"
        );
    }

    #[test]
    fn the_day_budget_resumes_on_a_later_day_not_on_a_later_window() {
        // Two windows on the same Monday. Spending the day budget in the first
        // must not resume in the second: it is the same day's budget.
        let h = hours(vec![
            window(&[1, 2], "09:00", "12:00", "UTC"),
            window(&[1, 2], "14:00", "17:00", "UTC"),
        ]);
        let (y, m, d) = MON_JAN_5;
        let spent_at = utc(y, m, d, 10, 0);
        assert_eq!(
            h.next_open_after(spent_at),
            Some(utc(y, m, d, 14, 0)),
            "the window boundary is still the afternoon"
        );
        assert_eq!(
            h.next_open_on_a_later_day(spent_at),
            Some(utc(y, m, d + 1, 9, 0)),
            "but the day budget resets on the next day's window"
        );
        // With no windows there is no schedule day to wait for, and saying so is
        // better than inventing a midnight in a zone nobody named.
        assert_eq!(
            OpenHours::always_open().next_open_on_a_later_day(spent_at),
            None
        );
    }

    #[test]
    fn the_clock_parser_takes_hh_mm_and_nothing_else() {
        assert_eq!(parse_clock("start", "00:00"), Ok(0));
        assert_eq!(parse_clock("start", "09:30"), Ok(570));
        assert_eq!(parse_clock("end", "24:00"), Ok(MINUTES_PER_DAY));
        for bad in [
            "", "9", "0900", "09:60", "25:00", "24:01", "09:00:00", "ab:cd",
        ] {
            assert_eq!(
                parse_clock("end", bad),
                Err(ScheduleError::BadClock {
                    field: "end",
                    got: bad.to_string()
                }),
                "`{bad}` parsed"
            );
        }
    }

    #[test]
    fn a_window_with_no_days_or_a_day_that_is_not_a_weekday_is_refused() {
        assert_eq!(
            ParsedWindow::parse(&window(&[], "09:00", "17:00", "UTC")),
            Err(ScheduleError::NoDays)
        );
        for bad in [0_u8, 8, 9] {
            assert_eq!(
                ParsedWindow::parse(&window(&[1, bad], "09:00", "17:00", "UTC")),
                Err(ScheduleError::BadDay { got: bad })
            );
        }
        // Monday is 1 and Sunday is 7, which is the ISO numbering the charter
        // documents and the one chrono counts from Monday.
        let w = ParsedWindow::parse(&window(&[7, 1, 1], "09:00", "17:00", "UTC"))
            .expect("Monday and Sunday");
        assert_eq!(w.days(), &[1, 7], "sorted and deduplicated");
        assert!(w.contains(utc(2026, 1, 5, 10, 0)), "Monday");
        assert!(w.contains(utc(2026, 1, 11, 10, 0)), "Sunday");
        assert!(!w.contains(utc(2026, 1, 10, 10, 0)), "Saturday");
        assert_eq!(w.start_minutes(), 540);
        assert_eq!(w.end_minutes(), 1020);
        assert_eq!(w.tz(), chrono_tz::UTC);
    }

    // ── The re-arm (§7 rows 1-2: "the driver re-arms on the window boundary") ──
    //
    // `Openness::re_arms_at` had no test. It is the instant the driver sleeps
    // until, so the two ways it can be wrong are both ways a session stops
    // working: an instant already past busy-loops the worker, and an instant
    // taken from the wrong side of the boundary sleeps through the window.

    #[test]
    fn a_calendar_that_found_no_boundary_offers_no_instant_to_re_arm_on() {
        // `Indeterminate` is the word `openness` returns when it could not find
        // the next boundary. Handing back an instant anyway would be the
        // guessed fence this enum exists to refuse, so it is `None` — and a
        // driver has to treat that as "hold", which is what
        // `HoldReason::ScheduleIndeterminate` is for.
        assert_eq!(Openness::Indeterminate.re_arms_at(), None);
        assert!(!Openness::Indeterminate.may_claim());
    }

    #[test]
    fn always_open_re_arms_on_nothing_which_is_not_the_same_as_re_arming_now() {
        // There is no boundary, so there is no instant. A caller reading `None`
        // as "wake immediately" would spin; a caller reading it as "there is no
        // calendar to wait for" is right, and `may_claim` is how it tells the
        // two `None`s apart.
        assert_eq!(Openness::AlwaysOpen.re_arms_at(), None);
        assert!(Openness::AlwaysOpen.may_claim());
        assert!(!Openness::Indeterminate.may_claim());
    }

    #[test]
    fn an_open_window_re_arms_on_its_close_and_a_closed_one_on_its_open() {
        // Swapped, a driver inside a window would sleep until the window it is
        // already in opens — i.e. not wake until next week — and a driver
        // outside one would wake at a close it will never see.
        let h = weekdays_nine_to_five();
        let (y, m, d) = MON_JAN_5;
        let inside = utc(y, m, d, 15, 0);
        let outside = utc(y, m, d, 23, 0);

        let open = h.openness(inside);
        assert!(open.may_claim());
        assert_eq!(open.re_arms_at(), h.closes_after(inside));

        let closed = h.openness(outside);
        assert!(!closed.may_claim());
        assert_eq!(closed.re_arms_at(), Some(h.next_open_after(outside).unwrap()));
    }

    #[test]
    fn the_re_arm_is_never_an_instant_already_past() {
        // A re-arm at or before the instant it was computed at is an immediate
        // re-wake, which is a driver spinning rather than waiting. Swept over a
        // week of a real schedule, and it counts what it checked so it cannot
        // pass by checking nothing — the same guard `next_open_after`'s own
        // sweep carries.
        let schedules = [
            weekdays_nine_to_five(),
            hours(vec![window(&[6, 7], "10:00", "14:00", "Europe/London")]),
            hours(vec![
                window(&[1], "22:00", "24:00", "UTC"),
                window(&[2], "00:00", "02:00", "UTC"),
            ]),
        ];
        let mut checked = 0;
        for h in &schedules {
            let (y, m, d) = MON_JAN_5;
            for hour in 0..(24 * 7) {
                let at = utc(y, m, d, 0, 0) + chrono::Duration::hours(i64::from(hour));
                if let Some(next) = h.openness(at).re_arms_at() {
                    assert!(
                        next > at,
                        "re-arming at {next} from {at} wakes the driver into the same decision"
                    );
                    checked += 1;
                }
            }
        }
        assert!(
            checked > 400,
            "the sweep found only {checked} boundaries, so it proved almost nothing"
        );
    }

    #[test]
    fn the_concurrency_rides_along_so_the_driver_holds_one_object() {
        let schedule = Schedule::default()
            .with_concurrency(3)
            .expect("three is inside the range");
        let h = OpenHours::parse(&schedule).expect("the default windows parse");
        assert_eq!(h.max_concurrent_runs(), 3);
        assert_eq!(OpenHours::always_open().max_concurrent_runs(), 1);
        assert!(h.windows().is_empty());
    }
}
