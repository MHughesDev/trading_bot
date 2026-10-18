-- 0210 — the skill registry (Set R-a RA-1.1 and RA-1.2; ADR-0028; AGENT-003 §3, §6).
--
-- Until this migration a skill was a directory of files that every session
-- mounted in full (`crates/skills::core`) and a separate table of Python
-- transformations (0060). There was nothing that could say which skill a
-- session saw, nothing a lower scope could be admitted into, no record of how
-- often a skill was in front of a model, and no single place a skill became
-- visible. This is that place.
--
-- Three properties are constraints, not conventions:
--
--   * A skill is visible only through an admitted version. `skill_glossary` is a
--     VIEW over `skill` joined to `skill_version`, so "no admitted skill is
--     absent from the glossary" (SK-02) holds by construction, and a CHECK keeps
--     a visible status from existing without a current version.
--   * A lower scope cannot shadow a core or global name (SK-01). A partial unique
--     index covers core and global; a trigger covers the cross-scope case an
--     index cannot express.
--   * A version is immutable and a description is at most 100 tokens, a body at
--     most 500 lines (SK-08), at the schema as well as in the Rust that checks
--     the same numbers before admission.
--
-- Admission itself is `skill_registry.admit(...)`: one function, one
-- transaction. The two foreign keys that point at each other (a skill names its
-- current version, a version names its skill) are DEFERRABLE INITIALLY
-- DEFERRED so that single transaction can insert both. `skills.admit` is never
-- an agent scope (SK-03); the grants below give the platform's role the write
-- and `agent_role` nothing at all.
--
-- Everything is granted to `app_role` in this migration, unconditionally: a database
-- with no `app_role` (0044 creates it) fails here rather than silently skipping.
-- The defect 0145, 0147
-- and 0166 each repaired was a schema created without a grant, which passes
-- every owner-connected test and fails in the running platform.

CREATE SCHEMA IF NOT EXISTS skill_registry;

-- Append-only guard, local to this schema so the migration depends on nothing
-- created after 0001.
CREATE OR REPLACE FUNCTION skill_registry.refuse_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: % refused', TG_TABLE_NAME, TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE IF NOT EXISTS skill_registry.skill (
  skill_id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name            TEXT NOT NULL CHECK (name ~ '^[a-z][a-z0-9-]{0,62}$'),
  scope           TEXT NOT NULL CHECK (scope IN ('core', 'global', 'user', 'project')),
  owner_user_id   UUID,
  project_id      UUID,
  category        TEXT NOT NULL CHECK (category <> ''),
  status          TEXT NOT NULL
                  CHECK (status IN ('provisional', 'admitted', 'quarantined', 'deprecated', 'archived')),
  pinned          BOOLEAN NOT NULL DEFAULT FALSE,
  alias_of        UUID REFERENCES skill_registry.skill (skill_id),
  current_version TEXT,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- Who a skill belongs to follows from its scope, and nothing else is allowed.
  CONSTRAINT skill_scope_owner CHECK (
       (scope = 'user'    AND owner_user_id IS NOT NULL AND project_id IS NULL)
    OR (scope = 'project' AND project_id IS NOT NULL    AND owner_user_id IS NULL)
    OR (scope IN ('core', 'global') AND owner_user_id IS NULL AND project_id IS NULL)
  ),
  -- Only a global skill can be pinned into every session.
  CONSTRAINT skill_pinned_is_global CHECK (NOT pinned OR scope = 'global'),
  -- A skill a session can see has a version a session can read.
  CONSTRAINT skill_visible_has_version CHECK (
    status NOT IN ('provisional', 'admitted') OR current_version IS NOT NULL
  ),
  CONSTRAINT skill_alias_not_self CHECK (alias_of IS NULL OR alias_of <> skill_id)
);

-- Names are unique within the place they are looked up from.
CREATE UNIQUE INDEX IF NOT EXISTS skill_core_global_name
  ON skill_registry.skill (name) WHERE scope IN ('core', 'global');
CREATE UNIQUE INDEX IF NOT EXISTS skill_user_name
  ON skill_registry.skill (owner_user_id, name) WHERE scope = 'user';
CREATE UNIQUE INDEX IF NOT EXISTS skill_project_name
  ON skill_registry.skill (project_id, name) WHERE scope = 'project';

