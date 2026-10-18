-- 0161 - the day budget in trials, and the contradiction pair a session can read
-- (AGENT-014 section 2.1, 4.5, 7 row 2; Set S tasks S-11.2, S-11.5, S-11.6; ADR-P11-05).
--
-- ## session_day_budget
--
-- AGENT-014 prices the day in dollars and a run is metered in trials, so the day
-- gate could never fire: `execute::day_bound` answered `NotDenominated` for a
-- lease in trials, and "a day budget hit behaves identically and resets at the
-- window" was not true in production. There is no per-run price to convert with
-- (`llm_usage.cost_usd` keys on the auth session, not the run), so the day gets
-- its own figure in the unit the platform actually counts.
--
-- It is a table beside the session rather than a field of the `budget` document
-- because `SessionBudget` is a serialised shape built by struct literal in other
-- tracks' routes. A session created before this migration has no row, and that
-- reads as "nobody said" (the lease names the gap) - never as a cap of none and
-- never as a default nobody wrote down.
--
-- ## session_contradictions
--
-- Source 4.5 asked for "unresolved contradiction pairs" and the platform held a
-- count (`agent_runs.contradictions_surfaced`) and a list on one entry
-- (`agent_memory.entry.conflicts_with`) - never the pair. This is the pair, with
-- the two entries it names, the question it raises, when it was found and when it
-- stopped being true. The driver materialises a pair from `conflicts_with` and
-- resolves it when either side is archived or superseded; nothing resolves one
-- by writing a winner, because AGENT-007 2.2.1 says conflicts are surfaced and
-- never merged.

CREATE TABLE IF NOT EXISTS session_day_budget (
    session_id     UUID PRIMARY KEY REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    trials_per_day BIGINT NOT NULL,
    set_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT a_day_buys_at_least_one_trial CHECK (trials_per_day > 0)
);

GRANT SELECT, INSERT, UPDATE ON session_day_budget TO app_role;
REVOKE DELETE, TRUNCATE ON session_day_budget FROM app_role;

CREATE TABLE IF NOT EXISTS session_contradictions (
    pair_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL,
    -- Ordered, so {a, b} and {b, a} are one pair whichever entry listed the other.
    left_entry   TEXT NOT NULL,
    right_entry  TEXT NOT NULL,
    subject      TEXT NOT NULL DEFAULT '',
    question     TEXT NOT NULL,
    found_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at  TIMESTAMPTZ,
    resolved_why TEXT,
    CONSTRAINT a_pair_is_two_entries CHECK (left_entry < right_entry),
    CONSTRAINT a_pair_asks_something CHECK (btrim(question) <> ''),
    CONSTRAINT a_resolution_says_why CHECK (
        (resolved_at IS NULL) = (resolved_why IS NULL)
    ),
    CONSTRAINT one_row_per_pair UNIQUE (user_id, left_entry, right_entry)
);

CREATE INDEX IF NOT EXISTS session_contradictions_open
    ON session_contradictions (user_id, found_at) WHERE resolved_at IS NULL;

-- A pair is found once and resolved once; the only mutation is resolving it.
CREATE OR REPLACE FUNCTION session_contradictions_resolve_only()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'session_contradictions is append-and-resolve only: DELETE refused';
    END IF;
    IF OLD.resolved_at IS NOT NULL THEN
        RAISE EXCEPTION 'contradiction % is already resolved', OLD.pair_id;
    END IF;
    IF NEW.pair_id IS DISTINCT FROM OLD.pair_id
       OR NEW.user_id IS DISTINCT FROM OLD.user_id
       OR NEW.left_entry IS DISTINCT FROM OLD.left_entry
       OR NEW.right_entry IS DISTINCT FROM OLD.right_entry
       OR NEW.subject IS DISTINCT FROM OLD.subject
       OR NEW.question IS DISTINCT FROM OLD.question
       OR NEW.found_at IS DISTINCT FROM OLD.found_at THEN
        RAISE EXCEPTION 'a contradiction pair is what it was found as; only resolved_at/resolved_why may be set';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_session_contradictions_resolve_only ON session_contradictions;
CREATE TRIGGER trg_session_contradictions_resolve_only
    BEFORE UPDATE OR DELETE ON session_contradictions
    FOR EACH ROW EXECUTE FUNCTION session_contradictions_resolve_only();

GRANT SELECT, INSERT, UPDATE ON session_contradictions TO app_role;
REVOKE DELETE, TRUNCATE ON session_contradictions FROM app_role;
