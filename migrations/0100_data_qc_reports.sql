-- Data-quality reports (DATA-005 §7; Set P task P-2.2).
--
-- One row per (instrument, venue, timeframe, window, config version). The last
-- of those is part of the key on purpose: re-grading the same window under
-- revised thresholds produces a **new** report rather than overwriting the old
-- one, so "this series was an A in September" stays answerable after the
-- thresholds tighten in October.
--
-- The reasons are the point. A grade with no reasons is unreviewable and
-- unappealable: the researcher refused at Experiment creation cannot tell
-- whether the answer is "collect more history" or "this venue is lying", and
-- the two have very different next actions.

CREATE TABLE IF NOT EXISTS data_qc_reports (
  report_id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),

  instrument_id   TEXT NOT NULL,
  venue_id        TEXT NOT NULL,
  timeframe       TEXT NOT NULL,
  window_from     TIMESTAMPTZ NOT NULL,
  window_to       TIMESTAMPTZ NOT NULL,

  grade           TEXT NOT NULL CHECK (grade IN ('A','B','C','D')),
  -- Never empty. An A carries "every check passed" rather than silence, so a
  -- reader never has to guess whether the reasons were omitted or absent.
  reasons         JSONB NOT NULL,
  -- Every check's kind, outcome and metric. Stored so a threshold change can
  -- re-grade history WITHOUT re-running the battery over every series, which is
  -- the expensive half.
  checks          JSONB NOT NULL,

  -- The thresholds that produced this letter. Two reports with different
  -- versions were not computed by the same rules, whatever their letters say.
  config_version  TEXT NOT NULL CHECK (config_version ~ '^[0-9a-f]{16}$'),
  -- Content digest of the findings, excluding prose. Rewording a message must
  -- not invalidate a stored digest; changing an outcome always must.
  digest          TEXT NOT NULL CHECK (digest ~ '^[0-9a-f]{32}$'),

  bars_examined   INT NOT NULL CHECK (bars_examined >= 0),
  expected_bars   BIGINT NOT NULL CHECK (expected_bars >= 0),
  -- What fraction of the eight checks actually produced evidence. A series
  -- where six could not run has not been graded so much as guessed at, and this
  -- is the column that says so.
  evidence_completeness DOUBLE PRECISION NOT NULL
                    CHECK (evidence_completeness BETWEEN 0 AND 1),

  graded_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

  CONSTRAINT chk_qc_window_ordered CHECK (window_to > window_from),
  -- `jsonb_array_length`, not `array_length`: the latter returns NULL for an
  -- empty Postgres array and a CHECK evaluating to NULL *passes*. This column is
  -- JSONB so the jsonb variant is the right one and returns 0 honestly — but
  -- the distinction is worth stating where the next person will read it.
  CONSTRAINT chk_qc_has_reasons CHECK (
    jsonb_typeof(reasons) = 'array' AND jsonb_array_length(reasons) >= 1
  ),
  CONSTRAINT chk_qc_has_checks CHECK (
    jsonb_typeof(checks) = 'array' AND jsonb_array_length(checks) = 8
  ),

  -- Re-grading under the same config is idempotent; under a new one it is a new
  -- row. This is what makes history answerable.
  CONSTRAINT uq_qc_report UNIQUE
    (instrument_id, venue_id, timeframe, window_from, window_to, config_version)
);

CREATE INDEX IF NOT EXISTS idx_qc_reports_series
  ON data_qc_reports (instrument_id, venue_id, timeframe, graded_at DESC);
-- The query the refusal path runs on every Experiment creation: the newest
-- grade for a series. Partial, because A and B never reach the refusal branch.
CREATE INDEX IF NOT EXISTS idx_qc_reports_blocking
  ON data_qc_reports (instrument_id, venue_id, timeframe, graded_at DESC)
  WHERE grade IN ('C', 'D');

-- A report is what the battery found when it ran. Re-running produces a new row
-- rather than editing an old one, for the same reason every other evidence
-- table on this platform is append-only: a grade somebody edited is a grade
-- nobody can rely on.
DROP TRIGGER IF EXISTS trg_data_qc_reports_immutable ON data_qc_reports;
CREATE TRIGGER trg_data_qc_reports_immutable
  BEFORE UPDATE OR DELETE ON data_qc_reports
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- The newest report per series under any config, which is what a manifest and
-- the refusal path both read.
CREATE OR REPLACE VIEW data_qc_current AS
SELECT DISTINCT ON (instrument_id, venue_id, timeframe)
  instrument_id, venue_id, timeframe, grade, reasons, config_version,
  evidence_completeness, bars_examined, window_from, window_to, graded_at, report_id
FROM data_qc_reports
ORDER BY instrument_id, venue_id, timeframe, graded_at DESC;

COMMENT ON TABLE data_qc_reports IS
  'DATA-005 §7 quality grades. Append-only. The config version is part of the '
  'uniqueness key so re-grading under revised thresholds adds a row rather than '
  'overwriting history. `reasons` is never empty: a grade with no reasons is '
  'unreviewable and unappealable.';
