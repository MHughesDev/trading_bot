-- 0052 — the campaign phase log is ordered and its transitions are legal
-- (SPEC §10, ADR-P2-04, AT-59).
--
-- A campaign has no status column: its state is a fold over `campaign_event`.
-- That only works if the log itself cannot hold a sequence the fold would
-- refuse, so the two rules the fold enforces in Rust are enforced here as well,
-- against every writer rather than against the driver alone:
--
--   1. events are ordered within a campaign (`seq`, unique, gapless by
--      construction), so two readers fold the same list in the same order even
--      when two events share an `occurred_at`;
--   2. a transition that SPEC §10 does not allow — including anything at all
--      after a terminal phase — is refused at INSERT.
--
-- Without (1) the fold is at the mercy of timestamp ties; without (2) one
-- `psql` session could append a `gate` after a `halted` and every fold taken
-- after that point would disagree with every fold taken before it.

ALTER TABLE mlops.campaign_event
    ADD COLUMN IF NOT EXISTS seq BIGINT;

-- Existing rows (none in any deployed database at the time of writing, but the
-- migration must be total) take their order from when they happened.
WITH ordered AS (
    SELECT event_id,
           row_number() OVER (PARTITION BY campaign_id ORDER BY occurred_at, event_id) - 1 AS n
    FROM mlops.campaign_event
    WHERE seq IS NULL
)
UPDATE mlops.campaign_event e
SET seq = ordered.n
FROM ordered
WHERE e.event_id = ordered.event_id;

ALTER TABLE mlops.campaign_event
    ALTER COLUMN seq SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS campaign_event_seq
    ON mlops.campaign_event (campaign_id, seq);

-- ── the §10 transition table ────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION mlops.campaign_transition_ok(prev TEXT, next TEXT)
RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        -- A campaign may end from any non-terminal phase: the budget runs out,
        -- the posterior stops moving, or a human halts it.
        WHEN prev IN ('converged','budget_exhausted','diminishing_returns','halted') THEN FALSE
        WHEN next IN ('converged','budget_exhausted','diminishing_returns','halted') THEN TRUE
        WHEN prev = 'define'      THEN next = 'baseline'
        WHEN prev = 'baseline'    THEN next = 'diagnose'
        WHEN prev = 'diagnose'    THEN next = 'hypothesize'
        WHEN prev = 'hypothesize' THEN next IN ('experiment','hypothesize')
        WHEN prev = 'experiment'  THEN next = 'compare'
        WHEN prev = 'compare'     THEN next = 'gate'
        WHEN prev = 'gate'        THEN next IN ('prune','reallocate')
        WHEN prev = 'prune'       THEN next IN ('reallocate','hypothesize')
        WHEN prev = 'reallocate'  THEN next = 'hypothesize'
        ELSE FALSE
    END
$$;

CREATE OR REPLACE FUNCTION mlops.check_campaign_transition()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    prev_state TEXT;
    prev_seq   BIGINT;
BEGIN
    SELECT state, seq INTO prev_state, prev_seq
    FROM mlops.campaign_event
    WHERE campaign_id = NEW.campaign_id
    ORDER BY seq DESC
    LIMIT 1;

    IF prev_state IS NULL THEN
        IF NEW.state <> 'define' THEN
            RAISE EXCEPTION
                'a campaign opens with define, not %; DEFINE is what makes the claim before anything runs',
                NEW.state;
        END IF;
        NEW.seq := 0;
        RETURN NEW;
    END IF;

    IF NOT mlops.campaign_transition_ok(prev_state, NEW.state) THEN
        RAISE EXCEPTION
            'campaign % cannot go from % to % (SPEC 10)',
            NEW.campaign_id, prev_state, NEW.state;
    END IF;

    -- The writer does not choose the position; the log does.
    NEW.seq := prev_seq + 1;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_campaign_transition ON mlops.campaign_event;
CREATE TRIGGER trg_campaign_transition BEFORE INSERT ON mlops.campaign_event
    FOR EACH ROW EXECUTE FUNCTION mlops.check_campaign_transition();
