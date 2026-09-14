-- Set L Phase 1 (COMP-005, ADR-0030): one durable job service and a
-- content-addressed artifact store.
--
-- Replaces the ad-hoc handling of long work: the backtest manager's fixed
-- 3-concurrent semaphore, sweeps and suites held in `RwLock<HashMap>`, asset-init
-- jobs and training runs. A restart used to orphan compute and could double-count
-- trials; jobs here survive restarts via leases, and trials are counted at
-- submission inside the same transaction (INV-1, JB-03).

-- ---------------------------------------------------------------------------
-- Jobs
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS jobs (
  job_id         TEXT PRIMARY KEY,               -- 'job_' || ULID
  kind           TEXT NOT NULL,
  project_id     UUID,                           -- NULL for system jobs
  user_id        UUID NOT NULL,
  session_id     UUID,                           -- agent session, if any
  submitted_by   TEXT NOT NULL CHECK (submitted_by IN ('agent','user','system')),
  experiment_id  TEXT,                           -- required for evaluation-counted kinds
  parent_job_id  TEXT REFERENCES jobs(job_id) ON DELETE CASCADE,
  member_index   INT,                            -- child position under parent_job_id
  queue          TEXT NOT NULL CHECK (queue IN ('agent','human','system')),
  worker_class   TEXT NOT NULL,                  -- backtest | research | trainer | data | eval
  priority       SMALLINT NOT NULL DEFAULT 5,
  manifest       JSONB NOT NULL,                 -- canonical JSON (RFC 8785 JCS)
  manifest_hash  TEXT NOT NULL,                  -- sha256 hex, see COMP-005 §4
  state          TEXT NOT NULL
                 CHECK (state IN ('queued','leased','running','paused',
                                  'succeeded','failed','cancelled')),
  progress       JSONB NOT NULL DEFAULT '{}'::jsonb,
  estimate       JSONB,
  actual         JSONB,
  result_summary TEXT,                           -- <= 1536 bytes, written for the model
  result         JSONB,
  error          JSONB,                          -- {code, field, rule, fix, detail_ref}
  attempts       SMALLINT NOT NULL DEFAULT 0,
  lease_owner       TEXT,
  lease_expires_at  TIMESTAMPTZ,
  heartbeat_at      TIMESTAMPTZ,
  cancel_requested  BOOLEAN NOT NULL DEFAULT false,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at     TIMESTAMPTZ,
  finished_at    TIMESTAMPTZ
);

-- Idempotency (JB-02): one top-level job per (project, manifest_hash). Children are
-- keyed by (parent, member_index) instead and are never deduplicated across parents.
CREATE UNIQUE INDEX IF NOT EXISTS jobs_idem
  ON jobs (coalesce(project_id, '00000000-0000-0000-0000-000000000000'::uuid), manifest_hash)
  WHERE parent_job_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS jobs_member
  ON jobs (parent_job_id, member_index)
  WHERE parent_job_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS jobs_queue
  ON jobs (worker_class, queue, state, priority DESC, created_at);
