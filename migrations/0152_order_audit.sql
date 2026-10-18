-- 0152 — the order audit trail is durable and append-only (Set E audit item #3).
--
-- `ExecutionEngine::submit` and `sync_order` have always built `AuditEntry`
-- records — the state transitions and broker interactions of every order — and
-- returned them to callers that dropped them. There was no table, so there was
-- no durable record that any order transition ever happened beyond the latest
-- `orders.state`.
--
-- `execution::journal::OrderJournal::record_audit` now writes every entry here,
-- from inside the engine, so no order door can skip it.
--
-- Append-only is enforced by the database, not by convention:
--   * app_role gets SELECT and INSERT only — no UPDATE, no DELETE;
--   * a trigger refuses UPDATE and DELETE for every role, owner included,
--     so a correction is a new row, never an edit.

CREATE TABLE IF NOT EXISTS order_audit (
    audit_id         BIGSERIAL PRIMARY KEY,
    idempotency_key  UUID NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN
                       ('state_transition', 'fill_received', 'broker_submit', 'broker_query')),
    from_state       TEXT,
    to_state         TEXT,
    broker_order_id  TEXT,
    reason           TEXT,
    -- The whole entry as serialized by `AuditEntry` (serde tag = kind).
    entry            JSONB NOT NULL,
    occurred_at      TIMESTAMPTZ NOT NULL,
    recorded_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT order_audit_transition_has_target
        CHECK (kind <> 'state_transition' OR to_state IS NOT NULL),
    CONSTRAINT order_audit_broker_rows_name_the_order
        CHECK (kind NOT IN ('broker_submit', 'broker_query', 'fill_received')
               OR broker_order_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS order_audit_key_time ON order_audit (idempotency_key, occurred_at);

CREATE OR REPLACE FUNCTION order_audit_is_append_only() RETURNS trigger
    LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'order_audit is append-only: % is refused', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END;
$$;

DROP TRIGGER IF EXISTS order_audit_is_append_only_trg ON order_audit;
CREATE TRIGGER order_audit_is_append_only_trg
    BEFORE UPDATE OR DELETE ON order_audit
    FOR EACH ROW EXECUTE FUNCTION order_audit_is_append_only();

-- The public-schema default privileges (0044) would grant UPDATE and DELETE;
-- take them back explicitly and grant only what an append-only log needs.
REVOKE ALL ON order_audit FROM app_role;
GRANT SELECT, INSERT ON order_audit TO app_role;
GRANT USAGE, SELECT ON SEQUENCE order_audit_audit_id_seq TO app_role;
