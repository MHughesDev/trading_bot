-- 0207 -- the dataset declaration is stored beside the resolved dataset (Set N N-1.5, ADR-N-01).
--
-- A training run now *declares* its data (`DatasetDeclaration`, N-1.1) and the planner
-- resolves it into the existing hashed spec. The resolved spec and `dataset_id` are
-- untouched (ADR-N-01); what is added here is the declaration that produced a
-- version, its canonical hash, the handle of the materialised Parquet, and the
-- leakage-harness report that had to pass before the version was registered.

ALTER TABLE dataset_versions
    ADD COLUMN IF NOT EXISTS dataset_spec      JSONB,
    ADD COLUMN IF NOT EXISTS declaration_hash  TEXT,
    ADD COLUMN IF NOT EXISTS dataset_handle    TEXT,
    ADD COLUMN IF NOT EXISTS leakage_report    JSONB;

-- A recorded leakage report is a verdict, and a failing one means the version
-- must not exist: the registration path refuses before inserting, and this
-- constraint is the schema's own statement of the same rule.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'dataset_versions_leakage_must_pass'
  ) THEN
    ALTER TABLE dataset_versions ADD CONSTRAINT dataset_versions_leakage_must_pass CHECK (
      leakage_report IS NULL OR (leakage_report ->> 'passed') = 'true'
    );
  END IF;
END $$;

-- The declaration and family a run was started with, so a retrain with no explicit
-- request is the same dataset (shifted to now), not a silent fallback to defaults.
ALTER TABLE training_runs
    ADD COLUMN IF NOT EXISTS declaration_json  JSONB,
    ADD COLUMN IF NOT EXISTS family_json       JSONB;

CREATE INDEX IF NOT EXISTS idx_dataset_versions_declaration_hash
    ON dataset_versions (declaration_hash) WHERE declaration_hash IS NOT NULL;

GRANT SELECT, INSERT, UPDATE ON dataset_versions TO app_role;
