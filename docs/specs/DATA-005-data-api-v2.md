# DATA-005: Data API v2, Point-in-Time Access, Data Quality, and Free Sources

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0025 (research cutoff, Desk, scopes), ADR-0027 (free-only sources);
builds on ADR-0008 (available_time ordering), ADR-0004 (storage split)
**Derived from:** BS-007 [06_DATA](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD)
**Plan set:** L (API core, cutoff, qc, universes, synthetic, Desk); P (new sources)
**Crates:** `crates/api` (new `data` route module), `crates/storage::clickhouse`,
`crates/collectors` (new source adapters), `crates/jobs` (backfill, qc workers),
`clickhouse/06_*` DDL, `migrations/0038_*`

---

## 1. Purpose

The single way the agent, the UI and every other client read market and alternative
data. The API:
- writes Parquet to the caller, with a manifest;
- is point-in-time correct;
- clips at the project's research cutoff;
- attaches a data-quality grade;
- logs every read to the exploration ledger.

## 2. Scope and non-goals

**In scope:**
- endpoints;
- PIT, revision and cutoff semantics;
- the Desk project's data rules;
- `data_qc`;
- as-of universes;
- the synthetic venue;
- text PIT rules;
- free-source adapters and backfill.

**Non-goals:**
- feature computation (DATA-006; exposed here only through `features`);
- job mechanics (COMP-005);
- the agent-side CLI (AGENT-002).

## 3. Pre-existing schema issues this spec must resolve (verify first)

`clickhouse/02_bars.sql` defines `market_bars` as
`ReplacingMergeTree(revision) ORDER BY (instrument_id, available_time)`.

1. **Cross-timeframe collapse risk:** `timeframe` isn't in the sorting key. Rows for
   different timeframes of the same instrument with the same `available_time` (for
   example the 1m and 1h bars closing at 10:00) are treated as duplicates and collapsed
   on merge. **Verify with a ClickHouse query before Set L.** If confirmed, this is a
   data-loss bug independent of this spec, and may explain the thin 1h history observed
   on 2026-09-08 (BS-007 G-13).
2. **Revisions are overwritten:** "latest revision wins after merge" destroys the
   revision history that `as_of` queries need.

**Required change (migration `clickhouse/06_market_bars_v2.sql`):**

```sql
CREATE TABLE market_bars_v2 (
  -- same columns as market_bars, plus:
  bar_open_time   DateTime64(9,'UTC'),
  event_time      DateTime64(9,'UTC'),        -- bar close (window_close)
  snapshot_id     UInt64 MATERIALIZED toUnixTimestamp64Nano(ingested_time)
) ENGINE = MergeTree                            -- append-only; no replacing
ORDER BY (instrument_id, timeframe, event_time, revision, ingested_time)
PARTITION BY (timeframe, toYYYYMM(event_time));
```

- **Latest-as-of reads** pick, per `(instrument_id, timeframe, event_time)`, the row
  with the greatest `(revision, ingested_time)` among rows with
  `available_time ≤ as_of` (`argMax` or `LIMIT 1 BY`).
- A backfilling migration copies `market_bars` into `market_bars_v2`. Writers switch to
  v2. `market_bars` stays read-only until it's retired.
- `market_trades` has the same pattern with `dedup_key` in its sorting key. It's
  acceptable for trades, since trades aren't revised; keep it.

## 4. Point-in-time, cutoff and Desk semantics

| Concept | Rule |
|---|---|
| `as_of` | Every endpoint filters on `available_time ≤ as_of`. The default is `as_of = end`. Revisions are selected per §3 |
| Research cutoff | For tokens bound to a **research** project, the effective `end = min(end, cutoff)` and `as_of = min(as_of, cutoff)`. Clipping is silent in data and explicit in the manifest (`cutoff_applied`) |
| Cutoff default | At project creation, `cutoff = now − holdout_len`, where `holdout_len` defaults per timeframe class (config `data.holdout_defaults`, e.g. 1m–1h crypto: max(20% of available history, 90 d)). It is immutable once the project has an Experiment (enforced in DB and API) |
| Vault reads | Only the vault-gate service (`data.holdout` internal capability, never in a token) reads post-cutoff data, once per Experiment, through Set J's vault |
| Desk project | `cutoff = NULL` (= now). `live` endpoints are allowed. Desk Experiments are refused at G3 and the vault (`403 desk_exploratory_only`) |
| Live reads in research projects | `live` and any request with `end > cutoff` beyond clipping semantics (e.g. `latest=true`) → `403 live_data_desk_only` with a fix pointing to the Desk |
| Data snapshot | Each response records `data_snapshot_id` = the max `ingested_time` visible to the query. Jobs pin it (COMP-005 §4) so re-runs see identical data |

## 5. Endpoints (`/api/data/*`)

All endpoints return either a Parquet artifact (`?format=parquet`, the default for
data) or small JSON (catalog and metadata).

**Parquet responses** are stored as `parquet_extract` artifacts (COMP-005 §8). The
response body is `{handle, manifest, summary}`. The `summary` is ≤ 10 lines of text:
- rows;
- window;
- `cutoff_applied`;
- `qc_grade`;
- gap count;
- the handle.

