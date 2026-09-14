-- Set L (AGENT-001 §5, DATA-005 §4, ADR-0024/0025): research projects and agent
-- sessions.
--
-- A research project is the unit of work: a workspace, a memory scope, a budget, and
-- — the part that carries the honesty guarantee — a **research cutoff**. The Data API
-- never returns data after a project's cutoff to a research token, so the holdout is
-- enforced by the service rather than asked of the agent (D-10, DA-03).

CREATE TABLE IF NOT EXISTS research_projects (
  project_id        UUID PRIMARY KEY,
  user_id           UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('research','desk')),
  name              TEXT NOT NULL,
  goal              TEXT,
  instruments       TEXT[] NOT NULL DEFAULT '{}',

  -- NULL means "now", which is the Desk project. A research project always has a
  -- concrete cutoff: everything after it is holdout.
  research_cutoff   TIMESTAMPTZ,
  holdout_len       INTERVAL,

  project_version   INT NOT NULL DEFAULT 1,
  model             TEXT NOT NULL DEFAULT 'claude-opus-5',
  effort            TEXT NOT NULL DEFAULT 'high',
  image_version     TEXT NOT NULL DEFAULT 'dev',
  budget_usd_day    NUMERIC(12,2),
  budget_usd_total  NUMERIC(12,2),
  budget_compute_s  BIGINT,
  tier              TEXT NOT NULL DEFAULT 'standard',
  workspace_volume  TEXT NOT NULL DEFAULT '',
  status            TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived')),
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- A research project without a cutoff would have no holdout at all, and a Desk
  -- project with one would not be able to answer "what is BTC doing now". Neither is
  -- a state the rest of the system knows how to interpret, so the database refuses
  -- both rather than leaving it to every caller to remember.
  CONSTRAINT chk_cutoff_matches_kind CHECK (
    (kind = 'desk'     AND research_cutoff IS NULL) OR
    (kind = 'research' AND research_cutoff IS NOT NULL)
  )
);

-- Exactly one Desk per user (DA-15). The Desk is where live questions go; having two
-- would make "the Desk" ambiguous in every API that takes a user rather than a
-- project.
CREATE UNIQUE INDEX IF NOT EXISTS one_desk_per_user
  ON research_projects (user_id) WHERE kind = 'desk';
CREATE INDEX IF NOT EXISTS research_projects_user
  ON research_projects (user_id, created_at DESC);

-- The cutoff is immutable once the project has an Experiment (DA-05).
--
-- This is the rule that makes a holdout mean anything. Without it a researcher who
-- did not like a result could move the cutoff forward, re-run, and call the second
-- answer the real one — and nothing in the data would show that had happened. It is
-- enforced here, in the database, because an API-level check protects only the
-- callers that go through that API.
CREATE OR REPLACE FUNCTION research_cutoff_immutable() RETURNS trigger AS $$
DECLARE experiment_count INT;
BEGIN
  IF NEW.research_cutoff IS DISTINCT FROM OLD.research_cutoff THEN
    SELECT count(*) INTO experiment_count
      FROM jobs
     WHERE project_id = OLD.project_id AND experiment_id IS NOT NULL;
    IF experiment_count > 0 THEN
      RAISE EXCEPTION
        'research_cutoff is immutable once the project has an experiment (% found)',
        experiment_count
        USING ERRCODE = 'check_violation';
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS research_cutoff_immutable_trg ON research_projects;
CREATE TRIGGER research_cutoff_immutable_trg
  BEFORE UPDATE ON research_projects
  FOR EACH ROW EXECUTE FUNCTION research_cutoff_immutable();

-- A project's kind must never change either: flipping a research project to a desk
-- would drop its cutoff and hand the agent the holdout.
CREATE OR REPLACE FUNCTION research_project_kind_immutable() RETURNS trigger AS $$
BEGIN
  IF NEW.kind IS DISTINCT FROM OLD.kind THEN
    RAISE EXCEPTION 'project kind is immutable (% -> %)', OLD.kind, NEW.kind
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS research_project_kind_immutable_trg ON research_projects;
CREATE TRIGGER research_project_kind_immutable_trg
  BEFORE UPDATE ON research_projects
  FOR EACH ROW EXECUTE FUNCTION research_project_kind_immutable();

-- ---------------------------------------------------------------------------
-- Sessions
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS agent_sessions (
  session_id      UUID PRIMARY KEY,
  project_id      UUID NOT NULL REFERENCES research_projects ON DELETE CASCADE,
  user_id         UUID NOT NULL,
  state           TEXT NOT NULL DEFAULT 'starting',
  is_initializer  BOOLEAN NOT NULL DEFAULT false,
  project_version INT NOT NULL DEFAULT 1,
  sdk_session_ref TEXT,                 -- ClaudeAgentOptions.resume handle
  container_id    TEXT,
  report_id       TEXT,
  abort_reason    TEXT,
  spend_usd       NUMERIC(12,4) NOT NULL DEFAULT 0,
  started_at      TIMESTAMPTZ,
  ended_at        TIMESTAMPTZ,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS agent_sessions_project
  ON agent_sessions (project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_events (
  id         BIGSERIAL PRIMARY KEY,
  session_id UUID NOT NULL REFERENCES agent_sessions ON DELETE CASCADE,
  seq        INT NOT NULL,
  kind       TEXT NOT NULL,
  payload    JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (session_id, seq)
);

CREATE TABLE IF NOT EXISTS approval_requests (
  approval_id    UUID PRIMARY KEY,
  project_id     UUID NOT NULL REFERENCES research_projects ON DELETE CASCADE,
  session_id     UUID,
  kind           TEXT NOT NULL,
  payload        JSONB NOT NULL,
  options        JSONB,
  default_option TEXT,
  timeout_at     TIMESTAMPTZ,
  state          TEXT NOT NULL DEFAULT 'pending'
                 CHECK (state IN ('pending','answered','defaulted','cancelled')),
  answer         JSONB,
  answered_by    UUID,
  answered_at    TIMESTAMPTZ,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS approval_requests_pending
  ON approval_requests (project_id, state, created_at);

CREATE TABLE IF NOT EXISTS llm_usage (
  id            BIGSERIAL PRIMARY KEY,
  session_id    UUID,
  project_id    UUID,
  user_id       UUID NOT NULL,
  role          TEXT,
  model         TEXT NOT NULL,
  input_tokens  BIGINT NOT NULL DEFAULT 0,
  output_tokens BIGINT NOT NULL DEFAULT 0,
  cache_read    BIGINT NOT NULL DEFAULT 0,
  cache_write   BIGINT NOT NULL DEFAULT 0,
  cost_usd      NUMERIC(12,6) NOT NULL DEFAULT 0,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS llm_usage_project ON llm_usage (project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS llm_usage_session ON llm_usage (session_id, created_at DESC);
