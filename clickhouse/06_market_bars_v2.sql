-- Set-L Phase 0 (L-0.1): OHLCV bars, v2 — append-only, no collapsing engine.
--
-- Replaces `market_bars` (02_bars.sql), which is
-- `ReplacingMergeTree(revision) ORDER BY (instrument_id, available_time)`.
-- `timeframe` is absent from that sorting key, so a 1m bar and a 1h bar closing at
-- the same instant are treated as duplicates and one is destroyed on merge. Measured
-- on the live table 2026-09-11: 18 of BTC-USD's 22 1h bars were queued for
-- destruction — every 1h bar overlapping 1m coverage. See DATA-005 §3 (DA-16, DA-17).
--
-- Three changes, each load-bearing:
--   1. The sorting key is COMPLETE: instrument, timeframe, venue, source, close time
--      and revision. v1 collapsed rows that differed in any of those, because none of
--      them were in its key.
--   2. `timeframe` is in the key, so 1m and 1h bars closing together stay separate.
--   3. `revision` is in the key, so a late-data correction is a new row rather than an
--      overwrite, and `as_of` reads can pick the revision current at a past instant
--      (DATA-005 §3 item 2).
--
-- Engine: ReplacingMergeTree(ingested_time), keyed as above.
--
-- DATA-005 §3 first specified a plain MergeTree, on the reasoning that an
-- append-only table cannot lose a row to a sorting-key mistake. That is true, and
-- it is why the key above is complete. But it traded one failure for another:
-- re-collecting a bar is routine — gap fill re-reads ranges on every boot — and v1
-- collapsed those exact repeats while an append-only table keeps every copy
-- forever. Measured on 2026-09-11, hours after the cutover: 1,688 rows were exact
-- re-collections, same `event_id`, same revision. Reads were still correct, because
-- they resolve with argMax, but the table grows without bound and `count()` stops
-- meaning "bars".
--
-- So rows collapse only when instrument, timeframe, venue, source, close time AND
-- revision all match — i.e. only for a genuine repeat of the same observation, with
-- the most recently ingested copy winning. Nothing that differs in any meaningful
-- dimension can be destroyed, which is exactly the property v1 lacked.
--
-- All OHLCV columns stay Decimal128 — never Float64.

CREATE TABLE IF NOT EXISTS market_bars_v2 (
    -- Envelope, unchanged from v1.
    event_id            UUID,
    lane                String,
    instrument_id       String,
    venue_id            String,
    source              String,
    trust_tier          String,

    -- The three timestamps, kept distinct on purpose.
    --   bar_open_time  : the bar's opening edge.
    --   event_time     : the bar's close (window_close). What a researcher means by
    --                    "the 14:00 bar", and the sorting key's time column.
    --   available_time : when the platform could first have known the bar. This is
    --                    what the research cutoff and every PIT filter compare
    --                    against (DA-02). Conflating it with event_time is how
    --                    lookahead gets in.
    bar_open_time       DateTime64(9, 'UTC'),
    event_time          DateTime64(9, 'UTC'),
    available_time      DateTime64(9, 'UTC'),
    ingested_time       DateTime64(9, 'UTC'),

    sequence            UInt64,
    timeframe           String,                -- '1s' | '1m' | '1h' | …

    -- Bar payload.
    open                Decimal128(10),        -- never Float64
    high                Decimal128(10),        -- never Float64
    low                 Decimal128(10),        -- never Float64
    close               Decimal128(10),        -- never Float64
    volume              Decimal128(10),        -- never Float64
    trade_count         UInt64,

    -- 0 = original, >0 = late-data revision. Kept, not overwritten: a revision is a
    -- new row, and a read chooses between them.
    revision            UInt32 DEFAULT 0,
    dedup_key           String,

    -- Monotonic ingest stamp, for as-of reads that need a total order within a
    -- (instrument, timeframe, event_time, revision) group.
    snapshot_id         UInt64 MATERIALIZED toUInt64(toUnixTimestamp64Nano(ingested_time))
)
ENGINE = ReplacingMergeTree(ingested_time)
ORDER BY (instrument_id, timeframe, venue_id, source, event_time, revision)
PARTITION BY (timeframe, toYYYYMM(event_time));

-- Canonical latest-as-of read. Every consumer MUST use this shape, so that two call
-- sites cannot disagree about which revision is current (DA-02, DA-16):
--
--   SELECT event_time,
--          argMax(open,   (revision, ingested_time)) AS open,
--          argMax(high,   (revision, ingested_time)) AS high,
--          argMax(low,    (revision, ingested_time)) AS low,
--          argMax(close,  (revision, ingested_time)) AS close,
--          argMax(volume, (revision, ingested_time)) AS volume
--   FROM market_bars_v2
--   WHERE instrument_id = {instrument:String}
--     AND timeframe     = {timeframe:String}
--     AND available_time <= {as_of:DateTime64(9)}
--   GROUP BY event_time
--   ORDER BY event_time;
--
-- NOTE: this file runs only on FIRST database init (the `clickhouse/` directory is
-- mounted at /docker-entrypoint-initdb.d). Existing installs converge through the
-- idempotent apply path in L-0.2, not by re-running this file by hand.
