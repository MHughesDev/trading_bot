-- 0126 — the run says what kind of run it is (AGENT-013 §2.1, §4, §5;
-- Set S task S-10.1).
--
-- ## What was missing
--
-- `WorkingState` has carried `mode`, `workflow`, `ring` and `stage` since
-- phase 1, and the state card renders all four. None of them survived the run:
-- `agent_runs` knew a goal, a model and a status, so every question about *what
-- a run was allowed to do* could only be answered from inside the run, while it
-- was still going.
--
-- That is the wrong way round for all four. The ring is the one that matters
-- most: it decides what the model could name, so a row that does not carry it
-- cannot be audited afterwards. "Could this run have armed an automation?" is
-- not a question anyone should have to re-derive from a transcript.
--
-- ## The CHECK lists mirror the Rust enums
--
-- Named with the strings `harness::state::{Mode, Workflow, Ring, Stage}`
-- serialise to, and asserted equal to them by
-- `crates/api/tests/schema_mirrors_lifecycle.rs` — the same arrangement
-- `0122` uses for the memory kinds, and for the same reason: a CHECK list and an
-- enum that drift apart fail at 3am on a row nobody was watching.

ALTER TABLE agent_runs
  ADD COLUMN IF NOT EXISTS mode TEXT NOT NULL DEFAULT 'chat',
  ADD COLUMN IF NOT EXISTS workflow TEXT,
  ADD COLUMN IF NOT EXISTS ring TEXT NOT NULL DEFAULT 'research',
  ADD COLUMN IF NOT EXISTS stage TEXT NOT NULL DEFAULT 'explore',
  -- AGENT-014 §10: every Research run has a session row, and a continuous
  -- session's children carry its id. Nullable because a Chat turn has none and
  -- a NOT NULL here would make the common case carry a sentinel.
  ADD COLUMN IF NOT EXISTS session_id UUID;

ALTER TABLE agent_runs
  ADD CONSTRAINT a_run_is_chat_or_research CHECK (mode IN ('chat', 'research'));

-- `harness::state::Workflow`, all seven. Nullable, because a Chat turn is not
-- doing any of them — and `investigate` would be a lie rather than a default.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_workflow_is_one_of_the_seven CHECK (
    workflow IS NULL
    OR workflow IN (
      'investigate',
      'falsify',
      'replicate',
      'compare',
      'diagnose',
      'survey',
      'optimise'
    )
  );

-- `harness::state::Ring`. Three, not four: ring 3 is the one with no door, so
-- there is no value for it and a row cannot claim it.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_ring_is_one_the_platform_grants CHECK (
    ring IN ('research', 'platform_artifacts', 'paper')
  );

-- `harness::state::Stage`, all ten.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_stage_is_one_of_the_lifecycle CHECK (
    stage IN (
      'explore',
      'hypothesise',
      'preregister',
      'design',
      'run',
      'refine',
      'compare',
      'reproduce',
      'propose',
      'closed'
    )
  );

-- A chat turn is ring 0 and cannot be widened (AGENT-013 §2.2). Enforced here as
-- well as in exposure because they are different mechanisms with different
-- failure modes: exposure is a routing property that a future code path could
-- route around, and this is a row that cannot exist.
--
-- The same belt-and-braces argument S-10.2 makes about the ticket path.
ALTER TABLE agent_runs
  ADD CONSTRAINT a_chat_turn_reaches_nothing CHECK (
    mode <> 'chat' OR ring = 'research'
  );

-- Sessions, and the runs that belong to one.
CREATE INDEX IF NOT EXISTS agent_runs_session_idx
  ON agent_runs (session_id, created_at DESC)
  WHERE session_id IS NOT NULL;