CREATE INDEX IF NOT EXISTS jobs_project ON jobs (project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS jobs_parent  ON jobs (parent_job_id);
CREATE INDEX IF NOT EXISTS jobs_lease   ON jobs (state, lease_expires_at)
  WHERE state IN ('leased','running');

-- Terminal states are immutable (COMP-005 §5). Enforced in the database so no code
-- path — including a future one nobody has written yet — can resurrect a finished job
-- and, with it, an already-counted trial.
CREATE OR REPLACE FUNCTION jobs_terminal_immutable() RETURNS trigger AS $$
BEGIN
  IF OLD.state IN ('succeeded','failed','cancelled') AND NEW.state <> OLD.state THEN
    RAISE EXCEPTION 'job % is terminal (%) and cannot move to %',
      OLD.job_id, OLD.state, NEW.state
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS jobs_terminal_immutable_trg ON jobs;
CREATE TRIGGER jobs_terminal_immutable_trg
  BEFORE UPDATE ON jobs
  FOR EACH ROW EXECUTE FUNCTION jobs_terminal_immutable();

CREATE TABLE IF NOT EXISTS job_events (
  id         BIGSERIAL PRIMARY KEY,
  job_id     TEXT NOT NULL REFERENCES jobs ON DELETE CASCADE,
  kind       TEXT NOT NULL,                      -- state | progress | log_ref | checkpoint
  payload    JSONB NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS job_events_job ON job_events (job_id, id);
CREATE INDEX IF NOT EXISTS job_events_seq ON job_events (id);

-- ---------------------------------------------------------------------------
-- Artifacts
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS artifacts (
  handle       TEXT PRIMARY KEY,                 -- 'art_' || first 32 hex of sha256
  sha256       TEXT NOT NULL UNIQUE,
  type         TEXT NOT NULL,
  project_id   UUID,                             -- NULL = global
  uri          TEXT NOT NULL,
  size_bytes   BIGINT NOT NULL,
  xxh3         TEXT NOT NULL,
  manifest     JSONB NOT NULL,
  producer_job TEXT REFERENCES jobs(job_id) ON DELETE SET NULL,
  pinned       BOOLEAN NOT NULL DEFAULT false,
  expires_at   TIMESTAMPTZ,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS artifacts_project ON artifacts (project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS artifacts_expiry  ON artifacts (expires_at)
  WHERE pinned = false AND expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS job_artifacts (
  job_id TEXT NOT NULL REFERENCES jobs ON DELETE CASCADE,
  handle TEXT NOT NULL REFERENCES artifacts ON DELETE CASCADE,
  role   TEXT,
  PRIMARY KEY (job_id, handle)
);

-- A citation pins an artifact. Any row here keeps it alive past its TTL.
CREATE TABLE IF NOT EXISTS artifact_refs (
  handle   TEXT NOT NULL REFERENCES artifacts ON DELETE CASCADE,
  ref_kind TEXT NOT NULL,                        -- experiment | report | finding | skill | dataset | model
  ref_id   TEXT NOT NULL,
  PRIMARY KEY (handle, ref_kind, ref_id)
);

-- Keep `pinned` true for as long as at least one citation exists. Doing this with
-- triggers rather than in application code means a pin cannot be lost by a caller
-- that forgot to update the flag.
CREATE OR REPLACE FUNCTION artifact_pin_sync() RETURNS trigger AS $$
DECLARE target TEXT;
BEGIN
  target := coalesce(NEW.handle, OLD.handle);
  UPDATE artifacts a
     SET pinned = EXISTS (SELECT 1 FROM artifact_refs r WHERE r.handle = target)
   WHERE a.handle = target;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS artifact_pin_sync_trg ON artifact_refs;
CREATE TRIGGER artifact_pin_sync_trg
  AFTER INSERT OR DELETE ON artifact_refs
  FOR EACH ROW EXECUTE FUNCTION artifact_pin_sync();

-- ---------------------------------------------------------------------------
-- Exploration ledger (D-13, JB-11)
-- ---------------------------------------------------------------------------
--
-- Every data read and sandbox analysis is logged here. These are NOT trials: they
-- are the looking-around that precedes a hypothesis. Reporting them alongside a
-- verdict is what lets a reader judge how much searching produced it, and the
-- hypothesis registry uses the variables touched to decide whether a hypothesis was
-- formed post hoc.

CREATE TABLE IF NOT EXISTS exploration_ledger (
  id           BIGSERIAL PRIMARY KEY,
  project_id   UUID NOT NULL,
  user_id      UUID NOT NULL,
  session_id   UUID,
  source       TEXT NOT NULL CHECK (source IN ('data_api','job','desk')),
  instruments  TEXT[] NOT NULL DEFAULT '{}',
  timeframe    TEXT,
  window_start TIMESTAMPTZ,
  window_end   TIMESTAMPTZ,
  variables    TEXT[] NOT NULL DEFAULT '{}',
  description  TEXT NOT NULL,
  handle       TEXT,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS exploration_ledger_proj
  ON exploration_ledger (project_id, created_at);
