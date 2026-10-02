-- 0143 — engine checkpoints (Set U U-4.4; FEAT-007 §9.5).
--
-- A live deployment checkpoints after every decision so a restart resumes the
-- strategy instead of starting it again. One row per deployment: the latest
-- state wins. `state` holds the engine's checkpoint (planner state, tick, last
-- bar per input) and the runner's working orders (tag → idempotency key and
-- broker order id), which together are what bars cannot give back.
--
-- `revision_hash` is stored beside the state so a restore into a different
-- graph is refused before the state is even parsed.

CREATE TABLE IF NOT EXISTS strategy_checkpoint (
  deployment_id   UUID PRIMARY KEY REFERENCES strategy_deployment (deployment_id),
  revision_hash   TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
  tick            BIGINT NOT NULL CHECK (tick >= 0),
  state           JSONB NOT NULL,
  written_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
