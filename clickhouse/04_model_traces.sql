-- AI Model Studio: inference trace log for latency and cost analytics.
-- cost fields stored as strings (ADR-0002 compliance).

CREATE TABLE IF NOT EXISTS model_traces (
    trace_id    UUID,
    model_id    String,
    version     UInt32,
    kind        String,
    latency_ms  UInt64,
    cost_usd_str String,
    input_hash  String,
    output_hash String,
    status      String,
    ts_us       Int64
)
ENGINE = MergeTree()
ORDER BY (model_id, ts_us)
PARTITION BY toYYYYMM(toDateTime(ts_us / 1000000));
