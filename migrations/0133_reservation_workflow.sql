-- 0133 — the workflow a look was taken under (AGENT-013 §3, §11; S-10.14).
--
-- ## The question it answers
--
-- S-10.14's acceptance: "the ledger answers *how many trials were replications*
-- from the workflow column". It could not — `workflow` was on `agent_runs` and
-- nowhere else, and `mlops.trial` has no run id to join through.
--
-- ## Why here rather than on `mlops.trial`
--
-- Because of what a workflow *is*. `investigate`, `replicate` and `falsify` are
-- properties of the **run** and of the call it made — the stage skeleton the run
-- is following when it reaches for a tool. They are not properties of the
-- compute: one Run of a backtest is the same arithmetic whichever workflow asked
-- for it, and `backtest::run` has no idea which did.
--
-- So it belongs on the row that records the call, which is the one this
-- separation created (ADR-S10-10). Putting it on `mlops.trial` would also have
-- meant a column outside that table's hash chain — the chain covers a named list
-- of columns, and extending it would invalidate every row already recorded,
-- which is ADR-I-14's argument. This table has no chain, so there is no such
-- trade to make.
--
-- ## Nullable, deliberately
--
-- A front door with no agent run behind it follows no stage skeleton. Refusing
-- the reservation for want of a workflow would refuse the call, which is the
-- failure `0132` exists to end. A NULL means "no workflow", not "unknown", and
-- the query that counts replications counts what it can name.

ALTER TABLE mlops.trial_reservation
    ADD COLUMN IF NOT EXISTS workflow TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'a_workflow_is_one_of_the_seven'
    ) THEN
        -- The seven stage skeletons of AGENT-013 §3, mirroring
        -- `harness::state::Workflow`. A workflow the harness can set and this
        -- refuses is a run whose every trial-spending call fails at the door.
        ALTER TABLE mlops.trial_reservation
            ADD CONSTRAINT a_workflow_is_one_of_the_seven CHECK (
                workflow IS NULL OR workflow IN (
                    'investigate',
                    'falsify',
                    'replicate',
                    'compare',
                    'diagnose',
                    'survey',
                    'optimise'
                )
            );
    END IF;
END
$$;

-- "How many of this tenant's looks were replications" — the question S-10.14
-- asks, answered by an index rather than a scan.
CREATE INDEX IF NOT EXISTS trial_reservation_workflow_idx
    ON mlops.trial_reservation (tenant_id, workflow, created_at DESC)
    WHERE workflow IS NOT NULL;
