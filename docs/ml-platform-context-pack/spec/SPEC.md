# Data Architecture & MLOps Specification
### Quantitative ML Trading Platform · v1.0

**Scope:** the data plane and the ML operations layer. Not the trading engine, not the broker integration, not the general application stack. Where this spec touches UI it is only to specify what the MLOps layer must expose.

**Context locked from Mason:** minute bars · crypto (24/7, venue-fragmented) + futures + options + ETFs/equities + DeFi pools · multi-tenant SaaS where every user is trader, quant and ML engineer · real capital routed to Coinbase/Alpaca · compute is not a constraint · all engineering decisions are mine.

**Research base:** 7 deep research passes, ~72,000 words, ~500 cited sources. Findings that changed my mind since the brief are marked **[REVISED]**.

---

## 0. The four irreversible decisions

Everything else in this document is a rewrite you can afford. These four are *data loss* — get them wrong and the information needed to fix them no longer exists.

**0.1 — Four timestamps at ingest, and `knowledge_time` as a real column. [REVISED]**
I said "bitemporal" in the brief. That was underspecified. You need four:

| Column | Meaning |
|---|---|
| `event_time` | the bar interval the fact describes |
| `venue_ts` | the exchange/chain timestamp |
| `ingest_time` | when your system received the bytes |
| `knowledge_time` | when the fact became **queryable** by a strategy |

Backtests filter on `knowledge_time`, never `ingest_time`. A batch that arrived at 09:00 but committed at 11:30 was not actionable at 10:00.

**And: Iceberg/Delta snapshots are not a bitemporal model.** Compaction creates new snapshots with new commit times while preserving old rows, so the snapshot timeline stops mapping 1:1 to knowledge events. Snapshot time travel is for *artifact pinning*, not PIT semantics. `knowledge_time` is a column you write, or you don't have it.

**0.2 — Store unadjusted prices plus event-based adjustment factors, keyed on surrogate instrument IDs.**
An adjusted close is a function of the entire *future* corporate-action stream. A split silently mutates all history, invalidates every cache, and breaks content-addressing — with no schema change and no error. You cannot recover unadjusted from adjusted. Tickers are also recycled aggressively after delisting, so `symbol` is not a key.

**0.3 — One feature code path, with online/offline consistency logging from day one.**
Once backfill and streaming are two implementations they diverge permanently, and "unify the pipelines" becomes perpetually next quarter. Unification is only achievable *before the second implementation exists*. Log every served feature vector immediately; nightly recompute and diff. You cannot measure consistency retroactively against logs you never wrote.

**0.4 — Propensity-logged, hash-chained Trial Ledger, written before results are returned.**
Without logged propensities the ledger is a self-selected sample and no internal model trained on it is valid. Without write-before-results, trial counting is optional and therefore fictional. Without hash chaining, "immutable" is a promise rather than a property.

**Plus one landmine that belongs here because it is silent and irreversible:**
Iceberg's `history.expire.max-snapshot-age-ms` defaults to **5 days**, `min-snapshots-to-keep` to **1**. Ship with defaults, run routine maintenance, and every experiment older than five days becomes irreproducible — no error, just missing snapshots. **Set ≥90 days / ≥50 snapshots before the first experiment runs, and create an Iceberg tag per registered artifact transactionally at registration.**

---

# PART I — DATA ARCHITECTURE

## 1. L0 — The market data plane

### 1.1 Identity model

```sql
-- Surrogate identity. Symbols are attributes, never keys.
CREATE TABLE instrument (
  instrument_id     BIGINT PRIMARY KEY,          -- opaque, never reused
  asset_class       TEXT NOT NULL,               -- equity|etf|future|option|crypto|pool
  base_instrument_id BIGINT,                     -- underlying for options/futures/perps
  first_seen        TIMESTAMPTZ NOT NULL,
  last_seen         TIMESTAMPTZ,
  static_attrs      JSONB                        -- immutable facts only
);

-- Symbols are bitemporal facts about an instrument.
CREATE TABLE instrument_symbol (
  instrument_id  BIGINT NOT NULL,
  venue_id       INT NOT NULL,
  symbol         TEXT NOT NULL,
  valid_from     TIMESTAMPTZ NOT NULL,   -- event time
  valid_to       TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (instrument_id, venue_id, symbol, valid_from, knowledge_time)
);

-- Venue is a first-class coordinate, not metadata. [REVISED — see §6.3]
CREATE TABLE venue (
  venue_id INT PRIMARY KEY, name TEXT, chain_id INT,
  session_model TEXT,        -- continuous_24_7 | rth_plus_ext | chain_block
  calendar_id TEXT,          -- pinned exchange_calendars version
  tick_size_series_id BIGINT, lot_size_series_id BIGINT,
  quality_tier SMALLINT      -- 1=primary, 2=usable, 3=reference-only
);
```

**Why venue is first-class:** the most relevant 2026 study (3.4M minute observations, 6 crypto pairs, Binance spot + perp) found models transfer well between spot and futures of the *same* asset, but not across assets. If that generalizes, the useful neighborhood metric is *(instrument family, venue)* — and venue identity must be a stored coordinate rather than something an embedding is expected to rediscover.

### 1.2 The canonical minute bar

One physical table shape for all asset classes. Asset-class specifics live in sidecar tables, not in a wide union schema.

```sql
CREATE TABLE bar_1m (
  instrument_id  BIGINT NOT NULL,
  venue_id       INT    NOT NULL,
  event_time     TIMESTAMP(9) NOT NULL,   -- bar OPEN, UTC, ns precision
  venue_ts       TIMESTAMP(9),
  ingest_time    TIMESTAMP(9) NOT NULL,
  knowledge_time TIMESTAMP(9) NOT NULL,
  open DECIMAL(38,18), high DECIMAL(38,18),
  low  DECIMAL(38,18), close DECIMAL(38,18),
  volume DECIMAL(38,18),
  trade_count INT,
  vwap  DECIMAL(38,18),
  -- microstructure, null where unavailable
  bid_close DECIMAL(38,18), ask_close DECIMAL(38,18),
  bipower_var DOUBLE, n_updates INT,
  quality_flags INT NOT NULL DEFAULT 0,   -- bitmask, see §1.7
  revision_seq  INT NOT NULL DEFAULT 0,
  source_id     INT NOT NULL
);
```

Notes that are not cosmetic:
- **`TIMESTAMP(9)`.** Iceberg v3 added `timestamp_ns` explicitly motivated by trading. Microsecond truncation silently reorders same-microsecond microstructure events. Even at minute bars, the sidecar tick/quote data needs it and mixing precisions across tables is a permanent irritation.
- **`DECIMAL`, not `DOUBLE`, for prices.** Crypto prices span 10⁻⁸ to 10⁵ and binary floating point will not round-trip them. This is a correctness issue in PnL reconciliation against a broker.
- **Bar timestamp is the OPEN, UTC, always.** Half of all financial data bugs are bar-convention bugs. One convention, documented, enforced at ingest.
- **Sparse storage: no empty bars.** Materializing every minute for every instrument costs 90–95% of rows for zero information. Absence means "no trades"; the query layer densifies against the master clock (§2).

### 1.3 Restatement resolution, and why it is fast

PIT-correct reads require, per `(instrument_id, event_time)`, the row with the greatest `knowledge_time ≤ as_of`. Done naively as a window function over 10 years, this is 5–20× slower than a non-PIT read.

The fix: restatements are *rare*. Maintain a small index of which partitions contain any.

```sql
CREATE TABLE restatement_index (
  instrument_id BIGINT, event_date DATE,
  n_revisions INT, max_knowledge_time TIMESTAMP(9),
  PRIMARY KEY (instrument_id, event_date)
);
```

Query planning rule: if `(instrument_id, event_date)` is absent from `restatement_index`, skip resolution entirely and read the single row. In practice >99% of partitions take this path, which keeps PIT reads within ~20% of non-PIT reads instead of 5–20× slower. **This makes PIT correctness cheap enough that there is never an excuse to offer a non-PIT read path.** There is no non-PIT read path.

### 1.4 Equities / ETFs

```sql
CREATE TABLE corporate_action (
  instrument_id BIGINT, action_type TEXT,       -- split|dividend|spinoff|merger|symbol_change
  announcement_time TIMESTAMP(9) NOT NULL,      -- when the market learned
  ex_date DATE NOT NULL, effective_time TIMESTAMP(9) NOT NULL,
  price_factor DECIMAL(38,18),                  -- multiplicative
  volume_factor DECIMAL(38,18),
  cash_amount DECIMAL(38,18), currency TEXT,
  knowledge_time TIMESTAMP(9) NOT NULL,
  revision_seq INT DEFAULT 0
);

CREATE TABLE index_membership (
  index_id INT, instrument_id BIGINT,
  announcement_time TIMESTAMP(9),   -- reconstitution is announced BEFORE it happens
  effective_from TIMESTAMPTZ, effective_to TIMESTAMPTZ,
  weight DECIMAL(18,10), knowledge_time TIMESTAMP(9)
);
```

Adjustment is applied **at read time** by composing factors with `knowledge_time ≤ as_of`. Never at write time. Two independent reasons: history stays immutable so content-addressed dataset hashes remain stable forever, and the announcement-vs-effective distinction (which is where index-reconstitution alpha actually lives) survives.

Halts, LULD bands, auction prints and odd-lot indicators go in a `session_event` sidecar keyed the same way.

### 1.5 Futures

Two tables, and the derived one is explicitly labelled a synthetic.

```sql
CREATE TABLE future_contract (
  instrument_id BIGINT PRIMARY KEY, root TEXT,  -- 'ES'
  expiry DATE, first_notice DATE, last_trade DATE,
  contract_size DECIMAL(38,18), tick_value DECIMAL(38,18)
);

CREATE TABLE roll_schedule (
  root TEXT, roll_rule_id INT,          -- calendar | oi_crossover | volume_crossover
  from_instrument_id BIGINT, to_instrument_id BIGINT,
  roll_event_time TIMESTAMP(9),
  decision_time TIMESTAMP(9) NOT NULL,  -- when the rule could FIRE, not when it applies
  knowledge_time TIMESTAMP(9) NOT NULL,
  ratio_factor DECIMAL(38,18)
);
```

Three rules:
1. **Back-adjusted continuous series are not prices.** Their entire history changes at every roll, and Panama/additive adjustment can go negative, which breaks log returns outright. Store raw per-contract bars; build continuous series as a *view*.
2. **`decision_time` is mandatory.** Roll rules based on open interest or volume have publication lag. Rolling on same-day OI is look-ahead — a small, extremely common, extremely profitable-looking bug.
3. **Forward-adjusted ratio is the default continuous method.** It leaves history immutable, so a backtest run in 2026 still reproduces bit-for-bit in 2031. Back-adjustment is available but flagged `non_reproducible=true` on any dataset that uses it, and that flag propagates into the trial ledger.

### 1.6 Options — the only real storage decision  *[AMENDED 2026-09-14 — Appendix C, C-1]*

Scale: ~1.6M live OPRA instruments, >200B updates/day, 50 Gbps bursts. Full-chain minute bars ≈ **7 TB per 10 years**; a moneyness/DTE-gated liquid universe ≈ **1.3 TB**.

