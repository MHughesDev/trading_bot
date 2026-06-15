-- AI Model Studio: dataset and feature-set registry.

CREATE TABLE IF NOT EXISTS feature_sets (
    feature_set_id  TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT,
    feature_list_json JSONB NOT NULL,
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS datasets (
    dataset_id    TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    description   TEXT,
    asset_class   TEXT NOT NULL,
    created_by    UUID NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS dataset_versions (
    dataset_version_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id          TEXT NOT NULL REFERENCES datasets(dataset_id) ON DELETE CASCADE,
    version             INTEGER NOT NULL,
    feature_set_id      TEXT REFERENCES feature_sets(feature_set_id),
    window_start        TIMESTAMPTZ NOT NULL,
    window_end          TIMESTAMPTZ NOT NULL,
    label_spec_json     JSONB,
    row_count           BIGINT NOT NULL DEFAULT 0,
    content_hash        TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (dataset_id, version)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_dataset_versions_hash ON dataset_versions(dataset_id, content_hash);
