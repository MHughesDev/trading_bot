-- 0222 -- the tables the model quality monitor and the model call path always queried (Set N N-4.1, N-4.5).
--
-- `quality_monitor.rs` reads `model_forecasts`, writes `model_quality_series` and
-- `model_quality_alerts`, and selects from `ai_model_aliases` and `ai_model_runs`;
-- `crates/api/src/model_call.rs` resolves an alias through `ai_model_aliases`. None of
-- these existed in any migration. Every one of those queries ended in
-- `.unwrap_or_default()` or `.ok()`, so each failure read as "no rows" and the monitor
-- was a quiet no-op while `GET /api/models/{id}/predict` failed on its alias lookup.
--
-- The two `ai_model_*` names are views over the real tables (`model_aliases`,
-- `training_runs`) rather than a rename, so no caller changes and there is one set of rows.

CREATE OR REPLACE VIEW ai_model_aliases AS
    SELECT model_id, alias, version, updated_by, updated_at FROM model_aliases;

CREATE OR REPLACE VIEW ai_model_runs AS
    SELECT run_id, model_id, status, created_at, started_at, finished_at FROM training_runs;

-- One row per forecast the platform served, filled in with the realised value when it is
-- known. Quantile columns are nullable: a model reports the levels it has, and a coverage
-- figure is computed only from the levels that exist.
CREATE TABLE IF NOT EXISTS model_forecasts (
    forecast_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id        TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version         INTEGER,
    instrument_id   TEXT,
    horizon_bars    INTEGER,
    predicted_q05   DOUBLE PRECISION,
    predicted_q10   DOUBLE PRECISION,
    predicted_q25   DOUBLE PRECISION,
    predicted_q50   DOUBLE PRECISION,
    predicted_q75   DOUBLE PRECISION,
    predicted_q90   DOUBLE PRECISION,
    predicted_q95   DOUBLE PRECISION,
    realized        DOUBLE PRECISION,
    realized_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_model_forecasts_model_time ON model_forecasts (model_id, created_at DESC);

CREATE TABLE IF NOT EXISTS model_quality_series (
    model_id      TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    window_end    TIMESTAMPTZ NOT NULL,
    crps          DOUBLE PRECISION,
    coverage_50   DOUBLE PRECISION,
    coverage_90   DOUBLE PRECISION,
    n_forecasts   INTEGER NOT NULL,
    recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (model_id, window_end)
);

CREATE TABLE IF NOT EXISTS model_quality_alerts (
    id             TEXT PRIMARY KEY,
    model_id       TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    kind           TEXT NOT NULL,
    message        TEXT NOT NULL,
    metric_value   DOUBLE PRECISION,
    threshold      DOUBLE PRECISION,
    triggered_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    retrain_run_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_model_quality_alerts_model ON model_quality_alerts (model_id, triggered_at DESC);

GRANT SELECT ON ai_model_aliases, ai_model_runs TO app_role;
GRANT SELECT, INSERT, UPDATE ON model_forecasts TO app_role;
GRANT SELECT, INSERT, UPDATE ON model_quality_series TO app_role;
GRANT SELECT, INSERT, UPDATE ON model_quality_alerts TO app_role;
