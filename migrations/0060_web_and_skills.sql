-- Web documents and the agent's Python skill library (ADR-P7-01 … ADR-P7-04).
--
-- Two tables that look unrelated and share one property: both hold things that
-- came from outside the platform's own reasoning, and both are only useful if
-- they can be looked at afterwards.

-- ── fetched web documents ────────────────────────────────────────────────────
--
-- Content-addressed, so the same page fetched twice is one row, and carrying a
-- `knowledge_time` that is **when the platform learned it** rather than any date
-- printed on the page.
--
-- That column is the point of the table. Most systems adding web access worry
-- about prompt injection; this one has that problem and a second one that is
-- easier to miss: a fetched page is dated, and using it inside a backtest is
-- look-ahead. An agent researching 2024 that reads a 2026 article about how 2024
-- turned out has leaked the future into its hypothesis, and no statistical
-- correction downstream can see it, because the contamination is in the proposer
-- rather than the candidate.
--
-- So a research context with a cutoff filters on `knowledge_time`, exactly as it
-- filters bars. The column is NOT NULL and has no default beyond `now()`.
CREATE TABLE IF NOT EXISTS mlops.web_document (
  body_sha256     TEXT NOT NULL CHECK (body_sha256 ~ '^[0-9a-f]{64}$'),
  tenant_id       TEXT NOT NULL,
  requested_url   TEXT NOT NULL,
  final_url       TEXT NOT NULL,
  status          INT  NOT NULL,
  content_type    TEXT NOT NULL,
  title           TEXT,
  -- Extracted text. Stored as text and **never** interpolated into a prompt
  -- without the fence in `webfetch::fence` — a reader of this column is reading
  -- something a stranger wrote.
  text            TEXT NOT NULL,
  bytes           INT  NOT NULL,
  truncated       BOOLEAN NOT NULL DEFAULT FALSE,
  knowledge_time  TIMESTAMPTZ NOT NULL DEFAULT now(),
  fetched_by      TEXT NOT NULL,

  PRIMARY KEY (tenant_id, body_sha256)
);

CREATE INDEX IF NOT EXISTS idx_web_document_url
  ON mlops.web_document(tenant_id, final_url, knowledge_time DESC);
-- The index a point-in-time read uses.
CREATE INDEX IF NOT EXISTS idx_web_document_knowledge
  ON mlops.web_document(tenant_id, knowledge_time);

-- A document is what it was when it was fetched. Re-fetching a changed page
-- produces a different hash and therefore a different row, which is the whole
-- reason the primary key is the content.
DROP TRIGGER IF EXISTS trg_web_document_immutable ON mlops.web_document;
CREATE TRIGGER trg_web_document_immutable
  BEFORE UPDATE OR DELETE ON mlops.web_document
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── the skill library ────────────────────────────────────────────────────────
--
-- Content-addressed and append-only, for the same reason `strategy_revision` is
-- (ADR-P6-04): a run recorded last week must not name code that no longer
-- exists. `name` is a pointer; `skill_hash` is the identity.
CREATE TABLE IF NOT EXISTS mlops.agent_skill (
  skill_hash   TEXT NOT NULL CHECK (skill_hash ~ '^[0-9a-f]{64}$'),
  tenant_id    TEXT NOT NULL,
  name         TEXT NOT NULL CHECK (name ~ '^[a-z][a-z0-9_]{2,63}$'),
  -- Read by the agent when choosing between skills, which is the difference
  -- between a library and a pile of files. Twenty characters is not a style
  -- rule: "helper" cannot distinguish two of these.
  description  TEXT NOT NULL CHECK (length(trim(description)) >= 20),
  source       TEXT NOT NULL CHECK (length(source) BETWEEN 1 AND 262144),
  -- The declared interface: two key lists and a sentence.
  contract     JSONB NOT NULL,
  parent_hash  TEXT,
  authored_by  TEXT NOT NULL,
  actor_kind   TEXT NOT NULL CHECK (actor_kind IN ('human','agent')),
  authored_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

  PRIMARY KEY (tenant_id, skill_hash),
  CONSTRAINT chk_skill_not_its_own_parent CHECK (parent_hash IS DISTINCT FROM skill_hash),
  -- `cardinality`, never `array_length`: the latter returns NULL for an empty
  -- array and a CHECK evaluating to NULL *passes*. A skill that promises no
  -- outputs cannot be checked, and one that cannot be checked cannot accumulate
  -- the record this whole design rests on.
  CONSTRAINT chk_skill_declares_outputs CHECK (
    jsonb_typeof(contract->'outputs') = 'array'
    AND jsonb_array_length(contract->'outputs') >= 1
  )
);

CREATE INDEX IF NOT EXISTS idx_agent_skill_name
  ON mlops.agent_skill(tenant_id, name, authored_at DESC);

