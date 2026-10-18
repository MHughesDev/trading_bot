# Phase 0 — Measure, and start the irreversible clock

**Completion: 0% (0 / 6 tasks)**

**Requirement IDs:** S5, S6, S8, DA-12 · `artifact.md` FM-11.
**Spec:** [DATA-005 §11 (S5, S6, S8), §12](../../../specs/DATA-005-data-api-v2.md);
[06_DATA §6](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD); ADR-0027; ADR-P2-24.
**Depends on:** nothing. Set L Phase 0 already delivered `market_bars_v2` and froze the v1 table.
**Blocks:** Phase 1 (`backfill_state`), Phase 3 (the same job scaffolding).
**Migration range:** 0097–0098.
**Migrations shipped:** 0097, 0098 (merged 2026-09-15, integration wave 5).
**Parallel with:** every Set S phase.

This is the cheapest phase in the whole programme and the only one whose cost of delay is
permanent. It does two things: it finds out how much history the platform actually has, and
it starts collecting the series that can never be collected retroactively.

**Nothing here is interesting and all of it is urgent.** A week of delay on Phase 1 costs a
week. A week of delay on P-0.3 costs a week-shaped hole in every funding-rate study the
platform will ever run, and no amount of later effort or money closes it under a free-only
data policy (ADR-0027).

---

## Measure first

### P-0.1 The coverage census — what history actually exists — M
`PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`
and the work is largely done.** `crates/dataplane/src/census.rs` is 749 lines and
carries acceptance clauses 2 and 4 as named tests
(`planted_three_bar_gap_is_reported_exactly`,
`rerun_is_identical_except_as_of`); migration `0097` is applied; and
`P-0.1-CENSUS-2026-09-15.md` publishes a nine-series result measured on live
ClickHouse. **The gap is a caller:** `take_census`/`persist` have **zero callers
workspace-wide** and `dataplane.data_coverage_census` holds 0 rows. Built,
tested, published, unreached.

A deterministic job that walks every `(instrument_id, timeframe)` in `market_bars_v2` and
reports: first bar, last bar, expected bar count over that span from the instrument's trading
calendar, present count, missing count and percentage, longest gap, count of rows whose
`knowledge_time` is derived rather than observed, and the span of continuous coverage with no
gap longer than one period. Results land in a new `data_coverage_census` table (migration
0097) with one row per `(instrument, timeframe, as_of)` so the census is a time series, not a
snapshot that overwrites its own history.

The job runs nightly and on demand. It writes nothing to the bars themselves.

**Why this is task one:** "the platform has thin data" is currently an inference from a single
2026-09-08 observation of 22 BTC-USD 1h bars. The whole roadmap's Stage ordering, and the
question of whether `strict_v1` is reachable this decade, rests on a number nobody has.

**Files:** `crates/dataplane/src/census.rs`, `crates/jobs` worker registration,
migration 0097.
**Acceptance:** the census runs against the live ClickHouse and emits one row per
(instrument, timeframe); a planted series with a known 3-bar gap reports exactly that gap and
its length; the derived-knowledge-time count is non-zero for rows the canonical migration
flagged and zero for rows written by a live collector; re-running on an unchanged database
produces identical output except `as_of`.
**Traces:** DATA-005 §7 (coverage checks), §12 (storage); ADR-P2-24; Set P §1, D-P5.

### P-0.2 Publish the census, and make the evidence clock visible — M
`NOT STARTED`

A read endpoint `GET /api/data/coverage` and a page in the desk showing the census as a table
and a per-instrument coverage bar, sorted by depth ascending so the thinnest series is first.
For each row it states, in words, what the depth permits: whether `strict_v1` Gate 9's
five-year floor is satisfiable, whether `paper_v1` is, and how many days until it would be at
the current collection rate.

This is the artefact the roadmap's Stage-0 exit depends on, and the thing to look at before
believing any claim about what the platform can evaluate.

**Files:** `crates/api/src/routes/data.rs`, `frontend/src/pages/` (one page, Meridian
primitives only — no `bg-surface-2`/`text-text-*` aliases, per S-6.1).
**Acceptance:** the page renders the census for every instrument; an instrument with fewer
bars than Gate 9 requires is labelled as such with the shortfall in days; the endpoint refuses
a request without a valid token; the page is reachable from the desk navigation.
**Traces:** DATA-005 §7; ROADMAP §1, §4 Stage 0 exit.

