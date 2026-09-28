//! The `backfill` job kind: deep history, resumably (DATA-005 §11 S1, §12;
//! DA-11; ADR-0030; Set P P-1.1).
//!
//! One job is one `(source, venue, instrument, timeframe, window)`. The window
//! is walked in pages of bar-close time; after every page the cursor moves in
//! `dataplane.backfill_state` (migration 0099), so the unit of lost work when a
//! worker dies is one page, never the window.
//!
//! ## The three properties, and what each would cost if it were only a comment
//!
//! * **Resumable.** A killed worker's replacement starts at
//!   `coalesce(cursor, window_start)`. Without the durable cursor, an eight-hour
//!   backfill that dies at hour seven starts again at hour zero — and against a
//!   rate-limited free endpoint, restarting is not merely slow, it is a second
//!   full day of requests the venue sees.
//! * **Idempotent.** Resubmitting a completed window performs **zero** fetches
//!   ([`RunOutcome::AlreadyComplete`]), and re-collecting an overlapping range
//!   writes nothing new — `BarStore` compares cell values and reports them
//!   `unchanged`. Both are proved by counting adapter calls, not by inspecting
//!   the result.
//! * **Rate-limit-safe.** A 429 is [`RunOutcome::Paused`] with a recorded
//!   `backoff_until`, never a failure. The distinction is operational, not
//!   cosmetic: an operator who sees `failed` on a healthy rate-limited backfill
//!   learns to ignore the status column, and the next real failure is invisible.
//!
//! ## What this module deliberately does not do
//!
//! It never talks HTTP and it never writes ClickHouse. It holds
//! [`BackfillSource`] and [`BarSink`], so the loop can be driven by counting
//! doubles. "No page is fetched twice" is a claim about call counts; a test that
//! cannot count calls cannot make it.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

use dataplane::backfill::{
    BackfillKey, BackfillSource, BackfillStateStore, BackfillStatus, BarSink, PageRequest,
    StateError,
};

use ledger::TerminalReason;

use crate::types::{JobError, JobKind, JobOutput, Progress};
use crate::worker::{JobContext, Worker};

// ── the manifest ─────────────────────────────────────────────────────────────

/// What a backfill job *is*. Serialised into the job manifest, so two identical
/// submissions collapse to one job through the service's content hash (JB-02),
/// and used verbatim as the `backfill_state` key, so the job and its cursor
/// cannot drift into describing different windows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillManifest {
    /// `dataplane.source.name`. Selects the declared vendor lag stamped on every
    /// row (ADR-P2-24) — `coinbase_rest` 60 s, `alpaca_rest` 900 s.
    pub source: String,
    pub venue: String,
    /// The platform symbol, e.g. `BTC-USD`.
    pub instrument: String,
    pub timeframe: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

impl BackfillManifest {
    #[must_use]
    pub fn key(&self) -> BackfillKey {
        BackfillKey {
            source: self.source.clone(),
            venue: self.venue.clone(),
            instrument: self.instrument.clone(),
            timeframe: self.timeframe.clone(),
            window_start: self.window_start,
            window_end: self.window_end,
        }
    }

    /// RFC 3339 throughout, so the canonical JSON of two callers who built the
    /// same window in different timezones is byte-identical.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "source": self.source,
            "venue": self.venue,
            "instrument": self.instrument,
            "timeframe": self.timeframe,
            "window_start": self.window_start.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            "window_end": self.window_end.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        })
    }

    /// Parse a manifest a worker was handed.
    ///
    /// # Errors
    /// A malformed or incomplete manifest.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("bad backfill manifest: {e}"))
    }

    /// The job-service idempotency hash of this window.
    ///
    /// `data_snapshot_id` is the *window*, not `now`: two submissions of the
    /// same window an hour apart are the same job, which is exactly the
    /// behaviour that makes a resubmitted backfill free.
    #[must_use]
    pub fn manifest_hash(&self) -> String {
        crate::manifest::hash(
            &self.to_value(),
            &[],
            "backfill",
            JobKind::Backfill.as_str(),
        )
    }

    /// Bar length in seconds for the platform's timeframe labels.
    ///
    /// # Errors
    /// An unknown label. Guessing a period would silently shift every bar.
    pub fn period_secs(&self) -> Result<u32, String> {
        period_secs(&self.timeframe)
    }
}

