-- 0148 — deployments bound to a selection (Set U U-6.4; FEAT-007 §4; ADR-0040
-- decision 4).
--
-- A deployment can trade whatever a selection strategy picks. It is recorded as
-- a *parent* row with its `target` — the selection's revision, `top_n`, when it
-- rebalances, what happens to a leaver, how capital is divided — and each member
-- it trades is a *child* deployment of the trading strategy on that one
-- instrument, pointing at its parent. A child is an ordinary deployment: the
-- same runner, journal and checkpoint as one started by hand.
--
-- The parent's `instrument_id` names the binding, `selection:<revision>`, so the
-- existing one-running-deployment-per-(user, instrument, strategy) index also
-- refuses binding the same trading strategy to the same selection twice.
--
-- Every allocation change — a member entering, leaving (and how), letting its
-- strategy exit, being reweighted — is a row in `deployment_allocation_event`,
-- with the capital it was given and the selection run that caused it.
-- Append-only.

ALTER TABLE strategy_deployment
  ADD COLUMN IF NOT EXISTS target JSONB,
  ADD COLUMN IF NOT EXISTS parent_deployment_id UUID
    REFERENCES strategy_deployment (deployment_id);

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'chk_strategy_deployment_target'
  ) THEN
    ALTER TABLE strategy_deployment ADD CONSTRAINT chk_strategy_deployment_target CHECK (
      target IS NULL
      OR (
        target->>'kind' = 'selection'
        AND instrument_id = 'selection:' || (target->>'selection')
        AND parent_deployment_id IS NULL
      )
    );
  END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_strategy_deployment_parent
  ON strategy_deployment (parent_deployment_id)
  WHERE parent_deployment_id IS NOT NULL;

-- The record stays fixed at start, the two new columns included.
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
  OR NEW.forward_deployment_id IS DISTINCT FROM OLD.forward_deployment_id
  OR NEW.target            IS DISTINCT FROM OLD.target
  OR NEW.parent_deployment_id IS DISTINCT FROM OLD.parent_deployment_id THEN
    RAISE EXCEPTION 'strategy_deployment % is fixed at start; only stopping may change it', OLD.deployment_id;
  END IF;
  IF OLD.stopped_at IS NOT NULL THEN
    RAISE EXCEPTION 'strategy_deployment % is already stopped', OLD.deployment_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE IF NOT EXISTS deployment_allocation_event (
  deployment_id    UUID NOT NULL REFERENCES strategy_deployment (deployment_id),
  seq              BIGINT NOT NULL CHECK (seq > 0),
  at               TIMESTAMPTZ NOT NULL,
  -- The selection run that caused it; NULL for an exit that finished between
  -- runs.
  selection_run_id UUID REFERENCES selection_run (run_id),
  change           TEXT NOT NULL CHECK (change IN ('entered', 'left', 'exiting', 'reweighted')),
  instrument_id    TEXT NOT NULL CHECK (length(instrument_id) > 0),
  -- The member's child deployment, once it has one.
  member_deployment_id UUID REFERENCES strategy_deployment (deployment_id),
  -- Fraction of the parent's capital after the change; 0 for a leaver.
  weight           NUMERIC(38,18) NOT NULL CHECK (weight >= 0 AND weight <= 1),
  capital          NUMERIC(38,18) NOT NULL CHECK (capital >= 0),
  -- How a leaver leaves.
  action           TEXT CHECK (action IN ('close', 'let_strategy_exit', 'hold')),
  written_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (deployment_id, seq),
  CHECK ((change = 'left') = (action IS NOT NULL))
);

CREATE OR REPLACE FUNCTION deployment_allocation_event_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'deployment_allocation_event is append-only (Set U U-6.4)';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_deployment_allocation_event_append_only ON deployment_allocation_event;
CREATE TRIGGER trg_deployment_allocation_event_append_only
  BEFORE UPDATE OR DELETE ON deployment_allocation_event
  FOR EACH ROW EXECUTE FUNCTION deployment_allocation_event_append_only();

GRANT SELECT, INSERT ON deployment_allocation_event TO app_role;
