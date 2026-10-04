-- 0189 — one model per walk-forward fold (Set U U-5.8).
--
-- Training fitted one model on the first fold and recorded the version's
-- training end, so a backtest longer than that fold's window could only hold:
-- the point-in-time rule (trained_through + embargo < decision) refused every
-- decision before the training end, and the one model could not answer them.
-- A version now also keeps each fold's own model — trained only on the data
-- before that fold's test window — as a `model_fold` artifact carrying its
-- fold index and its own training end. A backtest decision is answered by the
-- latest model (a fold's, or the version's own) trained early enough; live
-- serving still uses the version's `model` artifact.

ALTER TABLE model_artifacts
    ADD COLUMN IF NOT EXISTS fold_index      INTEGER,
    ADD COLUMN IF NOT EXISTS trained_through TIMESTAMPTZ;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'model_artifacts_fold_is_complete'
  ) THEN
    -- A fold artifact names its fold and its training end; nothing else
    -- carries a fold index.
    ALTER TABLE model_artifacts ADD CONSTRAINT model_artifacts_fold_is_complete CHECK (
      (artifact_type = 'model_fold'
         AND fold_index IS NOT NULL AND fold_index >= 0 AND trained_through IS NOT NULL)
      OR (artifact_type <> 'model_fold' AND fold_index IS NULL)
    );
  END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS model_artifacts_one_per_fold
    ON model_artifacts (model_id, version, fold_index)
    WHERE artifact_type = 'model_fold';
