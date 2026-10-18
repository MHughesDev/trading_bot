//! Resumability, idempotency and rate-limit safety of the `backfill` job kind
//! (Set P P-1.1; DATA-005 §11 S1; DA-11).
//!
//! Every claim here is proved by **counting adapter calls**, not by inspecting
//! the final state. "A restart fetches no page twice" is a statement about how
//! many times the venue was asked; a test that only checks the rows landed would
//! pass just as happily against an implementation that refetched the whole
//! window every time — which is exactly the implementation that gets the
//! platform banned from a free endpoint.
//!
//! The source double records the `[start, end)` of every page it is asked for,
//! so the assertions can be about *coverage*: the recorded intervals must tile
//! the window exactly — no overlap (nothing fetched twice) and no hole (nothing
//! skipped).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use dataplane::backfill::{
    BackfillBar, BackfillKey, BackfillPage, BackfillSource, BackfillState, BackfillStateStore,
    BackfillStatus, BarSink, FetchError, PageRequest, RateLimit, SinkError, SinkReport, StateError,
};
use jobs::kinds::backfill::{BackfillEngine, BackfillManifest, BackfillPolicy, RunOutcome};

// ── doubles ──────────────────────────────────────────────────────────────────

/// What the venue was asked for, in order.
type Calls = Arc<Mutex<Vec<(DateTime<Utc>, DateTime<Utc>)>>>;

/// A source that serves one bar per minute and records every call.
struct CountingSource {
    calls: Calls,
    /// Pages after which to answer `429` instead, by call index.
    rate_limit_at: Vec<usize>,
    /// Call index after which to return a terms refusal.
    terms_at: Option<usize>,
    /// Never advance the cursor (an adapter bug, deliberately reproduced).
    stuck: bool,
}

impl CountingSource {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            rate_limit_at: Vec::new(),
            terms_at: None,
            stuck: false,
        }
    }

    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl BackfillSource for CountingSource {
    fn source_name(&self) -> &str {
        "coinbase_rest"
    }

    fn declared_limit(&self) -> RateLimit {
        RateLimit::per_second(8)
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        let idx = {
            let mut c = self.calls.lock().unwrap();
            c.push((req.start, req.end));
            c.len() - 1
        };
        if self.rate_limit_at.contains(&idx) {
            return Err(FetchError::RateLimited {
                venue: "coinbase".into(),
                retry_after_ms: 750,
            });
        }
        if self.terms_at == Some(idx) {
            return Err(FetchError::Terms(
                "bulk history downloads are prohibited".into(),
            ));
        }
        if self.stuck {
            return Ok(BackfillPage {
                bars: Vec::new(),
                covered_to: Some(req.start),
            });
        }
        let mut bars = Vec::new();
        let mut t = req.start;
        while t < req.end {
            bars.push(BackfillBar {
                close_time: t,
                open: "100".into(),
                high: "100".into(),
                low: "100".into(),
                close: "100".into(),
                volume: "1".into(),
                trade_count: 1,
            });
            t += Duration::minutes(1);
        }
        Ok(BackfillPage {
            bars,
            covered_to: Some(req.end),
        })
    }
}

#[derive(Default)]
struct CountingSink {
    /// bar close -> how many times it was written. Anything above 1 is a refetch
    /// that reached the store.
    written: Mutex<HashMap<i64, u32>>,
}

#[async_trait]
impl BarSink for CountingSink {
    async fn write_page(
        &self,
        _source: &str,
        _venue: &str,
        _instrument: &str,
        _timeframe: &str,
        bars: &[BackfillBar],
    ) -> Result<SinkReport, SinkError> {
        let mut w = self.written.lock().unwrap();
        let mut inserted = 0;
        let mut unchanged = 0;
        for b in bars {
            let e = w.entry(b.close_time.timestamp()).or_insert(0);
            *e += 1;
            if *e == 1 {
                inserted += 1;
            } else {
                unchanged += 1;
            }
        }
        Ok(SinkReport {
            inserted,
            restated: 0,
            unchanged,
            rejected: 0,
        })
    }
}

