-- 0106 — Gate 15's statistics join the closed vocabulary (SPEC §12.3, §12.6;
-- ADR-P2-31; Set T T-0.2).
--
-- 0057 established that a gate reads its inputs from `mlops.trial_statistic`
-- rather than from a job manifest, because a statistic the submitter supplies
-- is a statistic the submitter chose. Gate 15 is the gate where that matters
-- most: it is the only one whose evidence cannot be re-derived by running
-- anything again, so a self-reported `paper_signal_match` of 0.999 would be
-- both undetectable and fatal.
--
-- Six names, not four. The four §12.6 measurements are what the gate judges on;
-- the observation **count** and **window** are part of the verdict, not a
-- footnote, because a gate passed on four observations and a gate passed on
-- four hundred are different claims and a stack that renders them identically
-- is lying by omission.
--
-- `paper_observed_days` counts observed-knowledge rows only (D-T1). A
-- backfilled row is not evidence about the future however recent it is.

-- 0057 wrote the CHECK inline, so its name is whatever Postgres generated.
-- Dropping a guessed name with IF EXISTS would be a no-op that leaves the old,
-- narrower constraint in place — and the new names would then be rejected at
-- INSERT time, which reads as "the gate never finds its input", the exact
-- failure 0057 was written to prevent. So find it by what it constrains.
DO $$
DECLARE c TEXT;
BEGIN
  FOR c IN
    SELECT con.conname
      FROM pg_constraint con
      JOIN pg_class rel ON rel.oid = con.conrelid
      JOIN pg_namespace ns ON ns.oid = rel.relnamespace
     WHERE ns.nspname = 'mlops'
       AND rel.relname = 'trial_statistic'
       AND con.contype = 'c'
       AND pg_get_constraintdef(con.oid) LIKE '%cpcv_p05_sharpe%'
  LOOP
    EXECUTE format('ALTER TABLE mlops.trial_statistic DROP CONSTRAINT %I', c);
  END LOOP;
END
$$;

ALTER TABLE mlops.trial_statistic ADD CONSTRAINT trial_statistic_name_check
  CHECK (name IN (
    'cpcv_p05_sharpe',
    'walk_forward_sharpe',
    'walk_forward_regimes',
    'pbo',
    'deflated_sharpe',
    'permutation_p_value',
    'breakeven_cost_multiple',
    'prereg_hash_present',
    -- Gate 15 (§12.6), written by the reconciliation path from
    -- `mlops.forward_observation` rows whose source is 'observed'.
    'paper_signal_match',
    'paper_slippage_ratio',
    'paper_turnover_ratio',
    'paper_model_rejects',
    'paper_observed_days',
    'paper_observation_window_days'
  ));
