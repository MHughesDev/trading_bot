-- AI Model Studio: model versions and stored artifacts.

CREATE TABLE IF NOT EXISTS model_versions (
    model_id            TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version             INTEGER NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft',
    training_run_id     UUID,
    dataset_version_id  UUID,
    artifact_id         UUID,
    metrics_json        JSONB,
    scorecard_json      JSONB,
    config_json         JSONB NOT NULL,
    notes               TEXT,
    created_by          UUID NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    promoted_at         TIMESTAMPTZ,
    PRIMARY KEY (model_id, version)
);
CREATE INDEX IF NOT EXISTS idx_model_versions_status ON model_versions(model_id, status);

CREATE TABLE IF NOT EXISTS model_artifacts (
    artifact_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id      TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version       INTEGER NOT NULL,
    storage_uri   TEXT NOT NULL,
    artifact_type TEXT NOT NULL,
    size_bytes    BIGINT NOT NULL,
    sha256        TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
