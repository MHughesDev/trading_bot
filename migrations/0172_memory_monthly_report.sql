-- 0172 -- the monthly memory-store report is an artefact, not an on-demand read
-- (Set S task S-4.16; AGENT-007 section 2.4).
--
-- `GET /api/memory/report` answers "how big is the store, how much of it is archived,
-- how much of what was shown was cited" at the moment somebody asks. The acceptance
-- wants the number to exist whether or not anyone asked: a degrading brief becomes a
-- number before it becomes bad research, and the month-over-month series is the only
-- thing that shows the drift. One row per (user, month), written by the retention sweep
-- on its first run of the month and never edited.
--
-- GRANT to app_role in this file (0145 and 0166 are what a missing grant costs; the
-- test that covers this switches to app_role). SELECT and INSERT only: a report that
-- could be updated would be a report that could be tuned.

CREATE TABLE IF NOT EXISTS agent_memory.monthly_report (
    user_id     UUID        NOT NULL,
    -- The first day of the month the report describes.
    month       DATE        NOT NULL CHECK (month = date_trunc('month', month)::date),
    report      JSONB       NOT NULL CHECK (jsonb_typeof(report) = 'object'),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, month)
);

GRANT USAGE ON SCHEMA agent_memory TO app_role;
GRANT SELECT, INSERT ON agent_memory.monthly_report TO app_role;
