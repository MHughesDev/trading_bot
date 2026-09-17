-- 0130 — a falsifier may name a metric the ledger can actually store
-- (AGENT-013 §6.2; Set S task S-10.4).
--
-- ## The defect
--
-- `lifecycle.hypothesise` validates a falsifier against a metric vocabulary, and
-- `routes/lifecycle.rs` describes that list as "the columns
-- `mlops.trial_statistic` records". It is not. The two lists were **disjoint**:
--
--   vocabulary   sharpe_after_costs, sortino_after_costs, max_drawdown,
--                hit_rate, profit_factor, turnover, exposure, calmar,
--                information_ratio, tail_ratio
--   the table    cpcv_p05_sharpe, walk_forward_sharpe, walk_forward_regimes,
--                pbo, deflated_sharpe, permutation_p_value,
--                breakeven_cost_multiple, prereg_hash_present, (+ gate 15)
--
-- Not one name in common. So every falsifier the platform accepted named a
-- statistic the ledger's CHECK refuses, `evaluate` would find no measurement for
-- any of them, and every hypothesis would resolve `inconclusive:
-- insufficient_data` forever — while the registration hashed cleanly, the gate
-- stamped it on every ticket, and nothing anywhere failed.
--
-- The comment in `routes/lifecycle.rs` names a test —
-- `the_metric_vocabulary_is_the_statistic_table` — as the guard that checks the
-- copy against the schema. **That test was never written.** Had it existed it
-- would have failed on the day the vocabulary was added. It exists now.
--
-- ## What changes
--
-- The table's vocabulary gains the objective metrics, spelled exactly as
-- `backtest::run::metrics::MetricSet`'s own fields. Spelled that way on purpose:
-- the funnel records them straight off the struct, so there is no mapping
-- between a metric's name and its value for anybody to get wrong — and
-- `sharpe_after_costs` is gone as a name, because the mapping it implied
-- (`MetricSet::sharpe`, which is already net of costs) was exactly such a
-- mapping, written in prose and nowhere else.
--
-- The gate statistics are untouched. A falsifier still may not name one: those
-- are the gates' own inputs, and a hypothesis whose falsifier named `pbo` would
-- be a run pre-registering against a threshold, which INV-23 forbids.

-- 0106's lesson, reused: 0057 wrote its CHECK inline and 0106 renamed it, so the
-- constraint's name depends on which migrations a database has seen. Dropping a
-- guessed name with IF EXISTS is a no-op that leaves the narrower constraint in
-- place, and the new names are then rejected at INSERT — which reads as "the
-- statistic was never recorded", the exact failure 0057 exists to prevent. Find
-- it by what it constrains.
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

ALTER TABLE mlops.trial_statistic ADD CONSTRAINT trial_statistic_name_check CHECK (
  name IN (
    -- gate statistics (0057, 0106): the gates' own inputs.
    'cpcv_p05_sharpe',
    'walk_forward_sharpe',
    'walk_forward_regimes',
    'pbo',
    'deflated_sharpe',
    'permutation_p_value',
    'breakeven_cost_multiple',
    'prereg_hash_present',
    'paper_signal_match',
    'paper_slippage_ratio',
    'paper_turnover_ratio',
    'paper_model_rejects',
    'paper_observed_days',
    'paper_observation_window_days',
    -- objective metrics (0130): what a falsifier may name.
    --
    -- Spelled as `MetricSet`'s own fields, so the funnel records them straight
    -- off the struct and there is no name-to-value mapping for anybody to get
    -- wrong. All are net of modelled costs, because a backtest applies costs
    -- before it measures — which is what `sharpe_after_costs` was trying to say
    -- in a name, and saying it in a name is what made it a mapping that lived
    -- only in prose.
    'sharpe',
    'sortino',
    'calmar',
    'information_ratio',
    'detrended_sharpe',
    'cagr',
    'total_return',
    'ann_vol',
    'max_drawdown',
    'cvar_95',
    'ulcer_index',
    'hit_rate',
    'profit_factor',
    'expectancy',
    'turnover',
    'exposure_net'
  )
);
