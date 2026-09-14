-- Set L Phase 6 (DATA-005 §9, AGENT-004): the synthetic venue registry and the
-- eval suite's run records.
--
-- Two tables and one rule. The rule is the reason the tables exist separately from
-- everything else: an eval measures whether the agent can find an edge that is
-- really there and refuse one that is not, and that measurement is void the moment
-- the agent can read the answer. So the planted mechanism lives in its own column,
-- behind its own scope, and the catalogue view the agent reads does not join to it.

-- ── Synthetic instruments ────────────────────────────────────────────────────
--
-- Bars themselves go to ClickHouse `market_bars_v2` with `venue_id='synthetic'`,
-- and flow through every endpoint exactly like real ones (DA-13). This table is
-- the catalogue entry: what generated them, and — separately — what was planted.
CREATE TABLE IF NOT EXISTS synthetic_instruments (
  instrument_id   TEXT PRIMARY KEY,
  generator       TEXT NOT NULL,
  seed            BIGINT NOT NULL,
  timeframe       TEXT NOT NULL,
  length          BIGINT NOT NULL,
  start_time      TIMESTAMPTZ NOT NULL,
  -- The generator parameters. Visible only with `evals.truth`: `phi = 0.05` is the
  -- answer to the task written on the box it came in.
  params          JSONB NOT NULL,
  -- The planted mechanism, strength and whether an edge exists at all. The grader
  -- reads this; the agent has no scope that can.
  truth           JSONB NOT NULL,
  -- Redacted metadata, safe to return to any reader: bar count, timeframe, span.
  public_meta     JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_by      UUID REFERENCES users(user_id) ON DELETE SET NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS synthetic_by_generator
  ON synthetic_instruments (generator, seed);

-- A synthetic instrument is immutable once written. Re-running the same
-- (generator, params, seed) must produce the same series, so an UPDATE either
-- changes nothing or means the generator drifted — and a silently redefined
-- instrument would invalidate every scorecard that referenced it.
CREATE OR REPLACE FUNCTION synthetic_instrument_immutable() RETURNS trigger AS $$
BEGIN
  IF NEW.generator IS DISTINCT FROM OLD.generator
     OR NEW.seed IS DISTINCT FROM OLD.seed
     OR NEW.params IS DISTINCT FROM OLD.params
     OR NEW.truth IS DISTINCT FROM OLD.truth
     OR NEW.length IS DISTINCT FROM OLD.length
     OR NEW.start_time IS DISTINCT FROM OLD.start_time THEN
    RAISE EXCEPTION
      'synthetic instrument % is immutable: a redefined generator invalidates every scorecard that cited it',
      OLD.instrument_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_synthetic_instrument_immutable ON synthetic_instruments;
CREATE TRIGGER trg_synthetic_instrument_immutable
  BEFORE UPDATE ON synthetic_instruments
  FOR EACH ROW EXECUTE FUNCTION synthetic_instrument_immutable();

-- ── Eval runs ────────────────────────────────────────────────────────────────
--
-- One row per trial: suite, task, seed, the session it ran, and the grade.
CREATE TABLE IF NOT EXISTS eval_runs (
  eval_run_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  -- The harness version this trial measures (AGENT-004 §6: scorecards are
  -- published per harness version, so a result without one cannot be compared).
  harness_version TEXT NOT NULL,
  suite           TEXT NOT NULL,
  task_id         TEXT NOT NULL,
  seed            BIGINT NOT NULL,
  -- The isolated project created for this trial, and the session that ran in it.
  project_id      UUID REFERENCES research_projects(project_id) ON DELETE SET NULL,
  session_id      UUID,
  job_id          TEXT,
  instrument_id   TEXT REFERENCES synthetic_instruments(instrument_id) ON DELETE SET NULL,
  status          TEXT NOT NULL DEFAULT 'running'
                  CHECK (status IN ('running','graded','error','triaged_grader_bug')),
  passed          BOOLEAN,
  -- Grader output: the rule that decided, plus its evidence.
  grade           JSONB NOT NULL DEFAULT '{}'::jsonb,
  -- Cost, so "dollars per correct verdict" is a query rather than an estimate.
  cost_usd        NUMERIC(12,6),
  tokens_in       BIGINT,
  tokens_out      BIGINT,
  cache_read      BIGINT,
  wall_clock_s    DOUBLE PRECISION,
  -- Every failure is triaged for grader bugs before it is counted, and the label
  -- is stored (AGENT-004 §5). A failure with no triage is not yet a data point.
  triage          TEXT,
  transcript_ref  TEXT,
  started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  finished_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS eval_runs_scorecard
  ON eval_runs (harness_version, suite, task_id);
CREATE INDEX IF NOT EXISTS eval_runs_recent ON eval_runs (started_at DESC);

-- A graded trial is final. Re-grading a trial after seeing the scorecard is how a
-- suite stops measuring anything, so the row locks on the way out.
CREATE OR REPLACE FUNCTION eval_run_grade_is_final() RETURNS trigger AS $$
BEGIN
  IF OLD.status IN ('graded','triaged_grader_bug')
     AND (NEW.passed IS DISTINCT FROM OLD.passed
          OR NEW.grade IS DISTINCT FROM OLD.grade) THEN
    RAISE EXCEPTION
      'eval run % is already graded: re-grading after the fact is how a suite stops measuring anything',
      OLD.eval_run_id
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_eval_run_grade_is_final ON eval_runs;
CREATE TRIGGER trg_eval_run_grade_is_final
  BEFORE UPDATE ON eval_runs
  FOR EACH ROW EXECUTE FUNCTION eval_run_grade_is_final();

-- ── Scorecards ───────────────────────────────────────────────────────────────
--
-- The published aggregate for one harness version (AGENT-004 §6). Kept as a table
-- rather than a view because the non-inferiority gate (§7) compares a PR's run
-- against a *baseline that was published at the time*, and a view would silently
-- re-derive the baseline from whatever the data says today.
CREATE TABLE IF NOT EXISTS eval_scorecards (
  scorecard_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  harness_version TEXT NOT NULL,
  -- 'baseline' marks the one the gate compares against.
  label           TEXT NOT NULL DEFAULT 'run',
  metrics         JSONB NOT NULL,
  trials          BIGINT NOT NULL,
  published_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (harness_version, label, published_at)
);

CREATE INDEX IF NOT EXISTS eval_scorecards_version
  ON eval_scorecards (harness_version, published_at DESC);

-- ── One more scope the agent may never hold ──────────────────────────────────
--
-- `evals.truth` reads the planted mechanism. AGENT-004 §2 states the requirement
-- as "generator parameters and planted-edge specifics are never visible to agent
-- tokens", and a requirement stated only in a document is a requirement that holds
-- until someone is in a hurry. Redefining the function is how a capability is added
-- to the list, and it is the same reviewable event that migration 0038 describes.
CREATE OR REPLACE FUNCTION agent_scopes_are_allowed() RETURNS trigger AS $$
DECLARE
  forbidden TEXT[] := ARRAY[
    'data.holdout',      -- the one-shot vault; only the vault-gate service reads it
    'orders.place',      -- research authority only (D-12)
    'orders.cancel',
    'automations.arm',
    'models.promote',    -- alias promotion is a human decision
    'skills.admit',      -- the platform admits skills, never the agent (ADR-0028)
    'evals.truth',       -- the answer key to its own evaluation (AGENT-004 §2)
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

-- ── The session inbox (AGENT-001 §16, UI-03) ─────────────────────────────────
--
-- Steering, interrupt and `ask_user` answers travel through a table rather than an
-- in-process channel, for one reason: the thing being steered is a container the
-- platform does not share memory with, and which may outlive the API process that
-- took the instruction. A steering message dropped because a web node restarted
-- would be invisible — the user typed it, the UI acknowledged it, and the agent
-- never saw it.
--
-- The agent-host drains this with the same lease discipline the job service uses.
CREATE TABLE IF NOT EXISTS session_inbox (
  id            BIGSERIAL PRIMARY KEY,
  session_id    UUID NOT NULL REFERENCES agent_sessions ON DELETE CASCADE,
  kind          TEXT NOT NULL
                CHECK (kind IN ('steer','interrupt','stop','answer')),
  body          JSONB NOT NULL,
  -- Who sent it. An instruction with no author cannot be audited, and steering is
  -- exactly the surface where "who told it to do that" gets asked later.
  sent_by       UUID NOT NULL,
  delivered_at  TIMESTAMPTZ,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS session_inbox_undelivered
  ON session_inbox (session_id, id)
  WHERE delivered_at IS NULL;

-- Delivery is recorded once. Re-delivering a steering message would repeat an
-- instruction the agent has already acted on, which is worse than losing one.
CREATE OR REPLACE FUNCTION session_inbox_delivery_is_once() RETURNS trigger AS $$
BEGIN
  IF OLD.delivered_at IS NOT NULL AND NEW.delivered_at IS DISTINCT FROM OLD.delivered_at THEN
    RAISE EXCEPTION
      'inbox message % was already delivered at %; re-delivering repeats an instruction the agent has acted on',
      OLD.id, OLD.delivered_at
      USING ERRCODE = 'check_violation';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_session_inbox_delivery_is_once ON session_inbox;
CREATE TRIGGER trg_session_inbox_delivery_is_once
  BEFORE UPDATE ON session_inbox
  FOR EACH ROW EXECUTE FUNCTION session_inbox_delivery_is_once();
