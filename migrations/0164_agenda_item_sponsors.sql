-- 0164 - two sessions, one question, one item (AGENT-014 P3, section 4, 12.2;
-- Set S task S-11.4; ADR-P11-07).
--
-- Layer one of the duplicate refusal is per session (`agenda_items_live_key`), so
-- two continuous sessions of one user whose scopes overlap each ran the same
-- question. The second session's offer now merges onto the first's live item:
-- the item's `sources` list is extended (one item, two sources) and the second
-- session is recorded here, so its log, its agenda and its digest can say that
-- the question is being answered for it and by whom.
--
-- The item stays where it was first raised. The alternatives are weighed in
-- ADR-P11-07: copying the item into the second session runs the question twice,
-- and moving it takes a card off a board its owner is already watching.

CREATE TABLE IF NOT EXISTS agenda_item_sponsors (
    item_id     UUID NOT NULL REFERENCES agenda_items(item_id) ON DELETE CASCADE,
    -- The session that *also* wanted the item. The owner is the item's own
    -- `session_id` and is not repeated here.
    session_id  UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    source      TEXT NOT NULL,
    source_refs JSONB NOT NULL DEFAULT '[]'::jsonb,
    offered_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Set once, when the item settles and the sponsor's log is told.
    told_at     TIMESTAMPTZ,
    PRIMARY KEY (item_id, session_id)
);

CREATE INDEX IF NOT EXISTS agenda_item_sponsors_session
    ON agenda_item_sponsors (session_id, offered_at DESC);

GRANT SELECT, INSERT, UPDATE ON agenda_item_sponsors TO app_role;
REVOKE DELETE, TRUNCATE ON agenda_item_sponsors FROM app_role;

-- ## The session log, pushed (AGENT-014 section 10, 12.6; Set S task S-11.14)
--
-- `GET /api/research/sessions/{id}/events` polled `session_events` every two
-- seconds, so an agenda transition reached an open journal "within one poll",
-- not "within one event". Every insert now publishes the session id on
-- `session_events`, and the stream wakes on it; the poll stays as the backstop
-- for a notification lost across a reconnect. The payload is the id and nothing
-- else - a NOTIFY is capped at 8000 bytes and an event's payload is not.
CREATE OR REPLACE FUNCTION session_events_notify()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('session_events', NEW.session_id::text);
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_session_events_notify ON session_events;
CREATE TRIGGER trg_session_events_notify
    AFTER INSERT ON session_events
    FOR EACH ROW EXECUTE FUNCTION session_events_notify();
