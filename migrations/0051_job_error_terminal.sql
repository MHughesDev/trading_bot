-- 0051 — `jobs.error` carries the terminal reason (ADR-P2-30, AT-60).
--
-- `JobError` gained a REQUIRED `terminal` field: the `TerminalReason` the ledger
-- records when this failure settles a trial. Before it existed, the settlement
-- paths recovered the reason by substring-matching the error code and message,
-- so a reworded message silently changed a trial's censoring and anything
-- unrecognised became `dependency_failure`.
--
-- The old guess runs exactly once more, here, over rows written before the field
-- existed, and is then gone from the code. Rows written from now on carry a
-- reason a worker chose. Without the backfill those historical rows would simply
-- fail to deserialise, which is the point of having no serde default.

UPDATE jobs
SET error = error || jsonb_build_object(
        'terminal',
        CASE
            WHEN error->>'code' = 'cancelled'                             THEN 'cancelled'
            WHEN error->>'code' = 'lost_worker'                           THEN 'preempted_abandoned'
            WHEN error->>'code' IN ('oom', 'out_of_memory')               THEN 'oom'
            WHEN error->>'code' = 'timeout'                               THEN 'timeout'
            WHEN error->>'code' IN ('nan', 'nan_divergence')              THEN 'nan_divergence'
            WHEN error->>'code' IN ('budget_exhausted','budget_exceeded') THEN 'budget_exceeded'
            WHEN error->>'code' LIKE '%data%'                             THEN 'data_error'
            WHEN error->>'code' LIKE '%coverage%'                         THEN 'data_error'
            ELSE 'dependency_failure'
        END)
WHERE error IS NOT NULL
  AND jsonb_typeof(error) = 'object'
  AND NOT (error ? 'terminal')
  -- Rows that predate 0045 can be counted kinds with no `trial_id`. That
  -- constraint was added NOT VALID precisely so they could stay, and *any*
  -- UPDATE to such a row re-checks it and fails. So the backfill steps over
  -- them rather than taking the whole migration down on a job from before the
  -- ledger existed. They keep an error with no `terminal`, which the NOT VALID
  -- constraint below also grandfathers; reading one back fails loudly at the
  -- deserialiser, which is the right place for a row nothing can interpret.
  AND (kind NOT IN ('backtest','sweep','study','gate_advance','train','hpo')
       OR trial_id IS NOT NULL);

-- Every failure written from now on says how its trial ended. A row that does
-- not is refused rather than defaulted.
--
-- NOT VALID, for the same reason 0045's `chk_counted_jobs_have_trial` is: the
-- field postdates some rows, validating history would fail this migration on a
-- database that has been running, and the enforcement that matters is on INSERT
-- and UPDATE — which NOT VALID still gives in full. A constraint that stops the
-- next bad row is worth more than one that refuses to be added at all.
DO $$ BEGIN
    ALTER TABLE jobs
        ADD CONSTRAINT chk_job_error_has_terminal
        CHECK (
            error IS NULL
            OR jsonb_typeof(error) <> 'object'
            OR (error ? 'terminal')
        ) NOT VALID;
EXCEPTION WHEN duplicate_object THEN NULL; END $$;
