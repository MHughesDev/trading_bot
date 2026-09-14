# Market Data Architecture for a Multi-Tenant Quant ML Platform

**Scope:** minute-bar (and finer) data across crypto, futures, options, equities/ETFs, and DeFi pools.
**Constraints:** point-in-time (PIT) correctness, reproducible training, immutable experiment ledgers, tenant isolation, real capital routed to brokers.
**Date of research:** September 2026. All version numbers verified against vendor/project sources listed in §12.

---

## 0. Executive position (the short version)

| Decision | Recommendation | Retrofittable? |
|---|---|---|
| Temporal model | **Bitemporal everywhere**: `event_time` + `knowledge_time`, append-only, never UPDATE | ❌ **No** |
| Price storage | **Unadjusted prices + separate adjustment-factor table**, never adjusted prices | ❌ **No** |
| Instrument identity | **Surrogate `instrument_id` (int64) + bitemporal symbol-mapping table**; tickers are attributes, not keys | ❌ **No** |
| Futures | Store **raw per-contract bars** + a **roll-schedule table**; continuous series is a *view/derived artifact*, never the source of truth | ❌ **No** |
| Table format | **Apache Iceberg v3** on object storage (fallback: DuckLake 1.0 for single-team simplicity) | ⚠️ Painful |
| File format | **Parquet** (zstd-3, delta+dictionary encodings), Arrow IPC for hot caches, Lance only for embedding/blob features | ✅ Yes |
| Partitioning | asset-class-specific; `days(event_date)` + `bucket(instrument_id, N)`; sorted by `(instrument_id, event_time)` | ⚠️ Partition evolution helps, but sort orders need rewrites |
| Serving engine | **DuckDB 1.5** embedded per-backtest-worker; **ClickHouse 26.8 LTS** for shared ad-hoc + experiment analytics | ✅ Yes |
| Feature compute | Single definition, two executions (backfill + streaming) with **mandatory online/offline consistency measurement** (Chronon pattern) | ❌ **No** (if you fork the code paths you never re-merge them) |
| Tenancy | Shared read-only market-data plane; per-tenant Iceberg namespace + S3 prefix + Postgres RLS with `FORCE ROW LEVEL SECURITY` | ⚠️ Data-plane split is retrofittable; *identity model* is not |
| Fixation | Content-addressed artifacts + hash-chained ledger in Postgres + Iceberg **tags** + S3 Object Lock (Compliance) on the ledger | ❌ **No** (you cannot retroactively prove what you didn't hash) |

**The four things that are impossible to retrofit** are called out in §11.

---

## 1. Bitemporal modeling

### 1.1 The two clocks (plus the two you forget)

Standard bitemporality (SQL:2011, XTDB, Snodgrass) gives:

- **Valid time / event time** — when the fact was true in the world. For a minute bar: the bar's interval start.
- **System time / transaction time / knowledge time** — when *your system* learned the fact.

For market data you actually need **four** timestamps, and conflating any two of them is a silent PIT bug:

| Column | Meaning | Source |
|---|---|---|
| `event_time` | bar interval start (exchange clock, UTC) | venue |
| `venue_ts` | exchange-stamped send/transact time | venue (`ts_recv` vs `ts_event` in Databento terms) |
| `ingest_time` | when *we* received the bytes | our gateway |
| `knowledge_time` | when the record became *queryable/asserted* in our store (commit time) | our catalog |

A backtest asking "what did I know at T?" must filter `knowledge_time <= T`, **not** `ingest_time <= T`, because a batch that arrived at 09:00 but only committed at 11:30 was not actionable at 10:00. Live trading, conversely, is bounded by `ingest_time` + decision latency. Store both; the gap between them is your *real* research/production skew and is worth monitoring.

### 1.2 The canonical physical pattern

Do **not** model bitemporality with `[valid_from, valid_to)` ranges that you `UPDATE` to close. That requires mutation, which destroys immutability and makes Iceberg/Delta copy-on-write churn brutal. Instead:

**Append-only "assertion log" + latest-wins resolution at read time.**

```sql
-- Logical shape for every fact table
(
  instrument_id   BIGINT      NOT NULL,
  event_time      TIMESTAMP(9) WITH TIME ZONE NOT NULL,  -- valid time
  knowledge_time  TIMESTAMP(9) WITH TIME ZONE NOT NULL,  -- transaction time
  revision        INT         NOT NULL,   -- 0 = original, n = nth restatement
  is_deleted      BOOLEAN     NOT NULL DEFAULT false,    -- tombstone for retractions
  source_id       SMALLINT    NOT NULL,   -- vendor/feed provenance
  ... payload ...
)
```

Read semantics (the *only* correct PIT read):

```sql
-- "The bars as I knew them at 2026-03-01T00:00:00Z"
SELECT * FROM (
  SELECT *, ROW_NUMBER() OVER (
           PARTITION BY instrument_id, event_time
           ORDER BY knowledge_time DESC, revision DESC) AS rn
  FROM bars_1m
  WHERE knowledge_time <= TIMESTAMP '2026-03-01 00:00:00+00'
    AND event_date BETWEEN DATE '2020-01-01' AND DATE '2026-03-01'
) WHERE rn = 1 AND NOT is_deleted;
```

This is expensive if run naively over 10 years. Mitigations, in order of leverage:

1. **Materialize the "as-of-now" projection** as a separate Iceberg table (`bars_1m_current`), rebuilt incrementally. 95%+ of queries (live trading, dashboards) want current truth. Only backtests need the log.
2. **Partition the log by `knowledge_date`** for restatement-heavy tables so `knowledge_time <= T` is a partition prune, not a row filter.
3. **Exploit the fact that restatements are rare.** Keep a small `restatements` table keyed by `(instrument_id, event_date)`. If an `event_date` has no restatement rows, the current projection *is* the PIT answer for any `knowledge_time` after its first commit. This collapses the expensive window function to a tiny anti-join for >99% of partitions. **This optimization is the single biggest practical win** and is what lets minute-bar PIT queries stay sub-second.

### 1.3 Iceberg / Delta as the bitemporal substrate

**Iceberg v3** (spec finalized 2025, broadly shipping through 2026) gives you three relevant primitives:

- **Deletion vectors** — Puffin-stored Roaring bitmaps, one per data file, replacing v2's scattered positional delete files. Materially reduces the cost of corrections when you *do* need to physically retract (GDPR-style, or vendor license revocation).
- **Row lineage** — `_row_id` and `_last_updated_sequence_number` (exposed by Trino as `$row_id`, `$last_updated_sequence_number`). Manifest-level `first_row_id` + `added_rows_count` assign IDs implicitly, so lineage costs almost nothing in metadata. This gives you free CDC over the market-data plane: "what changed since snapshot S" without a diff job.
- **`timestamp_ns` / `timestamptz_ns`** — nanosecond precision. **Use it.** Microsecond timestamps are already insufficient for options/OPRA and crypto matching-engine sequencing, and widening later is a schema migration across petabytes.

Engine support reality check (Sept 2026): Trino 483 marks Iceberg **format version 3 as experimental** (column defaults and encryption not fully supported). Starburst Enterprise 476-e / Galaxy support v3 including row lineage metadata columns. Databricks and Snowflake have shipped v3 read/write. **Do not** assume every engine in your stack can write v3 — pin writers, allow broader readers.

**Critical distinction:** Iceberg snapshots are **not** a bitemporal model. Snapshot time = commit time = your `knowledge_time` *only if* you commit exactly once per knowledge event and never compact across knowledge boundaries. Compaction (`rewrite_data_files`) creates new snapshots with new commit times while preserving old data — so `FOR TIMESTAMP AS OF` after compaction returns the *right rows* but the snapshot timeline no longer maps 1:1 to knowledge events. **Therefore: `knowledge_time` must be a real column, not inferred from snapshot metadata.** Snapshot time travel is for *operational* rollback and *artifact pinning*, not for PIT semantics.

**Delta Lake 4.0** offers an equivalent feature set (deletion vectors, liquid clustering, type widening, Delta Kernel). Liquid clustering is genuinely better than Iceberg's static sort orders for high-cardinality `instrument_id` because it reclusters incrementally without full rewrites. If you are already on Databricks, Delta is the lower-friction choice. If you want engine neutrality (DuckDB, Trino, ClickHouse, Spark, Polars all reading the same tables), Iceberg wins in 2026.

**DuckLake 1.0** (released 2026-04-13) is the dark-horse option: it puts *all* lakehouse metadata in a SQL database (Postgres/SQLite/DuckDB) instead of files in object storage. Benefits for this workload are real:
- Metadata queries are SQL, so "which files contain instrument X between dates D1 and D2" is an index lookup, not a manifest scan.
- **Data inlining** (≤10 rows by default) keeps small corrections in the catalog DB — exactly the shape of vendor restatements — avoiding the small-file explosion that plagues Iceberg when you commit thousands of tiny fix-up files.
- Sorted tables and hash bucketing on high-cardinality columns are first-class.
- Experimental Iceberg-v3-compatible deletion vectors via Puffin.

Trade-off: the catalog DB is a scaling and availability bottleneck, and the ecosystem is DuckDB-centric. **Recommendation: Iceberg v3 for the shared market-data plane; DuckLake is defensible for per-tenant experiment/feature tables where the catalog is naturally small and the access pattern is single-engine.**

**XTDB v2** is the purest bitemporal SQL database (SQL:2011 `FOR VALID_TIME AS OF`, `FOR SYSTEM_TIME AS OF`, immutable, Arrow-backed). It is excellent for **reference data**: security master, corporate actions, index membership, instrument lifecycle, tenant config. It is **not** the right store for minute bars (columnar analytical scan throughput is not its design point). Use it — or a hand-rolled bitemporal Postgres schema — for the low-volume/high-semantic-complexity half of the problem, and a lakehouse for the high-volume/low-semantic-complexity half. This split is the standard shape at real quant shops.

### 1.4 As-of joins: performance and correctness at minute scale

The as-of join is the workhorse of cross-asset PIT feature computation. Characteristics as of 2026:

| Engine | Syntax | Implementation | Notes / gotchas |
|---|---|---|---|
| **DuckDB 1.5** | `ASOF JOIN ... ON a.k = b.k AND a.t >= b.t`, `ASOF LEFT JOIN`, `USING(...)` | Hash-partition + sort RHS, then merge-join per partition. Not IEJoin. | Benchmarks: 5M-row self-join **0.425 s** vs IEJoin 3.52 s (9×); 1M build / 100K probe **0.077 s** vs IEJoin 49.5 s (~640×). All four inequality operators supported with well-defined half-open interval semantics. **`USING` gotcha:** merged columns come from the *left* table only — if you need the right-hand timestamp (to compute staleness), you must use explicit `ON` and select both. |
| **ClickHouse 26.8** | `ASOF LEFT JOIN ... ON eq AND t1.t >= t2.t` or `USING(..., t)` | `hash` and `full_sorting_merge` algorithms only. Not supported by the `Join` table engine. | The asof column **must be last** in `USING`. Only **one** inequality per query. With `hash`, the asof column cannot be the only join column. Default operator is `>=`. **Memory hazard:** the `hash` algorithm materializes the right side; for a full options chain this OOMs. Force `join_algorithm='full_sorting_merge'` for large RHS. |
| **Polars 2.0** (RC 2026-09-02) | `join_asof(..., by=..., strategy='backward'/'forward'/'nearest')` | Streaming engine is now the **default** LazyFrame engine, "easily 5×" faster with far lower memory. | `strategy='nearest'` is a **look-ahead bug generator** — it can match a future row. Ban it in feature code via lint. `by=` groups are required for per-instrument joins; without it Polars asof-joins globally and silently produces garbage across instruments. Predicate pushdown through `join_asof` is incomplete (open issue #25867), so filter *before* the join. |
| **kdb+ / q** | `aj[`sym`time; t; q]`, `aj0`, `ajf`, `ajf0` | Partitioned + `p#`-attributed merge. | `aj` returns the **left** table's time; `aj0` returns the **right** table's actual time. `ajf`/`ajf0` (v3.6+) fill nulls forward from the left. **Column order is load-bearing**: `` `sym`time `` is fast, `` `time`sym `` is catastrophically slow. On disk the right table needs `p#` on the first join column with the rest sorted within groups; a `where date=...` constraint *destroys* the `p#` attribute. Do not `select` from the right table — pass it whole so it stays memory-mapped. |

**Correctness rule that applies to all four engines:** an as-of join on `event_time` alone is *not* PIT-correct. You must as-of join on the **knowledge dimension too**, or pre-resolve the log to a PIT snapshot before joining. The common production shape is a two-stage pipeline:

```
resolve_pit(table, as_of_knowledge_time) -> materialized PIT view
   |
   +--> ASOF JOIN on event_time  (now safe)
```

Doing the knowledge filter inside the as-of join's inequality is tempting and wrong: you would need a *double* inequality (`k <= K AND e <= E`), which none of these engines express as a single asof, and the naive `ROW_NUMBER()` fallback is 10–100× slower.

### 1.5 What breaks: late data, restatements, vendor revisions

| Failure mode | Symptom | Mitigation |
|---|---|---|
| **Late-arriving bars** (venue outage backfill) | A bar for 10:00 arrives at 14:00. A feature computed at 10:05 legitimately didn't have it; a naive backfill *does*. Backtest outperforms live. | Bitemporal read (`knowledge_time <= T`). Additionally emit a `completeness` flag per (instrument, event_date) so features can refuse to compute on partial bars. |
| **Vendor restatement** (bad print corrected T+2) | Backtest sees corrected price; live saw the bad print. Strategy looks better than reality. | Never overwrite. Append `revision+1`. Backtests default to `knowledge_time = event_time + decision_lag`; a separate "final data" mode is available but must be *labelled* in the experiment ledger. |
| **Silent vendor rewrite** (file replaced in place) | You can't even detect it. | Hash every ingested file (SHA-256), store in `ingest_manifest`. Re-download and re-hash a sample daily. Vendors *do* silently rewrite history — this is not paranoia. |
| **Adjusted-price mutation** | Split occurs; the vendor's entire adjusted history changes. Your cached features are now inconsistent with your cached labels. | Store unadjusted only (§2.1). |
| **Blockchain reorg** | On-chain "facts" are literally retracted. | Genuine bitemporal retraction (§2.4). |
| **Timestamp reinterpretation** | Vendor changes from exchange-local to UTC, or from bar-start to bar-end labelling. | Version the *parser*, not just the data. `source_id` should encode `(vendor, feed, parser_version)`. A parser change is a new `source_id` and a full re-ingest with new `knowledge_time`. |
| **Compaction erasing knowledge boundaries** | §1.3. | `knowledge_time` as a column; tags before compaction. |

---

## 2. Asset-class PIT hazards and schemas

### 2.1 Equities / ETFs

**The adjusted-price trap.** An adjusted close is a function of the *entire future* corporate-action stream. The moment a new split or dividend occurs, every historical adjusted price mutates. Consequences:
- Features computed last month no longer reproduce.
- Caches silently go stale with no schema change and no error.
- Any content-addressing scheme that hashes adjusted prices produces different hashes for "the same" data.

**Rule: store `open/high/low/close/volume` exactly as printed, plus a bitemporal adjustment-factor series.** Apply adjustment at query time, as of a chosen knowledge date.

Math: the cumulative adjustment factor working backward from the most recent observation is

```
split_factor(t)  = Π over splits s > t of (1 / ratio_s)
div_factor(t)    = Π over ex-dividends d > t of (1 - amount_d / close_{d-1})
cum_adj(t)       = split_factor(t) * div_factor(t)     -- equals 1.0 at the last observation
adj_close(t)     = close(t) * cum_adj(t)
```

Because `cum_adj` is defined relative to "now", it is itself bitemporal: `cum_adj(t | knowledge_time = K)`. Storing the *events* and computing the factor is the only representation that is stable under new events.

**Other equity hazards:**
- **Survivorship** — the security master must contain delisted instruments with full lifecycle. If your universe query is `SELECT symbol FROM instruments WHERE active`, you have already lost.
- **Ticker reuse** — tickers are recycled aggressively after delisting. `AAPL` has been one company; many three-letter tickers have not. **Never key on ticker.** Key on a surrogate `instrument_id`; map symbols bitemporally.
- **Index reconstitution** — S&P/Russell membership must be stored with both **announcement date** and **effective date**. Using effective date as if it were known at announcement time is a classic look-ahead; using announcement date for the *portfolio* is also wrong (you can't hold it yet). Store both, let the strategy choose, record the choice in the ledger.
- **Halts** — LULD bands are 5% / 10% / 20% / lesser-of-$0.15-or-75% depending on price tier, **doubled during the opening and closing periods**, with a 5-minute pause after 15 seconds outside the band. Market-wide circuit breakers: Level 1 = 7%, Level 2 = 13% (both 15-min halts before 3:25 pm ET; no halt after), Level 3 = 20% (close for the day). A halted minute is *not* a zero-volume minute — it is a minute in which **you could not trade**. Encode it.

```sql
-- Instrument identity (bitemporal, low volume -> Postgres/XTDB)
CREATE TABLE instrument (
  instrument_id      BIGINT PRIMARY KEY,        -- surrogate, immutable, never reused
  asset_class        TEXT NOT NULL,             -- 'equity'|'etf'|'future'|'option'|'crypto_spot'|'crypto_perp'|'defi_pool'
  primary_venue      TEXT,
  currency           CHAR(3),
  first_seen_date    DATE NOT NULL,
  last_seen_date     DATE,                      -- NULL = live
  created_knowledge  TIMESTAMPTZ NOT NULL
);

CREATE TABLE instrument_symbol (                 -- bitemporal symbol mapping
  instrument_id   BIGINT NOT NULL,
  symbology       TEXT   NOT NULL,              -- 'ticker'|'figi'|'isin'|'cusip'|'occ'|'vendor:databento'
  symbol          TEXT   NOT NULL,
  valid_from      TIMESTAMPTZ NOT NULL,         -- event/valid time
  valid_to        TIMESTAMPTZ,                  -- NULL = open
  knowledge_from  TIMESTAMPTZ NOT NULL,
  knowledge_to    TIMESTAMPTZ,                  -- NULL = current assertion
  source_id       SMALLINT NOT NULL
);
CREATE INDEX ON instrument_symbol (symbology, symbol, valid_from);
CREATE INDEX ON instrument_symbol (instrument_id, valid_from);

CREATE TABLE corporate_action (                  -- events, not factors
  instrument_id   BIGINT NOT NULL,
  action_type     TEXT NOT NULL,                -- 'split'|'cash_div'|'stock_div'|'spinoff'|'merger'|'symbol_change'|'delist'
  ex_date         DATE NOT NULL,
  record_date     DATE,
  pay_date        DATE,
  announce_date   DATE NOT NULL,                -- <- needed for PIT: when did the market learn?
  split_ratio     NUMERIC(18,9),                -- new shares per old
  cash_amount     NUMERIC(18,9),
  currency        CHAR(3),
  knowledge_time  TIMESTAMPTZ NOT NULL,
  revision        INT NOT NULL DEFAULT 0,
  source_id       SMALLINT NOT NULL,
  PRIMARY KEY (instrument_id, action_type, ex_date, knowledge_time, revision, source_id)
);

CREATE TABLE index_membership (
  index_id        BIGINT NOT NULL,
  instrument_id   BIGINT NOT NULL,
  announce_date   DATE NOT NULL,
  effective_date  DATE NOT NULL,
  removal_announce_date DATE,
  removal_effective_date DATE,
  weight          NUMERIC(12,9),
  knowledge_time  TIMESTAMPTZ NOT NULL
);
```

```sql
-- Equity/ETF minute bars (Iceberg v3)
CREATE TABLE mkt.bars_1m_equity (
  instrument_id    BIGINT           NOT NULL,
  event_time       TIMESTAMPTZ_NS   NOT NULL,   -- bar interval START, UTC
  event_date       DATE             NOT NULL,   -- exchange session date (NOT UTC date)
  open             DECIMAL(18,6),
  high             DECIMAL(18,6),
  low              DECIMAL(18,6),
  close            DECIMAL(18,6),
  volume           BIGINT,
  trade_count      INT,
  vwap             DECIMAL(18,6),
  -- microstructure
  bid_close        DECIMAL(18,6),
  ask_close        DECIMAL(18,6),
  -- state flags: the difference between "no trades" and "could not trade"
  session_flag     TINYINT,                     -- 0=pre,1=RTH,2=post,3=auction_open,4=auction_close
  halt_flag        TINYINT,                     -- 0=none,1=LULD,2=news,3=regulatory,4=MWCB
  is_synthetic     BOOLEAN,                     -- true if forward-filled/interpolated by us
  -- bitemporal + provenance
  knowledge_time   TIMESTAMPTZ_NS   NOT NULL,
  ingest_time      TIMESTAMPTZ_NS   NOT NULL,
  revision         INT              NOT NULL DEFAULT 0,
  is_deleted       BOOLEAN          NOT NULL DEFAULT false,
  source_id        SMALLINT         NOT NULL
)
PARTITIONED BY (event_date, bucket(instrument_id, 16))
-- write.sort-order: instrument_id, event_time
TBLPROPERTIES (
  'format-version'='3',
  'write.target-file-size-bytes'='268435456',           -- 256 MB
  'write.parquet.compression-codec'='zstd',
  'write.parquet.compression-level'='3',
  'write.parquet.row-group-size-bytes'='16777216',      -- 16 MB: fine-grained pruning
  'write.distribution-mode'='range',
  'write.metadata.previous-versions-max'='200',
  'history.expire.min-snapshots-to-keep'='50',
  'history.expire.max-snapshot-age-ms'='7776000000'     -- 90 days, NOT the 5-day default
);
```

> **Note on `event_date`:** partition on the **exchange session date**, not the UTC calendar date. An 09:30 ET bar on 2026-03-08 is 14:30 UTC; a Sydney session spans two UTC dates. Partitioning on UTC date shreds sessions across partitions and makes "give me one trading day" a two-partition scan forever.

### 2.2 Futures

**The core problem:** a back-adjusted continuous series is not a price. It is a synthetic construct whose *entire history changes at every roll*. A Panama-adjusted ES series recomputed today differs from the one you computed last month at every point before the last roll. Worse, additive (Panama/difference) back-adjustment can produce **negative prices** deep in history for contangoed commodities, breaking log returns and any percentage-based feature.

**Rule: the source of truth is per-contract raw bars plus a roll-schedule table. Continuous series are derived, versioned artifacts with a `method_id`.**

Roll methods to support (all three are used in production, and results differ materially):
- **Calendar** — roll N days before expiry/first-notice. Deterministic, PIT-trivially-correct, slightly suboptimal liquidity.
- **Open interest** — roll when back-month OI exceeds front-month. **PIT hazard:** OI is published with a one-day lag by CME. Rolling on same-day OI is look-ahead.
- **Volume** — roll when back-month volume exceeds front-month, usually with an N-consecutive-day confirmation. **PIT hazard:** intraday volume crossover is only known at session end.

Databento's continuous symbology (`ES.c.0`, `ES.n.0`, `ES.v.0` for calendar / open-interest / volume rules, with `.N` as the rank index) explicitly delivers **unadjusted** prices: *"The continuous contract prices provided are the original, unadjusted prices. Unlike some vendor implementations that back-adjust prices to remove jumps during rollovers, our approach maintains the original properties of the data."* This is the correct vendor behaviour and the model to copy.

Adjustment methods, and what each is for:
- **None (stitched)** — correct for simulating actual fills. Has gaps at rolls. **Use this for execution simulation.**
- **Difference / Panama (additive)** — preserves absolute point moves. Can go negative. Use for P&L-in-points.
- **Ratio (multiplicative)** — preserves returns, never negative. **Use this for return-based features and ML targets.**

```sql
CREATE TABLE ref.futures_contract (
  instrument_id      BIGINT PRIMARY KEY,
  root_symbol        TEXT NOT NULL,             -- 'ES'
  exchange           TEXT NOT NULL,             -- 'XCME'
  contract_month     DATE NOT NULL,             -- 2026-03-01
  contract_code      TEXT NOT NULL,             -- 'ESH6'
  first_trade_date   DATE,
  last_trade_date    DATE NOT NULL,
  first_notice_date  DATE,
  expiry_date        DATE NOT NULL,
  settlement_type    TEXT,                      -- 'cash'|'physical'
  tick_size          DECIMAL(18,9) NOT NULL,
  point_value        DECIMAL(18,6) NOT NULL,    -- multiplier
  currency           CHAR(3),
  knowledge_time     TIMESTAMPTZ NOT NULL
);

-- Roll schedule: the PIT-correct record of when each rule SAID to roll,
-- and WHEN THAT WAS KNOWABLE.
CREATE TABLE ref.futures_roll_schedule (
  root_symbol        TEXT NOT NULL,
  method_id          SMALLINT NOT NULL,         -- FK -> continuous_method
  rank               SMALLINT NOT NULL,         -- 0 = front, 1 = second, ...
  roll_date          DATE NOT NULL,             -- session on which the switch takes effect
  from_instrument_id BIGINT NOT NULL,
  to_instrument_id   BIGINT NOT NULL,
  decision_time      TIMESTAMPTZ NOT NULL,      -- when the rule could first be evaluated
  knowledge_time     TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (root_symbol, method_id, rank, roll_date, knowledge_time)
);

CREATE TABLE ref.continuous_method (
  method_id       SMALLINT PRIMARY KEY,
  roll_rule       TEXT NOT NULL,                -- 'calendar_n_days'|'open_interest'|'volume'
  roll_params     JSON NOT NULL,                -- {"days_before_expiry":5} / {"confirm_days":2}
  adjustment      TEXT NOT NULL,                -- 'none'|'difference'|'ratio'
  anchor          TEXT NOT NULL,                -- 'back'|'forward'  (back-adjust vs forward-adjust)
  definition_hash CHAR(64) NOT NULL             -- content address of the full spec
);
```

Continuous bars are then a **materialized derived table** keyed by `(root_symbol, method_id, rank, event_time)`, carrying `underlying_instrument_id` (the actual contract) and `adj_factor` so you can always recover the raw price. Crucially it carries its own `knowledge_time`: when you rebuild the continuous series after a new roll, you **append a new generation**, you do not overwrite.

> **Forward-adjustment is the underrated fix.** Back-adjustment mutates history; *forward*-adjustment (anchor the series at its start and adjust future prices) leaves history immutable, so a series computed in 2020 still reproduces bit-for-bit in 2026. The cost is that the "current" level of the series drifts from the actual contract price, which matters for display but not for returns-based ML. **For a reproducibility-first platform, forward-adjusted ratio series should be the default**, with back-adjusted offered for human-facing charts only.

### 2.3 Options

**Cardinality is the whole problem.** Databento reports roughly **1.6 million live OPRA instruments**, 96 multicast channels across 18 exchanges, **>200 billion quote/NBBO updates per day**, with microbursts exceeding **50 Gbps**. Even after discarding the ~80% of exchange updates that don't move the NBBO, this is the largest feed in US markets — their zstd-compressed daily PCAPs are ~80× the size of Nasdaq TotalView MBO.

Naive minute bars for options: 1.6M instruments × 390 RTH minutes = **624 M rows/day**, ~160 B rows/year, before you even store greeks. At ~40 bytes/row compressed that is ~25 GB/day, ~6.3 TB/year. Ten years of that is not a research budget, it is a data-center.

**Compression strategies that actually work, in order of impact:**

1. **Don't store empty bars.** The overwhelming majority of the 1.6M instruments have zero volume in any given minute. Storing only bars with `trade_count > 0 OR quote_changed` cuts row count by roughly **90–95%**. The cost: consumers must as-of join rather than array-index. This is the correct trade.
2. **Moneyness/DTE gating.** Define a `liquid_universe` predicate — e.g. |log(K/S)| < 0.35 and DTE ≤ 90 — and store full minute granularity only inside it. Outside it, store end-of-day snapshots. Typical retention: **~2–5% of the chain carries >95% of the traded volume.** Store the *universe definition* bitemporally so backtests know which regime applied.
3. **Long schema, not wide.** A wide "one row per (underlying, minute) with 2,000 strike columns" schema is a schema-evolution nightmare (new strikes list daily) and defeats columnar compression. **Long format** — one row per (option_instrument_id, minute) — with `bucket(underlying_id, N)` partitioning and sort order `(underlying_id, expiry, strike, event_time)` gives you excellent dictionary and delta encoding: strikes are dense arithmetic sequences, expiries are low-cardinality, and adjacent rows differ by one tick.
4. **Store the surface, not just quotes.** A calibrated SVI (or SSVI/SABR) surface is ~5 parameters per (underlying, expiry, minute) vs ~200 strikes. For an underlying with 15 expiries that is 75 floats vs 3,000 quote rows — a **~40× reduction** for the features most models actually consume. **Store both**: raw quotes inside the liquid universe (needed for execution realism and for recalibration), surface parameters for everything (needed for features). Persist the calibration residuals too — a surface with bad fit quality is a data-quality signal.
5. **Greeks: recompute, don't store — with one exception.** Greeks are a deterministic function of (S, K, T, r, q, σ, model). Storing them multiplies row width ~2.5× and, worse, **freezes a model choice into your data**. Recompute from stored IV at feature time. The exception: store **IV** itself (not the greeks), because IV depends on the exact quote, timestamp alignment, and rate/dividend curve you used, and is therefore *not* cheaply reproducible later. Store IV plus the `pricing_model_id` and the rate/borrow curve version used to derive it.

**Expiry and assignment.** An option that expires is not "delisted with no data" — it has a terminal payoff. Store an explicit `option_settlement` record (settlement price of underlying, ITM/OTM determination, assignment for short positions, exercise style). PM vs AM settlement matters for index options and is a recurring source of silent one-day P&L errors.

```sql
CREATE TABLE ref.option_contract (
  instrument_id      BIGINT PRIMARY KEY,
  underlying_id      BIGINT NOT NULL,
  occ_symbol         TEXT NOT NULL,             -- root+yymmdd+C/P+strike*1000
  root               TEXT NOT NULL,             -- distinguishes adjusted (AAPL1) from standard (AAPL)
  expiry_date        DATE NOT NULL,
  strike             DECIMAL(18,6) NOT NULL,
  option_type        CHAR(1) NOT NULL,          -- 'C'|'P'
  exercise_style     CHAR(1) NOT NULL,          -- 'A'|'E'
  settlement_time    CHAR(2) NOT NULL,          -- 'AM'|'PM'
  contract_size      INT NOT NULL,              -- 100, or adjusted after corp action
  is_adjusted        BOOLEAN NOT NULL,          -- non-standard deliverable
  deliverable        JSON,                      -- post-corp-action deliverable basket
  listed_date        DATE,
  knowledge_time     TIMESTAMPTZ NOT NULL
);

CREATE TABLE mkt.bars_1m_option (                -- LONG format, sparse
  instrument_id    BIGINT         NOT NULL,
  underlying_id    BIGINT         NOT NULL,     -- denormalized for partition pruning
  expiry_date      DATE           NOT NULL,     -- denormalized for sort/prune
  strike           DECIMAL(18,6)  NOT NULL,     -- denormalized; delta-encodes beautifully
  option_type      CHAR(1)        NOT NULL,
  event_time       TIMESTAMPTZ_NS NOT NULL,
  event_date       DATE           NOT NULL,
  open             DECIMAL(14,4), high DECIMAL(14,4),
  low              DECIMAL(14,4), close DECIMAL(14,4),
  volume           INT,
  trade_count      INT,
  bid_close        DECIMAL(14,4),
  ask_close        DECIMAL(14,4),
  bid_size_close   INT,
  ask_size_close   INT,
  open_interest    INT,                         -- daily, carried forward; note T+1 publication lag
  iv_close         REAL,                        -- store IV, NOT greeks
  underlying_close DECIMAL(18,6),               -- the S used for IV: pins reproducibility
  pricing_model_id SMALLINT,                    -- which model produced iv_close
  curve_version_id SMALLINT,                    -- rate/borrow curve used
  knowledge_time   TIMESTAMPTZ_NS NOT NULL,
  revision         INT NOT NULL DEFAULT 0,
  source_id        SMALLINT NOT NULL
)
PARTITIONED BY (event_date, bucket(underlying_id, 64))
-- sort order: underlying_id, expiry_date, option_type, strike, event_time
TBLPROPERTIES ('format-version'='3','write.target-file-size-bytes'='536870912');

CREATE TABLE mkt.vol_surface_1m (                -- SVI per (underlying, expiry, minute)
  underlying_id    BIGINT NOT NULL,
  expiry_date      DATE   NOT NULL,
  event_time       TIMESTAMPTZ_NS NOT NULL,
  event_date       DATE   NOT NULL,
  forward          DECIMAL(18,6),
  tau              REAL,                        -- year fraction
  svi_a REAL, svi_b REAL, svi_rho REAL, svi_m REAL, svi_sigma REAL,
  n_quotes         SMALLINT,                    -- how many quotes fed the fit
  rmse             REAL,                        -- fit quality -> data-quality signal
  arb_free         BOOLEAN,                     -- butterfly/calendar arbitrage check passed
  calib_version    SMALLINT NOT NULL,
  knowledge_time   TIMESTAMPTZ_NS NOT NULL,
  source_id        SMALLINT NOT NULL
)
PARTITIONED BY (months(event_date), bucket(underlying_id, 8));
```

**Vendor reality (2026):**
- **Databento** (OPRA.PILLAR): full-depth and NBBO schemas (`mbp-1`, `tbbo`, `ohlcv-1m`, `definition`, `statistics`), pay-as-you-go by bytes, nanosecond `ts_recv`/`ts_event` separation — the cleanest bitemporal-friendly options feed available.
- **Polygon.io — now branded "Massive"** — flat files as **compressed CSV** over an S3-compatible endpoint (`https://files.massive.com`, bucket `flatfiles`), covering stocks, options, futures (CME), indices, forex, crypto. CSV is a real cost: you will re-encode to Parquet on ingest, budget the CPU.
- **Cboe DataShop** — end-of-day and summary products, open/close volume summary; good for EOD chains, not intraday.
- **ORATS** — pre-computed chains with greeks and smoothed IV surfaces. Convenient, but it bakes *their* model into your data; treat it as a derived/reference product, not raw.

### 2.4 Crypto

**No sessions.** 24/7/365, no holidays, no auctions. This is simplifying for bar construction and *complicating* for cross-asset alignment (§3). Minute bars are exactly 1,440/day with no exceptions — which means missing minutes are unambiguously data gaps, never "market closed." Exploit this: a completeness check on crypto is trivially `COUNT(*) = 1440`.

**Exchange fragmentation.** BTC-USD trades on 20+ venues with different tick sizes, lot sizes, fee schedules, and quality. There is no NBBO. Consequences:
- Store **per-venue** bars as the source of truth. Never store only a composite.
- Build a **composite/canonical price** as an explicitly-versioned derived series.

The industry-standard methodology is the **CME CF Reference Rate**: a **one-hour observation window** partitioned into **12 five-minute intervals**; within each partition compute the **volume-weighted median** trade price; the rate is the **equally-weighted average of the 12 partition medians**. Volume-weighted *median* (not mean) is the key robustness choice — it is insensitive to wash prints and fat-finger outliers. The companion BRTI republishes every ~200 ms from order-book data.

For minute bars, adapt: per minute, compute the volume-weighted median across eligible venues; require ≥ 3 eligible venues; publish `n_venues` and a dispersion metric alongside. Venue eligibility must itself be a bitemporal table (exchanges get delisted from the constituent set; Mt. Gox and FTX are the cautionary tales).

**Wash trading is a first-order data-quality problem, not an edge case.** Research finds suspicious centralized exchanges report **96–98% questionable volume**; Mt. Gox fabricated volume may have reached **60% of daily volume** in 2011–13. Detection signals worth computing and storing per (venue, asset, day):
- **Trade-size roundness** — excess mass at round sizes; responsive to short-term manipulation.
- **Benford's law** deviation on leading digits of trade size — detects sustained manipulation.
Store these as **venue quality scores** and use them as *weights* in composite construction, not as a binary filter (binary filters cause discontinuities in your price series at the threshold).

**Perpetual funding** is a distinct fact type with its own clock: most venues settle every 8 hours (00:00/08:00/16:00 UTC), some hourly, and the *predicted* rate updates continuously while the *realized* rate is known only at settlement. This is a textbook bitemporal case — store predicted and realized separately with their own knowledge times. Funding is a material component of perp carry; getting it wrong biases every basis strategy.

```sql
CREATE TABLE mkt.bars_1m_crypto (
  instrument_id   BIGINT NOT NULL,              -- (venue, base, quote, contract_type) surrogate
  venue_id        SMALLINT NOT NULL,
  event_time      TIMESTAMPTZ_NS NOT NULL,
  event_date      DATE NOT NULL,                -- UTC date; crypto has no session concept
  open DECIMAL(24,10), high DECIMAL(24,10),
  low  DECIMAL(24,10), close DECIMAL(24,10),
  volume_base     DECIMAL(28,10),
  volume_quote    DECIMAL(28,10),
  trade_count     INT,
  buy_volume_base DECIMAL(28,10),               -- taker-side split: cheap, high-signal
  vwap            DECIMAL(24,10),
  bid_close       DECIMAL(24,10),
  ask_close       DECIMAL(24,10),
  -- perp-specific
  mark_price      DECIMAL(24,10),
  index_price     DECIMAL(24,10),
  open_interest   DECIMAL(28,10),
  knowledge_time  TIMESTAMPTZ_NS NOT NULL,
  ingest_time     TIMESTAMPTZ_NS NOT NULL,
  revision        INT NOT NULL DEFAULT 0,
  source_id       SMALLINT NOT NULL
)
PARTITIONED BY (months(event_date), venue_id)
-- sort: instrument_id, event_time
;

CREATE TABLE mkt.perp_funding (
  instrument_id   BIGINT NOT NULL,
  venue_id        SMALLINT NOT NULL,
  funding_time    TIMESTAMPTZ NOT NULL,         -- settlement instant (valid time)
  interval_sec    INT NOT NULL,                 -- 28800 for 8h, 3600 for 1h
  rate_predicted  DECIMAL(18,12),               -- last predicted before settlement
  rate_realized   DECIMAL(18,12),               -- actual applied
  premium_index   DECIMAL(18,12),
  interest_rate   DECIMAL(18,12),
  knowledge_time  TIMESTAMPTZ NOT NULL,
  source_id       SMALLINT NOT NULL,
  PRIMARY KEY (instrument_id, funding_time, knowledge_time, source_id)
);

CREATE TABLE ref.venue_quality (                 -- bitemporal venue eligibility & scores
  venue_id            SMALLINT NOT NULL,
  asset_id            BIGINT NOT NULL,
  eval_date           DATE NOT NULL,
  roundness_score     REAL,                      -- wash-trading indicator
  benford_mad         REAL,                      -- mean abs deviation from Benford
  uptime_pct          REAL,
  is_eligible         BOOLEAN NOT NULL,          -- feeds composite construction
  composite_weight    REAL,
  knowledge_time      TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (venue_id, asset_id, eval_date, knowledge_time)
);
```

### 2.5 DeFi pools

**Reorgs are the purest bitemporal problem in the entire platform: the chain literally retracts facts you already asserted.** Envio's data: ~**1% of Ethereum blocks** undergo reorgs; Polygon has seen a **157-block reorg**; OP-Stack chains (Base, Optimism) are largely reorg-resistant at the head. If your indexer writes at the chain head without rollback logic, your historical data is quietly wrong.

The correct model: **blocks are facts with a finality status that evolves over time.**

```sql
CREATE TABLE chain.block (
  chain_id        INT NOT NULL,
  block_number    BIGINT NOT NULL,
  block_hash      BYTEA NOT NULL,               -- part of the key: forks share numbers
  parent_hash     BYTEA NOT NULL,
  block_time      TIMESTAMPTZ NOT NULL,         -- irregular: ~12s ETH, ~2s Polygon, ~400ms Solana
  finality        TEXT NOT NULL,                -- 'head'|'safe'|'finalized'|'orphaned'
  finalized_at    TIMESTAMPTZ,
  knowledge_time  TIMESTAMPTZ NOT NULL,         -- when we asserted THIS finality status
  PRIMARY KEY (chain_id, block_number, block_hash, knowledge_time)
);
```

Every downstream fact carries `(chain_id, block_number, block_hash)`, **not just block number**. A reorg is then modelled as: insert a new `block` row with `finality='orphaned'` and a fresh `knowledge_time`, and insert tombstone/retraction rows for dependent facts. Nothing is deleted. A backtest at knowledge time K sees exactly what an indexer would have believed at K — including the wrong, pre-reorg state, which is *correct*, because a live strategy would have acted on it.

**For research features, gate on finality.** Default features to `finality IN ('finalized')` with a documented lag (~13 min / 2 epochs on Ethereum post-Merge). Head-of-chain features are legitimate for latency-sensitive strategies but must carry an explicit `finality_horizon` parameter recorded in the experiment ledger.

**Block-time irregularity** breaks minute bars. Two valid representations, and you want both:
- **Block-native series** — one row per (pool, block). Exact, no interpolation, but irregular sampling.
- **Minute-aligned series** — last-known state as of each minute boundary, i.e. an as-of join from block-native to a minute grid, with `staleness_blocks` and `staleness_sec` columns so models can discount stale observations. **Never forward-fill silently.**

**MEV contamination.** Sandwich attacks mean the observed intra-block price path is partly an artifact of extraction, not information. Practical handling: store the **transaction index within block** and a `mev_flag` derived from an external classifier (EigenPhi-style) or heuristics (same-searcher tx before and after in the same block, opposite direction). Compute pool prices from **block-boundary reserves** rather than from individual swap prints wherever possible — block-boundary state is post-MEV-settlement and much less contaminated.

```sql
CREATE TABLE defi.pool_state (
  chain_id        INT NOT NULL,
  pool_id         BIGINT NOT NULL,
  block_number    BIGINT NOT NULL,
  block_hash      BYTEA NOT NULL,
  block_time      TIMESTAMPTZ NOT NULL,
  sqrt_price_x96  NUMERIC(60,0),                -- Uniswap v3/v4 native
  tick            INT,
  liquidity       NUMERIC(60,0),                -- active in-range liquidity
  reserve0        NUMERIC(60,0),
  reserve1        NUMERIC(60,0),
  tvl_usd         DECIMAL(28,6),
  fee_growth_global0 NUMERIC(78,0),
  fee_growth_global1 NUMERIC(78,0),
  fees_usd_cum    DECIMAL(28,6),
  finality        TEXT NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  is_deleted      BOOLEAN NOT NULL DEFAULT false,  -- reorg retraction
  source_id       SMALLINT NOT NULL
)
PARTITIONED BY (chain_id, months(block_time), bucket(pool_id, 16));
```

**Indexing infrastructure (2026).** Benchmark evidence (Sentio, May 2025) has Envio HyperIndex completing a Uniswap-V2-factory workload in **8 s**, ~15× faster than Subsquid, ~142× faster than The Graph, ~157× faster than Ponder. For an institutional platform the more relevant axis is *correctness and delivery*:
- **Allium** — warehouse-native delivery (Snowflake/BigQuery/Databricks), SOC 1 & SOC 2 Type 1/2, 99.9% SLA, Kafka/PubSub/SNS streaming at 1–2 s, 150+ chains, 10,000+ pre-decoded schemas; independently validated at 0.000011% deviation vs a reference, against 7.2% deviation measured for Dune in the same research.
- **Dune** — unbeatable for exploratory/community SQL; hosted-only, no certifications, data lives in their infrastructure. Fine for research spikes, not for a production feature pipeline.
- **Goldsky** — 150+ chains, Mirror streaming into your own DB; good middle ground.
- **The Graph** — largest subgraph ecosystem, but AssemblyScript handlers and the slowest indexing in the benchmark set.

**Recommendation:** buy decoded, reorg-aware data (Allium or Goldsky) rather than running indexers; run your own Envio/Ponder indexer only for pools your vendors don't decode. The reorg-handling and re-decoding burden is where self-hosted indexing actually costs you.

---

## 3. Cross-asset time alignment

### 3.1 The look-ahead surface

Aligning instruments with different clocks into one feature matrix is where most look-ahead bias is injected, because the errors are *invisible* — the output is a well-formed rectangular matrix either way.

Four distinct hazards:

1. **Naive resample/reindex.** `df.reindex(minute_grid).ffill()` forward-fills a value from *before* the grid point — usually correct — but pandas/Polars `reindex` with `method='nearest'` or `interpolate()` pulls from the future. Ban `nearest` and `interpolate` in feature code.
2. **Timezone-collapsed joins.** Joining on naive local timestamps across venues silently misaligns by the UTC offset, and DST transitions create duplicate/missing hours. **Store everything in UTC with nanosecond precision; keep `event_date` as the *session* date in a separate column.**
3. **Session-boundary leakage.** A US equity "daily close" at 16:00 ET is knowable to a crypto strategy at 21:00 UTC, but a Tokyo strategy's 15:00 JST close is *not* knowable to the US strategy until the next US session. As-of joining on raw UTC timestamps handles this correctly *automatically* — which is exactly why as-of joins are preferred over calendar-aligned joins.
4. **Publication lag on non-price data.** Open interest (T+1 from CME), short interest (bi-monthly, ~2-week lag), fundamentals, on-chain finality. Each needs an explicit `available_at = event_time + publication_lag` column; as-of join on `available_at`, never on `event_time`.

### 3.2 The correct primitive

> **"The last value known as of time t" is an ASOF LEFT JOIN with a strict-or-equal backward inequality on an availability timestamp, plus an explicit staleness column, plus a max-staleness policy.**

Concretely:

```sql
WITH grid AS (          -- the master clock: minute grid in UTC
  SELECT ts FROM GENERATE_SERIES(
    TIMESTAMP '2026-01-01 00:00:00+00',
    TIMESTAMP '2026-02-01 00:00:00+00',
    INTERVAL 1 MINUTE) AS t(ts)
),
eq AS (SELECT instrument_id, event_time AS available_at, close FROM pit_bars_equity),
cx AS (SELECT instrument_id, event_time AS available_at, close FROM pit_bars_crypto)
SELECT
  g.ts,
  e.close  AS spy_close,
  DATE_DIFF('second', e.available_at, g.ts) AS spy_staleness_sec,
  c.close  AS btc_close,
  DATE_DIFF('second', c.available_at, g.ts) AS btc_staleness_sec
FROM grid g
ASOF LEFT JOIN eq e ON e.instrument_id = 1 AND g.ts >= e.available_at
ASOF LEFT JOIN cx c ON c.instrument_id = 2 AND g.ts >= c.available_at;
```

Then apply a **staleness policy** per feature: `spy_close` is NULL-ed (not forward-filled) when `spy_staleness_sec > 86400` — i.e. a weekend gap is representable, and a model can learn "US market is closed" rather than being fed a stale price as if it were live. **Emitting staleness as a feature is strictly better than forward-filling silently**, and it is nearly free.

### 3.3 Session calendars

Use **`exchange_calendars` 4.13.2** (released 2026-03-10, 50+ exchange calendars, Python 3.10–3.14; the 2026 release added Eurex XEUR and Börse Stuttgart XSTU). Its API gives `sessions_in_range()`, `session_minutes()`, `is_trading_minute()`, plus break handling (`is_break_minute()`) for lunch-break exchanges like TSE and HKEX.

Two non-obvious cautions:
- The library models **regular trading hours only** — no pre/post market, no auction phases. If you trade extended hours, you need your own overlay.
- **Calendars are revised retroactively.** Exchanges announce holidays, and `exchange_calendars` ships corrections in new releases. A backtest run against v4.12 and re-run against v4.13 can differ. **Pin the calendar library version in the experiment ledger, and snapshot the materialized session table into the lakehouse** so a 2026 backtest reproduces in 2030 even if the library has moved on. This is an under-appreciated reproducibility hole.

```sql
CREATE TABLE ref.session_calendar (
  calendar_id     TEXT NOT NULL,                -- 'XNYS','XCME','24_7'
  session_date    DATE NOT NULL,                -- the trading session label
  open_utc        TIMESTAMPTZ NOT NULL,
  close_utc       TIMESTAMPTZ NOT NULL,
  break_start_utc TIMESTAMPTZ,
  break_end_utc   TIMESTAMPTZ,
  is_half_day     BOOLEAN NOT NULL DEFAULT false,
  calendar_pkg_version TEXT NOT NULL,           -- 'exchange_calendars==4.13.2'
  knowledge_time  TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (calendar_id, session_date, knowledge_time)
);
```

**Master clock choice.** Use a **UTC minute grid** as the canonical clock for cross-asset features, not any exchange's session grid. Rationale: crypto and DeFi have no sessions; a UTC grid is the only clock all five asset classes share; session structure is then recoverable as *features* (`is_rth_xnys`, `minutes_since_open`, `minutes_to_close`) rather than being baked into the index. Building the matrix on an equity session grid and bolting crypto on is the common mistake — it makes 24/7 assets un-representable overnight and forces exactly the kind of forward-fill that leaks.

---

## 4. Storage layout and formats

### 4.1 Format selection

| Format | Verdict | Why |
|---|---|---|
| **Parquet** | **Primary.** All at-rest market data. | Universal engine support, mature encodings (RLE_DICTIONARY, DELTA_BINARY_PACKED, DELTA_BYTE_ARRAY, BYTE_STREAM_SPLIT for floats), page-level statistics, predicate pushdown, zstd. |
| **ORC** | No. | Marginally better compression in some cases; materially worse ecosystem outside Hive/Spark. No reason to split the stack. |
| **Arrow IPC / Feather** | **Hot cache only.** | Zero-copy mmap, no decode cost. Ideal for the working set a backtest worker reads repeatedly. ~2–3× larger than Parquet on disk; that's fine on local NVMe. |
| **Lance v2.2** | **Narrow use.** | Claims 50%+ storage reduction vs prior version and up to 68× faster blob reads; genuinely excellent random access (the design point). Use for embedding/model-artifact/blob features. Do **not** migrate OHLCV to it — you lose Trino/ClickHouse/Spark interop for a benefit (random row access) that minute-bar scans don't need. |

### 4.2 Encodings and realistic compression

For OHLCV columns, expected Parquet behaviour:

| Column | Encoding | Typical ratio |
|---|---|---|
| `event_time` (ns, regular grid) | DELTA_BINARY_PACKED | **20–100×** (deltas are constant 60e9) |
| `instrument_id` (sorted) | RLE_DICTIONARY | **50–200×** within a sorted row group |
| `open/high/low/close` (decimal or scaled int) | DELTA_BINARY_PACKED on scaled int64 | **3–6×** |
| `open/high/low/close` (float64) | BYTE_STREAM_SPLIT + zstd | **2–3×** |
| `volume` | DELTA / bitpacked | **3–5×** |
| `option_type`, flags | RLE_DICTIONARY | **100×+** |
| `strike` (sorted, arithmetic) | DELTA | **10–30×** |
| symbol strings | FSST / DELTA_BYTE_ARRAY + dict | **5–15×** |

**Store prices as scaled `int64`/`DECIMAL`, not `float64`.** This is worth ~2× on the price columns alone because delta encoding works on integers and not on IEEE754 mantissas, and it eliminates a whole class of reproducibility bugs (float summation order). The tick size gives you the scale factor per instrument.

Codec choice: **zstd level 3** is the right default. ClickHouse's published guidance puts LZ4 at ~2× / 3–5 GB/s decompression, ZSTD(1–3) at ~2.5× / 1–2 GB/s, and ZSTD(9–19) at ~3× / 0.3–1 GB/s. Going from zstd-3 to zstd-15 buys roughly 15–20% size for a 3–5× decompression slowdown — a bad trade for a query-heavy workload. **Exception: cold-tier archives** where you rewrite once and read rarely — zstd-15 there is fine.

Reference points for what columnar compression achieves in practice: ClickHouse reports 100 M rows in **9.26 GiB** vs ~100 GiB in PostgreSQL (**~10×**), with "5–10× on typical analytical data and 30×+ on low-cardinality columns"; Character.AI reports **15–20× average, up to 50×** on some columns. TigerData claims up to **98% compression** with Hypercore columnstore. For minute bars specifically, **20–40 bytes/row compressed** is the realistic planning number for equities/crypto/futures, and **25–50 bytes/row** for options (wider schema, more columns).

### 4.3 Partitioning, file sizing, sort orders

**Governing principle:** partition on what you *filter*, sort on what you *scan and join*. For this workload the dominant query patterns are:
- **(A) Backtest load:** "all bars for universe U over date range D1..D2" — filters date, filters/semijoins instrument set, scans everything in range.
- **(B) Feature compute:** "one instrument, long history" — filters instrument, scans full history.
- **(C) Ad-hoc/research:** arbitrary.

(A) wants date partitioning. (B) wants instrument locality. You cannot partition on both (instrument cardinality is far too high for directory partitioning — 1.6 M options would create catastrophic small files). **Resolution: partition by date, bucket by instrument, sort within file by (instrument_id, event_time).** Bucketing gives (B) partition pruning to 1/N of files; the sort order gives row-group-level min/max pruning within them.

| Table | Partition spec | Sort order | Target file | Rationale |
|---|---|---|---|---|
| `bars_1m_equity` | `days(event_date), bucket(instrument_id, 16)` | `instrument_id, event_time` | 256 MB | ~10k instruments × 960 min ≈ 10 M rows/day ≈ 300 MB/day compressed → 16 buckets × ~20 MB is too small; use `days` only above ~5k instruments, else `months` |
| `bars_1m_crypto` | `months(event_date), venue_id` | `instrument_id, event_time` | 256 MB | 24/7, moderate instrument count; monthly partitions keep files healthy |
| `bars_1m_future` | `months(event_date), root_symbol` | `instrument_id, event_time` | 256 MB | Root-symbol partitioning matches how futures research actually queries |
| `bars_1m_option` | `days(event_date), bucket(underlying_id, 64)` | `underlying_id, expiry_date, option_type, strike, event_time` | 512 MB | Huge volume; the sort order makes strike/expiry ranges contiguous, which is how chains are queried |
| `vol_surface_1m` | `months(event_date), bucket(underlying_id, 8)` | `underlying_id, expiry_date, event_time` | 256 MB | ~40× smaller than raw chains |
| `defi_pool_state` | `chain_id, months(block_time), bucket(pool_id, 16)` | `pool_id, block_number` | 128 MB | Reorg rewrites are localized to recent partitions |
| `corporate_action`, reference | unpartitioned or `years()` | `instrument_id, ex_date` | 64 MB | Tiny; don't over-engineer |

**File sizing.** Iceberg's `write.target-file-size-bytes` defaults to ~512 MB; 128–256 MB is the common recommendation for streaming/incremental writes. Starburst's operational guidance: files **< 100 MB cause performance problems at scale across all engines**, and partitioning is worth it mainly above ~1 TB. Set `write.parquet.row-group-size-bytes` to **16 MB** (vs the 128 MB default) — smaller row groups mean finer min/max pruning, which matters enormously when a sorted `instrument_id` lets you skip 95% of row groups. The cost is slightly more metadata; at these file sizes it's negligible.

**Small-file management.** Minute-bar ingestion is naturally incremental and will generate small files. Budget for:
- Hourly `rewrite_data_files` on the current-day partition.
- Nightly full-partition compaction with sort order applied.
- `rewrite_manifests` weekly.
- `expire_snapshots` with `history.expire.min-snapshots-to-keep >= 50` and `max-snapshot-age-ms` of **90 days**, not the 5-day default (see §8 hazard).

**Z-order / clustering.** Iceberg supports Z-order in `rewrite_data_files` sort strategy; Delta's **liquid clustering** is better for this workload because it reclusters incrementally. For minute bars a plain lexicographic sort on `(instrument_id, event_time)` beats Z-order — Z-order helps when you filter on *multiple independent* dimensions with no dominant one, which is not the case here (date is always in the predicate, and instrument is the second key). **Don't use Z-order on OHLCV.** It is genuinely useful on the options chain table if you frequently filter by strike *without* underlying, which is rare.

### 4.4 Realistic volumes

Planning figures at 30 bytes/row compressed:

| Dataset | Rows/day | Compressed/day | 10 years |
|---|---|---|---|
| 2,000 equities/ETFs, 1m, RTH+ext (960 min) | 1.92 M | ~58 MB | **~150 GB** |
| 2,000 crypto instruments across venues, 1m, 1440 min | 2.88 M | ~86 MB | **~315 GB** |
| 1,000 futures contracts (≈200 active roots × 5 live months), 1m, 1380 min | 1.38 M | ~41 MB | **~150 GB** |
| **Subtotal: 5,000 instruments, 10 y** | | | **≈ 600 GB** |
| Quote/NBBO at 1m (bid/ask/sizes) for the same | | ~1.5× | **+300 GB** |
| DeFi: 5,000 pools, block-native, ~7,200 blocks/day ETH-equiv | ~10 M | ~400 MB | **~1.4 TB** |
| **Options, gated liquid universe** (~80k contracts, 390 min, sparse ~30% non-empty) | ~9.4 M | ~350 MB | **~1.3 TB** |
| **Options, full chain** (1.6 M contracts, 390 min, sparse ~8%) | ~50 M | ~1.9 GB | **~7 TB** |
| Vol surfaces (500 underlyings × 15 expiries × 390 min) | 2.9 M | ~90 MB | **~330 GB** |
| Bitemporal overhead (restatement log, ~3% of rows) | | | **+5%** |

**Headline: the non-options platform is ~1 TB and essentially free. Options are 5–10× everything else combined, and the liquid-universe gate is worth ~5.4 TB over ten years.** Budget ~3–4 TB total for a gated-options build, ~10 TB for full-chain.

Query latency expectations (DuckDB on a 16-core box, local NVMe or warm S3 cache):
- Single instrument, 10 y of minute bars (~3.5 M rows): **50–200 ms**.
- 500-instrument universe, 1 year (~180 M rows), 5 columns: **2–8 s** cold from S3, **0.5–2 s** warm.
- Full options chain, single underlying, single day: **100–400 ms** with the sort order above; **10–30 s** without it.
- PIT-resolved read with the restatement-index optimization (§1.2): **within 20%** of the non-PIT read. Without it: **5–20×** slower.

---

## 5. Query / compute engines (2026)

### 5.1 Benchmark evidence, honestly read

**ClickBench (2026, c6a.4xlarge self-hosted):** ClickHouse median **148 ms**, 32/43 wins, zero failures; DuckDB median **348 ms**, 10/43 wins, zero failures and a *better* tail (6.5 s vs 9.6 s); Trino-on-Parquet median **2.72 s**, 0 wins, 36.2 s tail; Druid and Pinot both post multiple query failures. Cloud: ClickHouse Cloud **109 ms** (38/43 wins), Snowflake 16×XL **366 ms**, Redshift **465 ms**, BigQuery **544 ms**, Databricks 16×L **546 ms**.

**TSBS (KX, 2026):** KDB-X beats QuestDB by 3.36×, TimescaleDB by 25.5×, InfluxDB by 53.1×, ClickHouse by 161.3× on geometric-mean query latency. **Read this with heavy skepticism**: it is a vendor benchmark, and although KX handicapped KDB-X to 4 threads and 16 GB, TSBS query shapes (`last-point`, `groupby-orderby-limit` over device time series) are precisely kdb+'s home turf and precisely ClickHouse's worst case. The honest conclusion is narrower: **for last-value and as-of-style queries over ordered time series, kdb+ remains in a class of its own; for wide analytical scans, ClickHouse wins.** Both are true.

**DataFusion 55.0.0** (2026-08-25, 877 commits / 175 contributors) landed changes directly relevant here: runtime row-group pruning (**4.2× on TopK TPC-H Q8**), Parquet pruning that cut a production query from **1.35 TB to 30.9 GB** scanned, `approx_distinct` 101× faster with many groups, native **range partitioning** (explicitly motivated by time-series data written daily/hourly), SQL:2003 `MERGE INTO` with Iceberg/Delta hooks, and virtual columns `file_row_index()` / `input_file_name()` — the latter two are quietly perfect for provenance tracking in a PIT system.

**Polars 2.0 RC** (2026-09-02) makes the streaming engine the default LazyFrame engine, "easily 5× faster" with much lower memory.

### 5.2 Recommendations by workload

**(a) Backtest data serving — DuckDB 1.5, embedded, one process per backtest worker.**

Rationale: backtests are embarrassingly parallel and each needs a *private, consistent, pinned* view of data. An embedded engine reading Iceberg/Parquet from object storage with a local NVMe cache gives perfect isolation (no noisy neighbours by construction), zero per-tenant cluster provisioning, and native `ASOF JOIN` with the best-in-class implementation. Cost scales linearly with compute you were going to spend on the backtest anyway. DuckDB 1.4 is the LTS line if you want stability over features.

Anti-recommendation: do **not** serve backtests from a shared ClickHouse/Trino cluster. One tenant's 10-year full-universe scan will wreck everyone else's latency, and quota systems mitigate but don't eliminate this.

**(b) Feature computation at scale — Polars 2.0 for per-instrument work; DataFusion 55 or Spark for cross-sectional shuffles; Chronon-style orchestration on top.**

Most minute-bar features are *within-instrument* time-series transforms (rolling stats, EWMAs, microstructure aggregates). These embarrassingly parallelize by instrument and never need a shuffle: Polars with the streaming engine, one instrument-bucket per task, is dramatically simpler and cheaper than Spark. Reserve a shuffle engine for genuinely cross-sectional features (ranks, industry-neutralization, PCA/factor exposures) — that's where Spark or DataFusion Ballista earns its keep. **Do not start with Spark.** The cluster-management tax is real and most teams never need it for 3 TB.

**ArcticDB 6.18.2** (2026-06-17) deserves specific mention: serverless (S3 + a Python process, no server), immutable versioned symbols with `as_of` retrieval by version number *or* datetime, snapshots, static/dynamic schemas, millions of symbols per library, `QueryBuilder` pushdown. It is purpose-built by a quant shop (Man Group) for exactly this shape of problem and gives you version-level time travel for free. **Strong candidate for the per-tenant feature/research store** — the thing you want is "give me my feature frame as it was on date D," which is its native operation. Weaker as the shared market-data plane because it's Python-centric and doesn't interop with Trino/ClickHouse.

**(c) Ad-hoc analytics over experiment results — ClickHouse 26.8 LTS.**

Experiment metadata, backtest P&L series, per-trade records, feature-importance tables, consistency metrics: high row counts, wide aggregations, many concurrent users, sub-second expectations. ClickHouse is the clear pick — 148 ms ClickBench median, mature RBAC with **row policies**, and a **keyed quota system** (`CREATE QUOTA ... KEYED BY ... FOR INTERVAL 1 hour MAX queries = N`) that maps cleanly onto tenants. It also reads Iceberg directly (26.8 added Snowflake Horizon catalog access and Iceberg writes, plus native Puffin support), so it can query the market-data plane without a second copy.

**(d) Federation / SQL over everything — Trino 483, optional.**

Useful if you must join Iceberg + Postgres + ClickHouse in one query. Its ClickBench numbers (2.72 s median, 36.2 s tail) mean it should never be the hot path. Note: **Iceberg v3 support is experimental in Trino 483** (column defaults and encryption unsupported); Starburst Enterprise 476-e ships v3 with row-lineage metadata columns.

### 5.3 Cost/complexity honesty table

| Engine | $ | Ops burden | Where it shines | Where it hurts |
|---|---|---|---|---|
| **DuckDB 1.5** | ~free (compute only) | Near-zero | Backtests, single-node feature work, ASOF | No concurrency story; memory-bound on huge shuffles |
| **DuckLake 1.0** | ~free + a Postgres | Low | Fast metadata, inlined small writes, per-tenant tables | Catalog DB is a SPOF; DuckDB-centric ecosystem |
| **ClickHouse 26.8** | Moderate (nodes or Cloud) | Medium | Shared analytics, concurrency, quotas, RLS | Expensive upserts; ASOF `hash` algorithm OOMs on big RHS |
| **Polars 2.0** | ~free | Near-zero | Per-instrument feature compute | No shuffle; asof pushdown gaps; 2.0 has breaking changes |
| **DataFusion 55** | ~free | Medium (you build on it) | Custom engine/embedding, Parquet pruning | It's a library, not a product |
| **Trino 483** | Moderate–high | High | Federation, SQL-over-everything | Slow; Iceberg v3 experimental |
| **Spark / Databricks** | High | High | Huge shuffles, Delta + liquid clustering, ML platform | Overkill under ~50 TB; ClickBench 546 ms at 16×L |
| **ArcticDB 6.18** | ~free (S3) | Low | Versioned research frames, PIT `as_of` | Python-only; not a shared SQL plane |
| **QuestDB** | Moderate | Medium | Extreme ingest throughput | **Self-serve cloud discontinued in 2026 — enterprise BYOC only.** Re-evaluate before adopting |
| **TimescaleDB / Tiger Cloud** | Moderate | Low | Postgres compatibility, Hypercore ~98% compression, continuous aggregates | Not built for 100 B-row scans |
| **kdb+ / KDB-X** | **Very high** | Very high | Unmatched as-of and last-value performance; 30 years of tick-store practice | License cost, scarce talent, ecosystem isolation. KDB-X Community Edition (v0.1.0, June 2025) is capped at 4 threads / 16 GB |

**Verdict:** DuckDB + Iceberg + ClickHouse covers 95% of this platform at a small fraction of kdb+ cost. Adopt kdb+ only if you are doing sub-millisecond tick research where `aj` over billions of rows is the inner loop of the business.

---

## 6. Feature computation architecture

### 6.1 The one rule

> **One feature definition, one code path, two execution modes. If backfill and streaming are separate implementations, they will diverge, and you will not find out until a model underperforms in production by an amount you can't explain.**

This is the Chronon thesis and it is correct: Airbnb built it because "training-serving skew led to hard-to-debug model degradation, and worse than expected model performance." Chronon's mechanism is a declarative feature definition compiled into both a batch (Spark) and a streaming (Spark Streaming/Flink + KV store) execution, with **partial aggregates** maintained internally and combined to produce features at arbitrary points in time — which is precisely how you get PIT-correct backfills over long windows without recomputing from scratch. Its `Accuracy` parameter (`Temporal` vs `Snapshot`) governs refresh semantics **identically** in online serving and offline backfill, which is what makes parity structural rather than aspirational.

### 6.2 Consistency measurement is not optional

Build this on day one:

```sql
CREATE TABLE feat.online_offline_consistency (
  feature_id       TEXT NOT NULL,
  tenant_id        UUID NOT NULL,
  instrument_id    BIGINT NOT NULL,
  event_time       TIMESTAMPTZ NOT NULL,
  online_value     DOUBLE,                      -- logged at serving time
  offline_value    DOUBLE,                      -- recomputed by backfill for the same instant
  abs_diff         DOUBLE,
  rel_diff         DOUBLE,
  online_knowledge_time  TIMESTAMPTZ,           -- what the server actually had
  offline_knowledge_time TIMESTAMPTZ,           -- what the backfill assumed
  measured_at      TIMESTAMPTZ NOT NULL
);
```

Mechanism: **log every feature vector served in production** (they're small), then nightly recompute the same vectors via the offline path and diff. Alert on distributional drift in `rel_diff`, not just on the mean. The `knowledge_time` columns are what turn "the numbers differ" into "the numbers differ *because the backfill assumed data that arrived 40 minutes late*" — which is the actionable version.

In a trading context this doubles as a **compliance artifact**: it is direct evidence that the model that traded real capital saw the inputs you claim it saw.

### 6.3 Incremental / materialized computation

At minute granularity, recomputing a 252-day rolling feature from scratch every minute is ~360,000× more work than necessary. Structure:

- **Decompose into associative partial aggregates** wherever possible (sum, count, sumsq → mean/var/skew; min/max via monotonic deques; EWMA is natively incremental). Non-decomposable features (median, quantiles, rank) need sketches (t-digest, KLL) or acceptance of full recompute.
- **Tiered materialization:** minute-level raw → 5-min/hourly/daily partial aggregates → feature values. A 252-day window then reads 252 daily partials + today's intraday partials, not 100k minute bars.
- **Watermark-driven invalidation:** when a restatement lands for `(instrument, event_date)`, mark every downstream partial and feature whose window covers that date as dirty. This is a dependency-graph problem; model it explicitly with a `feature_dependency` table rather than recomputing everything.

### 6.4 Content addressing

Every feature matrix produced must be addressable by a hash of its *complete determinants*:

```
feature_matrix_id = SHA256(
    canonical_json({
      "feature_defs":      [ {id, definition_hash, version}, ... ],  # code
      "universe_hash":     <hash of the resolved instrument_id list>,
      "date_range":        [start, end],
      "grid":              "utc_1m",
      "knowledge_time":    "2026-03-01T00:00:00Z",                   # PIT cutoff
      "source_snapshots":  { "mkt.bars_1m_equity": 3821094...,       # Iceberg snapshot IDs
                             "ref.corporate_action": 991221...,
                             "ref.session_calendar": 44120... },
      "calendar_pkg":      "exchange_calendars==4.13.2",
      "code_commit":       "a3f9c1e...",
      "engine_versions":   { "duckdb": "1.5.0", "polars": "2.0.0" }
    })
)
```

Two properties this buys you:
1. **Cache hit = provable identity.** If the hash matches, the matrix is bit-identical; skip recomputation. At minute granularity across many experiments, hit rates of 60–90% are normal and this is the single largest compute saving available.
2. **Reproducibility is mechanical.** Given the hash, you can reconstruct the exact inputs — including which Iceberg snapshot of each source table, which is why §8's tag discipline matters.

MLflow's `mlflow.data` module implements a weaker version of this (dataset name + auto-computed digest + `DatasetSource` lineage), which is worth wiring in for the ML-experiment half. But MLflow's digest hashes the *materialized data*, not the *determinants* — so it detects divergence after the fact rather than enabling cache hits before. Use both: determinant hash for caching, MLflow digest for verification.

### 6.5 Changing feature definitions

Feature definitions **must be immutable and versioned**, never edited in place:

```sql
CREATE TABLE feat.definition (
  feature_id       TEXT NOT NULL,               -- 'rv_20d'
  version          INT  NOT NULL,
  definition_hash  CHAR(64) NOT NULL,           -- content address of the spec + code
  spec             JSON NOT NULL,               -- declarative definition
  code_ref         TEXT NOT NULL,               -- git sha + path
  asset_classes    TEXT[] NOT NULL,
  dependencies     TEXT[] NOT NULL,             -- upstream feature_ids
  created_at       TIMESTAMPTZ NOT NULL,
  created_by       TEXT NOT NULL,
  deprecated_at    TIMESTAMPTZ,
  PRIMARY KEY (feature_id, version)
);
```

Rules: a semantic change is a **new version**, never an edit. Old versions are never deleted (a live model may depend on one). Models record `(feature_id, version)` pairs, not bare names. A "bug fix" to a feature is a new version plus a written note — and any model trained on the old version must be explicitly re-validated, not silently migrated.

---

## 7. Multi-tenancy

### 7.1 The shape of the problem

Two data populations with opposite requirements:
- **Market data** — identical for all tenants, enormous, read-only, expensive to store. Must be **shared** or your unit economics die.
- **Research artifacts** — features, models, backtests, signals, positions. Small per tenant, and a tenant's alpha is the *only* thing they're paying you to protect. A leak here is existential.

### 7.2 Recommended isolation model

```
s3://platform-market-data/          <- shared, read-only to all tenant roles
    iceberg/mkt.bars_1m_equity/...
    iceberg/ref.corporate_action/...

s3://platform-tenants/
    tenant=<uuid>/                  <- IAM session policy scoped to this prefix ONLY
        iceberg/feat.matrix/...
        iceberg/exp.backtest_result/...
        artifacts/<sha256>/...      <- content-addressed model/feature blobs

s3://platform-ledger/               <- Object Lock COMPLIANCE mode, no delete path
    tenant=<uuid>/ledger/...
```

**Object storage.** AWS's published patterns for multi-tenant S3 are: bucket-per-tenant (strong isolation, but ~10k bucket soft limit and high management overhead), **prefix-based isolation with IAM session policies** (the workhorse — but bucket policies cap at **20 KB**, so never enumerate tenants in a bucket policy; use `AssumeRole` with a dynamically-generated session policy, the "token vending machine" pattern), **S3 Access Points** (scales to tens of thousands of tenants with static config), and **S3 Access Grants** (dynamic, IdP-integrated, vends scoped STS credentials — the most flexible and the right answer above a few hundred tenants).

**Recommendation: token-vending-machine with `AssumeRole` + session policy for < 500 tenants; migrate to S3 Access Grants beyond that.** Critically: the credential a tenant's compute receives must *never* be able to name another tenant's prefix, so the scoping must happen at credential-vending time, not in application code.

**Relational metadata (Postgres).** Use **row-level security**, but only with the full discipline, because the failure modes are silent:

```sql
ALTER TABLE experiment ENABLE ROW LEVEL SECURITY;
ALTER TABLE experiment FORCE ROW LEVEL SECURITY;     -- <-- without this, the table OWNER bypasses RLS
CREATE POLICY tenant_isolation ON experiment
  USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- The application connects as a NON-OWNER role without BYPASSRLS:
CREATE ROLE app_runtime NOLOGIN NOBYPASSRLS;
```

The four pitfalls that actually bite, all confirmed in practice:
1. **Owner bypass.** The table owner and superusers bypass RLS unless `FORCE ROW LEVEL SECURITY` is set. Most teams enable RLS, connect as the owner, and ship a policy that does nothing.
2. **`SET` vs `SET LOCAL`.** With connection pooling you must use `SET LOCAL app.tenant_id = '...'` **inside a transaction**. A bare `SET` persists for the life of the pooled connection and bleeds tenant context across requests. This is the catastrophic one.
3. **Policies OR together.** Multiple policies on the same command combine with `OR`, not `AND`. Adding a policy *widens* access. This is the opposite of how everyone reasons about authorization.
4. **`SECURITY DEFINER` functions** bypass caller policies — the most common accidental cross-tenant path. Also: non-`STABLE` functions in a policy force sequential scans; mark helpers `STABLE` and keep `tenant_id` as the leading index column, then verify with `EXPLAIN ANALYZE`.

Also: `pg_dump` under `FORCE ROW LEVEL SECURITY` will export **zero rows** unless the dumping role has proper context or `BYPASSRLS`. Teams discover this during their first restore drill.

**When to escalate beyond RLS:** separate **schemas** per tenant when tenants need custom tables or per-tenant migrations; separate **databases/clusters** when a tenant contractually requires physical isolation or their own encryption keys. For a quant platform, expect your largest tenants to demand exactly this — **design the tenant-resolution layer so the physical location of a tenant's data is a lookup, not a hardcoded assumption.** Retrofitting "some tenants live in a different cluster" into code that assumes one connection string is a multi-quarter project.

**Lakehouse isolation.** Per-tenant Iceberg **namespaces** with catalog-level RBAC, plus the S3 prefix scoping above. Do *not* rely on catalog permissions alone: if a tenant can read the raw S3 prefix, catalog ACLs are decoration. Belt and braces.

### 7.3 Noisy neighbours on the compute plane

- **Backtests:** per-tenant Kubernetes namespaces with `ResourceQuota` + `LimitRange`; each backtest an isolated pod with its own DuckDB. Isolation by construction — this is the main argument for the embedded-engine choice in §5.
- **Shared ClickHouse:** `CREATE QUOTA ... KEYED BY <tenant> FOR INTERVAL 1 hour MAX queries=N, result_rows=..., read_rows=..., execution_time=...` plus per-user `max_memory_usage`, `max_execution_time`, `max_threads`, `max_concurrent_queries_for_user`. Combine with workload scheduling to reserve capacity. Note quotas reset on server restart and accumulate on the initiating server for distributed queries.
- **Object storage:** S3 request rates are per-prefix; tenant prefixes give natural request-rate isolation. Watch GET costs — a runaway backtest loop is a *billing* incident before it is a performance incident. Per-tenant cost attribution via S3 request metrics and Iceberg scan metrics should be built in from the start.

### 7.4 Shared market data, private research — and cross-tenant meta-learning

The uncomfortable question: can you learn from tenant A's research to improve tenant B's outcomes?

**Defensible (do these):**
- Aggregate **platform-level operational** statistics: which feature families are computed most, typical hyperparameter ranges, compute/cost profiles, data-quality incidents. These are about *your platform*, not their alpha.
- **Market-data quality signals** derived from many tenants hitting the same shared data (e.g. "this vendor's 2024-06 bars for instrument X look wrong") — this is shared-plane information, not tenant IP.
- **Infrastructure meta-learning**: query-plan optimization, cache pre-warming, autoscaling — no strategy content involved.

**Not defensible (don't):**
- Training any model on tenant feature sets, signals, positions, or backtest results to improve another tenant's models. Even with differential privacy. The published FL+DP literature is clear that meaningful utility requires ε in ranges that provide weak formal guarantees, and — more importantly — **a quant client will not accept "we applied DP noise to your alpha" as an answer.** The reputational asymmetry is total: the upside is a marginally better model, the downside is the end of the business.

**Contract and architecture must agree.** Put "we do not train on tenant research data" in the MSA, then make it *architecturally true*: the shared-plane compute role must have **no read path** to tenant prefixes at all. If an engineer *could* join across tenants, you will eventually have to prove they didn't, and logs are a much weaker proof than "the credential cannot name the bucket."

**The one legitimate gray area** is a tenant-opt-in consortium model (pooled signal research with explicit contracts and revenue share). If you want to offer this, build it as a **separate tenant** with explicit data-contribution grants — never as a platform-level capability that silently reads across the boundary.

---

## 8. Immutability and fixation

### 8.1 Layered scheme

```
Layer 4  Regulatory WORM   — S3 Object Lock COMPLIANCE mode on the ledger bucket
Layer 3  Hash chain        — append-only Postgres ledger; each row commits to the previous
Layer 2  Iceberg tags      — named, expiry-proof snapshot pins per experiment
Layer 1  Content address   — SHA-256 of every artifact; path IS the hash
```

### 8.2 Content-addressed artifacts

```
s3://platform-tenants/tenant=<uuid>/artifacts/sha256/<first2>/<next2>/<full-hash>
```

Every model binary, feature matrix, config, and backtest result is stored at its own hash. Properties: deduplication is automatic (identical configs across experiments cost storage once); tampering is detectable by recomputation; references are never ambiguous. Fan out the first four hex chars into subdirectories to avoid prefix hot-spotting.

### 8.3 Hash-chained experiment ledger

```sql
CREATE TABLE ledger.entry (
  seq              BIGSERIAL PRIMARY KEY,
  tenant_id        UUID        NOT NULL,
  entry_type       TEXT        NOT NULL,        -- 'experiment'|'backtest'|'model_train'|'deploy'|'order'|'fill'|'restatement'
  entry_time       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
  payload          JSONB       NOT NULL,        -- canonical JSON, key-sorted
  payload_hash     CHAR(64)    NOT NULL,        -- SHA256(canonical_json(payload))
  prev_hash        CHAR(64)    NOT NULL,        -- entry_hash of seq-1 for this tenant
  entry_hash       CHAR(64)    NOT NULL,        -- SHA256(prev_hash || payload_hash || seq || entry_time)
  actor            TEXT        NOT NULL,
  signature        BYTEA                        -- optional: sign with an HSM/KMS key
);
REVOKE UPDATE, DELETE ON ledger.entry FROM PUBLIC;   -- append-only by grant
CREATE RULE no_update AS ON UPDATE TO ledger.entry DO INSTEAD NOTHING;
CREATE RULE no_delete AS ON DELETE TO ledger.entry DO INSTEAD NOTHING;

-- Periodic anchoring: Merkle root over a window, written to WORM storage
CREATE TABLE ledger.anchor (
  anchor_id     BIGSERIAL PRIMARY KEY,
  tenant_id     UUID NOT NULL,
  from_seq      BIGINT NOT NULL,
  to_seq        BIGINT NOT NULL,
  merkle_root   CHAR(64) NOT NULL,
  anchored_at   TIMESTAMPTZ NOT NULL,
  worm_object   TEXT NOT NULL,                  -- s3://platform-ledger/... with Object Lock
  kms_signature BYTEA
);
```

A Merkle tree over each window gives **O(log n) inclusion proofs**: you can prove a single experiment record was in the ledger at anchor time without exposing the rest of the tenant's research. That property is exactly what auditors and counterparties want, and it is why a Merkle root beats a plain running hash.

### 8.4 Iceberg tags for reproducibility pinning

```sql
-- At experiment launch, pin every source table:
ALTER TABLE mkt.bars_1m_equity
  CREATE TAG `exp_7f3a9c21` AS OF VERSION 3821094 RETAIN 3650 DAYS;
```

Then record the tag names and snapshot IDs in the ledger payload. To reproduce: `SELECT * FROM mkt.bars_1m_equity FOR VERSION AS OF 'exp_7f3a9c21'`.

**The snapshot-expiry hazard — this is the one that silently destroys reproducibility.** Iceberg's defaults:

| Property | Default | What you must set |
|---|---|---|
| `history.expire.max-snapshot-age-ms` | **432,000,000 (5 days)** | ≥ 90 days |
| `history.expire.min-snapshots-to-keep` | **1** | ≥ 50 |
| `history.expire.max-ref-age-ms` | `Long.MAX_VALUE` | leave (main never expires) |
| `gc.enabled` | `true` | leave |

**Five days.** If your routine maintenance runs `expire_snapshots` with defaults, every experiment older than five days becomes irreproducible and you will not get an error — the tag-less snapshots are simply gone, and `FOR VERSION AS OF` starts failing. Five things protect a snapshot: it's current, it's newer than the cutoff, it's within `retain_last`, **it's referenced by a branch or tag**, or a retained snapshot still needs it. Tags are the mechanism that survives maintenance. Expiry is **irreversible** — recovery requires backups.

Secondary hazard: a long-running query that started before expiry and reads files after deletion will fail mid-flight. Schedule expiry in a maintenance window.

### 8.5 WORM and what regulated firms actually do

S3 Object Lock has two modes and only one is immutability: in **Governance** mode, a principal with `s3:BypassGovernanceRetention` can delete — this is a safety rail, not a compliance control. **Compliance** mode means *no one*, including the root account, can delete or shorten retention until it expires.

For the regulatory frame: the SEC's amendments to Rule 17a-4 (adopted 2022, compliance 2023) added an **audit-trail alternative** to the traditional WORM requirement for the first time in decades. The audit-trail path permits rewriteable media provided you capture comprehensive, tamper-evident logs — access (who, when, what), modification (nature, timestamp, actor), deletion (what, justification), and system events. AWS's own compliance position is that S3 Object Lock, S3 Glacier Vault Lock, FSx for NetApp ONTAP SnapLock, and AWS Backup Vault Lock support the non-erasable/non-rewritable requirement when properly configured; broker-dealers must supply AWS their registrant name and number so AWS files a Letter of Undertaking with the SEC.

**This is architecturally important:** the audit-trail alternative is *exactly* the hash-chained bitemporal ledger described above. A firm that builds proper bitemporal append-only storage with cryptographic chaining is **already compliant under the audit-trail path** and doesn't need to shove analytical data into WORM — which matters because WORM storage is operationally hostile (you cannot compact, re-partition, migrate, or re-encode it).

**Practical split:**
- **Analytical data (bars, features):** normal Iceberg, mutable at the file level, immutable at the semantic level via bitemporality. No Object Lock — you need to compact.
- **The ledger and its Merkle anchors:** S3 Object Lock **Compliance** mode with retention matching your regulatory horizon (typically 6 years for 17a-4, 7 for many jurisdictions).
- **Model binaries that traded real capital:** Object Lock Compliance. They are small; there is no reason not to.

Note that Cloudflare R2 and Backblaze B2 both support object lock/immutability, so WORM does not force you onto AWS.

### 8.6 Bitemporal supersession of corrected records

When a vendor restates, the ledger gets its own entry:

```json
{
  "entry_type": "restatement",
  "table": "mkt.bars_1m_equity",
  "instrument_id": 40312,
  "event_date": "2026-02-14",
  "rows_affected": 391,
  "prior_revision": 0,
  "new_revision": 1,
  "reason": "vendor CA-2026-0221: erroneous prints from XBOS feed",
  "source_id": 12,
  "detected_at": "2026-02-21T14:03:11Z",
  "new_snapshot_id": 3821577,
  "affected_experiments": ["exp_7f3a9c21", "exp_9a1b0e44"]
}
```

The `affected_experiments` field is the payoff: because experiments pinned snapshots via tags, you can mechanically compute which past results used data that has since been corrected. **Every regulated firm gets asked this question eventually**, and without pinned snapshots the answer is a multi-week forensic exercise.

---

## 9. Cost

### 9.1 Storage (2026 list prices, us-east-1 / equivalent)

| Class | $/GB-mo | $/TB-mo |
|---|---|---|
| S3 Standard | 0.023 | **$23.00** |
| S3 Standard-IA | 0.0125 | $12.50 |
| S3 One Zone-IA | 0.010 | $10.00 |
| S3 Glacier Instant Retrieval | 0.004 | $4.00 |
| S3 Glacier Flexible | 0.0036 | $3.60 |
| S3 Glacier Deep Archive | 0.00099 | $0.99 |
| Cloudflare R2 Standard | 0.015 | $15.00 |
| Cloudflare R2 Infrequent Access | 0.010 | $10.00 |
| Backblaze B2 | 0.00695 | **$6.95** |

Requests: S3 PUT $0.005/1k, GET $0.0004/1k. R2 Class A (write) $4.50/M, Class B (read) $0.36/M. B2 reads/writes free (Class D metadata $0.004/10k).
Egress: S3 $0.09/GB to 10 TB, $0.085 to 50 TB, $0.07 above; **R2 $0.00**; B2 free to 3× stored bytes then $0.01/GB.

### 9.2 Worked example — 5,000 instruments + options, 10 years

Using §4.4 volumes, gated-options build ≈ **3.5 TB**, with three copies of concern (raw landing, curated, derived features ≈ 1.5× curated):

| Component | Size | Tier | $/mo (S3) | $/mo (R2) |
|---|---|---|---|---|
| Curated bars (equity/crypto/futures, 10 y) | 0.9 TB | Standard | $21 | $14 |
| Options gated liquid universe | 1.3 TB | Standard (2 y) / Glacier IR (8 y) | $6 + $4 | $4 + $13 |
| Vol surfaces | 0.33 TB | Standard | $8 | $5 |
| DeFi pool state | 1.4 TB | Standard (1 y) / IA (9 y) | $3 + $16 | $2 + $13 |
| Raw landing (compressed vendor originals) | 2.5 TB | Glacier Deep Archive | $2 | n/a → B2 $17 |
| Feature matrices (content-addressed, deduped) | 2.0 TB | Standard | $46 | $30 |
| Experiment/model artifacts | 0.2 TB | Standard + Object Lock | $5 | $3 |
| **Storage subtotal** | ~8.6 TB | | **≈ $111/mo** | **≈ $101/mo** |

**Storage is not the bill. It is roughly $1,300/year.** What actually dominates:

| Cost driver | Realistic monthly | Notes |
|---|---|---|
| **Market data licences** | **$3k – $60k+** | OPRA alone can exceed everything else combined; per-tenant redistribution licensing is a legal and financial landmine. **This is the #1 line item.** |
| **Backtest compute** | $2k – $30k | Linear in tenants × experiments. Spot instances and content-addressed caching are the two levers that matter. |
| **Feature backfill compute** | $1k – $10k | Spiky: a feature-definition change can trigger a 10-year recompute. Incremental materialization (§6.3) is what keeps this bounded. |
| **Shared ClickHouse cluster** | $1k – $8k | Scales with concurrent researchers, not data volume. |
| **S3 GET requests** | $50 – $2,000 | A poorly-partitioned table turning one query into 500k GETs is a real and common failure. Watch it. |
| **Egress** | $0 – $5,000 | Zero if compute is co-located. **If tenants pull data out, use R2 and pay nothing.** |
| **Storage** | ~$110 | Noise. |

**Conclusions on cost:**
1. **Do not optimize storage. Optimize licences, compute, and request counts.** Teams spend months on compression for $40/month while a single unnecessary full-universe backfill costs $3,000.
2. **Full options chains at minute granularity are the only genuine storage decision** — the gate is worth ~5.4 TB and, more importantly, ~4× the scan cost on every options query forever.
3. **Egress is the cloud-lock-in tax.** R2's zero-egress is worth real money the moment tenants want their own data, and R2 supports object lock, so it doesn't compromise §8.
4. **Tier, but only cold data, and only after two years.** Glacier retrieval fees can dwarf the storage savings — the "$1/TB to store, $20k to retrieve" failure mode is real. Use Glacier Instant Retrieval (not Flexible/Deep) for anything a backtest might touch.

---

## 10. Recommended physical layout (consolidated)

```
CATALOG:  Iceberg REST catalog (Nessie/Polaris/Glue) + Postgres for reference & ledger

s3://platform-market-data/iceberg/
  mkt.bars_1m_equity/     part: days(event_date), bucket(instrument_id,16)
                          sort: instrument_id, event_time         256 MB / 16 MB RG
  mkt.bars_1m_crypto/     part: months(event_date), venue_id
                          sort: instrument_id, event_time         256 MB
  mkt.bars_1m_future/     part: months(event_date), root_symbol
                          sort: instrument_id, event_time         256 MB
  mkt.bars_1m_option/     part: days(event_date), bucket(underlying_id,64)
                          sort: underlying_id, expiry_date, option_type, strike, event_time
                                                                  512 MB
  mkt.vol_surface_1m/     part: months(event_date), bucket(underlying_id,8)
  mkt.perp_funding/       part: months(funding_time)
  defi.pool_state/        part: chain_id, months(block_time), bucket(pool_id,16)
  mkt.continuous_1m/      part: months(event_date), root_symbol, method_id   [DERIVED]

POSTGRES (bitemporal reference + control plane):
  ref.instrument, ref.instrument_symbol, ref.corporate_action,
  ref.index_membership, ref.futures_contract, ref.futures_roll_schedule,
  ref.continuous_method, ref.option_contract, ref.session_calendar,
  ref.venue_quality, ref.source
  ledger.entry (hash-chained, append-only), ledger.anchor
  feat.definition, feat.online_offline_consistency
  tenant.*  (RLS: FORCE ROW LEVEL SECURITY, non-owner app role)

s3://platform-tenants/tenant=<uuid>/
  iceberg/feat.matrix/        part: months(event_date)     [or DuckLake / ArcticDB]
  iceberg/exp.backtest_result/
  artifacts/sha256/<aa>/<bb>/<hash>

s3://platform-ledger/tenant=<uuid>/   [S3 Object Lock, COMPLIANCE mode, 7 y]
  anchors/<date>/<merkle_root>.json
  models/<sha256>                      (models that traded real capital)

ENGINES:
  Backtest serving .......... DuckDB 1.5 embedded, 1 process / worker, NVMe cache
  Per-instrument features ... Polars 2.0 (streaming), bucketed tasks
  Cross-sectional features .. DataFusion 55 / Spark (only when a shuffle is unavoidable)
  Research frames (tenant) .. ArcticDB 6.18 (versioned as_of) or DuckLake 1.0
  Shared ad-hoc analytics ... ClickHouse 26.8 LTS (quotas KEYED BY tenant, row policies)
  Federation (optional) ..... Trino 483 / Starburst 476-e (v3 support)
  Reference/bitemporal ...... Postgres 17 (or XTDB v2 if you want SQL:2011 natively)
```

---

## 11. The four things you cannot retrofit

**1. Bitemporality.** If you ingest without recording `knowledge_time`, that information is destroyed at the moment of ingestion and is unrecoverable — you cannot reconstruct when you learned something after the fact. Every day you run without it is a permanent hole in your PIT history. **This is the single highest-priority decision in the document.** Adding a `knowledge_time` column later gives you correct data from that date forward and a fabrication for everything before.

**2. Unadjusted prices + separate adjustment factors, and surrogate instrument IDs.** If you store adjusted prices, you cannot recover the unadjusted series (the factor is a function of the full future action stream, and you didn't keep the actions). If you key on tickers, ticker reuse and symbol changes have already merged and split entities in ways you cannot untangle without buying historical reference data and reprocessing everything. Both are one-way doors.

**3. A single feature code path with consistency measurement.** Once backfill and streaming are two codebases, they diverge immediately and permanently. Every subsequent feature is written twice, the divergence compounds, and "unify the feature pipelines" becomes a project that is always next quarter. The Chronon insight is not that unification is nice — it is that *it is only achievable before the second implementation exists*. Corollary: start logging served feature vectors on day one; you cannot measure consistency retroactively against logs you never wrote.

**4. Fixation: content addressing, hash-chained ledger, and Iceberg tags on every experiment.** You cannot prove what data an experiment used if you didn't pin it, and you cannot pin it after `expire_snapshots` ran with the **5-day default**. The specific, concrete, easy-to-miss failure: ship with Iceberg defaults, run routine maintenance, and every experiment older than five days silently becomes irreproducible. Set `history.expire.min-snapshots-to-keep >= 50` and `max-snapshot-age-ms >= 90 days` **before** the first experiment runs, and tag at experiment launch.

**Near-miss (expensive but survivable):** table format choice (Iceberg ↔ Delta ↔ DuckLake) is a rewrite, not a data loss; partitioning and sort orders can be evolved (Iceberg partition evolution) or rewritten; engine choices are swappable because the data is open Parquet. Tenant *physical* placement is retrofittable only if you build tenant resolution as a lookup from the start — hardcode one connection string and it becomes a multi-quarter refactor.

---

## 12. Sources

**Bitemporality, PIT, as-of joins**
- https://duckdb.org/2023/09/15/asof-joins-fuzzy-temporal-lookups
- https://duckdb.org/docs/current/guides/sql_features/asof_join
- https://clickhouse.com/docs/sql-reference/statements/select/join
- https://code.kx.com/q/ref/aj/
- https://docs.pola.rs/api/python/stable/reference/lazyframe/api/polars.LazyFrame.join_asof.html
- https://github.com/pola-rs/polars/issues/25867
- https://docs.xtdb.com/about/time-in-xtdb.html
- https://xtdb.com/blog/launching-xtdb-v2
- https://www.ivp.in/resources/blogs/bitemporal-point-in-time-reference-data-management/
- https://kx.com/blog/why-ai-in-capital-markets-needs-temporal-precision/
- https://saral.money/blog/point-in-time-data-lookahead-bias/

**Table formats**
- https://iceberg.apache.org/spec/
- https://opensource.googleblog.com/2025/08/whats-new-in-iceberg-v3.html
- https://www.databricks.com/blog/apache-icebergtm-v3-moving-ecosystem-towards-unification
- https://www.dremio.com/blog/apache-iceberg-v2-vs-v3-what-changed-and-what-it-means-for-your-tables/
- https://www.starburst.io/blog/iceberg-v3/
- https://github.com/apache/iceberg/blob/main/docs/docs/branching.md
- https://www.dremio.com/blog/apache-iceberg-snapshot-expiration/
- https://www.starburst.io/blog/iceberg-partitioning/
- https://www.dremio.com/blog/minimizing-iceberg-table-management-with-smart-writing/
- https://ducklake.select/2026/04/13/ducklake-10/
- https://ducklake.select/
- https://delta.io/blog/delta-lake-3-2/
- https://docs.delta.io/delta-deletion-vectors/

**Storage formats & compression**
- https://clickhouse.com/resources/engineering/database-compression
- https://www.lancedb.com/blog/lance-format-v2-2-benchmarks-half-the-storage-none-of-the-slowdown
- https://arxiv.org/html/2504.15247v1
- https://www.jeronimo.dev/compression-algorithms-parquet/
- https://www.timestored.com/data/store-market-tick-data

**Engines & benchmarks**
- https://clickhouse.com/resources/engineering/fastest-olap-databases
- https://github.com/ClickHouse/ClickBench
- https://kx.com/blog/benchmarking-kdb-x-vs-questdb-clickhouse-timescaledb-and-influxdb-with-tsbs/
- https://datafusion.apache.org/blog/output/2026/08/25/datafusion-55.0.0/
- https://pola.rs/posts/announcing-polars-2/
- https://pola.rs/posts/polars-in-aggregate-jul26/
- https://clickhouse.com/docs/whats-new/changelog
- https://clickhouse.com/docs/operations/quotas
- https://trino.io/docs/current/connector/iceberg.html
- https://www.tigerdata.com/learn/best-managed-time-series-databases-in-2026
- https://github.com/man-group/ArcticDB
- https://docs.arcticdb.io/latest/
- https://github.com/man-group/ArcticDB/releases
- https://code.kx.com/q/wp/query-scaling/

**Equities / corporate actions / calendars**
- https://riazarbi.github.io/quant/backtesting-adjusting-prices/
- https://quodd.com/hubfs/corporate-actions-handling-in-globalhistorical-v3.pdf
- https://www.crsp.org/research/crsp-survivor-bias-free-us-mutual-funds/
- https://www.sec.gov/investor/alerts/circuitbreakersbulletin.htm
- https://pypi.org/project/exchange_calendars/
- https://github.com/jenskeiner/exchange_calendars_extensions
- https://www.hbs.edu/ris/Publication%20Files/23-025_563e45c6-df92-4d9c-ae05-608d4d0acab1.pdf

**Futures**
- https://databento.com/docs/examples/symbology/continuous
- https://www.quantstart.com/articles/Continuous-Futures-Contracts-for-Backtesting-Purposes/
- https://quantpedia.com/continuous-futures-contracts-methodology-for-backtesting/
- https://hudson-and-thames-arbitragelab.readthedocs-hosted.com/en/latest/data/futures_rollover.html

**Options**
- https://databento.com/blog/beyond-40-gbps-processing-opra-in-real-time
- https://databento.com/microstructure/opra
- https://databento.com/datasets/OPRA.PILLAR
- https://massive.com/docs/flat-files/quickstart
- https://massive.com/docs/flat-files/options/minute-aggregates/2024/01
- https://datashop.cboe.com/data-products
- https://www.derivasys.com/what-is-svi
- https://repositori.upf.edu/items/eceeb187-f169-483e-bf67-416fd9e00d70
- https://chrischow.github.io/dataandstuff/2022-01-13-open-options-chains-part-i/

**Crypto**
- https://www.cmegroup.com/articles/faqs/cme-cf-cryptocurrency-benchmarks-faq.html
- https://www.cmegroup.com/trading/files/bitcoin-white-paper.pdf
- https://www.cfbenchmarks.com/data/indices/BRTI
- https://quantpedia.com/detecting-wash-trading-in-major-crypto-exchanges/
- https://www.sciencedirect.com/science/article/pii/S1057521926002103
- https://www.chainalysis.com/blog/crypto-market-manipulation-wash-trading-pump-and-dump-2025/
- https://www.mdpi.com/2227-7390/14/2/346
- https://www.coinapi.io/blog/historical-data-for-perpetual-futures

**DeFi / on-chain**
- https://docs.envio.dev/blog/indexing-and-reorgs
- https://docs.envio.dev/blog/best-blockchain-indexers-2026
- https://www.trmlabs.com/trm-tech-blog/how-trm-handles-blockchain-reorgs-across-evm-chains
- https://docs.tatum.io/docs/evm-block-finality-and-confidence
- https://www.allium.so/compare/allium-vs-dune
- https://www.allium.so/compare/allium-vs-goldsky
- https://arxiv.org/html/2405.17944v2
- https://www.mdpi.com/2813-2203/4/3/23
- https://developers.uniswap.org/docs/sdks/v3/guides/pool-data

**Feature platforms**
- https://medium.com/airbnb-engineering/chronon-a-declarative-feature-engineering-framework-b7b8ce796e04
- https://chronon.ai/
- https://github.com/airbnb/chronon
- https://docs.feast.dev/reference/offline-stores/overview
- https://mlflow.org/docs/latest/ml/dataset/

**Multi-tenancy**
- https://queryplane.com/blog/postgres-row-level-security-in-practice/
- https://propelius.tech/blogs/multi-tenant-database-isolation-postgresql-rls-schema/
- https://aws.amazon.com/blogs/storage/design-patterns-for-multi-tenant-access-control-on-amazon-s3
- https://docs.aws.amazon.com/prescriptive-guidance/latest/patterns/implement-saas-tenant-isolation-for-amazon-s3-by-using-an-aws-lambda-token-vending-machine.html
- https://hidekazu-konishi.com/entry/aws_saas_multi_tenant_architecture_guide.html
- https://cacm.acm.org/research/belt-and-braces-when-federated-learning-meets-differential-privacy/

**Immutability / WORM / compliance**
- https://aws.amazon.com/compliance/secrule17a-4f/
- https://www.luthor.ai/guides/worm-vs-audit-trail-17a-4-storage-method-2025-architecture
- https://mnemoshare.com/blog/s3-object-lock-governance-vs-compliance-mode
- https://aws.amazon.com/blogs/storage/protecting-data-with-amazon-s3-object-lock/
- https://oneuptime.com/blog/post/2026-02-12-configure-s3-object-lock-worm-compliance/view

**Cost**
- https://www.cloudzero.com/blog/s3-pricing/
- https://tech-insider.org/cloudflare-r2-vs-s3-vs-backblaze-b2-2026/
- https://egresscost.com/cloudflare/
- https://leanopstech.com/blog/aws-s3-glacier-pricing-2026/
- https://cloudchipr.com/blog/amazon-s3-pricing-explained
