# Phase 1 — Deep backfill

**Completion: 43% (3 / 7 tasks), plus 2 `PARTIAL` — 2026-09-28: this headline read 0% while four of the seven tasks had named tests for their acceptance clauses. P-1.1's one remaining gap, the missing worker registration, is closed; P-1.3's is the archive path, which nothing constructs; P-1.7 is an ops run.**

**Requirement IDs:** S1, DA-11.
**Spec:** [DATA-005 §11 S1, §12](../../../specs/DATA-005-data-api-v2.md);
[06_DATA §6 S1](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD); DA-16, DA-17;
ADR-P2-24 (the sentinel and the declared lags).
**Depends on:** Phase 0 (the census, so progress is measurable against a baseline).
**Blocks:** Phase 2 (nothing to grade without history), Phase 4.
**Migration range:** 0099.
**Migrations shipped:** 0099 (merged 2026-09-15, integration wave 5).
**Parallel with:** Phase 3, and every Set S phase.

Set L Phase 0 unblocked this in 2026-09-11 by replacing the bar table whose sorting key
destroyed rows on merge. The backfill has not run. This phase runs it, resumably, inside the
job service, without getting the platform banned.

**The target is DATA-005 §10.3:** ≥ 3 years of continuous 1m history for BTC-USD and ETH-USD,
graded A or B, then the remaining initialised instruments.

---

### P-1.1 The `backfill` job kind and a resumable cursor — M
`DONE` 2026-09-28 — all three acceptance clauses have named tests in
`jobs/tests/backfill_resume.rs` (`killing_the_worker_mid_window_refetches_no_page_and_skips_none`,
`resubmitting_a_completed_window_performs_zero_fetches`,
`a_429_pauses_with_backoff_and_never_marks_the_job_failed`), and the registration gap below is
closed. Earlier note: `PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`.**
`impl Worker for BackfillWorker` exists, `JobKind::Backfill` is in
`JobKind::ALL`, migration `0099` is applied, `dataplane.backfill_state` is live,
and `jobs/tests/backfill_resume.rs` counts adapter calls. ~~**The gap is one registration:** `apps/platform/src/main.rs` registers GateAdvance, DataQc,
SimulatePaths, Campaign, ResearchRun and Session — **and no backfill worker** — so a submitted
backfill job queues for ever.~~ **Closed 2026-09-28, and it was two mechanisms rather than one
registration.**

Both of the obvious repairs are silent. *One `BackfillWorker` per venue* does not work:
`WorkerPool::register` stores workers in a `HashMap<JobKind, _>` and all three adapters answer
`JobKind::Backfill`, so the second registration overwrites the first and the pool still reports
itself as having a backfill worker. *One worker over one adapter, taking whatever arrives* is
worse: `BackfillEngine` held a source and **never compared it with the manifest's**, while
`BarStoreSink` passes `manifest.source` to `write_collected`, which reads
`declared_vendor_lag_ms` to stamp `knowledge_time` (ADR-P2-24). A window claiming `alpaca_rest`
served by the Coinbase adapter would have run to completion, writing Coinbase candles stamped
900 s late — and in the other direction, genuinely delayed Alpaca bars stamped at 60 s, which
makes inadmissible rows look admissible. No error, no gap, and nothing in the rows to say
afterwards which venue they came from.

So: `jobs::kinds::backfill::BackfillRouter`, one worker dispatching on `manifest.source` and
taking its routing key from each adapter's own `source_name()`; and a guard in
`BackfillEngine::run` that refuses a mismatched window **before** `state.ensure`, so nothing is
fetched, nothing is written, and no cursor row is left behind — a row reading `failed` would send
an operator to inspect a window that is fine. Mutation-checked: with the comparison replaced by
`if false`, `an_engine_refuses_a_window_that_names_a_different_source` fails and the other five
pass. Three source-scan guards in `main.rs`, one of which asserts the Data arm does **not** also
submit a window — registering a worker makes a human's submitted window run, and must not itself
enqueue a collection nobody asked for on every restart.

Coinbase and Kraken are wired; Alpaca is not, and the absence is deliberate. See P-1.4.

A job kind in `crates/jobs` whose manifest is `(source, instrument_id, timeframe, window)` and
whose state is a cursor in a new `backfill_state` table (migration 0099: source, instrument,
timeframe, cursor, window_start, window_end, attempts, last_error, updated_at). Content-hash
idempotency on the manifest means resubmitting the same window is a no-op rather than a second
fetch.

A killed worker resumes from the cursor. A rate-limit response is a *pause with backoff*
recorded on the row, not a failure — the distinction matters because a failed job looks like
something to fix and a paused one looks like something to wait for.

**Files:** `crates/jobs/src/kinds/backfill.rs`, `crates/storage` store, migration 0099.
**Acceptance:** killing the worker mid-window and restarting fetches no page twice and skips
none, proven by a test that counts adapter calls; resubmitting a completed window performs zero
fetches; a 429 from a stub source advances `attempts` and sets a backoff without marking the job
failed.
**Traces:** DATA-005 §11 S1; DA-11; ADR-0030 (durable job service).

