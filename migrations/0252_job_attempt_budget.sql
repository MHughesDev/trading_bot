-- The GPU ceiling holds across a re-queue (SPEC §8, AT-66).
--
-- Merged from claude/resume-producer-and-consumer, whose 0122_job_resume_budget.sql
-- this is: renumbered (0122 was already 0122_semantic_memory.sql, applied), and cut
-- to its budget half. The other half — `code_hashes`, `data_snapshot_id` and an
-- `artifacts` index for a claim-time resume read — served a design main did not
-- adopt: main resumes through `JobContext::resume_plan`, reading checkpoints the
-- worker records as `job_events`, not `artifacts`.
--
-- `max_gpu_hours` is a REQUIRED manifest field with no default, refused at
-- submission and enforced by a hard kill in the worker. Before this the kill was a
-- fresh `max_gpu_hours` timeout on every attempt, so a job that died at 0.9 of a
-- 1-hour ceiling and was re-queued got another full hour, and a job that dies
-- repeatedly an unbounded one. `gpu_seconds_spent` accumulates across attempts and
-- the worker's deadline is the remainder (`jobs::checkpoint::remaining_budget`).
--
-- The spend is charged by the **database**, from `attempt_started_at` and the
-- lease (`jobs::store::charge_attempt`), not reported by the worker. A worker that
-- was killed reports nothing, and that is precisely the attempt whose spend must
-- still be charged.

ALTER TABLE jobs
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
