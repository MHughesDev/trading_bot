-- Set-J Phase 0: the immutable, content-addressed run store (spec §1.1).
--
-- Every executed Run — ok, failed, or rejected_integrity — is written here
-- keyed by its `run_id` (the SHA-256 of the full RunConfig). Writes are
-- idempotent: a row is inserted exactly once and never updated or deleted. The
-- high-volume per-bar series (equity curve, positions, trades) live in
-- ClickHouse (clickhouse/05_backtest_run_series.sql); this table is the
-- system-of-record metadata + the full config/result documents.

CREATE TABLE IF NOT EXISTS backtest_runs (
    run_id          TEXT PRIMARY KEY,          -- sha256:<hex> content hash
    status          TEXT NOT NULL,             -- ok | failed | rejected_integrity
    config_json     JSONB NOT NULL,            -- full RunConfig (reproducible input)
    result_json     JSONB NOT NULL,            -- full RunResult (sans bulk series)
    engine_version  TEXT NOT NULL,             -- produced_by
    unsafe          BOOLEAN NOT NULL DEFAULT FALSE,  -- INV-1: any protection disabled
    wall_ms         BIGINT NOT NULL DEFAULT 0,
    cpu_ms          BIGINT NOT NULL DEFAULT 0,
    created_by      TEXT,                      -- user scope (MASTER §8)
    produced_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Common funnel queries: by status (count failures/rejections toward trials),
-- by safety, and by producing engine.
CREATE INDEX IF NOT EXISTS idx_backtest_runs_status ON backtest_runs(status);
CREATE INDEX IF NOT EXISTS idx_backtest_runs_unsafe ON backtest_runs(unsafe);
CREATE INDEX IF NOT EXISTS idx_backtest_runs_engine ON backtest_runs(engine_version);

-- Immutability guard: forbid UPDATE/DELETE on stored runs. A second write of the
-- same run_id is an idempotent no-op (ON CONFLICT DO NOTHING at the call site);
-- this trigger makes mutation impossible even by hand.
CREATE OR REPLACE FUNCTION backtest_runs_immutable()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'backtest_runs is append-only and immutable (run_id=%)',
        COALESCE(OLD.run_id, NEW.run_id);
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_backtest_runs_no_update ON backtest_runs;
CREATE TRIGGER trg_backtest_runs_no_update
    BEFORE UPDATE OR DELETE ON backtest_runs
    FOR EACH ROW EXECUTE FUNCTION backtest_runs_immutable();
