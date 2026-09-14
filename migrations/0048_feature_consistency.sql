-- Nightly backfill-and-diff (SPEC §3.3, INV-14).
--
-- The diff job runs as the restricted runtime role and, under FORCE RLS, sees one
-- tenant at a time. `serving_tenants()` reveals which tenants have serves and
-- nothing else. A serve is diffed once: the diff table's key already refuses a
-- second row per (serve_id, feature_id), and its rows are append-only.

CREATE OR REPLACE FUNCTION dataplane.serving_tenants()
RETURNS SETOF TEXT
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = dataplane, pg_temp
AS $$
  SELECT DISTINCT tenant_id FROM dataplane.feature_serving_log
$$;
REVOKE ALL ON FUNCTION dataplane.serving_tenants() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION dataplane.serving_tenants() TO app_role;

CREATE INDEX IF NOT EXISTS idx_consistency_diff_recent ON dataplane.feature_consistency_diff (tenant_id, diffed_at DESC);

DROP TRIGGER IF EXISTS trg_consistency_diff_immutable ON dataplane.feature_consistency_diff;
CREATE TRIGGER trg_consistency_diff_immutable BEFORE UPDATE OR DELETE ON dataplane.feature_consistency_diff
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();
REVOKE UPDATE, DELETE, TRUNCATE ON dataplane.feature_consistency_diff FROM app_role;
