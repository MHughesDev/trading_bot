# Phase 0 — Bar schema v2 and verified cutover

**Completion: 100% (9 / 9 tasks) — PHASE COMPLETE 2026-09-11**

> **Amended the same day, after running in production.** Two defects surfaced once
> the platform was actually booting against the migrated table, and both are fixed
> with regression tests:
>
> 1. **The engine was wrong.** Plain `MergeTree` never collapses, so routine
>    re-collection (gap fill re-reads ranges on every boot) accumulated forever —
>    1,688 exact repeats within hours. v2 is now
>    `ReplacingMergeTree(ingested_time)` keyed on
>    `(instrument_id, timeframe, venue_id, source, event_time, revision)`. v1's bug
>    was an *incomplete key*, not a collapsing engine; with every distinguishing
>    dimension in the key, only genuine repeats can merge. An online migration
>    (`ensure_market_bars_v2_engine`) rebuilds and atomically swaps the table,
>    archiving the old one rather than dropping it.
> 2. **The backfill looped.** It fingerprinted full row content including
>    `ingested_time`, so a merged-away repeat read as a missing row and was
>    re-copied — 1,680 rows on every boot. Identity is now the sorting key. The
>    boot-time backfill also aborted on a healthy table, because v2 legitimately
>    holds more rows than v1 after cutover; it now compares row-by-row instead of by
>    totals.
>
> Verified on the live table: 102,113 rows and 102,113 distinct observations — no
> duplicates — with the 18 rescued collisions intact.

- **L-0.1 done.** `clickhouse/06_market_bars_v2.sql` written and validated against a
  live ClickHouse 24.3 in a throwaway database, including a demonstration that the
  schema fixes the collision (see below).
- **L-0.2 done.** `crates/storage/src/clickhouse/migrate.rs` replays the embedded DDL
  on boot, called from `apps/platform/src/main.rs` after the Postgres migrations.
  `market_bars_v2` now exists in the live `trading` database (empty; `market_bars`
  untouched at 99,789 rows). 6 unit tests + 2 gated integration tests green, clippy
  clean.

  Two things this turned up:
  - **`storage::clickhouse::connect` dropped the credentials and the database from the
    URL** — it was `Client::default().with_url(url)`, and the `clickhouse` crate does
    not parse those out. Anything using it would have written to `default.default`
    while looking like it worked. It had no callers, so nothing was broken in
    practice; it is fixed and now mirrors `backtest::store::BarStore::connect`, with a
    gated test (`connect_honours_database_in_url`) pinning the behaviour.
  - The statement splitter must strip `--` comments **before** splitting on `;`,
    because `06_market_bars_v2.sql` documents the canonical `argMax` read shape in a
    comment that ends in a semicolon. `--` inside a string literal
    (`DateTime64(9, 'UTC')`) must survive. Both cases are regression-tested.
- **L-0.3 done, and the migration has been run on the live database.** 99,789 rows
  copied from `market_bars_rescue_20260911` into `market_bars_v2` across 7
  `(timeframe, month)` chunks. Verified afterwards:

  | Check | Result |
  |---|---|
  | Row count, v1 / rescue / v2 | 99,789 / 99,789 / 99,789 |
  | Per-timeframe counts | BTC 1h 22, BTC 1m 99,717, ETH 1h 48, ETH 1m 2 — unchanged |
  | `OPTIMIZE TABLE market_bars_v2 FINAL` | 99,789 rows, **all 18 collisions intact** |
  | `sipHash64` over all 18 shared columns, rescue vs v2 | identical (`17230036672739449194`) |
  | Derived times, `BTC-USD 2026-05-31 06:00` | 1h `bar_open_time` 05:00, 1m 05:59, both `event_time` 06:00 |
  | Re-run, from the snapshot and from `market_bars` | 0 rows copied, 7 chunks already present |

  **The forced merge is the headline:** the same operation that destroys 18 bars in
  v1 leaves v2 untouched. `market_bars` was not modified.

  Two design points worth keeping:
  - `available_time` is **carried over, not recomputed**. v1 set it to the bar's close
    (`backtest::collect` computes `open_time + timeframe` at every call site), and it
    never recorded collection lag. Deriving a new one would fabricate provenance.
  - An unknown timeframe **aborts the whole backfill before any write**, rather than
    defaulting to a duration. A guessed `bar_open_time` would be wrong and plausible,
    which is the worst combination.
