# Phase 0 — Measure, and start the irreversible clock

**Completion: 83% (5 / 6 tasks), plus 1 `BLOCKED:UPSTREAM` (P-0.4) — 2026-10-04 (track F-DATA-T): P-0.1, P-0.3 and P-0.6 closed; earlier line: 17% (1 / 6 tasks), plus 4 `PARTIAL` and 1 `BLOCKED:UPSTREAM` — 2026-10-03: the forward clock ran for the first time (P-0.3), the census wrote its first rows (P-0.1), the downsampling half of P-0.6 has a caller, and `GET /api/data/coverage` exists; three of the four PARTIALs wait only on shared-file edits (platform spawns, route registration) reported by track DATA.**

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
`DONE` 2026-10-04 (track F-DATA-T) — **the nightly caller is in and ran.** `apps/platform/src/census_jobs.rs` is spawned by `main.rs` (`census_jobs::spawn`, guarded by `main_spawns_the_census`); `census_jobs::tests::live_pass` ran the real pass against the live stores on 2026-10-04: 11 series censused, 11 rows written. All four acceptance clauses hold (clause 3 on live rows: `ETH-USD/kraken` observed, `BTC-USD/coinbase` derived). The census now carries **ADR-P1-7c** (a missing stretch of at most 900 s is a no-trade interval, not a gap; see P-1.7). `platform` tests: 63 passed.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **all four acceptance clauses now have evidence; the nightly caller is one orchestrator edit away.**
Clause 1 met on the live stores: `backtest::census::take_census` + `persist` were run against live
ClickHouse and the platform Postgres (docker, 127.0.0.1:5432) at 2026-10-03 17:44Z and wrote **9 rows**
to `dataplane.data_coverage_census`, one per `(instrument, venue, timeframe)` — the first rows that
table has ever held. Clause 3 is visible in those rows: `ETH-USD/kraken/1m` (all from `kraken_ws`, a
live collector) reports `derived_knowledge_bars = 0`; `BTC-USD/coinbase/1m` reports 121,421 derived of
121,486. Clauses 2 and 4 remain the named tests in `census.rs`. Deepest continuous series: 208 days
(the synthetic fixture); no live series satisfies `paper_v1`.
**What keeps it PARTIAL:** nothing in the repo runs it nightly. `apps/platform` is outside this track;
the exact `census_jobs.rs` module and the `main.rs` line are in the DATA track report (2026-10-03). Once
applied, this is `DONE`.

**Wave 2 (2026-10-03, track DATA):** `apps/platform/src/census_jobs.rs` (`spawn(pg, ch_url)`, first tick at boot then every 24 h, a failed pass logged and retried) is written and compiles. It calls the same `take_census` + `persist` that produced the 11 live rows. It stays PARTIAL until `main.rs` has the `mod census_jobs;` line and the spawn call; a source-scan test (`main_spawns_the_census`) fails until then, on purpose. `apps/platform/Cargo.toml` also needs `dataplane = { workspace = true }`.

Earlier: `PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`
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
`DONE` 2026-10-03 (track UI2) — **the page exists and the route is registered** (`/api/data/coverage` in `routes/mod.rs`). `frontend/src/pages/CoveragePage.tsx` shows the census thinnest series first (re-sorted client-side on the same key, so a reordered response cannot bury the thin one), states depth in words from the **continuous** run (not the first-to-last span), the standing against both floors ("meets Gate 9", "paper only", "below both floors", "empty"), a depth meter against the strict floor, the server's `permits` sentences verbatim (so the shortfall in days is the server's wording, not a re-derivation), a `not accruing` badge on a stale series, and beside it every forward-only series with the age of its last row, stalest first. An unmeasured heartbeat table is "not measured", never an empty list, and the Desk-only 403 is stated rather than drawn as an empty census. Reachable from the primary menu (Data coverage) and from the desk rail. Evidence: the coverage block of `pages/__tests__/coverage-alerts-quality-and-governance-render.test.tsx` (5) and the coverage block of `lib/ops/__tests__/the-pages-state-what-they-do-not-know.test.ts` (6). The route line for `App.tsx` (`/data/coverage`) is requested from the orchestrator; until it lands the page is not mounted.