The SDK downloads the bytes to `/workspace/data/`.

| Endpoint | Params | Scope | Notes |
|---|---|---|---|
| `GET /api/data/catalog` | `asset_class?`, `q?` | `research:data.read` | Instruments with coverage summary and qc grade |
| `GET /api/data/coverage/{instrument}` | — | same | Per timeframe/source: first, last, gaps, revisions, qc grade, last qc run |
| `GET /api/data/search` | `q`, `asset_class?` | same | Ranked instrument search (liquidity, coverage) |
| `GET /api/data/bars` | `instrument, tf, start, end, as_of?, kind=time|volume|dollar|tick, threshold?` | same | `tf` any multiple of 1m (server-side resample from 1m v2 bars, close-stamped); information-driven bars from `market_trades` |
| `GET /api/data/trades` | `instrument, start, end, as_of?` | same | Where collected |
| `GET /api/data/quotes`, `/book` | `instrument, start, end, depth?` | same | Forward-collected only |
| `GET /api/data/funding`, `/open_interest` | `instrument, start, end, as_of?` | same | Source S6 |
| `GET /api/data/option_chain` | `underlying, as_of` | same | From snapshots; IV and Greeks computed server-side (Black–Scholes/Black-76, rates from FRED) |
| `GET /api/data/prediction_markets` | `q, as_of` | same | Kalshi |
| `GET /api/data/fundamentals` | `ticker, fields?, as_of` | same | EDGAR XBRL facts with `accepted_at ≤ as_of` |
| `GET /api/data/macro` | `series, start, end, as_of` | same | ALFRED vintage as of `as_of` |
| `GET /api/data/universe/{name}` | `as_of` | same | PIT membership (§8) |
| `GET /api/data/features` | `spec|name, instrument, tf, start, end` | `research:features` | Via DATA-006 batch mode |
| `GET /api/data/live/{instrument}` | — | same (**Desk only**) | Last mark, bid/ask/spread, 24h range and volume, last-bar age |
| `POST /api/data/text/request` | `q, instrument?, as_of, window` | `research:data.read` | Returns a `text_request_id`; the orchestrator fetches with the reader token and runs the reader (AGENT-001 §13) |
| `GET /api/data/text/{request_id}` | — | `research:text.reader` | Raw text, PIT-filtered; reader only |

Every call appends to the exploration ledger with instruments, window and variables
(COMP-005 §9).

## 6. Manifests

Every artifact manifest (COMP-005 §8.2) from this API includes:
- the endpoint and normalised params;
- `as_of`;
- `cutoff_applied`;
- `data_snapshot_id`;
- `qc_grade`;
- source and venue ids;
- revision policy (`latest_as_of`);
- row count;
- schema.

Numeric columns in Parquet are `decimal128(38,10)` for prices and volumes (ADR-0002),
and `float64` only for derived statistics.

## 7. `data_qc`

A deterministic `data_qc` job per `(instrument, timeframe, window)`. Results are stored
in `data_qc_reports` (Postgres; migration 0038) and cached into the instrument profile.

| Check | Rule (defaults in `config/data_qc.toml`) |
|---|---|
| Coverage | Expected bars vs present; gap count; longest gap; % missing |
| Revisions | Count of revised bars in the window |
| Bad ticks | abs(log return) > 8·σ_rolling that reverses ≥ 80% on the next bar; price ≤ 0; H < max(O,C) or L > min(O,C) |
| Stale runs | ≥ N consecutive identical closes with zero volume (N per timeframe) |
| Volume | Zero-volume share; spikes > 20× rolling median |
| Timestamps | `available_time ≥ event_time`; monotonic; timezone consistency |
| Cross-source | Median abs deviation vs a second venue where one exists |
| Survivorship | Flag if the instrument or universe membership is post-selected (catalog metadata) |

**Grades:**
- **A:** all pass.
- **B:** minor, meaning gaps < 0.5% and no bad ticks unrepaired.
- **C:** material (gaps < 5% or repaired bad ticks); usable only with an approved
  waiver (`approval_requests.kind='qc_waiver'`).
- **D:** refused, meaning gaps ≥ 5%, unrepaired bad ticks, or timestamp violations.

The grade is attached to every manifest. `POST /api/hypotheses` and
`POST /api/backtest/experiments` refuse grade D, and grade C without a waiver
(`422 data_quality_insufficient`, listing the reasons).

## 8. As-of universes

- **Tables** (migration 0038):
  - `universes` (name, definition, rule);
  - `universe_membership` (name, instrument_id, valid_from, valid_to, reason).
- Delisted and dead instruments remain in `instruments` with their history, and
  `status='delisted'`.
- `GET /api/data/universe/{name}?as_of=` returns the members valid at `as_of`.
- Universe-based Experiments must declare a universe and are evaluated on as-of
  membership (BACKTEST_SUITE_CORE_SPEC v2).

## 9. Synthetic venue

