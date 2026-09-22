//! The thing that actually runs the funding collector (P-0.3).
//!
//! `KrakenFuturesClient` could collect and `storage::clickhouse::forward` could
//! write, and between them there was nothing: the client was constructed nowhere
//! in the workspace, the writers had no callers, and the four tables in `trading`
//! held zero rows apiece. This module is the loop that joins them.
//!
//! What it does not do is retry. The limiter refuses rather than sleeps
//! ([`FundingError::RateLimited`]), and a refused tick is skipped, counted, and
//! picked up by the next one. Under a free-only data policy there is no second
//! venue to fail over to, so a ban costs more than a missed hour.
//!
//! # Why the source is a trait
//!
//! The same reason [`crate::derivs::sink::ForwardSink`] is: the properties worth
//! proving about a poll loop — it writes what it collected, it writes a heartbeat
//! only when it wrote rows, a refusal does not become an empty write — are
//! counting properties, and you cannot count through a venue you are forbidden to
//! call from a test.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{error, info, warn};

use super::funding::{FundingError, FundingObservation, KrakenFuturesClient, TickerSnapshot, VENUE};
use super::sink::{Beat, ForwardSink, SinkError};

/// The series names the heartbeat table records, matching
/// `dataplane.forward_series`.
pub const FUNDING_SERIES: &str = "funding";
pub const OPEN_INTEREST_SERIES: &str = "open_interest";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("collect: {0}")]
    Collect(#[from] FundingError),
    #[error("write: {0}")]
    Write(#[from] SinkError),
}

/// What one poll did.
///
/// Every field is a count of something that happened, not a status. `skipped`
/// names the instruments whose fields would not parse — they are reported rather
/// than folded into the success, because an instrument silently dropped every hour
/// is a hole that only shows up in a study months later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollReport {
    pub received_at: Option<DateTime<Utc>>,
    pub funding_written: usize,
    pub open_interest_written: usize,
    pub skipped: Vec<String>,
}

/// What a run of the loop did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunReport {
    pub polls: u64,
    pub funding_rows: u64,
    pub open_interest_rows: u64,
    /// Ticks the limiter refused. Not failures — the budget was spent, and the
    /// next tick picks up.
    pub rate_limited: u64,
    /// Ticks that failed for any other reason.
    pub failures: u64,
}

/// Where a poll's observations come from: the venue client in production, a
/// fixture in tests.
#[async_trait::async_trait]
pub trait TickerSource: Send + Sync {
    /// One `/tickers` poll.
    ///
    /// # Errors
    /// Refused by the limiter, an HTTP failure, or a venue error.
    async fn tickers(&self) -> Result<TickerSnapshot, FundingError>;

    /// One instrument's published funding history.
    ///
    /// # Errors
    /// As [`Self::tickers`].
    async fn funding_history(&self, instrument: &str) -> Result<Vec<FundingObservation>, FundingError>;
}

#[async_trait::async_trait]
impl TickerSource for KrakenFuturesClient {
    async fn tickers(&self) -> Result<TickerSnapshot, FundingError> {
        Self::tickers(self).await
    }

    async fn funding_history(&self, instrument: &str) -> Result<Vec<FundingObservation>, FundingError> {
        Self::funding_history(self, instrument).await
    }
}

/// Polls a venue's forward series and writes them.
pub struct ForwardCollector<S, K> {
    source: S,
    sink: K,
    /// When set, only these instruments are written. Empty means every perpetual
    /// the venue lists — which is the useful default, because an hour not
    /// collected for an instrument is permanently missing for it (FM-11).
    only: Option<BTreeSet<String>>,
}

impl<S: TickerSource, K: ForwardSink> ForwardCollector<S, K> {
    #[must_use]
    pub const fn new(source: S, sink: K) -> Self {
        Self { source, sink, only: None }
    }

    /// Restrict collection to named instruments.
    #[must_use]
    pub fn only<I: Into<String>>(mut self, instruments: impl IntoIterator<Item = I>) -> Self {
        let set: BTreeSet<String> = instruments.into_iter().map(Into::into).collect();
        self.only = (!set.is_empty()).then_some(set);
        self
    }

    fn wanted(&self, instrument: &str) -> bool {
        self.only.as_ref().is_none_or(|s| s.contains(instrument))
    }

