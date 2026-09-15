-- 0075 — the scoped, persisted kill switch, its event log, and per-strategy state
-- (COMP-007 §5, §4, §6; S-7.3, S-7.5…S-7.8, S-7.10).
--
-- What this replaces: an `AtomicBool` in `crates/risk/src/kill_switch.rs` with
-- `trip()` and `reset()`, persisted by "the caller". Nothing tripped it, nothing
-- scoped it, nothing recorded why, and it died with the process it was meant to
-- stop. Four separate ways for a halt to be indistinguishable from a bug.
--
-- `tripped_by` carries an `agent:` shape that must never appear. The column is not
-- there because agents might trip the switch; it is there so that "no agent ever
-- tripped or reset this" is a query anybody can run rather than a claim in a doc.
-- The CHECK makes writing such a row impossible, which is the only version of that
-- promise a database can keep.

-- The current state of each scope. History lives in monitor.events: one row per
-- scope here means the hot-path mirror refresh is a single small scan, and means
-- "is this venue halted" has exactly one answer.
CREATE TABLE IF NOT EXISTS monitor.kill_switch (
  scope          TEXT        PRIMARY KEY,
  state          TEXT        NOT NULL,
  -- 'monitor' | 'human:<user_id>' | 'drill'. Never 'agent:<run>'.
  tripped_by     TEXT,
  reason         TEXT,
  metric         TEXT,
  observed_value NUMERIC(38,18),
  limit_value    NUMERIC(38,18),
  tripped_at     TIMESTAMPTZ,
  reset_by       TEXT,
  reset_reason   TEXT,
  reset_at       TIMESTAMPTZ,
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

  CONSTRAINT monitor_kill_switch_scope_shape CHECK (
    scope = 'global'
    OR scope LIKE 'venue:%'
    OR scope LIKE 'strategy:%'
    OR scope LIKE 'drill:%'
  ),
  CONSTRAINT monitor_kill_switch_state CHECK (state IN ('armed', 'halted')),

  -- COMP-007 §5, §8.6. Two constraints, not one: an agent that cannot trip but
  -- can reset is an agent that can turn the safety off.
  CONSTRAINT monitor_kill_switch_never_tripped_by_an_agent
    CHECK (tripped_by IS NULL OR tripped_by NOT LIKE 'agent:%'),
  CONSTRAINT monitor_kill_switch_never_reset_by_an_agent
    CHECK (reset_by IS NULL OR reset_by NOT LIKE 'agent:%'),

  -- A halt with no attribution and no reason is a halt nobody can act on; the
  -- operator's first question is always "why", and P5 says the record answers it.
  CONSTRAINT monitor_kill_switch_halted_is_attributed CHECK (
    state <> 'halted' OR (tripped_by IS NOT NULL AND reason IS NOT NULL AND tripped_at IS NOT NULL)
  ),
  -- A reset is human and reasoned or it is not a reset (§5).
  CONSTRAINT monitor_kill_switch_reset_is_reasoned CHECK (
    reset_at IS NULL OR (reset_by IS NOT NULL AND reset_reason IS NOT NULL)
  )
);

COMMENT ON COLUMN monitor.kill_switch.tripped_by IS
  'COMP-007 §5: agents can never trip or reset; the agent: shape exists to prove it stays absent.';

-- Every state change in §4, exactly one row. This is the UI timeline, the audit
-- evidence, and the input to the monitor's own precision score (§4.1).
CREATE TABLE IF NOT EXISTS monitor.events (
  event_id       UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  occurred_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  scope          TEXT        NOT NULL,
  family         TEXT        NOT NULL,
  kind           TEXT        NOT NULL,
  metric         TEXT,
  observed_value NUMERIC(38,18),
  limit_value    NUMERIC(38,18),
  from_state     TEXT,
  to_state       TEXT,
  actor          TEXT        NOT NULL,
  reason         TEXT,
  -- A drill exercises the whole path (§4.2). Marking the row is what lets the desk
  -- show DRILL and what keeps a rehearsal out of the precision numerator.
  is_drill       BOOLEAN     NOT NULL DEFAULT FALSE,
  -- Suppressed by an open maintenance window (§4.3). Recorded rather than dropped:
  -- a window that erases evidence is a hole nobody can audit afterwards.
  suppressed_by_window UUID,

  CONSTRAINT monitor_events_family CHECK (family IN (
    'execution', 'behaviour', 'exposure', 'pnl', 'data', 'venue', 'platform', 'lease'
  )),
  CONSTRAINT monitor_events_kind CHECK (kind IN (
    'blip', 'warning', 'breach', 'scaled_down', 'scale_released', 'halted',
    'resumed', 'tripped', 'reset', 'window_opened', 'window_closed', 'drill'
  )),
  CONSTRAINT monitor_events_never_authored_by_an_agent
    CHECK (actor NOT LIKE 'agent:%')
);