---

## Then collect what cannot be collected later

### P-0.3 Funding rates and open interest, forward — M
`PARTIAL` — **corrected 2026-09-22; the marker said `NOT STARTED` over
`crates/collectors/src/derivs/`, which is three modules and 26 passing tests.**
The collector is built: `funding.rs` parses the venue's real shapes and keeps a
prediction apart from a settlement, `sink.rs` resolves provenance against
`dataplane.source` rather than defaulting it, `service.rs` polls, backfills and
beats. Migration 0098 is applied and seeds both series, enabled.

**The gap is a caller.** `ForwardCollector` has zero callers workspace-wide —
`grep -rn "derivs::" crates/ apps/ --include=*.rs` returns only matches inside
the module itself — so the acceptance clause that requires an hour against the
live venue is unmet, and `trading.funding_rates`, `trading.open_interest` and
`trading.forward_series_heartbeat` are reported at 0 rows. Built, tested,
unreached: the same shape as P-0.1.

**Switching it on is deliberately not part of the 2026-09-22 heartbeat-naming
fix, and needs a human.** Two reasons, the second of them a further gap:

1. Starting it is an outward-facing, continuing decision — a live poll against a
   third-party venue, writing the live store — and not a side effect of a rename.
2. Turning it on alone would still produce no alarm, because the chain needs
   two wires and this is one of them. The reading half landed on main in
   `7261301`: `storage::clickhouse::census::forward_census` calls
   `read_heartbeats` and maps each row's `series` straight into a `FeedKey`,
   and `silence::evaluate` now has three real call sites. What nothing calls is
   `census::silence_report` — no scheduler, no health job. So today a running
   collector would write rows that are read by a function nobody invokes, and
   the registry row would go on reporting `NeverSeen`, correctly. A caller for
   the collector is half the work; a caller for the census pass is the other
   half, and it is being chased separately.

What *was* fixed on 2026-09-22 is the join itself. `service.rs` wrote
`series = "funding"` and `series = "open_interest"`, while
`observability::silence::FeedExpectation::owns` matches a heartbeat to its
registry row on `series_id` **or** `store_table`: `"open_interest"` matched the
store table of the same name by coincidence, and `"funding"` matched neither
`kraken_futures.funding` nor `funding_rates`. The first hour the collector ran,
its funding heartbeat would have been reported `CannotJudge` (Info) while the
registry row for that same healthy feed reported `NeverSeen` (Alarm, a page) — a
permanent false page on a working feed, which is the one shape that reliably
gets an alarm muted. Half the join worked and half did not, and with both tables
at zero rows nothing observable would have said so first. The constants are now
the registry's own ids, and
`the_series_the_sink_writes_are_the_ones_the_alarm_joins_on` parses migration
0098 and pins both strings against the rows it actually seeds. The test lives in
`collectors` because that is the only crate able to see both sides — `storage`
and `observability` cannot depend on it.

**Remaining for `DONE`:** a caller, an hour against the live venue, and the
idempotency-on-restart clause verified against real rows.

A collector for a free, US-accessible public derivatives API (DATA-005 S6 names Kraken
Futures public as the candidate; **verify geo-restrictions and terms before building** — a
prior 451 from Binance is recorded in project memory). Two ClickHouse tables (migration 0098):
`funding_rates` and `open_interest`, each with the four-timestamp discipline and an observed
`knowledge_time`, since these are collected live.

Backfill where the venue offers it; forward collection regardless. The forward leg is the
point — this series has no vendor history under ADR-0027.

**Files:** `crates/collectors/src/derivs/funding.rs`, `clickhouse/` DDL, migration 0098,
`crates/dataplane/src/source.rs` (declare the source and its lag: live stream = 0).
**Acceptance:** the collector runs for an hour against the live venue and writes rows with
observed `knowledge_time` and no `BACKFILLED_KNOWLEDGE_TIME` flag; the rate limiter refuses to
exceed the declared limit under a test that drives it past; a restart resumes without
duplicating a row (idempotent on `(venue, instrument, event_time)`); if the terms forbid this
source, the task is `BLOCKED:UPSTREAM` with the term quoted and the next candidate named.
**Traces:** DATA-005 §11 S6, §12; DA-11; `artifact.md` FM-11.

