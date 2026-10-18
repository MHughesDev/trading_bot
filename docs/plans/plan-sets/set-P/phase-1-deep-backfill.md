# Phase 1 — Deep backfill

**Completion: 71% (5 / 7 tasks), plus 2 `PARTIAL` (P-1.3 archive files, `BLOCKED:HUMAN`; P-1.5 first CI run) — 2026-10-04 (track F-DATA-T): P-1.7 closed on the ADR rows below; earlier line: 57% (4 / 7 tasks), plus 3 `PARTIAL` — 2026-10-03: the deep backfill ran (P-1.7): 3.09 years of BTC-USD and ETH-USD 1m from Coinbase at 99.94 % coverage, graded D — depth is met, DATA-005 §10.3's grade is not, and that is a decision for the user.**

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
`DONE` 2026-10-03 (track DATA) — **all four clauses, the first on the live venue.**
Clause 1: a 30-day BTC-USD 1m window (2026-09-01 → 2026-10-01) was run through the production path —
`BackfillEngine` + `CoinbaseBackfill` + `BarStoreSink` + `PgBackfillState`, exactly as the platform's
`BackfillRouter` builds it — against `api.exchange.coinbase.com` and the live stores: 145 pages, and the
window holds **43,200 distinct minutes of 43,200**, no gaps. Clause 2: all 894 newly written rows carry
`BACKFILLED_KNOWLEDGE_TIME` and `knowledge_time = close + 60 s` exactly; the 2 rows whose values differed
from what was stored landed as `revision_seq = 1` with the observation instant as knowledge time — a
restatement, not an overwrite. Clause 3: `collectors/src/backfill/mod.rs::the_adapter_limiter_refuses_above_its_declared_rate`
and `dataplane/src/backfill.rs::the_limiter_refuses_rather_than_exceeding_its_declared_rate`;
`collectors/tests/coinbase_backfill_window.rs` (5 tests) walks a 30-day fixture venue with a planted hole.
Clause 4: `cargo xtask check-bars-v1-frozen` OK.
**Found while running it:** `BackfillEngine` passed the run's *cumulative* row count to
`BackfillStateStore::advance`, whose Postgres implementation *adds* it — the 145-page window above recorded
79,597 rows for 896 written. Fixed in `jobs/src/kinds/backfill.rs` (per-page delta), the trait documented,
both test doubles corrected to add, and `killing_the_worker_mid_window_refetches_no_page_and_skips_none`
now asserts `rows_written` across a restart. jobs: 104 tests green.

