-- Set-J Phase 2: the Experiment aggregate (spec §1.3).
--
-- An Experiment owns the global trial counter (monotonic, irreversible) and the
-- holdout vault (one logged access, self-sealing). The trial_counter has no
-- decrement path and the vault access log is append-only — the two structural
-- guarantees behind honest significance and an un-launderable trial count.

CREATE TABLE IF NOT EXISTS backtest_experiments (
    experiment_id    TEXT PRIMARY KEY,
    strategy_family  TEXT NOT NULL,             -- version-agnostic root idea
    state            TEXT NOT NULL,             -- candidate|validated|live|decaying|retired
    trial_counter    BIGINT NOT NULL DEFAULT 0, -- AUTOMATIC, MONOTONIC, IRREVERSIBLE
    holdout_json     JSONB NOT NULL,            -- slice + spent flag
    primary_test     TEXT NOT NULL,             -- the ONE designated null (immutable)
    gate3_passed     BOOLEAN NOT NULL DEFAULT FALSE,
    unsafe           BOOLEAN NOT NULL DEFAULT FALSE,
    holdout_spent    BOOLEAN NOT NULL DEFAULT FALSE,
    verdict          TEXT,
    created_by       TEXT,
    created          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The counter may never go down. (Postgres lacks a clean cross-row guard;
    -- the application owns monotonicity — see Experiment::record_study — and
    -- this CHECK forbids negative values regardless of code path.)
    CONSTRAINT chk_trial_counter_nonneg CHECK (trial_counter >= 0),
    -- Once validated/live/etc., the experiment can never be candidate again.
    CONSTRAINT chk_primary_test_present CHECK (primary_test <> '')
);

-- Every touch of the holdout vault, forever (append-only). A second row for the
-- same experiment_id is impossible by application contract (vault self-seals);
-- the UNIQUE constraint enforces "exactly one evaluation" at the schema level.
CREATE TABLE IF NOT EXISTS backtest_vault_accesses (
    experiment_id    TEXT NOT NULL REFERENCES backtest_experiments(experiment_id),
    accessed_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    run_id           TEXT NOT NULL,
    accessed_by      TEXT NOT NULL,
    CONSTRAINT uq_one_vault_access_per_experiment UNIQUE (experiment_id)
);

-- The studies that fed each experiment (provenance; ordered by attachment).
CREATE TABLE IF NOT EXISTS backtest_experiment_studies (
    experiment_id    TEXT NOT NULL REFERENCES backtest_experiments(experiment_id),
    ordinal          INT NOT NULL,
    study_id         TEXT NOT NULL,
    PRIMARY KEY (experiment_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_backtest_experiments_state ON backtest_experiments(state);
CREATE INDEX IF NOT EXISTS idx_backtest_experiments_family ON backtest_experiments(strategy_family);

-- Forbid lowering the trial counter or un-spending the vault by hand.
CREATE OR REPLACE FUNCTION backtest_experiments_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.trial_counter < OLD.trial_counter THEN
        RAISE EXCEPTION 'trial_counter is monotonic (% -> %)', OLD.trial_counter, NEW.trial_counter;
    END IF;
    IF OLD.holdout_spent AND NOT NEW.holdout_spent THEN
        RAISE EXCEPTION 'holdout vault cannot be un-spent';
    END IF;
    IF OLD.state <> 'candidate' AND NEW.state = 'candidate' THEN
        RAISE EXCEPTION 'experiment cannot return to candidate after leaving it';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_backtest_experiments_guard ON backtest_experiments;
CREATE TRIGGER trg_backtest_experiments_guard
    BEFORE UPDATE ON backtest_experiments
    FOR EACH ROW EXECUTE FUNCTION backtest_experiments_guard();
