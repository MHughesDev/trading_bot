-- AI Model Studio: deployment control (where a version actually serves).

CREATE TABLE IF NOT EXISTS model_deployments (
    deployment_id TEXT PRIMARY KEY,
    model_id      TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version       INTEGER NOT NULL,
    environment   TEXT NOT NULL,
    alias         TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    traffic_pct   INTEGER NOT NULL DEFAULT 100,
    deployed_by   UUID NOT NULL,
    deployed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_traffic_pct CHECK (traffic_pct >= 0 AND traffic_pct <= 100)
);
CREATE INDEX IF NOT EXISTS idx_model_deployments_model ON model_deployments(model_id, environment);
