-- 0211 — what a registry version is made of, and the writes that change a skill's
-- standing (Set R-a RA-2.4, RA-2.6, RA-3.3, RA-4.2, RA-4.5; Set R-b RB-2.3).
--
-- 0210 made a skill visible only through an admitted version, but a version was a
-- hash and a description: the *bundle* (SKILL.md and its files) had nowhere to live,
-- so a project skill could be admitted and never mounted. This adds the bundle, the
-- proposal's draft, the verification verdicts the admission reads, the notice a
-- quarantine emits, and the functions through which a skill's standing changes.
--
-- A status change is a function, not an UPDATE a route issues, so that the notice
-- is written in the statement that changes the status: "quarantining a global skill
-- removes it from all glossaries in the same transaction and notifies live sessions"
-- (AGENT-003 acceptance 5). The glossary is a view, so removal is the UPDATE itself.
--
-- Every table and function is granted to app_role below, in this file.

-- A version's files. Text only (a skill is instructions and small pure scripts);
-- a path -> content object. Append-only like the version it belongs to.
CREATE TABLE IF NOT EXISTS skill_registry.skill_bundle (
  version_hash TEXT PRIMARY KEY REFERENCES skill_registry.skill_version (version_hash)
               DEFERRABLE INITIALLY DEFERRED,
  files        JSONB NOT NULL CHECK (jsonb_typeof(files) = 'object' AND files ? 'SKILL.md'),
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
DROP TRIGGER IF EXISTS trg_skill_bundle_immutable ON skill_registry.skill_bundle;
CREATE TRIGGER trg_skill_bundle_immutable
  BEFORE UPDATE OR DELETE ON skill_registry.skill_bundle
  FOR EACH ROW EXECUTE FUNCTION skill_registry.refuse_mutation();

-- The draft a proposal carries until it is admitted or rejected.
CREATE TABLE IF NOT EXISTS skill_registry.skill_proposal_bundle (
  proposal_id UUID PRIMARY KEY REFERENCES skill_registry.skill_proposal (proposal_id),
  category    TEXT NOT NULL CHECK (category <> ''),
  files       JSONB NOT NULL CHECK (jsonb_typeof(files) = 'object' AND files ? 'SKILL.md'),
  sources     JSONB NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(sources) = 'array')
);

