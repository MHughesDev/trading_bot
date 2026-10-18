-- 0208 -- the model trial counter (Set N N-2.10, MD-05, FEAT-006 section 9).
--
-- Every training run and every HPO trial is a model specification tried, and the
-- deflated scores (N-3.5) divide by how many were tried. A counter that anyone
-- can write is a counter that can be lowered, so:
--
--   * `trial_count` is monotonic (a BEFORE UPDATE trigger refuses a decrease, for
--     every role including the owner);
--   * the only writer is `model_experiment_count()`, SECURITY DEFINER, and the
--     platform role has SELECT on the table and EXECUTE on the function and no
--     UPDATE, so a direct UPDATE by the platform is refused by privilege;
--   * a training-run INSERT calls the function from a trigger, so the count rises
--     in the same transaction as the job insert and cannot be skipped.

CREATE TABLE IF NOT EXISTS model_experiments (
    model_id         TEXT PRIMARY KEY REFERENCES ai_models(model_id) ON DELETE CASCADE,
    trial_count      BIGINT NOT NULL DEFAULT 0 CHECK (trial_count >= 0),
    hpo_trial_count  BIGINT NOT NULL DEFAULT 0 CHECK (hpo_trial_count >= 0),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION model_experiment_monotonic() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.trial_count < OLD.trial_count OR NEW.hpo_trial_count < OLD.hpo_trial_count THEN
    RAISE EXCEPTION 'model_experiments.trial_count is monotonic: % -> % refused',
      OLD.trial_count, NEW.trial_count USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS model_experiments_monotonic ON model_experiments;
CREATE TRIGGER model_experiments_monotonic
    BEFORE UPDATE ON model_experiments
    FOR EACH ROW EXECUTE FUNCTION model_experiment_monotonic();

-- The counting hook. `p_hpo` marks the trials as hyper-parameter-search trials,
-- which count toward `trial_count` exactly as a run does and are also kept apart.
CREATE OR REPLACE FUNCTION model_experiment_count(p_model_id TEXT, p_n BIGINT, p_hpo BOOLEAN DEFAULT FALSE)
RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
  total BIGINT;
BEGIN
  IF p_n IS NULL OR p_n < 1 THEN
    RAISE EXCEPTION 'a trial count increment must be at least 1, got %', p_n
      USING ERRCODE = 'check_violation';
  END IF;
  INSERT INTO model_experiments AS e (model_id, trial_count, hpo_trial_count)
  VALUES (p_model_id, p_n, CASE WHEN p_hpo THEN p_n ELSE 0 END)
  ON CONFLICT (model_id) DO UPDATE
     SET trial_count     = e.trial_count + p_n,
         hpo_trial_count = e.hpo_trial_count + CASE WHEN p_hpo THEN p_n ELSE 0 END,
         updated_at      = now()
  RETURNING e.trial_count INTO total;
  RETURN total;
END $$;

REVOKE ALL ON FUNCTION model_experiment_count(TEXT, BIGINT, BOOLEAN) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION model_experiment_count(TEXT, BIGINT, BOOLEAN) TO app_role;
-- 0044's default privileges hand app_role full DML on every new table, so it is
-- revoked by name, not just from PUBLIC.
REVOKE INSERT, UPDATE, DELETE ON model_experiments FROM PUBLIC, app_role;
GRANT SELECT ON model_experiments TO app_role;

-- A training run is one trial, counted in the same transaction as its insert.
-- SECURITY DEFINER: the inserting role has no right to the counter, only to the
-- fact that a run was inserted.
CREATE OR REPLACE FUNCTION training_run_counts_a_trial() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
  PERFORM model_experiment_count(NEW.model_id, 1, FALSE);
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS training_runs_count_a_trial ON training_runs;
CREATE TRIGGER training_runs_count_a_trial
    AFTER INSERT ON training_runs
    FOR EACH ROW EXECUTE FUNCTION training_run_counts_a_trial();
