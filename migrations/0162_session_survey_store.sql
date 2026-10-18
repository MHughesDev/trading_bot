-- 0162 - what a replenish survey proposed, and what absorbing it did
-- (AGENT-014 section 4.9, 7; Set S task S-11.11; ADR-P11-06).
--
-- `replenish::absorb` turns a survey's `proposed_questions[]` into agenda items
-- with the survey's own value estimates, and had no caller because nothing stored
-- a survey's proposals: they reached the agenda through the ordinary harvest, as
-- `open_question` entries scored like any other source 4.3 item, with the
-- estimates dropped on the floor.
--
-- ## session_survey_proposals
--
-- One row per question a `survey` run wrote down, read back from the
-- `open_question` entries the run filed (`lifecycle.open_question` is the only
-- door a survey has - it cannot spend a trial). The estimate is kept in both
-- forms: `value_label` is what the run said (`curious | useful | decisive`) and
-- `value_estimate` is the declared ordinal reading of it. A proposal with no
-- label has NULL, never 0: an absent estimate is not a worthless one.
--
-- ## session_survey_absorptions
--
-- One row per survey run, written once: how many it proposed, how many became
-- items, how many the dedupe merged and how many the charter refused. A survey
-- that proposed nothing new has `new_questions = 0` *explicitly*, which is what
-- makes "a fully-deduped replenish counts toward the barren streak" something the
-- stall detector can read rather than infer from a yield that was never going to
-- count a question.

CREATE TABLE IF NOT EXISTS session_survey_proposals (
    proposal_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id     UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    item_id        UUID NOT NULL REFERENCES agenda_items(item_id) ON DELETE CASCADE,
    run_id         UUID NOT NULL,
    -- The `open_question` memory entry this was read from, so the card can open it.
    source_entry   TEXT,
    question       TEXT NOT NULL,
    subject        TEXT NOT NULL DEFAULT '',
    workflow       TEXT,
    value_label    TEXT,
    value_estimate DOUBLE PRECISION,
    outcome        TEXT NOT NULL DEFAULT 'pending',
    proposed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT a_proposal_asks_something CHECK (btrim(question) <> ''),
    CONSTRAINT an_estimate_is_a_share CHECK (
        value_estimate IS NULL OR (value_estimate >= 0 AND value_estimate <= 1)
    ),
    CONSTRAINT an_estimate_has_its_label CHECK (
        (value_estimate IS NULL) OR (value_label IS NOT NULL)
    ),
    CONSTRAINT an_outcome_is_one_of_four CHECK (
        outcome IN ('pending', 'added', 'merged', 'skipped')
    ),
    CONSTRAINT one_row_per_question_per_run UNIQUE (run_id, question)
);

CREATE INDEX IF NOT EXISTS session_survey_proposals_session
    ON session_survey_proposals (session_id, proposed_at DESC);

CREATE TABLE IF NOT EXISTS session_survey_absorptions (
    run_id        UUID PRIMARY KEY,
    session_id    UUID NOT NULL REFERENCES research_sessions(session_id) ON DELETE CASCADE,
    item_id       UUID NOT NULL REFERENCES agenda_items(item_id) ON DELETE CASCADE,
    proposed      INT NOT NULL CHECK (proposed >= 0),
    new_questions INT NOT NULL CHECK (new_questions >= 0),
    merged        INT NOT NULL CHECK (merged >= 0),
    skipped       INT NOT NULL CHECK (skipped >= 0),
    absorbed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT the_parts_do_not_exceed_the_whole CHECK (new_questions + merged + skipped <= proposed)
);

-- An absorption is a record of what happened; rewriting one rewrites the barren
-- streak it feeds.
DROP TRIGGER IF EXISTS trg_session_survey_absorptions_no_change ON session_survey_absorptions;
CREATE TRIGGER trg_session_survey_absorptions_no_change
    BEFORE UPDATE OR DELETE ON session_survey_absorptions
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

GRANT SELECT, INSERT, UPDATE ON session_survey_proposals TO app_role;
REVOKE DELETE, TRUNCATE ON session_survey_proposals FROM app_role;
GRANT SELECT, INSERT ON session_survey_absorptions TO app_role;
REVOKE UPDATE, DELETE, TRUNCATE ON session_survey_absorptions FROM app_role;