-- A lower scope cannot shadow a core or global name, and a promotion into core
-- or global cannot collide with a lower-scope skill that already has the name.
-- An index cannot say this across scopes, so a trigger does.
CREATE OR REPLACE FUNCTION skill_registry.refuse_shadowing()
RETURNS TRIGGER AS $$
BEGIN
  IF NEW.scope IN ('user', 'project') THEN
    IF EXISTS (
      SELECT 1 FROM skill_registry.skill s
      WHERE s.scope IN ('core', 'global') AND s.name = NEW.name
        AND s.status <> 'archived' AND s.skill_id <> NEW.skill_id
    ) THEN
      RAISE EXCEPTION 'skill name % is taken by a core or global skill; a lower scope may not shadow it', NEW.name;
    END IF;
  ELSE
    IF EXISTS (
      SELECT 1 FROM skill_registry.skill s
      WHERE s.scope IN ('user', 'project') AND s.name = NEW.name
        AND s.status <> 'archived' AND s.skill_id <> NEW.skill_id
    ) THEN
      RAISE EXCEPTION 'skill name % is already used at a lower scope; resolve it before promoting', NEW.name;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_skill_no_shadowing ON skill_registry.skill;
CREATE TRIGGER trg_skill_no_shadowing
  BEFORE INSERT OR UPDATE OF name, scope, status ON skill_registry.skill
  FOR EACH ROW EXECUTE FUNCTION skill_registry.refuse_shadowing();

CREATE TABLE IF NOT EXISTS skill_registry.skill_version (
  version_hash       TEXT PRIMARY KEY CHECK (version_hash ~ '^[0-9a-f]{64}$'),
  skill_id           UUID NOT NULL REFERENCES skill_registry.skill (skill_id)
                     DEFERRABLE INITIALLY DEFERRED,
  parent_version     TEXT REFERENCES skill_registry.skill_version (version_hash),
  description        TEXT NOT NULL CHECK (description <> ''),
  -- Counted by the same function admission uses; a version that exceeds either
  -- number cannot be stored, whatever wrote it (SK-08).
  description_tokens INT  NOT NULL CHECK (description_tokens BETWEEN 1 AND 100),
  body_lines         INT  NOT NULL CHECK (body_lines BETWEEN 0 AND 500),
  bundle_digest      TEXT NOT NULL CHECK (bundle_digest ~ '^[0-9a-f]{64}$'),
  created_by         TEXT NOT NULL CHECK (created_by <> ''),
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS skill_version_skill ON skill_registry.skill_version (skill_id, created_at);

DROP TRIGGER IF EXISTS trg_skill_version_immutable ON skill_registry.skill_version;
CREATE TRIGGER trg_skill_version_immutable
  BEFORE UPDATE OR DELETE ON skill_registry.skill_version
  FOR EACH ROW EXECUTE FUNCTION skill_registry.refuse_mutation();

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'skill_current_version_fk') THEN
    ALTER TABLE skill_registry.skill
      ADD CONSTRAINT skill_current_version_fk
      FOREIGN KEY (current_version) REFERENCES skill_registry.skill_version (version_hash)
      DEFERRABLE INITIALLY DEFERRED;
  END IF;
END $$;

-- What a session may see. A VIEW, not a table: it cannot be stale, so a
-- quarantined skill leaves every glossary in the statement that quarantines it.
CREATE OR REPLACE VIEW skill_registry.skill_glossary AS
SELECT s.skill_id,
       s.name,
       s.scope,
       s.owner_user_id,
       s.project_id,
       s.category,
       s.status,
       s.pinned,
       s.alias_of,
       s.current_version,
       v.description,
       v.description_tokens
  FROM skill_registry.skill s
  JOIN skill_registry.skill_version v ON v.version_hash = s.current_version
 WHERE s.status IN ('provisional', 'admitted');

-- Admission: the only intended path to a visible skill.
CREATE OR REPLACE FUNCTION skill_registry.admit(
  p_name TEXT, p_scope TEXT, p_owner_user_id UUID, p_project_id UUID, p_category TEXT,
  p_status TEXT, p_pinned BOOLEAN, p_version_hash TEXT, p_parent_version TEXT,
  p_description TEXT, p_description_tokens INT, p_body_lines INT, p_bundle_digest TEXT,
  p_created_by TEXT
) RETURNS UUID AS $$
DECLARE
  v_id UUID;
BEGIN
  IF p_status NOT IN ('provisional', 'admitted') THEN
    RAISE EXCEPTION 'admit() makes a skill visible; % is not a visible status', p_status;
  END IF;
  IF p_parent_version IS NOT NULL THEN
    -- A patch: the same skill gets a new current version.
    SELECT skill_id INTO v_id FROM skill_registry.skill_version WHERE version_hash = p_parent_version;
    IF v_id IS NULL THEN
      RAISE EXCEPTION 'parent version % does not exist', p_parent_version;
    END IF;
    INSERT INTO skill_registry.skill_version
      (version_hash, skill_id, parent_version, description, description_tokens, body_lines, bundle_digest, created_by)
    VALUES
      (p_version_hash, v_id, p_parent_version, p_description, p_description_tokens, p_body_lines, p_bundle_digest, p_created_by);
    UPDATE skill_registry.skill SET current_version = p_version_hash, updated_at = now() WHERE skill_id = v_id;
  ELSE
    INSERT INTO skill_registry.skill
      (name, scope, owner_user_id, project_id, category, status, pinned, current_version)
    VALUES
      (p_name, p_scope, p_owner_user_id, p_project_id, p_category, p_status, COALESCE(p_pinned, FALSE), p_version_hash)
    RETURNING skill_id INTO v_id;
    INSERT INTO skill_registry.skill_version
      (version_hash, skill_id, parent_version, description, description_tokens, body_lines, bundle_digest, created_by)
    VALUES
      (p_version_hash, v_id, NULL, p_description, p_description_tokens, p_body_lines, p_bundle_digest, p_created_by);
  END IF;
  RETURN v_id;
