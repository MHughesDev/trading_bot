-- 0142 — the deployment record (Set U U-4.2; FEAT-007 §1 defect 10, §9.5).
--
-- Until this table a running strategy left no record that it was running.
-- `mlops.paper_deployment` (0105) was designed for forward evidence and never
-- had a writer, so a deployment's revision, its starting capital and who
-- started it were nowhere — and a realised series cannot be measured from a
-- base nobody wrote down.
--
-- One row per deployment of a `tbot.strategy/3` graph. Starting writes it;
-- stopping closes it, once. Every other column is fixed at start: a deployment
-- whose strategy, instrument or capital could be edited after the fact is not
-- a record of what ran.
--
-- Set T's proposal path sits in front of this: when a deployment is also a
-- forward-evidence deployment it names its `mlops.paper_deployment` row in
-- `forward_deployment_id`. A plain paper deployment has none.

CREATE TABLE IF NOT EXISTS strategy_deployment (
  deployment_id     UUID PRIMARY KEY,
  user_id           TEXT NOT NULL CHECK (length(user_id) > 0),
  strategy_name     TEXT NOT NULL CHECK (length(strategy_name) > 0),
  -- The graph's content-addressed revision (sha256 of its canonical JSON).
  revision_hash     TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
  -- The graph itself, so the record reproduces what ran without a join to a
  -- table whose head may have moved.
  definition_json   JSONB NOT NULL,
  instrument_id     TEXT NOT NULL CHECK (length(instrument_id) > 0),
  -- Paper only. A live environment is roadmap gate H-4, and widening this is a
  -- migration and a conversation, not a value typed into a request.
  environment       TEXT NOT NULL CHECK (environment IN ('paper')),
  -- The platform's own paper venue (ADR-0048).
  venue             TEXT NOT NULL CHECK (venue IN ('platform_paper')),
  starting_capital  NUMERIC(38,18) NOT NULL CHECK (starting_capital > 0),
  -- Every model the graph calls, at its pinned version: [{node, model, version}].
  model_pins        JSONB NOT NULL DEFAULT '[]'::jsonb,
  started_by        TEXT NOT NULL CHECK (length(started_by) > 0),
  started_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  stopped_at        TIMESTAMPTZ,
  stop_reason       TEXT,
  forward_deployment_id UUID,
  CONSTRAINT chk_strategy_deployment_stop_pair
    CHECK ((stopped_at IS NULL) = (stop_reason IS NULL))
);

-- One running deployment per (user, instrument, strategy). A second start of the
-- same binding is a duplicate, refused here even if the in-process registry is
-- bypassed.
CREATE UNIQUE INDEX IF NOT EXISTS uq_strategy_deployment_running
  ON strategy_deployment (user_id, instrument_id, strategy_name)
  WHERE stopped_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_strategy_deployment_running
  ON strategy_deployment (started_at)
  WHERE stopped_at IS NULL;

-- Fixed at start; closed once.
CREATE OR REPLACE FUNCTION strategy_deployment_immutable() RETURNS trigger AS $$
BEGIN
  IF NEW.deployment_id     IS DISTINCT FROM OLD.deployment_id
  OR NEW.user_id           IS DISTINCT FROM OLD.user_id
  OR NEW.strategy_name     IS DISTINCT FROM OLD.strategy_name
  OR NEW.revision_hash     IS DISTINCT FROM OLD.revision_hash
  OR NEW.definition_json   IS DISTINCT FROM OLD.definition_json
  OR NEW.instrument_id     IS DISTINCT FROM OLD.instrument_id
  OR NEW.environment       IS DISTINCT FROM OLD.environment
  OR NEW.venue             IS DISTINCT FROM OLD.venue
  OR NEW.starting_capital  IS DISTINCT FROM OLD.starting_capital
  OR NEW.model_pins        IS DISTINCT FROM OLD.model_pins
  OR NEW.started_by        IS DISTINCT FROM OLD.started_by
  OR NEW.started_at        IS DISTINCT FROM OLD.started_at
  OR NEW.forward_deployment_id IS DISTINCT FROM OLD.forward_deployment_id THEN
    RAISE EXCEPTION 'strategy_deployment % is fixed at start; only stopping may change it', OLD.deployment_id;
  END IF;
  IF OLD.stopped_at IS NOT NULL THEN
    RAISE EXCEPTION 'strategy_deployment % is already stopped', OLD.deployment_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_deployment_immutable ON strategy_deployment;
CREATE TRIGGER trg_strategy_deployment_immutable
  BEFORE UPDATE ON strategy_deployment
  FOR EACH ROW EXECUTE FUNCTION strategy_deployment_immutable();
