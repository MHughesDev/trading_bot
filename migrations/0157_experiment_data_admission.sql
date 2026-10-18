-- Set P P-2.3: what an Experiment was admitted on, forever.
--
-- `POST /api/backtest/experiments` now checks the persisted `data_qc_reports` grade
-- of every series the Experiment declares (grade D refused, grade C refused without
-- an approved `qc_waiver`). This table is the lineage half: one row per (Experiment,
-- series) recording the report that was read, the grade, and the waiver that
-- admitted it if one did. A waiver is evidence about the Experiment, never a
-- property of the data, so it lives here and not on the report.
--
-- Append-only: the admission an Experiment ran under cannot be rewritten after the
-- fact, in either direction (a later regrade does not retroactively clean or taint
-- it).

CREATE TABLE IF NOT EXISTS experiment_data_admission (
  admission_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  experiment_id TEXT        NOT NULL,
  user_id       UUID        NOT NULL,
  instrument_id TEXT        NOT NULL,
  timeframe     TEXT        NOT NULL,
  -- NULL means the series had no persisted report when the Experiment was created.
  report_id     UUID,
  grade         TEXT        CHECK (grade IN ('A','B','C','D')),
  -- Present only when a grade-C series was admitted by a waiver.
  waiver_id     UUID        REFERENCES approval_requests(approval_id),
  decided_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- A waiver exists for a grade C and only for one: it can never be recorded
  -- against a D (which cannot be waived) or against a series that needed none.
  CONSTRAINT admission_waiver_only_for_c
    CHECK (waiver_id IS NULL OR grade = 'C'),
  CONSTRAINT admission_grade_has_report
    CHECK (grade IS NULL OR report_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS experiment_data_admission_by_experiment
  ON experiment_data_admission (experiment_id);

DROP TRIGGER IF EXISTS trg_experiment_data_admission_immutable ON experiment_data_admission;
CREATE TRIGGER trg_experiment_data_admission_immutable
  BEFORE UPDATE OR DELETE ON experiment_data_admission
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

GRANT SELECT, INSERT ON experiment_data_admission TO app_role;
