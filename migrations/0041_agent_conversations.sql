-- Conversations: the agent becomes a chat, not a form.
--
-- Before this, one run was one goal typed into a form with an instrument, a
-- timeframe, an iteration cap and a time budget. The user was configuring the agent.
-- Now the user only prompts; the agent chooses the instrument, how many backtests to
-- run and when it is finished.
--
-- A conversation holds many turns. A turn is still an `agent_runs` row — the run is
-- the unit of *agent work*, and keeping it means the timeline, the approvals and the
-- typed outcome all keep working unchanged.

CREATE TABLE IF NOT EXISTS agent_conversations (
  conversation_id  UUID PRIMARY KEY,
  user_id          UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,

  -- NULL until the summariser has run. The UI shows the first prompt truncated in
  -- the meantime rather than a placeholder, so the row is never anonymous.
  title            TEXT,

  provider         TEXT NOT NULL,
  model            TEXT NOT NULL,

  -- The agent's own folder. One per conversation, not per run: an agent that loses
  -- its notes between turns is an agent with no memory of its own work.
  workspace_path   TEXT,

  created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- Drives the sidebar order. Bumped on every prompt and every completion, so a
  -- conversation that is working sorts above one that is merely recent.
  last_activity_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  archived_at      TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS agent_conversations_user
  ON agent_conversations (user_id, last_activity_at DESC);

-- ── A run becomes a turn in a conversation ─────────────────────────────────
ALTER TABLE agent_runs
  ADD COLUMN IF NOT EXISTS conversation_id UUID
  REFERENCES agent_conversations(conversation_id) ON DELETE CASCADE;

ALTER TABLE agent_runs ADD COLUMN IF NOT EXISTS turn_index INT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS agent_runs_conversation
  ON agent_runs (conversation_id, turn_index);

-- ── The limits the user no longer sets ─────────────────────────────────────
--
-- The agent runs until it calls `finish_task`. There is no wall clock and no
-- iteration cap: a research session that needed four hours and one that needed forty
-- minutes were never distinguishable in advance, and a cap that fires mid-sweep
-- throws away the work rather than bounding it.
--
-- The columns stay (old rows carry real values and the history should keep them) but
-- they are nullable now, and nothing writes them.
ALTER TABLE agent_runs ALTER COLUMN wallclock_budget_secs DROP NOT NULL;
ALTER TABLE agent_runs ALTER COLUMN max_iterations DROP NOT NULL;

COMMENT ON COLUMN agent_runs.wallclock_budget_secs IS
  'Deprecated (0041): the agent runs until it finishes. Kept for historical rows.';
COMMENT ON COLUMN agent_runs.max_iterations IS
  'Deprecated (0041): the agent decides how many iterations it needs.';
