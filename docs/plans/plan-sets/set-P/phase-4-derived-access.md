# Phase 4 — Derived access and the source fence

**Completion: 100% (5 / 5 tasks) — 2026-10-04 (track F-DATA-T): P-4.1 and P-4.2 closed; earlier line: 60% (3 / 5 tasks), plus 2 `PARTIAL` — 2026-10-03: P-4.3 was closed by Set U's selection runs, P-4.4 is enforced by migration 0155 and an xtask guard, and the provenance read P-4.1/P-4.2 were blocked on now exists.**

**Requirement IDs:** DA-08, DA-10, DA-14.
**Spec:** [DATA-005 §8 (universes), §11 (sources), §12](../../../specs/DATA-005-data-api-v2.md);
ADR-0027.
**Depends on:** Phase 1 (bars to resample), Phase 2 (grades, so a derived series inherits one).
**Blocks:** nothing in this set.
**Migration range:** 0104.
**Migrations shipped:** 0104 (`cost_class` plus the universe event log, 2026-09-28).

Two unrelated things, both small, both belonging at the end: making stored bars reachable at
whatever resolution a study needs, and making the free-only policy a mechanism instead of a
habit.

---

### P-4.1 Server-side resampling — M
`DONE` 2026-10-04 (track F-DATA-T) — **reachable, and the grade it needs now exists.** `GET /api/data/bars/derived` is registered (`routes/mod.rs`); `resample` refuses a series with no persisted grade, and the 2026-10-04 re-grade (P-1.7) gave every Coinbase 1m/1h/1d series a persisted B, so the route returns data for them (before, every derived read was refused as ungraded). The live read matches the venue's own 1h bars on 24 of 24 (`canonical_read_live`); worst-grade/largest-lag and the refusal to go finer have their `resample.rs` tests.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the blocker named below is removed; the route is written and awaits registration.**
`storage::clickhouse::canonical_read::canonical_bars_as_of` returns `CanonicalBar` **with `source_id` and
`vwap`**, point-in-time exactly as `BarStore` resolves it (knowledge_time <= as_of, whole-row `argMax` by
`(knowledge_time, revision_seq)` so no restatement is assembled from two revisions), and `provenance` looks
up each source's declared lag in `dataplane.source` and the series' persisted grade in `data_qc_reports` —
returning nothing for an ungraded series, so `resample` refuses rather than this layer inventing a letter.
`GET /api/data/bars/derived?tf=` drives `dataplane::resample::resample`; a refusal is a 422 with the reason.
`derived_tests` pins the request parsing (exactly one derivation; unknown timeframes refused).
**ADR (DATA, 2026-10-03):** the provenance read lives in `storage::clickhouse` rather than extending
`backtest::store::LoadedBar`, which is outside this track; it repeats BarStore's PIT resolution in one
query, and the two must agree. Rejected: inventing `source_id`/grade in the route.
Clause 1 on the live store through this read: `storage/tests/canonical_read_live.rs` (read-only) resamples 2026-09-15's 1,440 Coinbase BTC-USD 1m bars to 1h and matches all 24 of Coinbase's own 1h bars on open/high/low/close, and the same read as of 2026-09-01 returns nothing. **Missing:** route registration (report).

Earlier: `PARTIAL` 2026-09-28 — **every acceptance clause is verified and the code has no caller**, which in
this repository is the failure mode worth naming rather than a technicality.

`crates/dataplane/src/resample.rs` (23 tests): a 1m to 1h resample agrees bar for bar with the
venue's own 1h series, nested grids agree on the extremes they share, a derived series inherits the
**worst** grade and the **largest** lag of its inputs, a D-graded input cannot be washed out by
aggregation, a source that declared no grade is refused rather than assumed clean, and upsampling
and a non-nesting coarser period are both refused rather than interpolated.

Two design decisions recorded because a later reader would otherwise call them bugs. A trailing
bucket the input does not cover is **dropped** — that is what makes the venue comparison valid,
since a venue does not publish the hour it is still in — while an *interior* venue gap is kept and
flagged, because dropping it would turn a 4-minute hole into a 60-minute one. And the output is a
`ResampledBar`, not a `CanonicalBar`, deliberately: handing back a canonical bar invites a caller to
insert one, and a read-time derivation has no `source_id`, `ingest_time` or `revision_seq` to be
honest about.