- **L-0.4 done.** Verification is now part of the migration rather than something a
  human remembers to run: `backfill_market_bars_v2` calls it before reporting success,
  because a copy that moved the wrong rows is worse than one that never ran — it looks
  finished, and L-0.7 retires the source on the strength of it.

  **The invariant is containment, not equality**, and that choice is load-bearing.
  Right after the backfill the two tables match exactly, but they then drift apart in
  one direction forever: `market_bars` is the `ReplacingMergeTree` that *destroys*
  rows on merge, which is the entire reason v2 exists. An equality check would
  therefore start failing on a perfectly healthy system the first time v1 lost a bar,
  and whoever saw that alarm would learn to ignore it. So the check asks the question
  that stays meaningful: **is any row of the source absent from v2?**
  `verification_tolerates_v2_being_a_superset` pins this.

  A count comparison would pass a copy that moved the right *number* of rows with the
  wrong content, and a summed hash can collide, so the check is a row-level anti-join
  on a per-row fingerprint. `verification_fails_when_a_row_is_missing_from_v2` deletes
  a row from v2 and asserts the failure — a verifier that has never rejected anything
  is decoration.
- **L-0.5 done — as a dual-write, which is a deliberate deviation from this task as
  written.** The task said "move the writer" to v2. Doing exactly that would have
  broken the system for the length of one task: the readers in this module still query
  `market_bars` until L-0.6, so a writer that switched ahead of them would make every
  freshly collected bar invisible to every backtest in between. The migration is safe
  in the standard order instead — **write both, migrate readers, then stop the old
  write** — so L-0.7 now also removes the v1 write, and the system is correct at every
  point rather than only at the end of the phase.

  v1 therefore keeps losing the occasional coarse bar to a merge until L-0.7. That is
  unchanged from today's behaviour and is no longer consequential, because v2 holds the
  truth from this point on.

  `crates/backtest/tests/bars_v2_writer.rs` drives the real writer against a live
  ClickHouse and asserts the v2 rows are correct, not merely present: derived
  `bar_open_time` (05:00 for the 1h bar, 05:59 for the 1m), `event_time` equal to the
  collector's close, `available_time` equal to `event_time` (v1 never measured a
  collection lag, so inventing one would be fabricated provenance), and the
  `Decimal128` volumes round-tripping. It then merges **v1** and asserts it collapsed
  to a single row — so the premise of this whole phase is tested rather than assumed.
- **L-0.6 done.** All five bar readers moved to `market_bars_v2`. Two decisions:

  **The PIT horizon belongs to the store, not to each query.** DA-02 wants every read
  filtered on `available_time <= as_of`, but these readers have 13 call sites across
  `api`, `model-registry`, `backtest` and `platform`, and threading an `as_of`
  parameter through all of them is really Phase 2's work (the cutoff comes from the
  research project). So `BarStore` carries the horizon: `connect()` reads as of *now*,
  and `as_of(t)` returns a store that reads the world as it stood at `t`. That matches
  how DATA-005 actually models the cutoff — as a property of the reader's authority,
  not of the question — and Phase 2 wires the project cutoff into one constructor
  instead of editing every call site.

  The default is not a no-op dressed up as enforcement: `as_of = now` means no caller
  can read a bar before the instant it became available, which is a real guarantee the
  v1 readers never made. `as_of_hides_bars_that_were_not_yet_available` proves the
  mechanism hides data, and deliberately checks the **aggregate** readers too
  (`daily_counts`, `list_coverage`, `last_bar_time`) — a cutoff that leaks through a
  count is not a cutoff.

  **Revisions resolve one way everywhere.** The readers previously used
  `argMax(x, revision)`, which has no tiebreak between two rows of the same revision —
  exactly the ambiguity that made the v1 collapse non-deterministic. They now all use
  `argMax(x, (revision, ingested_time))`, the expression documented in the DDL.

  The existing `e2e.rs` needed a fix as a consequence: it assumed the tables already
  existed in whatever database it pointed at, and silently depended on `market_bars`
  being there. It now applies the schema through the migrator first.

  Read back from the live database through the new shape: BTC-USD 1h reports all 22
  bars, and the 06:00 hourly bar — one of the 18 that v1 had queued for destruction —
  reads back with its own open (74054) and volume (75.0697), not the minute bar's.
