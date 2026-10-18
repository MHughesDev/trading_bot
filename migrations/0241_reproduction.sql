-- 0241 — a reproduction is not a look (AGENT-013 §6.9, §8.4; ADR-P2-17; S-10.8,
-- ADR-S10-18, INV-18).
--
-- ## The question
--
-- `lifecycle.reproduce` re-runs a stored result under its own receipt and reports
-- whether the platform still computes the same number. A re-run needs an executor,
-- the executor takes a `TrialTicket` (INV-16: no compute without one), and a ticket
-- is minted only by registering a trial. Registering a trial for a reproduction
-- would make every check of the platform cost a trial, change `trials_counted`, the
-- exploration floor's denominator and `N_eff`, and put a duplicate of a registered
-- look into the hash chain — "a platform that charged for that would be a platform
-- where nobody checked" (`backtest::receipt`).
--
-- ## The decision
--
-- A reproduction runs under a **reproduction ticket**: a `TrialTicket` the ledger
-- mints for a trial that already completed, which names that trial and carries no
-- trial row of its own. It cannot be transitioned, settled or given a return
-- series (those are trial operations and refuse it), and its one terminal act is a
-- row here. Nothing in `mlops.trial` moves, so every statistic that reads it reads
-- what it read before.
--
-- ## Why a table and not a column on `mlops.trial`
--
-- A trial is a look. A reproduction is a *check on* a look, and there are many per
-- look over time. One-to-many, append-only, outside the trial's hash chain
-- (extending the chain would invalidate every recorded row — ADR-I-14).

CREATE TABLE IF NOT EXISTS mlops.reproduction (
    reproduction_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id          TEXT NOT NULL,
    -- The registered look being checked. A reproduction of nothing is not one.
    original_trial_id  UUID NOT NULL REFERENCES mlops.trial(trial_id),
    run_id             TEXT NOT NULL,
    outcome            TEXT NOT NULL,
    -- The receipt the original carried, when it carried one; NULL means the
    -- original predates receipts and the outcome can only be bit-equality of the
    -- number, which `receipt_missing` says out loud.
    receipt_stored     JSONB,
    receipt_rerun      JSONB NOT NULL,
    receipt_missing    BOOLEAN NOT NULL,
    -- Named differences when the receipt moved; empty otherwise.
    differences        JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- The number before and after. Only drift carries them: a bit-identical
    -- result has one number, and a changed receipt explains the difference by
    -- the receipt rather than by the values.
    was_value          DOUBLE PRECISION,
    now_value          DOUBLE PRECISION,
    reproduced_by      TEXT NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT a_reproduction_has_a_known_outcome CHECK (
        outcome IN ('bit_identical', 'drifted', 'receipt_changed')
    ),
    CONSTRAINT a_drift_names_both_numbers CHECK (
        outcome <> 'drifted' OR (was_value IS NOT NULL AND now_value IS NOT NULL)
    ),
    CONSTRAINT a_changed_receipt_names_what_changed CHECK (
        outcome <> 'receipt_changed' OR jsonb_array_length(differences) > 0
    ),
    CONSTRAINT a_reproduction_names_its_run CHECK (btrim(run_id) <> ''),
    CONSTRAINT a_reproduction_names_who_ran_it CHECK (btrim(reproduced_by) <> '')
);

-- A reproduction is a record of what a check found. Editing or deleting one would
-- make it a record of what somebody now wishes it had found.
DROP TRIGGER IF EXISTS trg_reproduction_no_update ON mlops.reproduction;
CREATE TRIGGER trg_reproduction_no_update
    BEFORE UPDATE OR DELETE ON mlops.reproduction
    FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE INDEX IF NOT EXISTS reproduction_by_trial_idx
    ON mlops.reproduction (tenant_id, original_trial_id, created_at DESC);
-- The nightly job's read: the last thirty days, newest first.
CREATE INDEX IF NOT EXISTS reproduction_recent_idx
    ON mlops.reproduction (tenant_id, created_at DESC);

ALTER TABLE mlops.reproduction ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.reproduction FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.reproduction;
CREATE POLICY tenant_isolation ON mlops.reproduction
    USING (tenant_id = current_setting('app.tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- Append-only for the platform's role, like every other ledger table (0044).
GRANT SELECT, INSERT ON mlops.reproduction TO app_role;
