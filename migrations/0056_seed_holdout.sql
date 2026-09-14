-- 0056 — the frozen seed holdout (SPEC §13, checklist 4.15, ADR-P4-02, AT-50).
--
-- The first 200 trials of a tenant, frozen at the moment the first internal
-- model is trained. A **prefix**, not a random sample: a prefix is reproducible
-- from the ledger by anyone, cannot be re-drawn if the first draw was
-- inconvenient, and is exactly the data the platform held before it began
-- learning from its own decisions.
--
-- One row per tenant, ever. The primary key says so and the append-only trigger
-- enforces it: a second freeze is a re-draw, and a re-draw is the thing freezing
-- exists to prevent.

CREATE TABLE IF NOT EXISTS mlops.seed_holdout (
  tenant_id     TEXT PRIMARY KEY,
  trial_ids     UUID[] NOT NULL CHECK (cardinality(trial_ids) >= 1),
  -- What the freeze was triggered by, so a reader can date it against the
  -- ledger rather than against a deployment.
  frozen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  frozen_for    TEXT NOT NULL CHECK (length(frozen_for) > 0),
  ledger_seq_at BIGINT NOT NULL CHECK (ledger_seq_at >= 0)
);

DROP TRIGGER IF EXISTS trg_seed_holdout_immutable ON mlops.seed_holdout;
CREATE TRIGGER trg_seed_holdout_immutable BEFORE UPDATE OR DELETE ON mlops.seed_holdout
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.seed_holdout ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.seed_holdout FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.seed_holdout;
CREATE POLICY tenant_isolation ON mlops.seed_holdout
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The internal-model trainer reads it to exclude it and writes it once. The
-- agent may not see which trials are held out: an agent that knows the holdout
-- is an agent that can avoid resembling it.
GRANT SELECT, INSERT ON mlops.seed_holdout TO app_role, internal_ml_role;
