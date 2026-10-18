-- 0145 — let the platform reach the monitor schema (COMP-007 §4; ADR-P0-13).
--
-- Migrations 0074–0078 created the `monitor` schema after 0044 moved the runtime
-- pool onto `platform_app` (a member of app_role), and none of them granted it
-- anything. Every test connects as the owner, so they all passed; the running
-- platform got `permission denied for schema monitor` on its first lease read and,
-- correctly failing closed, refused every order while the monitor was healthy.
--
-- The platform reads the whole schema and writes the operator's side of it —
-- kill-switch trips and resets, strategy states, limits, maintenance windows,
-- drills, breach labels, precision. It never deletes.
--
-- `monitor.lease` stays read-only. The lease is the monitor vouching that it is
-- alive; a platform that could renew it could vouch for a dead monitor, and the
-- order gate would be checking nothing.

GRANT USAGE ON SCHEMA monitor TO app_role;

GRANT SELECT ON ALL TABLES IN SCHEMA monitor TO app_role;

GRANT INSERT, UPDATE ON
  monitor.breach_label,
  monitor.drill,
  monitor.dwell,
  monitor.events,
  monitor.family_precision,
  monitor.heartbeat,
  monitor.kill_switch,
  monitor.limit_change,
  monitor.limits,
  monitor.maintenance_window,
  monitor.strategy_state
TO app_role;

-- A future monitor table is readable by default; writing it is a decision its own
-- migration makes explicitly, as above.
ALTER DEFAULT PRIVILEGES IN SCHEMA monitor GRANT SELECT ON TABLES TO app_role;
