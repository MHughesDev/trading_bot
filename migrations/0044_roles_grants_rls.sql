-- Roles, grants and row-level security for the ML platform (SPEC §7.1, §14.3,
-- §15; INV-19, INV-23, S-2; AT-20, AT-29, AT-37, AT-38, AT-49).
--
-- The platform previously connected as the table-owning superuser, which bypasses
-- every GRANT and every RLS policy. From this migration on the runtime pool
-- connects as `platform_app` (a member of app_role), and migrations keep running as
-- the owner. There is no fallback to the owner at runtime (ADR-P0-13).
--
-- Group roles are NOLOGIN. `platform_app` is created NOLOGIN here and enabled with a
-- password at boot from PLATFORM_DB_APP_PASSWORD — a migration must not carry a
-- secret.

DO $$
DECLARE r TEXT;
BEGIN
  FOREACH r IN ARRAY ARRAY['app_role','agent_role','internal_ml_role','backtest_role'] LOOP
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r) THEN
      EXECUTE format('CREATE ROLE %I NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE', r);
    END IF;
  END LOOP;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'platform_app') THEN
    CREATE ROLE platform_app NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE IN ROLE app_role;
  END IF;
END $$;

-- ── public schema: the existing platform (trading engine, auth, UI state) ─────
GRANT USAGE ON SCHEMA public TO app_role;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO app_role;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO app_role;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO app_role;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO app_role;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO app_role;
-- sqlx's own bookkeeping is migration-only.
REVOKE ALL ON TABLE public._sqlx_migrations FROM app_role;

-- ── mlops: append-only ledger, read-only governance ───────────────────────────
REVOKE ALL ON ALL TABLES IN SCHEMA mlops FROM PUBLIC;
GRANT USAGE ON SCHEMA mlops TO app_role, agent_role, internal_ml_role;

GRANT SELECT, INSERT ON
  mlops.trial, mlops.trial_event, mlops.decision, mlops.ledger_anchor,
  mlops.sealed_holdout_call, mlops.sealed_holdout_attempt,
  mlops.agent_trajectory, mlops.trajectory_label, mlops.audit_event,
  mlops.campaign, mlops.campaign_event,
  mlops.model_promotion, mlops.internal_model_freeze, mlops.internal_model_registry,
  mlops.trial_return_series
TO app_role;
GRANT SELECT ON mlops.trial_return_series TO internal_ml_role;
GRANT SELECT ON mlops.gate_profile TO app_role;
REVOKE UPDATE, DELETE, TRUNCATE ON ALL TABLES IN SCHEMA mlops FROM app_role, agent_role, internal_ml_role;

-- Agents may read their campaigns and the gate profile, and log trajectories. They
-- hold nothing on governance: no campaign writes (delta_practical after DEFINE),
-- no gate profiles, no sealed holdout, no promotions (Tier C, INV-23).
GRANT SELECT ON mlops.campaign, mlops.gate_profile TO agent_role;
GRANT INSERT, SELECT ON mlops.agent_trajectory TO agent_role;

-- Internal-model training reads the ledger. It writes nothing that decides what
-- passes: no gate profiles, campaigns, holdout, or promotions (AT-49).
GRANT SELECT ON mlops.trial, mlops.trial_event, mlops.decision, mlops.agent_trajectory,
  mlops.trajectory_label, mlops.internal_model_registry, mlops.model_promotion,
  mlops.internal_model_freeze TO internal_ml_role;

-- The status view must evaluate RLS as the caller, not as its (superuser) owner.
CREATE OR REPLACE VIEW mlops.trial_state WITH (security_invoker = true) AS
SELECT t.trial_id, t.tenant_id, t.campaign_id, t.experiment_id, t.config_hash, t.policy_id,
       t.propensity, t.exploration_flag, t.registered_at,
       COALESCE(e.state, 'registered') AS state,
       COALESCE(e.censoring, 'none')   AS censoring,
       e.terminal_reason, e.run_id, e.deduplicated_of, e.outcome, e.gate_profile, e.gate_results,
       e.returns_uri, e.predictions_uri, e.occurred_at AS state_since,
       COALESCE(e.state IN ('rejected','deduplicated','completed','completed_pass','completed_fail','failed'), FALSE) AS terminal
FROM mlops.trial t
LEFT JOIN LATERAL (
  SELECT * FROM mlops.trial_event ev WHERE ev.trial_id = t.trial_id ORDER BY ev.event_seq DESC LIMIT 1
) e ON TRUE;
GRANT SELECT ON mlops.trial_state TO app_role, internal_ml_role;

-- ── RLS: FORCE (owners bypass without it), transaction-local context only ─────
-- A single restrictive-by-construction policy per table. Policies combine with OR,
-- so each table has exactly ONE permissive policy; `rls_policy_audit` asserts it.
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY[
    'trial','trial_event','decision','ledger_anchor','sealed_holdout_call','sealed_holdout_attempt',
    'agent_trajectory','trajectory_label','audit_event','campaign','campaign_event','trial_return_series'
  ] LOOP
    EXECUTE format('ALTER TABLE mlops.%I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE mlops.%I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON mlops.%I', t);
    EXECUTE format($p$
      CREATE POLICY tenant_isolation ON mlops.%I
        USING (tenant_id = current_setting('app.tenant_id', true))
        WITH CHECK (tenant_id = current_setting('app.tenant_id', true))
    $p$, t);
  END LOOP;
END $$;

-- Per-tenant internal models are tenant rows; global ones are platform rows visible
-- to everyone. Writes to the registry are platform operations, not tenant ones.
ALTER TABLE mlops.internal_model_registry ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.internal_model_registry FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.internal_model_registry;
CREATE POLICY tenant_isolation ON mlops.internal_model_registry
  USING (tenant_id IS NULL OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id IS NULL OR tenant_id = current_setting('app.tenant_id', true));

-- Every tenant table must carry exactly one permissive policy and FORCE RLS.
CREATE OR REPLACE VIEW mlops.rls_policy_audit AS
SELECT c.relname AS table_name,
       c.relrowsecurity AS rls_enabled,
       c.relforcerowsecurity AS rls_forced,
       (SELECT count(*) FROM pg_policy p WHERE p.polrelid = c.oid AND p.polpermissive) AS permissive_policies
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = 'mlops' AND c.relkind = 'r'
  AND EXISTS (SELECT 1 FROM information_schema.columns col
              WHERE col.table_schema = 'mlops' AND col.table_name = c.relname AND col.column_name = 'tenant_id');
GRANT SELECT ON mlops.rls_policy_audit TO app_role;
