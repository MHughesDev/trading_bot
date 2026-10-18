-- 0171 — a sub-run is paid for out of its parent's budget (AGENT-010 §9;
-- Set S task S-2.12).
--
-- SUBAGENTS-DESIGN §6: the lease is spent by exactly one run, and the sum of the
-- leases handed out can never exceed what the parent had. `harness::subrun`
-- makes the first a type (`BudgetLease`: not `Clone`, consumed by value) and the
-- second arithmetic (`BudgetLease::split` subtracts from the parent's remaining).
-- What the type cannot do is outlive the process, and a parent that is suspended
-- on its campaign for forty minutes is exactly the case where it has to: the
-- parent's remaining budget is computed from these rows, not from memory.
--
-- All three are nullable. A sub-run claimed before this migration, or by a parent
-- whose log has no `RunSpec` row (and therefore no recorded cap to subtract
-- from), holds no lease; saying so with NULL is honest, and a zero would claim
-- the worker was given nothing.

ALTER TABLE agent_lifecycle.sub_run
    ADD COLUMN IF NOT EXISTS lease_tokens BIGINT
        CHECK (lease_tokens IS NULL OR lease_tokens >= 0),
    ADD COLUMN IF NOT EXISTS lease_trials INT
        CHECK (lease_trials IS NULL OR lease_trials >= 0),
    ADD COLUMN IF NOT EXISTS lease_steps INT
        CHECK (lease_steps IS NULL OR lease_steps >= 0);

-- The read every claim does: what a parent has already handed out.
CREATE INDEX IF NOT EXISTS sub_run_parent_lease_idx
    ON agent_lifecycle.sub_run (parent_run_id)
    WHERE lease_tokens IS NOT NULL;

COMMENT ON COLUMN agent_lifecycle.sub_run.lease_tokens IS
    'S-2.12. Tokens split from the parent run''s remaining budget for this sub-run. '
    'The parent''s remaining is its cap, minus what it spent, minus the sum of these.';

-- `sub_run` is already granted SELECT, INSERT, UPDATE to app_role (0166), and a
-- table-level grant covers the columns added here. Stated rather than repeated:
-- a second GRANT would be a second place for the two to drift apart.