-- One row per verification step per attempt; the verdict an admission reads is the
-- latest row of each step. A skill failing any step is not admitted, and the row
-- names the step (RA-3.3).
CREATE TABLE IF NOT EXISTS skill_registry.skill_verdict (
  verdict_id  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  proposal_id UUID NOT NULL REFERENCES skill_registry.skill_proposal (proposal_id),
  step        TEXT NOT NULL CHECK (step IN
              ('lint', 'procedure', 'tests', 'replay', 'triggers', 'coexistence', 'savings', 'reviewer')),
  passed      BOOLEAN NOT NULL,
  detail      JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS skill_verdict_proposal ON skill_registry.skill_verdict (proposal_id, step, verdict_id DESC);

-- What a live session is told when a resident skill changes standing (SK-14). The
-- system prompt is never edited; sessions poll this and append a system message.
CREATE TABLE IF NOT EXISTS skill_registry.skill_notice (
  notice_id  BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  skill_id   UUID NOT NULL REFERENCES skill_registry.skill (skill_id),
  skill_name TEXT NOT NULL,
  kind       TEXT NOT NULL CHECK (kind IN ('quarantined', 'deprecated', 'archived')),
  reason     TEXT NOT NULL CHECK (btrim(reason) <> ''),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS skill_notice_created ON skill_registry.skill_notice (notice_id);

-- A skill leaves its visible standing, and says so, in one statement.
CREATE OR REPLACE FUNCTION skill_registry.retire(p_skill UUID, p_status TEXT, p_reason TEXT)
RETURNS VOID AS $$
DECLARE
  v_name TEXT;
BEGIN
  IF p_status NOT IN ('quarantined', 'deprecated', 'archived') THEN
    RAISE EXCEPTION 'retire() moves a skill out of view; % is not such a status', p_status;
  END IF;
  UPDATE skill_registry.skill
     SET status = p_status, updated_at = now()
   WHERE skill_id = p_skill
   RETURNING name INTO v_name;
  IF v_name IS NULL THEN
    RAISE EXCEPTION 'skill % does not exist', p_skill;
  END IF;
  INSERT INTO skill_registry.skill_notice (skill_id, skill_name, kind, reason)
  VALUES (p_skill, v_name, p_status, p_reason);
END;
$$ LANGUAGE plpgsql;

-- Provisional to admitted (RA-4.5) and the scope moves (RB-2.1, RB-2.2). The owner
-- columns change with the scope in one UPDATE, because the CHECK in 0210 would
-- refuse every intermediate state.
CREATE OR REPLACE FUNCTION skill_registry.set_admitted(p_skill UUID) RETURNS VOID AS $$
BEGIN
  UPDATE skill_registry.skill SET status = 'admitted', updated_at = now()
   WHERE skill_id = p_skill AND status = 'provisional';
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE FUNCTION skill_registry.move_scope(p_skill UUID, p_scope TEXT, p_owner UUID)
RETURNS VOID AS $$
BEGIN
  IF p_scope = 'user' THEN
    UPDATE skill_registry.skill SET scope = 'user', owner_user_id = p_owner, project_id = NULL, updated_at = now()
     WHERE skill_id = p_skill;
  ELSIF p_scope = 'global' THEN
    UPDATE skill_registry.skill SET scope = 'global', owner_user_id = NULL, project_id = NULL, updated_at = now()
     WHERE skill_id = p_skill;
  ELSE
    RAISE EXCEPTION 'a skill moves to user or global scope, not %', p_scope;
  END IF;
END;
$$ LANGUAGE plpgsql;

-- A deprecated skill points at its replacement (RB-2.6). Deprecated only, and never at
-- itself; the CHECK on 0210 already refuses self-aliasing, this refuses aliasing a live
-- skill (which would hide a skill that is still in the glossary).
CREATE OR REPLACE FUNCTION skill_registry.set_alias(p_skill UUID, p_target UUID) RETURNS VOID AS $$
BEGIN
  UPDATE skill_registry.skill SET alias_of = p_target, updated_at = now()
   WHERE skill_id = p_skill AND status = 'deprecated';
  IF NOT FOUND THEN
    RAISE EXCEPTION 'skill % is not deprecated; only a deprecated skill can be an alias', p_skill;
  END IF;
END;
$$ LANGUAGE plpgsql;

-- Follow alias_of from a skill to the live skill that replaced it. At most eight hops,
-- so a cycle cannot loop; NULL when the chain ends at something that is not visible.
CREATE OR REPLACE FUNCTION skill_registry.resolve_alias(p_skill UUID) RETURNS UUID AS $$
DECLARE
  v_cur UUID := p_skill;
  v_next UUID;
  v_status TEXT;
  i INT := 0;
BEGIN
  WHILE i < 8 LOOP
    SELECT status, alias_of INTO v_status, v_next FROM skill_registry.skill WHERE skill_id = v_cur;
    IF NOT FOUND THEN RETURN NULL; END IF;
    IF v_status IN ('provisional', 'admitted') THEN RETURN v_cur; END IF;
    IF v_next IS NULL THEN RETURN NULL; END IF;
    v_cur := v_next;
    i := i + 1;
  END LOOP;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql STABLE;

GRANT SELECT, INSERT ON skill_registry.skill_bundle TO app_role;
GRANT SELECT, INSERT, DELETE ON skill_registry.skill_proposal_bundle TO app_role;
GRANT SELECT, INSERT ON skill_registry.skill_verdict TO app_role;
GRANT SELECT, INSERT ON skill_registry.skill_notice TO app_role;
GRANT EXECUTE ON FUNCTION skill_registry.retire(UUID, TEXT, TEXT) TO app_role;
GRANT EXECUTE ON FUNCTION skill_registry.set_admitted(UUID) TO app_role;
GRANT EXECUTE ON FUNCTION skill_registry.move_scope(UUID, TEXT, UUID) TO app_role;
GRANT EXECUTE ON FUNCTION skill_registry.set_alias(UUID, UUID) TO app_role;
GRANT EXECUTE ON FUNCTION skill_registry.resolve_alias(UUID) TO app_role;

-- ── admit() is the only way in (RA-1.2) ─────────────────────────────────────
--
-- 0210 granted app_role INSERT and UPDATE on skill and INSERT on skill_version, so
-- "there is no INSERT into skill outside admit()" was a convention a second writer
-- could break. The functions that change a skill's standing now run with their
-- owner's rights and the direct table writes are taken back: the grants below leave
-- app_role able to EXECUTE admit(), retire(), set_admitted() and move_scope() and
-- unable to write the two tables any other way. A function that runs as its owner
-- pins its search_path, so a schema of the caller's cannot shadow a table it names.
ALTER FUNCTION skill_registry.admit(TEXT, TEXT, UUID, UUID, TEXT, TEXT, BOOLEAN, TEXT, TEXT, TEXT, INT, INT, TEXT, TEXT)
  SECURITY DEFINER SET search_path = skill_registry, pg_temp;
ALTER FUNCTION skill_registry.retire(UUID, TEXT, TEXT) SECURITY DEFINER SET search_path = skill_registry, pg_temp;
ALTER FUNCTION skill_registry.set_admitted(UUID) SECURITY DEFINER SET search_path = skill_registry, pg_temp;
ALTER FUNCTION skill_registry.move_scope(UUID, TEXT, UUID) SECURITY DEFINER SET search_path = skill_registry, pg_temp;
ALTER FUNCTION skill_registry.set_alias(UUID, UUID) SECURITY DEFINER SET search_path = skill_registry, pg_temp;
REVOKE INSERT, UPDATE ON skill_registry.skill FROM app_role;
REVOKE INSERT ON skill_registry.skill_version FROM app_role;
-- skill_bundle is inserted in the admitting transaction, after admit() has made the
-- version it references; that INSERT is granted above and is append-only.