Earlier note (track DATA) — **the endpoint is built; the page and the route line are not this track's files.**
`GET /api/data/coverage` is `crates/api/src/routes/data.rs::get_coverage`: the newest census row per
series, sorted thinnest-first, each with its `strict_v1` Gate 9 and `paper_v1` verdicts **in words**
(`floor_sentence`), and the forward-only series (P-0.6 clause 3) listed beside them from
`forward_series_heartbeat` with their last-row age. Clause 2: `a_series_short_of_gate_nine_states_its_shortfall_in_days`;
a stale series gets no countdown (`a_stale_series_is_not_given_a_countdown`) — "satisfiable in N days"
on a dead feed would be the reassurance that hides it. Clause 3: the handler takes `BearerToken`
(`the_coverage_handler_requires_a_bearer_token` pins it). 5 tests in `coverage_tests`.
**ADR (DATA, 2026-10-03):** the endpoint is **Desk-only** (403 `desk_only` for a research project)
rather than horizon-clipped — the census's whole point is the true last bar, and "history ends on the
4th" is information about the future for a project with a cutoff. Rejected: clipping each row to the
horizon, which would publish a census that disagrees with the table it reads.
**Missing:** the route registration in `routes/mod.rs` (shared) and the desk page in `frontend/src/pages/`
(not DATA territory) — both written out in the DATA track report.

Earlier: `NOT STARTED`

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
`DONE` 2026-10-04 (track F-DATA-T) — **verified live, and the platform boot spawns it.** `main.rs` calls `collectors::derivs::runner::spawn` (the source-scan and boot path are in the tree); on 2026-10-04 the `forward-collector` binary ran one poll against Kraken Futures and `funding_rates` went 2080 → 2496 rows (+416, the dedup keeps restated rates only), `open_interest` +208, every new row `quality_flags = 0` (observed, no `BACKFILLED_KNOWLEDGE_TIME`). Clauses 2 and 3 keep their named tests.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the forward clock is running for the first time; the platform-boot spawn is one orchestrator line.**
The collector, sink and limiter existed and nothing constructed them: on 2026-10-03 `funding_rates` and
`open_interest` held **0 rows** while `dataplane.forward_series` had claimed `collecting_since =
2026-09-21` — twelve days of an unbackfillable series (FM-11) lost to a missing caller.
New: `crates/collectors/src/derivs/runner.rs` (`build`/`run`/`spawn`, refuses to start if either source
row is missing or if asked to poll slower than the registry's 3600 s cadence — 3 tests) and
`crates/collectors/src/bin/forward-collector.rs`. **Clause 1, run live:** the binary ran 66 minutes
against Kraken Futures from 17:37Z (15-minute polls), writing 208 perpetuals per poll to
`funding_rates` (realized + predicted) and `open_interest`, every row `quality_flags = 0` (observed,
no `BACKFILLED_KNOWLEDGE_TIME`), heartbeats in `forward_series_heartbeat`. Clause 2 is
`funding.rs::the_client_refuses_before_it_sends` and `service.rs::a_limiter_refusal_is_counted_apart_from_a_failure`. Clause 3 **failed on the live run and is fixed**: `knowledge_time` is in `funding_rates`' sorting key
(so restatements survive), which made every re-poll of an unchanged settlement a new row — after 4 polls
the 416 realized `(instrument, settlement)` pairs were stored as 832 rows. The parse-level test
(`a_restart_writes_rows_whose_keys_already_exist_rather_than_new_ones`) could not see it.
`storage::clickhouse::forward::write_funding` now skips a row whose rate equals the latest stored one
for that `(venue, instrument, kind, event_time)` and still writes a changed rate as a restatement;
`storage/tests/clickhouse_forward_idempotent.rs` proves both on a scratch ClickHouse. (Readers already
kept the latest row per settlement, so no study was affected; the 416 duplicate rows from the first
run remain in the live table as harmless re-observations.) Terms: verified 2026-09-15 and recorded in `funding.rs`.
**Missing:** `collectors::derivs::runner::spawn` is not yet called from `apps/platform/src/main.rs`
(shared file); the exact edit is in the DATA track report. Until applied the series collects only
while someone runs the binary.

Earlier: `NOT STARTED`

**The heartbeat join, fixed 2026-09-22 on a branch and merged 2026-10-18
(`claude/magical-bohr-313c13`).** `service.rs` wrote `series = "funding"` and
`series = "open_interest"`, while `observability::silence::FeedExpectation::owns`
matches a heartbeat to its registry row on `series_id` **or** `store_table`:
`"open_interest"` matched the store table of the same name by coincidence, and
`"funding"` matched neither `kraken_futures.funding` nor `funding_rates` — so a healthy
funding feed's heartbeat reads `CannotJudge` while its registry row reads `NeverSeen`, a
false page. The constants are now the registry's own ids (`kraken_futures.funding`,
`kraken_futures.open_interest`), and
`the_series_the_sink_writes_are_the_ones_the_alarm_joins_on` parses migration 0098 and
pins both strings against the rows it seeds. Heartbeat rows already stored under the old
names stay where they are and read as unowned until they age out.

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
`BLOCKED:UPSTREAM` — **re-checked 2026-10-04 (track F-DATA-T): unchanged.** Deribit's Articles 2.4 and 4.6 still stand and no free, US-accessible crypto-options source was found; the decision needed (a venue, or an ADR-0027 revision) is the owner's. **Corrected 2026-09-21; the marker said `NOT STARTED`
while the task's own escape clause had already been satisfied.** The sibling
document `P-0.4-BLOCKED-crypto-option-chains.md` quotes Deribit's terms (Art.
2.4, 4.6) and records the block. The acceptance says *"terms verified and quoted
… or `BLOCKED:UPSTREAM`"*, and that was done six days before anyone changed the
marker. Needs a human: a decision on the next crypto-options candidate, or on
revisiting ADR-0027.