### P-1.2 Coinbase Exchange candle backfill — M
`NOT STARTED`

Paginated historical candles from the Coinbase Exchange public endpoint, newest-to-oldest, into
`market_bars_v2` through `BarStore::insert_collected`. Declared rate limit honoured by a
limiter, not by a sleep. Declared vendor lag 60 s (as seeded in `dataplane.source`), so every
row written carries `knowledge_time = bar close + 60 s` and the `BACKFILLED_KNOWLEDGE_TIME`
flag.

**Files:** `crates/collectors/src/backfill/coinbase.rs`.
**Acceptance:** a 30-day BTC-USD 1m window lands with no gaps the venue does not itself have;
every written row carries the backfilled flag and the 60 s sentinel; a test drives the limiter
past its declared rate and asserts it refuses rather than delays past the limit; no row is
written to `market_bars` (`cargo xtask check-bars-v1-frozen` stays green).
**Traces:** DATA-005 §11 S1; DA-11, DA-16; ADR-P2-24; D-P2.

### P-1.3 Kraken OHLC and downloadable history files — M
`PARTIAL` 2026-09-28 — **the marker said `NOT STARTED` and two of three clauses have had named tests
for some time.** `crates/collectors/src/backfill/kraken.rs`:
`the_api_and_the_archive_agree_bar_for_bar_on_an_overlapping_window` is clause 3 verbatim, and
`an_archive_whose_checksum_does_not_match_is_refused_and_pauses` is clause 2 including the half that
matters — the error is `is_pause()` rather than a failure, and its text says nothing was inserted.
`an_archive_with_no_declared_checksum_is_refused_rather_than_trusted` covers the case the clause does
not name and should have.

**What keeps it `PARTIAL` is clause 1, and only for the archive path.** Both
`KrakenOhlcBackfill` and `KrakenArchiveBackfill` implement `BackfillSource`, and every source reaches
storage through `BarSink`, whose only production implementation is `backtest::backfill_sink::BarStoreSink`
over `BarStore::write_collected` — so the write path is single by construction. But
`KrakenArchiveBackfill` has **zero callers anywhere outside its own file**: nothing constructs it, and
the platform's backfill router (registered 2026-09-28) wires the OHLC path only. It cannot write
through anything because nothing runs it.

What it needs is configuration rather than code: the constructor takes an on-disk directory and a
`BTreeMap<String, String>` of per-file SHA-256 checksums, and nothing in `cfg` supplies either. The
checksums are the load-bearing half — `file_for` refuses a file with no declared checksum, so an
empty map means the archive path is off, which is the right default and also why it has stayed off.

Two paths, because Kraken offers both: the public OHLC endpoint for recent history, and the
downloadable history archives for depth the API will not serve. The archive path is a different
shape — fetch, verify, parse, then insert through the same `BarStore` — and it is where multiple
years actually come from.

**Files:** `crates/collectors/src/backfill/kraken.rs`.
**Acceptance:** both paths write through `BarStore::insert_collected`; an archive file whose
checksum does not match is refused and the job pauses rather than inserting partial data; the
two paths agree bar-for-bar on an overlapping window, and a test asserts it.
**Traces:** DATA-005 §11 S1; DA-11.

### P-1.4 Alpaca free-plan historical bars — S
`DONE` 2026-09-28 — **the marker said `NOT STARTED`; both clauses have named tests.**
`crates/dataplane/src/source.rs`'s
`two_backfilled_rows_for_one_bar_close_from_different_sources_differ_in_knowledge_time` is clause 1,
and it is the right shape for it: the claim is that the declared lag *reaches the row*, not merely
that a constant is 900 000 somewhere. `crates/collectors/src/backfill/alpaca.rs`'s
`the_declared_limit_sits_under_the_published_free_plan_limit` is clause 2, bounded on both sides —
declaring one request per minute would make a three-year backfill take months and would also pass a
one-sided check.

**Recorded while closing it: this adapter cannot run in the platform as configured, and that is not
this task's acceptance.** `collectors::backfill::HttpGet` sends a User-Agent and an Accept header and
no authorization, so `alpaca_rest` cannot authenticate through the backfill path at all. It is
deliberately left out of the router rather than half-wired, so a window naming it gets a terminal
`no_backfill_engine` listing the routable sources instead of a 403 from the venue that reads like a
plan restriction. Adding an auth header to `HttpGet` is the next step and belongs to whichever task
needs Alpaca history, not to this one.

The third venue, for crypto and for whatever equity history the free plan serves. Declared lag
900 s per ADR-P2-24, which is materially larger than the crypto venues and must reach the
source row rather than being assumed equal.

**Files:** `crates/collectors/src/backfill/alpaca.rs`.
**Acceptance:** rows carry the 900 s declared lag, distinct from Coinbase's 60 s, and a test
asserts two backfilled rows for the same bar-close from different sources carry different
`knowledge_time`; the free plan's own limits are declared and honoured.
**Traces:** DATA-005 §11 S1; ADR-P2-24.

