-- 0182 — the fleet's ceilings (Set T T-2.1).
--
-- Three numbers bound how many paper deployments run and how much they hold. They
-- are declared, by a person, and **there are no rows here until somebody declares
-- them**: the deployment gate treats an undeclared ceiling as a refusal, never as
-- "unlimited" (`ledger::fleet_ceiling::CeilingError::Undeclared`). A seed row
-- with a generous number would be the silent default this table exists to avoid.
--
-- Append-only: raising a ceiling is a new row and the old one stays legible, so
-- "what was the bound when this deployment was approved" is answerable.

CREATE TABLE IF NOT EXISTS deployment_ceiling (
  ceiling_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name        TEXT NOT NULL
              CHECK (name IN ('max_concurrent', 'max_aggregate_capital', 'max_instrument_exposure')),
  value       NUMERIC(38,18) NOT NULL CHECK (value > 0),
  declared_by TEXT NOT NULL CHECK (length(trim(declared_by)) > 0),
  declared_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  note        TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_deployment_ceiling_current
  ON deployment_ceiling (name, declared_at DESC);

DROP TRIGGER IF EXISTS trg_deployment_ceiling_immutable ON deployment_ceiling;
CREATE TRIGGER trg_deployment_ceiling_immutable
  BEFORE UPDATE OR DELETE ON deployment_ceiling
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE OR REPLACE VIEW current_deployment_ceiling AS
SELECT DISTINCT ON (name) name, value, declared_by, declared_at
  FROM deployment_ceiling
 ORDER BY name, declared_at DESC, ceiling_id DESC;

GRANT SELECT, INSERT ON deployment_ceiling TO app_role;
GRANT SELECT ON current_deployment_ceiling TO app_role;
GRANT SELECT ON deployment_ceiling, current_deployment_ceiling TO agent_role;