**The blocker for the route, precisely.** `resample` requires a `SourceProvenance` for every
distinct `source_id` in its inputs, and refuses a source that declared no grade — which is the whole
of clause 2 and is right. `backtest::store::BarStore::load_bars` returns `LoadedBar`, which carries
`ts_ns`, `open_ns`, `knowledge_ns`, OHLCV, `trade_count` and `quality_flags` and **no `source_id`
and no `vwap`**; the `market_bar` SELECT in `read_facts` does not project either. So a route written
today would have to invent a `source_id` and a grade, which is exactly the laundering step clause 2
exists to prevent. What is needed first: `source_id` and `vwap` added to `LoadedBar` and to both SQL
variants in `read_facts` (the fast path and the restatement-resolved path), and a grade-plus-lag
lookup per source. That is `crates/backtest/src/store.rs`, not this file.

Any stored timeframe resampled to any coarser timeframe at read time, computed once in the
data layer rather than by every caller. The resampled series inherits the **worst** grade and
the **largest** declared lag of its inputs — a derived series is never cleaner than what it was
derived from, and the arithmetic must be that way round or resampling becomes a laundering step.

Resampling is bar-convention-sensitive. The canonical timestamp rule (DATA-005 §3) applies, and
a test asserts a 1m→1h resample agrees bar-for-bar with the venue's own 1h series where both
exist.

**Files:** `crates/dataplane/src/resample.rs`, `crates/api/src/routes/data.rs`.
**Acceptance:** 1m→1h resampling matches the venue's own 1h series on an overlapping window; the
derived series carries the worst input grade and largest input lag; resampling to a *finer*
timeframe is refused rather than interpolated.
**Traces:** DA-10; DATA-005 §3, §7.