/// The cursor store, in memory. Mirrors the 0099 semantics that matter:
/// `ensure` never resets, `advance` never rewinds, `complete` pins the cursor to
/// `window_end`, and a completed row is immutable.
#[derive(Default)]
struct MemState {
    rows: Mutex<HashMap<BackfillKey, BackfillState>>,
}

impl MemState {
    fn get(&self, key: &BackfillKey) -> Option<BackfillState> {
        self.rows.lock().unwrap().get(key).cloned()
    }

    fn edit(&self, key: &BackfillKey, f: impl FnOnce(&mut BackfillState)) {
        let mut rows = self.rows.lock().unwrap();
        if let Some(r) = rows.get_mut(key) {
            if r.status == BackfillStatus::Completed {
                return; // completed rows are terminal
            }
            f(r);
            r.updated_at = Utc::now();
        }
    }
}

#[async_trait]
impl BackfillStateStore for MemState {
    async fn load(&self, key: &BackfillKey) -> Result<Option<BackfillState>, StateError> {
        Ok(self.get(key))
    }

    async fn ensure(&self, key: &BackfillKey) -> Result<BackfillState, StateError> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(key.clone()).or_insert_with(|| BackfillState {
            key: key.clone(),
            cursor: None,
            status: BackfillStatus::Pending,
            attempts: 0,
            backoff_until: None,
            last_error: None,
            pages_fetched: 0,
            rows_written: 0,
            updated_at: Utc::now(),
        });
        Ok(row.clone())
    }

    async fn start(&self, key: &BackfillKey) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.status = BackfillStatus::Running;
            r.backoff_until = None;
        });
        Ok(())
    }

    async fn advance(
        &self,
        key: &BackfillKey,
        cursor: DateTime<Utc>,
        rows_written: i64,
    ) -> Result<(), StateError> {
        self.edit(key, |r| {
            let floor = r.cursor.unwrap_or(r.key.window_start);
            r.cursor = Some(floor.max(cursor.min(r.key.window_end)));
            r.pages_fetched += 1;
            r.rows_written += rows_written;
            r.status = BackfillStatus::Running;
            r.backoff_until = None;
            r.last_error = None;
        });
        Ok(())
    }

    async fn pause(
        &self,
        key: &BackfillKey,
        until: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.status = BackfillStatus::Paused;
            r.backoff_until = Some(until);
            r.attempts += 1;
            r.last_error = Some(reason.to_string());
        });
        Ok(())
    }

    async fn complete(&self, key: &BackfillKey) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.status = BackfillStatus::Completed;
            r.cursor = Some(r.key.window_end);
            r.backoff_until = None;
            r.last_error = None;
        });
        Ok(())
    }

    async fn fail(&self, key: &BackfillKey, reason: &str) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.status = BackfillStatus::Failed;
            r.attempts += 1;
            r.last_error = Some(reason.to_string());
        });
        Ok(())
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────────

fn t(h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, h, 0, 0).unwrap()
}

/// A 24-hour 1m window; with a 5-hour page span that is 5 pages.
fn manifest() -> BackfillManifest {
    BackfillManifest {
        source: "coinbase_rest".into(),
        venue: "coinbase".into(),
        instrument: "BTC-USD".into(),
        timeframe: "1m".into(),
        window_start: t(0),
        window_end: t(0) + Duration::hours(24),
    }
}

fn policy(max_pages: u64) -> BackfillPolicy {
    BackfillPolicy {
        page_span: Duration::hours(5),
        max_pages_per_run: max_pages,
        ..BackfillPolicy::default()
    }
}

fn never_cancelled() -> impl Fn() -> bool + Sync {
    || false
}

