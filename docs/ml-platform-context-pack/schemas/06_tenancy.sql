-- §7 Multi-tenancy. Isolation by construction. [INV-24, S-2]

-- ---------------------------------------------------------------------------
-- 1. RLS. Three silent killers, all handled here.
-- ---------------------------------------------------------------------------
-- (a) Table OWNERS bypass RLS unless FORCE is set. The migration role owns tables.
-- (b) Bare SET bleeds tenant context across pooled connections. Use SET LOCAL only.
-- (c) Policies combine with OR, not AND. Adding a policy WIDENS access.

DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY[
    'dataset_spec','campaign','trial','decision','agent_trajectory',
    'feature_serving_log','outcome_tensor_fact','tensor_model',
    'recommendation','insight','asset_cluster','ledger_anchor'
  ] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);   -- (a)
    EXECUTE format($f$
      CREATE POLICY tenant_isolation ON %I
        USING (tenant_id = current_setting('app.tenant_id')::BIGINT)
        WITH CHECK (tenant_id = current_setting('app.tenant_id')::BIGINT)
    $f$, t);
  END LOOP;
END $$;

-- Application MUST use:  SET LOCAL app.tenant_id = '<id>';      -- (b)
-- Bare SET is a cross-tenant leak under PgBouncer transaction mode.
-- AT-37 runs the isolation suite THROUGH the pooler AS the owning role.

-- ---------------------------------------------------------------------------
-- 2. Ledger immutability. [INV-19]
-- ---------------------------------------------------------------------------
REVOKE UPDATE, DELETE ON trial            FROM app_role;
REVOKE UPDATE, DELETE ON decision         FROM app_role;
REVOKE UPDATE, DELETE ON agent_trajectory FROM app_role;
REVOKE UPDATE, DELETE ON ledger_anchor    FROM app_role;
GRANT  INSERT, SELECT  ON trial, decision, agent_trajectory, ledger_anchor TO app_role;

-- ---------------------------------------------------------------------------
-- 3. Regime causality. Enforced by grant, not by review. [R-07, INV-23]
-- ---------------------------------------------------------------------------
GRANT USAGE  ON SCHEMA regime_causal TO backtest_role;
GRANT SELECT ON ALL TABLES IN SCHEMA regime_causal TO backtest_role;
REVOKE ALL   ON SCHEMA regime_research FROM backtest_role;
REVOKE ALL   ON ALL TABLES IN SCHEMA regime_research FROM backtest_role;
-- AT-42 attempts the read as backtest_role and asserts denial.

-- ---------------------------------------------------------------------------
-- 4. Tier C: agents and internal-model training cannot touch governance. [INV-23]
-- ---------------------------------------------------------------------------
CREATE TABLE gate_profile (
  profile_id   TEXT PRIMARY KEY,              -- 'strict_v1' -- immutable
  thresholds   JSONB NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL,
  created_by   TEXT NOT NULL
);
REVOKE UPDATE, DELETE ON gate_profile FROM app_role, agent_role, internal_ml_role;
GRANT  SELECT ON gate_profile TO app_role, agent_role, internal_ml_role;
-- Changing a threshold creates strict_v2. Comparisons spanning profiles are flagged
-- non-comparable. [AT-29]

REVOKE ALL ON campaign FROM agent_role;       -- delta_practical is set at DEFINE only
GRANT  SELECT ON campaign TO agent_role;
REVOKE ALL ON sealed_holdout_call FROM agent_role;

-- ---------------------------------------------------------------------------
-- 5. The feature firewall. A whitelist you can unit-test. [INV-24, AT-36]
-- ---------------------------------------------------------------------------
CREATE TABLE internal_model_registry (
  model_id       TEXT PRIMARY KEY,            -- 'M5', 'M8', ...
  scope          TEXT NOT NULL CHECK (scope IN ('global','hierarchical','per_tenant')),
  tier           CHAR(1) NOT NULL CHECK (tier IN ('A','B','C')),
  feature_list   TEXT[] NOT NULL,
  champion_version TEXT,
  frozen_holdout_trial_ids UUID[] NOT NULL,   -- never in any training set [AT-50]
  policy_entropy DOUBLE PRECISION,            -- SLO with a hard floor [AT-51]
  entropy_floor  DOUBLE PRECISION NOT NULL,
  last_trained_at TIMESTAMPTZ,
  promoted_at     TIMESTAMPTZ,
  promoted_by     TEXT                        -- NULL only permitted for tier A
);

CREATE OR REPLACE VIEW firewall_violations AS
SELECT m.model_id, f.feature_id, f.info_class
FROM internal_model_registry m
CROSS JOIN LATERAL unnest(m.feature_list) AS fl(feature_id)
JOIN feature_def f ON f.feature_id = fl.feature_id
WHERE m.scope IN ('global','hierarchical')
  AND f.info_class NOT IN ('platform_physics','methodology','market_public');
-- AT-36 asserts this view is empty. It fails the build otherwise.

-- M8 may never be global. [§7.4, AT-41]
ALTER TABLE internal_model_registry ADD CONSTRAINT m8_per_tenant
  CHECK (model_id <> 'M8' OR scope = 'per_tenant');

-- Tier B and C require a named human promoter. [§14.3]
ALTER TABLE internal_model_registry ADD CONSTRAINT tier_bc_requires_approver
  CHECK (tier = 'A' OR promoted_at IS NULL OR promoted_by IS NOT NULL);

-- ---------------------------------------------------------------------------
-- 6. Object storage. The credential cannot name the bucket. [§7.1, AT-39]
-- ---------------------------------------------------------------------------
-- Shared-plane role: read s3://market-data/**  only.
-- Tenant role:      read/write s3://tenant-<id>/**  only.
-- No role holds both. Enforced in IAM, asserted by AT-39 at the credential layer.
