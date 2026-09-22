-- Track 18 (2.5, ADR-P2-07, ADR-P6-12): the two things a re-queued job needs
-- before `jobs::checkpoint::plan_resume` can be asked a truthful question.
--
-- 1. **The computation's identity.** A checkpoint is only resumable into the
--    same code, the same image and the same dataset — `plan_resume` compares all
--    three and restarts when any differs. The job row carried none of them: the
--    code hashes and the data snapshot went into `manifest_hash` at submission
--    and were then unrecoverable, so a worker had nothing to compare a stored
--    checkpoint against. Two hashes that are equal does not tell you what they
--    were hashed from.
--
-- 2. **What the killed attempt already spent.** `max_gpu_hours` is a REQUIRED
--    manifest field with no default, refused at submission and enforced by a
--    hard kill in the worker. Before this the kill was a fresh `max_gpu_hours`
--    timeout on every attempt, so a job that died at 0.9 of a 1-hour ceiling and
--    was re-queued got another full hour. Resume would have turned a hard cap
--    into a per-attempt allowance, and a job that dies repeatedly into an
--    unbounded one. `gpu_seconds_spent` accumulates across attempts and the
--    worker's deadline is the remainder.
--
-- The spend is charged by the **database**, from `attempt_started_at` and the
-- lease, not reported by the worker. A worker that was killed reports nothing,
-- and that is precisely the attempt whose spend must still be charged.

ALTER TABLE jobs
  -- The code hashes and data snapshot the submission declared. Defaulted for
  -- existing rows: an empty identity is a fact about a row written before this
  -- column existed, and it makes `plan_resume` restart rather than resume, which
  -- is the correct answer for a checkpoint whose provenance nobody recorded.
  ADD COLUMN IF NOT EXISTS code_hashes       TEXT[] NOT NULL DEFAULT '{}',
  ADD COLUMN IF NOT EXISTS data_snapshot_id  TEXT   NOT NULL DEFAULT '',
  -- When the current attempt was claimed. NULL between attempts.
  ADD COLUMN IF NOT EXISTS attempt_started_at TIMESTAMPTZ,
  -- Wall-clock seconds this job has consumed across every attempt. Never
  -- decreases; the CHECK says so rather than trusting every future writer.
  ADD COLUMN IF NOT EXISTS gpu_seconds_spent DOUBLE PRECISION NOT NULL DEFAULT 0;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'jobs_gpu_seconds_spent_nonneg'
  ) THEN
    ALTER TABLE jobs
      ADD CONSTRAINT jobs_gpu_seconds_spent_nonneg
      CHECK (gpu_seconds_spent >= 0);
  END IF;
END $$;

-- A re-claimed job asks for its newest checkpoint on every claim, inside the
-- claiming transaction. Without this it is a sequential scan of `artifacts`
-- while a row is locked.
CREATE INDEX IF NOT EXISTS artifacts_checkpoint_by_job
  ON artifacts (producer_job, created_at DESC)
  WHERE type = 'checkpoint';
