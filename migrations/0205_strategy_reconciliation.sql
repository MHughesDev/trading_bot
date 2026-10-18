-- 0205 — the research-to-deploy reconciliation record (Set M M-4.1; FEAT-004 §7, ST-10).
--
-- A Layer 1 research idea earns a Layer 2 version only by reconciling: both run on
-- one window under one policy and costs, and the positions agree to a stated
-- fraction with a stated tracking error, every disagreement inside a declared
-- approximation class. This table is where that verdict lives.
--
-- It keys to `strategy_revision` (0058), the lineage record that is built, and not
-- to the `strategy_versions` table FEAT-004 §4 proposed and no migration ever
-- created (Q-G4-01, ADR-M-06). Both columns are foreign keys: a reconciliation
-- cannot name a revision the platform does not hold.
--
-- Three things are constraints:
--
--   * Append-only. A verdict is what it was when it was reached; a re-run under
--     other thresholds is another row.
--   * `passed` cannot contradict the thresholds the same row records. A row that
--     says `passed = true` beside an agreement under its own `agreement_min` is
--     refused, so the record cannot be made to lie by whoever wrote it.
--   * `passed` requires zero unexplained disagreements, however high the agreement.
--
-- Granted to `app_role` here, with no UPDATE or DELETE: the platform inserts a
-- verdict and reads it, and the trigger would refuse the rest anyway.

CREATE TABLE IF NOT EXISTS strategy_reconciliation (
  reconciliation_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  layer1_revision     TEXT NOT NULL REFERENCES strategy_revision (revision_hash),
  layer2_revision     TEXT NOT NULL REFERENCES strategy_revision (revision_hash),
  experiment_id       TEXT NOT NULL CHECK (experiment_id <> ''),
  n_decisions         INT  NOT NULL CHECK (n_decisions > 0),
  agreement           DOUBLE PRECISION NOT NULL CHECK (agreement BETWEEN 0 AND 1),
  pnl_tracking_error  DOUBLE PRECISION NOT NULL CHECK (pnl_tracking_error >= 0),
  -- {"agreement_min": 0.99, "te_max": 0.0001}; there is no default for either.
  thresholds          JSONB NOT NULL,
  explained_diffs     JSONB NOT NULL CHECK (jsonb_typeof(explained_diffs) = 'array'),
  unexplained_count   INT  NOT NULL CHECK (unexplained_count >= 0),
  passed              BOOLEAN NOT NULL,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

  CONSTRAINT recon_layers_differ CHECK (layer1_revision <> layer2_revision),
  -- COALESCE: a missing key makes jsonb_typeof NULL, and a CHECK that evaluates to
  -- NULL passes. Without it a row with no `te_max` would be accepted, and the
  -- consistency check below would compare against NULL and pass too.
  CONSTRAINT recon_thresholds_complete CHECK (
    COALESCE(jsonb_typeof(thresholds -> 'agreement_min'), '') = 'number'
    AND COALESCE(jsonb_typeof(thresholds -> 'te_max'), '') = 'number'
  ),
  CONSTRAINT recon_pass_is_consistent CHECK (
    NOT passed OR (
      unexplained_count = 0
      AND agreement >= (thresholds ->> 'agreement_min')::DOUBLE PRECISION
      AND pnl_tracking_error <= (thresholds ->> 'te_max')::DOUBLE PRECISION
    )
  ),
  -- The same pair, experiment and thresholds is one verdict; the same inputs give
  -- the same answer, so a second row would only be a way to record a different one.
  CONSTRAINT recon_one_verdict UNIQUE (layer1_revision, layer2_revision, experiment_id, thresholds)
);

CREATE INDEX IF NOT EXISTS idx_recon_layer2
  ON strategy_reconciliation (layer2_revision, created_at DESC);

CREATE OR REPLACE FUNCTION public.strategy_reconciliation_refuse_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION 'strategy_reconciliation is append-only: % refused', TG_OP;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_reconciliation_immutable ON strategy_reconciliation;
CREATE TRIGGER trg_strategy_reconciliation_immutable
  BEFORE UPDATE OR DELETE ON strategy_reconciliation
  FOR EACH ROW EXECUTE FUNCTION public.strategy_reconciliation_refuse_mutation();

-- 0044's default privileges would hand app_role UPDATE and DELETE on a new table
-- in `public`; this table is append-only, so take them back explicitly.
GRANT SELECT, INSERT ON strategy_reconciliation TO app_role;
REVOKE UPDATE, DELETE ON strategy_reconciliation FROM app_role;
