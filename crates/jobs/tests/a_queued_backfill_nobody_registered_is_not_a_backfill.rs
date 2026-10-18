//! `JobKind::Backfill` had no registered worker, and the two ways of fixing that
//! wrongly are both silent (Set P P-1.1, P-1.4; ADR-P2-24).
//!
//! `apps/platform/src/main.rs` registered GateAdvance, DataQc, SimulatePaths,
//! Campaign, ResearchRun and Session — and nothing for `backfill` — so every
//! submitted window queued for ever. That is this repository's usual
//! built-but-unreached shape and it is the easy half.
//!
//! The hard half is that the obvious repairs are worse than the gap:
//!
//! * **Register one `BackfillWorker` per venue.** `WorkerPool::register` stores
//!   workers in a `HashMap<JobKind, _>` and all three answer
//!   `JobKind::Backfill`, so the second `register` overwrites the first and the
//!   third overwrites the second. Two of the three venues silently vanish and
//!   the pool still reports itself as having a backfill worker.
//! * **Register one worker over one adapter and let it take whatever arrives.**
//!   `BackfillEngine` held a source and never compared it with the manifest's,
//!   and `BarStoreSink` passes `manifest.source` to `write_collected`, which
//!   reads `declared_vendor_lag_ms` from `dataplane.source` to stamp
//!   `knowledge_time` (ADR-P2-24). So a window claiming `alpaca_rest` served by
//!   the Coinbase adapter writes Coinbase's candles stamped 900 s late — and, in
//!   the other direction, Alpaca's 900 s-delayed bars stamped 60 s late, which
//!   makes inadmissible rows look admissible. No error, no gap, and no way to
//!   tell afterwards which rows came from where.
//!
//! So there are two mechanisms here, and this file drives both: a guard in
//! `BackfillEngine::run` that refuses a manifest whose source is not its own, and
//! `BackfillRouter`, one worker that dispatches on the field the manifest already
//! carries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use dataplane::backfill::{
    BackfillBar, BackfillKey, BackfillPage, BackfillSource, BackfillState, BackfillStateStore,
    BackfillStatus, BarSink, FetchError, PageRequest, RateLimit, SinkError, SinkReport, StateError,
};
use jobs::kinds::backfill::{
    BackfillEngine, BackfillManifest, BackfillPolicy, BackfillRouter, RunOutcome,
};
use jobs::types::JobKind;
use jobs::worker::Worker;

// ── doubles ──────────────────────────────────────────────────────────────────

/// A source that answers for exactly one declared name and counts its calls.
struct NamedSource {
    name: &'static str,
    calls: Arc<Mutex<usize>>,
}

impl NamedSource {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl BackfillSource for NamedSource {
    fn source_name(&self) -> &str {
        self.name
    }

    fn declared_limit(&self) -> RateLimit {
        RateLimit::per_second(8)
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        *self.calls.lock().unwrap() += 1;
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

/// Records the `source` string each page was written under — the field that
/// selects the declared vendor lag downstream.
#[derive(Default)]
struct SourceRecordingSink {
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl BarSink for SourceRecordingSink {
    async fn write_page(
        &self,
        source: &str,
        _venue: &str,
        _instrument: &str,
        _timeframe: &str,
        bars: &[BackfillBar],
    ) -> Result<SinkReport, SinkError> {
        self.seen.lock().unwrap().push(source.to_string());
        Ok(SinkReport {
            inserted: bars.len() as u64,
            restated: 0,
            unchanged: 0,
            rejected: 0,
        })
    }
}

/// The cursor store, in memory, with the 0099 semantics the guard depends on:
/// `ensure` creates a row. If the guard ever runs *after* `ensure`, `rows()`
/// stops being zero and the test says so.
#[derive(Default)]
struct MemState {
    rows: Mutex<HashMap<BackfillKey, BackfillState>>,
}

impl MemState {
    fn rows(&self) -> usize {
        self.rows.lock().unwrap().len()
    }

    fn edit(&self, key: &BackfillKey, f: impl FnOnce(&mut BackfillState)) {
        let mut rows = self.rows.lock().unwrap();
        if let Some(r) = rows.get_mut(key) {
            f(r);
        }
    }
}

#[async_trait]
impl BackfillStateStore for MemState {
    async fn load(&self, key: &BackfillKey) -> Result<Option<BackfillState>, StateError> {
        Ok(self.rows.lock().unwrap().get(key).cloned())
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
        self.edit(key, |r| r.status = BackfillStatus::Running);
        Ok(())
    }

    async fn advance(
        &self,
        key: &BackfillKey,
        cursor: DateTime<Utc>,
        rows_written: i64,
    ) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.cursor = Some(cursor.min(r.key.window_end));
            r.pages_fetched += 1;
            r.rows_written += rows_written;
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
            r.last_error = Some(reason.to_string());
        });
        Ok(())
    }

    async fn complete(&self, key: &BackfillKey) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.status = BackfillStatus::Completed;
            r.cursor = Some(r.key.window_end);
        });
        Ok(())
    }

    async fn fail(&self, key: &BackfillKey, reason: &str) -> Result<(), StateError> {
        self.edit(key, |r| {
            r.status = BackfillStatus::Failed;
            r.last_error = Some(reason.to_string());
        });
        Ok(())
    }
}