**Re-checked 2026-10-03 (track DATA): still `BLOCKED:UPSTREAM`.** Deribit's Art. 2.4 (US persons may not access the Platform, API included) and Art. 4.6 (market data for personal use only) are the recorded terms and nothing has superseded them; no free, US-accessible crypto-options venue has appeared — OKX, Bybit and Binance restrict US access, Kraken and Coinbase list no crypto options, and CME/vendor feeds (e.g. CoinAPI) are paid, which ADR-0027 forbids without a new ADR. Nothing was built against any of them. The storage half is ready the moment a source is chosen: `option_chain_snapshots`, its IV-not-Greeks guard, the `crypto.option_chains` disk budget and the downsampling pass (P-0.6) all exist. Still needs a human: a venue decision or an ADR-0027 revision.

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
`DONE` 2026-09-28 — **the marker said `NOT STARTED`; all three clauses have named tests.**
`crates/collectors/src/options/tradier_chain_snapshot.rs`:
`a_snapshots_knowledge_time_is_its_observation_nine_hundred_seconds_after_the_market_it_describes`
asserts both halves of clause 1 rather than one — if `knowledge_time` were the slot the row would
claim to have been knowable before Tradier published it, and if `event_time` were the receipt the row
would claim the 14:15 market when Tradier had shown us 14:00, which is fifteen minutes of lookahead
in every study that reads it. `the_idempotence_key_is_symbol_expiry_and_as_of` is clause 2 over the
acceptance's own tuple, and separates a call from a put on the same strike and expiry, which is where
a too-coarse key would collapse two contracts into one.

