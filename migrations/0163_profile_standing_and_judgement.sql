-- 0163 - a profile's current standing and its `judgement` pass rates
-- (AGENT-014 section 7 row 9, 8; AGENT-009 section 5; Set S tasks S-11.7, S-11.9; ADR-P11-04).
--
-- Two questions the continuous-research gate asks about a tier profile and the
-- platform could not answer:
--
--   * **Is its scorecard still good?** `couplings::ProfileHealth` has three words
--     (measured, degraded, withdrawn) and the driver read the profile's *static*
--     tier, so "withdrawing a scorecard pauses a session on that profile" had
--     nothing to read. `profile_standing` is that record.
--   * **Does it pass `judgement` for this workflow?** `charter::admit` needs a
--     pass rate per (profile, workflow) and `eval_scorecards.metrics` is a
--     harness-version aggregate. `profile_judgement` is that record, so
--     `autonomous` can be granted for the first time - and only on a measurement.
--
-- Both are append-only. A standing is a fact *as of when it was recorded*; the
-- reader takes the latest row, and an older one is the history a human audits
-- ("when did this profile degrade, and who said so").

CREATE TABLE IF NOT EXISTS profile_standing (
    standing_id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id           TEXT NOT NULL,
    health               TEXT NOT NULL,
    reason               TEXT NOT NULL,
    -- The profile's own line for `autonomous` (AGENT-014 section 8: "the profile's
    -- threshold"). NULL is "not declared" and is read as the strictest, 1.0.
    autonomous_threshold DOUBLE PRECISION,
    -- Who said so: the eval harness, an operator, or the platform noticing a
    -- credential had gone.
    recorded_by          TEXT NOT NULL,
    recorded_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT standing_is_one_of_three CHECK (health IN ('measured', 'degraded', 'withdrawn')),
    CONSTRAINT standing_names_the_profile CHECK (btrim(profile_id) <> ''),
    CONSTRAINT standing_gives_a_reason CHECK (btrim(reason) <> ''),
    CONSTRAINT threshold_is_a_rate CHECK (
        autonomous_threshold IS NULL OR (autonomous_threshold > 0 AND autonomous_threshold <= 1)
    )
);

CREATE INDEX IF NOT EXISTS profile_standing_latest
    ON profile_standing (profile_id, recorded_at DESC);

CREATE TABLE IF NOT EXISTS profile_judgement (
    judgement_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id    TEXT NOT NULL,
    workflow      TEXT NOT NULL,
    pass_rate     DOUBLE PRECISION NOT NULL,
    -- The suite's own pass line for this workflow. Stored with the rate so the
    -- verdict is reproducible from the row; the suite can move its line and old
    -- rows keep the line they were judged against.
    pass_line     DOUBLE PRECISION NOT NULL,
    trials        INT NOT NULL,
    suite_version TEXT NOT NULL,
    measured_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT judgement_names_what CHECK (btrim(profile_id) <> '' AND btrim(workflow) <> ''),
    CONSTRAINT rate_is_a_rate CHECK (pass_rate >= 0 AND pass_rate <= 1),
    CONSTRAINT line_is_a_rate CHECK (pass_line > 0 AND pass_line <= 1),
    -- A pass rate over no trials is not a measurement.
    CONSTRAINT judgement_has_trials CHECK (trials > 0)
);

CREATE INDEX IF NOT EXISTS profile_judgement_latest
    ON profile_judgement (profile_id, workflow, measured_at DESC);

DROP TRIGGER IF EXISTS trg_profile_standing_append_only ON profile_standing;
CREATE TRIGGER trg_profile_standing_append_only
    BEFORE UPDATE OR DELETE ON profile_standing
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

DROP TRIGGER IF EXISTS trg_profile_judgement_append_only ON profile_judgement;
CREATE TRIGGER trg_profile_judgement_append_only
    BEFORE UPDATE OR DELETE ON profile_judgement
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

GRANT SELECT, INSERT ON profile_standing, profile_judgement TO app_role;
REVOKE UPDATE, DELETE, TRUNCATE ON profile_standing, profile_judgement FROM app_role;
