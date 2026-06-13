-- Backtest run history.
--
-- One row per backtest job.  Live progress is in-memory only; rows are
-- upserted best-effort at phase transitions so finished runs (and their
-- results) survive restarts.  Market data itself stays in ClickHouse.

CREATE TABLE IF NOT EXISTS backtest_runs (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    strategy_slug   TEXT NOT NULL,
    -- Frozen copy of the v1.0 strategy definition the run used.
    definition      JSONB NOT NULL,
    instrument_id   TEXT NOT NULL,
    venue_id        TEXT NOT NULL,
    asset_class     TEXT NOT NULL,
    timeframe       TEXT NOT NULL,
    start_time      TIMESTAMPTZ NOT NULL,
    end_time        TIMESTAMPTZ NOT NULL,
    initial_balance NUMERIC(30, 10) NOT NULL,
    quote_currency  TEXT NOT NULL,
    auto_collect    BOOLEAN NOT NULL DEFAULT TRUE,
    status          TEXT NOT NULL,
    progress        REAL NOT NULL DEFAULT 0,
    error           TEXT,
    -- Phase the job was in when it failed (mid-processing diagnosis).
    failed_phase    TEXT,
    coverage        JSONB,
    result          JSONB,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_backtest_runs_created_at
    ON backtest_runs (created_at DESC);
