-- 0074 — the monitor's lease and the heartbeats it reads (COMP-007 §4, §6; S-7.1, S-7.2).
--
-- The load-bearing property of this whole phase is that the checking mechanism is
-- independent of the thing it checks. A monitor living inside the API process dies
-- with it and has therefore checked nothing. So the monitor is its own process, and
-- the only thing connecting it to the order path is this row: the monitor renews it
-- every cadence, and the runtime refuses every new order when it is absent or stale.
--
-- The direction of the coupling is what makes it fail *closed*. The runtime does not
-- ask "has the monitor told me to stop"; it asks "has the monitor told me it is
-- alive, recently". Silence is a halt. A monitor that crashes, wedges, loses its
-- database or is simply never started produces exactly the same answer as one that
-- deliberately halted trading, which is the answer that keeps money safe.

CREATE SCHEMA IF NOT EXISTS monitor;

COMMENT ON SCHEMA monitor IS
  'COMP-007: the independent live monitor. Written by tbot-monitor, read by the API and the order path.';

-- One row, id 'tbot-monitor'. A second monitor instance taking the lease is a
-- takeover, not a duplicate: `holder` changes and the previous holder learns it
-- lost the lease the next time it tries to renew under its own holder id.
CREATE TABLE IF NOT EXISTS monitor.lease (
  lease_id    TEXT        PRIMARY KEY,
  holder      TEXT        NOT NULL,
  cadence_ms  BIGINT      NOT NULL CHECK (cadence_ms > 0),
  renewed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at  TIMESTAMPTZ NOT NULL,
  -- A lease that expires at or before the moment it was renewed is already
  -- expired, which would make every renewal a no-op and the gate permanently
  -- closed for a reason nobody could see.
  CONSTRAINT monitor_lease_expiry_is_in_the_future
    CHECK (expires_at > renewed_at)
);

COMMENT ON TABLE monitor.lease IS
  'COMP-007 §4: the runtime refuses orders with monitor_unavailable when no row here is unexpired.';

-- Liveness of the things the monitor watches rather than of the monitor itself
-- (COMP-007 §3, Platform family): API, job service, collectors. Three missed
-- beats is a global halt, and a maintenance window suppresses *this* family and
-- nothing else (§4.3).
CREATE TABLE IF NOT EXISTS monitor.heartbeat (
  component  TEXT        NOT NULL,
  instance   TEXT        NOT NULL,
  beat_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  cadence_ms BIGINT      NOT NULL CHECK (cadence_ms > 0),
  PRIMARY KEY (component, instance)
);

CREATE INDEX IF NOT EXISTS monitor_heartbeat_beat_at ON monitor.heartbeat (beat_at DESC);

-- The runtime keeps an in-memory mirror so `is_active(scope)` stays an atomic
-- read on the hot path (COMP-007 §5). The mirror is refreshed on every lease
-- renewal and on this NOTIFY, so a trip propagates without waiting a cadence.
CREATE OR REPLACE FUNCTION monitor.notify_lease() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('monitor_lease', NEW.lease_id);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS monitor_lease_notify_trg ON monitor.lease;
CREATE TRIGGER monitor_lease_notify_trg
  AFTER INSERT OR UPDATE ON monitor.lease
  FOR EACH ROW EXECUTE FUNCTION monitor.notify_lease();
