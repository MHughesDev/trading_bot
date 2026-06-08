-- ClickHouse: computed feature events from the feature engine.
-- Values are stored as Decimal128 where precision matters (e.g. EMAs).

CREATE TABLE IF NOT EXISTS features_technical (
    event_id            UUID,
    lane                String,
    instrument_id       String,
    venue_id            String,
    source              String,
    available_time      DateTime64(9, 'UTC'),
    ingested_time       DateTime64(9, 'UTC'),
    sequence            UInt64,
    -- Features are key/value pairs; one row per (instrument, time, feature_name)
    feature_name        String,
    value               Decimal128(18),        -- never Float64
    dedup_key           String
)
ENGINE = ReplacingMergeTree()
ORDER BY (instrument_id, available_time, feature_name)
PARTITION BY toYYYYMM(available_time);