    /// One poll: collect, write, then beat.
    ///
    /// The heartbeat is written last and only for instruments that produced rows.
    /// A heartbeat written regardless of the outcome is worse than none — it is a
    /// silence alarm that reports a dead collector as healthy.
    ///
    /// # Errors
    /// [`ServiceError::Collect`] if the venue refused or the response would not
    /// parse; [`ServiceError::Write`] if the store did. Neither is reported as a
    /// poll that wrote nothing.
    pub async fn poll_once(&self) -> Result<PollReport, ServiceError> {
        let snap = self.source.tickers().await?;

        let funding: Vec<FundingObservation> = snap.funding.into_iter().filter(|f| self.wanted(&f.instrument)).collect();
        let open_interest: Vec<_> = snap.open_interest.into_iter().filter(|o| self.wanted(&o.instrument)).collect();

        let received_at = funding.first().map(|f| f.ingest_time).or_else(|| open_interest.first().map(|o| o.ingest_time));

        let funding_written = self.sink.funding(&funding).await?;
        let open_interest_written = self.sink.open_interest(&open_interest).await?;

        let mut beats: Vec<Beat> = Vec::new();
        beats.extend(latest_per_instrument(FUNDING_SERIES, funding.iter().map(|f| (f.instrument.as_str(), f.event_time))));
        beats.extend(latest_per_instrument(OPEN_INTEREST_SERIES, open_interest.iter().map(|o| (o.instrument.as_str(), o.event_time))));
        self.sink.heartbeat(&beats).await?;

        Ok(PollReport {
            received_at,
            funding_written,
            open_interest_written,
            skipped: snap.skipped.into_iter().filter(|s| self.wanted(s)).collect(),
        })
    }

    /// Backfill one instrument's published funding history.
    ///
    /// Every row this writes is derived and flagged — the venue publishes about a
    /// rolling year of settlements nobody watched arrive. It is worth running
    /// once, and it is not a substitute for the forward leg: a backfilled row
    /// supplies Gate 15 with nothing (D-P3).
    ///
    /// # Errors
    /// As [`Self::poll_once`].
    pub async fn backfill_funding(&self, instrument: &str) -> Result<usize, ServiceError> {
        let rows = self.source.funding_history(instrument).await?;
        let written = self.sink.funding(&rows).await?;
        if let Some(last) = rows.iter().map(|r| r.event_time).max() {
            self.sink
                .heartbeat(&[Beat { series: FUNDING_SERIES.to_string(), venue: VENUE.to_string(), instrument: instrument.to_string(), last_event: last, rows: written as u64 }])
                .await?;
        }
        Ok(written)
    }

    /// Poll every `every` until `shutdown` resolves.
    ///
    /// A failed tick is logged and counted; the loop continues, because the next
    /// settlement is still worth having. A tick the limiter refused is counted
    /// separately and is *not* retried — retrying into a refusal is how a free
    /// public endpoint becomes a banned one.
    pub async fn run(&self, every: Duration, shutdown: impl std::future::Future<Output = ()> + Send) -> RunReport {
        let mut report = RunReport::default();
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                () = &mut shutdown => break,
                _ = ticks.tick() => match self.poll_once().await {
                    Ok(r) => {
                        report.polls += 1;
                        report.funding_rows += r.funding_written as u64;
                        report.open_interest_rows += r.open_interest_written as u64;
                        if r.skipped.is_empty() {
                            info!(funding = r.funding_written, open_interest = r.open_interest_written, "forward poll written");
                        } else {
                            warn!(funding = r.funding_written, open_interest = r.open_interest_written, skipped = ?r.skipped, "forward poll written, some instruments skipped");
                        }
                    }
                    Err(ServiceError::Collect(FundingError::RateLimited(d))) => {
                        report.rate_limited += 1;
                        warn!(error = %d, "poll refused by the limiter; skipping this tick rather than retrying");
                    }
                    Err(e) => {
                        report.failures += 1;
                        error!(error = %e, "forward poll failed");
                    }
                },
            }
        }
        report
    }
}

