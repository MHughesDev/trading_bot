-- AI Model Studio: evaluation run history.

CREATE TABLE IF NOT EXISTS evaluation_runs (
    eval_id                 UUID PRIMARY KEY,
    model_id                TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version                 INTEGER NOT NULL,
    eval_dataset_version_id UUID,
    baseline_version        INTEGER,
    status                  TEXT NOT NULL DEFAULT 'queued',
    metrics_json            JSONB,
    scorecard_json          JSONB,
    regression_report_json  JSONB,
    sample_outputs_json     JSONB,
    error                   TEXT,
    created_by              UUID NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at              TIMESTAMPTZ,
    finished_at             TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_evaluation_runs_model ON evaluation_runs(model_id, created_at DESC);
