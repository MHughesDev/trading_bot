//! The silence alarm: telling a feed that stopped from a market that is quiet.
//!
//! A data feed that stops is indistinguishable from a market that stopped
//! trading. A strategy trading off a dead feed sees a flat price and takes no
//! action, which looks exactly like a strategy correctly deciding not to trade.
//! Nothing downstream reports an error, because nothing went wrong — rows simply
//! stopped arriving, and every count of them is still a valid number.
//!
//! # The distinction this module exists to make
//!
//! There are two clocks on every observation and the platform has been reading
//! only one of them.
//!
//! * **The venue's clock** — `last_event_time`, when the fact happened. It moves
//!   when the market moves. A closed exchange, a maintenance window, an
//!   instrument that does not trade overnight: all of these legitimately stop it.
//! * **Our clock** — `last_write_time`, when the collector last wrote anything at
//!   all. Nothing in any market stops *our* clock. Only we can stop it, by dying.
//!
//! An alarm on the venue's clock fires every weekend, and an alarm that fires
//! every weekend gets muted — which is worse than no alarm, because it converts a
//! real signal into noise someone has already learned to ignore. An alarm on our
//! clock needs no trading calendar, no holiday table and no venue timetable, and
//! it is wrong in exactly one situation: when our collector has stopped. That is
//! [`Verdict::CollectorSilent`], and it is the only verdict here that alarms
//! under every arrival pattern, in every market, on every calendar.
//!
//! **This module does not contain a calendar and must not grow one.** Where the
//! platform cannot know whether a series *should* be arriving, it says so
//! ([`Arrival::Unknown`], [`Roster::WhateverAppears`], [`Verdict::CannotJudge`])
//! and alarms only on what it can defend.
//!
//! # Why the venue's clock alone is not merely weak but actively wrong
//!
//! Measured against the live ClickHouse store on 2026-09-22, `market_bars_v2`
//! holds four instrument feeds. Three have an event age and a write age that
//! agree to the second — genuinely dead persisters, stopped 8 to 9 days ago. The
//! fourth, `SYN-PLANTED-AR1-424242`, carries a newest event time in **2020** and
//! a newest write time **ten days ago**: a planted synthetic fixture. An alarm
//! reading `max(event_time)` would page that its data is six years overdue, about
//! rows that a test wrote a fortnight ago. One such page per week is all it takes
//! for the alarm to be muted, after which the three genuinely dead feeds stay
//! dead in silence. Separating the clocks is not a refinement; it is the
//! difference between an alarm that survives contact with the store and one that
//! does not.
//!
//! # What is delegated
//!
//! The deadline arithmetic for the venue's clock is
//! [`dataplane::retention::alarm`] and is called, not copied — two silence alarms
//! with two deadline rules would drift, and the one nobody was looking at would
//! be the one that was right. What this module adds is everything that a
//! single-series, single-timestamp function structurally cannot express: the
//! second clock, the per-instrument fan-out, the distinction between an absent
//! measurement and a measured zero, and the rollup that turns a shared outage
//! into one page instead of N.
//!
//! # Purity
//!
//! There is no database client here, of any kind, and there must not be one. A
//! component that measures staleness *from* the store it is measuring cannot tell
//! a dead feed from its own outage — it reports every series silent and means
//! nothing by it. This module is handed a [`Census`] by whoever holds the
//! connection and judges it. `no_db_client_creeps_into_this_crate` is the test
//! that keeps it that way.

use chrono::{DateTime, Duration, Utc};
use dataplane::retention::{self, AlarmState, ForwardSeriesSpec};
use serde::{Deserialize, Serialize};

/// The instrument placeholder used when a series is judged as a whole because no
/// roster of instruments exists to judge it against.
pub const WHOLE_SERIES: &str = "*";

/// How far a timestamp may sit in the future before it is a clock fault rather
/// than ordinary skew between our host and the collector's.
pub const CLOCK_SKEW_TOLERANCE_SECS: i64 = 120;

/// Below this many judgeable feeds, "they all went silent together" is not
/// evidence of a shared cause — it is two feeds, which is a coincidence.
pub const MIN_FEEDS_FOR_SHARED_CAUSE: usize = 3;

/// The share of judgeable feeds that must be silent before a shared cause is
/// more likely than that many independent collector deaths.
pub const SHARED_CAUSE_PCT: usize = 80;

// ── Identity ─────────────────────────────────────────────────────────────────

/// One feed: a series, at a venue, for one instrument.
///
/// The per-instrument grain is the point. `dataplane.forward_series_alarm` is
/// keyed on `series_id` alone, one row per series, so "the series produced a row"
/// can be true while every instrument but one is dead. Kraken Futures publishes
/// 279 perpetuals; a series-grained alarm reads healthy when 278 of them have
/// stopped.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FeedKey {
    /// Either the registry id (`dataplane.forward_series.series_id`, e.g.
    /// `"kraken_futures.funding"`) or the store table the heartbeat writer
    /// actually records (`"funding_rates"`). See [`FeedExpectation::owns`] for
    /// why both are accepted.
    pub series: String,
    pub venue: String,
    pub instrument: String,
}

impl FeedKey {
    pub fn new(series: impl Into<String>, venue: impl Into<String>, instrument: impl Into<String>) -> Self {
        Self { series: series.into(), venue: venue.into(), instrument: instrument.into() }
    }
}

/// One row of `forward_series_heartbeat`, as a reader hands it over.
///
/// Both clocks are mandatory and neither has a default. A heartbeat that could
/// be constructed with only the venue's timestamp would reintroduce the exact
/// conflation this module exists to undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    pub feed: FeedKey,
    /// The newest event the feed carried — the venue's clock. Legitimately stops
    /// when the market does. May legitimately sit in the *future*: a predicted
    /// funding rate is stamped with the settlement instant it predicts, which is
    /// up to one settlement interval ahead.
    pub last_event_time: DateTime<Utc>,
    /// When the collector last wrote. Our clock. Nothing in any market stops it.
    pub last_write_time: DateTime<Utc>,
    /// Rows in the write that produced this beat. Zero is a real measurement —
    /// "we polled and the venue had nothing" — and is not the same as no beat.
    pub rows_written: u64,
}

// ── What the platform can know about when a series should arrive ─────────────

/// The arrival pattern a series was declared with.
///
/// This is platform-internal knowledge — what we built the collector to do — and
/// never a claim about market hours. Deciding which variant a series gets is the
/// one judgement call in this module, and it is made once, in configuration,
/// rather than inferred per tick from data that cannot support the inference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "arrival", rename_all = "snake_case")]
pub enum Arrival {
    /// The venue publishes on a fixed period with no sessions and no closures.
    ///
    /// Kraken Futures settles perpetual funding every hour of every day including
    /// weekends and holidays, and publishes open interest continuously; both are
    /// registered in `dataplane.forward_series` at `cadence_secs = 3600`. For
    /// these, and only these, an absence of *events* is itself wrong and is
    /// alarmed. Justifying this variant for a series means being able to name the
    /// venue's published guarantee that it never stops.
    Always,
    /// We poll on our own schedule and the venue may legitimately have nothing.
    ///
    /// An equity option chain outside regular hours, a venue in a maintenance
    /// window, an instrument that does not trade overnight. The platform holds no
    /// trading calendar and this module refuses to invent one, so an absence of
    /// events here is reported ([`Verdict::QuietAndPolledOnTime`]) and never
    /// alarmed. What *is* alarmed is an absence of our own poll, which no market
    /// closure can explain.
    Polled { poll_period_secs: u32 },
    /// We do not know when this series should arrive.
    ///
    /// Stated rather than guessed. Yields [`Verdict::CannotJudge`] and alarms on
    /// nothing — an honest blank is worth more than a threshold somebody made up,
    /// because the made-up threshold is the one that gets the alarm muted.
    Unknown,
}

/// Which instruments must appear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "roster", rename_all = "snake_case")]
pub enum Roster {
    /// The instruments the platform has promised to collect. One that produces no
    /// heartbeat is [`Verdict::NeverSeen`] — the only way a feed that never
    /// started can be noticed at all.
    Declared { instruments: Vec<String> },
    /// No roster exists, so only feeds that have been seen can be judged: an
    /// instrument that never appeared cannot be missed. The series as a whole is
    /// still judged under [`WHOLE_SERIES`], so a series that has produced nothing
    /// at all is still caught.
    ///
    /// This is a stated limitation, not a silent one. `dataplane.forward_series`
    /// has no instrument column, so today every series registered there lands
    /// here.
    WhateverAppears,
}

/// What the platform expects of one series.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedExpectation {
    /// The registry row. Reused rather than re-declared so the cadence and the
    /// `alarm_after_cadences` deadline cannot drift from the ones
    /// `dataplane::retention` measures against.
    pub spec: ForwardSeriesSpec,
    pub arrival: Arrival,
    pub roster: Roster,
}