/// One beat per instrument, carrying that instrument's newest event time and how
/// many rows it contributed.
fn latest_per_instrument<'a>(series: &str, rows: impl Iterator<Item = (&'a str, DateTime<Utc>)>) -> Vec<Beat> {
    let mut by: std::collections::BTreeMap<&str, (DateTime<Utc>, u64)> = std::collections::BTreeMap::new();
    for (instrument, event) in rows {
        let e = by.entry(instrument).or_insert((event, 0));
        if event > e.0 {
            e.0 = event;
        }
        e.1 += 1;
    }
    by.into_iter()
        .map(|(instrument, (last_event, rows))| Beat { series: series.to_string(), venue: VENUE.to_string(), instrument: instrument.to_string(), last_event, rows })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derivs::funding::{parse_tickers, FundingKind, OpenInterestObservation};
    use chrono::TimeZone;
    use std::sync::Mutex;

    const TICKERS: &str = r#"{
      "result": "success",
      "serverTime": "2026-09-15T05:40:26.212Z",
      "tickers": [
        {"symbol":"PF_XBTUSD","tag":"perpetual","fundingRate":0.5202031591870675,
         "fundingRatePrediction":0.48,"relativeFundingRate":6.707804166667e-06,
         "openInterest":1234.5,"markPrice":64250.5},
        {"symbol":"PF_ETHUSD","tag":"perpetual","fundingRate":0.11,"openInterest":22.0},
        {"symbol":"FI_XBTUSD_260925","tag":"month","markPrice":64500.0,"openInterest":99.0}
      ]
    }"#;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, h, m, 0).unwrap()
    }

    /// A source that hands back a fixture, and counts how often it was asked. No
    /// network: collecting live from a venue in a test is forbidden, and it would
    /// make the counting properties untestable anyway.
    struct FixtureSource {
        body: &'static str,
        received_at: DateTime<Utc>,
        fail_with: Mutex<Option<FundingError>>,
        calls: Mutex<u32>,
    }

    impl FixtureSource {
        fn new() -> Self {
            Self { body: TICKERS, received_at: at(5, 40), fail_with: Mutex::new(None), calls: Mutex::new(0) }
        }
    }

    #[async_trait::async_trait]
    impl TickerSource for FixtureSource {
        async fn tickers(&self) -> Result<TickerSnapshot, FundingError> {
            *self.calls.lock().unwrap() += 1;
            if let Some(e) = self.fail_with.lock().unwrap().take() {
                return Err(e);
            }
            parse_tickers(self.body, self.received_at)
        }

        async fn funding_history(&self, instrument: &str) -> Result<Vec<FundingObservation>, FundingError> {
            let body = r#"{"result":"success","rates":[
                {"timestamp":"2025-09-10T08:00:00Z","fundingRate":0.655791706953},
                {"timestamp":"2025-09-10T09:00:00Z","fundingRate":0.66}
            ]}"#;
            crate::derivs::funding::parse_history(body, instrument, self.received_at)
        }
    }

    #[derive(Default)]
    struct CountingSink {
        funding: Mutex<Vec<FundingObservation>>,
        open_interest: Mutex<Vec<OpenInterestObservation>>,
        beats: Mutex<Vec<Beat>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl ForwardSink for CountingSink {
        async fn funding(&self, rows: &[FundingObservation]) -> Result<usize, SinkError> {
            if self.fail {
                return Err(SinkError::Store("refused".into()));
            }
            self.funding.lock().unwrap().extend_from_slice(rows);
            Ok(rows.len())
        }

        async fn open_interest(&self, rows: &[OpenInterestObservation]) -> Result<usize, SinkError> {
            if self.fail {
                return Err(SinkError::Store("refused".into()));
            }
            self.open_interest.lock().unwrap().extend_from_slice(rows);
            Ok(rows.len())
        }

        async fn heartbeat(&self, beats: &[Beat]) -> Result<usize, SinkError> {
            if self.fail {
                return Err(SinkError::Store("refused".into()));
            }
            self.beats.lock().unwrap().extend_from_slice(beats);
            Ok(beats.len())
        }
    }

    /// The whole point of the module: a poll reaches the store. Before this
    /// existed, the client was constructed nowhere and the writers had no callers.
    #[tokio::test]
    async fn a_poll_reaches_the_sink() {
        let c = ForwardCollector::new(FixtureSource::new(), CountingSink::default());
        let r = c.poll_once().await.unwrap();

        // Two perpetuals: two realized rates plus one prediction, and two open
        // interest samples. The dated future contributes neither.
        assert_eq!(r.funding_written, 3);
        assert_eq!(r.open_interest_written, 2);
        assert_eq!(r.received_at, Some(at(5, 40)));

        let written = c.sink.funding.lock().unwrap();
        assert_eq!(written.len(), 3);
        assert!(written.iter().all(|f| f.source == "kraken_futures_rest"));
        assert!(written.iter().any(|f| f.kind == FundingKind::Predicted));
        assert!(!c.sink.open_interest.lock().unwrap().iter().any(|o| o.instrument.starts_with("FI_")));
    }

    /// The heartbeat is per instrument and carries that instrument's newest event
    /// time, so a silence alarm can tell which instrument went quiet rather than
    /// only that something did.
    #[tokio::test]
    async fn the_heartbeat_is_per_instrument_and_carries_the_newest_event() {
        let c = ForwardCollector::new(FixtureSource::new(), CountingSink::default());
        c.poll_once().await.unwrap();

        let beats = c.sink.beats.lock().unwrap();
        let funding: Vec<_> = beats.iter().filter(|b| b.series == FUNDING_SERIES).collect();
        assert_eq!(funding.len(), 2, "one beat per instrument, not one per row");
        assert!(funding.iter().all(|b| b.venue == VENUE));
        // PF_XBTUSD contributed a realized rate at 05:00 and a prediction for
        // 06:00; the newest of the two is what the alarm measures against.
        let btc = funding.iter().find(|b| b.instrument == "PF_XBTUSD").unwrap();
        assert_eq!(btc.last_event, at(6, 0));
        assert_eq!(btc.rows, 2);

        assert_eq!(beats.iter().filter(|b| b.series == OPEN_INTEREST_SERIES).count(), 2);
    }

    /// A store that refused must not leave a heartbeat behind. A silence alarm
    /// that beats when nothing was written reports a dead collector as healthy,
    /// which is worse than having no alarm.
    #[tokio::test]
    async fn a_refused_write_leaves_no_heartbeat() {
        let c = ForwardCollector::new(FixtureSource::new(), CountingSink { fail: true, ..CountingSink::default() });
        let e = c.poll_once().await.unwrap_err();
        assert!(matches!(e, ServiceError::Write(_)), "{e}");
        assert!(c.sink.beats.lock().unwrap().is_empty(), "nothing was written, so nothing may beat");
    }

    /// A venue error is an error, not a poll that collected nothing. The
    /// difference is what separates "the venue is down" from "funding stopped".
    #[tokio::test]
    async fn a_venue_failure_is_not_a_poll_that_wrote_zero() {
        let src = FixtureSource::new();
        *src.fail_with.lock().unwrap() = Some(FundingError::Http("503".into()));
        let c = ForwardCollector::new(src, CountingSink::default());
        assert!(matches!(c.poll_once().await, Err(ServiceError::Collect(_))));
        assert!(c.sink.funding.lock().unwrap().is_empty());
        assert!(c.sink.beats.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_instrument_filter_narrows_what_is_written() {
        let c = ForwardCollector::new(FixtureSource::new(), CountingSink::default()).only(["PF_ETHUSD"]);
        let r = c.poll_once().await.unwrap();
        assert_eq!(r.funding_written, 1);
        assert_eq!(r.open_interest_written, 1);
        assert!(c.sink.funding.lock().unwrap().iter().all(|f| f.instrument == "PF_ETHUSD"));
    }

    /// The backfill leg writes derived rows, and they stay derived: a year of
    /// settlements nobody watched arrive must not become forward evidence.
    #[tokio::test]
    async fn the_backfill_leg_writes_flagged_rows_only() {
        let c = ForwardCollector::new(FixtureSource::new(), CountingSink::default());
        let n = c.backfill_funding("PF_XBTUSD").await.unwrap();
        assert_eq!(n, 2);
        let rows = c.sink.funding.lock().unwrap();
        assert!(rows.iter().all(|r| r.knowledge.is_derived()), "history rows are derived by construction");
        assert!(rows.iter().all(|r| !r.knowledge.is_forward_evidence()));
        assert!(rows.iter().all(|r| r.source == "kraken_futures_history"), "and they name the history source, not the ticker");
    }

    /// The loop polls, and a refusal is skipped rather than retried into a ban.
    #[tokio::test(start_paused = true)]
    async fn the_loop_polls_on_its_interval_and_stops_when_told() {
        let c = ForwardCollector::new(FixtureSource::new(), CountingSink::default());
        let report = c.run(Duration::from_secs(3600), tokio::time::sleep(Duration::from_secs(3600 * 3 + 1))).await;
        // The first tick fires immediately, then one per hour.
        assert_eq!(report.polls, 4, "polls: {report:?}");
        assert_eq!(report.funding_rows, 12);
        assert_eq!(report.failures, 0);
        assert_eq!(report.rate_limited, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_limiter_refusal_is_counted_apart_from_a_failure() {
        let src = FixtureSource::new();
        *src.fail_with.lock().unwrap() = Some(FundingError::RateLimited(crate::ratelimit::Denied { retry_after: Duration::from_secs(60), available: 0, needed: 1 }));
        let c = ForwardCollector::new(src, CountingSink::default());
        let report = c.run(Duration::from_secs(3600), tokio::time::sleep(Duration::from_secs(3600 + 1))).await;
        assert_eq!(report.rate_limited, 1);
        assert_eq!(report.failures, 0, "a refusal is not a failure: {report:?}");
        assert_eq!(report.polls, 1, "and the next tick still runs");
    }
}