/// # Errors
/// An unknown timeframe label.
pub fn period_secs(timeframe: &str) -> Result<u32, String> {
    Ok(match timeframe {
        "1m" => 60,
        "5m" => 300,
        "15m" => 900,
        "1h" => 3_600,
        "4h" => 14_400,
        "1d" => 86_400,
        other => return Err(format!("unknown timeframe {other:?}")),
    })
}

// ── the engine ───────────────────────────────────────────────────────────────

/// How a single run of one window ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// The window was already complete before this run started. **Zero fetches.**
    AlreadyComplete,
    /// The window is now complete.
    Completed { pages: u64, rows: u64 },
    /// Waiting, not broken: rate limited, transiently unavailable, an archive
    /// that did not verify, or this run's page budget spent.
    Paused {
        until: DateTime<Utc>,
        reason: String,
        pages: u64,
        rows: u64,
    },
    /// Terminal and human-facing.
    Failed { reason: String },
}

/// Tuning that a deployment may reasonably want to change.
#[derive(Clone, Copy, Debug)]
pub struct BackfillPolicy {
    /// How much bar-close time one page asks for.
    pub page_span: Duration,
    /// Pages per run before the job yields, so one deep window cannot starve
    /// every other instrument behind it for a day.
    pub max_pages_per_run: u64,
    /// Floor of the exponential backoff, doubled per recorded attempt.
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
}

impl Default for BackfillPolicy {
    fn default() -> Self {
        Self {
            page_span: Duration::hours(5),
            max_pages_per_run: 2_000,
            backoff_base: Duration::seconds(2),
            backoff_cap: Duration::minutes(15),
        }
    }
}

impl BackfillPolicy {
    /// Exponential backoff for `attempts` recorded pauses, never below what the
    /// venue itself asked for. A venue's `Retry-After` outranks our arithmetic:
    /// it is the only party that knows when it will answer again.
    #[must_use]
    pub fn backoff(&self, attempts: i32, venue_asked_ms: Option<u64>) -> Duration {
        let shift = attempts.clamp(0, 8) as u32;
        let ours = self.backoff_base * 2_i32.saturating_pow(shift);
        let ours = if ours > self.backoff_cap {
            self.backoff_cap
        } else {
            ours
        };
        match venue_asked_ms {
            Some(ms) => {
                let asked = Duration::milliseconds(i64::try_from(ms).unwrap_or(i64::MAX));
                if asked > ours {
                    asked
                } else {
                    ours
                }
            }
            None => ours,
        }
    }
}

/// Drives one window to completion, or to an honest stop.
pub struct BackfillEngine {
    source: Arc<dyn BackfillSource>,
    sink: Arc<dyn BarSink>,
    state: Arc<dyn BackfillStateStore>,
    policy: BackfillPolicy,
}

impl BackfillEngine {
    #[must_use]
    pub fn new(
        source: Arc<dyn BackfillSource>,
        sink: Arc<dyn BarSink>,
        state: Arc<dyn BackfillStateStore>,
    ) -> Self {
        Self {
            source,
            sink,
            state,
            policy: BackfillPolicy::default(),
        }
    }