### P-0.4 Crypto option chain snapshots, forward — S
`BLOCKED:UPSTREAM` — **corrected 2026-09-21; the marker said `NOT STARTED`
while the task's own escape clause had already been satisfied.** The sibling
document `P-0.4-BLOCKED-crypto-option-chains.md` quotes Deribit's terms (Art.
2.4, 4.6) and records the block. The acceptance says *"terms verified and quoted
… or `BLOCKED:UPSTREAM`"*, and that was done six days before anyone changed the
marker. Needs a human: a decision on the next crypto-options candidate, or on
revisiting ADR-0027.

Hourly snapshots of BTC and ETH option chains from a free public API (DATA-005 S5 names
Deribit public; market-data endpoints only, never trading endpoints — **verify US access terms
first**). One ClickHouse table `option_chain_snapshots` carrying strike, expiry, side, bid,
ask, mark, and **implied volatility as the stored quantity, with Greeks computed at read time,
never stored** (INV-07).

The universe gate is `universe_id = 1` as decided in ADR-P2-26: `|moneyness − 1| ≤ 0.30`,
`dte ∈ [1, 400]`, two-sided quote. Outside the gate, end-of-day snapshots where the source
provides them — never deletion.

**Files:** `crates/collectors/src/options/crypto_public.rs`, `clickhouse/` DDL, migration 0098.
**Acceptance:** a snapshot writes IV and not Greeks, and a static test fails the build if a
Greek column is added to the table; rows outside `universe_id = 1` are stored at end-of-day
cadence rather than dropped; the snapshot's `knowledge_time` is its observation time; terms
verified and quoted in the task, or `BLOCKED:UPSTREAM`.
**Traces:** DATA-005 §11 S5, §12; DA-12; INV-07; ADR-P2-26.

### P-0.5 Equity option chain snapshots on the existing Tradier collector — S
`NOT STARTED`

DA-12's SHOULD: a forward-stored chain snapshot job on the Tradier collector the platform
already has. Same table and same IV-not-Greeks rule as P-0.4, same universe gate. Delayed
source, so the declared lag is 900 s and `knowledge_time` reflects it — the row is observed,
but observed *late*, and the two are not the same thing.

Scope note: this collects **snapshots**, not an equities onboarding. The equities schema
questions (corporate actions, INV-03, surrogate instrument ids) stay closed behind roadmap
gate H-6.

**Files:** `crates/collectors/src/options/tradier_chain_snapshot.rs`, migration 0098 (source
row only).
**Acceptance:** a snapshot's `knowledge_time` equals observation time plus the declared 900 s
lag and the source row records that lag; the job is idempotent per `(symbol, expiry, as_of)`;
no corporate-action handling is introduced by this task.
**Traces:** DATA-005 §11 S8; DA-12; ADR-P2-24 (declared lags).

### P-0.6 Disk budget, downsampling and the silence alarm — M
`NOT STARTED`

Three things that must exist *before* the first snapshot, not after:

1. **A disk budget** per forward series in `config/data_retention.toml`, with a downsampling
   rule when it is approached (chain snapshots degrade hourly → daily beyond *n* days; funding
   and OI are small and kept at full resolution). This answers DATA-005 §14 open question 2 for
   the series this phase switches on, and only those.
2. **A gap alarm.** Each forward source declares an expected cadence; a monitor fires when the
   most recent row is older than *k* cadences. **Silence is this phase's real failure mode** —
   a collector that dies quietly costs exactly what never collecting it would have.
3. A row in the census (P-0.1) for every forward series, so the coverage page shows the
   forward clock beside the historical one.

**Files:** `config/data_retention.toml`, `crates/dataplane/src/retention.rs`,
`crates/observability` alarm registration.
**Acceptance:** stopping a forward collector raises the alarm within *k* cadences in a test
that advances a clock rather than sleeping; the downsampling rule is exercised by a test that
drives a table past its budget and asserts older rows are downsampled, not deleted; the
coverage page lists every forward series with its last-row age.
**Traces:** DATA-005 §14 (open question 2); Set P §8 risks; ROADMAP §4 Stage 0 exit.

---

## Phase exit

- The census exists, runs nightly, and is published on a page a human reads.
- Every forward-only series this phase names is either collecting, or recorded as
  `BLOCKED:UPSTREAM` with the term that forbids it.
- Stopping a collector is noticed automatically.
- `backfill_state` (created in Phase 1) is the only thing Phase 1 needs that this phase does
  not already provide.
