-- 0123 — structured turn summaries (AGENT-007 §2.1; Set S task S-4.1).
--
-- ## What this replaces
--
-- `conversations::prior_context` builds the carry between turns out of
-- `agent_runs.summary`, which is **the model's own final prose**. So the thing a
-- later turn is told it concluded is whatever the previous turn said it
-- concluded, with nothing checking that against what the run actually did — and
-- on the hosted driver that prose was then ignored entirely.
--
-- A turn summary is written by the **harness**, from `WorkingState` and the two
-- ledgers. `TurnSummary::from_state` has no parameter through which prose could
-- arrive, which is S-4.1 stated as a function signature; this is the table that
-- lets the value outlive the process.
--
-- ## Numbered 0123, not the plan's 0065
--
-- Same collision ADR-S3-08 records: Set S's ranges were allocated against a tree
-- that has since moved past them. 0122 is the semantic store; this follows it.
--
-- ## Columns versus the document
--
-- The whole summary is JSONB, because it is rendered as a unit and its shape is
-- `TurnSummary`'s — a column per field would be a second schema to keep in step
-- with a struct that already serialises.
--
-- What is lifted out is what a *query* needs: the conversation and turn to order
-- and window by, the outcome to filter on, and the trial count, which is the one
-- number somebody will want summed across a conversation without parsing
-- documents to get it.
--
-- ## One row per turn, and re-running a turn replaces it
--
-- The primary key is `(conversation_id, turn)`. A turn that is retried produces
-- one summary, not two: the carry a later turn reads must be what the turn
-- finally did, and an append-only log here would make "the previous turn" a
-- question with two answers.

CREATE TABLE IF NOT EXISTS agent_turn_summaries (
  conversation_id UUID NOT NULL,
  turn            INTEGER NOT NULL,

  -- `finished | refused | escalated | fenced | cancelled`. A closed set: an
  -- outcome nobody enumerated is an outcome the renderer has no sentence for.
  outcome         TEXT NOT NULL CHECK (outcome IN (
                    'finished', 'refused', 'escalated', 'fenced', 'cancelled'
                  )),

  -- Lifted out of the document because it is summed across a conversation, and
  -- a sum over JSONB is a sum nobody will write.
  trials_spent    INTEGER NOT NULL DEFAULT 0 CHECK (trials_spent >= 0),

  -- `harness::memory::TurnSummary`, whole.
  summary         JSONB NOT NULL CHECK (jsonb_typeof(summary) = 'object'),

  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

  PRIMARY KEY (conversation_id, turn)
);

-- The only read: the turns before this one, in order. The primary key already
-- serves it, so this index exists for the *descending* case — the digest and the
-- desk both want the most recent few.
CREATE INDEX IF NOT EXISTS turn_summaries_recent
  ON agent_turn_summaries (conversation_id, turn DESC);