/// The coverage assertion: the recorded calls tile `[start, end)` exactly.
/// Overlap means a page was fetched twice; a hole means one was skipped.
fn assert_tiles_exactly(
    calls: &[(DateTime<Utc>, DateTime<Utc>)],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) {
    let mut sorted = calls.to_vec();
    sorted.sort();
    let mut at = start;
    for (s, e) in &sorted {
        assert_eq!(
            *s, at,
            "page starting {s} does not continue from {at}: a page was fetched twice or skipped"
        );
        assert!(e > s, "empty page request {s}..{e}");
        at = *e;
    }
    assert_eq!(at, end, "coverage stopped at {at}, short of {end}");
}

// ── the acceptance criteria ──────────────────────────────────────────────────

/// P-1.1: killing the worker mid-window and restarting fetches no page twice and
/// skips none. The "kill" is a page budget that stops the run partway, which is
/// indistinguishable from a lost lease as far as the cursor is concerned.
#[tokio::test]
async fn killing_the_worker_mid_window_refetches_no_page_and_skips_none() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource::new());
    let sink = Arc::new(CountingSink::default());
    let calls = src.calls.clone();

    // Run 1 dies after two pages.
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(2));
    let out = engine.run(&m, &never_cancelled()).await.unwrap();
    assert!(
        matches!(out, RunOutcome::Paused { .. }),
        "budget reached is a pause, not a failure: {out:?}"
    );
    assert_eq!(src.count(), 2);

    // Run 2 is a *different engine instance* — a new worker process — sharing
    // only the durable cursor.
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));
    let out = engine.run(&m, &never_cancelled()).await.unwrap();
    assert!(matches!(out, RunOutcome::Completed { .. }), "{out:?}");

    let calls = calls.lock().unwrap().clone();
    assert_eq!(
        calls.len(),
        5,
        "24 h in 5 h pages is 5 pages, fetched once each"
    );
    assert_tiles_exactly(&calls, m.window_start, m.window_end);

    // And nothing reached the store twice.
    {
        let written = sink.written.lock().unwrap();
        assert_eq!(written.len(), 24 * 60, "one row per minute of the window");
        assert!(
            written.values().all(|n| *n == 1),
            "a bar was written more than once: the resume refetched"
        );
    }

    // The cursor row's tally survives the restart too. `advance` is handed each
    // page's own count and the store adds it; the engine used to hand it the
    // run's running total, which the live store summed into a triangle number
    // (896 rows recorded as 79,597, 2026-10-03) and which restarts at zero
    // after a kill.
    let st = state.load(&m.key()).await.unwrap().unwrap();
    assert_eq!(
        st.rows_written,
        24 * 60,
        "rows_written must equal the rows that landed, across both runs"
    );
}

/// P-1.1: resubmitting a completed window performs **zero** fetches.
#[tokio::test]
async fn resubmitting_a_completed_window_performs_zero_fetches() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource::new());
    let sink = Arc::new(CountingSink::default());

    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));
    assert!(matches!(
        engine.run(&m, &never_cancelled()).await.unwrap(),
        RunOutcome::Completed { .. }
    ));
    let after_first = src.count();
    assert_eq!(after_first, 5);

    for _ in 0..3 {
        let out = engine.run(&m, &never_cancelled()).await.unwrap();
        assert_eq!(out, RunOutcome::AlreadyComplete);
    }
    assert_eq!(
        src.count(),
        after_first,
        "a completed window must not be fetched again"
    );
}

/// P-1.1: a 429 advances `attempts` and sets a backoff **without** marking the
/// job failed. The adversarial half: a run that reported `Failed` here would
/// make every rate-limited backfill look like an incident.
#[tokio::test]
async fn a_429_pauses_with_backoff_and_never_marks_the_job_failed() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource {
        rate_limit_at: vec![1],
        ..CountingSource::new()
    });
    let sink = Arc::new(CountingSink::default());

    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));
    let out = engine.run(&m, &never_cancelled()).await.unwrap();

    let RunOutcome::Paused { until, reason, .. } = out else {
        panic!("a 429 must pause, got {out:?}")
    };
    assert!(
        reason.contains("rate limited"),
        "the row must record why it paused, got {reason:?}"
    );

    let row = state.get(&m.key()).unwrap();
    assert_eq!(
        row.status,
        BackfillStatus::Paused,
        "a rate limit is a pause, never a failure"
    );
    assert_ne!(row.status, BackfillStatus::Failed);
    assert_eq!(row.attempts, 1, "the pause is counted");
    assert_eq!(
        row.backoff_until,
        Some(until),
        "a pause with no deadline is a stall"
    );
    assert!(until >= Utc::now() - Duration::seconds(1));
    // The first page's work is not lost.
    assert_eq!(row.cursor, Some(m.window_start + Duration::hours(5)));
}

