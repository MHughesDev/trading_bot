# BS-007 / 06 — Data: Data API v2, point-in-time access, data quality, free sources

**Spec status:** spec-ready draft · **Req prefix:** DA- · **Formal spec:** DATA-005 ·
**ADR:** ADR-0025 (platform-side enforcement), ADR-0027 (free-only data) · **Plan set:** L
(API core) / P (sources) · **Decisions:** D-02, D-09, D-10, D-13 · **Gaps:** G-08, G-09,
G-13

---

## 1. Scope

This spec covers:
- how the agent (and the UI) reads data;
- point-in-time and holdout enforcement;
- data quality grading;
- as-of universes;
- text data rules;
- synthetic instruments;
- which **free** sources are added.

Collectors, NATS and the four-timestamp model are kept as they are (01 §2).

## 2. Data API v2

Data leaves the platform as **Parquet files into the workspace**, with a short summary
for the model's context. Endpoints under `/api/data/*`, with SDK `tbot.data.*` and CLI
`tbot data …`:

| Endpoint | Returns | Notes |
|---|---|---|
| `bars(instr, tf, start, end, as_of, kind=time\|volume\|dollar\|tick)` | Parquet | Any timeframe resampled server-side from 1m; information-driven bars built from trades where trades exist |
| `trades`, `quotes`, `book_snapshots` | Parquet | Only where collected (forward only for L2) |
| `funding`, `open_interest` | Parquet | Where a free source exists (§6) |
| `option_chain(underlying, as_of)` | Parquet with IV and Greeks computed server-side | Only from forward-collected snapshots (§6) |
| `prediction_markets(query, as_of)` | Parquet | Kalshi (existing collector) |
| `text(instrument?, query?, as_of, window)` | Handle for the quarantined reader | Social, news, filings; **reader scope only** (B-11, §7) |
| `fundamentals(ticker, as_of)` | Parquet | EDGAR XBRL, PIT by filing acceptance time |
| `macro(series, as_of)` | Parquet | FRED/ALFRED vintage-correct |
| `features(spec, instr, tf, window)` | Parquet | Same engine as backtests (07) |
| `universe(name, as_of)` | Member list | PIT membership; delisted instruments kept (§5) |
| `catalog()`, `coverage(instr)` | Small JSON | What exists; first and last timestamps, gaps, revisions, `data_qc` grade |

**CLI behaviour:**
- `tbot data bars BTC-USD 1h --start … --end …` writes
  `data/BTC-USD_1h_<hash>.parquet`;
- it prints ≤ 10 lines: rows, window, cutoff applied, `qc_grade`, gaps, path, handle;
- it never prints rows unless given `--head N`.

## 3. Point-in-time and the research cutoff

- **`as_of` everywhere:** every endpoint filters on `available_time ≤ as_of`. The default
  `as_of` is the request's `end`. Revisions are new versions and never overwrite; revised
  1m bars come from the `market.bars.1m.revised` lane.
- **Research cutoff per project** (D-10): research tokens receive data with
  `event_time ≤ cutoff`. The API clips `end` silently and states the clipping in the
  manifest. Only the **vault gate** (a platform service) evaluates past the cutoff, once
  per Experiment (Set J vault).
- **Cutoff default:** a project's cutoff defaults to `now − holdout_len`, with
  `holdout_len` set per project (for example the most recent 20% of available history,
  minimum 90 days for 1h crypto). It is immutable once any Experiment exists; changing it
  means cloning the project.
- **Manifests:** every response includes `{query, params, content hash, rows, schema,
  cutoff_applied, as_of, qc_grade, source versions}` and becomes an artifact (05 §6).
- **Ledger:** every call appends to the exploration ledger (05 §7).

### 3.1 The Desk project: live questions without touching any holdout

Questions about *now* ("what's ETH doing today?") need data past any research cutoff.
Letting a research project see it would contaminate that project's holdout, which is the
most recent period. So:
- **Every user has one Desk project with `cutoff = now`.** It answers live and recent
  questions (J1, J7), and is the only project where `tbot data live` works. Desk
  Experiments are **exploratory only**: they can run G0–G2 but can never reach G3 or the
  vault.
