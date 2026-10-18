# Phase 3 — Alternative data

**Completion: 17% (1 / 6 tasks), plus 3 `PARTIAL` and 2 `BLOCKED:HUMAN` (P-3.1 key, P-3.3 credentials) — 2026-10-04 (track F-DATA-T); earlier line: 0% (0 / 6 tasks), plus 6 `PARTIAL` — 2026-10-03: every source now has a store and a point-in-time read proven on a real database; collection waits on credentials (FRED key, Alpaca keys, an EDGAR operator User-Agent) and route registration.**

**Requirement IDs:** S2, S3, S4, S7, DA-11.
**Spec:** [DATA-005 §10 (text PIT rules), §11 S2–S4, S7, §12](../../../specs/DATA-005-data-api-v2.md);
[06_DATA §7](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD).
**Depends on:** Phase 0 (the job scaffolding and the census's forward-series rows).
**Blocks:** nothing in this set; supplies inputs the regime work in Stage 5 needs.
**Migration range:** 0102–0103.
**Parallel with:** Phases 1 and 2, and every Set S phase.

Bars are one series. Everything that explains *why* a bar moved — rates, filings, news, event
probabilities — is here, and every one of these sources is point-in-time-hostile by default:
the naive read returns today's revised value for a historical date and the resulting backtest
is fiction.

**The common rule across all four sources: store the vintage, not the current value.** A macro
series has a release date and a revision history; a filing has an acceptance timestamp; a news
item has a publication time. The value a strategy could have used on date *t* is the one
published by *t*, and nothing else.

---

### P-3.1 FRED / ALFRED with vintages — M
`BLOCKED:HUMAN` 2026-10-04 (track F-DATA-T) — **a FRED API key is mandatory, and there is no keyless path.** FRED's terms: "In order to use the FRED® API, you must have register for an API Key" and the St. Louis Fed "will block requests with an invalid key"; no exception for `fredgraph.csv`, so the keyless CSV graph endpoint was not built against. Exact remaining steps: (1) register a free key at https://fredaccount.stlouisfed.org/apikeys (account creation is the owner's); (2) the platform has no data-source credential store (the encrypted store in `crates/api/src/credentials` is keyed by LLM provider, and `AlfredClient::new` refuses a key that did not come from a store), so add one table and one reader and call `AlfredClient::fetch_all_vintages` → `storage::clickhouse::alt::write_macro` from a runner. The store and PIT read are proven; the table `macro_observations` now exists in the live ClickHouse.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the store and the point-in-time read exist and are proven on ClickHouse; nothing collects, because there
is no FRED key.**
`clickhouse/10_alt_data.sql` adds `macro_observations` (one row per vintage, `(series_id,
observation_date, vintage_date)`), registered in `storage::clickhouse::migrate`; migration 0156 declares
`alfred_rest` (row 21, free, 86 399 s). `storage::clickhouse::alt::write_macro` / `macro_as_of`, and
`GET /api/data/macro` (horizon-clipped `as_of`). Clauses 1–2:
`storage/tests/clickhouse_alt_pit.rs::a_macro_revision_is_invisible_before_its_vintage` (live ClickHouse,
throwaway DB): original before the revision, revised after, identical on re-query, and a vintage that
reported "." stays missing instead of falling back to the older number (`argMax(tuple(value), …)` —
plain `argMax` skips NULLs). Clause 3 keeps `fred.rs::a_key_not_from_the_credential_store_is_refused` and
the three redaction tests.
**Missing:** a FRED API key in the encrypted credential store (BLOCKED:HUMAN — FRED requires one), then a
runner calling `AlfredClient::fetch_all_vintages` → `write_macro`; and the route registration (report).

Earlier: `NOT STARTED`

Macro series into a ClickHouse `macro_observations` table keyed on
`(series_id, observation_date, vintage_date)` — ALFRED's vintage endpoint is what makes this
honest, and it is why S4 names ALFRED and not only FRED. A read as of *t* returns the latest
vintage published on or before *t*.

API key in the encrypted credential store, never in config.

**Files:** `crates/collectors/src/macro/fred.rs`, `clickhouse/` DDL, migration 0102 (source row).
**Acceptance:** a series with a known revision returns the *original* value for a read as of a
date before the revision and the revised value after; the same query run twice returns the
same answer; the key is read from the credential store and a test asserts it is absent from
config and logs.
**Traces:** DATA-005 §11 S4, §12; DA-11.

