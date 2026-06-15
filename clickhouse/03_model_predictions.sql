-- AI Model Studio: high-volume prediction log (append-only analytics).
-- All decimal-bearing fields stored as strings (ADR-0002 compliance).

CREATE TABLE IF NOT EXISTS model_predictions (
    model_id        String,
    version         UInt32,
    instrument_id   String,
    event_time_us   Int64,
    produced_time_us Int64,
    direction       String,
    magnitude_str   String,
    confidence      Float64,
    horizon         String
)
ENGINE = MergeTree()
ORDER BY (model_id, instrument_id, event_time_us)
PARTITION BY toYYYYMM(toDateTime(event_time_us / 1000000));