- **Research projects** refuse live and post-cutoff reads with a teachable error that
  points to the Desk.
- Desk sessions are logged in the user's exploration ledger. A research project's
  hypothesis on an instrument the user recently examined at the Desk is flagged
  `post_hoc` if the Desk session touched the same variables (11 §3).
- The agent routes a conversation to the Desk automatically when the question is about
  the present and no research project is named.

## 4. Token scopes on data

| Scope | Reads |
|---|---|
| `research:data.read` | All market data ≤ cutoff, catalog, coverage, universes, fundamentals, macro, features |
| `research:text.reader` | Text endpoints. Granted only to the quarantined reader sub-agent |
| `data.holdout` | Post-cutoff data. Held only by the vault-gate service; never by any token |

## 5. Data quality and universes

**`data_qc`** is a deterministic job per (instrument, timeframe, window), cached in the
instrument profile. It checks:
- coverage and gaps (count, longest gap, share of bars missing); revised-bar count;
- bad ticks: returns beyond k·σ that reverse on the next bar; zero or negative prices;
  OHLC inconsistency (H < max(O,C) or L > min(O,C));
- stale-price runs; volume zeros and spikes;
- timestamp integrity (`available_time ≥ event_time`, monotonic, timezone);
- cross-source consistency where more than one venue covers an instrument;
- survivorship flag (the instrument or universe was selected with hindsight).

**Grades:**
- **A/B** usable.
- **C** usable only with an explicit waiver recorded in the dossier, approved through the
  approvals inbox.
- **D** refused: Experiment creation fails.

The grade is attached to every manifest.

**As-of universes:** `universe(name, as_of)` returns membership as of that date.
Delisted and dead instruments stay in the catalog with their history. Universe strategies
MUST use as-of membership. Single-instrument research MUST acknowledge instrument
selection bias (11 T19).

## 6. Sources (free only, D-09)

**Kept (existing collectors, as configured):**
- Kraken crypto spot;
- Alpaca equities;
- Tradier options bars and quotes;
- Tradovate futures;
- OANDA FX;
- Kalshi;
- 0x DEX quotes;
- Reddit and web.

What's new is exposure through the Data API, and a forward-stored option **chain
snapshot** job built on the existing Tradier collector.

**Added (free; verify each provider's current terms, limits and US access before
building):**

| # | Source | Gives the agent | Notes |
|---|---|---|---|
| S1 | **Deep history backfill** from free public endpoints: Coinbase Exchange candles (paginated), Kraken public trades/OHLC history and its downloadable history files, Alpaca historical bars on the free plan | Years of 1m/1h/1d for existing instruments, so CPCV, walk-forward and regime work become possible | **Start now as an ops task.** BTC 1h had 22 bars at the 2026-09-08 check (G-13). Rate-limit friendly; resumable `backfill` jobs |
| S2 | **Alpaca News API and corporate actions** | Timestamped equity and crypto news; splits and dividends | Credentials exist; confirm news history depth on the free plan |
| S3 | **SEC EDGAR** (submissions, companyfacts/XBRL) | Fundamentals and filing events, PIT by acceptance time | Fair-access rules: declared User-Agent, ≤ 10 req/s |
| S4 | **FRED / ALFRED** | Rates, inflation, macro series with vintages | Free API key; ALFRED gives vintage-correct history |
| S5 | **Crypto options public market data** (e.g. Deribit public API) | BTC/ETH chains, IV, Greeks, forward-collected | Confirm access terms from the US. Market data only, never trading endpoints |
| S6 | **Crypto funding and open interest** from a free, US-accessible public API (e.g. Kraken Futures) | Carry and crowding series | Check geo-restrictions (Binance returned 451 earlier, per memory) |
| S7 | **Kalshi history** (existing collector) | Event probabilities | Exposure only |

**Out of scope (paid):** equity options history (OPRA vendors), paid fundamentals,
purchased tick and L2 history.

**Consequences:**
- equity options research uses forward-collected Tradier snapshots only;
- L2/OFI research uses forward-collected snapshots only;
- the `ofi_features` and VRP archetypes wait until enough forward data accumulates.

## 7. Text data rules

- Store `published_at`, `ingested_time` and `available_time`. Every search takes `as_of`
  and filters on `available_time`. Revisions are versions.
