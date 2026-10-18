-- 0209 -- model storage columns and the series link table (Set N N-4.6, FEAT-006 section 9).
--
-- Not the prediction-series table: Set U built that (U-5.5). This adds what a model
-- version needs to say about itself (its family, the code that produced it, the
-- engine that ran it, whether it must be retrained, what it may have seen) and the
-- link from a version to the prediction series computed from it.

ALTER TABLE model_versions
    ADD COLUMN IF NOT EXISTS family            TEXT,
    ADD COLUMN IF NOT EXISTS family_variant    TEXT,
    ADD COLUMN IF NOT EXISTS code_handle       TEXT,
    ADD COLUMN IF NOT EXISTS engine_version    TEXT,
    ADD COLUMN IF NOT EXISTS requires_retrain  BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS contamination     JSONB;

CREATE TABLE IF NOT EXISTS model_series_links (
    link_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id     TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version      INTEGER NOT NULL,
    series_id    TEXT NOT NULL,
    series_hash  TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (model_id, version) REFERENCES model_versions(model_id, version) ON DELETE CASCADE,
    UNIQUE (model_id, version, series_id)
);
CREATE INDEX IF NOT EXISTS idx_model_series_links_series ON model_series_links (series_id);

GRANT SELECT, INSERT, UPDATE, DELETE ON model_series_links TO app_role;
GRANT SELECT, INSERT, UPDATE ON model_versions TO app_role;
