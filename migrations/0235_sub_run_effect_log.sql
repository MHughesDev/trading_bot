-- 0235 — a sub-run has its own effect log (AGENT-010 §9; Set S task S-2.12).
--
-- A campaign worker sub-run is a real model call made on a leased budget, and until
-- now nothing of it survived except its settled outcome: no record of what it was
-- asked, what it answered, or what it spent, and no row tying it to the parent run
-- that dispatched it. §9 asks for both sides of the delegation to be traceable, so
-- "what did the worker see" has an answer that does not depend on asking the worker.
--
-- Same shape as agent_effect_log (0061) and for the same reason: append-only,
-- ordered, keyed on the variant name. The parent's side of the pairing needs no new
-- table: it is a 'state'/'Delegation' row in the parent's own agent_effect_log.
--
-- No ON DELETE CASCADE: the immutability trigger refuses DELETE, so a cascade would
-- turn the deletion of a sub_run row into a refusal nobody can explain (the trap
-- pg_agent_effect_log.rs documents). A sub-run with a log cannot be deleted; that is
-- the property.

CREATE TABLE IF NOT EXISTS agent_lifecycle.sub_run_effect (
    sub_run_id      UUID NOT NULL REFERENCES agent_lifecycle.sub_run(sub_run_id),
    seq             INT  NOT NULL CHECK (seq > 0),
    direction       TEXT NOT NULL CHECK (direction IN ('effect', 'input', 'state')),
    kind            TEXT NOT NULL CHECK (kind <> ''),
    payload_json    JSONB NOT NULL,
    harness_version TEXT NOT NULL CHECK (harness_version <> ''),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (sub_run_id, seq)
);

DROP TRIGGER IF EXISTS trg_sub_run_effect_immutable ON agent_lifecycle.sub_run_effect;
CREATE TRIGGER trg_sub_run_effect_immutable
    BEFORE UPDATE OR DELETE ON agent_lifecycle.sub_run_effect
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

COMMENT ON TABLE agent_lifecycle.sub_run_effect IS
    'S-2.12. A worker sub-run''s own effect log: its Delegation (child side), the model call it made and what came back, what it spent. Append-only. The parent side of the Delegation is a state row in agent_effect_log.';

GRANT SELECT, INSERT ON agent_lifecycle.sub_run_effect TO app_role;
