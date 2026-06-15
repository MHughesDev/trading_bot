-- AI Model Studio: training run history (mirrors backtest_runs pattern).

CREATE TABLE IF NOT EXISTS training_runs (
    run_id              UUID PRIMARY KEY,
    model_id            TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    dataset_version_id  UUID,
    status              TEXT NOT NULL DEFAULT 'queued',
    progress            REAL NOT NULL DEFAULT 0,
    phase               TEXT,
    hyperparameters_json JSONB,
    metrics_json        JSONB,
    logs_uri            TEXT,
    sidecar_job_ref     TEXT,
    error               TEXT,
    created_by          UUID NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at          TIMESTAMPTZ,
    finished_at         TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_training_runs_model ON training_runs(model_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_training_runs_status ON training_runs(status);