Earlier: `NOT STARTED`

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
`PARTIAL` 2026-10-04 (track F-DATA-T) — **`BLOCKED:HUMAN` for the archive files, and not fetched on purpose.** Kraken distributes the OHLCVT archives as a manual download from its support page ("Downloadable historical OHLCVT data"; a multi-gigabyte zip on Google Drive), and a download needs the owner's say-so (filename `Kraken_OHLCVT.zip`, source support.kraken.com, several GB). Exact remaining steps: (1) download it and unzip the `XBTUSD_1.csv`, `ETHUSD_1.csv` (and the 60-minute files) into one directory; (2) `sha256sum` each file and put the digests under `[checksums]` in `config/kraken_archive.toml` with `dir = "<that directory>"`; (3) restart the platform so the router registers `kraken_archive`. The path refuses any file with no declared digest and pauses on a mismatch (named tests), so nothing is inserted from an unverified file. Declaring checksums for files this track did not download would have been inventing them.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the archive path is constructible and routable; it stays off until an operator places files.**
Two defects stood between `KrakenArchiveBackfill` and a caller. It answered `source_name() =
"kraken_rest"`, the same as the OHLC adapter, so the router could never carry both; it now answers
`kraken_archive` — its own `SourceDecl` (`VendorFile`, 60 s, free) and `dataplane.source` row 23
(migration 0156). And nothing could build it from configuration: `KrakenArchiveBackfill::from_config_file`
reads `config/kraken_archive.toml` (dir + per-file sha256), returns `None` when no checksum is declared
(the shipped file), and refuses a malformed file rather than reading it as "off" —
`the_archive_path_is_configured_off_by_default_and_never_by_accident`,
`the_shipped_archive_config_parses_and_is_off`. Clauses 2 and 3 keep their named tests.
**Missing:** clause 1 for the archive path needs (a) the router registration in `apps/platform/src/main.rs`
(shared; exact edit in the DATA track report) and (b) the files themselves — Kraken distributes the OHLCVT
archives as a manual download, which this track did not perform (downloads need the user's say-so).

Earlier: `PARTIAL` 2026-09-28 — **the marker said `NOT STARTED` and two of three clauses have had named tests
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
`PARTIAL` 2026-10-04 (track F-DATA-T) — **the CI step was wrong and is corrected; its first CI run is the remaining evidence.** The step as written would have run nothing: `--test-threads=1` before `--` is not a cargo flag, the trailing names (`bar_store_pit clickhouse_`) are filters on test *function* names rather than test files, `--include-ignored` over whole crates would have run unrelated ignored suites, and `STORAGE_LIVE_READ_CLICKHOUSE_URL` would have made `canonical_read_live` fail on an empty CI ClickHouse (it reads the operator's backfilled 2026-09-15 day). Now: `cargo test -p backtest --test bar_store_pit` and `cargo test -p storage --test clickhouse_alt_pit --test clickhouse_forward_idempotent --test clickhouse_retention --test clickhouse_migrate`, each with `-- --test-threads=1` after the separator, with the four env vars those files read (`BACKTEST_E2E_`, `STORAGE_RETENTION_`, `STORAGE_ALT_`, `STORAGE_MIGRATE_CLICKHOUSE_URL`); the suites are not `#[ignore]`d, the env var is their switch. `qc_reports_live` and `canonical_read_live` are deliberately not in CI (they need the platform schema / live data) and are run by hand.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the suite ran against live ClickHouse and passed; CI still does not run it.**
`BACKTEST_E2E_CLICKHOUSE_URL=http://trading:trading@localhost:8123 cargo test -p backtest --test bar_store_pit`
on 2026-10-03 (ClickHouse 24.3, throwaway databases): **5 passed** —
`identical_recollection_writes_nothing_and_a_change_is_a_revision` (clause 1),
`live_revision_is_invisible_to_an_earlier_as_of` (clause 2),
`backfilled_history_is_flagged_and_invisible_before_its_sentinel`,
`venues_are_never_blended_and_timeframes_never_collide`, `pit_read_is_within_1_2x_of_a_raw_scan`. The same
property held on the production store during P-1.2's live window: two bars whose values had changed landed
as `revision_seq = 1` beside their originals. Clause 3 holds by inspection (no `OPTIMIZE`, no collapsing
engine on `market_bar`).
**Missing:** clause 4, *in the live ClickHouse suite* — no CI job starts a ClickHouse service or sets the
variable, so on CI the suite still self-skips. The job is written out in the DATA track report
(`.github/workflows/ci.yml` is not this track's file).

Earlier: `PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`
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
`DONE` 2026-10-04 (track F-DATA-T) — **both escalated questions decided (ADR rows below); the series re-graded B; the depth target is met on grade and the continuity shortfall is a quantified fact about Coinbase outages.** The nightly census now grades every series (`census_jobs::grade_series`), and the 2026-10-04 live pass wrote reports under config `09d9787685af8472`: **BTC-USD and ETH-USD coinbase 1m, 1h and 1d are all B** (not A: cross-source and survivorship cannot run on one venue, which is the design). `BTC-USD/kraken/1m` is D (91.6 % missing: a dead 13k-bar forward series) and `ETH-USD/kraken/1m` D (99.99 %); they are not what research reads.

**ADR-P1-7a: the >8σ spikes are real moves, not bad prints.** All 16 flagged spikes (10 BTC-USD, 6 ETH-USD) were checked against **Kraken's own trade tape** (public `Trades` endpoint, the minute of each spike): Kraken's high/low for the minute reproduced Coinbase's extremes in every case (for example 2024-09-05 21:35, BTC-USD: Coinbase 55,628–56,031, Kraken traded 55,601–56,085; 2026-01-31 18:43, BTC-USD: Coinbase low 75,678, Kraken low 76,400 from the first 1,000 trades of a 994-trade-capped minute), and every one sat on 4.5x–130x the rolling-median volume (`volume_confirm_multiple`; most above 10x). A bad print has no volume behind it. Rule: a reversing spike on a bar with volume ≥ 3× the rolling median is a traded move and is not counted (`config/data_qc.toml`, `bad_ticks.volume_confirm_multiple = 3.0`; structural impossibilities are never excused; tests `bad_ticks_excludes_a_spike_that_traded_heavy_volume`, `bad_ticks_structural_failures_ignore_volume`). Rejected: raising `sigma_multiple` (the smallest of the 16 is ~10σ, the largest ~14σ against a quiet 50-bar window, so any number that cleared them would clear real prints too) and deleting the check.
**ADR-P1-7b:** the volume-spike check fires on the *share* of bars that are spikes (`volume.max_spike_share = 0.01`), not on any single one: 0.28 % (BTC) and 0.39 % (ETH) of three years of 1m bars exceed 20× the median, and "any spike fails" is certain to fire on a million-bar series. The spike count is still reported and listed. Rejected: raising `spike_multiple` (hides the tail instead of bounding it).
**ADR-P1-7c: a no-trade minute is not a gap; a venue outage is.** Coinbase publishes a candle only for a minute with a trade. The 58 holes in three years of BTC-USD 1m: 50 are 1–12 minutes, the other eight are outages (391 min on 2026-05-08, 349 on 2025-10-25, 71, 67, …) which Coinbase's candle endpoint still does not serve (re-fetched 2026-10-04: 9–16 candles returned for windows of 70–390 minutes), so no backfill can fill them. `dataplane::census::NO_TRADE_TOLERANCE_SECS = 900`: a hole of at most 15 minutes is carried across by the continuity run (it still counts as missing in coverage and the grade); it is wall-clock, so it forgives nothing at 1h/1d. Rejected: counting every missing minute as a gap (reset the run at every quiet minute and reported 305 days for a series present for 3.09 years), and a tolerance long enough to absorb the outages (that would call a 6-hour venue outage no-trade). **Result:** BTC-USD 1m longest continuous run **363 days** (was 305), ETH-USD **194** (was 147); `strict_v1` (1,826 d) and `paper_v1` (365 d) are both still short for 1m. DATA-005 §10.3 ("≥ 3 years of continuous 1m history graded A or B") is therefore met on depth (3.09 years, 99.94 % present) and grade (B) and **not** on literal continuity: the eight Coinbase outages break it, which is a property of the venue and not of the pipeline. Decision for the owner if literal continuity matters: accept outage-bounded runs for 1m, or add a second venue's history as a separate series (never blended). 1h and 1d are unaffected (785 d and 1,128 d continuous; `paper_v1` met).

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **run to completion; depth is met, the grade is not, and that is the finding.**
**The run (2026-10-03, 18:04–19:06Z).** Coinbase Exchange, `coinbase_rest`, window 2023-09-01 →
2026-10-03, through the production engine (`BackfillEngine` + `CoinbaseBackfill` + `BarStoreSink` +
`PgBackfillState`) into the live `market_bar`: BTC-USD and ETH-USD at 1m (5,433 pages each), 1h and 1d.
**It survived restarts:** the BTC-USD 1m worker was SIGKILLed after 90 s with the cursor at
2023-11-19 02:39 (`backfill_state.status = running`), and the restarted run resumed from that cursor and
completed; the 1h windows were killed mid-run again (to pick up the page-span fix below) and resumed.
Kraken's OHLC endpoint serves only the latest 720 candles and its archives are a manual download
(P-1.3), Alpaca needs credentials (P-1.4) — so Coinbase is the only free source that yielded depth.
**The census after it (rows in `dataplane.data_coverage_census`, as_of 19:06Z):**

| Series | Bars | Expected | Missing | Longest continuous | strict_v1 (1826 d) | paper_v1 | Grade (`data_qc_reports`) |
|---|---:|---:|---:|---:|---|---|---|
| BTC-USD coinbase 1m | 1,623,311 | 1,624,320 | 0.06 % | 305 d | short 1,521 d | short | **D** |
| ETH-USD coinbase 1m | 1,623,269 | 1,624,320 | 0.06 % | 147 d | short 1,679 d | short | **D** |
| BTC-USD / ETH-USD coinbase 1h | 27,062 | 27,072 | 0.03 % | 785 d | short 1,041 d | **met** | C |
| BTC-USD / ETH-USD coinbase 1d | 1,128 | 1,128 | 0 | 1,128 d | short 698 d | **met** | B |

**DATA-005 §10.3 — "≥ 3 years of continuous 1m history graded A or B" — is not met, and the shortfall
is not depth.** 3.09 years of 1m is present at 99.94 % coverage. What fails is (a) *continuity* as the
census defines it (no gap longer than one period): Coinbase publishes no candle for a minute with no
trades, so ~1,000 venue-absent minutes cut the longest run to 305 / 147 days; and (b) the *grade*: the
battery grades both 1m series **D** for 10 (BTC) / 6 (ETH) price spikes beyond 8σ that reverse ≥ 80 % on
the next bar, plus 4,571 / 6,319 volume spikes over 20× the rolling median (C on its own). Both are
decisions for the user, not thresholds to loosen here: whether a no-trade minute is a gap or a
zero-volume bar, and whether those spikes are venue bad prints to repair (→ C) or real moves the
8σ/20× defaults misjudge at 1m. Strict_v1's five-year floor is further away still: Coinbase serves BTC-USD
from 2015, so depth for Gate 9 is *available* free; the continuity rule, not the source, is the binding
constraint. **Escalation to ROADMAP §8 is for the orchestrator** (ROADMAP.md is not DATA's file).
**Found while running it:** the engine paged every timeframe in the policy's fixed 5-hour slices — a
three-year 1d window cost ~5,400 requests for 1,128 bars. `BackfillSource::max_page_span` now lets an
adapter declare its cap (Coinbase/Kraken/Alpaca do); the same windows took 66 / 4 pages
(`the_engine_pages_by_the_span_the_venue_declares`). And the `rows_written` triangle-sum fix (P-1.2).
**Missing:** the coverage page (P-0.2) to show this, and the ROADMAP §8 entry.

Earlier: `NOT STARTED`

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