impl FeedExpectation {
    /// Whether this expectation covers a given feed key.
    ///
    /// Accepts the registry id *or* the store table in `FeedKey::series`, because
    /// the two disagree in the live system: `storage::clickhouse::forward::beat`
    /// writes `series` = the store table (`"funding_rates"`), while
    /// `dataplane.forward_series.series_id` is `"kraken_futures.funding"`. A join
    /// on either alone silently matches nothing, and a census that matches
    /// nothing looks exactly like a census where nothing is wrong. Store tables
    /// are not unique — both option-chain series land in
    /// `option_chain_snapshots` — so the venue is always part of the match.
    #[must_use]
    pub fn owns(&self, feed: &FeedKey) -> bool {
        feed.venue == self.spec.venue && (feed.series == self.spec.series_id || feed.series == self.spec.store_table)
    }

    /// The feeds this expectation asserts must exist, when nothing has been
    /// observed to enumerate them.
    fn promised_feeds(&self) -> Vec<FeedKey> {
        match &self.roster {
            Roster::Declared { instruments } => {
                instruments.iter().map(|i| FeedKey::new(&self.spec.series_id, &self.spec.venue, i)).collect()
            }
            Roster::WhateverAppears => vec![FeedKey::new(&self.spec.series_id, &self.spec.venue, WHOLE_SERIES)],
        }
    }

    /// The deadline on *our* clock, in seconds: how long the collector may write
    /// nothing before that is wrong regardless of what any market is doing.
    ///
    /// `None` when the arrival pattern gives no basis to set one.
    #[must_use]
    pub fn write_deadline_secs(&self) -> Option<i64> {
        let cadences = i64::from(self.spec.alarm_after_cadences.max(1));
        match self.arrival {
            Arrival::Always => Some(i64::from(self.spec.cadence_secs.max(1)) * cadences),
            Arrival::Polled { poll_period_secs } => Some(i64::from(poll_period_secs.max(1)) * cadences),
            Arrival::Unknown => None,
        }
    }
}

// ── The census ───────────────────────────────────────────────────────────────

/// What a reader saw when it went to look.
///
/// Two variants rather than one `Vec`, because an absent measurement is never a
/// zero. A reader whose query fails cannot express that as an empty success
/// without deliberately constructing the wrong variant, and even then the failure
/// mode is loud: an empty [`Census::Taken`] yields [`Verdict::NeverSeen`] for
/// every promised feed, which alarms. There is no spelling of a failed read that
/// produces an all-clear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "census", rename_all = "snake_case")]
pub enum Census {
    Taken { at: DateTime<Utc>, beats: Vec<Heartbeat> },
    /// The read did not happen or did not complete. We are blind, which is a
    /// different fact from everything being fine.
    Failed { attempted_at: DateTime<Utc>, reason: String },
}

impl Census {
    /// Build a census straight from a reader's `Result`. This is the spelling to
    /// use at a call site: an error becomes [`Census::Failed`] without the caller
    /// having to remember that it should.
    pub fn from_read<E: std::fmt::Display>(at: DateTime<Utc>, read: Result<Vec<Heartbeat>, E>) -> Self {
        match read {
            Ok(beats) => Self::Taken { at, beats },
            Err(e) => Self::Failed { attempted_at: at, reason: e.to_string() },
        }
    }

    #[must_use]
    pub const fn at(&self) -> DateTime<Utc> {
        match self {
            Self::Taken { at, .. } | Self::Failed { attempted_at: at, .. } => *at,
        }
    }
}

// ── Verdicts ─────────────────────────────────────────────────────────────────

/// What is true of one feed right now.
///
/// Exactly one variant means health. Everything else is at minimum reportable,
/// because silence is never health and an unasked question is never an answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Writing on time, and — where the series was declared continuous — carrying
    /// fresh events. The only healthy verdict there is.
    Live { write_age_secs: i64, event_age_secs: i64 },

    /// Nothing written past the deadline on *our* clock. The dead persister.
    ///
    /// The one shape that is wrong under every arrival pattern, in every market,
    /// on every calendar — no closure, holiday or maintenance window stops our own
    /// collector from writing. This is what the alarm is really for, and it is
    /// checked before the venue's clock because it is the more actionable
    /// diagnosis: restart the collector, rather than go and ask the venue.
    CollectorSilent { write_age_secs: i64, deadline_secs: i64 },

    /// The collector is writing on time and the venue has published nothing new
    /// past the event deadline.
    ///
    /// Only reachable under [`Arrival::Always`], where an absence of events has
    /// no innocent explanation. `rows_missed` states the cost rather than the
    /// status: on a forward-only series those observations are unrecoverable at
    /// any price.
    EventsSilent { event_age_secs: i64, rows_missed: u64 },

    /// The poll is on time and the venue had nothing to give.
    ///
    /// A closed market, a maintenance window, an instrument that does not trade
    /// overnight. Not an alarm — and deliberately not [`Verdict::Live`] either. A
    /// quiet feed and a working one are different facts, and a dashboard that
    /// paints them the same colour is how a feed that went quiet in March gets
    /// noticed in June.
    QuietAndPolledOnTime { event_age_secs: i64, write_age_secs: i64 },

    /// Promised and never seen at all.
    ///
    /// Distinct from [`Verdict::CollectorSilent`] on purpose: a collector that
    /// never started and one that died look identical in a row count and need
    /// different fixes.
    NeverSeen,

    /// The census could not be taken. Blind, not healthy.
    ///
    /// Never an alarm against the collector — it has not been accused of
    /// anything, because nobody managed to look.
    Unobserved { reason: String },

    /// No defensible basis to judge this feed: either nothing declared an
    /// expectation for it, or the expectation declares [`Arrival::Unknown`].
    ///
    /// An honest blank. Inventing a threshold here is how an alarm earns its
    /// first false page.
    CannotJudge { reason: String },

    /// Collection deliberately switched off.
    ///
    /// Not health: a recorded decision to accept a permanent, unfillable hole in
    /// a series that cannot be re-collected. It should sit on a dashboard looking
    /// like what it is.
    Disabled,

    /// Timestamps that cannot both be true — a write stamped in the future.
    ///
    /// Reported rather than rounded off, because a clock fault is precisely how a
    /// dead feed acquires an alibi.
    ClockFault { detail: String },
}

/// How loud a verdict is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Reserved for [`Verdict::Live`] and nothing else.
    Ok,
    /// True, worth showing, nobody needs waking.
    Info,
    /// Something is wrong with our ability to judge, not with the feed.
    Warn,
    /// A feed that should be arriving is not.
    Alarm,
}

impl Verdict {
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            Self::Live { .. } => Severity::Ok,
            Self::QuietAndPolledOnTime { .. } | Self::CannotJudge { .. } | Self::Disabled => Severity::Info,
            Self::Unobserved { .. } | Self::ClockFault { .. } => Severity::Warn,
            Self::CollectorSilent { .. } | Self::EventsSilent { .. } | Self::NeverSeen => Severity::Alarm,
        }
    }

    #[must_use]
    pub fn is_alarming(&self) -> bool {
        self.severity() == Severity::Alarm
    }

    /// How this verdict may be recorded in `dataplane.forward_series_alarm`,
    /// whose CHECK admits only `'ok' | 'stale' | 'never_collected'`.
    ///
    /// `None` means **do not write a row** — the schema has no honest spelling for
    /// this verdict. It must never be read as "write `'ok'`". Three of this
    /// module's verdicts have no representation there at all, and the one that
    /// matters most is [`Verdict::Unobserved`]: a failed census recorded as `'ok'`
    /// is the platform certifying health it never measured. `migration_0098`'s
    /// `alarm_raised_iff_not_ok` CHECK actively pushes a caller that way, since
    /// any non-`ok` state demands a `raised_at`. See the module-level note in the
    /// crate README for the migration this needs.
    #[must_use]
    pub const fn pg_alarm_state(&self) -> Option<&'static str> {
        match self {
            Self::Live { .. } => Some("ok"),
            Self::CollectorSilent { .. } | Self::EventsSilent { .. } => Some("stale"),
            Self::NeverSeen => Some("never_collected"),
            // No honest column value exists. Writing nothing is correct; writing
            // 'ok' would be a lie the CHECK constraint would happily accept.
            Self::Unobserved { .. }
            | Self::CannotJudge { .. }
            | Self::Disabled
            | Self::ClockFault { .. }
            | Self::QuietAndPolledOnTime { .. } => None,
        }
    }
}

// ── Findings and the report ──────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub feed: FeedKey,
    pub verdict: Verdict,
}

impl Finding {
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.verdict.severity()
    }
}

/// Why N feeds going silent at once is probably one failure and not N.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedCause {
    pub silent_feeds: usize,
    pub judgeable_feeds: usize,
    pub reason: String,
}

/// What a single call should wake somebody for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "page", rename_all = "snake_case")]
pub enum Page {
    Feed(Box<Finding>),
    /// One page for a group that failed together. The per-feed findings are still
    /// in the report — nothing is hidden — but 279 simultaneous pages is a request
    /// to be muted, and a muted alarm is worse than none.
    SharedCause { cause: SharedCause, feeds: Vec<FeedKey> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SilenceReport {
    pub taken_at: DateTime<Utc>,
    pub findings: Vec<Finding>,
    pub shared_cause: Option<SharedCause>,
}

impl SilenceReport {
    /// True only when something was actually judged and every judgement was
    /// [`Verdict::Live`].
    ///
    /// An empty report is **not** all-clear. A feed with no rows and a feed nobody
    /// asked about must not produce the same answer, and "we evaluated zero feeds"
    /// is the second of those.
    #[must_use]
    pub fn all_clear(&self) -> bool {
        !self.findings.is_empty() && self.findings.iter().all(|f| f.severity() == Severity::Ok)
    }

    #[must_use]
    pub fn worst(&self) -> Option<Severity> {
        self.findings.iter().map(Finding::severity).max()
    }

    #[must_use]
    pub fn with_severity(&self, s: Severity) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.severity() == s).collect()
    }