- **L-0.7 done — cutover completed 2026-09-11.** `market_bars` receives nothing
  further. The bleed is stopped.

  **Read-only is enforced in CI, not in ClickHouse, and that was forced.** The
  intended enforcement was `REVOKE INSERT ON trading.market_bars`, which fails:

  ```
  Code: 495. Cannot update user `trading` in users_xml because this storage is readonly.
  (ACCESS_STORAGE_READONLY)
  ```

  The ClickHouse user is defined in `users_xml`, so SQL cannot alter its grants — and
  even if it could, the grant would not survive a container rebuild, which on this box
  happens routinely. A CI check does survive, and it fails at the moment someone
  reintroduces the write rather than months later when a merge eats the bars. So
  `cargo xtask check-bars-v1-frozen` joins `check-money-f64` and
  `lint-no-json-hotpath` as a source guard, wired into `.github/workflows/ci.yml`.
  Reads of v1 are still allowed; only inserts are forbidden.

  **The guard paid for itself immediately** by failing on
  `crates/storage/src/clickhouse/bars.rs` — a dead module, unreferenced anywhere, whose
  schema no longer matched the table (it wrote a non-existent `event_time_us` column and
  omitted eight required ones). It was deleted. That file had been sitting there as a
  loaded gun: the next person to reach for "the ClickHouse bars insert helper" would
  have found it, and it writes to the table that destroys bars.

  `bars_v2_writer.rs` now asserts **both** halves of the cutover: the writer puts
  nothing in v1 (0 rows), and v1 — when written directly — still collapses the two
  bars into one. Keeping the second assertion means that if the v1 schema is ever
  fixed independently, the test fails and this phase gets revisited, instead of
  quietly carrying a stale justification.
- **L-0.8 done.** The regression test makes "it fails against v1" a fact the suite
  establishes rather than a claim this document makes. One property is defined once
  and run against both tables; v1 is asserted to fail it, v2 to pass it. The test
  prints what it saw:

  ```
  v1 failed the property, as designed: market_bars destroyed a bar:
    expected both 1h and 1m to survive the merge, found ["1m"]
  v2 satisfied the property
  ```

  It also seeds each bar in its **own** insert batch on purpose. A single batch would
  destroy the coarse bar before any merge — a harsher failure that would not exercise
  the merge path this test guards.

  Surviving is not the whole property: the test also checks the two rows keep their
  own payloads (volumes 75.0697 and 1.4108), because two rows where one has
  overwritten the other would pass a naive count.

- **L-0.9 done. Phase 0 complete, and the backfill is unblocked.**

**Goal:** Stop `market_bars` from destroying bars, permanently. Replace it with an
append-only `market_bars_v2` that carries `timeframe` in its sorting key and retains
revisions, move every writer and reader onto it, and prove row-for-row that nothing was
lost in the move.

**Requirement IDs:** DA-16 (append-only schema, `timeframe` in the key, revisions
retained), DA-17 (verified cutover, writers moved before any backfill, `market_bars`
read-only).

**Spec:** [DATA-005 §3](../../../specs/DATA-005-data-api-v2.md)
**Depends on:** nothing. This phase is independent of the rest of Set L and should start
immediately.
**Blocks:** the deep-history backfill (DATA-005 §11 S1), Set L Phase 2 reads, and any
`OPTIMIZE` of `market_bars`.

---

## Why this is Phase 0

Measured on the live table, 2026-09-11, with read-only queries:

| Measurement | Value |
|---|---|
| Rows, raw | 99,789 |
| Rows after a full merge (`FINAL`) | 99,531 |
| Benign exact-duplicate keys | 240 |
| **Cross-timeframe collisions** | **18** |
| **BTC-USD 1h bars at risk** | **18 of 22 (81.8%)** |

The 4 surviving 1h bars are exactly those dated before 1m coverage begins
(2026-05-16 00:48). **Every 1h bar that overlaps 1m coverage collides.** The colliding
rows hold genuinely different data — for `BTC-USD 2026-05-31 06:00` the 1h bar is
`o 74054.00 / h 74100.72 / l 73939.99 / v 75.0697` against the 1m bar's
`o 73961.99 / h 73961.99 / l 73947.01 / v 1.4108`. Only `close` coincides, because an
hour's close *is* its last minute's close — which is precisely what makes the collision
invisible to a spot check. Both rows carry `revision = 0`, so
`ReplacingMergeTree(revision)` has no tiebreak; in practice `FINAL` keeps the 1m row and
the hourly bar dies.

The rows survive today only because ReplacingMergeTree collapses *within a part*, and
the colliding rows sit in two different parts of partition `202605`
(`202605_1_402_15` and `202605_403_407_1`).

**Demonstrated during L-0.1, in a throwaway database.** The same colliding pair was
inserted into a v1-shaped control table and into `market_bars_v2`:

| Table | Write pattern | Rows before merge | Rows after `OPTIMIZE … FINAL` | Survivors |
|---|---|---|---|---|
| v1 control | both bars in **one** insert batch | **1** | 1 | `1m` only |
| v1 control | bars in **two** insert batches | 2 | **1** | `1m` only |
| **v2** | either pattern | 2 | **2** | `1m` and `1h` |