    #[must_use]
    pub fn with_policy(mut self, policy: BackfillPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The `dataplane.source.name` this engine's adapter declares.
    ///
    /// Read by [`BackfillRouter::with`] so the routing key comes from the adapter
    /// rather than from whoever is assembling the table.
    #[must_use]
    pub fn source_name(&self) -> &str {
        self.source.source_name()
    }

    /// Run one window as far as it gets.
    ///
    /// # Errors
    /// Only the cursor store failing. Every *source* failure is an outcome, not
    /// an error: a rate limit is a normal event in a backfill's life and must
    /// not travel the same path as a bug.
    #[allow(clippy::too_many_lines)]
    pub async fn run(
        &self,
        m: &BackfillManifest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<RunOutcome, StateError> {
        let key = m.key();
        // **The manifest names a source; this engine holds one.** Nothing checked
        // that they were the same, and the consequence is not a wrong answer, it is
        // a *plausible* one: `BarStoreSink` reads `declared_vendor_lag_ms` for
        // `manifest.source`, so an Alpaca window run through the Coinbase adapter
        // writes Coinbase's candles stamped with Alpaca's 900 s knowledge time. The
        // rows look observed-late and are silently inadmissible as PIT evidence;
        // the reverse direction stamps 900 s-delayed bars as 60 s and is worse,
        // because it makes them look admissible. See ADR-P2-24 and P-1.4.
        //
        // This refuses **before** `state.ensure`, so a mis-routed job leaves no
        // `backfill_state` row behind: the window was not attempted, and a row
        // saying `failed` would tell an operator to look at the window rather than
        // at whoever built the routing table.
        if m.source != self.source.source_name() {
            return Ok(RunOutcome::Failed {
                reason: format!(
                    "this engine drives `{}`, and the manifest asks for `{}`. Nothing was fetched and no cursor was created. A backfill's source selects the declared vendor lag stamped on every row, so running a window through the wrong adapter is a point-in-time corruption rather than a mix-up.",
                    self.source.source_name(),
                    m.source
                ),
            });
        }
        let period = match m.period_secs() {
            Ok(p) => p,
            Err(e) => {
                self.state.fail(&key, &e).await?;
                return Ok(RunOutcome::Failed { reason: e });
            }
        };
        let st = self.state.ensure(&key).await?;

        // The idempotency claim, and the cheapest branch in the module: a
        // completed window returns before a single adapter call.
        if st.status == BackfillStatus::Completed {
            return Ok(RunOutcome::AlreadyComplete);
        }
        // A paused window whose backoff has not elapsed is likewise not touched.
        // Without this, a worker pool would hammer a venue that just said no.
        if st.status == BackfillStatus::Paused {
            if let Some(until) = st.backoff_until {
                if until > Utc::now() {
                    return Ok(RunOutcome::Paused {
                        until,
                        reason: st
                            .last_error
                            .unwrap_or_else(|| "backoff not elapsed".into()),
                        pages: 0,
                        rows: 0,
                    });
                }
            }
        }
        if st.status == BackfillStatus::Failed {
            return Ok(RunOutcome::Failed {
                reason: st.last_error.unwrap_or_else(|| "failed".into()),
            });
        }

        self.state.start(&key).await?;
        let mut cursor = st.resume_from();
        let mut attempts = st.attempts;
        let (mut pages, mut rows) = (0_u64, 0_u64);

        loop {
            if cursor >= key.window_end {
                self.state.complete(&key).await?;
                info!(instrument = %key.instrument, timeframe = %key.timeframe, pages, rows, "backfill window complete");
                return Ok(RunOutcome::Completed { pages, rows });
            }
            if cancelled() {
                let until = Utc::now();
                self.state
                    .pause(
                        &key,
                        until,
                        "cancelled; the window resumes where it stopped",
                    )
                    .await?;
                return Ok(RunOutcome::Paused {
                    until,
                    reason: "cancelled".into(),
                    pages,
                    rows,
                });
            }
            if pages >= self.policy.max_pages_per_run {
                // Yield, not fail. The row stays claimable immediately; the point
                // is fairness between instruments, not a cooldown.
                let until = Utc::now();
                self.state
                    .pause(&key, until, "page budget for this run reached; resuming")
                    .await?;
                return Ok(RunOutcome::Paused {
                    until,
                    reason: "page budget reached".into(),
                    pages,
                    rows,
                });
            }

            let page_end = std::cmp::min(cursor + self.policy.page_span, key.window_end);
            let req = PageRequest {
                venue: key.venue.clone(),
                symbol: key.instrument.clone(),
                timeframe: key.timeframe.clone(),
                period_secs: period,
                start: cursor,
                end: page_end,
            };

            let page = match self.source.fetch_page(&req).await {
                Ok(p) => p,
                Err(e) if e.is_pause() => {
                    attempts += 1;
                    let back = self.policy.backoff(attempts, e.retry_after_ms());
                    let until = Utc::now() + back;
                    // Recorded on the row, in the reason, so the operator can see
                    // it was the venue's decision and not ours.
                    self.state.pause(&key, until, &e.to_string()).await?;
                    warn!(instrument = %key.instrument, attempts, backoff_s = back.num_seconds(), error = %e, "backfill paused");
                    return Ok(RunOutcome::Paused {
                        until,
                        reason: e.to_string(),
                        pages,
                        rows,
                    });
                }
                Err(e) => {
                    self.state.fail(&key, &e.to_string()).await?;
                    return Ok(RunOutcome::Failed {
                        reason: e.to_string(),
                    });
                }
            };

            // Defensive clip. A venue that returns a bar outside the window is
            // not malicious, it is paginating differently than documented; the
            // cursor arithmetic below assumes the clip, so it happens here once
            // rather than being remembered in three places.
            let bars: Vec<_> = page
                .bars
                .iter()
                .filter(|b| b.close_time >= key.window_start && b.close_time < key.window_end)
                .cloned()
                .collect();

            if !bars.is_empty() {
                match self
                    .sink
                    .write_page(
                        &key.source,
                        &key.venue,
                        &key.instrument,
                        &key.timeframe,
                        &bars,
                    )
                    .await
                {
                    Ok(report) => rows += report.written(),
                    Err(e) => {
                        // A sink failure is *not* a source failure: pause rather
                        // than fail so a ClickHouse restart does not lose a
                        // half-finished multi-year window.
                        attempts += 1;
                        let until = Utc::now() + self.policy.backoff(attempts, None);
                        self.state.pause(&key, until, &format!("sink: {e}")).await?;
                        return Ok(RunOutcome::Paused {
                            until,
                            reason: format!("sink: {e}"),
                            pages,
                            rows,
                        });
                    }
                }
            }
            pages += 1;

            let Some(next) = page.covered_to else {
                // The source says there is nothing further in this window. That
                // is completion, not a gap: whatever the venue never had, no
                // number of retries will produce.
                self.state.complete(&key).await?;
                return Ok(RunOutcome::Completed { pages, rows });
            };
            if next <= cursor {
                // A source that does not advance would spin forever, fetching
                // the same page and writing nothing. Fail loudly: this is an
                // adapter bug and it should read as one.
                let reason = format!(
                    "source {} did not advance past {cursor}",
                    self.source.source_name()
                );
                self.state.fail(&key, &reason).await?;
                return Ok(RunOutcome::Failed { reason });
            }
            let next = std::cmp::min(next, key.window_end);
            self.state
                .advance(
                    &key,
                    next,
                    i64::try_from(rows)
                        .unwrap_or(i64::MAX)
                        .min(i64::from(u32::MAX)),
                )
                .await?;
            cursor = next;
        }
    }
}

// ── the job-service worker ───────────────────────────────────────────────────

/// Runs `backfill` jobs inside the durable job service.
pub struct BackfillWorker {
    engine: Arc<BackfillEngine>,
}

impl BackfillWorker {
    #[must_use]
    pub const fn new(engine: Arc<BackfillEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl Worker for BackfillWorker {
    fn kind(&self) -> JobKind {
        JobKind::Backfill
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let m = BackfillManifest::from_value(manifest)
            .map_err(|e| JobError::logic(TerminalReason::DataError, "invalid_manifest", e))?;
        ctx.progress(Progress {
            pct: Some(0.0),
            stage: Some("backfill".into()),
            message: Some(format!(
                "{} {} from {}",
                m.instrument, m.timeframe, m.source
            )),
        })
        .await;

        let cancelled = || ctx.is_cancelled();
        let outcome = self
            .engine
            .run(&m, &cancelled)
            .await
            .map_err(|e| JobError::infrastructure(format!("backfill_state: {e}")))?;

        match outcome {
            RunOutcome::AlreadyComplete => Ok(output("already_complete", 0, 0, &m)),
            RunOutcome::Completed { pages, rows } => Ok(output("completed", pages, rows, &m)),
            // A pause is a *retryable infrastructure* failure rather than a
            // success: the job service re-queues the window instead of marking
            // it done with a hole in it. The `backfill_state` row says `paused`,
            // which is what an operator reads; the job says "retry", which is
            // what the scheduler needs. Both are true and neither is the other.
            RunOutcome::Paused {
                until,
                reason,
                pages,
                rows,
            } => {
                let mut e = JobError::infrastructure("backfill_paused");
                e.fix = Some(format!(
                    "resumes at {until}: {reason} ({pages} pages, {rows} rows this run)"
                ));
                Err(e)
            }
            RunOutcome::Failed { reason } => Err(JobError::logic(
                TerminalReason::DataError,
                "backfill_failed",
                reason,
            )),
        }
    }
}

fn output(status: &str, pages: u64, rows: u64, m: &BackfillManifest) -> JobOutput {
    JobOutput {
        summary: Some(format!(
            "{status}: {} {} from {} — {pages} pages, {rows} rows",
            m.instrument, m.timeframe, m.source
        )),
        result: json!({
            "status": status,
            "pages": pages,
            "rows": rows,
            "source": m.source,
            "instrument": m.instrument,
            "timeframe": m.timeframe,
        }),
        artifacts: Vec::new(),
        actual: crate::types::Cost::default(),
    }
    .clamped()
}

// ── routing one job kind across several venues ───────────────────────────────

/// One `backfill` worker over several venues.
///
/// **Why this exists rather than a `BackfillWorker` per venue.** `WorkerPool`
/// stores its workers in a `HashMap<JobKind, _>` and every backfill worker
/// answers `JobKind::Backfill`, so registering three of them keeps whichever was
/// registered last and silently discards the other two. The pool is right to do
/// that — two workers for one kind would both claim the same rows — so the fan-out
/// has to happen one level down, here, on the field the manifest already carries.
///
/// The map is built at the composition root, because *which* venues a deployment
/// can reach is a deployment fact: Coinbase and Kraken serve their candle
/// endpoints to anyone, and Alpaca does not.
///
/// **An unroutable source is a terminal, named refusal**, not a pause and not a
/// default. Defaulting to "the first engine" is the failure this type exists to
/// prevent (see the guard in [`BackfillEngine::run`]); pausing would leave the
/// operator waiting for a retry that cannot change the outcome. The message lists
/// the sources that *are* routable, because the most likely cause is a typo in a
/// source name and the second most likely is a venue this deployment has no
/// credential for.
pub struct BackfillRouter {
    engines: std::collections::BTreeMap<String, Arc<BackfillEngine>>,
}

impl BackfillRouter {
    /// A router with no venues. Registering it is legal and does nothing useful:
    /// every job gets the "no engine" refusal, which is a truthful answer and the
    /// one an operator can act on.
    #[must_use]
    pub fn new() -> Self {
        Self {
            engines: std::collections::BTreeMap::new(),
        }
    }

    /// Adds one venue's engine under the `dataplane.source.name` its adapter
    /// declares.
    ///
    /// The key is taken from `source_name()` rather than passed in, so a caller
    /// cannot register the Coinbase engine under `"alpaca_rest"` — which would
    /// route past [`BackfillEngine::run`]'s guard into exactly the mis-stamping
    /// that guard exists to stop.
    #[must_use]
    pub fn with(mut self, engine: Arc<BackfillEngine>) -> Self {
        let name = engine.source_name().to_string();
        self.engines.insert(name, engine);
        self
    }

    /// The source names this router can drive, in a stable order.
    #[must_use]
    pub fn sources(&self) -> Vec<&str> {
        self.engines.keys().map(String::as_str).collect()
    }
}

impl Default for BackfillRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Worker for BackfillRouter {
    fn kind(&self) -> JobKind {
        JobKind::Backfill
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let m = BackfillManifest::from_value(manifest)
            .map_err(|e| JobError::logic(TerminalReason::DataError, "invalid_manifest", e))?;
        let Some(engine) = self.engines.get(&m.source) else {
            let known = self.sources().join(", ");
            let known = if known.is_empty() {
                "none — this deployment registered no backfill venues".to_string()
            } else {
                known
            };
            let mut e = JobError::logic(
                TerminalReason::DataError,
                "no_backfill_engine",
                format!("no backfill engine for source `{}`", m.source),
            );
            e.fix = Some(format!("routable sources here: {known}"));
            return Err(e);
        };
        BackfillWorker::new(Arc::clone(engine))
            .run(ctx, manifest)
            .await
    }
}
