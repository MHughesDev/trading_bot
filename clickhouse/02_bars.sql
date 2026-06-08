-- ClickHouse: OHLCV bar events (primary analytics table).
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