### P-3.2 SEC EDGAR fundamentals, PIT by acceptance time — M
`PARTIAL` 2026-10-04 (track T-P) — **the Data API route the earlier notes list as missing exists and is mounted; the remaining items are the operator's.** `GET /api/data/fundamentals` (`routes/data.rs::get_fundamentals`, `routes/mod.rs`) reads `dataplane.fundamentals_fact` directly with `accepted_at <= as_of` clipped to the project cutoff, so `api` needed no dependency on `collectors`; and the same table now feeds `fundamental_<cik>_<tag>` feature columns through `GET /api/data/features` (P-3.6, `features_compute::load_external_rows`, every filing kept so each decision picks its own). Still open: the real operator User-Agent in `config/edgar.toml` and a standing run against the platform database once 0156 is applied there (`BLOCKED:HUMAN`: the SEC wants a real contact address, which this track does not invent).

`PARTIAL` 2026-10-04 (track F-DATA-T) — **ran for real filers; the standing collector needs the operator's real User-Agent.** `config/edgar.toml` carries `user_agent` as an explicit field with a clearly marked `PLACEHOLDER` value; `edgar-collector` reads `EDGAR_USER_AGENT` first, then that field, and refuses to run while it still says PLACEHOLDER unless `EDGAR_ALLOW_PLACEHOLDER=1` (a one-off smoke). Smoke run 2026-10-04 against the SEC with the placeholder, into a scratch Postgres carrying 0046/0104/0156: Apple (CIK 320193) **25,135 facts, 15,068 with an observed acceptance time, 22,618 written**; Microsoft partially (541, the run was cut by a timeout); the 10 req/s limiter held. Remaining: put the real operator name and contact address in `config/edgar.toml` (the SEC asks for one), run it against the platform database once 0156 is applied there (the live database is at migration 145), and a Data API route (`api` does not depend on `collectors`).

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **DATA-005 §10.4 holds on the real table; the Data API route is one dependency line away.**
Migration 0156: `dataplane.fundamentals_fact` (Postgres), `accepted_at` the knowledge time, a CHECK that
refuses a filing accepted before its period ended, append-only (UPDATE/DELETE refused), granted to
`app_role`; `sec_edgar` declared (row 20, free). `collectors::fundamentals::store::{insert_facts,
facts_as_of}` and `bin/edgar-collector` (requires the operator's `EDGAR_USER_AGENT`; refuses to start
without one). Clause 1: `collectors/tests/pg_fundamentals.rs::a_request_as_of_t_never_returns_a_value_filed_after_t`
(Postgres 17 scratch DB with 0046/0104/0156): a Q1 fact accepted a quarter later is invisible mid-April
and one second before acceptance, visible at acceptance, superseded by a 10-Q/A after its own acceptance,
both rows kept; `app_role` can insert and read (SET LOCAL ROLE). Mutation-checked: filtering on
`period_end` instead of `accepted_at` fails it. Clauses 2–3: `edgar.rs::the_user_agent_is_carried_on_the_client`,
`a_client_without_a_declared_operator_cannot_be_built`, `the_client_budget_is_the_published_ten_per_second`.
**Not run live:** the SEC requires a User-Agent naming the operator and a contact address; this track did
not invent one. **Missing:** `GET /api/data/fundamentals` — `api` does not depend on `collectors`
(report), so the route was not written.

Earlier: `NOT STARTED`

Submissions and companyfacts/XBRL into `fundamentals_facts` (Postgres, migration 0102: cik,
tag, value, unit, period, accepted_at). `accepted_at` is the knowledge time — **the period a
number describes is not when it became knowable**, and conflating the two is the single most
common lookahead bug in fundamentals research.

Fair-access rules are binding: a declared User-Agent identifying the operator, and ≤ 10
requests per second, enforced by the limiter.

**Files:** `crates/collectors/src/fundamentals/edgar.rs`, migration 0102.
**Acceptance:** DATA-005 §10.4 — a fundamentals request as of date *t* never returns a value
filed after *t*, tested with a fact whose period precedes its acceptance by a quarter; the
User-Agent header is present on every request and a test asserts it; the limiter refuses above
10 req/s.
**Traces:** DATA-005 §10.4, §11 S3, §12; DA-11.