    /// What to wake somebody for: one page per alarming feed, or a single page
    /// naming the group when they went silent together.
    #[must_use]
    pub fn pages(&self) -> Vec<Page> {
        let alarming: Vec<&Finding> = self.findings.iter().filter(|f| f.verdict.is_alarming()).collect();
        if let Some(cause) = &self.shared_cause {
            let feeds = alarming
                .iter()
                .filter(|f| matches!(f.verdict, Verdict::CollectorSilent { .. }))
                .map(|f| f.feed.clone())
                .collect();
            let mut pages = vec![Page::SharedCause { cause: cause.clone(), feeds }];
            // Alarms with a different shape are not explained by the shared cause
            // and still stand on their own.
            pages.extend(
                alarming
                    .iter()
                    .filter(|f| !matches!(f.verdict, Verdict::CollectorSilent { .. }))
                    .map(|f| Page::Feed(Box::new((*f).clone()))),
            );
            return pages;
        }
        alarming.into_iter().map(|f| Page::Feed(Box::new(f.clone()))).collect()
    }

    /// One report from several, for a pass that had to ask more than one store.
    ///
    /// The shared cause is **recomputed over the union**, not carried over from
    /// the parts. This is the whole reason merging is a function rather than a
    /// `Vec::extend`: a host, a disk or a writer process that dies takes several
    /// stores with it, and each store's own report sees too few silent feeds to
    /// call it. Two reports of two silent feeds each are four silent feeds and one
    /// incident, and pasting the findings together without re-running
    /// [`infer_shared_cause`] would page four times for it.
    ///
    /// `at` is the instant of the pass, supplied rather than inferred, because
    /// merging nothing must still produce a report that is stamped — and one that
    /// is **not** all-clear. `SilenceReport::merged(at, vec![])` has no findings,
    /// so [`Self::all_clear`] is false: a pass that asked nothing learned nothing.
    #[must_use]
    pub fn merged(at: DateTime<Utc>, reports: Vec<Self>) -> Self {
        let findings: Vec<Finding> = reports.into_iter().flat_map(|r| r.findings).collect();
        let shared_cause = infer_shared_cause(&findings);
        Self { taken_at: at, findings, shared_cause }
    }
}

// ── Evaluation ───────────────────────────────────────────────────────────────

/// Judge one feed against one expectation.
///
/// `beat` is `None` when the feed was promised and no heartbeat exists for it.
#[must_use]
pub fn judge(exp: &FeedExpectation, beat: Option<&Heartbeat>, now: DateTime<Utc>) -> Verdict {
    if !exp.spec.enabled {
        return Verdict::Disabled;
    }

    let Some(deadline_secs) = exp.write_deadline_secs() else {
        return Verdict::CannotJudge {
            reason: format!("{} declares an unknown arrival pattern; no deadline can be defended", exp.spec.series_id),
        };
    };

    let Some(b) = beat else { return Verdict::NeverSeen };

    // Our own clock cannot run ahead of us. A write stamped in the future is a
    // fault, and left unflagged it is an alibi: it makes `write_age` negative and
    // a dead collector look eternally fresh.
    let write_age_secs = (now - b.last_write_time).num_seconds();
    if write_age_secs < -CLOCK_SKEW_TOLERANCE_SECS {
        return Verdict::ClockFault {
            detail: format!(
                "{} last wrote at {}, which is {}s after the census at {now}",
                b.feed.instrument,
                b.last_write_time,
                -write_age_secs
            ),
        };
    }

    // Our clock first: it is the one no market can stop, and the one whose
    // failure has an unambiguous fix.
    if write_age_secs > deadline_secs {
        return Verdict::CollectorSilent { write_age_secs, deadline_secs };
    }

    let event_age_secs = (now - b.last_event_time).num_seconds();

    match exp.arrival {
        Arrival::Always => {
            // Delegated, never copied: one deadline rule for the venue's clock.
            let a = retention::alarm(&exp.spec, Some(b.last_event_time), now);
            match a.state {
                AlarmState::Stale => Verdict::EventsSilent { event_age_secs, rows_missed: a.rows_missed },
                // `NeverCollected` is unreachable here — we hold a heartbeat, so a
                // last event time exists. Treating it as live would be a guess;
                // saying so is not.
                AlarmState::NeverCollected => Verdict::CannotJudge {
                    reason: format!("{} reported no last event despite a heartbeat", b.feed.instrument),
                },
                AlarmState::Ok => Verdict::Live { write_age_secs, event_age_secs },
            }
        }
        Arrival::Polled { .. } => {
            // The venue's clock is reported and never alarmed. This is the whole
            // of the weekend problem: we do not know this venue's calendar, so we
            // do not pretend to, and we do not page about it either.
            if event_age_secs > deadline_secs {
                Verdict::QuietAndPolledOnTime { event_age_secs, write_age_secs }
            } else {
                Verdict::Live { write_age_secs, event_age_secs }
            }
        }
        Arrival::Unknown => unreachable!("write_deadline_secs returns None for Unknown and we returned above"),
    }
}

/// Judge a whole census.
#[must_use]
pub fn evaluate(expectations: &[FeedExpectation], census: &Census) -> SilenceReport {
    let taken_at = census.at();

    let beats = match census {
        Census::Failed { reason, .. } => {
            // Blind. Every promised feed is Unobserved — never NeverSeen, which
            // would blame a collector for our own failure to look, and never Ok.
            let findings = expectations
                .iter()
                .flat_map(|e| {
                    e.promised_feeds()
                        .into_iter()
                        .map(|feed| Finding { feed, verdict: Verdict::Unobserved { reason: reason.clone() } })
                })
                .collect();
            return SilenceReport { taken_at, findings, shared_cause: None };
        }
        Census::Taken { beats, .. } => beats,
    };

    let mut findings: Vec<Finding> = Vec::new();

    for exp in expectations {
        let mine: Vec<&Heartbeat> = beats.iter().filter(|b| exp.owns(&b.feed)).collect();

        match &exp.roster {
            Roster::Declared { instruments } => {
                for inst in instruments {
                    let beat = mine.iter().copied().find(|b| &b.feed.instrument == inst);
                    let feed = FeedKey::new(&exp.spec.series_id, &exp.spec.venue, inst);
                    findings.push(Finding { feed, verdict: judge(exp, beat, taken_at) });
                }
            }
            Roster::WhateverAppears => {
                if mine.is_empty() {
                    // No roster, and nothing has ever appeared. We cannot name a
                    // missing instrument, but we can say the series as a whole has
                    // produced nothing — which is the live state of every forward
                    // series registered today.
                    findings.push(Finding {
                        feed: FeedKey::new(&exp.spec.series_id, &exp.spec.venue, WHOLE_SERIES),
                        verdict: judge(exp, None, taken_at),
                    });
                } else {
                    for b in mine {
                        let feed = FeedKey::new(&exp.spec.series_id, &exp.spec.venue, &b.feed.instrument);
                        findings.push(Finding { feed, verdict: judge(exp, Some(b), taken_at) });
                    }
                }
            }
        }
    }

    // A heartbeat nobody declared: something is collecting that no registry knows
    // about. Worth saying, not worth paging.
    for b in beats {
        if !expectations.iter().any(|e| e.owns(&b.feed)) {
            findings.push(Finding {
                feed: b.feed.clone(),
                verdict: Verdict::CannotJudge {
                    reason: format!("no registered series covers {}/{}", b.feed.series, b.feed.venue),
                },
            });
        }
    }

    let shared_cause = infer_shared_cause(&findings);
    SilenceReport { taken_at, findings, shared_cause }
}

/// When enough of what we could judge went silent at once, one cause is more
/// likely than that many independent ones — the writer, the store, the host, the
/// network. Naming it turns N pages into one.
///
/// Only [`Verdict::CollectorSilent`] counts. `NeverSeen` feeds have not gone
/// silent together; they have never started, which is a different investigation.
fn infer_shared_cause(findings: &[Finding]) -> Option<SharedCause> {
    let judgeable = findings
        .iter()
        .filter(|f| matches!(f.verdict, Verdict::Live { .. } | Verdict::CollectorSilent { .. } | Verdict::EventsSilent { .. } | Verdict::QuietAndPolledOnTime { .. }))
        .count();
    let silent = findings.iter().filter(|f| matches!(f.verdict, Verdict::CollectorSilent { .. })).count();

    if judgeable < MIN_FEEDS_FOR_SHARED_CAUSE || silent * 100 < judgeable * SHARED_CAUSE_PCT {
        return None;
    }
    Some(SharedCause {
        silent_feeds: silent,
        judgeable_feeds: judgeable,
        reason: format!(
            "{silent} of {judgeable} judgeable feeds stopped writing. Look for one shared cause \
             (the persister, the store, the host) before {silent} separate ones"
        ),
    })
}

// ── Building expectations from the registry ──────────────────────────────────

