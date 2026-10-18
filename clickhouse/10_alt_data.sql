-- Set P Phase 3 (P-3.1, P-3.4, P-3.5): the alternative-data stores.
--
-- Every one of these sources is point-in-time-hostile by default: the naive read
-- returns today's revised value for a historical date. The common rule is to store
-- the VINTAGE, not the current value, and to read through `knowledge_time` /
-- `available_time` and nothing else. Revisions are new rows; nothing is updated.

-- ── Macro observations, one row per vintage (S4, P-3.1) ────────────────────────
-- `observation_date` is the period the number describes; `vintage_date` the day
-- that number became the published one. A read as of t returns, per observation
-- date, the latest vintage whose knowledge_time <= t.
CREATE TABLE IF NOT EXISTS macro_observations (
    series_id         LowCardinality(String),
    observation_date  Date,
    vintage_date      Date,
    -- NULL where ALFRED reports "." — missing, never zero.
    value             Nullable(Decimal(38, 18)),
    knowledge_time    DateTime64(9, 'UTC'),
    ingest_time       DateTime64(9, 'UTC'),
    quality_flags     UInt32,
    source_id         Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (series_id, observation_date, vintage_date)
PARTITION BY toYear(observation_date);

-- ── Prediction-market probabilities (S7, P-3.4) ────────────────────────────────
-- Exposure only: a readable series, no trading surface. `event_time` is the
-- period's close; `knowledge_time` is the receipt (live) or close + declared lag
-- (history, flagged BACKFILLED_KNOWLEDGE_TIME).
CREATE TABLE IF NOT EXISTS prediction_probabilities (
    venue              LowCardinality(String),
    ticker             String,
    event_time         DateTime64(9, 'UTC'),
    knowledge_time     DateTime64(9, 'UTC'),
    ingest_time        DateTime64(9, 'UTC'),
    close_probability  Decimal(38, 18),
    volume             Int64,
    open_interest      Int64,
    quality_flags      UInt32,
    source_id          Int32
)
ENGINE = ReplacingMergeTree(ingest_time)
ORDER BY (venue, ticker, event_time, knowledge_time)
PARTITION BY toYYYYMM(event_time);

-- ── Text items, one row per revision (S2, P-3.5; DATA-005 §10) ─────────────────
-- A corrected article is a second row with a higher `revision`, never an update.
-- Reads filter `available_time <= as_of` and take the highest revision per item.
CREATE TABLE IF NOT EXISTS text_items (
    item_id         String,
    revision        UInt32,
    source_id       Int32,
    external_id     String,
    headline        String,
    body            Nullable(String),
    symbols         Array(String),
    published_at    DateTime64(9, 'UTC'),
    ingested_time   DateTime64(9, 'UTC'),
    available_time  DateTime64(9, 'UTC')
)
ENGINE = ReplacingMergeTree(ingested_time)
ORDER BY (item_id, revision)
PARTITION BY toYYYYMM(published_at);
