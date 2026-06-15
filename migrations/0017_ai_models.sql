-- AI Model Studio: core model identity, alias pointers, and audit log.

CREATE TABLE IF NOT EXISTS ai_models (
    model_id        TEXT PRIMARY KEY,
    slug            TEXT NOT NULL UNIQUE,
    display_name    TEXT NOT NULL,
    description     TEXT,
    model_kind      TEXT NOT NULL,
    asset_class     TEXT NOT NULL,
    definition_json JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'draft',
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_ai_models_owner ON ai_models(created_by, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_ai_models_kind  ON ai_models(model_kind, asset_class);

CREATE TABLE IF NOT EXISTS model_aliases (
    model_id   TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    alias      TEXT NOT NULL,
    version    INTEGER NOT NULL,
    updated_by UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (model_id, alias)
);

CREATE TABLE IF NOT EXISTS model_events (
    event_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id   TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    payload    JSONB NOT NULL,
    actor      UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_model_events_model ON model_events(model_id, created_at DESC);