### P-4.2 Information-driven bars — S
`DONE` 2026-10-04 (track F-DATA-T) — **reachable through the same registered route** (`?kind=volume|dollar|tick&threshold=`) with the same persisted grades; a request without a threshold, and a metric the source does not supply (Coinbase has no trade count), are stated refusals (422), not empty series.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **reachable through the same route as P-4.1** (`?kind=volume|dollar|tick&threshold=`), with the same
provenance read; a request without a threshold is refused rather than defaulted
(`an_information_bar_needs_its_threshold_stated`), and an absent input metric (Coinbase's trade count) is the
generator's stated refusal, returned as a 422. **Missing:** route registration (report).

Earlier: `PARTIAL` 2026-09-28 — all three acceptance clauses verified in
`crates/dataplane/src/bars/information_driven.rs` (21 tests); no caller, for the same reason as
P-4.1 and with the same fix.

Volume, dollar and tick bars, each recording the threshold it closed on; regenerating with the same
threshold is byte-identical, and the hash distinguishes two kinds that happened to produce the same
bars.

**Clause 3's case is live rather than hypothetical, which is why it is worth its own note.**
Coinbase's candle endpoint reports no trade count, so its adapter writes `0`. Tick bars over that
window would have returned an empty series reading identically to *this instrument did not trade*.
The refusal names the metric, the source, and what would supply it.

Volume, dollar and tick bars where the inputs exist. These are sampled on information rather
than on the clock, which is the point — clock bars oversample quiet periods and undersample
the ones that matter.

Each generator declares its threshold and records it with the series, so a bar set is
reproducible. They are read-time derivations like P-4.1, not a second stored bar table.

**Files:** `crates/dataplane/src/bars/information_driven.rs`.
**Acceptance:** volume, dollar and tick bars are produced from stored trades or bars with the
threshold recorded; regenerating with the same threshold is byte-identical; a request for a
generator whose inputs are absent returns a stated reason rather than an empty series.
**Traces:** DA-10; 06_DATA §2.

### P-4.3 As-of universes — M
`DONE` 2026-10-03 (track DATA) — **clause 2 was closed by Set U and nobody moved this marker.** Experiment evaluation resolves universes
through `dataplane::universe::members_known_as_of` (`strategy_engine::env::Membership::members_at`, loaded by
`backtest::env::universe::load`), and
`backtest/tests/selection_run.rs::as_of_a_past_date_the_run_sees_that_dates_membership_and_data_only` is the
clause verbatim: the same log gives one member on day 5 and two on day 10, and the day-5 answer is unchanged
by later bars. `backtest/tests/pg_universe.rs` loads a named universe from the append-only table. Clauses 1
and 3 keep their `universe.rs` tests and 0104's insert-only trigger. Added: `GET /api/data/universe/{name}?as_of=`
(`routes/data.rs::get_universe`, the same loader, `as_of` clipped to the horizon, each member labelled with
its listing `status` so a delisted member is shown rather than dropped) — registration in the report.

Earlier: `PARTIAL` 2026-09-28 — clauses 1 and 3 verified, clause 2 needs the experiment evaluator.

**The write model this task sketches cannot satisfy its own acceptance, so it was not built.**
`(name, instrument_id, valid_from, valid_to, reason)` fails clause 3 by construction: the `UPDATE`
that closes `valid_to` destroys the answer a previous study got, which is the survivorship bias this
task exists to prevent, one level up. One row per `(universe, instrument)` also cannot represent an
instrument that left an index and rejoined it. So `crates/dataplane/src/universe.rs` (15 tests) is
an append-only `MembershipEvent` log, and intervals are a derived read model where every `valid_to`
is computed. Migration 0104 ships `dataplane.universe_membership_event` with an insert-only trigger,
because *never overwritten* enforced only by the application layer is a rule the next migration
drops.

Two clocks, both required: `effective` is when the change took effect in the world, `recorded_at`
when this platform learned of it. An index change announced on the 1st and effective on the 15th is
knowable from the 1st, and `members_known_as_of` is the function anything a study reads must call —
`members_as_of` alone would let an announced-but-not-yet-recorded change reach a backtest.

`QcContext::resolve_membership` folds a log into `Membership::Historical`, so `qc::survivorship`
stops answering `InsufficientMetadata` for every universe. Before it, the only
`Membership::Historical` anywhere in the workspace was one typed by hand in that file's own test
helper.

**What keeps it `PARTIAL`:** clause 2 — an Experiment declaring a universe is evaluated on as-of
membership, proven by a test where the two answers differ. Experiment evaluation does not call
`members_known_as_of`, and the test the clause asks for needs the two answers to differ, so it
cannot be written against a caller that does not exist.

`universes` and `universe_membership` (migration 0104: name, instrument_id, valid_from,
valid_to, reason) and `GET /api/data/universe/{name}?as_of=` returning membership valid at
`as_of`. Universe-based Experiments declare a universe and are evaluated on as-of membership.

**Files:** `crates/dataplane/src/universe.rs`, `crates/api/src/routes/data.rs`, migration 0104.
**Acceptance:** membership as of a past date excludes instruments added later and includes ones
since removed; an Experiment declaring a universe is evaluated on as-of membership, proven by a
test where the two answers differ; `valid_to` is never overwritten — a removal is a new row.
**Traces:** DA-08; DATA-005 §8.

### P-4.4 Delisted instruments are retained, not deleted — M
`DONE` 2026-10-03 (track DATA) — **all three clauses, with the rule in the database and not only in review.** Migration **0155**:
`instruments` gains `status ('active'|'delisted')`, `delisted_at`, `delisting_reason`, with a CHECK tying
them together (a delisted row has a date and is not `active`); a row trigger refuses DELETE and a statement
trigger refuses TRUNCATE for every role including the owner; `app_role` loses DELETE/TRUNCATE;
`delist_instrument(id, at, reason)` is the one way to delist (keeps the first date, refuses an empty reason).
Exercised on a Postgres 17 scratch DB, applied twice: owner DELETE and TRUNCATE refused by the trigger,
`app_role` DELETE refused by privilege, an inconsistent UPDATE refused by the CHECK, `app_role` delists
through the function, and `active`-only listings exclude the delisted row while the table keeps it.
Clause 2: universe membership as of a past date includes members since removed (`universe.rs`; P-4.3), and
`get_universe` labels them `delisted`. Clause 3: `cargo xtask check-instruments-retained`
(`xtask/src/instruments_retained.rs`, 5 tests) scans every tracked `.rs/.sql/.py/.ts` file plus untracked
migrations for an instrument DELETE/TRUNCATE/DROP, or anything that drops 0155's guard or re-grants DELETE;
`the_workspace_has_no_instrument_deletion_path` runs it inside `cargo test -p xtask`. Mutation-checked: a
probe migration containing `DELETE FROM instruments` fails it. `dataplane.instrument` already refused
deletion (0046).

Earlier: `NOT STARTED`

Dead and delisted instruments stay in `instruments` with their full history and
`status='delisted'`. This is the only defence against survivorship bias that actually works,
and it is a deletion policy, not an analysis technique: once the row is gone the bias is
unrecoverable.

Pairs with P-2.5, whose survivorship check reads exactly this history.

**Files:** `crates/storage/src/postgres/instruments.rs`, migration 0104.
**Acceptance:** delisting sets `status` and retains every bar; a query for a historical
universe returns instruments now delisted; a test asserts no code path deletes an instrument
row, and the static invariant suite fails the build if one is added.
**Traces:** DATA-005 §8; DA-08; P-2.5.

### P-4.5 The free-only fence, as a CI check — M
`DONE` 2026-09-28 — all four clauses, and the check found two undeclared sources on its first run.

`crates/dataplane/src/source.rs`: `CostClass` has no `Default` and `SourceDecl.cost_class` is
required, so *a source with no cost_class fails to compile* is now literally true;
`CostClass::parse` refuses the empty string rather than reading it as free. `PAID_SOURCE_ALLOWLIST`
is empty, and `check_cost_class_against` is parameterised over an allowlist for one reason: a test
that could only ever exercise the refusal would prove the fence is *closed* rather than that it has
a gate.

Migration 0104 adds `dataplane.source.cost_class TEXT NOT NULL` with a `CHECK` restricting it to
free or paid, and **no DEFAULT**, which is the mechanism rather than a detail of it: a
`DEFAULT 'free'` would undo both halves of the crate-side refusal at once. Every shipped source is
declared free one statement at a time, so adding a source means adding a line that says which it is.

`cargo xtask check-free-sources` runs in CI beside `check-bars-v1-frozen` and
`check-bars-v2-frozen`. It **calls** `dataplane::source::check_declared_cost_classes` rather than
restating the predicate — that is the only dependency `xtask` has, and it is there so this job and
the crate cannot disagree about what free means. The two halves are deliberate and stated in the
job's own comment: the CHECK stops an inadmissible *value*, and the task stops an undeclared
*source*, which the CHECK cannot see because a source inserted without the column never reaches it.

**Exercised, not merely written.** 0104 was applied to a throwaway database (PostgreSQL 16) and each
mechanism driven: an `INSERT` omitting `cost_class` fails the NOT NULL, `'freemium'` fails the CHECK
by name, `'paid'` is accepted *by the column* — correctly, because the ADR gate is the xtask half and
not the database's job — `information_schema` confirms `column_default IS NULL`, and `UPDATE` and
`DELETE` on `universe_membership_event` are both refused by the trigger with the row still present
afterwards. Re-running the whole file is clean, and it does **not** rewrite a source already declared
`paid` back to `free`, because every declaration carries `AND cost_class IS NULL` — a re-run cannot
launder a paid source into a free one.

**Not yet applied to the platform's database.** Docker was down, and the server answering on the
port the repo's `DATABASE_URL` names turned out to be a different PostgreSQL holding migrations 1-35
against the 140 on disk — it accepts the platform's credentials, which is exactly what makes it
misleading. 0104 is written and validated; applying it is a `sqlx migrate run` against the real
database once it is up.

**Two findings from building it.** The check's first run failed on `kraken_futures_rest` and
`kraken_futures_history` — two sources 0098 added that migration 0104's first draft did not classify,
which is exactly the failure mode the task is for, caught on the day the mechanism landed. And the
first version of the scanner's statement splitter was `find(';')`; `dataplane.source` row 1's note
contains a semicolon, so it ended the 0046 insert after its first row and reported four sources over
a table holding nineteen. A guard that silently checks a quarter of its subject is worse than no
guard, because the number it prints reads like a measurement. `statement_end` now honours string
literals and doubled-quote escapes, pinned by nine tests including one over the real migrations that
fails if the found count drops below nineteen.

DA-14 says no paid source may be added without revisiting ADR-0027. Today that is a convention.
This makes it a mechanism: every source is declared in `dataplane.source` with a `cost_class`
that has **no default**, and an `xtask` check fails the build when a source declares anything
but `free` unless an allowlist entry names the ADR that approved it.

The point is not to forbid paid data — it is to make adding it a decision someone signs rather
than a dependency that appears.

**Files:** `xtask/src/check_free_sources.rs`, `crates/dataplane/src/source.rs`, migration 0104,
CI workflow.
**Acceptance:** adding a source with `cost_class = 'paid'` fails CI; adding one with an
allowlist entry naming an ADR passes; a source with no `cost_class` fails to compile or fails
the migration's CHECK, not the review; the check runs in the same CI job set as the other
`xtask` guards.
**Traces:** DA-14; ADR-0027; D-P1.

---

## Phase exit

- Any study can get the resolution it needs without a caller reimplementing resampling, and a
  derived series never looks cleaner than its inputs.
- Universe membership is point-in-time and delisted instruments are retained forever.
- A paid source cannot enter the platform without an ADR, enforced by CI.
