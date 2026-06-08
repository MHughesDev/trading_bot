-- ClickHouse: normalized trade events.
-- ReplacingMergeTree provides eventual dedup on the deterministic dedup key.
-- price and size are Decimal128 — never Float64.

CREATE TABLE IF NOT EXISTS market_trades (
    -- Envelope fields
    event_id            UUID,
    lane                String,
    instrument_id       String,
    venue_id            String,
    source              String,
    trust_tier          String,
    event_time          Nullable(DateTime64(9, 'UTC')),
    observed_time       DateTime64(9, 'UTC'),
    ingested_time       DateTime64(9, 'UTC'),
    available_time      DateTime64(9, 'UTC'),  -- replay sort key
    sequence            UInt64,
    -- Trade payload
    price               Decimal128(10),        -- never Float64
    size                Decimal128(10),        -- never Float64
    side                String,
    exchange_trade_id   String,
    -- Dedup
    dedup_key           String                 -- venue_id + exchange_trade_id
)
ENGINE = ReplacingMergeTree()
ORDER BY (instrument_id, available_time, dedup_key)
PARTITION BY toYYYYMM(available_time);
