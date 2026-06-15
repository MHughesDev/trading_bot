-- Asset lifecycle and initialization tracking.
--
-- asset_lifecycle is the source of truth for whether a symbol has been
-- initialized and is actively being traded.
--
-- asset_init_jobs tracks background bar-seeding jobs spawned by POST
-- /assets/init/:symbol so the UI can poll for progress.

CREATE TABLE IF NOT EXISTS asset_lifecycle (
    symbol          TEXT PRIMARY KEY,
    asset_class     TEXT NOT NULL,
    venue_id        TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'initialized_not_active',
    strategy_id     TEXT,
    execution_mode  TEXT NOT NULL DEFAULT 'paper',
    initialized_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS asset_init_jobs (
    job_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    symbol          TEXT NOT NULL,
    lookback_days   INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'running',   -- 'running' | 'done' | 'error'
    bars_collected  BIGINT NOT NULL DEFAULT 0,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS asset_init_jobs_symbol ON asset_init_jobs(symbol);
