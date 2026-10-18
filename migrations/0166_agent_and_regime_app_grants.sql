-- 0166 — let the platform reach the agent schemas and write regime labels
-- (ADR-P0-13; the same defect 0145 fixed for `monitor`).
--
-- 0044 moved the runtime pool onto `platform_app`, a member of `app_role`.
-- Migrations written after it that created a schema and granted nothing passed
-- every test — tests connect as the owner — and failed in the running platform
-- with `permission denied`:
--
--   * `agent_lifecycle` (0127–0137): preregistrations, hypotheses, designs,
--     proposals, campaign runs and sub-runs. No grant of any kind.
--   * `agent_memory` (0122–0125): the semantic memory entries. No grant.
--   * `regime_causal.regime_state` (0055, 0117): 0055 granted SELECT and INSERT,
--     but the rule-tier labeller (`api::knowledge`) upserts with
--     `ON CONFLICT ... DO UPDATE SET series_log_return = ...`, and an upsert
--     needs UPDATE on the column it sets even when it only ever inserts.
--     `regime_jobs` logged `permission denied for table regime_state` every run.
--
-- The grants are exactly what the Rust in this workspace does to each table,
-- read off its SQL, and nothing more: no DELETE anywhere, and UPDATE only where a
-- statement updates. Immutability triggers on these tables (campaign_run, for
-- one) still apply on top — a grant opens the door the trigger guards.
--
-- Each block is guarded by an existence check, because 0055 and 0122 need the
-- `vector` extension and a database without it never created these objects; a
-- grant on a missing relation would fail this migration for a reason that is
-- not this migration's.

DO $$
BEGIN
  -- ── agent_lifecycle ──────────────────────────────────────────────────────
  IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'agent_lifecycle') THEN
    GRANT USAGE ON SCHEMA agent_lifecycle TO app_role;

    IF to_regclass('agent_lifecycle.preregistration') IS NOT NULL THEN
      GRANT SELECT, INSERT ON agent_lifecycle.preregistration TO app_role;
    END IF;
    IF to_regclass('agent_lifecycle.design') IS NOT NULL THEN
      GRANT SELECT, INSERT ON agent_lifecycle.design TO app_role;
    END IF;
    IF to_regclass('agent_lifecycle.campaign_run') IS NOT NULL THEN
      GRANT SELECT, INSERT ON agent_lifecycle.campaign_run TO app_role;
    END IF;
    -- These three are updated in place (status, verdicts, completion).
    IF to_regclass('agent_lifecycle.hypothesis') IS NOT NULL THEN
      GRANT SELECT, INSERT, UPDATE ON agent_lifecycle.hypothesis TO app_role;
    END IF;
    IF to_regclass('agent_lifecycle.proposal') IS NOT NULL THEN
      GRANT SELECT, INSERT, UPDATE ON agent_lifecycle.proposal TO app_role;
    END IF;
    IF to_regclass('agent_lifecycle.sub_run') IS NOT NULL THEN
      GRANT SELECT, INSERT, UPDATE ON agent_lifecycle.sub_run TO app_role;
    END IF;
  END IF;

  -- ── agent_memory ─────────────────────────────────────────────────────────
  IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'agent_memory') THEN
    GRANT USAGE ON SCHEMA agent_memory TO app_role;
    IF to_regclass('agent_memory.entry') IS NOT NULL THEN
      GRANT SELECT, INSERT, UPDATE ON agent_memory.entry TO app_role;
    END IF;
  END IF;

  -- ── regime_causal.regime_state ───────────────────────────────────────────
  -- Column-level: the upsert sets `series_log_return` and nothing else, and the
  -- label itself stays immutable to the platform role.
  IF to_regclass('regime_causal.regime_state') IS NOT NULL
     AND EXISTS (SELECT 1 FROM information_schema.columns
                  WHERE table_schema = 'regime_causal'
                    AND table_name = 'regime_state'
                    AND column_name = 'series_log_return') THEN
    GRANT UPDATE (series_log_return) ON regime_causal.regime_state TO app_role;
  END IF;
END $$;