```sql
CREATE TABLE option_contract (
  instrument_id BIGINT PRIMARY KEY, underlying_id BIGINT,
  expiry DATE, strike DECIMAL(38,18), right CHAR(1),   -- C|P
  exercise_style CHAR(1), multiplier INT,
  occ_symbol TEXT, adjusted_flag BOOLEAN     -- post-corporate-action nonstandard deliverable
);

CREATE TABLE option_bar_1m (            -- LONG, not wide. Sparse.
  instrument_id BIGINT, event_time TIMESTAMP(9),
  knowledge_time TIMESTAMP(9), ingest_time TIMESTAMP(9),
  open DECIMAL, high DECIMAL, low DECIMAL, close DECIMAL,
  volume BIGINT, open_interest BIGINT,       -- OI is T+1. knowledge_time proves it.
  bid_close DECIMAL, ask_close DECIMAL,
  underlying_close DECIMAL,                   -- denormalized: joins at this cardinality are ruinous
  iv_close DOUBLE,                            -- STORE IV
  iv_model TEXT, iv_rate DOUBLE, iv_div DOUBLE,
  moneyness DOUBLE, dte INT                   -- materialized for partition pruning
);
```

**Store implied volatility; do not store greeks. [REVISED]** Greeks are deterministic functions of (IV, S, K, T, r, q) — storing them freezes one model choice into your data forever and multiplies width by five. IV is *not* cheaply reproducible later because it depends on the rate and dividend curves as known *at that moment*, which is why `iv_model`, `iv_rate` and `iv_div` are stored alongside it. Greeks are computed at read time.

**Universe gating is a first-class, bitemporal object**, because "which options were liquid enough to trade" is itself a PIT fact:
```sql
CREATE TABLE option_universe_membership (
  universe_id INT, instrument_id BIGINT,
  valid_from TIMESTAMP(9), valid_to TIMESTAMP(9), knowledge_time TIMESTAMP(9)
);
```
Default gate: `|moneyness - 1| ≤ 0.30 AND dte BETWEEN 1 AND 400 AND has_two_sided_quote`. Everything outside it is retained in cold tier, not deleted — someone will eventually want to study the tails, and by then it would be gone.

Surface parameterizations (SVI/SABR fits) are **L1 features**, never L0. They are model output.

### 1.7 Crypto

No sessions, fragmented venues, and data quality that varies by orders of magnitude between exchanges.

- **No canonical cross-venue price is stored as a fact.** A consolidated price is an L1 feature with an explicit, versioned construction rule (volume-weighted across `quality_tier=1` venues with staleness bounds). Storing it as L0 would fabricate a fact that no one could have traded.
- **Perpetual funding** is its own bitemporal series (`predicted` and `realized` rates carry different knowledge times — the predicted rate is tradeable information, the realized one is not, until it is).
- **Wash trading and venue quality**: `quality_flags` bitmask includes `SUSPECT_VOLUME`, `STALE_QUOTE`, `CROSSED_BOOK`, `VENUE_OUTAGE`. Flags are never silently filtered; they are exposed and the dataset spec declares which flags it excludes, so the exclusion becomes part of the content hash.
- **Listing/delisting** is bitemporal membership like index membership. Crypto survivorship bias is worse than equity survivorship bias and nobody corrects for it.
- **Intraday seasonality is real but not U-shaped.** Saturday volatility runs 30–50% below average, the peak is ~16:00 UTC, and there are sharp minute-of-hour spikes at :00/:15/:30/:45 driven by perpetual funding settlement. **Deflate by a venue-specific seasonal profile before computing any volatility feature**, or your asset clustering will group instruments by funding schedule and you will think you have found something.

### 1.8 DeFi pools — the purest bitemporal case

Reorgs literally rewrite history. ~1% of Ethereum blocks reorg; Polygon has seen 157-block depth.

```sql
CREATE TABLE chain_block (
  chain_id INT, block_number BIGINT, block_hash BYTEA,
  parent_hash BYTEA, block_time TIMESTAMP(9),
  finalized_at TIMESTAMP(9),          -- NULL until finality
  orphaned_at TIMESTAMP(9),           -- NOT NULL ⇒ reorged out
  PRIMARY KEY (chain_id, block_number, block_hash)   -- hash is part of the key
);
```
Every pool observation keys on `(chain_id, block_number, block_hash)`. `knowledge_time` is when your indexer saw it; the row becomes *trustworthy* at `finalized_at`. A strategy's PIT read must specify its finality policy (`n_confirmations` or `finalized_only`) and that policy is part of the dataset spec hash.

**Do not treat DeFi pool prices as a cheap price feed.** Uniswap correlates only **0.328** with centralized ETH (vs 0.968 CEX–CEX) and lags 3–12 minutes. It is a different instrument with different microstructure, not a substitute. MEV contamination means raw swap prices are not clean mid-prices.

### 1.9 Quality flags bitmask (shared across asset classes)

```
0x0001 STALE_QUOTE      0x0020 SUSPECT_VOLUME     0x0400 REORG_PENDING
0x0002 CROSSED_BOOK     0x0040 HALTED             0x0800 SYNTHETIC_ROLL
0x0004 WIDE_SPREAD      0x0080 AUCTION_ONLY       0x1000 VENDOR_REVISED
0x0008 LOW_UPDATE_COUNT 0x0100 CORP_ACTION_ADJ    0x2000 INTERPOLATED
0x0010 VENUE_OUTAGE     0x0200 EXPIRY_WEEK        0x4000 QUALITY_TIER_3
```
Flags are data, not filters. A dataset spec declares its exclusion mask; that mask enters the content hash; two datasets with different masks are different datasets. This is how "we cleaned the data differently" stops being an untracked source of irreproducibility.

---

## 2. Cross-asset time alignment

Five asset classes with five different clocks must produce one feature matrix without look-ahead.

**Master clock: the UTC minute grid.** It is the only clock all five share. Sessions, holidays and block times are *attributes* of an observation, never the index.

**Three hard rules:**

1. **Emit staleness, never forward-fill silently.** Every aligned feature carries a companion column:
   ```
   feature_x, feature_x_age_minutes, feature_x_quality
   ```
   Forward-filling without exposing age is how a model learns to trade a stale quote and looks brilliant in backtest. Models may *use* age as a feature; that is fine and often informative. What is not fine is age being invisible.

2. **`asof` joins are backward-only, always.** `strategy='nearest'` is banned at the library level — wrapped, not documented. It is a one-character look-ahead bug.

3. **Pin the calendar version.** `exchange_calendars` gets revised retroactively (holidays corrected, historical session times amended). The calendar package version is part of the dataset spec hash. This is an under-appreciated reproducibility hole that will otherwise produce "the same backtest gave different numbers six months later" with no explanation.

**Densification contract:** the reader densifies sparse bars against the master clock at query time, producing `(value, age, quality)` triples. There is exactly one implementation of this, in the feature runtime (§4), and both backfill and live paths call it.

---

## 3. L1 — Dataset and feature plane

### 3.1 A dataset is a hash, not a path

```python
dataset_id = blake3(canonical_json({
    "universe_spec_id":     ...,   # bitemporal membership query
    "instrument_ids":       ...,   # resolved at spec time, stored explicitly
    "date_range":           ...,
    "frequency":            "1m",
    "feature_set_id":       ...,   # versioned feature DAG hash
    "label_spec_id":        ...,
    "split_spec_id":        ...,
    "as_of_knowledge_time": ...,   # THE pit anchor
    "quality_exclusion_mask": ...,
    "calendar_version":     ...,
    "adjustment_policy":    ...,   # read-time factor composition rules
    "finality_policy":      ...,   # DeFi only
    "runtime_image_digest": ...,
}))
```

Two runs with the same `dataset_id` used byte-identical data. That is the guarantee, and it is enforced by materializing the resolved instrument list into the spec rather than re-running the universe query (which would silently change as membership data is revised).

### 3.2 Feature definitions

```sql
CREATE TABLE feature_def (
  feature_id       TEXT PRIMARY KEY,     -- 'realized_vol_21d_v3'
  version          INT NOT NULL,
  code_hash        BYTEA NOT NULL,
  lookback_bars    INT NOT NULL,         -- DECLARED, and it is load-bearing
  knowledge_lag_ms BIGINT NOT NULL,      -- min delay before inputs are knowable
  output_dtype     TEXT,
  asset_classes    TEXT[],
  deflators        TEXT[],               -- e.g. crypto seasonal profile
  info_class       TEXT NOT NULL         -- see §8.3 feature firewall
);
```

**`lookback_bars` and `knowledge_lag_ms` are not documentation.** They are inputs to the automatic embargo computation (§12.2). A feature that lies about its lookback produces leakage that no test will catch except the causal access guard. Therefore: the feature runtime **enforces** the declared lookback by executing every feature under a windowed view that physically cannot see further back. A feature that needs more data fails loudly at registration.

### 3.3 One code path — the non-negotiable

A feature is a pure function `f(WindowedFrame, params) -> Series`. The same compiled DAG serves:
- **backfill** (Polars, batched over instrument × date partitions)
- **live** (same Polars expression, incremental window)

There is no second implementation. Not "kept in sync" — one.

**Consistency measurement is built in from day one:**
```sql
CREATE TABLE feature_serving_log (      -- write on EVERY live serve
  serve_id UUID, tenant_id BIGINT, instrument_id BIGINT,
  event_time TIMESTAMP(9), knowledge_time TIMESTAMP(9),
  feature_set_id TEXT, values BYTEA,        -- packed vector
  served_at TIMESTAMP(9)
);

CREATE TABLE feature_consistency_diff (  -- nightly backfill-and-diff
  serve_id UUID, feature_id TEXT,
  served_value DOUBLE, recomputed_value DOUBLE, abs_diff DOUBLE,
  served_knowledge_time TIMESTAMP(9), recomputed_knowledge_time TIMESTAMP(9),
  diagnosis TEXT   -- late_arrival | code_drift | nondeterminism | precision
);
```
Carrying **both** knowledge times in the diff is what converts "the numbers differ" into "the backfill assumed data that arrived 40 minutes late." That single column turns a mystery into a ticket. In a live-capital context it doubles as compliance evidence.

**SLO:** p99 absolute relative diff < 1e-9 for deterministic features; any `code_drift` diagnosis is a P1.

### 3.4 Label specs

```sql
CREATE TABLE label_spec (
  label_spec_id TEXT PRIMARY KEY, kind TEXT,   -- triple_barrier|horizon_return|meta_label|custom
  horizon_bars INT NOT NULL,
  pt_sl_multiples DOUBLE[], vol_estimator TEXT,
  min_return_threshold DOUBLE,
  sample_weight_method TEXT,     -- uniqueness | return_attribution | time_decay | none
  code_hash BYTEA
);
```
`horizon_bars` feeds the embargo formula. Labels with overlapping horizons **must** declare a `sample_weight_method`; `none` is permitted but sets `overlapping_labels_unweighted=true` on every trial that uses it, which the comparison layer surfaces in any head-to-head.

### 3.5 Split specs

A split is an object with a deterministic expansion, never a slice.

```sql
CREATE TABLE split_spec (
  split_spec_id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,          -- walk_forward | purged_kfold | cpcv | holdout | sealed
  n_folds INT, n_test_groups INT,          -- CPCV
  train_window TEXT,                        -- expanding | rolling:N
  embargo_bars INT NOT NULL,                -- COMPUTED, see §12.2
  purge_on TEXT NOT NULL DEFAULT 't1',      -- t1 = label END. t0 is a bug.
  min_train_bars INT, regime_stratified BOOLEAN
);
```

**`purge_on = 't1'` is the default and `t0` requires an override with a written reason.** Purging on label start silently under-purges by the entire label horizon. It is invisible, common, and inflates everything.

---

## 4. L2 — The Trial Ledger