### P-3.3 Alpaca news and corporate actions — S
`BLOCKED:HUMAN` 2026-10-04 (track F-DATA-T) — **no Alpaca credentials exist in any configured store** (`.env` holds none; checked 2026-10-04). Exact remaining steps: create an Alpaca (free-plan) data key, store it where the collectors read it, and add an auth header to `collectors::backfill::HttpGet` (P-1.4's note: today it sends none). The clauses are tested and the depth is recorded above.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **the clauses are tested; collection is blocked on Alpaca credentials.** Clause 1:
`corp_actions/alpaca.rs::the_stored_price_is_unadjusted_and_the_read_applies_the_factor` (and
`a_split_is_invisible_before_it_was_declared`). Clause 2: `news/alpaca.rs::an_item_is_never_returned_before_its_publication_time`,
now also on the store — `text_items` (`clickhouse/10_alt_data.sql`) with
`clickhouse_alt_pit.rs::a_corrected_article_is_two_rows_and_reads_as_it_stood`. Clause 3, the free plan's
depth: **news history to 2015, not tiered** (Alpaca's historical-news page, re-verified 2026-09-22 and
recorded in `news/alpaca.rs`) — backfillable, so news does not join the FM-11 forward-only argument, though
a correction's *original* text is only observable by collecting forward. Corporate-actions archive depth is
**unpublished** and recorded as unverified. `alpaca_news` declared (row 22, free, migration 0156).
**Missing:** no Alpaca key is configured here, and `collectors::backfill::HttpGet` sends no auth header
(P-1.4's note), so neither collector can run; a writer from `CorporateAction` into
`dataplane.corporate_action`.

Earlier: `NOT STARTED`

Two collectors. News into `text_items` with publication time as knowledge time, subject to the
§10 text PIT rules. Corporate actions — splits and dividends — into the adjustment tables
INV-03 requires, so that **prices stay unadjusted in storage and factors are applied at read
time**, which is the rule the platform has never had to honour because it has never held an
equity corporate action.

Confirm the free plan's news history depth first and record it; if it is shallow, that is a
forward-collect series and belongs to the FM-11 argument in Phase 0.

**Files:** `crates/collectors/src/news/alpaca.rs`, `crates/collectors/src/corp_actions/`,
migration 0103.
**Acceptance:** a stored price is unadjusted and a read applies the factor as of the read's
`as_of`, with a test over a split; a news item is never returned for a read before its
publication time; the free plan's actual history depth is recorded in this phase file.
**Traces:** DATA-005 §11 S2, §10 (text rules); INV-03; 06_DATA §7.

### P-3.4 Kalshi history — S
`DONE` 2026-10-04 (track F-DATA-T) — **the fetch and the sink exist and ran.** Kalshi changed its candlestick payload from integer cents to dollar strings (`close_dollars`, `volume_fp`) and the parser read only cents, so it would have skipped every live period; both shapes now parse (`the_live_dollar_shape_parses_and_a_no_trade_period_is_skipped`, recorded from the live endpoint). New in `prediction::history`: `fetch_candlesticks` (limiter asked before the request, a refusal is not sent: `the_fetch_sends_a_get_and_the_limiter_refuses_past_its_budget`; self-imposed 5 req/s, not a quoted limit), `to_rows` (derived rows stay flagged: `stored_rows_keep_the_derived_flag`), and the `kalshi-history` binary. Migration **0191** declares `kalshi_history` (source 24, free, 60 s). Run 2026-10-04 against the public API: two `KXHIGHNY` markets, 39 candles with trades each, written to the live `prediction_probabilities` (flag 32768 = backfilled knowledge time on every row). `kalshi_exposure_only` (guards the client surface) passes. Not yet: 0191 is not applied to the live Postgres (the platform applies it at boot).

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **Kalshi probabilities now have a store and a cutoff-applied read; no collector writes them yet.**
`prediction_probabilities` (`clickhouse/10_alt_data.sql`), `storage::clickhouse::alt::{write_probabilities,
probabilities_as_of}`, and `GET /api/data/prediction_markets` (window and `as_of` clipped to the project
horizon; the manifest counts forward-evidence rows). Clause 1:
`clickhouse_alt_pit.rs::a_prediction_market_row_is_not_readable_before_its_knowledge_time` — a backfilled
candlestick is invisible at its own close, readable after the declared 60 s, flagged; a probability outside
[0, 1] is refused. Clause 2 keeps `collectors/tests/kalshi_exposure_only.rs`.
**Missing:** a runner that fetches candlesticks (`prediction::history::parse_candlesticks` has no fetch) and
the live collector's sink into this table; the route registration (report).

Earlier: `PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`
and both stated halves are already true.** `collectors/src/prediction/kalshi.rs`
exists, and `dataplane.source` row 13 `kalshi_rest` carries
`declared_vendor_lag_ms = 60000`. The remaining gap is that no readable
event-probability series or table exists.

The collector already exists. This task brings its history into the same PIT discipline as
everything else and records it in `dataplane.source` with a declared lag. Exposure only — event
probabilities as a readable series, no trading endpoints, no execution path.

**Files:** `crates/collectors/src/kalshi/` (existing), migration 0103 (source row).
**Acceptance:** Kalshi series are readable through the Data API with the cutoff applied and a
declared lag on the source row; no trading endpoint is reachable from this adapter, proven by a
test over its client surface.
**Traces:** DATA-005 §11 S7; DA-11.

### P-3.5 Text PIT rules, enforced — M
`PARTIAL` 2026-10-04 (track T-P) — **re-checked: `GET /api/data/text` is registered (`routes/mod.rs`), the scope constant lives in `auth/scopes.rs::TEXT_READER`, and all three clauses keep their named tests; the only thing left is a writer.** Nothing in this track's territory can supply one: the only text source is Alpaca news (P-3.3, `BLOCKED:HUMAN`, credentials). `text_as_of` has a production caller: the registered handler in `routes/data.rs`, not only tests.

`PARTIAL` 2026-10-04 (track F-DATA-T) — **unchanged: still waiting for a writer.** The only text source is Alpaca news (P-3.3, `BLOCKED:HUMAN`); `text_items` now exists in the live ClickHouse.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **all three clauses are mechanical now; the route needs registering and the table needs a writer.**
`text_items` (one row per revision) and `storage::clickhouse::alt::text_as_of` (highest revision with
`available_time <= as_of`). `GET /api/data/text` refuses 403 unless the token holds
`research:text.reader`, checks that **before** it reads (`the_text_handler_checks_the_scope_before_it_reads`),
and clips window and `as_of` to the project cutoff. Clause 1:
`clickhouse_alt_pit.rs::a_corrected_article_is_two_rows_and_reads_as_it_stood`. Clause 2:
`no_scope_but_the_reader_may_read_text` — one assertion per `RESEARCH_SCOPES` entry, so a scope added later
is checked by name. **Finding:** "DA-09, already enforced" was not true — no `text.reader` scope existed;
the constant lives in `routes/data.rs` (`TEXT_READER_SCOPE`) and should move to `auth/scopes.rs` (report).
**Missing:** route registration; a writer (Alpaca news, P-3.3).

Earlier: `NOT STARTED`

DATA-005 §10's rules made mechanical rather than documented: a text item's knowledge time is
its publication time; a revised item is a new row, not an update; a read as of *t* returns the
text as it stood at *t*; and text endpoints are callable only with the reader scope (DA-09,
already enforced — this task adds the *revision* half).

**Files:** `crates/api/src/routes/data.rs` (text endpoints), `crates/dataplane/src/text.rs`.
**Acceptance:** a corrected article produces two rows and an as-of read returns the version
current at that time; a non-reader scope receives 403 on every text endpoint; no text endpoint
returns an item whose publication time exceeds the project cutoff.
**Traces:** DATA-005 §10, DA-09; 06_DATA §7.

### P-3.6 Macro and fundamentals as PIT feature inputs — S
`PARTIAL` 2026-10-04 (track T-P) — **the feature runtime's external features now have a production caller.** `GET /api/data/features?name=macro_dgs10,...` (and `fundamental_<cik>_<tag>`) resolves such names with `features::external_features::resolve_external` (declaration checked, lag included in the lookback), loads the rows with `api::features_compute::load_external_rows` (ClickHouse `storage::clickhouse::alt::macro_vintages` — new, **every** vintage knowable by `as_of`, not one per observation — and `dataplane.fundamentals_fact`, every filing) and hands them to `backfill_column_external`; the runtime still reads nothing. Evidence: `storage` `clickhouse_alt_pit::the_feature_reader_returns_every_vintage_knowable_by_as_of` (live ClickHouse, 4 pass; mutation-checked: dropping the `knowledge_time <= as_of` filter fails it) and `api` `external_feature_inputs::a_column_over_a_revised_macro_series_and_a_restated_fact_steps_only_when_the_revision_is_knowable` (live PG + ClickHouse, ignored-by-default, env `EXTERNAL_INPUTS_*`): the column is absent before a vintage's day ends, the original before the revision, the revised value after, absent when the newest period reported missing; the fundamentals column steps from the original to the restated figure only after its acceptance. **Why still `PARTIAL`:** (1) the model-serving path (`features_compute::serve_vector`, fed by named feature sets) and the dataset materialiser still refuse external names, so a feature set cannot yet contain one (`features::feature_sets::is_known` rejects them on purpose: accepting them before the materialiser and `training_frame` can evaluate them would move the failure from registration to training); (2) the backtest runner and strategy-language feature resolution (Set U files: `crates/backtest/src/{runner,models}.rs`, `crates/strategy-engine/**`) do not call `resolve_external`. Exact edits are in the T-P report. No macro rows exist in the platform ClickHouse until P-3.1's FRED key (`BLOCKED:HUMAN`).

`PARTIAL` 2026-10-04 (track F-DATA-T) — **the feature runtime now holds external inputs.** `crates/features/src/external_features.rs` (new; 8 tests): a registered feature can declare an external input: `macro_<series>` (ALFRED) and `fundamental_<cik>_<tag>` (EDGAR, by acceptance) resolve through `resolve_external(name, bar_secs)`, whose declared lookback and knowledge lag include the source's publication lag and are checked by `check_declaration` before the feature is handed out (a 1d-lag series declares 25 bars on 1h bars, 2 on 1d). One evaluation routine, `evaluate_external`, takes the rows as arguments; `backtest_and_live_agree_at_every_bar` runs it with the whole table and with only the rows that had arrived at each bar and gets the same column; a series with nothing knowable is absent, not zero; the warm-up is refused like a windowed feature's. The windowed `runtime::feature` refuses these names and says where they live. `features` stays free of I/O crates. **Why PARTIAL, not DONE:** nothing in the backtest runner or the live server loads the macro/fundamental rows and hands them in (Set U's runner and the feature server are outside this track); that caller is the one remaining line of work.

Earlier: `PARTIAL` 2026-10-03 (track DATA) — **re-checked; the remaining work is in `crates/features`, outside DATA's territory.** The read paths the
feature engine must be *handed* (never fetch) now exist: `storage::clickhouse::alt::macro_as_of`
(vintage-correct) and `collectors::fundamentals::store::facts_as_of` (acceptance-correct). Not built:
`crates/features/src/inputs.rs` — the input type carrying a series and its declared lag, the lookback that
includes it, and the refusal test.

**Wave 2 (2026-10-03, track DATA):** `crates/features/src/external_inputs.rs` (5 tests): `ExternalInputs { macro_rows, facts }` are **handed in** (no I/O crate; `cargo tree -p features` shows no sqlx, clickhouse or redis) and resolved by `resolve(input, decision_time)` through the vintage- and acceptance-correct readers. A macro series whose newest vintage reports missing resolves to `Missing`, never to an older number or zero. Clause 1: `backtest_and_live_resolve_identically_at_every_decision_instant` (the full table against only the rows that had arrived). Clause 3: `required_lookback_bars` is window plus the largest source lag in whole bars, and `check_declaration` refuses a `FeatureDef` whose `lookback_bars` or `knowledge_lag_ms` omits it (`a_feature_that_omits_the_source_lag_is_refused`). Mutation-checked: reading as of the far future instead of `decision_time` fails two tests. **Why PARTIAL, not DONE:** nothing calls it. No registered feature declares an external input and the runtime does not pass `ExternalInputs`; wiring it into `registry` / `runtime` is the feature engine's track, and one more unreached module is this repository's dominant defect.

Earlier: `NOT STARTED`

The point of collecting these is to use them. Expose macro observations and fundamentals facts
to the one feature engine (`crates/features`) as vintage-correct inputs — arriving as
arguments, never fetched inside the engine, so the purity rule holds (`builders-purity` fails
the build if `sqlx`/`clickhouse`/`redis` enter `features`).

A feature built on a macro series declares the series and its vintage rule, and the declared
lookback includes the publication lag, so the warm-up requirement is honest.

**Files:** `crates/features/src/inputs.rs`, `crates/dataplane` read path.
**Acceptance:** a feature over a macro series computes identically in backtest and live serve;
`cargo tree -p builders --edges normal` stays clean of I/O crates; the declared lookback of such
a feature includes the source's declared lag, and a test asserts a feature that omits it is
refused.
**Traces:** DATA-006; INV-13, INV-14; DATA-005 §12.

---

## Phase exit

- Every alternative source stores vintages, and a read as of *t* returns what was knowable at
  *t*, each proven by a test over a planted revision.
- Corporate actions exist as read-time factors over unadjusted prices — INV-03 stops being
  vacuous the moment the first equity action lands.
- Free-plan history depths are recorded, so anything shallow joins the forward-collect argument
  rather than being assumed backfillable.
