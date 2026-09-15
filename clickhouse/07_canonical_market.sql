-- ML platform L0 in ClickHouse (SPEC §1.2–§1.8). Surrogate identity, four
-- timestamps, DECIMAL(38,18) prices, bar-OPEN convention, quality flags, restatement
-- index, and the high-volume asset-class sidecars.
--
-- Supersedes market_bars_v2 as the canonical bar store. v2 stored the bar CLOSE in
-- `event_time`, keyed rows by symbol string, used Decimal(38,10), and stamped REST
-- backfills as knowable at the close. Rows are copied here once, at boot, with
-- surrogate ids, the open convention, and an honest BACKFILLED_KNOWLEDGE_TIME flag on
-- every row that was not observed live (CLAUDE.md §6).
--
-- Measured 2026-09-15 (Set P P-0.1): all 158,259 v2 rows carry
-- `available_time = event_time`, minimum and maximum lag zero, including the three
-- REST backfill sources. The copy below is therefore the only place those rows ever
-- get an honest knowledge time on this install, and the rule it uses is the one that
-- matters. That rule is the ROW'S OWN RECEIPT, not its source's transport: a forward
-- REST poll was watched arrive and is observed; a `live_stream` source's row that
-- only reached the store weeks later was not. Keying on transport marks every
-- forward poll derived — which would leave Gate 15 with no evidence at all, the
-- opposite of the point (ADR-P2-24 / OQ-01; Set P D-P2, D-P3).

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
    source_id       Int32,

    -- ── The two rules that make the v2 failure structurally impossible ────────
    -- ClickHouse evaluates CONSTRAINT ... CHECK on every INSERT, so these hold for
    -- any writer, including raw SQL that never passes through `BarStore`. They
    -- exist because a comment did not stop this happening the first time: v2
    -- documented the three timestamps' distinct meanings at length and then stamped
    -- all 158,259 of its rows identically anyway.

    -- A bar cannot be knowable before it ends. `event_time` is the bar OPEN
    -- (INV-06), so the close is `event_time + period_secs`.
    CONSTRAINT knowledge_time_is_after_the_bar_closes
        CHECK knowledge_time >= event_time + toIntervalSecond(period_secs),

    -- And a row may not claim to have been knowable before the platform held it
    -- unless it admits, in its flags, that its knowledge time is the declared
    -- sentinel rather than a receipt. This is the v2 bug's exact shape: a REST
    -- backfill ingested weeks after the close, stamped as knowable AT the close,
    -- with nothing recording the difference. Such a row is now refused at INSERT.
    -- 32768 = QualityFlags::BACKFILLED_KNOWLEDGE_TIME (0x8000); the bit values are
    -- persisted and pinned by dataplane::quality::tests::spec_bit_values_are_fixed.
    CONSTRAINT unflagged_knowledge_time_is_a_real_receipt
        CHECK bitAnd(quality_flags, 32768) != 0 OR knowledge_time >= ingest_time
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (instrument_id, venue_id, period_secs, event_time, knowledge_time, revision_seq)
PARTITION BY (period_secs, toYYYYMM(event_time));

-- The third rule — `knowledge_time >= close + the source's declared vendor lag` —
-- cannot be a CHECK, because a ClickHouse constraint may not join to another table
-- and the declared lag lives in Postgres (`dataplane.source`). It is enforced in the
-- writer instead, by `dataplane::bar::validate_against_source`, which `BarStore`
-- calls on every row. `source_dim` below carries the lag into ClickHouse so that
-- read-side auditing (the census's provenance alarm) can compute the same floor
-- without a Postgres round trip per row.

-- Replicated source dimension. Postgres (`dataplane.source`) is the system of
-- record; this is a synced copy. `kind` records the TRANSPORT and decides nothing:
-- provenance is per row, from whether a receipt exists for that row
-- (`dataplane::source::Collection`, `dataplane::bar::classify_receipt`).
CREATE TABLE IF NOT EXISTS source_dim (
    source_id              Int32,
    name                   String,
    kind                   String,
    declared_vendor_lag_ms Int64
)
ENGINE = ReplacingMergeTree
ORDER BY source_id;

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
--
-- `provenance_rule` records WHICH rule stamped the chunk's knowledge times. Rule 1
-- keyed observed-versus-derived on the source's transport (`kind = 'live_stream'`),
-- which marks every forward poll derived and replaces a real receipt with a sentinel
-- that can land later than the truth. Rule 2 keys it on the row's own receipt. A
-- chunk copied under rule 1 holds knowledge times nobody should trust, and the
-- version is in the table so that is discoverable rather than lost.
CREATE TABLE IF NOT EXISTS canonical_migration_chunk (
    source_table    String,
    timeframe       String,
    month           UInt32,
    rows_copied     UInt64,
    copied_at       DateTime64(9, 'UTC'),
    provenance_rule UInt8 DEFAULT 1
)
ENGINE = ReplacingMergeTree(copied_at)
ORDER BY (source_table, timeframe, month);

-- The knowledge-provenance split, as counts, per copied chunk.
--
-- The census answers "how much history is there". This answers the question that
-- turned out to matter more: **how much of it can honestly be called observed** —
-- because under Set P D-P3 that number, and only that number, is the platform's
-- stock of Gate-15-admissible forward evidence. On `market_bars_v2` as measured on
-- 2026-09-15 the answer was expected to be zero, and a repair that quietly produced
-- a large number would be the failure, not the fix.
--
-- Append-only by construction: one row per (chunk, run). Two runs of the repair over
-- an unchanged store produce identical counts, so a changed count is a real change.
CREATE TABLE IF NOT EXISTS knowledge_time_repair (
    source_table      String,
    timeframe         String,
    month             UInt32,
    -- Rows whose ingest stamp is a credible forward receipt: knowledge_time is that
    -- receipt, unflagged, and the row is forward evidence.
    observed_rows     UInt64,
    -- Rows ingested long after their vendor published them: sentinel + flag.
    history_pull_rows UInt64,
    -- Rows whose ingest stamp precedes their own bar close. Impossible, so never
    -- promoted to an observation: sentinel + flag, counted separately because a
    -- non-zero value means some clock or parser is wrong and should be found.
    impossible_rows   UInt64,
    provenance_rule   UInt8,
    repaired_at       DateTime64(9, 'UTC')
)
ENGINE = MergeTree
ORDER BY (source_table, timeframe, month, repaired_at);