This is the asset. Everything else exists to feed it or to read it.

### 4.1 The core table

```sql
CREATE TABLE trial (
  trial_id        UUID PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  campaign_id     UUID NOT NULL,
  parent_trial_id UUID,                  -- lineage: branch/resume/duplicate
  seq             BIGINT NOT NULL,       -- monotonic per tenant
  prev_hash       BYTEA NOT NULL,        -- hash chain
  row_hash        BYTEA NOT NULL,

  -- WHAT (written before execution)
  config_hash     BYTEA NOT NULL,
  config          JSONB NOT NULL,
  dataset_id      TEXT NOT NULL,
  split_spec_id   TEXT NOT NULL,
  code_hash       BYTEA NOT NULL,
  image_digest    TEXT NOT NULL,
  seed_set        INT[] NOT NULL,

  -- WHO / WHY (decision provenance)
  actor_kind      TEXT NOT NULL,         -- human | agent | scheduler
  actor_id        TEXT NOT NULL,
  on_behalf_of    BIGINT,                -- principal attribution
  policy_id       TEXT, policy_version INT,
  candidate_set_hash BYTEA,
  propensity      DOUBLE PRECISION,      -- p(this config | context, policy)
  exploration_flag BOOLEAN NOT NULL,     -- forced-exploration draw?
  hypothesis_id   UUID,

  -- PRE-REGISTRATION (hash-locked before first execution)
  prereg_hash     BYTEA NOT NULL,
  delta_practical DOUBLE PRECISION NOT NULL,   -- REQUIRED. no default.

  -- LIFECYCLE
  state           TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL,
  started_at      TIMESTAMPTZ, ended_at TIMESTAMPTZ,
  terminal_reason TEXT,
  censoring       TEXT NOT NULL DEFAULT 'none',  -- none|right_asha|right_budget|
                                                  -- right_preempt|right_cancel|failed
  censor_at_step  INT, planned_steps INT,

  -- COST
  gpu_seconds DOUBLE, cpu_seconds DOUBLE, peak_vram_bytes BIGINT,
  usd_cost DECIMAL(18,6),

  -- OUTCOME (vector, never a scalar) -- see §4.3
  outcome         JSONB,
  gate_results    JSONB,

  -- POINTERS
  artifacts_uri   TEXT, metrics_uri TEXT, predictions_uri TEXT, returns_uri TEXT,

  supersedes      UUID, knowledge_time TIMESTAMPTZ NOT NULL
);
```

### 4.2 The eight invariants

1. **Write-before-execute.** The row is inserted in state `REGISTERED` with `config_hash`, `prereg_hash` and `propensity` populated **before** any compute is dispatched. The executor will not accept a job without a `trial_id` in `REGISTERED`. There is no opt-out flag, no `--no-log`, no admin bypass. *If a side door exists, an audit will find it was used.*
2. **Everything is logged**, including crashed, cancelled, pre-empted, gate-failed and "just exploring." Failures are the training data for M2 and the denominator for trial accounting.
3. **Full out-of-sample return series persisted** (`returns_uri`), not only summaries. PBO, SPA and Romano–Wolf all need the full trial matrix; summaries are a lossy projection you can always recompute, the series you cannot.
4. **Raw per-fold validation predictions persisted** (`predictions_uri`). This is TabRepo's central insight and it is worth more here than in AutoML: it lets you simulate any ensemble, recompute any metric, and re-score history under a new cost model, for free, forever.
5. **Censoring is a first-class field.** An ASHA-killed run is a **right-censored observation**, not a bad one and not a missing row. Dropping or zero-imputing these is the single largest source of bias in experiment databases.
6. **Propensity is logged** with `candidate_set_hash`, `policy_id/version`, and `exploration_flag`. A deterministic logging policy makes off-policy evaluation *formally impossible*.
7. **`N_eff` is platform-computed**, never self-reported — by correlation clustering over stored return series (§12.4).
8. **Hash-chained.** `row_hash = blake3(prev_hash || canonical(row_minus_hashes))`. Tampering is detectable, not merely discouraged. Corrections are new rows with `supersedes`; the original stays, so a meta-model trained six months ago remains explainable against the ledger as it was.

### 4.3 The outcome is a typed vector

```sql
CREATE TYPE outcome_vector AS (
  -- predictive
  auc DOUBLE, logloss DOUBLE, brier DOUBLE, ece DOUBLE, ic DOUBLE, ic_ir DOUBLE,
  -- portfolio, NET of costs
  sharpe_net DOUBLE, sortino_net DOUBLE, calmar DOUBLE,
  psr DOUBLE, dsr DOUBLE, pbo DOUBLE,
  max_dd DOUBLE, dd_duration_days INT, turnover_annual DOUBLE,
  capacity_usd DOUBLE, breakeven_cost_multiple DOUBLE,
  -- robustness
  seed_sharpe_std DOUBLE, regime_pnl_hhi DOUBLE, cpcv_p05_sharpe DOUBLE,
  bootstrap_p05_sharpe DOUBLE, param_cliff_score DOUBLE,
  -- attribution
  alpha_t_stat DOUBLE, factor_r2 DOUBLE,
  -- operational
  inference_latency_p99_ms DOUBLE, model_bytes BIGINT, train_gpu_seconds DOUBLE
);
```

**There is no `score` column.** The database makes "this model is optimal" unrepresentable as a single number. Selection requires an explicit objective + constraint set (§12.1). This is your "never call it optimal from one metric" rule enforced by the type system rather than by discipline.

### 4.4 Decision log (separate from trials)

Not every decision produces a trial. Pruning a branch, adjusting a search space, reallocating budget, and choosing *not* to run something are all decisions that a policy-learning model needs.

```sql
CREATE TABLE decision (
  decision_id UUID PRIMARY KEY, tenant_id BIGINT, campaign_id UUID,
  decision_kind TEXT,          -- propose|select|prune|reallocate|stop|promote|reject
  actor_kind TEXT, actor_id TEXT, on_behalf_of BIGINT,
  context_hash BYTEA, context_uri TEXT,
  candidate_set JSONB,          -- ALL options considered
  chosen JSONB,
  propensity DOUBLE, policy_id TEXT, policy_version INT,
  exploration_flag BOOLEAN,
  decision_tier TEXT,           -- which fallback tier produced it, see §14.5
  rationale TEXT,
  decided_at TIMESTAMPTZ, prev_hash BYTEA, row_hash BYTEA
);
```

`candidate_set` holding the rejected options is what makes this a learning-to-rank dataset rather than a log.

### 4.5 Forced exploration is a hard floor

**≥5% of dispatched trials per campaign are drawn from a uniform random arm over the declared search space, flagged `exploration_flag=true`, and the agent cannot lower this.** Configurable upward, never below 5%.

Three independent justifications:
- It is a surrogate-misspecification canary. In LLMSYS-HPOBench (364k configs, ~95k GPU-hours) random search beat Hyperband, BOHB, SMAC *and* HEBO on two of seven systems.
- It is the unbiased sample that keeps the ledger a valid training set.
- Without it, propensities concentrate and every off-policy estimator's variance explodes.

**Exploration is a data-collection obligation, not wasted compute.** Budget it explicitly and report it as a line item so nobody "optimizes" it away.

### 4.6 Fixation

Four stacked mechanisms:
1. **Content addressing** — artifacts stored under blake3 of their bytes.
2. **Hash chaining** — per §4.2(8), with a daily anchor row signed and written to WORM.
3. **Iceberg tags at registration** — transactional, per registered artifact, ≥90-day expiry floor, audited monthly (§0).
4. **Bitemporal supersession** — corrections append, never mutate.

**WORM scope: the ledger anchors and the artifacts of traded models only.** The 2022 SEC 17a-4 audit-trail alternative means a proper hash-chained bitemporal ledger already satisfies the requirement — so do *not* push the analytical tables into Object Lock, which would block compaction and re-partitioning forever.

---

## 5. L3 — The knowledge plane

Everything here is **recomputable from L0–L2**. That is what makes it safe to iterate on, and it is why L3 gets its own schema with no WORM and no retention guarantees.

### 5.1 Five spaces

| Space | Key | Contents |
|---|---|---|
| **A** — Asset | (instrument_id, venue_id, window, embedding_version, valid_from) | fingerprint |
| **R** — Regime | (market_scope, window, model_version, valid_from) | market state, **filtered only** |
| **S** — Config | config_hash | structured encoding of the configuration tree |
| **O** — Outcome | trial_id | the outcome vector from §4.3 |
| **F** — Failure | trial_id | diagnostic signature of failed/rejected trials |

### 5.2 Asset fingerprints — build order [REVISED]

I recommended "hand-crafted features + performance landmarkers" in the brief. The research sharpened this in three ways.

**Tier 1 — statistical fingerprint (~50 dims).** Ship first.

Realized vol at 5 horizons · vol-of-vol · realized skew/kurtosis · bipower variation and jump intensity · variance ratios (1/5/21) · autocorrelation at lags 1/5/21 · Hurst · downside/upside vol ratio · Amihud illiquidity · Kyle's lambda · **EDGE spread estimate** · turnover/ADV · overnight-vs-intraday variance split · intraday seasonality shape coefficients · factor betas · asset-class and venue one-hots.

> **[REVISED] Use EDGE, retire Roll and Corwin–Schultz.** Ardia–Guidotti–Kroencke (JFE 2024): at a 0.50% true spread, RMSE is **EDGE 0.38% vs CS 0.57%, AR 0.83%, Roll 1.72%**. Against TAQ 1993–2020: EDGE 1.22% vs CS 2.08%, correlation 76.5% vs 66.9%, non-positive estimates 5% vs 20–30%. It is explicitly validated at minute level and stays unbiased under infrequent trading — precisely the illiquid-crypto and long-dated-option case. There is no reason to use the older estimators.

> **[REVISED] Deflate crypto by venue seasonal profile before computing any vol feature** (§1.7). Otherwise clustering discovers funding schedules.

**Feature-set choice barely matters; the wrapper does.** Over 124 UCR problems, **85.3% of pairwise feature-set comparisons were statistical ties**. catch22 runs in <10 ms/1000 samples vs tsfresh's 2.53 s — a 250× cost gap for a tie. So: **catch22 + a TSFEL subset + the finance block above. Do not ship tsfresh's 783-feature default.**

**Tier 2 — reference-portfolio scores, not "landmarkers." [REVISED]**
I said landmarkers. The correct construction is a **greedy submodular-selected complementary portfolio** evaluated against your own trial ledger — which is what Auto-sklearn 2.0 does after *deliberately deleting* meta-features, and what TabRepo validates (a 3-config portfolio beats most AutoML systems; 15 configs beats full AutoGluon; saturation ~150). One caution worth carrying: a 2025 benchmark found catch22 specifically weak for *algorithm selection*, which is exactly our task — so Tier 2's portfolio scores, not Tier 1's statistics, should carry the routing signal.

Start with 8 configs, grow greedily to ~24 as the ledger fills. Each is a cheap strategy/model run on the asset; **their scores are the asset's coordinates.**

**Tier 3 — learned encoder. Gated, not scheduled.**
Adopt only on a measured probing win. Evidence says plan for **fusion, not replacement**: the only 2026 study evaluating TSFM *embeddings* found frozen Chronos-2/MOMENT inconsistent, fine-tuned Chronos-2 +28% F1 over an MLP, and **MLP + Chronos-2 concatenation best (.876 vs .839 vs .704)**.