Two things follow, and the first was not known when this phase was written:

1. **When both timeframes arrive in the same insert batch, the coarse bar is lost
   immediately — no merge required.** The 18 rows that survive on the live table do so
   only because the 1m and 1h bars happened to arrive in different batches. Loss is not
   merely deferred; it is deferred *by luck*.
2. This sharpens the backfill ordering constraint to a hard one. A backfill that writes
   several timeframes for an instrument in one batch would destroy the coarse bars
   instantly and silently, with nothing in the logs and no row to recover. **The v2
   cutover must precede the backfill** (DA-17).

**They have been rescued** (see the README in the `trading_bot_backups/` directory
alongside the repo — deliberately outside it, so it is not a link here): table
`trading.market_bars_rescue_20260911` plus a `Native` export verified to restore
byte-identical. So this phase is no longer an emergency — but it is still first,
because the backfill writes exactly the bars that collide.

---

## Design notes

**Append-only, not replacing.** The v2 engine is plain `MergeTree`. Nothing collapses,
so no sorting-key mistake can ever destroy a row again. Deduplication becomes a *read*
concern, which is the right place for it: a read can be fixed, a merge cannot be undone.

**Revisions are kept, not overwritten.** The v1 comment "latest revision wins after
merge" destroys exactly the history that `as_of` queries need (DATA-005 §3 item 2). In
v2 a revision is another row, and a read picks one.

**Two time columns, deliberately.** `event_time` is the bar's close and is what a
researcher means by "the 14:00 bar". `available_time` is when the platform could first
have known it, and is what the cutoff and PIT filters compare against. Conflating them
is how lookahead gets in.

**Sorting key.** `(instrument_id, timeframe, event_time, revision, ingested_time)` per
DATA-005 §3. `venue_id` and `source` are *not* in the key: with no collapsing engine
they cannot cause loss, and leaving them out keeps the key short. The rescue table used
a wider key only because it was a belt-and-braces snapshot.

**Latest-as-of read shape**, for every consumer:

```sql
SELECT argMax(open, (revision, ingested_time)) AS open, ...
FROM market_bars_v2
WHERE instrument_id = ? AND timeframe = ? AND available_time <= {as_of}
GROUP BY event_time
```

or the cheaper `LIMIT 1 BY (instrument_id, timeframe, event_time)` with an
`ORDER BY revision DESC, ingested_time DESC`. Pick one and use it everywhere; a mix is
how two call sites end up disagreeing.

---

## Tasks

