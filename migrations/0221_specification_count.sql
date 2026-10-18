-- 0221 -- the Experiment's specification count (Set N N-1.6, BACKTEST_SUITE v2 section 2.3).
--
-- Every feature, transform or interaction a child dataset adds beyond its parent raises
-- the owning Experiment's specification count. It is reported and used in the protocol
-- checklist; it is not an automatic penalty. It only ever rises: a removed feature does
-- not give a degree of freedom back, because the search that found the removal already
-- spent it (`dataset_declaration::specification_delta` counts additions only).
--
-- The only writer is `experiment_add_specifications()`; a BEFORE UPDATE trigger refuses a
-- decrease from any role, so a hand edit cannot lower it either.

ALTER TABLE backtest_experiments
    ADD COLUMN IF NOT EXISTS specification_count INTEGER NOT NULL DEFAULT 0;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'chk_specification_count_nonneg'
  ) THEN
    ALTER TABLE backtest_experiments
        ADD CONSTRAINT chk_specification_count_nonneg CHECK (specification_count >= 0);
  END IF;
END $$;

CREATE OR REPLACE FUNCTION backtest_experiments_spec_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.specification_count < OLD.specification_count THEN
    RAISE EXCEPTION 'specification_count is monotonic (% -> %)',
      OLD.specification_count, NEW.specification_count USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS backtest_experiments_spec_guard ON backtest_experiments;
CREATE TRIGGER backtest_experiments_spec_guard
    BEFORE UPDATE ON backtest_experiments
    FOR EACH ROW EXECUTE FUNCTION backtest_experiments_spec_guard();

-- Returns the new count; an unknown experiment raises rather than silently counting nothing.
CREATE OR REPLACE FUNCTION experiment_add_specifications(p_experiment_id TEXT, p_n INTEGER)
RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
  total INTEGER;
BEGIN
  IF p_n IS NULL OR p_n < 0 THEN
    RAISE EXCEPTION 'specifications added must be non-negative, got %', p_n
      USING ERRCODE = 'check_violation';
  END IF;
  UPDATE backtest_experiments
     SET specification_count = specification_count + p_n, updated = now()
   WHERE experiment_id = p_experiment_id
   RETURNING specification_count INTO total;
  IF total IS NULL THEN
    RAISE EXCEPTION 'unknown experiment %', p_experiment_id USING ERRCODE = 'no_data_found';
  END IF;
  RETURN total;
END $$;

REVOKE ALL ON FUNCTION experiment_add_specifications(TEXT, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION experiment_add_specifications(TEXT, INTEGER) TO app_role;
