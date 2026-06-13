-- Scope backtest runs to their creator.
--
-- Adds the owning user to each run so the list/get/stop/delete endpoints can
-- be filtered by the authenticated identity (see api::auth::BearerToken).
-- Nullable for backward compatibility with rows created before scoping; such
-- legacy rows are treated as owned by the nil user and are not surfaced to any
-- real token.

ALTER TABLE backtest_runs ADD COLUMN IF NOT EXISTS user_id UUID;

CREATE INDEX IF NOT EXISTS idx_backtest_runs_user_created
    ON backtest_runs (user_id, created_at DESC);