CREATE INDEX IF NOT EXISTS monitor_events_recent ON monitor.events (occurred_at DESC);
CREATE INDEX IF NOT EXISTS monitor_events_scope ON monitor.events (scope, occurred_at DESC);

-- The per-strategy state machine (§4) plus its dwell counters (§4.1), persisted so
-- a monitor restart does not throw away a strategy's progress through a sustained
-- breach and start the three-cadence count again.
CREATE TABLE IF NOT EXISTS monitor.strategy_state (
  strategy_id      TEXT        PRIMARY KEY,
  state            TEXT        NOT NULL DEFAULT 'nominal',
  scale_pct        SMALLINT    NOT NULL DEFAULT 100 CHECK (scale_pct BETWEEN 0 AND 100),
  halt_policy      TEXT        NOT NULL DEFAULT 'hold' CHECK (halt_policy IN ('hold', 'flatten', 'reduce')),
  last_evaluated_at TIMESTAMPTZ,
  updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT monitor_strategy_state_state CHECK (
    state IN ('nominal', 'warning', 'breach', 'scaled_down', 'halted')
  )
);

CREATE TABLE IF NOT EXISTS monitor.dwell (
  strategy_id      TEXT     NOT NULL,
  metric           TEXT     NOT NULL,
  consecutive_outside INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_outside >= 0),
  consecutive_inside  INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_inside >= 0),
  updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (strategy_id, metric)
);

CREATE OR REPLACE FUNCTION monitor.notify_kill_switch() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('monitor_kill_switch', NEW.scope);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS monitor_kill_switch_notify_trg ON monitor.kill_switch;
CREATE TRIGGER monitor_kill_switch_notify_trg
  AFTER INSERT OR UPDATE ON monitor.kill_switch
  FOR EACH ROW EXECUTE FUNCTION monitor.notify_kill_switch();

-- ── The authority side of §8.6 ──────────────────────────────────────────────
--
-- `monitor.control` is the scope the monitor POST routes require. It joins the
-- list an agent session may never hold (migration 0038), and the trigger is
-- widened from project-bound service sessions to *every* service session.
--
-- The narrower form was a real hole: 0038 only fired when `project_id IS NOT NULL`,
-- so a service token minted without a project — which is exactly what
-- `POST /auth/service-tokens` produces — could have been granted `web:full` by a
-- direct write and would have satisfied every check in the API. Nothing does that
-- today; the point is that nothing *can*.
CREATE OR REPLACE FUNCTION agent_scopes_are_allowed() RETURNS trigger AS $$
DECLARE
  forbidden TEXT[] := ARRAY[
    'data.holdout',      -- the one-shot vault; only the vault-gate service reads it
    'orders.place',      -- research authority only (D-12)
    'orders.cancel',
    'automations.arm',
    'models.promote',    -- alias promotion is a human decision
    'skills.admit',      -- the platform admits skills, never the agent (ADR-0028)
    'evals.truth',       -- the answer key to the agent's own evaluation (AGENT-004 §2)
    'monitor.control',   -- COMP-007 §5: no agent trips, scales, halts or resets
    'web:full'
  ];
  offending TEXT;
BEGIN
  IF NEW.kind = 'service' THEN
    SELECT s INTO offending
      FROM unnest(NEW.scopes) AS s
     WHERE s = ANY(forbidden)
     LIMIT 1;
    IF offending IS NOT NULL THEN
      RAISE EXCEPTION
        'scope % may never be granted to a service session', offending
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
