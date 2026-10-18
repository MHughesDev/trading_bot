-- 0185 — what a training run needs in order to be picked up again after the
-- platform dies (SPEC §9, checklist 2.4/2.5, ADR-P2-07; OPEN-TASKS C1.2).
--
-- Training runs as an in-process task around one HTTP call to the trainer. When
-- the platform restarts, the task is gone and the run row says `running` for
-- ever. Two things were missing before a run could be re-driven:
--
--   * `training_run_spec` — the request that started the run, with the data
--     window *resolved to instants*. The window is "now minus N days" at request
--     time, so a plan rebuilt after a restart without it would name different
--     data, a different dataset id, and `plan_resume` would (correctly) refuse
--     every checkpoint as belonging to another computation.
--   * `training_checkpoint` — the manifests the trainer publishes mid-run. Append
--     only; the newest is what `jobs::checkpoint::plan_resume` judges.
--
-- Both are written by the platform role and never edited.

CREATE TABLE IF NOT EXISTS mlops.training_run_spec (
  run_id          UUID PRIMARY KEY,
  model_id        TEXT NOT NULL,
  user_id         UUID NOT NULL,
  -- The trial this run is a look at. Recovery resumes *this* trial: the trial is
  -- the look, not the process.
  trial_id        UUID NOT NULL REFERENCES mlops.trial(trial_id),
  hyperparameters JSONB,
  data            JSONB,
  version_note    TEXT,
  window_start    TIMESTAMPTZ NOT NULL,
  window_end      TIMESTAMPTZ NOT NULL,
  recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (window_end > window_start)
);

CREATE TABLE IF NOT EXISTS mlops.training_checkpoint (
  id          BIGSERIAL PRIMARY KEY,
  run_id      UUID NOT NULL,
  step        BIGINT NOT NULL CHECK (step >= 0),
  manifest    JSONB NOT NULL CHECK (jsonb_typeof(manifest) = 'object'),
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_training_checkpoint_run
  ON mlops.training_checkpoint (run_id, id DESC);

-- Granted here, in the tables' own migration. Owner-connected tests cannot see a
-- missing grant; `pg_training_resume` checks them as app_role.
GRANT SELECT, INSERT ON mlops.training_run_spec, mlops.training_checkpoint TO app_role;
GRANT USAGE, SELECT ON SEQUENCE mlops.training_checkpoint_id_seq TO app_role;
REVOKE UPDATE, DELETE, TRUNCATE ON mlops.training_run_spec, mlops.training_checkpoint
  FROM app_role, agent_role, internal_ml_role;
