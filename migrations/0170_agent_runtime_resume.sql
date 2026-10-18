-- 0170 — what the one agent runtime needs to survive a restart (AGENT-010 §5, §7;
-- Set S tasks S-1.19, S-2.6, S-2.12).
--
-- Two things, both small, both about a run outliving the process that started it.
--
-- ── 1. A write the run already made is answered, not repeated (S-1.19) ───────
--
-- The in-run idempotency record (`mcp_server_lib::idempotency`) is in memory and
-- per context: an in-run retry and a reconnect are covered, a post-crash resume is
-- not. A run that crashed after `create_experiment` returned and before the effect
-- log recorded the answer used to resume with "interrupted; not retried", which is
-- safe and loses the answer. This table holds the answer, keyed the same way, so a
-- resume that finds the write done hands the model its original result.
--
-- Keyed `(run_id, key)` where `key` is the idempotency key of
-- `(run_id, step, tool, canonical(arguments))`. Written only on success — replaying
-- an error would turn one transient failure into a permanent one.

CREATE TABLE IF NOT EXISTS agent_idempotency (
    run_id      UUID NOT NULL REFERENCES agent_runs(run_id) ON DELETE CASCADE,
    key         TEXT NOT NULL CHECK (key <> ''),
    tool        TEXT NOT NULL CHECK (tool <> ''),
    result      JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, key)
);

-- A recorded answer is evidence about what a write returned. Rewriting one would
-- let a resume hand the model a result the platform never produced.
CREATE OR REPLACE FUNCTION agent_idempotency_refuse_update() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'agent_idempotency rows are append-only (S-1.19)';
END $$;

DROP TRIGGER IF EXISTS trg_agent_idempotency_immutable ON agent_idempotency;
CREATE TRIGGER trg_agent_idempotency_immutable
    BEFORE UPDATE ON agent_idempotency
    FOR EACH ROW EXECUTE FUNCTION agent_idempotency_refuse_update();

COMMENT ON TABLE agent_idempotency IS
    'S-1.19. The result of every successful write an agent run made, keyed by the '
    'idempotency key of (run_id, step, tool, canonical(arguments)), so a post-crash '
    'resume answers the write rather than repeating it. Append-only.';

-- The platform reads and writes it; nothing deletes or rewrites it.
GRANT SELECT, INSERT ON agent_idempotency TO app_role;

-- ── 2. An answered approval wakes the run that asked (AGENT-010 §7) ──────────
--
-- The GOVERNOR loop polled `approval_requests` every five seconds while a run was
-- paused on a human. That is zero tokens but it is not an event subscription,
-- and a poll is a latency floor on every approval. The answer is written by the
-- desk's route; this trigger turns the write into a notification on the channel
-- `agent_approval`, carrying the approval id, so the waiting driver wakes on the
-- answer rather than on its next tick. The driver keeps a slow poll as a backstop:
-- a NOTIFY sent while no listener was connected is not queued.

CREATE OR REPLACE FUNCTION agent_approval_notify() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state AND OLD.state = 'pending' THEN
        PERFORM pg_notify('agent_approval', NEW.approval_id::text);
    END IF;
    RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS trg_agent_approval_notify ON approval_requests;
CREATE TRIGGER trg_agent_approval_notify
    AFTER UPDATE ON approval_requests
    FOR EACH ROW EXECUTE FUNCTION agent_approval_notify();
