-- The leakage suite's record (SPEC §12.5, checklist 1.7-1.11).
--
-- Two things are recorded, both append-only:
--
--   1. `dataplane.dataset_frame_digest` -- the digest of the bytes a `dataset_id`
--      produced, the first time it produced them. Snapshot reproducibility
--      (§12.5·3) is the comparison between that digest and a later rebuild's, so
--      the first one has to be on record before there is anything to compare
--      against. The row is immutable: if a rebuild disagrees, the answer is a
--      finding, never an update.
--
--   2. `mlops.leakage_run` + `mlops.leakage_finding` -- every pass of the suite,
--      clean or not. A suite that only records its failures cannot tell "no
--      leaks" from "never ran", and "never ran" is the state that matters.

CREATE TABLE IF NOT EXISTS dataplane.dataset_frame_digest (
  dataset_id   TEXT PRIMARY KEY,
  tenant_id    TEXT NOT NULL,
  -- Digest of the materialized frame's bytes. Distinct from `dataset_id`, which
  -- is the hash of the *spec*: INV-12 is the claim that one determines the other,
  -- and this column is what makes that claim falsifiable.
  frame_digest TEXT NOT NULL,
  row_count    BIGINT NOT NULL CHECK (row_count >= 0),
  -- The request that produced these bytes, verbatim. The spec alone cannot be
  -- replayed: it carries the feature set's *version hash*, not the name a
  -- reader resolves, and none of the read parameters. Without this column a
  -- reproducibility check can only compare two things it cannot regenerate.
  request      JSONB NOT NULL,
  recorded_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
DROP TRIGGER IF EXISTS trg_dataset_frame_digest_immutable ON dataplane.dataset_frame_digest;
CREATE TRIGGER trg_dataset_frame_digest_immutable
  BEFORE UPDATE OR DELETE ON dataplane.dataset_frame_digest
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE dataplane.dataset_frame_digest ENABLE ROW LEVEL SECURITY;
ALTER TABLE dataplane.dataset_frame_digest FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON dataplane.dataset_frame_digest;
CREATE POLICY tenant_isolation ON dataplane.dataset_frame_digest
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

CREATE TABLE IF NOT EXISTS mlops.leakage_run (
  leakage_run_id UUID PRIMARY KEY,
  tenant_id      TEXT NOT NULL,
  -- What was audited: a dataset id, or 'platform' for the harness-level pass.
  subject        TEXT NOT NULL,
  -- Which checks ran. A check that did not run is absent, never recorded as a pass.
  checks_run     TEXT[] NOT NULL,
  blocking_count INT NOT NULL CHECK (blocking_count >= 0),
  flag_count     INT NOT NULL CHECK (flag_count >= 0),
  -- The random-label pass's headline number, when it ran.
  permuted_max_abs_t DOUBLE PRECISION,
  real_sharpe    DOUBLE PRECISION,
  started_at     TIMESTAMPTZ NOT NULL,
  finished_at    TIMESTAMPTZ NOT NULL,
  CHECK (finished_at >= started_at),
  CHECK (cardinality(checks_run) >= 1)
);
CREATE INDEX IF NOT EXISTS idx_leakage_run_recent ON mlops.leakage_run (tenant_id, finished_at DESC);
DROP TRIGGER IF EXISTS trg_leakage_run_immutable ON mlops.leakage_run;
CREATE TRIGGER trg_leakage_run_immutable BEFORE UPDATE OR DELETE ON mlops.leakage_run
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.leakage_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.leakage_run FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.leakage_run;
CREATE POLICY tenant_isolation ON mlops.leakage_run
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

CREATE TABLE IF NOT EXISTS mlops.leakage_finding (
  leakage_run_id UUID NOT NULL REFERENCES mlops.leakage_run(leakage_run_id),
  ordinal        INT  NOT NULL,
  tenant_id      TEXT NOT NULL,
  check_name     TEXT NOT NULL CHECK (check_name IN
                   ('causal_access','random_label','snapshot_reproducibility','cv_wf_gap',
                    'target_correlation','full_sample_normalization')),
  severity       TEXT NOT NULL CHECK (severity IN ('blocking','flag')),
  subject        TEXT NOT NULL,
  -- The number that decided it, so changing a threshold later is auditable
  -- against findings that were already recorded.
  statistic      DOUBLE PRECISION NOT NULL,
  detail         TEXT NOT NULL,
  PRIMARY KEY (leakage_run_id, ordinal)
);
DROP TRIGGER IF EXISTS trg_leakage_finding_immutable ON mlops.leakage_finding;
CREATE TRIGGER trg_leakage_finding_immutable BEFORE UPDATE OR DELETE ON mlops.leakage_finding
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.leakage_finding ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.leakage_finding FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.leakage_finding;
CREATE POLICY tenant_isolation ON mlops.leakage_finding
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON dataplane.dataset_frame_digest, mlops.leakage_run, mlops.leakage_finding TO app_role;
REVOKE UPDATE, DELETE, TRUNCATE ON dataplane.dataset_frame_digest, mlops.leakage_run, mlops.leakage_finding FROM app_role;
GRANT SELECT ON mlops.leakage_run, mlops.leakage_finding TO internal_ml_role;

-- Tenants the leakage job must audit: anyone who has materialized a dataset.
CREATE OR REPLACE FUNCTION dataplane.dataset_tenants()
RETURNS SETOF TEXT
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = dataplane, pg_temp
AS $$
  SELECT DISTINCT tenant_id FROM dataplane.dataset_spec
$$;
REVOKE ALL ON FUNCTION dataplane.dataset_tenants() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION dataplane.dataset_tenants() TO app_role;
