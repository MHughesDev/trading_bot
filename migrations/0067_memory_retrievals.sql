-- 0067 — the retrieval log (AGENT-007 §4; Set S task S-4.5).
--
-- Every item the task brief put in front of a run, in rank order, with the
-- reason it was chosen — and whether the run went on to CITE it.
--
-- That last column is the point. Without it "the agent has memory" is a claim
-- nobody can check: a brief can be assembled perfectly, retrieved into context,
-- and ignored, and every dashboard stays green. `cited` is the memory-recall
-- signal AGENT-009 §4 scores, and it is the one place these two specs meet.
--
-- `why` is stored because P4 requires each retrieved item to say why it was
-- retrieved. A surprising retrieval is otherwise unexplainable after the fact,
-- and retrieval is where a brief silently starts serving the wrong run.

CREATE TABLE IF NOT EXISTS knowledge.memory_retrieval (
  retrieval_id  BIGSERIAL PRIMARY KEY,
  tenant_id     TEXT NOT NULL,
  run_id        UUID NOT NULL REFERENCES agent_runs(run_id) ON DELETE CASCADE,
  entry_id      UUID NOT NULL REFERENCES knowledge.memory_entry(entry_id) ON DELETE CASCADE,
  slot          TEXT NOT NULL CHECK (slot IN (
                  'user_preference','instrument_profile','finding',
                  'negative_result','open_question','conflict')),
  rank          INT NOT NULL CHECK (rank >= 0),
  why           TEXT NOT NULL CHECK (length(why) > 0),
  -- Set when a later `record_finding` or `finish_task` names the entry's id.
  cited         BOOLEAN NOT NULL DEFAULT FALSE,
  cited_at      TIMESTAMPTZ,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (run_id, entry_id)
);

CREATE INDEX IF NOT EXISTS idx_memory_retrieval_run
  ON knowledge.memory_retrieval (run_id, rank);
-- The recall metric's query: cited share per tenant over a window.
CREATE INDEX IF NOT EXISTS idx_memory_retrieval_recall
  ON knowledge.memory_retrieval (tenant_id, created_at DESC, cited);

ALTER TABLE knowledge.memory_retrieval ENABLE ROW LEVEL SECURITY;
ALTER TABLE knowledge.memory_retrieval FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON knowledge.memory_retrieval;
CREATE POLICY tenant_isolation ON knowledge.memory_retrieval
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- Written by the platform when it assembles a brief, and updated by the platform
-- when a citation lands. The agent does not write its own recall score — that is
-- the same principle as P1, applied to the metric that grades P1.
GRANT SELECT, INSERT, UPDATE ON knowledge.memory_retrieval TO app_role;
GRANT USAGE, SELECT ON SEQUENCE knowledge.memory_retrieval_retrieval_id_seq TO app_role;
GRANT SELECT ON knowledge.memory_retrieval TO internal_ml_role;
