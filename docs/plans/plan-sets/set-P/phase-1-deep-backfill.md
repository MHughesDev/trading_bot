# Phase 1 — Deep backfill

**Completion: 0% (0 / 7 tasks)**

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
`PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`.**
`impl Worker for BackfillWorker` exists, `JobKind::Backfill` is in
`JobKind::ALL`, migration `0099` is applied, `dataplane.backfill_state` is live,
and `jobs/tests/backfill_resume.rs` counts adapter calls. **The gap is one
registration:** `apps/platform/src/main.rs` registers GateAdvance, DataQc,
SimulatePaths, Campaign, ResearchRun and Session — **and no backfill worker** —
so a submitted backfill job queues for ever. This is the same shape as the
reconciliation sweep and the trials route: the caller built, the callee
registered nowhere.

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