DROP TRIGGER IF EXISTS trg_agent_skill_immutable ON mlops.agent_skill;
CREATE TRIGGER trg_agent_skill_immutable
  BEFORE UPDATE OR DELETE ON mlops.agent_skill
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- The pointer from a name to its current revision, so `run_skill('x')` resolves.
CREATE TABLE IF NOT EXISTS mlops.agent_skill_head (
  tenant_id   TEXT NOT NULL,
  name        TEXT NOT NULL,
  skill_hash  TEXT NOT NULL,
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (tenant_id, name),
  FOREIGN KEY (tenant_id, skill_hash) REFERENCES mlops.agent_skill(tenant_id, skill_hash)
);

-- ── every run, including the failures ────────────────────────────────────────
--
-- Especially the failures. A library that keeps only its successes has
-- statistics saying every skill is perfect, and the entire "gets better over
-- time" claim is that the agent can tell a working skill from a broken one
-- without anybody training anything.
CREATE TABLE IF NOT EXISTS mlops.skill_run (
  run_id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id         TEXT NOT NULL,
  skill_hash        TEXT NOT NULL,
  -- 'bad_input' and 'runner_failed' are facts about the CALLER and the
  -- PLATFORM. `mlops.skill_stats` excludes them from reliability on purpose: a
  -- good skill handed bad payloads must not read as unreliable, because that is
  -- the number the agent would then act on.
  outcome           TEXT NOT NULL CHECK (outcome IN (
                      'ok','bad_input','raised','broke_contract',
                      'timed_out','killed','runner_failed')),
  detail            TEXT,
  duration_ms       BIGINT NOT NULL CHECK (duration_ms >= 0),
  peak_memory_bytes BIGINT,
  ran_by            TEXT NOT NULL,
  ran_at            TIMESTAMPTZ NOT NULL DEFAULT now(),

  FOREIGN KEY (tenant_id, skill_hash) REFERENCES mlops.agent_skill(tenant_id, skill_hash)
);

CREATE INDEX IF NOT EXISTS idx_skill_run_skill
  ON mlops.skill_run(tenant_id, skill_hash, ran_at DESC);

DROP TRIGGER IF EXISTS trg_skill_run_immutable ON mlops.skill_run;
CREATE TRIGGER trg_skill_run_immutable
  BEFORE UPDATE OR DELETE ON mlops.skill_run
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── what a skill's history adds up to ────────────────────────────────────────
--
-- `reliability` is NULL until something attributable has happened. That is
-- deliberate and is the same rule as the five-state self-monitor: 0/0 is not
-- zero and not one, and rendering either turns an absence of evidence into a
-- number people read as evidence.
CREATE OR REPLACE VIEW mlops.skill_stats AS
SELECT
  s.tenant_id,
  s.skill_hash,
  s.name,
  s.description,
  count(r.run_id)                                              AS runs,
  count(*) FILTER (WHERE r.outcome = 'ok')                     AS ok,
  count(*) FILTER (WHERE r.outcome IN ('raised','broke_contract','timed_out','killed')) AS failures,
  count(*) FILTER (WHERE r.outcome = 'bad_input')              AS bad_input,
  count(*) FILTER (WHERE r.outcome = 'runner_failed')          AS runner_failed,
  CASE
    WHEN count(*) FILTER (WHERE r.outcome IN ('ok','raised','broke_contract','timed_out','killed')) = 0
      THEN NULL
    ELSE count(*) FILTER (WHERE r.outcome = 'ok')::float8
       / count(*) FILTER (WHERE r.outcome IN ('ok','raised','broke_contract','timed_out','killed'))
  END                                                          AS reliability,
  avg(r.duration_ms)                                           AS mean_duration_ms,
  max(r.ran_at)                                                AS last_run_at
FROM mlops.agent_skill s
LEFT JOIN mlops.skill_run r
       ON r.tenant_id = s.tenant_id AND r.skill_hash = s.skill_hash
GROUP BY s.tenant_id, s.skill_hash, s.name, s.description;

-- ── tenancy and grants ───────────────────────────────────────────────────────
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['web_document','agent_skill','agent_skill_head','skill_run'] LOOP
    EXECUTE format('ALTER TABLE mlops.%I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE mlops.%I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON mlops.%I', t);
    EXECUTE format($p$
      CREATE POLICY tenant_isolation ON mlops.%I
        USING (tenant_id = current_setting('app.tenant_id', true))
        WITH CHECK (tenant_id = current_setting('app.tenant_id', true))
    $p$, t);
  END LOOP;
END $$;

-- The agent writes its own skills and reads its own history. `agent_skill_head`
-- takes an UPDATE because moving a name to a new revision is the one mutable
-- thing here, and it is a pointer move rather than an edit.
GRANT SELECT, INSERT ON mlops.web_document, mlops.agent_skill, mlops.skill_run TO agent_role;
GRANT SELECT, INSERT, UPDATE ON mlops.agent_skill_head TO agent_role;
GRANT SELECT ON mlops.skill_stats TO agent_role, app_role, internal_ml_role;
GRANT SELECT, INSERT, UPDATE ON
  mlops.web_document, mlops.agent_skill, mlops.agent_skill_head, mlops.skill_run
  TO app_role;
