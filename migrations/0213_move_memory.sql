-- 0213 — move memory, hypothesis sources and the veto's overrides (Set R-b RB-1.2,
-- RB-1.3; KM-03, KM-04; AGENT-003 §3, §5).
--
-- Three small tables in the memory schema, each keyed to a user the way
-- agent_memory.entry is, and each append-only: what was tried and what happened is
-- evidence, and a record that could be edited afterwards would let a campaign remove
-- the failure that vetoes its favourite move.
--
-- Granted to app_role here, with INSERT and SELECT only. No UPDATE and no DELETE is
-- granted, and a trigger refuses both for the owner too.

CREATE SCHEMA IF NOT EXISTS agent_memory;

CREATE OR REPLACE FUNCTION agent_memory.refuse_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: % refused', TG_TABLE_NAME, TG_OP;
END;
$$ LANGUAGE plpgsql;

-- What happened each time a structural move (FEAT-004 section 6) was tried.
CREATE TABLE IF NOT EXISTS agent_memory.move_record (
  record_id  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id    UUID NOT NULL,
  instrument TEXT NOT NULL CHECK (btrim(instrument) <> ''),
  motif      TEXT NOT NULL CHECK (btrim(motif) <> '' AND char_length(motif) <= 200),
  outcome    TEXT NOT NULL CHECK (outcome IN ('improved', 'failed', 'neutral')),
  -- The campaign or run the record cites; a veto quotes these.
  run_ref    TEXT NOT NULL CHECK (btrim(run_ref) <> ''),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS move_record_motif ON agent_memory.move_record (user_id, motif, record_id);

-- A person's decision to proceed past a blocking veto. The agent cannot write this:
-- the route that does requires skills.admit's sibling authority (a web login).
CREATE TABLE IF NOT EXISTS agent_memory.move_veto_override (
  override_id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id     UUID NOT NULL,
  motif       TEXT NOT NULL CHECK (btrim(motif) <> ''),
  reason      TEXT NOT NULL CHECK (btrim(reason) <> ''),
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Where a hypothesis came from, and whether it repeats an earlier trial (KM-03).
CREATE TABLE IF NOT EXISTS agent_memory.hypothesis_source (
  hypothesis_id UUID PRIMARY KEY,
  user_id       UUID NOT NULL,
  source_kind   TEXT NOT NULL CHECK (source_kind IN ('memory', 'other')),
  finding_id    TEXT,
  source_text   TEXT NOT NULL CHECK (btrim(source_text) <> ''),
  instrument    TEXT NOT NULL CHECK (btrim(instrument) <> ''),
  window_start  DATE NOT NULL,
  window_end    DATE NOT NULL,
  -- How many earlier hypotheses this one repeats (same finding, same instrument,
  -- overlapping window). Zero for a new idea.
  repeats       INT NOT NULL DEFAULT 0 CHECK (repeats >= 0),
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT source_window_runs_forwards CHECK (window_end >= window_start),
  CONSTRAINT memory_source_names_a_finding CHECK (
    (source_kind = 'memory') = (finding_id IS NOT NULL)
  )
);
CREATE INDEX IF NOT EXISTS hypothesis_source_finding
  ON agent_memory.hypothesis_source (user_id, finding_id, instrument)
  WHERE finding_id IS NOT NULL;

DROP TRIGGER IF EXISTS trg_move_record_append_only ON agent_memory.move_record;
CREATE TRIGGER trg_move_record_append_only
  BEFORE UPDATE OR DELETE ON agent_memory.move_record
  FOR EACH ROW EXECUTE FUNCTION agent_memory.refuse_mutation();
DROP TRIGGER IF EXISTS trg_move_override_append_only ON agent_memory.move_veto_override;
CREATE TRIGGER trg_move_override_append_only
  BEFORE UPDATE OR DELETE ON agent_memory.move_veto_override
  FOR EACH ROW EXECUTE FUNCTION agent_memory.refuse_mutation();
DROP TRIGGER IF EXISTS trg_hypothesis_source_append_only ON agent_memory.hypothesis_source;
CREATE TRIGGER trg_hypothesis_source_append_only
  BEFORE UPDATE OR DELETE ON agent_memory.hypothesis_source
  FOR EACH ROW EXECUTE FUNCTION agent_memory.refuse_mutation();

GRANT USAGE ON SCHEMA agent_memory TO app_role;
GRANT SELECT, INSERT ON agent_memory.move_record TO app_role;
GRANT SELECT, INSERT ON agent_memory.move_veto_override TO app_role;
GRANT SELECT, INSERT ON agent_memory.hypothesis_source TO app_role;