| Task | Req | Description |
|---|---|---|
| **L-0.1** | DA-16 | Write `clickhouse/06_market_bars_v2.sql`: all 18 v1 columns plus `bar_open_time`, `event_time`, and `snapshot_id MATERIALIZED toUnixTimestamp64Nano(ingested_time)`. `ENGINE = MergeTree`, `ORDER BY (instrument_id, timeframe, event_time, revision, ingested_time)`, `PARTITION BY (timeframe, toYYYYMM(event_time))`. Note that `clickhouse/` is mounted at `/docker-entrypoint-initdb.d`, so it runs on **first init only** — the file is the source of truth, and L-0.2 applies it to existing databases. |
| ~~**L-0.2**~~ ✅ | DA-16 | **Done.** `crates/storage/src/clickhouse/migrate.rs`: DDL embedded with `include_str!`, comment-aware statement splitter, replayed on every boot from `apps/platform/src/main.rs`. Idempotent (all statements are `CREATE TABLE IF NOT EXISTS`). Gated integration test `crates/storage/tests/clickhouse_migrate.rs` proves both the fresh-database and the already-migrated case against a real server. |
| ~~**L-0.3**~~ ✅ | DA-16 | **Done.** `crates/storage/src/clickhouse/backfill.rs`, plus the `backfill_bars_v2` example as an ops entry point and a boot-time call. Chunked by `(timeframe, month)`; a finished chunk is skipped, an empty one is copied, a partially-populated one aborts rather than duplicating (v2 has no engine to collapse duplicates away). **Run on the live database from the rescue snapshot: 99,789 rows across 7 chunks.** |
| ~~**L-0.4**~~ ✅ | DA-17 | **Done.** `verify_market_bars_v2` in `backfill.rs`, called automatically by the backfill whenever it copies anything, and by the ops example on every run. The check is a **row-level `LEFT ANTI JOIN` on a `sipHash64` fingerprint** of all 18 shared columns, not a count and not a summed hash — see the note below on why containment, not equality. Verified on the live database against both the snapshot and `market_bars`: 99,789 source rows, 0 missing. |
| ~~**L-0.5**~~ ✅ | DA-17 | **Done, as a dual-write rather than a switch** — see the note below. `CollectedBarRowV2` in `crates/backtest/src/store.rs`; `insert_collected` writes every bar to both tables, deriving `bar_open_time` from the timeframe's own period. The v1 write is removed in L-0.7, once no reader needs it. |
| ~~**L-0.6**~~ ✅ | DA-16 | **Done.** All five readers — `daily_counts`, `load_bars`, `load_bars_bucketed`, `list_coverage`, `last_bar_time` — now query `market_bars_v2`, group by `event_time`, and resolve revisions with the one shared `argMax(x, (revision, ingested_time))` expression. The PIT horizon is a property of the store (`BarStore::as_of`), not a parameter on 13 call sites — see the note below. No production read targets v1 any more. |
| ~~**L-0.7**~~ ✅ | DA-17 | **Done. Cutover date: 2026-09-11.** The v1 write is removed (`insert_collected` writes v2 only) and the dead `CollectedBarRow` struct with it. Read-only is enforced by **`cargo xtask check-bars-v1-frozen`** in CI, not by a database grant — see the note below. `02_bars.sql` carries a RETIRED banner. The table is kept readable and is **not** dropped; retirement is at the end of Set L. |
| ~~**L-0.8**~~ ✅ | DA-16 | **Done.** `crates/backtest/tests/bars_collision_regression.rs`. One property — *two bars, same instrument, same close, different timeframes, both survive a full merge and stay distinguishable* — checked by the **same code** against both schemas, asserting that v1 **fails** it and v2 passes. Observed: `market_bars destroyed a bar: expected both 1h and 1m to survive the merge, found ["1m"]`. |
| ~~**L-0.9**~~ ✅ | DA-17 | **Done.** DATA-005 §3 records the cutover and its verification numbers; G-13 notes the schema half is fixed and the depth half remains; 17_BUILD_ORDER §1 marks the backfill **UNBLOCKED**. |

---

## Acceptance criteria

1. `market_bars_v2` exists with `timeframe` in its sorting key and a non-collapsing
   engine. **(DA-16)**
2. Every row of `market_bars` is present in `market_bars_v2`, proven by a checksum over
   all shared columns, not by a row count alone. **(DA-17)**
3. `OPTIMIZE TABLE market_bars_v2 FINAL` changes no row count and preserves all 18 known
   collisions as distinct rows. **(DA-16)**
4. The L-0.8 regression test passes against v2 and is demonstrated to fail against the
   v1 schema. A test that has never failed proves nothing.
5. All bar writes go to v2; `market_bars` accepts no new rows. **(DA-17)**
6. Every bar read filters on `available_time <= as_of` and resolves revisions with one
   shared expression. **(DA-02, DA-16)**
7. The backfill ops task is unblocked only after 1–6 hold. **(DA-17)**

---

## Risks

| Risk | Mitigation |
|---|---|
| A reader nobody knew about still points at `market_bars` | L-0.7 keeps v1 readable; grep `market_bars` across `crates/`, `apps/` and `frontend/` as part of L-0.6, and leave the table in place until the end of Set L |
| A merge collapses rows mid-migration | L-0.3 reads from the rescue snapshot, which is a plain `MergeTree` and cannot collapse |
| `event_time` derivation is wrong for a timeframe | L-0.4's checksum covers only shared columns; add a spot check that `event_time = available_time` for the current writer and that `bar_open_time + timeframe = event_time` for every row |
| The dev box's ClickHouse is a fresh volume and behaves unlike the real one | L-0.2 must be exercised against the *existing* volume, which currently holds the 99,789 rows and the 18 collisions — the most useful test fixture available. Do not reset it |
| Doing this after the backfill | Explicitly forbidden by DA-17; the backfill multiplies the colliding rows |

---

## Notes for whoever picks this up

- The live ClickHouse is reachable as
  `docker exec trading_bot-clickhouse-1 clickhouse-client --user trading --password trading --database trading -q "…"`.
- **Do not run `OPTIMIZE` on `market_bars`.** The snapshot makes the loss recoverable,
  not harmless.
- `crates/storage/src/clickhouse/bars.rs` is dead code with a stale schema that no
  longer matches the DDL — it is not a writer, do not migrate it. It has its own
  removal task.
- Both dropped columns and added columns change the `SELECT *` shape, so the backfill
  in L-0.3 must name its columns explicitly.