/// A declared arrival pattern, as a registry row would spell it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrivalDecl {
    /// `"always"`, `"polled"` or `"unknown"`.
    pub kind: String,
    /// Required by `"polled"`; ignored otherwise.
    pub poll_period_secs: Option<i32>,
}

/// One row of `dataplane.forward_series`, as plain data.
///
/// A caller maps its query straight onto these fields, so there is no judgement
/// call to get wrong at the call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryRow {
    pub series_id: String,
    pub store_table: String,
    pub venue: String,
    pub cadence_secs: i32,
    pub alarm_after_cadences: i32,
    pub disk_budget_bytes: i64,
    /// `"none"` or `"downsample"` — the registry's CHECK admits no others.
    pub retention_policy: String,
    pub downsample_after_days: Option<i32>,
    pub downsample_to_secs: Option<i32>,
    pub enabled: bool,
    /// The arrival pattern.
    ///
    /// `dataplane.forward_series` has **no such column today**, so this is `None`
    /// at every call site until the migration named in the crate docs adds one.
    /// `None` is not a failure: see [`FeedExpectation::from_registry`].
    pub arrival: Option<ArrivalDecl>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExpectationError {
    #[error("{series_id}: cadence_secs must be positive, got {got}")]
    BadCadence { series_id: String, got: i32 },
    #[error("{series_id}: alarm_after_cadences must be at least 1, got {got}")]
    BadDeadline { series_id: String, got: i32 },
    #[error("{series_id}: disk_budget_bytes must be positive, got {got}")]
    BadBudget { series_id: String, got: i64 },
    #[error("{series_id}: retention_policy {got:?} is not one of none|downsample")]
    BadPolicy { series_id: String, got: String },
    #[error("{series_id}: a downsampling policy must state both after_days and to_secs")]
    IncompletePolicy { series_id: String },
    #[error("{series_id}: arrival {got:?} is not one of always|polled|unknown")]
    BadArrival { series_id: String, got: String },
    #[error("{series_id}: arrival 'polled' must state poll_period_secs")]
    PolledWithoutPeriod { series_id: String },
}

impl FeedExpectation {
    /// Build an expectation from a registry row.
    ///
    /// # The default when the registry does not declare an arrival pattern
    ///
    /// `None` becomes [`Arrival::Polled`] at the registry's own `cadence_secs` —
    /// **not** [`Arrival::Always`], and not [`Arrival::Unknown`].
    ///
    /// This is the load-bearing choice in the whole module, so it is worth being
    /// explicit about why each alternative is worse:
    ///
    /// * [`Arrival::Always`] would be a claim about the *venue* — that it never
    ///   closes — which no column in the registry supports. Defaulting to it means
    ///   every session-bound series alarms every weekend, and an alarm that fires
    ///   every weekend gets muted.
    /// * [`Arrival::Unknown`] would be safe and useless: it alarms on nothing, so
    ///   the dead persister that prompted all of this stays invisible.
    /// * [`Arrival::Polled`] at `cadence_secs` is a claim about *us*: we undertook
    ///   to collect this series every `cadence_secs`, and that promise is already
    ///   in the registry, seeded and CHECK-constrained. If we have written nothing
    ///   in `cadence_secs * alarm_after_cadences`, our collector has stopped. No
    ///   market closure, holiday, or maintenance window can explain that, so the
    ///   alarm needs no calendar and cannot fire on a weekend.
    ///
    /// The consequence is that the silence alarm works today, with no migration,
    /// on every series already registered. Declaring `"always"` later is a strict
    /// upgrade that *adds* the event-silence dimension; it is never required to
    /// catch a dead collector.
    ///
    /// # Errors
    /// A row that violates the registry's own CHECK constraints.
    pub fn from_registry(row: &RegistryRow, roster: Roster) -> Result<Self, ExpectationError> {
        let id = || row.series_id.clone();
        let cadence_secs = u32::try_from(row.cadence_secs)
            .ok()
            .filter(|c| *c > 0)
            .ok_or_else(|| ExpectationError::BadCadence { series_id: id(), got: row.cadence_secs })?;
        let alarm_after_cadences = u32::try_from(row.alarm_after_cadences)
            .ok()
            .filter(|c| *c >= 1)
            .ok_or_else(|| ExpectationError::BadDeadline { series_id: id(), got: row.alarm_after_cadences })?;
        let disk_budget_bytes = u64::try_from(row.disk_budget_bytes)
            .ok()
            .filter(|b| *b > 0)
            .ok_or_else(|| ExpectationError::BadBudget { series_id: id(), got: row.disk_budget_bytes })?;

        let retention = match row.retention_policy.as_str() {
            "none" => retention::RetentionPolicy::None,
            "downsample" => {
                let (Some(after_days), Some(to_secs)) = (row.downsample_after_days, row.downsample_to_secs) else {
                    return Err(ExpectationError::IncompletePolicy { series_id: id() });
                };
                let after_days = u32::try_from(after_days)
                    .map_err(|_| ExpectationError::IncompletePolicy { series_id: id() })?;
                let to_secs =
                    u32::try_from(to_secs).map_err(|_| ExpectationError::IncompletePolicy { series_id: id() })?;
                retention::RetentionPolicy::Downsample { after_days, to_secs }
            }
            other => return Err(ExpectationError::BadPolicy { series_id: id(), got: other.to_string() }),
        };

        let arrival = match &row.arrival {
            // The registry cannot say. Fall back to what it *can* say: our own
            // collection cadence. See the doc comment above.
            None => Arrival::Polled { poll_period_secs: cadence_secs },
            Some(d) => match d.kind.as_str() {
                "always" => Arrival::Always,
                "unknown" => Arrival::Unknown,
                "polled" => {
                    let p = d
                        .poll_period_secs
                        .and_then(|p| u32::try_from(p).ok())
                        .filter(|p| *p > 0)
                        .ok_or_else(|| ExpectationError::PolledWithoutPeriod { series_id: id() })?;
                    Arrival::Polled { poll_period_secs: p }
                }
                other => return Err(ExpectationError::BadArrival { series_id: id(), got: other.to_string() }),
            },
        };

        Ok(Self {
            spec: ForwardSeriesSpec {
                series_id: row.series_id.clone(),
                store_table: row.store_table.clone(),
                venue: row.venue.clone(),
                cadence_secs,
                alarm_after_cadences,
                disk_budget_bytes,
                retention,
                enabled: row.enabled,
            },
            arrival,
            roster,
        })
    }
}

// ── Emission ─────────────────────────────────────────────────────────────────

/// A clean pass, in one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SilenceSummary {
    pub feeds: usize,
    pub live: usize,
    pub quiet: usize,
    pub alarming: usize,
    pub unobserved: usize,
    pub unjudgeable: usize,
    pub disabled: usize,
}

impl From<&SilenceReport> for SilenceSummary {
    fn from(r: &SilenceReport) -> Self {
        let count = |f: fn(&Verdict) -> bool| r.findings.iter().filter(|x| f(&x.verdict)).count();
        Self {
            feeds: r.findings.len(),
            live: count(|v| matches!(v, Verdict::Live { .. })),
            quiet: count(|v| matches!(v, Verdict::QuietAndPolledOnTime { .. })),
            alarming: count(Verdict::is_alarming),
            unobserved: count(|v| matches!(v, Verdict::Unobserved { .. })),
            unjudgeable: count(|v| matches!(v, Verdict::CannotJudge { .. } | Verdict::ClockFault { .. })),
            disabled: count(|v| matches!(v, Verdict::Disabled)),
        }
    }
}

/// Log a report, so that a caller is one line rather than a match arm per verdict.
///
/// Deliberately mirrors the shape `apps/platform/src/health_jobs.rs` already uses:
/// alarms at `error`, degraded-but-explicable at `warn`, and signals that could
/// not be computed logged *separately from both* — a check that is broken is a
/// different problem from a check that failed, and treating them alike is how the
/// broken one stays broken.
///
/// A report with no findings logs a warning rather than nothing. "We evaluated
/// zero feeds" arriving as silence is the failure this module is about, and it
/// would be absurd for the silence alarm to have it.
pub fn emit(report: &SilenceReport) {
    if report.findings.is_empty() {
        tracing::warn!(
            taken_at = %report.taken_at,
            "silence alarm evaluated no feeds at all — nothing was asked, so nothing is known"
        );
        return;
    }

    for page in report.pages() {
        match page {
            Page::SharedCause { cause, feeds } => {
                tracing::error!(
                    silent = cause.silent_feeds,
                    judgeable = cause.judgeable_feeds,
                    feeds = ?feeds.iter().map(|f| f.instrument.as_str()).collect::<Vec<_>>(),
                    detail = %cause.reason,
                    "P1: feeds stopped writing together"
                );
            }
            Page::Feed(f) => {
                tracing::error!(
                    series = %f.feed.series,
                    venue = %f.feed.venue,
                    instrument = %f.feed.instrument,
                    verdict = ?f.verdict,
                    "P1: a feed that should be arriving is not"
                );
            }
        }
    }

    for f in report.with_severity(Severity::Warn) {
        tracing::warn!(
            series = %f.feed.series,
            instrument = %f.feed.instrument,
            verdict = ?f.verdict,
            "silence alarm: this feed could not be judged"
        );
    }

    for f in report.with_severity(Severity::Info) {
        tracing::info!(
            series = %f.feed.series,
            instrument = %f.feed.instrument,
            verdict = ?f.verdict,
            "silence alarm: quiet, and not an error"
        );
    }

    if report.all_clear() {
        tracing::info!(summary = ?SilenceSummary::from(report), "silence alarm clean");
    }
}

