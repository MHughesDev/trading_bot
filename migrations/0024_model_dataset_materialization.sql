-- Set-H Phase 2: dataset materialization support.
--
-- The Rust `DatasetManager` (crates/model-registry/src/datasets.rs) materializes
-- immutable dataset versions keyed by a deterministic content_hash. The original
-- 0019 schema modelled feature sets as a separate table; Phase 2 stores the
-- `feature_set_ref` directly on the version row alongside the Parquet URI and the
-- resolved instrument list, so the trainer sidecar can fetch the dataset by URI.

-- Relax the `datasets` identity table so a logical dataset can be upserted from a
-- materialization request that only carries a feature_set_ref + label_spec.
ALTER TABLE datasets
    ADD COLUMN IF NOT EXISTS feature_set_ref TEXT,
    ADD COLUMN IF NOT EXISTS label_spec_json JSONB;
ALTER TABLE datasets ALTER COLUMN name DROP NOT NULL;
ALTER TABLE datasets ALTER COLUMN asset_class DROP NOT NULL;
ALTER TABLE datasets ALTER COLUMN created_by DROP NOT NULL;

-- Augment dataset_versions with the columns the materializer records directly.
ALTER TABLE dataset_versions
    ADD COLUMN IF NOT EXISTS feature_set_ref TEXT,
    ADD COLUMN IF NOT EXISTS instruments_json JSONB,
    ADD COLUMN IF NOT EXISTS start_time TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS end_time TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS parquet_uri TEXT;

-- The original schema required window_start/window_end + a feature_sets FK; make
-- them optional so a version can be written from the new code path.
ALTER TABLE dataset_versions ALTER COLUMN window_start DROP NOT NULL;
ALTER TABLE dataset_versions ALTER COLUMN window_end DROP NOT NULL;

-- Allow content-hash lookups across the whole table (dedupe re-materializations).
CREATE INDEX IF NOT EXISTS idx_dataset_versions_content_hash
    ON dataset_versions(content_hash);
