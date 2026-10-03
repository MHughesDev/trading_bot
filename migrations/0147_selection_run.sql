-- 0147 — selection runs, journaled (Set U U-6.3; FEAT-007 §4, §10).
--
-- A selection strategy answers "which assets should we be in?" — a screener
-- result a person reads, the agent cites, and a deployment rebalances on. Every
-- run is written down: who asked (a person, the agent, the schedule, a
-- deployment), as of which instant, the selection itself — every member,
-- selected with its rank, score and attributes, or excluded with its reason —
-- and the digest of the whole evaluation, so two runs over the same strategy,
-- instant and data can be shown to agree byte for byte.
--
-- Append-only: a screener result that could be edited is not evidence.
--
-- Also: the platform pool could not read the universes P-4.3 created. 0046's
-- `GRANT SELECT ON ALL TABLES IN SCHEMA dataplane` covered the tables that
-- existed then; 0104 added `universe` and `universe_membership_event` after it
-- and granted nothing, so the first live load of a named universe would be
-- refused while every test — run as the owner — passed.

CREATE TABLE IF NOT EXISTS selection_run (
  run_id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  -- As the caller named it: a revision hash or a strategy id.
  strategy_ref           TEXT NOT NULL,
  -- The revision that ran.
  revision               TEXT NOT NULL,
  -- The instant the selection describes: the last close of the strategy's
  -- timeframe at or before what was asked for.
  as_of                  TIMESTAMPTZ NOT NULL,
  requested_by           TEXT NOT NULL
                           CHECK (requested_by IN ('person', 'agent', 'schedule', 'deployment')),
  universe_size          INTEGER NOT NULL CHECK (universe_size >= 0),
  -- false: a list or an asset class — today's membership applied to the past.
  universe_point_in_time BOOLEAN NOT NULL,
  selected_count         INTEGER NOT NULL CHECK (selected_count >= 0),
  selection              JSONB NOT NULL,
  digest                 TEXT NOT NULL CHECK (digest ~ '^[0-9a-f]{64}$'),
  created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (selected_count <= universe_size)
);

CREATE INDEX IF NOT EXISTS idx_selection_run_revision_as_of
  ON selection_run (revision, as_of DESC);
CREATE INDEX IF NOT EXISTS idx_selection_run_created
  ON selection_run (created_at DESC);

CREATE OR REPLACE FUNCTION selection_run_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'selection_run is append-only (Set U U-6.3): a selection is a record of what a strategy said, and is never rewritten';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_selection_run_append_only ON selection_run;
CREATE TRIGGER trg_selection_run_append_only
  BEFORE UPDATE OR DELETE ON selection_run
  FOR EACH ROW EXECUTE FUNCTION selection_run_append_only();

GRANT SELECT, INSERT ON selection_run TO app_role;

GRANT SELECT ON dataplane.universe, dataplane.universe_membership_event TO app_role;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'backtest_role') THEN
    GRANT SELECT ON dataplane.universe, dataplane.universe_membership_event TO backtest_role;
  END IF;
END $$;
