-- 0078 — drills and maintenance windows (COMP-007 §4.2, §4.3; S-7.13).
--
-- Nobody will trip a global halt on a live book to see whether it works, so without
-- a rehearsal the most important path in this system is the one path never
-- exercised — first exercised on the day it matters, by someone under pressure.
--
-- And a rolling deploy looks exactly like a heartbeat loss, so without a bounded,
-- visible, auto-expiring window every deploy halts trading and the operator learns
-- to pre-emptively reset the switch. That is how a safety mechanism becomes a
-- nuisance and then a habit of ignoring it.

CREATE TABLE IF NOT EXISTS monitor.drill (
  drill_id        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  scope           TEXT        NOT NULL,
  trigger_kind    TEXT        NOT NULL,
  started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  cadence_ms      BIGINT      NOT NULL CHECK (cadence_ms > 0),
  -- The three numbers that matter (§4.2): breach to state change, state change to
  -- the runtime refusing, state change to the notification landing.
  detection_ms    BIGINT      NOT NULL CHECK (detection_ms >= 0),
  propagation_ms  BIGINT      NOT NULL CHECK (propagation_ms >= 0),
  notification_ms BIGINT      NOT NULL CHECK (notification_ms >= 0),
  passed          BOOLEAN     NOT NULL,
  -- A drill that refused a real order is not a drill. The column exists so that
  -- recording one is impossible rather than merely discouraged: a drill whose
  -- rehearsal stopped real money would have been a live incident nobody declared.
  refused_real    BOOLEAN     NOT NULL DEFAULT FALSE,
  operator        TEXT        NOT NULL,
  notes           TEXT,

  CONSTRAINT monitor_drill_scope_is_a_drill CHECK (scope LIKE 'drill:%'),
  CONSTRAINT monitor_drill_trigger CHECK (trigger_kind IN ('deploy', 'scheduled', 'manual')),
  CONSTRAINT monitor_drill_refused_nothing_real CHECK (refused_real IS FALSE),
  -- §4.2: "a drill whose propagation latency exceeds one cadence fails the deploy".
  -- The verdict is the arithmetic, so a passing drill cannot be recorded for a slow
  -- one — which is the only version of this assertion that stays true under
  -- pressure to ship.
  CONSTRAINT monitor_drill_verdict_follows_propagation
    CHECK (passed = (propagation_ms <= cadence_ms)),
  CONSTRAINT monitor_drill_never_run_by_an_agent
    CHECK (operator NOT LIKE 'agent:%')
);

CREATE INDEX IF NOT EXISTS monitor_drill_recent ON monitor.drill (started_at DESC);

-- A window suppresses platform-health trips only. It does not suppress
-- reconciliation mismatches, venue errors, data staleness or any strategy-level
-- breach — those are facts about the market and the book, and a deploy is no reason
-- to stop believing them. The suppressed-family list is *not* a column: making it
-- configurable is how "platform only" becomes "whatever the operator needed once".
CREATE TABLE IF NOT EXISTS monitor.maintenance_window (
  window_id   UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
  opened_by   TEXT        NOT NULL,
  reason      TEXT        NOT NULL CHECK (length(btrim(reason)) > 0),
  opened_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at  TIMESTAMPTZ NOT NULL,
  closed_at   TIMESTAMPTZ,

  CONSTRAINT monitor_window_expires_after_it_opens CHECK (expires_at > opened_at),
  -- Bounded, and bounded in the schema rather than in the handler: a window that
  -- can be opened for a week is a switch that can be left off.
  CONSTRAINT monitor_window_is_bounded
    CHECK (expires_at <= opened_at + INTERVAL '4 hours'),
  CONSTRAINT monitor_window_never_opened_by_an_agent
    CHECK (opened_by NOT LIKE 'agent:%')
);

CREATE INDEX IF NOT EXISTS monitor_window_open
  ON monitor.maintenance_window (expires_at DESC)
  WHERE closed_at IS NULL;

COMMENT ON TABLE monitor.maintenance_window IS
  'COMP-007 §4.3: suppresses platform-health trips only; visible in the desk chrome exactly as a trip is.';
