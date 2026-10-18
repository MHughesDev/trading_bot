-- 0065 — episodic memory: the turn summary (AGENT-007 §2.1, Set S task S-4.1).
--
-- What this replaces is `conversations::prior_context`, which built a
-- "you concluded: …" string from the model's own final prose — and which the
-- hosted driver then ignored entirely (AGENT-005 §0). So the only cross-turn
-- memory this platform had was a paraphrase, written by the thing being
-- remembered, that nothing read.
--
-- A row here is written by the HARNESS from the findings ledger and the effect
-- log. `prior_context` becomes a renderer over this table, and the difference is
-- not tidiness: a paraphrase loses the ids, and an id is the only part of a
-- finding the next turn can act on.
--
-- The document is stored whole in `summary` rather than shredded into columns.
-- §2.1 fixes the schema, `harness::memory::turn::TurnSummary` is its Rust twin,
-- and `the_schema_round_trips_so_the_table_and_the_renderer_agree` asserts the
-- two agree. Shredding it would mean a migration every time §2.1 gains a field,
-- and the renderer is the only reader.

CREATE TABLE IF NOT EXISTS agent_turn_summaries (
  conversation_id UUID NOT NULL
    REFERENCES agent_conversations(conversation_id) ON DELETE CASCADE,
  turn_index      INT  NOT NULL CHECK (turn_index >= 0),
  run_id          UUID REFERENCES agent_runs(run_id) ON DELETE SET NULL,
  -- finished | refused | escalated | fenced | cancelled. Closed, because a turn
  -- that ended some sixth way is a turn the loop does not know it produced.
  outcome         TEXT NOT NULL CHECK (outcome IN
                    ('finished','refused','escalated','fenced','cancelled')),
  -- The §2.1 document. Never the model's final message.
  summary         JSONB NOT NULL,
  -- Denormalised for the retention job and the desk, which count these without
  -- opening the document.
  findings_n      INT NOT NULL DEFAULT 0 CHECK (findings_n >= 0),
  trials_spent    INT NOT NULL DEFAULT 0 CHECK (trials_spent >= 0),
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (conversation_id, turn_index)
);

-- The renderer reads a conversation's turns in order and stops at the current
-- one. That is the whole query, so it is the whole index.
CREATE INDEX IF NOT EXISTS idx_turn_summaries_conversation
  ON agent_turn_summaries (conversation_id, turn_index);

-- A summary is a record of a turn that has happened. Rewriting one rewrites what
-- the next turn believes, which is exactly the door this table exists to close:
-- the model cannot write its own history here, and neither can a later run.
CREATE OR REPLACE FUNCTION refuse_turn_summary_rewrite() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION
    'agent_turn_summaries is append-only: turn % of conversation % has already been summarised',
    OLD.turn_index, OLD.conversation_id;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_turn_summary_append_only ON agent_turn_summaries;
CREATE TRIGGER trg_turn_summary_append_only BEFORE UPDATE ON agent_turn_summaries
  FOR EACH ROW EXECUTE FUNCTION refuse_turn_summary_rewrite();
