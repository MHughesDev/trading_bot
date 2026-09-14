-- Ledger fixation jobs (SPEC §4.6, §16.2; INV-19).
--
-- The daily anchor and the chain verifier run as the restricted runtime role, which
-- under FORCE RLS can see one tenant at a time and therefore cannot discover which
-- tenants exist. `ledger_tenants()` reveals tenant ids and nothing else.
--
-- Every verification pass is recorded, pass or fail, append-only: a verifier whose
-- failures leave no trace is a verifier nobody can prove ran.

CREATE OR REPLACE FUNCTION mlops.ledger_tenants()
RETURNS SETOF TEXT
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = mlops, pg_temp
AS $$
  SELECT tenant_id FROM mlops.trial
  UNION SELECT tenant_id FROM mlops.decision
  UNION SELECT tenant_id FROM mlops.campaign
  UNION SELECT tenant_id FROM mlops.ledger_anchor
$$;
REVOKE ALL ON FUNCTION mlops.ledger_tenants() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION mlops.ledger_tenants() TO app_role;

CREATE TABLE IF NOT EXISTS mlops.ledger_verification (
  verification_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       TEXT NOT NULL,
  verified_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  ok              BOOLEAN NOT NULL,
  trials          BIGINT NOT NULL,
  events          BIGINT NOT NULL,
  decisions       BIGINT NOT NULL,
  anchors         BIGINT NOT NULL,
  failures        JSONB NOT NULL CHECK (jsonb_typeof(failures) = 'array'),
  CONSTRAINT chk_verification_ok CHECK (ok = (jsonb_array_length(failures) = 0))
);
CREATE INDEX IF NOT EXISTS idx_ledger_verification_recent ON mlops.ledger_verification(tenant_id, verified_at DESC);
DROP TRIGGER IF EXISTS trg_ledger_verification_immutable ON mlops.ledger_verification;
CREATE TRIGGER trg_ledger_verification_immutable BEFORE UPDATE OR DELETE ON mlops.ledger_verification
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

REVOKE ALL ON mlops.ledger_verification FROM PUBLIC;
GRANT SELECT, INSERT ON mlops.ledger_verification TO app_role;
GRANT SELECT ON mlops.ledger_verification TO internal_ml_role;

ALTER TABLE mlops.ledger_verification ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.ledger_verification FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.ledger_verification;
CREATE POLICY tenant_isolation ON mlops.ledger_verification
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));