END;
$$ LANGUAGE plpgsql;

-- What was in front of a model, and whether it helped. The registry is the only
-- home of usage statistics (SK-13); nothing counts exposures anywhere else.
CREATE TABLE IF NOT EXISTS skill_registry.skill_run (
  run_id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  skill_id     UUID NOT NULL REFERENCES skill_registry.skill (skill_id),
  version_hash TEXT NOT NULL REFERENCES skill_registry.skill_version (version_hash),
  project_id   UUID,
  session_id   UUID,
  state        TEXT NOT NULL CHECK (state IN ('exposed', 'activated')),
  vote         SMALLINT NOT NULL DEFAULT 0 CHECK (vote IN (-1, 0, 1)),
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS skill_run_skill ON skill_registry.skill_run (skill_id, state);

-- A proposal cites where it came from: a speculative proposal has no source and
-- cannot be written (AGENT-003 acceptance 3).
CREATE TABLE IF NOT EXISTS skill_registry.skill_proposal (
  proposal_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  target_skill_id UUID REFERENCES skill_registry.skill (skill_id),
  proposed_name   TEXT NOT NULL CHECK (proposed_name ~ '^[a-z][a-z0-9-]{0,62}$'),
  project_id      UUID NOT NULL,
  proposer        TEXT NOT NULL CHECK (proposer <> ''),
  reason          TEXT NOT NULL CHECK (reason IN ('T1', 'T2', 'T3', 'T4')),
  source_ref      TEXT NOT NULL CHECK (btrim(source_ref) <> ''),
  bundle_artifact TEXT,
  similarity      DOUBLE PRECISION CHECK (similarity IS NULL OR similarity BETWEEN 0 AND 1),
  status          TEXT NOT NULL DEFAULT 'submitted'
                  CHECK (status IN ('submitted', 'verifying', 'rejected', 'admitted')),
  verdict         JSONB,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  decided_at      TIMESTAMPTZ,
  CONSTRAINT proposal_decided_has_verdict CHECK (
    status IN ('submitted', 'verifying') OR (verdict IS NOT NULL AND decided_at IS NOT NULL)
  )
);

-- The resident selection a project froze at a version bump. Append-only: a new
-- project version is new rows, so a session's resident set can be reproduced.
CREATE TABLE IF NOT EXISTS skill_registry.project_skill_selection (
  project_id      UUID NOT NULL,
  project_version INT  NOT NULL CHECK (project_version >= 0),
  skill_name      TEXT NOT NULL,
  rank            INT  NOT NULL CHECK (rank >= 0),
  PRIMARY KEY (project_id, project_version, skill_name)
);
DROP TRIGGER IF EXISTS trg_selection_frozen ON skill_registry.project_skill_selection;
CREATE TRIGGER trg_selection_frozen
  BEFORE UPDATE OR DELETE ON skill_registry.project_skill_selection
  FOR EACH ROW EXECUTE FUNCTION skill_registry.refuse_mutation();

-- ── grants: exactly what the platform does, and nothing for an agent ────────
--
-- `skills.admit` is internal (SK-03). The agent's database role gets no grant
-- here at all: its path to a skill is a proposal through the API, which the
-- platform writes under its own role.
GRANT USAGE ON SCHEMA skill_registry TO app_role;
GRANT SELECT, INSERT, UPDATE ON skill_registry.skill TO app_role;
GRANT SELECT, INSERT ON skill_registry.skill_version TO app_role;
GRANT SELECT ON skill_registry.skill_glossary TO app_role;
GRANT SELECT, INSERT, UPDATE ON skill_registry.skill_run TO app_role;
GRANT SELECT, INSERT, UPDATE ON skill_registry.skill_proposal TO app_role;
GRANT SELECT, INSERT ON skill_registry.project_skill_selection TO app_role;
GRANT EXECUTE ON FUNCTION skill_registry.admit(
  TEXT, TEXT, UUID, UUID, TEXT, TEXT, BOOLEAN, TEXT, TEXT, TEXT, INT, INT, TEXT, TEXT
) TO app_role;
