-- 0072 — trajectories and model tapes (AGENT-009 §2, §6; Set S task S-4.8).
--
-- A trajectory is the ordered `(Input, Vec<Effect>)` pairs a `Loop` produced,
-- plus the profile, the registry hash, the task and the per-step state hash.
-- Because the loop is sans-IO, that is a COMPLETE description of a run: feeding
-- the inputs back to the same loop version must produce the same effects, on a
-- machine with no accelerator and no network.
--
-- ── Why `registry_hash` is NOT NULL ────────────────────────────────────────
--
-- A replay against a different catalogue is measuring the catalogue. The whole
-- premise of a replay suite is that the only thing that changed is the harness,
-- and a trajectory that cannot say which catalogue it ran against cannot support
-- that claim. `is_replayable` refuses one without it.
--
-- ── Why the tape is separate ───────────────────────────────────────────────
--
-- A tape is the input half alone — in practice the raw model replies — and §2
-- says it is hand-editable, because that is how a case is CONSTRUCTED: a
-- truncated reply, a wrong tool, a fake id. Storing it as its own document means
-- a case author edits a tape rather than surgically rewriting a trajectory and
-- hoping the effects still line up.

CREATE TABLE IF NOT EXISTS agent_trajectory (
  trajectory_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  run_id          UUID REFERENCES agent_runs(run_id) ON DELETE CASCADE,
  profile_id      TEXT NOT NULL CHECK (length(profile_id) > 0),
  registry_hash   TEXT NOT NULL CHECK (length(registry_hash) > 0),
  harness_version TEXT NOT NULL CHECK (length(harness_version) > 0),
  task            JSONB NOT NULL,
  -- The ordered (input, effects, state, exposure) records.
  steps           JSONB NOT NULL,
  -- The input half, extracted. Editable; the case files are written from it.
  tape            JSONB NOT NULL,
  -- `trace::is_replayable` at record time. Recorded rather than recomputed so a
  -- nightly sample can ask "how many of yesterday's runs replay" without
  -- replaying them (AGENT-009 §8.1).
  replayable      BOOLEAN NOT NULL DEFAULT FALSE,
  unreplayable_why TEXT,
  CONSTRAINT an_unreplayable_trajectory_says_why CHECK (
    replayable OR unreplayable_why IS NOT NULL
  ),
  recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_trajectory_nightly_sample
  ON agent_trajectory (recorded_at DESC, replayable);
CREATE INDEX IF NOT EXISTS idx_trajectory_profile
  ON agent_trajectory (profile_id, recorded_at DESC);

-- ── Shadow replay (§6) ─────────────────────────────────────────────────────
--
-- Nightly, the day's real trajectories are replayed through the candidate
-- harness version. This is the non-inferiority check on the distribution that
-- actually matters — the user's own questions — at zero model cost.
--
-- Differences are CLASSIFIED, not counted. A candidate that reorders two
-- independent reads has not regressed, and one that drops a tool call has;
-- reporting "3 differences" makes those the same event and guarantees the report
-- is ignored within a month.
CREATE TABLE IF NOT EXISTS agent_shadow_diff (
  diff_id         BIGSERIAL PRIMARY KEY,
  trajectory_id   UUID NOT NULL REFERENCES agent_trajectory(trajectory_id) ON DELETE CASCADE,
  candidate_version TEXT NOT NULL,
  severity        TEXT NOT NULL CHECK (severity IN
                    ('neutral','cost','routing_regression','blocking')),
  detail          TEXT NOT NULL CHECK (length(detail) > 0),
  -- A blocking difference on a real session stops promotion unless a human
  -- accepts it WITH A NOTE. The note is required, not optional: an acceptance
  -- with no reason is indistinguishable from nobody having looked.
  accepted_by     UUID REFERENCES users(user_id),
  accepted_note   TEXT,
  CONSTRAINT an_acceptance_carries_its_reason CHECK (
    (accepted_by IS NULL) = (accepted_note IS NULL)
  ),
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_shadow_diff_blocking
  ON agent_shadow_diff (candidate_version, severity)
  WHERE severity = 'blocking' AND accepted_by IS NULL;

GRANT SELECT, INSERT, UPDATE ON agent_trajectory, agent_shadow_diff TO app_role;
GRANT USAGE, SELECT ON SEQUENCE agent_shadow_diff_diff_id_seq TO app_role;
GRANT SELECT ON agent_trajectory, agent_shadow_diff TO internal_ml_role;