/// Adversarial: while the backoff has not elapsed, the window is not fetched at
/// all. Without this, a worker pool turns one 429 into a thousand.
#[tokio::test]
async fn a_paused_window_inside_its_backoff_is_not_fetched() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource {
        rate_limit_at: vec![0],
        ..CountingSource::new()
    });
    let sink = Arc::new(CountingSink::default());
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));

    assert!(matches!(
        engine.run(&m, &never_cancelled()).await.unwrap(),
        RunOutcome::Paused { .. }
    ));
    assert_eq!(src.count(), 1);

    for _ in 0..20 {
        let out = engine.run(&m, &never_cancelled()).await.unwrap();
        assert!(matches!(out, RunOutcome::Paused { .. }), "{out:?}");
    }
    assert_eq!(
        src.count(),
        1,
        "20 further claims of a backed-off window must not reach the venue once"
    );
}

/// Adversarial: an adapter that reports no progress must fail loudly rather than
/// spin forever asking the venue for the same page.
#[tokio::test]
async fn a_source_that_never_advances_fails_instead_of_spinning() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource {
        stuck: true,
        ..CountingSource::new()
    });
    let sink = Arc::new(CountingSink::default());
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));

    let out = engine.run(&m, &never_cancelled()).await.unwrap();
    let RunOutcome::Failed { reason } = out else {
        panic!("expected a failure, got {out:?}")
    };
    assert!(reason.contains("did not advance"), "{reason}");
    assert_eq!(
        src.count(),
        1,
        "one call is enough to know; the loop must not retry a stuck adapter"
    );
    assert_eq!(state.get(&m.key()).unwrap().status, BackfillStatus::Failed);
}

/// Adversarial: a terms refusal is terminal, and it is *not* a pause. The
/// platform never builds against terms (Set P MASTER §5), and a source that says
/// no must not be retried on a backoff until somebody notices.
#[tokio::test]
async fn a_terms_refusal_is_terminal_and_is_never_retried() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource {
        terms_at: Some(0),
        ..CountingSource::new()
    });
    let sink = Arc::new(CountingSink::default());
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));

    let out = engine.run(&m, &never_cancelled()).await.unwrap();
    assert!(matches!(out, RunOutcome::Failed { .. }), "{out:?}");
    let row = state.get(&m.key()).unwrap();
    assert_eq!(row.status, BackfillStatus::Failed);
    assert_ne!(
        row.status,
        BackfillStatus::Paused,
        "a terms refusal is not something to wait out"
    );

    // And a re-run does not ask again.
    assert!(matches!(
        engine.run(&m, &never_cancelled()).await.unwrap(),
        RunOutcome::Failed { .. }
    ));
    assert_eq!(src.count(), 1);
}

/// Cancellation is a pause: the window resumes where it stopped rather than
/// restarting or being marked failed.
#[tokio::test]
async fn cancelling_mid_window_pauses_at_the_cursor() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource::new());
    let sink = Arc::new(CountingSink::default());
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));

    let stop_after = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || stop_after.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2;
    let out = engine.run(&m, &cancel).await.unwrap();
    assert!(matches!(out, RunOutcome::Paused { .. }), "{out:?}");

    let row = state.get(&m.key()).unwrap();
    assert_eq!(row.status, BackfillStatus::Paused);
    assert_eq!(
        row.cursor,
        Some(m.window_start + Duration::hours(10)),
        "two pages of work survived the cancel"
    );

    // Resuming continues rather than restarting.
    let engine =
        BackfillEngine::new(src.clone(), sink.clone(), state.clone()).with_policy(policy(99));
    assert!(matches!(
        engine.run(&m, &never_cancelled()).await.unwrap(),
        RunOutcome::Completed { .. }
    ));
    assert_tiles_exactly(&src.calls.lock().unwrap(), m.window_start, m.window_end);
}

