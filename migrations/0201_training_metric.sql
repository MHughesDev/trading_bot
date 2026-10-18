-- 0201 - per-epoch training metrics, kept (Set H H-5.5).
--
-- The trainer publishes `{epoch, train_loss, val_loss}` on every progress frame,
-- and the platform merged each frame over the last one in memory, so the run
-- snapshot only ever held the latest epoch and a restart held none. The
-- Training Console draws loss curves, which need the history: one row per
-- (run, epoch), appended as the frames arrive and never edited.
--
-- A loss the trainer did not measure (no validation split) is NULL, not 0.

CREATE TABLE IF NOT EXISTS mlops.training_metric (
  run_id      UUID NOT NULL,
  epoch       INTEGER NOT NULL CHECK (epoch >= 0),
  train_loss  DOUBLE PRECISION,
  val_loss    DOUBLE PRECISION,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (run_id, epoch)
);

-- Granted here, in the table's own migration; owner-connected tests cannot see a
-- missing grant, so `pg_training_curve` checks them as app_role.
GRANT SELECT, INSERT ON mlops.training_metric TO app_role;
REVOKE UPDATE, DELETE, TRUNCATE ON mlops.training_metric
  FROM app_role, agent_role, internal_ml_role;
