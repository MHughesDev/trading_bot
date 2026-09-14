-- 0057 — the statistics a trial produced (SPEC §12.3, ADR-P2-31).
--
-- Gates 5–8 read numbers the funnel already computes — the CPCV 5th-percentile
-- path Sharpe, the walk-forward median, the probability of backtest overfitting,
-- the deflated Sharpe. Until now those lived in memory for the length of one
-- funnel run and were rendered once.
--
-- The gate worker cannot accept them from its job manifest: a statistic the
-- submitter supplies is a statistic the submitter chose, and §12.7's
-- gate-hacking countermeasures mean nothing if `{"dsr": 0.99}` passes Gate 8.
-- So they go here, written by whatever computed them, read by the gate.
--
-- Append-only like everything else in `mlops`: a statistic is an observation,
-- and an observation that can be edited is an opinion. Re-computing writes a new
-- row and the newest wins, so the history of what a trial was believed to score
-- is legible rather than overwritten.

CREATE TABLE IF NOT EXISTS mlops.trial_statistic (
  statistic_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id    TEXT NOT NULL,
  trial_id     UUID NOT NULL REFERENCES mlops.trial(trial_id),
  -- A closed vocabulary, checked here rather than agreed by convention: a
  -- misspelled name is a statistic the gate silently never finds, and a gate
  -- that never finds its input is inconclusive forever without anybody noticing.
  name         TEXT NOT NULL CHECK (name IN (
                 'cpcv_p05_sharpe',
                 'walk_forward_sharpe',
                 'walk_forward_regimes',
                 'pbo',
                 'deflated_sharpe',
                 'permutation_p_value',
                 'breakeven_cost_multiple',
                 'prereg_hash_present'
               )),
  value        DOUBLE PRECISION NOT NULL CHECK (value = value),  -- NaN is not an observation
  -- Which study, run or artifact produced it. A number with no provenance is a
  -- number nobody can re-derive.
  produced_by  TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_trial_statistic_lookup
  ON mlops.trial_statistic (tenant_id, trial_id, name, recorded_at DESC);

DROP TRIGGER IF EXISTS trg_trial_statistic_immutable ON mlops.trial_statistic;
CREATE TRIGGER trg_trial_statistic_immutable BEFORE UPDATE OR DELETE ON mlops.trial_statistic
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.trial_statistic ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.trial_statistic FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.trial_statistic;
CREATE POLICY tenant_isolation ON mlops.trial_statistic
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The platform writes; the agent may read its own trials' statistics but not
-- write them. Writing one is how a candidate would hand the gate its own answer.
GRANT SELECT, INSERT ON mlops.trial_statistic TO app_role;
GRANT SELECT ON mlops.trial_statistic TO agent_role, internal_ml_role, backtest_role;