/// The manifest is the identity of the window: two submissions of the same
/// window hash identically, and any change of window is a different job.
#[test]
fn the_manifest_hash_identifies_the_window_and_nothing_else() {
    let a = manifest();
    let mut b = manifest();
    assert_eq!(a.manifest_hash(), b.manifest_hash());

    b.window_end += Duration::hours(1);
    assert_ne!(
        a.manifest_hash(),
        b.manifest_hash(),
        "a different window is a different job"
    );

    let mut c = manifest();
    c.source = "kraken_rest".into();
    assert_ne!(
        a.manifest_hash(),
        c.manifest_hash(),
        "the same window from a different source is a different claim about knowledge time, not a duplicate"
    );
}

/// The backoff never undercuts what the venue asked for. Ours is a guess; the
/// venue's `Retry-After` is the only number that knows.
#[test]
fn the_venue_s_retry_after_outranks_our_exponential_backoff() {
    let p = BackfillPolicy::default();
    assert_eq!(p.backoff(1, None), Duration::seconds(4));
    assert_eq!(
        p.backoff(1, Some(90_000)),
        Duration::milliseconds(90_000),
        "a 90 s Retry-After is honoured in full"
    );
    assert_eq!(
        p.backoff(1, Some(10)),
        Duration::seconds(4),
        "a tiny Retry-After does not shorten our own backoff"
    );
    assert!(
        p.backoff(30, None) <= p.backoff_cap,
        "the backoff is capped rather than unbounded"
    );
}

/// An unknown timeframe fails the window rather than guessing a bar length. A
/// guessed period silently shifts every bar in the series.
#[tokio::test]
async fn an_unknown_timeframe_fails_rather_than_guessing_a_period() {
    let mut m = manifest();
    m.timeframe = "3m".into();
    let state = Arc::new(MemState::default());
    let src = Arc::new(CountingSource::new());
    let sink = Arc::new(CountingSink::default());
    let engine = BackfillEngine::new(src.clone(), sink.clone(), state.clone());

    let out = engine.run(&m, &never_cancelled()).await.unwrap();
    assert!(matches!(out, RunOutcome::Failed { .. }), "{out:?}");
    assert_eq!(
        src.count(),
        0,
        "nothing is fetched for a window we cannot interpret"
    );
}

/// A source that declares how much one request can carry: the engine must ask for
/// exactly that, not the policy's fixed slice, and still tile the window.
struct WideSource(CountingSource);

#[async_trait]
impl BackfillSource for WideSource {
    fn source_name(&self) -> &str {
        self.0.source_name()
    }
    fn declared_limit(&self) -> RateLimit {
        self.0.declared_limit()
    }
    fn max_page_span(&self, _period_secs: u32) -> Option<Duration> {
        Some(Duration::hours(10))
    }
    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        self.0.fetch_page(req).await
    }
}

/// A venue's declared page span sets the page size: 24 h in 10 h pages is three
/// requests, not the five a fixed 5 h policy slice costs, and coverage still tiles.
#[tokio::test]
async fn the_engine_pages_by_the_span_the_venue_declares() {
    let m = manifest();
    let state = Arc::new(MemState::default());
    let src = Arc::new(WideSource(CountingSource::new()));
    let sink = Arc::new(CountingSink::default());
    let engine = BackfillEngine::new(src.clone(), sink.clone(), state).with_policy(policy(99));
    assert!(matches!(
        engine.run(&m, &never_cancelled()).await.unwrap(),
        RunOutcome::Completed { .. }
    ));
    let calls = src.0.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_tiles_exactly(&calls, m.window_start, m.window_end);
    assert_eq!(sink.written.lock().unwrap().len(), 24 * 60);
}