> **Do not port TSFM benchmark numbers to your data.** An audit of 22 TSFMs × 401 datasets found only **6% of datasets unused by any model**; TSFMAudit puts contamination at **85–96% for Moirai-1/2 and Kairos**, 14% Chronos/TiRex, 9% TimesFM-2.0. And fev-bench, the better benchmark, **contains no financial datasets at all**. Every TSFM claim must be re-earned on your own minute bars.

### 5.3 Schema

```sql
CREATE TABLE asset_embedding (
  instrument_id BIGINT, venue_id INT,          -- venue is a coordinate [REVISED §1.1]
  embedding_version TEXT, window_spec TEXT,     -- e.g. '21d@1m'
  valid_from TIMESTAMPTZ, valid_to TIMESTAMPTZ,
  knowledge_time TIMESTAMPTZ NOT NULL,          -- meta-leakage guard
  tier1 REAL[], tier2 REAL[], tier3 REAL[],
  retrieval_vec vector(48),                     -- whitened, reduced, the ONLY kNN target
  intrinsic_dim REAL, quality_flags INT,
  PRIMARY KEY (instrument_id, venue_id, embedding_version, valid_from)
);
```

**`knowledge_time` here is load-bearing, not decoration.** A retrieval that uses an asset vector computed with future data is meta-level leakage, and it is very easy to do by accident — the fingerprint is computed in a batch job that naturally sees everything.

**Retrieval happens at 48 whitened dims, not 200 raw.** Distance concentration is survivable; **hubness is the real problem** — apply mutual-proximity hubness reduction and monitor intrinsic dimension.

### 5.4 Regimes — filtered probabilities only, enforced by GRANT [REVISED]

A controlled lookahead ladder on a 2-state HMM measured: **Sharpe 0.78 with filtered probabilities, 0.77 with full-sample parameters but filtered inference, 1.74 with smoothed probabilities.** A **2.2× Sharpe inflation from non-causal state inference alone** — and notably, parameter lookahead was nearly harmless. The dangerous part is the state estimate, not the fit.

```sql
CREATE SCHEMA regime_causal;     -- backtester has SELECT
CREATE SCHEMA regime_research;   -- backtester has NO GRANT

CREATE TABLE regime_causal.regime_state (
  market_scope TEXT, event_time TIMESTAMP(9), model_version TEXT,
  p_filtered REAL[],                     -- causal. the only thing a strategy may see.
  regime_vec vector(16), knowledge_time TIMESTAMP(9)
);
CREATE TABLE regime_research.regime_state_smoothed (
  market_scope TEXT, event_time TIMESTAMP(9), model_version TEXT,
  p_smoothed REAL[], viterbi_path INT      -- analysis only
);
```

**Enforce by permission, not by review.** The backtest execution role has no GRANT on `regime_research`. A reviewer will miss this once; a missing GRANT never will.

Regimes are labelled by statistical properties (`high_vol/high_disp/neg_mom`), never narratively ("the 2022 regime"). Validation is out-of-sample regime-conditional performance separation, or the regime model is decorative.

**Two layers:** a global cross-asset regime (vol level, dispersion, correlation, breadth, term slope, credit) and a per-asset-class local regime (crypto funding regime, futures term structure, options vol-surface regime). Cross-asset regime alignment uses the UTC minute grid with staleness columns (§2), not session mapping.

### 5.5 The Outcome Tensor

```
T[asset_cluster, venue, regime, strategy_family, config, metric] → value
```

Materialized as a sparse fact table with a factorization sidecar. Almost every meta-question is a slice.

**This is collaborative filtering** — assets are users, configs are items, outcomes are ratings. Which means it inherits collaborative filtering's central pathology, and here it is severe.

**The missingness is not at random, and two corrections are mandatory:**

1. **Selection bias** — configs ran because something liked them. Handled by the logged propensities (§4.2·6).
2. **Censoring** — ASHA-killed runs have *truncated* outcomes. Handled by censored regression / survival models over `censoring` and `censor_at_step` (§4.1).

> **[REVISED] Prefer MNAR joint-likelihood over pure IPW when propensities are extreme.** Exponential-family CP tensor completion with `P = logit⁻¹(b₀ + b₁X)` has bounds valid for probabilities arbitrarily near 0 or 1, and provides a **sample-split test of H₀: b₁ = 0**. Ship `b₁` as a platform health metric — it is quantitative, publishable proof that the naive tensor is biased, and it tells you when the correction stops mattering.

> **[REVISED · THE MOST IMPORTANT SAFETY RULE IN L3] Off-policy hyperparameter selection is actively dangerous.** Selecting configurations by maximizing an IPS/DR estimate on logged data can pick policies **~15% worse than the logging policy you started from.** Therefore every ledger-derived recommendation is ranked on a **lower confidence bound**, and shrunk toward the default policy by a significance-tested weight:
> ```
> score(c) = LCB_α( DR_estimate(c) )
> w(c)     = shrink_toward_default(score(c), significance_test_pvalue)
> ```
> **Make this non-bypassable in the recommender service.** No "expert mode" that returns raw point estimates.

**HRP is a taxonomy, not an allocator. [REVISED]** A 2011–2025 Brazil + US study (756-day windows) found minimum-variance had the lowest realized volatility in both markets and hierarchical methods had higher turnover, with "no evidence" of outperformance. Use hierarchical clustering to define the tensor's row space (asset_cluster), and do not use it for portfolio construction.

### 5.6 Agent memory records

```sql
CREATE TABLE insight (
  insight_id UUID PRIMARY KEY, tenant_id BIGINT,
  tier SMALLINT,                 -- 1 run feedback | 2 tactical | 3 cross-campaign
  scope JSONB,                   -- asset_cluster / regime / strategy_family / venue
  claim TEXT NOT NULL,           -- natural language
  evidence_trial_ids UUID[],     -- REQUIRED. no evidence, no insight.
  support_n INT, contradicted_n INT,
  embedding vector(768),         -- over claim text ONLY
  created_at TIMESTAMPTZ, last_confirmed_at TIMESTAMPTZ,
  decay_score REAL, info_class TEXT NOT NULL
);
```

Three rules from the agent-memory research:
- **Never embed numeric results.** Numbers live in SQL. The vector index covers natural-language claims only.
- **Hard ~4K-token injection cap** on retrieved memory per agent turn. ML-Master 2.0's three-tier cache works *because* of the cap.
- **Skip skill libraries.** SkillEvolBench (180 tasks, 10 model configs): raw-trajectory reuse frequently beats distilled skills; static curated skills underperformed the no-skill baseline by −2.44 points; multi-skill composition hit 0% in some environments. Insights + trajectories, not skills.

`contradicted_n` and `decay_score` exist because stale beliefs are the dominant long-horizon memory failure. An insight whose support is not reconfirmed within its scope's regime decays out of retrieval.

---

## 6. Physical layout, engines, cost

### 6.1 Engine assignment  *[AMENDED 2026-09-14 — Appendix C, C-2]*

| Workload | Engine | Why |
|---|---|---|
| Backtest data serving | **DuckDB 1.5, embedded per worker** | isolation by construction — never serve backtests from a shared cluster, one tenant's scan cannot stall another's |
| Feature computation | **Polars 2.0** | one expression DAG serving backfill and live |
| Shared analytics / run comparison | **ClickHouse 26.8 LTS** | keyed quotas map cleanly onto tenants |
| System of record | **Postgres 18 + pgvector** | transactions, FK integrity, RLS |
| Table format | **Iceberg v3** | deletion vectors, row lineage, `timestamp_ns`, variant |
| Live metric stream | **NATS JetStream → SSE** | scales with viewers, not runs |
| Artifacts | content-addressed object store | dedup, immutability |

On the kdb+ question: the vendor benchmark claiming 161× over ClickHouse is KX-run on KX's home turf. The honest reading is that kdb+ wins narrowly on as-of/last-value queries, not generally. Not worth the licence or the hiring constraint.

### 6.2 Partitioning and sort

```
bar_1m/          asset_class / venue_id / event_date        sort: instrument_id, event_time
option_bar_1m/   underlying_id / expiry_month / event_date  sort: dte, moneyness, event_time
pool_obs/        chain_id / event_date                      sort: pool_id, block_number
trial/           tenant_id / created_month                  sort: campaign_id, seq
metrics/         tenant_id / trial_id_bucket                sort: trial_id, step
```
Target file size 128–512 MB. Options sort by `(dte, moneyness)` first because every real query is moneyness/DTE-gated and this turns the gate into partition pruning.

### 6.3 Cost reality

~8.6 TB for 5,000 instruments × 10 years plus a gated options universe ≈ **$110/month** of object storage.

**Storage is noise. Market-data licences ($3k–$60k+/month) and backtest compute dominate by two to three orders of magnitude.** Do not spend engineering on compression ratios. Spend it on not re-running backtests that a content hash proves you already ran — config-hash dedup at dispatch is worth more than every codec decision combined.

Tiering: hot (90 days, local NVMe cache) / warm (2 years, standard object storage) / cold (everything, infrequent-access). Options outside the liquid universe gate go straight to cold, never deleted.

---

## 7. Multi-tenancy and the feature firewall

Shared market data, private research. This is the hardest product-shaped constraint in the system, because your users are quants and their experiment history *is* their edge.

### 7.1 Isolation by construction, not by policy

| Plane | Isolation |
|---|---|
| L0 market data | shared, read-only, single copy |
| L1 dataset specs | per-tenant rows, shared feature *definitions* |
| L2 trial ledger | per-tenant partition + Postgres RLS + separate object-storage prefix |
| L3 knowledge | split by information class — §7.3 |
| Compute | per-tenant Kueue ClusterQueue, embedded DuckDB per worker |

**The shared-plane service role has no read path to tenant prefixes at all.** Not "is not supposed to read them" — *cannot name them*. A credential that cannot name the bucket is far stronger evidence under diligence than any volume of access logs, and a sophisticated trading client will ask.

**Two Postgres RLS landmines, both silent:**
1. Table **owners bypass RLS** unless you set `FORCE ROW LEVEL SECURITY`. The migration role usually owns the tables.
2. Bare `SET` (rather than `SET LOCAL`) **bleeds tenant context across pooled connections.** With PgBouncer in transaction mode this is a cross-tenant data leak that passes every test written against a non-pooled connection.
3. And a third worth knowing: **RLS policies combine with `OR`, not `AND`.** Adding a policy widens access.

All three are covered by mandatory CI tests that run against a pooled connection as the owning role.

### 7.2 Cross-tenant meta-learning: the honest answer

**No for anything touching strategy content, and this is not a differential-privacy problem.**

A global strategy-family recommender is functionally *a mechanism for broadcasting one tenant's edge to every other tenant*. Three distinct harms: direct misappropriation; accelerated crowding (modelled as compressing signal half-life from ~58 to ~18 months); and a reputational hit that is total and asymmetric when a client asks what their data trains.

**Skip federated learning and differential privacy.** FL protects *raw data* while your actual risk is that the learned function transfers competitive information — it defends the wrong thing. DP costs a measured 5–10 accuracy points at ε=6, and the memorization literature undercuts the usual reassurances (8× LoRA rank did not increase total PII extraction but broadened identifier diversity; repetition frequency predicts extraction poorly, R²=0.237). Schema discipline plus row-level isolation gets you further at roughly 1/50th the cost — and has the underrated property of making per-tenant deletion *architecturally true* rather than aspirational.

Revisit only if a tenant contractually requires DP, or if you want to publish cross-tenant aggregate research — where DP on released aggregates (not on training) is the right and cheap tool.