- Collapse near-duplicates by embedding similarity. Novelty = 1 − max similarity to the
  entity's items in the prior N days.
- **Contamination:** label every text-derived feature with the extracting model and its
  knowledge cutoff. Backtests of text features over periods before the model's cutoff are
  labelled `contaminated`. Forward paper evidence is preferred. `contamination_probe`
  is available (research/R2 §9).
- Text reaches the main agent only as **typed extractions** from the quarantined reader
  (03 §9).
- Embeddings: the local model (12 §6).

## 8. Synthetic instruments

A `synthetic` venue registers generated series as first-class, backtestable instruments,
with the same Data API, `data_qc` and backtest paths. Generators:
- GARCH-t;
- jump-diffusion;
- regime-switching;
- planted-edge variants (AR(1), hour-of-day drift, vol-breakout drift);
- later, diffusion generators that must pass a stylized-facts check.

Used by the eval suite (16), `skill_verify` (13) and `SyntheticPaths` studies.

## 9. Requirements

| ID | Requirement | Pri |
|---|---|---|
| DA-01 | The Data API MUST return Parquet to the workspace with a manifest, and a ≤ 10-line CLI summary by default | MUST |
| DA-02 | Every endpoint MUST filter on `available_time ≤ as_of` and never overwrite revisions | MUST |
| DA-03 | Research tokens MUST NOT receive data past the project cutoff; clipping MUST be stated in the manifest | MUST |
| DA-04 | Only the vault-gate service MAY read post-cutoff data | MUST |
| DA-05 | The cutoff MUST be immutable once an Experiment exists in the project | MUST |
| DA-06 | Every call MUST append to the exploration ledger | MUST |
| DA-07 | `data_qc` MUST implement §5 checks and grades. Experiments on grade D MUST be refused; grade C MUST need an approved waiver | MUST |
| DA-08 | `universe(name, as_of)` MUST return PIT membership, and delisted instruments MUST be retained | MUST |
| DA-09 | Text endpoints MUST be callable only with the reader scope, and MUST return PIT-filtered results | MUST |
| DA-10 | Server-side resampling to any timeframe and information-driven bars MUST be supported where the inputs exist | MUST |
| DA-11 | Sources S1–S4 MUST be implemented as resumable, rate-limited collector or backfill jobs; S5–S7 SHOULD follow | MUST/SHOULD |
| DA-12 | A forward-stored option chain snapshot job (IV and Greeks computed) SHOULD run on the existing Tradier collector | SHOULD |
| DA-13 | The `synthetic` venue MUST support the §8 generators with seeds recorded | MUST |
| DA-14 | No paid data source MAY be added without revisiting D-09 | MUST |
| DA-16 | Bars MUST be stored append-only in `market_bars_v2`, with `timeframe` in the sorting key and no engine that collapses rows. Revisions MUST be retained, not overwritten, so `as_of` reads can pick a revision **(E)** | MUST |
| DA-17 | The v2 cutover MUST be verified row-for-row: every row of `market_bars` present in v2, checked by an order-independent checksum over all columns. Writers MUST move to v2 before any bar backfill runs, and `market_bars` MUST become read-only at cutover **(E)** | MUST |
| DA-15 | Each user MUST have a Desk project (cutoff = now, exploratory only, no G3 or vault); live and post-cutoff reads MUST be refused in research projects (§3.1) | MUST |

## 10. Acceptance criteria

1. A research token requesting bars through `now` receives data up to the cutoff, and the
   manifest states `cutoff_applied`. The same request by the vault service returns the
   full range.
2. `data_qc` on a series with planted bad ticks, stale runs and a gap grades it
   correctly, and Experiment creation on a grade-D series fails with a teachable error.
3. After backfill, BTC-USD and ETH-USD have ≥ 3 years of continuous 1m history, graded A
   or B.
4. An EDGAR fundamentals request as of date *t* never returns a value filed after *t*.
5. A synthetic planted-edge instrument can be backtested exactly like a real one.

## 11. Open questions

1. Default `holdout_len` per timeframe and asset class.
2. Retention for forward-collected L2 and option snapshots (disk budget).
3. Which free crypto funding source is reliably US-accessible (S6).
