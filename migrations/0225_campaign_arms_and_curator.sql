-- 0225 — the shadow random arm's record and the curator's proposals (Set R-b RB-1.8,
-- RB-3.1 to RB-3.3; EV-09; AGENT-003 section 6.4).
--
-- A campaign's random-search arm is part of the campaign record (ADR-RB-03): it is
-- created with the campaign, with the agent arm's budget and a seed the platform
-- derives from the campaign id, so it cannot be re-rolled afterwards until it
-- flatters. Scores are appended one per trial for either arm; a comparison reads
-- both from here, so nobody can hand it a convenient list.
--
-- The curator's items are proposals to a person, not writes: a row here changes
-- nothing until an admin acts on it through the proposal path.
--
-- Everything is granted to app_role in this file. Append-only: no UPDATE or DELETE is
-- granted and a trigger refuses both.

CREATE SCHEMA IF NOT EXISTS agent_memory;

CREATE OR REPLACE FUNCTION agent_memory.refuse_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: % refused', TG_TABLE_NAME, TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE IF NOT EXISTS agent_memory.campaign_arm (
  campaign_id UUID PRIMARY KEY,
  user_id     UUID NOT NULL,
  budget      INT  NOT NULL CHECK (budget > 0),
  seed        BIGINT NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS agent_memory.campaign_arm_score (
  campaign_id UUID NOT NULL REFERENCES agent_memory.campaign_arm (campaign_id),
  arm         TEXT NOT NULL CHECK (arm IN ('agent', 'random')),
  seq         INT  NOT NULL CHECK (seq >= 0),
  -- The sealed metric of one trial. Not NULL: a failed trial is recorded with its
  -- failure value by the caller, never skipped, so both arms count the same trials.
  score       DOUBLE PRECISION NOT NULL,
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (campaign_id, arm, seq)
);

DROP TRIGGER IF EXISTS trg_campaign_arm_score_append_only ON agent_memory.campaign_arm_score;
CREATE TRIGGER trg_campaign_arm_score_append_only
  BEFORE UPDATE OR DELETE ON agent_memory.campaign_arm_score
  FOR EACH ROW EXECUTE FUNCTION agent_memory.refuse_mutation();
DROP TRIGGER IF EXISTS trg_campaign_arm_append_only ON agent_memory.campaign_arm;
CREATE TRIGGER trg_campaign_arm_append_only
  BEFORE UPDATE OR DELETE ON agent_memory.campaign_arm
  FOR EACH ROW EXECUTE FUNCTION agent_memory.refuse_mutation();

CREATE TABLE IF NOT EXISTS skill_registry.curator_item (
  item_id    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  run_id     UUID NOT NULL,
  kind       TEXT NOT NULL CHECK (kind IN ('playbook_caution', 'playbook_prioritise', 'merge_skills', 'deprecate')),
  -- The numbers the item rests on, as the curator saw them. Identifiers and numbers
  -- only: the curator reads no free text.
  detail     JSONB NOT NULL CHECK (jsonb_typeof(detail) = 'object'),
  status     TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'accepted', 'dismissed')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  decided_by UUID,
  decided_at TIMESTAMPTZ,
  CONSTRAINT decided_has_decider CHECK (status = 'open' OR (decided_by IS NOT NULL AND decided_at IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS curator_item_open ON skill_registry.curator_item (status, item_id);

GRANT USAGE ON SCHEMA agent_memory TO app_role;
GRANT SELECT, INSERT ON agent_memory.campaign_arm TO app_role;
GRANT SELECT, INSERT ON agent_memory.campaign_arm_score TO app_role;
GRANT SELECT, INSERT, UPDATE ON skill_registry.curator_item TO app_role;