/// Time until a healthy feed would trip the write deadline. `None` once it has,
/// or when no deadline can be defended.
#[must_use]
pub fn time_to_alarm(exp: &FeedExpectation, beat: &Heartbeat, now: DateTime<Utc>) -> Option<Duration> {
    let deadline = exp.write_deadline_secs()?;
    let age = (now - beat.last_write_time).num_seconds();
    if age > deadline {
        return None;
    }
    Some(Duration::seconds(deadline - age))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use dataplane::retention::RetentionPolicy;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 0, 0, 0).unwrap() + Duration::seconds(secs)
    }

    fn hours(h: i64) -> DateTime<Utc> {
        t(h * 3600)
    }

    fn spec(id: &str, table: &str, venue: &str) -> ForwardSeriesSpec {
        ForwardSeriesSpec {
            series_id: id.into(),
            store_table: table.into(),
            venue: venue.into(),
            cadence_secs: 3600,
            alarm_after_cadences: 3,
            disk_budget_bytes: 2_147_483_648,
            retention: RetentionPolicy::None,
            enabled: true,
        }
    }

    /// Kraken perpetual funding: hourly, every hour, forever.
    fn funding() -> FeedExpectation {
        FeedExpectation {
            spec: spec("kraken_futures.funding", "funding_rates", "kraken_futures"),
            arrival: Arrival::Always,
            roster: Roster::Declared { instruments: vec!["PF_XBTUSD".into()] },
        }
    }

    /// An equity option chain: polled hourly, but the venue is shut most of the week.
    fn chains() -> FeedExpectation {
        FeedExpectation {
            spec: spec("equity.option_chains", "option_chain_snapshots", "tradier"),
            arrival: Arrival::Polled { poll_period_secs: 3600 },
            roster: Roster::Declared { instruments: vec!["SPY".into()] },
        }
    }

    fn beat(exp: &FeedExpectation, inst: &str, event: DateTime<Utc>, write: DateTime<Utc>, rows: u64) -> Heartbeat {
        Heartbeat {
            feed: FeedKey::new(&exp.spec.store_table, &exp.spec.venue, inst),
            last_event_time: event,
            last_write_time: write,
            rows_written: rows,
        }
    }

    // ── The distinction the whole module exists for ──────────────────────────

    /// A closed market and a dead collector produce the same event age. Only the
    /// write clock tells them apart, and it must, because one is a weekend and the
    /// other is data loss.
    #[test]
    fn a_closed_market_and_a_dead_collector_are_not_the_same_verdict() {
        let exp = chains();
        let now = hours(60); // the venue shut 60 hours ago; it is Sunday.

        // The venue gave us nothing for 60 hours, but we polled ten minutes ago.
        let quiet = beat(&exp, "SPY", hours(0), now - Duration::minutes(10), 0);
        let v = judge(&exp, Some(&quiet), now);
        assert!(matches!(v, Verdict::QuietAndPolledOnTime { .. }), "a closed venue must not page: {v:?}");
        assert_eq!(v.severity(), Severity::Info);
        assert!(!v.is_alarming(), "an alarm that fires every weekend gets muted");

        // Same event age, but we stopped polling too.
        let dead = beat(&exp, "SPY", hours(0), hours(0), 0);
        let v = judge(&exp, Some(&dead), now);
        assert!(matches!(v, Verdict::CollectorSilent { .. }), "a collector that stopped writing must page: {v:?}");
        assert_eq!(v.severity(), Severity::Alarm);
    }

    /// And the quiet one is not reported as healthy either. A quiet feed and a
    /// working feed are different facts.
    #[test]
    fn quiet_is_not_health() {
        let exp = chains();
        let now = hours(60);
        let quiet = beat(&exp, "SPY", hours(0), now - Duration::minutes(10), 0);
        let v = judge(&exp, Some(&quiet), now);
        assert_ne!(v.severity(), Severity::Ok, "only Live is Ok");
        assert!(!matches!(v, Verdict::Live { .. }));
    }

    /// On a series the venue publishes every hour of every day, an absence of
    /// events has no innocent explanation and is alarmed — with the cost stated.
    #[test]
    fn a_continuously_published_series_alarms_on_absent_events() {
        let exp = funding();
        let now = hours(48);
        // Writing every minute (the poll is alive) but the venue's last settlement
        // was 48 hours ago.
        let b = beat(&exp, "PF_XBTUSD", hours(0), now - Duration::minutes(1), 1);
        match judge(&exp, Some(&b), now) {
            Verdict::EventsSilent { event_age_secs, rows_missed } => {
                assert_eq!(event_age_secs, 48 * 3600);
                assert_eq!(rows_missed, 47, "47 hourly settlements no backfill can recover");
            }
            other => panic!("expected EventsSilent, got {other:?}"),
        }
    }

    /// The same absence on a polled series is never alarmed, because we hold no
    /// calendar that could justify it. This module refuses to invent one.
    #[test]
    fn the_same_absence_on_a_polled_series_is_never_alarmed() {
        let now = hours(48);
        let always = funding();
        let polled = chains();
        let alive_write = now - Duration::minutes(1);

        let a = judge(&always, Some(&beat(&always, "PF_XBTUSD", hours(0), alive_write, 1)), now);
        let p = judge(&polled, Some(&beat(&polled, "SPY", hours(0), alive_write, 0)), now);

        assert!(a.is_alarming());
        assert!(!p.is_alarming());
        // Identical event ages, opposite verdicts. The difference is a declared
        // arrival pattern, not a guess about market hours.
    }

    // ── Absent measurements ──────────────────────────────────────────────────

    /// The rule, at the level of the report: a feed with no rows and a feed nobody
    /// asked about must not produce the same answer.
    #[test]
    fn nothing_measured_is_not_all_clear() {
        let empty = SilenceReport { taken_at: t(0), findings: vec![], shared_cause: None };
        assert!(!empty.all_clear(), "evaluating zero feeds is not a clean bill of health");

        let exp = funding();
        let now = hours(1);
        let live = evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", now, now, 1)] },
        );
        assert!(live.all_clear());
    }

    /// A census that could not be taken is blindness, not health — and not the
    /// collector's fault either.
    #[test]
    fn a_failed_census_is_unobserved_not_never_seen_and_not_ok() {
        let exp = funding();
        let r = evaluate(
            std::slice::from_ref(&exp),
            &Census::Failed { attempted_at: t(0), reason: "clickhouse: connection refused".into() },
        );
        assert_eq!(r.findings.len(), 1);
        match &r.findings[0].verdict {
            Verdict::Unobserved { reason } => assert!(reason.contains("connection refused")),
            other => panic!("a failed read must not be a collector verdict: {other:?}"),
        }
        assert!(!r.all_clear());
        assert_eq!(r.worst(), Some(Severity::Warn), "we cannot accuse a collector we never looked at");
        assert!(r.pages().is_empty(), "blindness does not page a feed owner");
    }

    /// `from_read` is the spelling that makes the right thing automatic: an error
    /// cannot arrive as an empty success.
    #[test]
    fn a_reader_error_cannot_become_an_empty_success() {
        let err: Result<Vec<Heartbeat>, String> = Err("timeout".into());
        assert!(matches!(Census::from_read(t(0), err), Census::Failed { .. }));
        let ok: Result<Vec<Heartbeat>, String> = Ok(vec![]);
        assert!(matches!(Census::from_read(t(0), ok), Census::Taken { .. }));
    }

    /// And if a caller lies anyway — passing an empty `Taken` on error — the
    /// failure mode is loud, not silent. That direction is the design.
    #[test]
    fn even_a_lying_caller_cannot_manufacture_an_all_clear() {
        let exp = funding();
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: t(0), beats: vec![] });
        assert!(!r.all_clear());
        assert_eq!(r.findings[0].verdict, Verdict::NeverSeen);
        assert!(!r.pages().is_empty());
    }

    /// A collector that never started and one that died need different fixes.
    #[test]
    fn never_seen_is_not_collector_silent() {
        let exp = funding();
        let now = hours(100);
        assert_eq!(judge(&exp, None, now), Verdict::NeverSeen);
        let b = beat(&exp, "PF_XBTUSD", hours(0), hours(0), 1);
        assert!(matches!(judge(&exp, Some(&b), now), Verdict::CollectorSilent { .. }));
        assert_ne!(judge(&exp, None, now), judge(&exp, Some(&b), now));
    }

    // ── The per-instrument grain ─────────────────────────────────────────────

    /// The live failure this module was written for: the series produced rows, so
    /// a series-grained alarm reads healthy, while most of its instruments are
    /// dead. `dataplane.forward_series_alarm` is keyed on `series_id` alone and
    /// cannot express this.
    #[test]
    fn one_live_instrument_does_not_certify_a_series_whose_others_are_dead() {
        let exp = FeedExpectation {
            roster: Roster::Declared {
                instruments: vec!["PF_XBTUSD".into(), "PF_ETHUSD".into(), "PF_SOLUSD".into(), "PF_XRPUSD".into()],
            },
            ..funding()
        };
        let now = hours(200);
        let beats = vec![
            beat(&exp, "PF_XBTUSD", now, now, 1),
            beat(&exp, "PF_ETHUSD", hours(0), hours(0), 1),
            beat(&exp, "PF_SOLUSD", hours(0), hours(0), 1),
            beat(&exp, "PF_XRPUSD", hours(0), hours(0), 1),
        ];
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: now, beats });
        assert!(!r.all_clear());
        assert_eq!(r.with_severity(Severity::Alarm).len(), 3);
        assert_eq!(r.with_severity(Severity::Ok).len(), 1);
    }

    /// Without a roster we cannot name a missing instrument — and we say so rather
    /// than reporting the ones we have as the whole truth.
    #[test]
    fn without_a_roster_only_what_appeared_can_be_judged() {
        let exp = FeedExpectation { roster: Roster::WhateverAppears, ..funding() };
        let now = hours(200);
        let r = evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", now, now, 1)] },
        );
        // The one we saw is judged. The 278 we did not are not invented, and not
        // certified either — they are simply absent from the report.
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].feed.instrument, "PF_XBTUSD");
    }

    /// But a series that has produced nothing at all is still caught, roster or no.
    #[test]
    fn a_series_that_produced_nothing_is_caught_without_a_roster() {
        let exp = FeedExpectation { roster: Roster::WhateverAppears, ..funding() };
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: hours(200), beats: vec![] });
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].feed.instrument, WHOLE_SERIES);
        assert_eq!(r.findings[0].verdict, Verdict::NeverSeen);
    }

    // ── The join that silently matches nothing ───────────────────────────────

    /// The heartbeat writer records `series` = the store table; the registry calls
    /// it `kraken_futures.funding`. A census that matches nothing looks exactly
    /// like a census where nothing is wrong, so both spellings are accepted.
    #[test]
    fn the_store_table_and_the_registry_id_both_match() {
        let exp = funding();
        assert!(exp.owns(&FeedKey::new("funding_rates", "kraken_futures", "PF_XBTUSD")));
        assert!(exp.owns(&FeedKey::new("kraken_futures.funding", "kraken_futures", "PF_XBTUSD")));
        assert!(!exp.owns(&FeedKey::new("funding_rates", "binance", "PF_XBTUSD")));
    }

    /// Store tables are not unique — both option-chain series land in
    /// `option_chain_snapshots` — so the venue must disambiguate them.
    #[test]
    fn two_series_sharing_a_store_table_are_told_apart_by_venue() {
        let equity = chains();
        let crypto = FeedExpectation {
            spec: spec("crypto.option_chains", "option_chain_snapshots", "deribit"),
            ..chains()
        };
        let k = FeedKey::new("option_chain_snapshots", "tradier", "SPY");
        assert!(equity.owns(&k));
        assert!(!crypto.owns(&k), "a shared store table must not make two series one");
    }

    // ── Clock faults and alibis ──────────────────────────────────────────────

    /// A write stamped in the future is a fault, not freshness. Rounded off, it is
    /// an alibi: negative age reads as healthy under any threshold.
    #[test]
    fn a_write_from_the_future_is_a_fault_not_freshness() {
        let exp = funding();
        let now = hours(10);
        let b = beat(&exp, "PF_XBTUSD", now, now + Duration::hours(5), 1);
        let v = judge(&exp, Some(&b), now);
        assert!(matches!(v, Verdict::ClockFault { .. }), "got {v:?}");
        assert_ne!(v.severity(), Severity::Ok);
        // Ordinary skew is tolerated; we are not paging on NTP jitter.
        let b = beat(&exp, "PF_XBTUSD", now, now + Duration::seconds(30), 1);
        assert!(matches!(judge(&exp, Some(&b), now), Verdict::Live { .. }));
    }

    /// Adversarial, and a live hazard: `funding_rates` stores `kind = 'predicted'`
    /// rows stamped with a settlement instant in the *future*. A single-clock
    /// alarm reading `max(event_time)` sees eternal freshness. The write clock is
    /// not fooled, and a dead collector still pages.
    #[test]
    fn a_future_event_time_cannot_give_a_dead_collector_an_alibi() {
        let exp = funding();
        let now = hours(100);
        // The last thing written was a prediction for a settlement far ahead — and
        // it was written 100 hours ago.
        let b = beat(&exp, "PF_XBTUSD", now + Duration::hours(8), hours(0), 1);
        let v = judge(&exp, Some(&b), now);
        assert!(matches!(v, Verdict::CollectorSilent { .. }), "a future event time must not confer health: {v:?}");

        // And the single-clock function it replaces *is* fooled — which is why the
        // second clock had to exist.
        let single = retention::alarm(&exp.spec, Some(b.last_event_time), now);
        assert_eq!(single.state, AlarmState::Ok, "this is the failure mode, pinned so it cannot be forgotten");
    }

    /// The other live hazard, from the store as it stands today: a planted
    /// synthetic fixture whose event time is in 2020 and whose write time is
    /// recent. A single-clock alarm pages that it is six years overdue; that page
    /// is how the alarm gets muted.
    #[test]
    fn a_planted_fixture_does_not_page_for_being_six_years_old() {
        let exp = FeedExpectation {
            spec: spec("synthetic.planted", "market_bars_v2", "synthetic"),
            arrival: Arrival::Polled { poll_period_secs: 60 },
            roster: Roster::Declared { instruments: vec!["SYN-PLANTED-AR1-424242".into()] },
        };
        let now = hours(0);
        let b = beat(&exp, "SYN-PLANTED-AR1-424242", now - Duration::days(2270), now - Duration::seconds(30), 5000);
        let v = judge(&exp, Some(&b), now);
        assert!(!v.is_alarming(), "a six-year-old event on a fresh write is a fixture, not an outage: {v:?}");
        assert!(matches!(v, Verdict::QuietAndPolledOnTime { .. }));
    }

    // ── Muting pressure ──────────────────────────────────────────────────────

    /// Feeds that stop together are one incident. Nothing is hidden — every
    /// finding is still in the report — but the page count is one.
    #[test]
    fn feeds_that_stop_together_page_once_not_once_each() {
        let exp = FeedExpectation {
            roster: Roster::Declared {
                instruments: (0..40).map(|i| format!("PF_{i:03}USD")).collect(),
            },
            ..funding()
        };
        let now = hours(300);
        let beats: Vec<Heartbeat> =
            (0..40).map(|i| beat(&exp, &format!("PF_{i:03}USD"), hours(0), hours(0), 1)).collect();
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: now, beats });

        assert_eq!(r.findings.len(), 40, "every feed is still reported");
        assert_eq!(r.with_severity(Severity::Alarm).len(), 40);
        let pages = r.pages();
        assert_eq!(pages.len(), 1, "40 simultaneous pages is a request to be muted");
        match &pages[0] {
            Page::SharedCause { cause, feeds } => {
                assert_eq!(cause.silent_feeds, 40);
                assert_eq!(feeds.len(), 40);
            }
            other @ Page::Feed(_) => panic!("expected a shared-cause page: {other:?}"),
        }
    }

    /// One dead feed among many live ones is not a shared cause, and pages on its
    /// own. The rollup must not swallow a single real failure.
    #[test]
    fn a_single_dead_feed_among_live_ones_still_pages_individually() {
        let exp = FeedExpectation {
            roster: Roster::Declared {
                instruments: (0..10).map(|i| format!("PF_{i:03}USD")).collect(),
            },
            ..funding()
        };
        let now = hours(300);
        let beats: Vec<Heartbeat> = (0..10)
            .map(|i| {
                let stamp = if i == 0 { hours(0) } else { now };
                beat(&exp, &format!("PF_{i:03}USD"), stamp, stamp, 1)
            })
            .collect();
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: now, beats });
        assert!(r.shared_cause.is_none());
        let pages = r.pages();
        assert_eq!(pages.len(), 1);
        assert!(matches!(pages[0], Page::Feed(_)));
    }

    /// Two feeds is a coincidence, not a pattern. The rollup needs a floor or it
    /// starts explaining away genuinely independent failures.
    #[test]
    fn two_feeds_is_not_evidence_of_a_shared_cause() {
        let exp = FeedExpectation {
            roster: Roster::Declared { instruments: vec!["A".into(), "B".into()] },
            ..funding()
        };
        let now = hours(300);
        let beats = vec![beat(&exp, "A", hours(0), hours(0), 1), beat(&exp, "B", hours(0), hours(0), 1)];
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: now, beats });
        assert!(r.shared_cause.is_none());
        assert_eq!(r.pages().len(), 2);
    }

    /// The reason merging is a function and not `Vec::extend`.
    ///
    /// One process writes several stores. When it dies, each store's own report
    /// sees two silent feeds — under the floor — and calls it two independent
    /// failures. The union sees four and calls it one incident. Pasting the
    /// findings together without re-running the inference pages four times for
    /// one process death, which is how an alarm gets muted.
    #[test]
    fn a_cause_shared_across_two_stores_is_only_visible_in_the_union() {
        let exp = FeedExpectation {
            roster: Roster::Declared { instruments: vec!["A".into(), "B".into()] },
            ..funding()
        };
        let other = FeedExpectation { roster: exp.roster.clone(), ..chains() };
        let now = hours(300);
        let dead = |e: &FeedExpectation| {
            let beats = ["A", "B"].iter().map(|n| beat(e, n, hours(0), hours(0), 1)).collect();
            evaluate(std::slice::from_ref(e), &Census::Taken { at: now, beats })
        };
        let one = dead(&exp);
        let two = dead(&other);
        assert!(one.shared_cause.is_none(), "two feeds is a coincidence");
        assert!(two.shared_cause.is_none());
        assert_eq!(one.pages().len() + two.pages().len(), 4, "separately, four pages");

        let m = SilenceReport::merged(now, vec![one, two]);
        assert_eq!(m.findings.len(), 4, "nothing is hidden by merging");
        assert!(m.shared_cause.is_some(), "four of four judgeable feeds silent is one cause");
        assert_eq!(m.pages().len(), 1, "one incident, one page");
        assert_eq!(m.taken_at, now);
    }

    /// Merging nothing is not a clean bill of health. A pass that asked no store
    /// anything learned nothing, and the report has to say so rather than come
    /// back empty and green.
    #[test]
    fn a_merge_of_nothing_is_not_all_clear() {
        let m = SilenceReport::merged(hours(1), Vec::new());
        assert!(m.findings.is_empty());
        assert!(!m.all_clear(), "zero feeds evaluated is not zero feeds broken");
        assert_eq!(m.worst(), None);
        assert!(m.pages().is_empty());
    }

    /// A merge must not launder a blind read into health either: a store that
    /// could not be asked keeps its `Unobserved` findings alongside a second
    /// store's clean ones, and the merged report is not all-clear.
    #[test]
    fn a_blind_store_survives_the_merge_next_to_a_healthy_one() {
        let now = hours(300);
        let blind = evaluate(
            std::slice::from_ref(&funding()),
            &Census::Failed { attempted_at: now, reason: "connection refused".into() },
        );
        let exp = chains();
        let ok = evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "SPY", now, now, 1)] },
        );
        assert!(ok.all_clear());

        let m = SilenceReport::merged(now, vec![blind, ok]);
        assert!(!m.all_clear(), "one store unread is not every store healthy");
        assert_eq!(m.worst(), Some(Severity::Warn));
        assert_eq!(m.with_severity(Severity::Warn).len(), 1);
    }

    /// A deliberately disabled series is not health. It is a recorded decision to
    /// accept a permanent hole and must look like one.
    #[test]
    fn a_disabled_series_is_not_alarming_and_not_healthy_either() {
        let exp = FeedExpectation { spec: ForwardSeriesSpec { enabled: false, ..funding().spec }, ..funding() };
        let v = judge(&exp, None, hours(1000));
        assert_eq!(v, Verdict::Disabled);
        assert!(!v.is_alarming());
        assert_ne!(v.severity(), Severity::Ok, "an accepted hole is not a clean bill of health");
        let r = evaluate(std::slice::from_ref(&exp), &Census::Taken { at: hours(1000), beats: vec![] });
        assert!(!r.all_clear());
    }

    /// An undeclared arrival pattern alarms on nothing, and says why.
    #[test]
    fn an_undeclared_cadence_alarms_on_nothing_and_says_so() {
        let exp = FeedExpectation { arrival: Arrival::Unknown, ..funding() };
        let v = judge(&exp, None, hours(1000));
        match &v {
            Verdict::CannotJudge { reason } => assert!(reason.contains("unknown arrival")),
            other => panic!("expected CannotJudge, got {other:?}"),
        }
        assert!(!v.is_alarming());
        assert_eq!(exp.write_deadline_secs(), None);
    }

    /// A collector writing for a series no registry knows about is reported, not
    /// paged, and certainly not ignored.
    #[test]
    fn an_undeclared_feed_is_reported_rather_than_dropped() {
        let exp = funding();
        let now = hours(1);
        let stray = Heartbeat {
            feed: FeedKey::new("open_interest", "binance", "BTCUSDT"),
            last_event_time: now,
            last_write_time: now,
            rows_written: 1,
        };
        let r = evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", now, now, 1), stray] },
        );
        assert_eq!(r.findings.len(), 2);
        assert!(r.findings.iter().any(|f| matches!(f.verdict, Verdict::CannotJudge { .. })));
        assert!(!r.all_clear(), "an unexplained collector is not a clean report");
    }

    // ── Boundaries and delegation ────────────────────────────────────────────

    /// The write deadline is three cadences, and it trips one second past it —
    /// not a second before.
    #[test]
    fn the_write_deadline_is_exact() {
        let exp = funding();
        let last = hours(0);
        for h in 0..=3 {
            let b = beat(&exp, "PF_XBTUSD", hours(h), last, 1);
            let v = judge(&exp, Some(&b), hours(h));
            assert!(!matches!(v, Verdict::CollectorSilent { .. }), "hour {h} is inside the deadline: {v:?}");
        }
        let b = beat(&exp, "PF_XBTUSD", hours(3), last, 1);
        let v = judge(&exp, Some(&b), hours(3) + Duration::seconds(1));
        match v {
            Verdict::CollectorSilent { write_age_secs, deadline_secs } => {
                assert_eq!(deadline_secs, 3 * 3600);
                assert_eq!(write_age_secs, 3 * 3600 + 1);
            }
            other => panic!("expected CollectorSilent, got {other:?}"),
        }
    }

    /// The venue-clock deadline is `dataplane::retention`'s, called rather than
    /// copied. Two silence alarms with two rules would drift, and the one nobody
    /// watched would be the one that was right.
    #[test]
    fn the_event_deadline_is_delegated_and_flips_at_the_same_instant() {
        let exp = funding();
        let alive = |now: DateTime<Utc>| now - Duration::seconds(1);
        for h in 0..8 {
            for extra in [-1i64, 0, 1] {
                let now = hours(h) + Duration::seconds(extra);
                let b = beat(&exp, "PF_XBTUSD", hours(0), alive(now), 1);
                let ours = matches!(judge(&exp, Some(&b), now), Verdict::EventsSilent { .. });
                let theirs = retention::alarm(&exp.spec, Some(hours(0)), now).state == AlarmState::Stale;
                assert_eq!(ours, theirs, "the two disagree at {now}");
            }
        }
    }

    /// The write clock is checked before the venue clock: when both are late the
    /// verdict names the one with an unambiguous fix.
    #[test]
    fn a_dead_collector_is_diagnosed_before_a_quiet_venue() {
        let exp = funding();
        let now = hours(100);
        let b = beat(&exp, "PF_XBTUSD", hours(0), hours(0), 1);
        assert!(matches!(judge(&exp, Some(&b), now), Verdict::CollectorSilent { .. }));
    }

    #[test]
    fn headroom_is_reported_while_healthy() {
        let exp = funding();
        let b = beat(&exp, "PF_XBTUSD", hours(0), hours(0), 1);
        assert_eq!(time_to_alarm(&exp, &b, hours(1)).unwrap().num_hours(), 2);
        assert!(time_to_alarm(&exp, &b, hours(10)).is_none());
        let unknown = FeedExpectation { arrival: Arrival::Unknown, ..funding() };
        assert!(time_to_alarm(&unknown, &b, hours(1)).is_none());
    }

    // ── Persistence ──────────────────────────────────────────────────────────

    /// The one that matters: nothing but `Live` may be written as `'ok'`.
    /// `dataplane.forward_series_alarm`'s CHECK admits three states, and its
    /// `alarm_raised_iff_not_ok` constraint actively pushes a caller towards
    /// recording an unrepresentable verdict as `'ok'` — which would be the
    /// platform certifying health it never measured.
    #[test]
    fn no_verdict_but_live_may_be_recorded_as_ok() {
        let all = [
            Verdict::Live { write_age_secs: 0, event_age_secs: 0 },
            Verdict::CollectorSilent { write_age_secs: 1, deadline_secs: 0 },
            Verdict::EventsSilent { event_age_secs: 1, rows_missed: 1 },
            Verdict::QuietAndPolledOnTime { event_age_secs: 1, write_age_secs: 0 },
            Verdict::NeverSeen,
            Verdict::Unobserved { reason: String::new() },
            Verdict::CannotJudge { reason: String::new() },
            Verdict::Disabled,
            Verdict::ClockFault { detail: String::new() },
        ];
        for v in &all {
            if v.pg_alarm_state() == Some("ok") {
                assert!(matches!(v, Verdict::Live { .. }), "{v:?} must not be recorded as ok");
            }
        }
        // And blindness has no row at all rather than a reassuring one.
        assert_eq!(Verdict::Unobserved { reason: String::new() }.pg_alarm_state(), None);
        assert_eq!(Verdict::ClockFault { detail: String::new() }.pg_alarm_state(), None);
        assert_eq!(Verdict::Disabled.pg_alarm_state(), None);
    }

    /// Only `Live` is `Severity::Ok`. If a variant is ever added, it lands
    /// somewhere louder by default rather than quietly joining the healthy set.
    #[test]
    fn exactly_one_verdict_is_healthy() {
        let all = [
            Verdict::Live { write_age_secs: 0, event_age_secs: 0 },
            Verdict::CollectorSilent { write_age_secs: 1, deadline_secs: 0 },
            Verdict::EventsSilent { event_age_secs: 1, rows_missed: 1 },
            Verdict::QuietAndPolledOnTime { event_age_secs: 1, write_age_secs: 0 },
            Verdict::NeverSeen,
            Verdict::Unobserved { reason: String::new() },
            Verdict::CannotJudge { reason: String::new() },
            Verdict::Disabled,
            Verdict::ClockFault { detail: String::new() },
        ];
        assert_eq!(all.iter().filter(|v| v.severity() == Severity::Ok).count(), 1);
    }

    // ── The trap this crate must not walk into ───────────────────────────────

    /// A component that measures staleness *from* the store it is measuring cannot
    /// tell a dead feed from its own outage: it reports every series silent and
    /// means nothing by it. This module is handed its evidence and holds no
    /// connection of any kind. Enforced here rather than promised in a doc
    /// comment, because a doc comment has never stopped a dependency.
    #[test]
    fn no_db_client_creeps_into_this_crate() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["clickhouse", "sqlx", "reqwest", "tokio-postgres", "redis", "diesel"] {
            assert!(
                !manifest.contains(forbidden),
                "observability must stay free of {forbidden}: a staleness monitor that reads the store \
                 it measures cannot distinguish a dead feed from its own outage"
            );
        }
    }

    // ── The registry mapping ─────────────────────────────────────────────────

    /// The two rows `migration 0098` actually seeded, exactly as they stand.
    fn seeded_rows() -> Vec<RegistryRow> {
        ["kraken_futures.funding", "kraken_futures.open_interest"]
            .into_iter()
            .zip(["funding_rates", "open_interest"])
            .map(|(id, table)| RegistryRow {
                series_id: id.into(),
                store_table: table.into(),
                venue: "kraken_futures".into(),
                cadence_secs: 3600,
                alarm_after_cadences: 3,
                disk_budget_bytes: 2_147_483_648,
                retention_policy: "none".into(),
                downsample_after_days: None,
                downsample_to_secs: None,
                enabled: true,
                // No such column exists in `dataplane.forward_series` today.
                arrival: None,
            })
            .collect()
    }

    /// The load-bearing default. With no arrival column the registry still knows
    /// our own collection cadence, which is a claim about us and not about any
    /// venue — so the alarm works today, on the seeded rows, with no migration.
    #[test]
    fn an_undeclared_arrival_falls_back_to_our_own_cadence_not_to_a_venue_claim() {
        for row in seeded_rows() {
            let exp = FeedExpectation::from_registry(&row, Roster::WhateverAppears).unwrap();
            assert_eq!(exp.arrival, Arrival::Polled { poll_period_secs: 3600 });
            assert_eq!(exp.write_deadline_secs(), Some(3 * 3600), "three cadences, as the registry says");
            assert_ne!(exp.arrival, Arrival::Always, "we hold no evidence this venue never closes");
            assert_ne!(exp.arrival, Arrival::Unknown, "an alarm that alarms on nothing is not safe, it is useless");
        }
    }

    /// And that default catches the thing the whole task is about: a collector
    /// that stopped writing. Without a single migration.
    #[test]
    fn the_default_still_catches_a_dead_persister() {
        let row = seeded_rows().remove(0);
        let exp = FeedExpectation::from_registry(&row, Roster::WhateverAppears).unwrap();
        let now = hours(200);
        let b = Heartbeat {
            feed: FeedKey::new("funding_rates", "kraken_futures", "PF_XBTUSD"),
            last_event_time: hours(0),
            last_write_time: hours(0),
            rows_written: 1,
        };
        assert!(matches!(judge(&exp, Some(&b), now), Verdict::CollectorSilent { .. }));
    }

    /// ...and does not fire on a venue that is merely shut, which is what makes it
    /// survivable.
    #[test]
    fn the_default_does_not_fire_on_a_shut_venue() {
        let row = seeded_rows().remove(0);
        let exp = FeedExpectation::from_registry(&row, Roster::WhateverAppears).unwrap();
        let now = hours(200);
        let b = Heartbeat {
            feed: FeedKey::new("funding_rates", "kraken_futures", "PF_XBTUSD"),
            last_event_time: hours(0),
            last_write_time: now - Duration::minutes(5),
            rows_written: 0,
        };
        assert!(!judge(&exp, Some(&b), now).is_alarming());
    }

    /// Declaring `"always"` is a strict upgrade: it adds the event dimension and
    /// keeps the write dimension.
    #[test]
    fn declaring_always_adds_the_event_dimension() {
        let mut row = seeded_rows().remove(0);
        row.arrival = Some(ArrivalDecl { kind: "always".into(), poll_period_secs: None });
        let exp = FeedExpectation::from_registry(&row, Roster::WhateverAppears).unwrap();
        assert_eq!(exp.arrival, Arrival::Always);
        let now = hours(48);
        let b = Heartbeat {
            feed: FeedKey::new("funding_rates", "kraken_futures", "PF_XBTUSD"),
            last_event_time: hours(0),
            last_write_time: now - Duration::minutes(1),
            rows_written: 1,
        };
        assert!(matches!(judge(&exp, Some(&b), now), Verdict::EventsSilent { .. }));
    }

    #[test]
    fn a_registry_row_that_violates_its_own_checks_is_refused() {
        let base = seeded_rows().remove(0);
        let bad_cadence = RegistryRow { cadence_secs: 0, ..base.clone() };
        assert!(matches!(
            FeedExpectation::from_registry(&bad_cadence, Roster::WhateverAppears),
            Err(ExpectationError::BadCadence { .. })
        ));
        let bad_policy = RegistryRow { retention_policy: "delete".into(), ..base.clone() };
        assert!(
            matches!(
                FeedExpectation::from_registry(&bad_policy, Roster::WhateverAppears),
                Err(ExpectationError::BadPolicy { .. })
            ),
            "there is no delete tier, here or anywhere"
        );
        let incomplete = RegistryRow { retention_policy: "downsample".into(), ..base.clone() };
        assert!(matches!(
            FeedExpectation::from_registry(&incomplete, Roster::WhateverAppears),
            Err(ExpectationError::IncompletePolicy { .. })
        ));
        let bad_arrival = RegistryRow {
            arrival: Some(ArrivalDecl { kind: "sometimes".into(), poll_period_secs: None }),
            ..base.clone()
        };
        assert!(matches!(
            FeedExpectation::from_registry(&bad_arrival, Roster::WhateverAppears),
            Err(ExpectationError::BadArrival { .. })
        ));
        let polled_no_period =
            RegistryRow { arrival: Some(ArrivalDecl { kind: "polled".into(), poll_period_secs: None }), ..base };
        assert!(matches!(
            FeedExpectation::from_registry(&polled_no_period, Roster::WhateverAppears),
            Err(ExpectationError::PolledWithoutPeriod { .. })
        ));
    }

    /// The live state of this platform on 2026-09-22: `forward_series_heartbeat`
    /// holds zero rows and all three forward tables are empty. Wired up today, the
    /// alarm should fire immediately on both seeded series — and it does, as
    /// `NeverSeen` rather than as an all-clear.
    #[test]
    fn the_platform_as_it_stands_today_alarms_rather_than_reporting_health() {
        let exps: Vec<FeedExpectation> = seeded_rows()
            .iter()
            .map(|r| FeedExpectation::from_registry(r, Roster::WhateverAppears).unwrap())
            .collect();
        let r = evaluate(&exps, &Census::Taken { at: hours(1), beats: vec![] });
        assert!(!r.all_clear());
        assert_eq!(r.with_severity(Severity::Alarm).len(), 2);
        assert!(r.findings.iter().all(|f| f.verdict == Verdict::NeverSeen));
        // Two is below the shared-cause floor, so both page on their own.
        assert_eq!(r.pages().len(), 2);
    }

    #[test]
    fn a_summary_counts_every_finding_exactly_once() {
        let exp = funding();
        let now = hours(300);
        let r = evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", hours(0), hours(0), 1)] },
        );
        let s = SilenceSummary::from(&r);
        assert_eq!(s.feeds, 1);
        assert_eq!(s.alarming, 1);
        assert_eq!(s.live, 0);
    }

    /// `emit` must not panic on any report shape, including the empty one it is
    /// specifically meant to complain about.
    #[test]
    fn emit_handles_every_report_shape() {
        emit(&SilenceReport { taken_at: t(0), findings: vec![], shared_cause: None });
        let exp = funding();
        let now = hours(300);
        emit(&evaluate(std::slice::from_ref(&exp), &Census::Failed { attempted_at: now, reason: "down".into() }));
        emit(&evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", now, now, 1)] },
        ));
        emit(&evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", hours(0), hours(0), 1)] },
        ));
    }

    /// Serde round-trips, since a report is meant to cross a route.
    #[test]
    fn a_report_survives_a_round_trip() {
        let exp = funding();
        let now = hours(300);
        let r = evaluate(
            std::slice::from_ref(&exp),
            &Census::Taken { at: now, beats: vec![beat(&exp, "PF_XBTUSD", hours(0), hours(0), 1)] },
        );
        let json = serde_json::to_string(&r).unwrap();
        let back: SilenceReport = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
