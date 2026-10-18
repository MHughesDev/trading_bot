-- 0160 — a child's lease outlives the iteration that issued it
-- (AGENT-014 §6.3, §12.1, §12.4; Set S tasks S-11.1, S-11.5).
--
-- `research_session::execute::Lease` is sealed to `execute::lease` and was
-- persisted nowhere: the size rode on the `run_started` event as prose and
-- nothing could read it back as a number. That cost two acceptance clauses:
--
--   * §12.1's "one child, one lease" had nothing to count — a lease was a
--     paragraph in a payload;
--   * §12.4's "three concurrent children cannot together exceed the day budget"
--     had nothing to sum, so the driver refused a second child outright whenever
--     one was running rather than sizing it against a total missing a number.
--
-- This is the number. One row per (session, item, run), written when the child
-- starts and released when it finishes; `execute::LeaseRequest::in_flight` is
-- the unreleased rows' sizes.
--
-- ## What the constraints hold
--
-- * **One live lease per agenda item.** The partial unique index is "one child,
--   one lease" as a schema fact: a driver that restarts between the start and
--   the record re-derives the same child (ADR-P11-01) and its INSERT hits the
--   key it already wrote, rather than committing the item's budget twice.
-- * **The size is never rewritten.** A lease is committed in full the moment it
--   is issued (§12.4), so the only mutation is releasing it, once. The trigger
--   refuses anything else, including DELETE: a lease nobody can see any more is
--   a commitment nobody can count.
-- * **The unit is said, not assumed.** `trials` or `usd`, CHECKed to the same
--   two words as `session_digests.spend_unit` (0139), so a lease cannot be
--   summed against a cap in the other unit by accident.

CREATE TABLE IF NOT EXISTS session_leases (
    session_id  UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    item_id     UUID NOT NULL REFERENCES agenda_items(item_id) ON DELETE CASCADE,
    run_id      UUID NOT NULL,
    size        DOUBLE PRECISION NOT NULL,
    unit        TEXT NOT NULL,
    capped_by   TEXT NOT NULL,
    -- Whether the workflow's estimate took part in sizing it. `false` is the
    -- cold start, when no run of the workflow has finished on this tier yet.
    estimated   BOOLEAN NOT NULL,
    issued_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_at TIMESTAMPTZ,
    PRIMARY KEY (session_id, item_id, run_id),
    CONSTRAINT a_lease_buys_something
        CHECK (size > 0 AND size < 'Infinity'::double precision),
    CONSTRAINT a_lease_names_its_unit
        CHECK (unit IN ('trials', 'usd')),
    CONSTRAINT a_lease_names_what_cut_it
        CHECK (capped_by IN ('estimate', 'per_task', 'day', 'total')),
    CONSTRAINT a_lease_is_released_after_it_is_issued
        CHECK (released_at IS NULL OR released_at >= issued_at)
);

-- One child, one lease: at most one unreleased lease per agenda item.
CREATE UNIQUE INDEX IF NOT EXISTS session_leases_one_live_per_item
    ON session_leases (item_id) WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS session_leases_live_by_session
    ON session_leases (session_id) WHERE released_at IS NULL;

CREATE OR REPLACE FUNCTION session_leases_release_only()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'session_leases is append-and-release only: DELETE refused';
    END IF;
    IF OLD.released_at IS NOT NULL THEN
        RAISE EXCEPTION 'lease (%, %, %) is already released', OLD.session_id, OLD.item_id, OLD.run_id;
    END IF;
    IF NEW.session_id IS DISTINCT FROM OLD.session_id
       OR NEW.item_id IS DISTINCT FROM OLD.item_id
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.size IS DISTINCT FROM OLD.size
       OR NEW.unit IS DISTINCT FROM OLD.unit
       OR NEW.capped_by IS DISTINCT FROM OLD.capped_by
       OR NEW.estimated IS DISTINCT FROM OLD.estimated
       OR NEW.issued_at IS DISTINCT FROM OLD.issued_at THEN
        RAISE EXCEPTION 'a lease is committed in full when it is issued; only released_at may be set';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_session_leases_release_only ON session_leases;
CREATE TRIGGER trg_session_leases_release_only
    BEFORE UPDATE OR DELETE ON session_leases
    FOR EACH ROW EXECUTE FUNCTION session_leases_release_only();

-- The platform pool runs as app_role (0044). It issues and releases; it never
-- deletes, and the trigger refuses a DELETE from the owner too.
GRANT SELECT, INSERT, UPDATE ON session_leases TO app_role;
-- 0044's default privileges hand every new public table DELETE as well; this one
-- takes it back so the refusal is a missing privilege first and a trigger second.
REVOKE DELETE, TRUNCATE ON session_leases FROM app_role;