// ── fixtures ─────────────────────────────────────────────────────────────────

fn window_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn manifest_for(source: &str) -> BackfillManifest {
    BackfillManifest {
        source: source.into(),
        venue: "coinbase".into(),
        instrument: "BTC-USD".into(),
        timeframe: "1m".into(),
        window_start: window_start(),
        window_end: window_start() + Duration::hours(2),
    }
}

fn policy() -> BackfillPolicy {
    BackfillPolicy {
        page_span: Duration::hours(1),
        max_pages_per_run: 10,
        ..BackfillPolicy::default()
    }
}

fn engine(
    name: &'static str,
) -> (
    Arc<BackfillEngine>,
    Arc<Mutex<usize>>,
    Arc<SourceRecordingSink>,
    Arc<MemState>,
) {
    let src = NamedSource::new(name);
    let calls = Arc::clone(&src.calls);
    let sink = Arc::new(SourceRecordingSink::default());
    let state = Arc::new(MemState::default());
    let e = Arc::new(
        BackfillEngine::new(Arc::new(src), sink.clone(), state.clone()).with_policy(policy()),
    );
    (e, calls, sink, state)
}

// ── the guard ────────────────────────────────────────────────────────────────

/// `[was a PIT corruption]` Before the guard, this window ran to completion: two
/// pages of Coinbase candles written under the source name `alpaca_rest`, whose
/// declared vendor lag is 900 s rather than 60 s.
#[tokio::test]
async fn an_engine_refuses_a_window_that_names_a_different_source() {
    let (e, calls, sink, state) = engine("coinbase_rest");
    let m = manifest_for("alpaca_rest");

    let out = e.run(&m, &|| false).await.expect("the store did not fail");

    match out {
        RunOutcome::Failed { reason } => {
            assert!(
                reason.contains("coinbase_rest") && reason.contains("alpaca_rest"),
                "the refusal must name both sides or an operator cannot tell which is wrong: {reason}"
            );
            assert!(
                reason.contains("point-in-time") || reason.contains("declared vendor lag"),
                "the refusal must say why this is not a mix-up: {reason}"
            );
        }
        other => panic!("a mis-routed window must be refused, not {other:?}"),
    }

    assert_eq!(*calls.lock().unwrap(), 0, "nothing may be fetched");
    assert!(
        sink.seen.lock().unwrap().is_empty(),
        "nothing may be written"
    );
    assert_eq!(
        state.rows(),
        0,
        "no cursor row either: a `failed` row points an operator at the window, and the window is fine"
    );
}

/// The guard must not have been bought by refusing everything.
#[tokio::test]
async fn an_engine_runs_the_window_that_names_its_own_source() {
    let (e, calls, sink, _state) = engine("coinbase_rest");
    let out = e
        .run(&manifest_for("coinbase_rest"), &|| false)
        .await
        .unwrap();
    assert!(
        matches!(out, RunOutcome::Completed { pages: 2, .. }),
        "{out:?}"
    );
    assert_eq!(*calls.lock().unwrap(), 2);
    assert_eq!(
        *sink.seen.lock().unwrap(),
        vec!["coinbase_rest", "coinbase_rest"],
        "the source reaching the sink is the one that selects the declared lag"
    );
}

// ── the router ───────────────────────────────────────────────────────────────

#[test]
fn the_router_is_the_backfill_kind_so_one_registration_covers_every_venue() {
    let (a, ..) = engine("coinbase_rest");
    let (b, ..) = engine("kraken_rest");
    let r = BackfillRouter::new().with(a).with(b);
    assert_eq!(r.kind(), JobKind::Backfill);
    assert_eq!(
        r.sources(),
        vec!["coinbase_rest", "kraken_rest"],
        "both venues survive one registration; registering two workers would not"
    );
}

/// The key comes from the adapter, not from the caller — otherwise a typo in the
/// composition root routes a window past the engine's guard, which compares
/// against the adapter's own name and would then agree.
#[test]
fn the_routing_key_is_the_adapters_own_declared_name() {
    let (a, ..) = engine("coinbase_rest");
    let r = BackfillRouter::new().with(a);
    assert_eq!(r.sources(), vec!["coinbase_rest"]);
}

#[test]
fn an_empty_router_says_so_rather_than_claiming_a_venue() {
    assert!(BackfillRouter::new().sources().is_empty());
    assert_eq!(BackfillRouter::default().kind(), JobKind::Backfill);
}

/// Registering the same adapter twice replaces rather than duplicates, so a
/// composition root that lists a venue twice cannot end up with two engines
/// racing for one source's windows.
#[test]
fn registering_one_source_twice_leaves_one_engine() {
    let (a, ..) = engine("coinbase_rest");
    let (b, ..) = engine("coinbase_rest");
    let r = BackfillRouter::new().with(a).with(b);
    assert_eq!(r.sources(), vec!["coinbase_rest"]);
}
