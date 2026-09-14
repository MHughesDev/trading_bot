-- ML platform L0 in ClickHouse (SPEC §1.2–§1.8). Surrogate identity, four
-- timestamps, DECIMAL(38,18) prices, bar-OPEN convention, quality flags, restatement
-- index, and the high-volume asset-class sidecars.
--
-- Supersedes market_bars_v2 as the canonical bar store. v2 stored the bar CLOSE in
-- `event_time`, keyed rows by symbol string, used Decimal(38,10), and stamped REST
-- backfills as knowable at the close. Rows are copied here once, at boot, with
-- surrogate ids, the open convention, and an honest BACKFILLED_KNOWLEDGE_TIME flag on
-- every row that was not observed live (CLAUDE.md §6).

-- The canonical bar. One table for every asset class and bar length; sparse.
CREATE TABLE IF NOT EXISTS market_bar (
    instrument_id   Int64,
    venue_id        Int32,
    period_secs     UInt32,
    event_time      DateTime64(9, 'UTC'),            -- bar OPEN, UTC (INV-06)
    venue_ts        Nullable(DateTime64(9, 'UTC')),
    ingest_time     DateTime64(9, 'UTC'),
    knowledge_time  DateTime64(9, 'UTC'),            -- queryable by a strategy (INV-01)
    open            Decimal(38, 18),                 -- never Float64 (INV-05)
    high            Decimal(38, 18),
    low             Decimal(38, 18),
    close           Decimal(38, 18),
    volume          Decimal(38, 18),
    trade_count     Nullable(UInt32),
    vwap            Nullable(Decimal(38, 18)),
    bid_close       Nullable(Decimal(38, 18)),
    ask_close       Nullable(Decimal(38, 18)),
    bipower_var     Nullable(Float64),
    n_updates       Nullable(UInt32),
    quality_flags   UInt32,
    revision_seq    UInt32,
    source_id       Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (instrument_id, venue_id, period_secs, event_time, knowledge_time, revision_seq)
PARTITION BY (period_secs, toYYYYMM(event_time));

-- Partitions that contain any restatement (§1.3). Absent ⇒ one version per cell, and
-- the PIT reader skips resolution entirely.
CREATE TABLE IF NOT EXISTS restatement_index (
    instrument_id       Int64,
    venue_id            Int32,
    period_secs         UInt32,
    event_date          Date,
    n_revisions         UInt32,
    max_knowledge_time  DateTime64(9, 'UTC')
)
ENGINE = ReplacingMergeTree(max_knowledge_time)
ORDER BY (instrument_id, venue_id, period_secs, event_date);

-- Replicated identity dimensions. Postgres (dataplane.*) is the system of record;
-- these are synced copies so the single PIT reader can resolve symbols in-query.
CREATE TABLE IF NOT EXISTS instrument_symbol_dim (
    instrument_id   Int64,
    venue_id        Int32,
    symbol          String,
    valid_from      DateTime64(9, 'UTC'),
    valid_to        Nullable(DateTime64(9, 'UTC')),
    knowledge_time  DateTime64(9, 'UTC')
)
ENGINE = ReplacingMergeTree
ORDER BY (venue_id, symbol, instrument_id, valid_from, knowledge_time);

CREATE TABLE IF NOT EXISTS venue_dim (
    venue_id      Int32,
    name          String,
    quality_tier  UInt8
)
ENGINE = ReplacingMergeTree
ORDER BY venue_id;

-- Options: long, sparse, IV stored with its inputs, no greeks (§1.6, INV-07).
CREATE TABLE IF NOT EXISTS option_bar_1m (
    instrument_id     Int64,
    underlying_id     Int64,
    expiry            Date,
    event_time        DateTime64(9, 'UTC'),
    ingest_time       DateTime64(9, 'UTC'),
    knowledge_time    DateTime64(9, 'UTC'),
    open              Nullable(Decimal(38, 18)),
    high              Nullable(Decimal(38, 18)),
    low               Nullable(Decimal(38, 18)),
    close             Nullable(Decimal(38, 18)),
    volume            Nullable(Int64),
    open_interest     Nullable(Int64),
    bid_close         Nullable(Decimal(38, 18)),
    ask_close         Nullable(Decimal(38, 18)),
    underlying_close  Nullable(Decimal(38, 18)),
    iv_close          Nullable(Float64),
    iv_model          Nullable(String),
    iv_rate           Nullable(Float64),
    iv_div            Nullable(Float64),
    moneyness         Float64,
    dte               Int32,
    in_liquid_universe UInt8,
    quality_flags     UInt32,
    source_id         Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (underlying_id, dte, moneyness, instrument_id, event_time, knowledge_time)
PARTITION BY (toYYYYMM(expiry), toYYYYMM(event_time));

-- DeFi pool observations keyed on (chain, block_number, block_hash) (§1.8, INV-09).
CREATE TABLE IF NOT EXISTS pool_observation (
    chain_id        Int32,
    block_number    Int64,
    block_hash      String,
    pool_id         Int64,
    reserve0        Nullable(Decimal(38, 18)),
    reserve1        Nullable(Decimal(38, 18)),
    tvl_usd         Nullable(Decimal(38, 18)),
    fee_bps         Nullable(Int32),
    sqrt_price      Nullable(Decimal(38, 18)),
    liquidity       Nullable(Decimal(38, 18)),
    knowledge_time  DateTime64(9, 'UTC'),
    quality_flags   UInt32
)
ENGINE = ReplacingMergeTree(knowledge_time)
ORDER BY (chain_id, pool_id, block_number, block_hash)
PARTITION BY (chain_id, toYYYYMM(knowledge_time));

-- Boot-time copy bookkeeping for the v2 → canonical migration.
CREATE TABLE IF NOT EXISTS canonical_migration_chunk (
    source_table  String,
    timeframe     String,
    month         UInt32,
    rows_copied   UInt64,
    copied_at     DateTime64(9, 'UTC')
)
ENGINE = ReplacingMergeTree(copied_at)
ORDER BY (source_table, timeframe, month);
