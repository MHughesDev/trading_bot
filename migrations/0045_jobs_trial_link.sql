-- Link trial-counted jobs to the Trial Ledger (INV-16). A job that dispatches compute
-- for a counted kind carries the trial it was registered as. NOT VALID so the
-- pre-ledger history stays readable; every new row is checked.

ALTER TABLE jobs ADD COLUMN IF NOT EXISTS trial_id UUID;
CREATE INDEX IF NOT EXISTS idx_jobs_trial ON jobs(trial_id);

DO $$ BEGIN
  ALTER TABLE jobs ADD CONSTRAINT chk_counted_jobs_have_trial
    CHECK (kind NOT IN ('backtest','sweep','study','gate_advance','train','hpo') OR trial_id IS NOT NULL) NOT VALID;
EXCEPTION WHEN duplicate_object THEN NULL; END $$;
