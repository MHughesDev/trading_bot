-- Set L Phase 2 (AGENT-001 §6, ADR-0025, DA-09): scoped session tokens.
--
-- Authority the agent cannot grant itself. A research session's token carries a
-- fixed set of scopes; anything outside them is refused by middleware, not by a
-- prompt. The scopes that matter most are the ones that are never issued:
-- `data.holdout`, order placement, automation arming and model-alias promotion
-- (D-12). There is deliberately no code path that mints them for an agent.

ALTER TABLE sessions ADD COLUMN IF NOT EXISTS scopes TEXT[] NOT NULL DEFAULT '{}';

-- The project a service token is bound to, if any. A research token is scoped to
-- exactly one project, which is what makes "this token may not read past the cutoff"
-- a decidable question.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS project_id UUID;

CREATE INDEX IF NOT EXISTS sessions_project ON sessions (project_id)
  WHERE project_id IS NOT NULL;

-- Web logins keep full user rights: scopes apply to service sessions only
-- (AGENT-001 §6). Marking them explicitly avoids a middleware that has to guess.
UPDATE sessions SET scopes = ARRAY['web:full'] WHERE kind = 'web' AND scopes = '{}';

-- Scopes an agent session may ever hold. Enforced as a CHECK so that a bug in the
-- minting path cannot quietly widen an agent's authority: adding a capability
-- requires a migration, which is a reviewable event.
CREATE OR REPLACE FUNCTION agent_scopes_are_allowed() RETURNS trigger AS $$
DECLARE
  forbidden TEXT[] := ARRAY[
    'data.holdout',      -- the one-shot vault; only the vault-gate service reads it
    'orders.place',      -- research authority only (D-12)
    'orders.cancel',
    'automations.arm',
    'models.promote',    -- alias promotion is a human decision
    'skills.admit',      -- the platform admits skills, never the agent (ADR-0028)
    'web:full'
  ];
  offending TEXT;
BEGIN
  IF NEW.kind = 'service' AND NEW.project_id IS NOT NULL THEN
    SELECT s INTO offending
      FROM unnest(NEW.scopes) AS s
     WHERE s = ANY(forbidden)
     LIMIT 1;
    IF offending IS NOT NULL THEN
      RAISE EXCEPTION
        'scope % may never be granted to a project-bound agent session', offending
        USING ERRCODE = 'check_violation';
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS agent_scopes_are_allowed_trg ON sessions;
CREATE TRIGGER agent_scopes_are_allowed_trg
  BEFORE INSERT OR UPDATE ON sessions
  FOR EACH ROW EXECUTE FUNCTION agent_scopes_are_allowed();
