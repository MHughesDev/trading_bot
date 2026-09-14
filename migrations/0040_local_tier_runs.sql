-- Local tier: run-scoped approvals and the harness facts a run was decided on.
--
-- D-18 / ADR-0032. Two changes, both small, both about making a run's tier and its
-- outcome legible after the fact.

-- ── 1. Approvals reach the internal agent, not only project sessions ────────
--
-- `approval_requests` was project-scoped because the only thing that asked for an
-- approval was a research session. The GOVERNOR loop's `AwaitingApproval` state is a
-- first-class loop state at every tier, so a plain `agent_runs` run needs the same
-- row — and it needs the SAME table, so the approvals UI keeps one queue rather than
-- growing a second one nobody looks at.
ALTER TABLE approval_requests
  ADD COLUMN IF NOT EXISTS run_id UUID REFERENCES agent_runs(run_id) ON DELETE CASCADE;

ALTER TABLE approval_requests
  ALTER COLUMN project_id DROP NOT NULL;

-- An approval belonging to nothing could never be surfaced or answered.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'approval_requests_scoped'
  ) THEN
    ALTER TABLE approval_requests
      ADD CONSTRAINT approval_requests_scoped
      CHECK (project_id IS NOT NULL OR run_id IS NOT NULL);
  END IF;
END $$;

CREATE INDEX IF NOT EXISTS approval_requests_run
  ON approval_requests (run_id, state, created_at);

-- ── 2. What the run was admitted as ────────────────────────────────────────
--
-- Recorded on the run rather than inferred from the model name later. The tier a run
-- executed at changes how its results should be read, and "which profile was this?"
-- is not answerable from `model` alone once a profile has been edited.
ALTER TABLE agent_runs ADD COLUMN IF NOT EXISTS profile_id TEXT;
ALTER TABLE agent_runs ADD COLUMN IF NOT EXISTS tier TEXT;
ALTER TABLE agent_runs ADD COLUMN IF NOT EXISTS hardware TEXT;

-- The typed terminal state from `harness::drive::Outcome`. `summary` stays the prose
-- for a human; this is the machine-readable half, and it is the only place a fence is
-- distinguishable from an ordinary failure.
ALTER TABLE agent_runs ADD COLUMN IF NOT EXISTS outcome_json JSONB;

-- ── 3. A scorecard records the tier it was earned on ───────────────────────
--
-- ADR-0032 says promotion from `local_mid` to `local_high` is an eval result, not a
-- config edit. That is only mechanically true if a scorecard says which tier and
-- which profile produced it: comparing a `local_mid` run against a `frontier`
-- baseline, or against a scorecard earned on different hardware, measures the
-- hardware and reports it as a harness change.
ALTER TABLE eval_scorecards ADD COLUMN IF NOT EXISTS profile_id TEXT;
ALTER TABLE eval_scorecards ADD COLUMN IF NOT EXISTS tier TEXT;
ALTER TABLE eval_scorecards ADD COLUMN IF NOT EXISTS hardware TEXT;