### 7.3 The feature firewall — a whitelist you can unit-test

Every feature and every insight carries `info_class`. Only two classes may cross a tenant boundary.

| `info_class` | Crosses? | Examples |
|---|---|---|
| `platform_physics` | **yes** | runtime, VRAM, failure mode, convergence step, throughput, queue wait |
| `methodology` | **yes** | CV scheme, embargo length, trial counts, seed count, data-hygiene results, calibration method |
| `market_public` | yes (it's L0) | realized vol of SPY, venue uptime |
| `strategy_content` | **never** | signal definitions, feature sets, code embeddings, instrument selections |
| `performance_conditional` | **never** | outcome conditioned on strategy family / instrument / config |
| `tenant_operational` | never | budgets, usage, org structure |

**CI enforcement:**
```python
def test_global_model_feature_firewall():
    for model in registry.models(scope="global"):
        for feat in model.feature_list:
            assert feature_def[feat].info_class in ALLOWED_GLOBAL, \
                f"{model.id} uses {feat} (info_class={...}) — firewall violation"
```
**A firewall you can unit-test beats ten pages of policy.** This test blocks the build.

### 7.4 Per-tenant vs global, per model

| Model | Scope | Reason |
|---|---|---|
| M1 cost predictor | global | encodes your infrastructure |
| M2 failure classifier | global | encodes your infrastructure |
| M3 learning-curve extrapolator | global | encodes optimizer dynamics |
| M10 leakage detector | global | encodes methodology |
| M11 anomaly detector | global | encodes metric physics |
| M13 step critic | global | encodes tool semantics |
| M12 executor adapter | global, one adapter | see §14.6 |
| M4 config surrogate | hierarchical + firewall | global prior, per-tenant adaptation |
| M5 gate pre-screener | hierarchical + firewall | gates are methodology; features must pass §7.3 |
| M9 proposal ranker | hierarchical + firewall | |
| M6 asset encoder | global (L0 only) | built from public market data |
| M7 regime model | global (L0 only) | |
| **M8 strategy-family recommender** | **per-tenant, always** | this is the one that would broadcast edge |

**Hierarchical means partial pooling (hierarchical Bayes), never a one-hot `tenant_id` in a global model** — the one-hot is an invitation for the model to memorize a tenant. And **per-tenant regression is a rollback criterion**: a global model that improves the mean while degrading small tenants does not ship.

**Avoid per-tenant LoRA adapters** (§14.6). The 97% multi-adapter serving saving assumes adapters share a batch, but tenant load here is *correlated* — market open, volatility spikes — so you get cache thrash exactly at peak.

---

# PART II — MLOPS

## 8. Control plane  *[AMENDED 2026-09-14 — Appendix C, C-3]*

**Temporal (control) + Ray (compute) + Kueue/JobSet (admission).**

Static-DAG engines are structurally wrong here: an agent operator makes the DAG dynamically unknown — step N+1 depends on step N's result. You need durable execution (event-sourced replay), not DAG compilation. Temporal's 2026 features map onto this exactly: Worker Versioning GA (an agent editing pipeline code no longer breaks in-flight workflows), Task Queue Priority & Fairness GA (research sweeps cannot starve a production retrain), External Payload Storage, and **Principal Attribution** — non-spoofable "agent X acting for user Y," which is what makes §4.1's `on_behalf_of` trustworthy rather than self-asserted.

Flyte 2 is the more elegant single-system alternative and I considered it seriously. Rejected: GA'd one month ago, two core contributors, one commercial sponsor, and this system must run for years under real capital.

Determinism constraints are cheap because the real work is coarse-grained external steps.

## 9. Training job state machine

```
                         ┌──────────────┐
                         │  REGISTERED  │  ledger row written; nothing dispatched
                         └──────┬───────┘
                   ┌────────────┼────────────┐
                   ▼            ▼            ▼
             ┌──────────┐ ┌──────────┐ ┌──────────┐
             │  QUEUED  │ │ REJECTED │ │DEDUPLICATED│ config_hash already terminal
             └────┬─────┘ └──────────┘ └──────────┘   (returns prior trial_id)
                  │ admitted by Kueue (quota + budget)
                  ▼
            ┌───────────┐
            │ PROVISION │ image pull, dataset materialize, seed set
            └─────┬─────┘
                  ▼
            ┌───────────┐ ◄──────────────┐
            │  RUNNING  │                │ resume from checkpoint
            └─────┬─────┘                │
      ┌───────────┼───────────┬──────────┴──┐
      ▼           ▼           ▼             │
┌──────────┐ ┌─────────┐ ┌─────────┐  ┌──────────┐
│ EVALUATE │ │ PAUSED  │ │PREEMPTED│─►│RECOVERING│
└────┬─────┘ └────┬────┘ └─────────┘  └──────────┘
     │            │ freeze-thaw (ASHA rung boundary)
     ▼            └──► QUEUED
┌──────────┐
│  GATED   │ promotion gate stack §12
└────┬─────┘
     ├──► COMPLETED_PASS ──► registry candidate
     ├──► COMPLETED_FAIL ──► ledger, counted in N_eff, feeds M5
     ▼
┌──────────┐   terminal_reason ∈ {oom, nan_divergence, data_error, timeout,
│  FAILED  │                      leakage_detected, budget_exceeded, cancelled,
└──────────┘                      dependency_failure, asha_stopped}
```

**Rules:**
- `REGISTERED` precedes every dispatch. No exceptions, no admin bypass.
- `DEDUPLICATED` is a real, common, valuable state. Config-hash dedup at dispatch kills a surprising share of agent spend outright and returns the prior `trial_id` so lineage stays honest.
- `PREEMPTED → RECOVERING → RUNNING` requires bit-reproducible checkpoints (RNG state, optimizer state, dataloader position, AMP scaler). Without them, resume is a different experiment wearing the same trial_id.
- Every terminal transition sets `censoring`. `asha_stopped` is right-censored, not failed — this distinction is load-bearing for M3/M4/M5.
- Cancellation is cooperative with a hard-kill deadline. A cancelled trial is still written, still counted.

## 10. Campaign lifecycle  *[AMENDED 2026-09-14 — Appendix C, C-3]*

A campaign is a durable Temporal workflow. It is the "many sessions, not one run" object.

```
DEFINE ─► BASELINE ─► DIAGNOSE ─► HYPOTHESIZE ─► EXPERIMENT ─► COMPARE
   ▲                                                              │
   │                                                              ▼
   └──────────── REALLOCATE ◄──── PRUNE ◄──── GATE ◄──────────────┘
                     │
                     └─► CONVERGED | BUDGET_EXHAUSTED | DIMINISHING_RETURNS | HALTED
```

**DEFINE requires, and will not start without:**
```yaml
objective:        # explicit, multi-objective, no scalarization by default
  maximize: [sharpe_net, dsr]
  subject_to:
    max_dd: {lte: 0.20}
    turnover_annual: {lte: 12}
    inference_latency_p99_ms: {lte: 50}
    capacity_usd: {gte: 5_000_000}
benchmark:        # what "better than baseline" means
delta_practical:  0.15          # REQUIRED. smallest meaningful Sharpe difference.
budget:
  usd: 4000
  gpu_hours: 900
  wall_clock_hours: 168
  max_trials: 2500              # this is also the N you will be deflated against
exploration_floor: 0.05         # cannot go lower
gates_profile: strict_v1        # versioned; changing it invalidates prior comparisons
```

**`delta_practical` has no default.** Most stopping and promotion questions are literally unanswerable without it, and a default would be answered wrong silently.

**Multi-session continuity** is why the campaign is durable: checkpoints, lineage, datasets, configs, metrics, artifacts, insights and *rejections* all persist and are injected into the next session's context (subject to the 4K memory cap). A campaign resumed after three weeks knows what has already been ruled out — and, importantly, knows its own trial count, so the deflation gets *stricter* as the campaign continues rather than resetting.

**DIMINISHING_RETURNS** is defined, not vibed: stop when the posterior probability that any remaining candidate exceeds the incumbent by `delta_practical` falls below 0.05, estimated over the last 20% of trials.

## 11. Search and optimization stack

### 11.1 Searcher selection — automatic, from (B, D, P)  *[AMENDED 2026-09-14 — Appendix C, C-4]*

| Budget B (full-train equivalents) | Dim D | Parallel P | Searcher |
|---|---|---|---|
| < 10 | any | any | **portfolio replay** — no search |
| 10–30 | any | any | PriorBand / ifBO |
| 30–200 | mixed/conditional | any | TPE (Optuna) or SMAC3 |
| 30–200 | continuous | any | GP-BO |
| > 200 | high, continuous | any | CMA-ES |
| > 200 | discrete + multi-fidelity | any | DEHB |
| any | any | ≥ 8 | **+ ASHA underneath** |
| **always** | | | **+ ≥5% uniform random arm** |

**High-dimensional GP-BO: fix the prior, not the algorithm.** Scaling the LogNormal lengthscale prior by √D (`ℓ ~ LogNormal(μ₀ + log(D)/2, σ₀)`) makes vanilla GP-BO match or beat SAASBO/TuRBO/ALEBO up to 6,392 dimensions. Trust regions and embeddings are a later resort, not a first move.

### 11.2 ASHA hardening — mandatory, all five

Noisy validation metrics make naive ASHA promotion near-random and create a compounding winner's curse. All of these ship on:
1. Rung metric is smoothed (EMA or last-3 mean), never a single last step.
2. `grace_period` set past the learning-curve crossing region, estimated per task family from the ledger.
3. **PASHA-style noise-estimated soft ranking** — ε = 90th percentile of observed rank-swap gaps. (2.3–3.4× speedups on NAS-Bench-201, 15.5× on WMT.)
4. `WilcoxonPruner` when replicates exist.
5. **Top-3 re-evaluated with fresh seeds before any incumbent is crowned.** Cheap, and it catches most of the damage.

### 11.3 Learning-curve stopping

Posterior-based, via M3 (LC-PFN/FT-PFN class — single forward pass, >10,000× faster than MCMC parametric ensembles):
```
stop if P(final > incumbent + delta_practical | partial_curve, config) < 0.05
```
Freeze-thaw pause/resume is enabled only when (a) checkpoints are bit-reproducible and (b) measured resume cost < 15% of rung duration. Otherwise stopping is terminal and right-censored.

### 11.4 Post-hoc pipeline — fixed order, not options

This is the cheapest remaining performance on the table and it is *free at inference*.

```
1. greedy model soup over the campaign's own checkpoints
2. greedy ensemble selection (Caruana) over per-fold predictions (already stored, §4.2·4)
3. calibrate on a DEDICATED split — logistic-family (Platt-on-logits, beta, quadratic)
4. threshold from the cost matrix IN CLOSED FORM
```
Order matters and is enforced. **Do not tune the threshold** — a tuned threshold is another trial and gets counted as one. Binning-based calibrators are excluded despite flattering ECE numbers; greedy ensemble selection beats unconstrained weight optimization on threshold-dependent metrics because its implicit sparsity prevents validation overfitting.

### 11.5 Statistically defensible comparison

Search-loop validation scores are **inadmissible** as performance estimates. Promotion decisions use a separate protocol:
- randomize *all* nuisance sources (the biased-but-randomized estimator matched the ideal one at **51× less compute**)
- ~29 paired replicates
- declare A ≻ B iff `P(A>B) ≥ 0.75` **and** `P(A>B) − CI_lower > 0.5`
- Holm for promotion decisions; Benjamini–Hochberg / e-BH for screening
- because agents peek continuously: **betting-based anytime-valid confidence sequences** with a ROPE of ±`delta_practical`

Critical-difference diagrams are banned in the UI — unstable under comparate-set changes and gameable. Use a Multiple Comparison Matrix.

---

## 12. Evaluation and promotion gates

### 12.1 Selection is multi-objective by construction

No scalarization by default. A candidate is selected only if it is **Pareto non-dominated** on the declared `maximize` set while satisfying every `subject_to` constraint. Where a scalar is unavoidable (ranking a display list), it is computed from an explicitly declared and versioned preference vector stored on the campaign — never a hidden default.

### 12.2 Embargo is computed, never typed

```
embargo_bars = label_spec.horizon_bars
             + max(feature_def[f].lookback_bars for f in feature_set)
             + ceil(max(knowledge_lag_ms) / 60_000)
             + settlement_lag_bars
             ; minimum horizon_bars + 1
```
Derived from the pipeline, not guessed as a percentage. A user *may* raise it; lowering it requires an override that is recorded on the trial and shown in every comparison involving that trial.

Purge on `t1` (§3.5).

### 12.3 The gate stack — cheapest first  *[AMENDED 2026-09-14 — Appendix C, C-5]*

| # | Gate | Threshold (profile `strict_v1`) |
|---|---|---|
| 1 | Pre-registration hash-locked before first backtest | required |
| 2 | Leakage suite (§12.5) | 100% pass |
| 3 | Cost sensitivity | breakeven cost multiple ≥ 3× |
| 4 | Capacity | deploy ≤ 20% of capacity-at-half-Sharpe; ≤5% ADV soft / 10% hard |
| 5 | CPCV | **5th-percentile** path Sharpe > 0 |
| 6 | Strictly-causal walk-forward | Sharpe > 0 across ≥3 regimes |
| 7 | PBO (CSCV) | < 0.20 |
| 8 | Deflated Sharpe | ≥ 0.95 on **platform-counted** N_eff |
| 9 | Minimum backtest length | SR ≥ 1.5·√(2·ln N_eff / y); ≥5y; ≥300 independent events |
| 10 | Factor attribution | net alpha t-stat ≥ 3.0 vs FF5+MOM+STR+BAB+QMJ, Newey–West; factor R² < 0.7 |
| 11 | Regime coverage | no single regime > 50% of PnL |
| 12 | Perturbation robustness | no parameter cliff; no single instrument > 20% of PnL |
| 13 | Stationary bootstrap | 5th-percentile Sharpe > 0 |
| 14 | **Romano–Wolf stepdown vs the full candidate family** | p < 0.05 |
| 15 | Paper/shadow process gates (§12.6) | see below |
| 16 | Capital ramp | 10 / 25 / 50 / 100% |

**Gates 5 and 6 are both required.** CPCV is *not* a backtest — its paths train on data after some test blocks, so it measures model-class generalization, not what a trader could have earned, and it assumes the stationarity it does not test. The 2024 result favouring CPCV over walk-forward is on *synthetic* data with mild parametric non-stationarity. Promote on neither alone.

**Gate 14 is the most defensible single gate**, because it uses the actual correlation structure of your trials rather than a guessed N_eff. A 200-point sweep over one idea is correctly not penalized as 200 independent tests.

**Gate profiles are versioned and immutable.** Changing thresholds creates `strict_v2`; it does not edit `strict_v1`. Any comparison spanning two profiles is flagged as non-comparable in the UI. This is what stops threshold drift from silently rewriting history.

### 12.4 N_eff is platform-computed

Correlation clustering over stored OOS return series across the tenant's entire ledger — not the campaign, the **ledger**. Every trial counts, including gate-failures and exploration draws. Self-reporting is not an available API.

**Deflation for sizing:** `SR_expected = min(DSR_implied_SR, 0.5 × backtest_SR)`. The multiple-testing haircut is non-linear — SR < 0.4 typically loses >50%, SR > 1.0 loses ≤25% — so "just halve it" is wrong in both directions. Use BHY (FDR), not Bonferroni.

### 12.5 The leakage suite — three tests, all in CI  *[AMENDED 2026-09-14 — Appendix C, C-6]*

1. **Causal access guard.** The dataframe is wrapped in a proxy that raises on any read of rows past the decision timestamp. The entire pipeline runs under it. This is the only test that catches a feature lying about its lookback.
2. **Random-label test.** Permuted labels must yield Sharpe ≈ 0. This validates the *harness*, not the features — it belongs in platform CI and runs nightly against the platform itself, not just per-strategy.
3. **Snapshot reproducibility test.** Re-running on a 6-month-old data snapshot must reproduce identical historical positions. Catches retroactive adjusted-price mutation and calendar revisions.

Plus a soft flag: `CV_Sharpe − WF_Sharpe > 1.0` ⇒ suspect overlapping-label leakage.

### 12.6 Paper trading tests the pipeline, not the alpha

Over 60 days the standard error of Sharpe is ~2.0 — it cannot distinguish 0 from 2. Gate on **process**:
- signal reproduction ≥ 99% match vs backtest *(failures here are almost always PIT-data bugs and are the most valuable output of the whole exercise)*
- realized slippage ≤ 1.5× modelled
- turnover within ±20% of backtest
- zero broker rejects attributable to model output

Reject on realized Sharpe only if catastrophically negative (sign error).

**Kill criteria are pre-registered at deployment time**, because post-hoc kill decisions always arrive late — the drawdown that should trigger them always comes with a plausible excuse. SPRT/CUSUM on live Sharpe against `H₀: SR = SR_expected`, plus MDD > 1.5× backtest MDD.

### 12.7 Gate-hacking countermeasures

Agents will optimize against whatever is measured.
- The trial counter includes **gate-failing** runs. Failing a gate is not a free retry.
- Gates 11–13 use a **platform-held random seed** the agent cannot read, re-drawn per campaign.
- The **sealed holdout is ledger-rate-limited to one evaluation per strategy lineage, ever**, enforced at the data tool — not in a prompt. Requesting it a second time returns the first result with a notice.
- The sealed holdout never informs gate tuning.
- Expect single-digit-percent pass rates. That is the design. The failure mode to watch is not a low pass rate; it is people routing around the registry — so **the registry is the only path to capital**.

---

## 13. Internal model roster  *[AMENDED 2026-09-14 — Appendix C, C-7]*

| ID | Model | Label | Trainable at | Class | Day-1 heuristic |
|---|---|---|---|---|---|
| M1 | runtime/cost predictor | wall-clock, GPU-h, $ | ~200 runs | quantile GBDT | analytic from data size × steps |
| M2 | failure classifier | terminal_reason | ~300 failures | GBDT | static VRAM/shape rules |
| M3 | learning-curve extrapolator | final metric posterior | ~1k partial curves | LC-PFN class | median stopping rule |
| M4 | config→outcome surrogate | outcome vector | ~2k trials/cluster | GBDT ens. / TabPFN-class | searcher's internal model |
| **M5** | **gate pre-screener** | P(pass gates 1–14) | ~1k gated candidates | calibrated GBDT | run cheap gates first |
| M6 | asset encoder | — (self-supervised) | day 1 | Tier1+2 → Tier3 | §5.2 |
| M7 | regime model | — | day 1 | HMM/HSMM + BOCPD, **filtered** | vol terciles |
| M8 | strategy-family recommender | outcome \| (A,R,venue) | ~5k trials, ≥100 assets | MNAR tensor completion | global family win rates |
| M9 | proposal ranker | value per dollar | ~2k proposals w/ outcomes | LambdaMART | acquisition function |
| M10 | leakage detector | P(leak) | **day 1 (synthetic)** | GBDT + static analysis | the 3 tests in §12.5 |
| M11 | metric-stream anomaly | anomaly score | ~500 runs | robust-z + CUSUM/BOCPD | hard rules |
| M12 | executor adapter (LLM) | accepted trajectories | ~5–10k trajectories | LoRA, one global adapter | frontier model + tool search |
| M13 | step critic (PRM) | good\|unnecessary\|mistake\|recover | ~3k labeled steps | 4B-class PRM | rules + judge triage |

**Priority: M5 > M9 > M3 > M1.**

**M5 is the money model.** With a single-digit-percent gate pass rate, every candidate that reaches full CPCV + bootstrap + Romano–Wolf costs real compute. A calibrated model that says "2% chance of passing," operated at a deliberately high-recall threshold, is a direct multiplier on how many hypotheses you can afford. Note it must be *calibrated*, not merely accurate — its output feeds a budget decision.

**M10 is special: trainable on day one.** You can generate unlimited labelled data by deliberately injecting known leaks into known-clean pipelines. Synthetic-label bootstrapping is available here and nowhere else on this list. Build it early; it protects everything.

**M11 catches what nothing else does:** features suddenly all-zero ⇒ loss drops implausibly fast ⇒ that is *leakage*, not success. NaN/Inf and monotone-loss-increase stay hard rules, not ML.

### 13.1 The cold-start ladder (applies to every model above)

Every internal model ships as a **three-tier fallback chain**, and the tier that produced each decision is recorded in `decision.decision_tier`:

```
analytic/rule  →  shrunk-to-prior learned model  →  full learned model
```

The learned tier replaces the rule tier only when it wins by more than `delta_practical` on a **frozen holdout slice of the ledger** that its training never touched, under §11.5's protocol. Because the tier is logged on every decision, you get a free production A/B measuring whether the learned tier actually beat the rule — which settles arguments that otherwise run for months.

---

## 14. Retraining policy

### 14.1 Do not trigger on distribution drift

Evidence is consistent: drift-triggered retraining "performs poorly when retraining costs are high, as it tends to recommend retraining far too often" (ADWIN-5% cost 3.27 vs oracle 2.68), and PSI's 0.1/0.25 thresholds are sample-size-dependent folklore.

**Use the learning-debt rule:**
```
retrain iff  ρ_t > c_churn / (c_churn + c_wait)
```
where ρ_t is estimated expected-loss reduction from retraining now. It beat calendar retraining in 24/24 gradual-drift cells and achieved **0.36× the excess loss** of semi-annual retraining on a 104-week production backtest, triggering 16 weeks early after a policy shock. Honest caveat from the same work: a tuned CUSUM on a performance proxy stays competitive, and *monitoring-dashboard proxies performed poorly* — dashboards are not a retraining signal.

### 14.2 CBPE cannot gate alpha models

NannyML's CBPE/DLE estimate performance without labels, which is genuinely useful when labels lag by weeks. **But CBPE explicitly assumes no concept drift.** It answers "the market looks different but your model should still work." It cannot answer "your edge is gone," which in trading is the entire question. Use it for M1/M2/M3/M11; never as a gate on M4/M5/M8/M9.

### 14.3 Three tiers

**Tier A — autonomous train and promote.** M1, M2, M3, M6, M7, M11.
Labels arrive in minutes, errors are cheap and visible, P(Y|X) is genuinely stable. Learning-debt trigger. Auto-promote only if the challenger beats the champion on a frozen ledger holdout its training never saw. Auto-rollback on degradation.

**Tier B — autonomous training, human-approved promotion.** M4, M5, M8, M9, M12, M13.
These **shape what gets explored**, which is a feedback loop: if M8 stops recommending mean-reversion on high-vol assets, you stop generating evidence about it, and the belief becomes self-confirming and unfalsifiable. Training is continuous and cheap; *promotion* is a decision with an owner. Required alongside: the exploration floor (§4.5), off-policy evaluation with LCB ranking (§5.5) before promotion, and an entropy check (§14.4).

**Tier C — never autonomous.** Gate definitions, thresholds, `delta_practical`, N_eff computation, the deflation formula, position sizing, capital allocation.
A system that learns to adjust its own passing criteria based on its own outcomes will learn to pass. **Make this an access-control boundary, not a norm:** agents may propose experiments; they hold no grant on the gates, holdouts, thresholds or the deflation code path.

Plus a global **"freeze internal models"** switch. During a dislocation the last thing you want is meta-models retraining on three weeks of unprecedented data.

### 14.4 Self-improvement guardrails

- **Model collapse is driven by data replacement, not accumulation.** Accumulate, never replace. Maintain a real-outcome floor: every retraining set must include ground truth from paper/live performance, not only backtest results. Monitor with ≥2 divergence metrics — KL can look stable while Wasserstein grows monotonically.
- **Policy entropy is an SLO with a hard floor that blocks promotion.** This is the failure I expect first: a ranker trained on its own choices only ever sees its own choices, and GRPO-family updates have an intrinsic entropy-decreasing bias.
- **Goodhart defenses:** vector-valued rewards requiring Pareto non-domination (never a weighted scalar), an explicit KL budget against the previous champion, and **evaluator rotation** to break policy–judge co-adaptation.
- **An immutable seed holdout** frozen at inception, never used to train any internal model, retained for detecting long-run drift in the platform's own judgment.
- **Full archive lineage** for every internal model version, with scope restriction enforced by permissions.

### 14.5 Judges and critics

LLM-as-judge raw agreement overstates chance-corrected κ by 33.8–41.2 points — an "85% agreement" judge is κ ≈ 0.48. And an expert audit of BFCL v4 / τ²-Bench / LiveMCPBench / MCP-Atlas found **18.5% of official labels disagree with expert judgment**, with one harness scoring the identical setup between **57.9% and 76.8% across 23 repeats**.

Therefore: **judges triage, gates decide.** Judges may score novelty, leakage plausibility and failure recoverability, with periodic human κ calibration. A judge never decides whether a strategy is good. Build M13 as a small environment-grounded PRM instead — a 4B ternary-reward PRM beat 72B generic PRMs and self-rewarding with a 235B model (+7–11% Best-of-N).

**Build a private eval harness.** Public tool-use benchmarks are not trustworthy enough to gate releases.

### 14.6 Fine-tuning: not yet, but instrument for it now

**Break-even math.** A cached frontier executor call (20k in @ 90% cache hit, 500 out, Sonnet-class) ≈ **$0.019**. The same call on self-hosted 8B+LoRA ≈ **$0.0012** — but the H100 costs **$1,825/month whether used or not**. Covering the GPU alone: ~3,400 calls/day. Covering GPU + the 0.5–1.0 FTE a fine-tuning program actually consumes (~$35k/mo): **~100k executor-calls/day sustained** ≈ 3,300 agent experiments/day at ~30 calls each. Prompt caching pushed this break-even up from ~10k/day a year ago.

Meanwhile the prompt-side wins are large and unexhausted: tool search moved Opus 4.5 from **79.5% → 88.1%** while cutting 85% of definition tokens; tool-use examples moved complex-parameter handling **72% → 90%**; programmatic tool calling cut tokens 37%. GEPA adds ~+10% avg at **35× fewer rollouts** than GRPO.

**Two conditions override the volume math:** a determinism/reproducibility requirement (a pinned local checkpoint is bit-reproducible; a hosted model that silently updates is not — and you have an immutable ledger to keep honest), and a tenant contractually forbidding payloads leaving your VPC. Both are plausible here.

**The structural risk is schema churn:** a fine-tuned executor is a cache of your API surface, and every breaking tool change invalidates it. **Measure trajectory-corpus half-life from `schema_hash` churn.** If it is shorter than your training cadence, fine-tuning is structurally unprofitable — no amount of GPU fixes that.

**Log for it now** (cheap today, impossible later):
```sql
CREATE TABLE agent_trajectory (
  traj_id UUID, tenant_id BIGINT, campaign_id UUID,
  step_idx INT, tool_name TEXT,
  tool_schema_hash BYTEA, tool_semver TEXT,       -- corpus half-life measurement
  arguments JSONB, result_summary JSONB, error JSONB,
  latency_ms INT, tokens_in INT, tokens_out INT,
  critic_label TEXT,                  -- good|unnecessary|mistake|recover
  propensity DOUBLE, exploration_flag BOOLEAN,
  outcome_trial_id UUID, label_available_at TIMESTAMPTZ
);
```
`critic_label` matters: SRFT beats plain rejection sampling (32.2% vs 30.9% on SWE-bench Verified) and **recovers ~61% of otherwise-discarded trajectories**, while naive mixing of unfiltered failures makes things *worse* (28.5%).

When the time comes the recipe is **on-policy distillation**, not SFT or RL: 1,800 GPU-hours → 74.4% AIME'24 vs RL's 17,920 → 67.6% (9–30× cheaper), and it doubles as the antidote to forgetting. PEFT settings: LoRA on all linear layers, **r=256 for SFT / r=1–32 for RL**, LR 10× full-FT, effective batch < 32. Attention-only targeting is the common mistake. **One global adapter; personalize by retrieval, not per-tenant adapters** (§7.4).

---

## 15. Agent tool surface (MLOps subset)  *[AMENDED 2026-09-14 — Appendix C, C-8]*

~15 always-on tools plus a searchable tail. Under 20 visible per turn.

**Cross-cutting conventions on every tool:** `dry_run`, `estimate_cost`, `idempotency_key`, `response_format: concise|detailed` (~67% token savings), structured errors carrying `nearest_valid` and `suggested_fix`, self-describing truncation with continuation hints, **human-readable slugs instead of UUIDs**, and tool-use examples in every schema.

```
# data & datasets
inspect_dataset(dataset_id|spec, sample_strategy, response_format) -> DatasetProfile
build_dataset_spec(universe, range, feature_set, label_spec, split_spec) -> DatasetSpec
  # returns computed embargo_bars and a leakage pre-check. dry_run by default.
diff_datasets(a, b) -> SpecDiff

# assets & regimes
describe_asset(instrument_id, venue_id, as_of) -> AssetFingerprint
find_similar_assets(instrument_id, venue_id, as_of, k, filters) -> [Neighbor]
  # exact kNN over retrieval_vec; knowledge_time-bounded; LCB-shrunk transfer hints
get_regime(market_scope, as_of) -> RegimeState    # filtered only. no smoothed path.

# experiments
propose_experiment(hypothesis, config, rationale) -> Proposal   # ranked by M9
estimate_cost(config, dataset_id) -> CostEstimate               # M1
launch_trials(configs[], search_strategy, budget, idempotency_key) -> [TrialRef]
  # writes REGISTERED rows + propensities before dispatch; dedups by config_hash
get_trial(trial_id, response_format) -> Trial
cancel_trials(trial_ids[], reason) -> CancelResult

# search control
adjust_search_space(campaign_id, changes, rationale) -> SearchSpace
  # exploration_floor is not writable
reallocate_budget(campaign_id, allocations, rationale) -> BudgetPlan
prune_branch(campaign_id, branch_id, rationale) -> PruneResult

# checkpoints & artifacts
list_checkpoints(trial_id) -> [Checkpoint]
branch_from_checkpoint(checkpoint_id, config_overrides) -> TrialRef
inspect_artifact(uri, query) -> ArtifactView    # paginated, truncation-aware

# evaluation
compare_candidates(trial_ids[], objective) -> ComparisonMatrix
  # §11.5 protocol. never a critical-difference diagram.
run_gates(trial_id, profile) -> GateResults
explain_failure(trial_id) -> FailureAnalysis    # M2 + M11 + logs
request_promotion(trial_id, target_stage, evidence) -> ApprovalRequest

# memory
search_insights(scope, query, k) -> [Insight]   # 4K token cap enforced server-side
write_insight(claim, scope, evidence_trial_ids) -> Insight  # evidence REQUIRED
```

**Not exposed to agents at any permission level:** gate thresholds, `delta_practical` after DEFINE, N_eff computation, the deflation formula, the sealed holdout beyond its one rate-limited call, `regime_research`, and the exploration floor.

**Approval envelopes, not per-action prompts.** Users approve ~93% of permission prompts, so per-action approval buys the feeling of control and none of the substance. Four things gate:
1. spend above the campaign's per-decision threshold
2. promotion to paper or live
3. sealed-holdout access
4. L3 tier-3 memory writes

Everything else runs free inside the envelope. **The model proposes; the harness enforces.** Hash-chained audit trail with `record_phase: pre` so you can prove denials prevented actions.

---

## 16. Observability

### 16.1 Streaming

Batched ingest → Parquet (system of record) → NATS JetStream → stateless SSE gateways. Snapshot-from-ClickHouse plus SSE deltas. **Connections scale with viewers, not runs.**

Telemetry uses a **bounded drop-oldest queue**: a monitoring system that can stall a 3-day GPU job is worse than no monitoring.

**Metric cardinality is a hard ceiling, not a soft one.** Budget: ≤100k distinct metric keys per tenant. **Per-instrument results are Parquet artifacts, never metric series** — 3,000 tickers × N runs blows the ceiling instantly and turns the tracking store into a deployment dependency.

### 16.2 Platform self-monitoring — the layer nobody builds  *[AMENDED 2026-09-14 — Appendix C, C-9]*

The system reports on **its own judgment**, not just on models:

| Signal | Alarm |
|---|---|
| M5 calibration (ECE on recent gated candidates) | ECE > 0.08 |
| M8/M9 policy entropy | below floor ⇒ blocks promotion (§14.4) |
| MNAR `b₁` significance (§5.5) | significant ⇒ naive tensor reads are biased |
| Gate pass rate by profile, 30d | drift > 2× baseline either direction |
| Exploration fraction actually achieved | < declared floor ⇒ P1 |
| Feature consistency p99 diff | > 1e-9 for deterministic features |
| Trajectory-corpus half-life | informs §14.6 |
| N_eff growth vs trial growth | divergence ⇒ trials are more correlated than they look |
| Iceberg tag coverage of registered artifacts | < 100% ⇒ P1 |
| Sealed-holdout call ledger | any second attempt logged and surfaced |

---

## 17. Failure handling

| Failure | Response |
|---|---|
| OOM | M2-predicted pre-dispatch; on occurrence, retry once at modified resource request (Flyte-style), then fail |
| NaN/divergence | hard rule, immediate stop, `censoring='failed'`, feeds M2 |
| Spot preemption | `PREEMPTED → RECOVERING`, resume from bit-reproducible checkpoint, no new trial_id |
| Data source outage | `quality_flags |= VENUE_OUTAGE`; datasets declaring that venue fail loudly rather than silently shortening |
| Late-arriving restatement | new row + `restatement_index` update; affected trials flagged `stale_input`, not deleted |
| Chain reorg | rows keyed by block_hash; `orphaned_at` set; dependent trials flagged |
| Leakage detected post-hoc | trial superseded with `leakage_detected`; **all descendants in the lineage graph flagged**; N_eff recomputed |
| Agent budget exhausted | refuse-with-alternatives so it replans; never silent kill |
| Internal model degradation | auto-rollback (Tier A) / block promotion (Tier B) |
| Tracking store degradation | training continues — the tracker is never a deployment dependency |

---

# PART III — BUILD ORDER

Sequenced by **retrofittability**, not by visibility. The things everyone wants to build first (live dashboards, cost charts, the pipeline builder) are last here on purpose — they are always addable, and the things below them never are.

### Phase 0 — Foundations that cannot be retrofitted *(build before anything runs)*
1. Instrument identity + bitemporal symbol/venue tables (§1.1)
2. Four-timestamp `bar_1m` with `knowledge_time` as a real column (§1.2)
3. `restatement_index` and the single PIT read path — no non-PIT path exists (§1.3)
4. Corporate actions / roll schedules / options contracts with `decision_time` and `announcement_time` (§1.4–1.6)
5. **Iceberg retention set to ≥90 days / ≥50 snapshots, with a tag-on-registration hook** (§0)
6. Trial Ledger with write-before-execute, hash chaining, propensity, censoring (§4)
7. Feature serving log + nightly consistency diff (§3.3)
8. Postgres RLS with `FORCE ROW LEVEL SECURITY`, `SET LOCAL`, and the three CI tests (§7.1)

### Phase 1 — Correctness *(before any result is trusted)*
9. One feature runtime, windowed-view lookback enforcement (§3.2–3.3)
10. Computed embargo + `purge_on='t1'` (§12.2)
11. Leakage suite in CI, including the platform-level random-label test (§12.5)
12. **M10 leakage detector via synthetic injection** — trainable day 1, protects everything (§13)
13. UTC master clock, staleness columns, `asof` nearest banned at library level (§2)
14. Gate stack `strict_v1`, versioned and immutable (§12.3)
15. Platform-computed N_eff (§12.4)

### Phase 2 — The training system
16. Temporal campaign workflow + job state machine with DEDUPLICATED (§9–10)
17. Ray execution, Kueue admission with mandatory `max_gpu_hours` and no default
18. Bit-reproducible checkpointing (RNG, optimizer, dataloader, AMP scaler)
19. Searcher auto-selection, hardened ASHA (all five mitigations), **≥5% exploration floor** (§11.1–11.2)
20. Fixed post-hoc pipeline: soup → ensemble → calibrate → closed-form threshold (§11.4)
21. Comparison protocol + Multiple Comparison Matrix (§11.5)
22. Agent tool surface with approval envelopes (§15)
23. Trajectory logging with `tool_schema_hash` (§14.6)

### Phase 3 — The knowledge plane
24. Tier-1 asset fingerprints with EDGE and crypto seasonal deflation (§5.2)
25. Regime model, **causal/research schema split enforced by GRANT** (§5.4)
26. Tier-2 reference portfolio, greedy-grown from the ledger (§5.2)
27. Outcome Tensor with MNAR correction, censoring, and **LCB-shrunk recommendations** (§5.5)
28. Insight store with evidence requirement and 4K injection cap (§5.6)

### Phase 4 — Internal models, in ROI order
29. M1 cost → M2 failure → M3 learning curve *(Tier A, auto-promote)*
30. **M5 gate pre-screener** — the money model *(Tier B)*
31. M9 proposal ranker *(Tier B)*
32. M4 surrogate, M8 recommender *(Tier B, per-tenant for M8)*
33. M11 anomaly, M13 critic
34. Cold-start ladder + `decision_tier` logging on all of the above (§13.1)
35. Learning-debt retraining trigger; the three tiers; freeze switch (§14)

### Phase 5 — Surfaces
36. Live metric streaming (NATS → SSE), viewer-scaled
37. **Platform self-monitoring dashboard** (§16.2) — build before the pretty charts
38. Run comparison + lineage diff + "why did A win" attribution
39. Preset / guided / expert configuration modes
40. Visual pipeline builder

### Phase 6 — Deferred by decision, not by oversight
41. Tier-3 learned asset encoder — **only on a measured probing win**, planned as fusion not replacement (§5.2)
42. M12 executor fine-tune — **only when sustained ≥100k executor-calls/day, or a determinism/privacy requirement lands, AND trajectory-corpus half-life exceeds training cadence** (§14.6)
43. ClickHouse for shared analytics — when concurrent analysts exceed ~10
44. ANN indexes on `retrieval_vec` — only when measured p99 on exact kNN fails (§5.3)

---

## Appendix A — What changed since the research brief

| # | Brief said | Spec says | Why |
|---|---|---|---|
| 1 | Bitemporal: `event_time` + `knowledge_time` | **Four** timestamps; Iceberg snapshots are *not* a bitemporal model | Compaction decouples snapshot time from knowledge events |
| 2 | Corwin–Schultz / Roll for spreads | **EDGE** | RMSE 0.38% vs CS 0.57%, Roll 1.72%; validated at minute level |
| 3 | Performance landmarkers as task representation | **Greedy submodular reference portfolio**, grown from the ledger | Auto-sklearn 2.0 deleted meta-features; TabRepo validates portfolios |
| 4 | HRP/HERC for structure and allocation | Taxonomy **only**; min-variance beat it out of sample | 2011–2025 US+Brazil study: no evidence of outperformance |
| 5 | IPW for the tensor | **MNAR joint-likelihood**; ship `b₁` as a health metric | Valid bounds when propensities approach 0/1, plus a test for bias |
| 6 | Reuse the ledger to recommend configs | Same, but **LCB-ranked and shrunk to default, non-bypassable** | Naive off-policy HPO can pick policies ~15% *worse* than the logger |
| 7 | Regime vectors | **Filtered only, enforced by GRANT**; smoothed in a separate schema | Smoothed probabilities inflate Sharpe 0.78 → 1.74 |
| 8 | Store greeks or recompute | **Store IV, never greeks** | Greeks freeze a model choice into data; IV isn't cheaply reproducible |
| 9 | Futures continuous series | **Forward-adjusted ratio default**; back-adjustment flags trials non-reproducible | Back-adjusted history mutates at every roll; can go negative |
| 10 | Asset similarity is the transfer metric | **(instrument family, venue)** is; venue is a stored coordinate | Transfer works spot↔perp, fails across assets |
| 11 | Drift-triggered retraining | **Learning-debt rule**; drift triggers retrain far too often | 0.36× excess loss vs semi-annual; ADWIN cost 3.27 vs oracle 2.68 |
| 12 | NannyML-style estimation to gate models | Fine for Tier A; **never for alpha-adjacent** | CBPE assumes no concept drift — the exact thing you're testing for |
| 13 | Cross-tenant meta-learning with care | **No for strategy content**; skip FL and DP; schema firewall + CI test | FL protects raw data; your risk is the learned function |
| 14 | Consider a vector DB | **Exact kNN in pgvector, no ANN index** | 10⁵–10⁷ × 64-d ≈ 1 ms in BLAS; also closes an approximate-neighbor side channel |
| 15 | Public tool-use benchmarks | **Private eval harness required** | 18.5% of official labels disagree with experts; 57.9–76.8% across repeats |
| 16 | Fine-tune when the toolset stabilizes | Same, plus a **measurable trigger**: corpus half-life vs training cadence | A fine-tuned executor is a cache of your API surface |

## Appendix B — The claim this architecture actually supports

A critical August 2026 review concludes the public evidence does **not** establish that AI methods deliver persistent, cross-regime, capacity-aware net alpha, with time-series foundation models showing only "small and sparse" improvements over a random walk once costs are included.

So the defensible product claim is not *"our models find alpha."* It is:

> **"We make honest testing cheap and self-deception expensive."**

Because the platform holds an immutable, propensity-logged count of every trial ever run, it can compute a deflated Sharpe ratio against a **truthful N** rather than a self-reported one. Nobody else can do that, because nobody else is structurally unable to not count. That makes the gate discipline the product rather than the overhead — and it is the one thing in this document that is genuinely hard to copy.

---

## Appendix C — Amendments (2026-09-14)

Decisions made under Mason's delegation ("you chose and continue") after the Phase 0–1 build showed where this spec assumed infrastructure the platform does not run. Each amendment changes a *mechanism*, never a requirement; the ADR named in each row records the reasoning and the revisit trigger. `INVARIANTS.md` is unchanged. Detail: `backlog/PHASE-2-5-PLAN.md`.

| # | Section | Amendment | ADR |
|---|---|---|---|
| C-1 | §1.6 | The default universe gate is confirmed as `universe_id = 1`, bitemporal from the first row. Outside it: end-of-day snapshots when a source provides them; never deletion. Under free-only data (BS-007 D-09) options are forward-collected; no minute-granular tails. | ADR-P2-26 (OQ-03) |
| C-2 | §6.1 | The engine table is read as *requirements*, not products. On this stack: Postgres 16 (+ pgvector) is the system of record; ClickHouse is the bar store, the metric store and shared analytics; features run in one Rust runtime (ADR-P0-19); artifacts are content-addressed on fs/S3; Iceberg (ADR-P0-07), MLflow, OpenLineage and per-worker DuckDB are not present and their requirements are met elsewhere. | ADR-P0-07, ADR-P0-19, ADR-P2-23 |
| C-3 | §8, §10 | "Temporal (control) + Ray (compute) + Kueue/JobSet (admission)" is replaced by: the durable job service (ADR-0030) is the control and execution plane; a campaign is an **event-sourced fold over `mlops.campaign_event`** driven by a `JobKind::Campaign` job whose phase work is idempotent child jobs. Principal attribution comes from the job service stamping the submitter from the token. `max_gpu_hours` is a REQUIRED manifest field. The §10 phase diagram and DEFINE requirements are unchanged. | ADR-P2-04, ADR-P2-05 |
| C-4 | §11.1 | The selection table stands; the arms are restricted to those this box can run — portfolio replay, prior-weighted random, TPE, GP-BO with the √D prior. CMA-ES/DEHB are N/A until a campaign exceeds 200 full-train equivalents. | ADR-P2-08 |
| C-5 | §12.3 | (a) A second profile, **`paper_v1`**, exists: `strict_v1` minus Gate 9's five-year calendar floor and minus Gate 16; it authorises paper only, never capital; `strict_v1` is unchanged and remains the only path to capital. (b) Gate 10's factor battery is declared **per asset class** on the profile (crypto: market, size, momentum, carry-when-available; equity: FF5+MOM+STR+BAB+QMJ). (c) Gate 11's crisis windows are a list on the profile. (d) Gates 13 and 14 are statistics over stored return series, not new Runs. (e) Romano–Wolf is the primary family-wise control for campaigns; BHY-within-N_eff remains for screening. | ADR-P2-14…17 |
| C-6 | §12.5 | The random-label threshold is on the t-statistic of the mean out-of-sample return, not a Sharpe ratio. Two frame screens (target correlation, full-sample normalization) join the three tests. Datasets spanning the knowledge-time sentinel are flagged, never blocked; the sources' declared lag enters the computed embargo. | ADR-P1-06, ADR-P2-24 (OQ-01) |
| C-7 | §13 | The cold-start ladder is the product at this platform's scale: every model's rule tier ships with `decision_tier` logging; each learned tier is behind an explicit ledger-size trigger. The roster and priorities are unchanged. | ADR-P4-01 |
| C-8 | §15 | The catalogue is AGENT-002 (BS-007 14); §15 is a conformance checklist over it — conventions, the not-exposed list, and the four approval-gated actions — enforced by static tests. Tools listed here but absent from AGENT-002 are added there. `approval_spend_usd` is a REQUIRED DEFINE field. | ADR-P2-19…21 |
| C-9 | §16.2 | "Iceberg tag coverage" becomes **artifact pin coverage**. An unfitted signal renders an explicit `not fitted` state. | ADR-P5-01 |

**Open questions resolved by this appendix:** OQ-01, OQ-02, OQ-03, OQ-04, OQ-09…13 (`decisions/OPEN-QUESTIONS.md`).
