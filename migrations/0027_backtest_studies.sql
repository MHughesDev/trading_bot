-- Set-J Phase 1: Study persistence (spec §1.2, "What the Study logs").
--
-- A Study logs its config + human-readable `question` at CREATION (before any
-- result exists — `question_logged_at` precedes `completed_at`), then attaches
-- its sealed distribution + trial_delta + member references on completion. The
-- distribution is best-member-sealed (INV-2): there is intentionally no
-- "best_run_id" column and the members table carries no rank/order-by-metric.

CREATE TABLE IF NOT EXISTS backtest_studies (
    study_id            TEXT PRIMARY KEY,
    kind                TEXT NOT NULL,          -- parameter_sweep | walk_forward | ...
    base_run_id         TEXT NOT NULL,          -- the "center" config (FK-ish to backtest_runs)
    vary_json           JSONB NOT NULL,         -- the VarySpec
    metric              TEXT NOT NULL,          -- which metric the distribution is over
    null_ref            TEXT,                   -- required iff kind = permutation_null
    question            TEXT NOT NULL,          -- logged up front; defends against post-hoc spin
    selection_rule      TEXT NOT NULL DEFAULT 'none',
    distribution_json   JSONB,                  -- sealed StudyResult.distribution (on completion)
    verdict_json        JSONB,
    trial_delta         BIGINT NOT NULL DEFAULT 0,
    carried_forward_json JSONB,                 -- output of the pre-declared SelectionRule, if any
    unsafe              BOOLEAN NOT NULL DEFAULT FALSE,
    created_by          TEXT,
    question_logged_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at        TIMESTAMPTZ,
    CONSTRAINT chk_permutation_has_null
        CHECK (kind <> 'permutation_null' OR null_ref IS NOT NULL),
    CONSTRAINT chk_completed_after_question
        CHECK (completed_at IS NULL OR completed_at >= question_logged_at)
);

-- Provenance: every member run_id of a study, in INSERTION order (ordinal), not
-- ranked by metric. There is deliberately no performance column here — the best
-- member is not addressable (INV-2 / ADR-002).
CREATE TABLE IF NOT EXISTS backtest_study_members (
    study_id    TEXT NOT NULL REFERENCES backtest_studies(study_id),
    ordinal     INT NOT NULL,           -- insertion order, NOT a rank
    run_id      TEXT NOT NULL,
    PRIMARY KEY (study_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_backtest_studies_kind ON backtest_studies(kind);
CREATE INDEX IF NOT EXISTS idx_backtest_study_members_run ON backtest_study_members(run_id);