- `venue_id = 'synthetic'`, with instruments `SYN-<generator>-<seed>`.
- Generators, parameterised and seeded:
  - `garch_t`;
  - `merton_jump`;
  - `regime_switch`;
  - `planted_ar1`;
  - `planted_hour_drift`;
  - `planted_vol_breakout`;
  - `planted_carry`.
- `POST /api/data/synthetic` (`{generator, params, seed, tf, length}`) submits a
  `simulate_paths` job that writes bars to `market_bars_v2` with `source='synthetic'`
  and registers the instrument.
- Synthetic instruments flow through every endpoint, `data_qc` and backtests exactly
  like real ones.
- The generator and params are recorded in the instrument's catalog metadata, and are
  hidden from agent tokens during eval tasks (AGENT-004).

## 10. Text PIT rules

- Text rows store `published_at`, `ingested_time` and `available_time`. Queries filter
  on `available_time ≤ as_of`. Revisions are new rows.
- Near-duplicates are collapsed by embedding similarity (local model, AGENT-003).
  Novelty = 1 − max cosine to the entity's items in the prior N days.
- Text-derived features carry `{extractor_model, extractor_cutoff}`. Backtests over
  periods before `extractor_cutoff` are labelled `contaminated` in their manifests.

## 11. Sources (free only, ADR-0027)

| ID | Source | Adapter | Mode | Notes (verify terms before building) |
|---|---|---|---|---|
| S1 | Deep backfill: Coinbase Exchange candles, Kraken public OHLC/trades history (and downloadable history files), Alpaca free-plan historical bars | `collectors::backfill::{coinbase,kraken,alpaca}` | `backfill` jobs, resumable, rate-limited, idempotent per (instrument, tf, window) | **Ops task, starts in Phase 0.** Target ≥ 3 years 1m for BTC-USD and ETH-USD, then initialised assets |
| S2 | Alpaca News API, corporate actions | `collectors::news::alpaca`, `…::corp_actions` | Forward stream + backfill | Confirm news history depth on the free plan |
| S3 | SEC EDGAR submissions + companyfacts/XBRL | `collectors::fundamentals::edgar` | Backfill + daily | Declared User-Agent; ≤ 10 req/s |
| S4 | FRED / ALFRED | `collectors::macro::fred` | Backfill + daily | Free API key (stored in the credential store) |
| S5 | Crypto options public market data (e.g. Deribit public API) | `collectors::options::crypto_public` | Forward snapshots (e.g. hourly chains) | Market data endpoints only; confirm US access terms |
| S6 | Crypto funding/OI from a free US-accessible venue (e.g. Kraken Futures public) | `collectors::derivs::funding` | Backfill (where offered) + forward | Check geo-restrictions |
| S7 | Kalshi history | Existing collector | Exposure only | — |
| S8 | Equity option chain snapshots on the existing Tradier collector | `collectors::options::tradier_chain_snapshot` | Forward snapshots | No paid history |

**Out of scope:** paid feeds. Adding one requires a new ADR (ADR-0027).

## 12. Storage additions

| Store | Additions |
|---|---|
| ClickHouse | `market_bars_v2` (§3); `funding_rates`; `open_interest`; `option_chain_snapshots` (with computed IV/Greeks); `text_items` (if not already in Postgres); `macro_observations` (series, observation_date, vintage_date, value) |
| Postgres (0038) | `data_qc_reports`; `universes`; `universe_membership`; `fundamentals_facts` (cik, tag, value, unit, period, accepted_at); `backfill_state` (source, instrument, tf, cursor) |

## 13. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| D1 | A research token requesting bars through now gets data to the cutoff with `cutoff_applied`; the vault service gets the full range | DA-02, DA-03, DA-04 |
| D2 | A cutoff change after an Experiment exists → `409` | DA-05 |
| D3 | Every data call creates an exploration-ledger row with its variables | DA-06 |
| D4 | Planted bad ticks, stale runs and a gap are graded correctly; an Experiment on grade D → `422` | DA-07 |
| D5 | A universe as of 2023 includes a since-delisted token | DA-08 |
| D6 | Text endpoints deny non-reader tokens | DA-09 |
| D7 | 4h and dollar bars resample correctly vs a reference implementation | DA-10 |
| D8 | After S1, BTC-USD and ETH-USD have ≥ 3 years of 1m bars graded A/B | DA-11 |
| D9 | EDGAR `as_of=t` never returns a fact accepted after `t` | DA-11 |
| D10 | A synthetic planted-edge instrument backtests like a real one | DA-13 |
| D11 | A live read in a research project → `403 live_data_desk_only`; the Desk answers; a Desk Experiment is refused at G3 | DA-15 |
| D12 | Cross-timeframe bars with an equal `available_time` both survive in `market_bars_v2`; a revised bar is returned only for `as_of` after its availability | §3 |

## 14. Open questions

1. `holdout_len` defaults per timeframe and asset class.
2. Retention for forward L2 and option snapshots (disk budget).
3. Which free funding source is reliably US-accessible (S6).

## 15. Traceability

Implements BS-007 DA-01…DA-15 and supports EV-02 (qc gate) and AE-01 (synthetic venue).