Clause 3 is `no_corporate_action_handling_is_introduced_by_this_task`, and it is a source scan rather
than a value assertion — correctly, because *this code does not reach for the adjustment machinery*
is a fact about the code and not about any one value it produces. The source row and its declared lag
are in migration 0098: source 19, `tradier_chain_rest`, 900 000 ms.

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
`DONE` 2026-10-04 (track F-DATA-T) — **the hourly pass has its caller.** `apps/platform/src/retention_jobs.rs::spawn` is called from `main.rs` (guarded by `main_spawns_the_retention_pass`), running `storage::clickhouse::retention::retention_pass` hourly; an over-budget series with no rule logs at error and nothing is deleted. All three clauses have evidence (alarm test, the live-ClickHouse downsample-not-delete test, the coverage page's forward series).

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the downsampling half now has a caller and a test that drives a real table past its budget; the
hourly hook is one orchestrator edit.**
New `crates/storage/src/clickhouse/retention.rs`: `venue_bytes` measures a series' disk use from
`system.parts`, `apply` asks `dataplane::retention::retention_action` and carries the answer out, and
`retention_pass` does it for every row of `dataplane.forward_series`. Clause 2 is
`storage/tests/clickhouse_retention.rs::a_chain_table_past_its_budget_is_downsampled_not_deleted`, run
2026-10-03 against live ClickHouse in a throwaway database: 144 hourly rows (2 contracts × 3 days, 100 days
old) driven past a budget set below their measured bytes become 6 rows, one per `(contract, day)`, each the
day's **last real observation** relabelled `daily`; recent rows and another venue's rows untouched;
continuity checked before and after (`DownsampleReport::continuity_held`); a second pass is a no-op.
Mutation-checked: keeping each bucket's *first* row instead of its last fails the test. Over budget with
policy `none` is reported as a decision for a human and touches nothing
(`over_budget_without_a_rule_is_a_decision_for_a_human`). Clause 3 (forward series with last-row age on
the coverage page) is in `GET /api/data/coverage` (P-0.2).
**ADR (DATA, 2026-10-03):** "downsampled, not deleted" is read as *every `(contract, bucket)` that held an
observation still holds one*: the finer rows of a bucket are removed once its coarse row exists (label
first, then remove, so no bucket is ever empty). Rejected: keeping every fine row and adding coarse ones,
which frees no disk and so cannot be what a budget rule means; and averaging a bucket, which would store
an IV no venue quoted (INV-07).
**Missing:** `retention_pass` is not yet called on a schedule — the `health_jobs.rs` line is in the DATA
track report (apps/platform is not this track's).

**Wave 2 (2026-10-03, track DATA):** `apps/platform/src/retention_jobs.rs` (`spawn(pg, ch_url)`, hourly `retention_pass`; an over-budget series with no rule logs at error and nothing is deleted) is written and compiles. Pending the `mod` line and the spawn in `main.rs` (guarded by `main_spawns_the_retention_pass`).

Earlier: `PARTIAL` 2026-09-28 — **the two halves of this task are in opposite states, and the marker was
hiding both.**

The alarm half is done and *reached*: `crates/dataplane/src/retention.rs`'s
`stopping_a_collector_raises_the_alarm_within_k_cadences` advances a clock rather than sleeping,
as clause 1 asks, and `retention::alarm` is genuinely called from
`crates/observability/src/silence.rs:637`.

**The downsampling half is tested and unreached.** `over_budget_never_deletes` passes, so clause
2's stated property holds of the function — but `retention_action`, the function that *decides*
the downsampling, has **no caller workspace-wide**. So a forward series will reach its disk
budget and nothing will downsample it, while the test that exists for that clause goes on
passing. Same shape as the census, and worth stating in the task rather than in a commit
message: clause 2's acceptance asks for a test that *drives a table past its budget*, and a pure
function nobody calls cannot drive a table. Wiring it needs a job.

Clause 3 — the coverage page listing every forward series with its last-row age — is the census
read path and belongs with P-0.2.

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