### P-1.5 Revisions retained, never overwritten — M
`PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`
and the acceptance is in the tree verbatim.** `backtest/tests/bar_store_pit.rs`
holds all three clauses as named tests; `trading.market_bar`'s sorting key ends
`event_time, knowledge_time, revision_seq`, so revisions survive
ReplacingMergeTree merges **structurally**, and `store.rs` reads
`argMax(…, (knowledge_time, revision_seq))`. **The one gap is the one that
matters today:** the suite self-skips unless `BACKTEST_E2E_CLICKHOUSE_URL` is
set, and **no CI file sets it**. By the rule established the same day — a test
that skips silently is not a test, and a claim citing one is not verified — this
cannot be `DONE` until CI runs it.

DA-16 requires revisions to be retained so an `as_of` read can pick the revision that was
current then. Backfill is exactly where a second version of an already-stored bar arrives.
This task proves the path end to end: insert a bar, insert a revised version of the same bar,
and confirm both rows survive and an `as_of` read before the revision returns the first.

**Files:** `crates/backtest/src/store.rs` (the PIT read path), tests.
**Acceptance:** a revised bar produces a second row, not an update; `as_of` before the
revision's `knowledge_time` returns the original value and after it returns the revision; no
`OPTIMIZE` or collapsing engine is introduced; the check runs in the live ClickHouse suite.
**Traces:** DA-16; DATA-005 §3; INV-02.

### P-1.6 Backfilled and observed rows stay distinguishable forever — M
`DONE` 2026-09-28 — all three clauses, and the third had the shape this set keeps finding.

Clauses 1 and 2 are in `crates/dataplane/src/source.rs`:
`a_mixed_window_carries_the_maximum_declared_lag_of_its_sources` and
`the_maximum_lag_lengthens_the_embargo_it_is_computed_for` — the second matters more than it
looks, because a maximum computed and then not used in the embargo is a number in a struct.

**Clause 3's machinery existed with no tests and no callers.**
`dataset.rs`'s `backfilled_share`, `wholly_backfilled` and `fraction` were all three written and
unexercised. Now: `a_wholly_backfilled_dataset_is_flagged_with_its_fraction_rather_than_refused`
is the clause verbatim, `dropping_the_backfilled_flag_makes_a_derived_window_look_observed` is the
failure the whole task exists to prevent, driven rather than described, and
`an_exclusion_mask_does_not_rewrite_the_share_the_manifest_reports` covers the case where a
caller narrows the window and the reported fraction silently follows.

Also pinned: `BACKFILLED_KNOWLEDGE_TIME.0 == 0x8000` in `quality.rs`. Every other quality flag's
bit value was asserted somewhere and this one was not — which is the flag whose value decides
whether the v2 incident is detectable.

The failure this guards against already happened once: v2 stamped REST backfills with their
close time, so history looked observed-live. The canonical migration flagged them. This task
makes the property structural rather than remembered — a test that asserts, for any bar present
from both a live collector and a backfill, that the two rows are distinguishable by flag and by
`knowledge_time`, and that a dataset spec's `max_knowledge_lag_ms` picks up the larger of the
two.

**Files:** `crates/dataplane/src/canonical.rs` (assertions only), `crates/features` embargo
inputs, tests.
**Acceptance:** the test exists and fails if the flag is dropped; a dataset built over a mixed
window carries the maximum declared lag of its sources into `EmbargoInputs`; a dataset whose
rows are entirely backfilled is not refused, only flagged, and the manifest states the fraction.
**Traces:** ADR-P2-24; INV-01; D-P2, D-P3; PROGRESS "fabricated knowledge time".

### P-1.7 Run it, and prove the target — M
`NOT STARTED`

The ops task. Submit the backfill for BTC-USD and ETH-USD 1m/1h/1d across the deepest window
each venue serves, let it run to completion across restarts, then re-run the census (P-0.1) and
show the result on the coverage page (P-0.2).

Record what the free sources actually yielded. If three years is not available from any free
source, **that is the finding**, and it goes to the roadmap as a decision for the user (§9
gate: does ADR-0027 get revisited and costed?) rather than being quietly absorbed as a lower
target.

**Files:** none — an operations task with a written result in this phase file.
**Acceptance:** the census shows the achieved depth per instrument per timeframe; DATA-005
§10.3 either passes with evidence or the shortfall is quantified in days and escalated to the
roadmap's §8 decision gates; the run survived at least one worker restart.
**Traces:** DATA-005 §10.3, §11 S1; ROADMAP §4 Stage 0/1, §8.

---

## Phase exit

- The backfill is resumable, rate-limit-safe and idempotent, proven by tests rather than by a
  successful run.
- Backfilled rows are permanently distinguishable from observed rows.
- The census shows the achieved depth, and DATA-005 §10.3 is either met or quantified as a
  decision for the user.
