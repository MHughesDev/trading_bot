# Phase 3 — Alternative data

**Completion: 0% (0 / 6 tasks)**

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`PARTIAL` — **corrected 2026-09-21 by audit; the marker said `NOT STARTED`
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
`NOT STARTED`

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
`NOT STARTED`

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
