-- 0187 — a selection that runs on its own schedule (Set U U-6.3).
--
-- A selection strategy bound to a deployment already runs on its schedule,
-- through the deployment's allocator. A screener a person or the agent only
-- wants to *watch* — "tell me what this picks every morning" — had nowhere to
-- say so. A watch pins the selection's revision (so an edit to the strategy is
-- a new watch, not a silent change to an old one) and its schedule; the
-- platform runs every running watch when its schedule fires, and journals each
-- run in `selection_run` with `requested_by = 'schedule'` (0147).
--
-- A watch is stopped, never deleted or edited: the only change it accepts is
-- `stopped_at` being set, once.

CREATE TABLE IF NOT EXISTS selection_watch (
  watch_id      UUID PRIMARY KEY,
  user_id       TEXT NOT NULL CHECK (length(user_id) > 0),
  strategy_ref  TEXT NOT NULL CHECK (length(strategy_ref) > 0),
  -- The revision the watch runs, pinned when it was created.
  revision      TEXT NOT NULL CHECK (length(revision) > 0),
  -- Five-field UTC cron, from the strategy's `triggers.schedule`.
  schedule      TEXT NOT NULL CHECK (length(trim(schedule)) > 0),
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  stopped_at    TIMESTAMPTZ,
  CHECK (stopped_at IS NULL OR stopped_at >= created_at)
);

-- One running watch per person and revision.
CREATE UNIQUE INDEX IF NOT EXISTS uq_selection_watch_running
  ON selection_watch (user_id, revision) WHERE stopped_at IS NULL;

CREATE OR REPLACE FUNCTION selection_watch_only_stops() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION 'selection_watch rows are stopped, never deleted (Set U U-6.3)';
  END IF;
  IF OLD.stopped_at IS NOT NULL
     OR NEW.stopped_at IS NULL
     OR NEW.watch_id IS DISTINCT FROM OLD.watch_id
     OR NEW.user_id IS DISTINCT FROM OLD.user_id
     OR NEW.strategy_ref IS DISTINCT FROM OLD.strategy_ref
     OR NEW.revision IS DISTINCT FROM OLD.revision
     OR NEW.schedule IS DISTINCT FROM OLD.schedule
     OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
    RAISE EXCEPTION 'a selection_watch accepts one change: being stopped, once (Set U U-6.3)';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_selection_watch_only_stops ON selection_watch;
CREATE TRIGGER trg_selection_watch_only_stops
  BEFORE UPDATE OR DELETE ON selection_watch
  FOR EACH ROW EXECUTE FUNCTION selection_watch_only_stops();

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') THEN
    GRANT SELECT, INSERT, UPDATE ON selection_watch TO app_role;
    -- 0044's default privileges grant DELETE on every new public table.
    REVOKE DELETE ON selection_watch FROM app_role;
  END IF;
END $$;
