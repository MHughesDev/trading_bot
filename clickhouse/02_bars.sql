-- ClickHouse: OHLCV bar events.
--
-- *** RETIRED AND FROZEN READ-ONLY: 2026-09-11 (Set L, L-0.7). ***
--
-- Superseded by `market_bars_v2` (06_market_bars_v2.sql). This table's
-- ReplacingMergeTree(revision) sorts on (instrument_id, available_time) WITHOUT
-- `timeframe`, so two bars of different timeframes that close at the same instant
-- are treated as duplicates: one is destroyed on merge, and destroyed immediately
-- when both arrive in the same insert batch. Measured on the live table on
-- 2026-09-11: 18 of BTC-USD's 22 1h bars were queued for destruction — every 1h bar
-- overlapping 1m coverage (DATA-005 §3).
--
-- Nothing writes this table any more; `cargo xtask check-bars-v1-frozen` enforces
-- that in CI. It is kept readable as a fallback and is scheduled for removal at the
-- end of Set L. Do NOT run OPTIMIZE on it.
--
-- Original description follows.
-- ReplacingMergeTree ordered on (instrument_id, available_time) for range scans.
-- All OHLCV columns are Decimal128 — never Float64.

CREATE TABLE IF NOT EXISTS market_bars (
    event_id            UUID,
    lane                String,
    instrument_id       String,
    venue_id            String,
    source              String,
    trust_tier          String,
    available_time      DateTime64(9, 'UTC'),  -- replay sort key and ORDER BY key
    ingested_time       DateTime64(9, 'UTC'),
    sequence            UInt64,
    -- Bar payload
    timeframe           String,                -- '1s' | '1m' | etc.
    open                Decimal128(10),        -- never Float64
    high                Decimal128(10),        -- never Float64
    low                 Decimal128(10),        -- never Float64
    close               Decimal128(10),        -- never Float64
    volume              Decimal128(10),        -- never Float64
    trade_count         UInt64,
    revision            UInt32 DEFAULT 0,      -- 0 = original, >0 = late-data revision
    -- Dedup: lane + instrument_id + venue_id + sequence + source
    dedup_key           String
)
ENGINE = ReplacingMergeTree(revision)   -- latest revision wins after merge
ORDER BY (instrument_id, available_time)
PARTITION BY toYYYYMM(available_time);
