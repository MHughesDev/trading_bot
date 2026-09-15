-- The falsification ledger: kills and evaluator defects
-- (EXTERNAL-REVIEW-2026-09 §4; ADR-P6-05, ADR-P6-06).
--
-- The agent's primary output for year one is refutation, not discovery. "Find
-- alpha" yields a usable learning signal about twice a year on a ninety-day lag;
-- "prove this is noise" yields one every few minutes with an unambiguous label.
-- Anything the agent can actually learn has to be scored on the dense signal.
--
-- The degenerate policy — kill everything — is blocked by what a kill *is*: a
-- specific, reproducible, falsifiable defect, checkable by a human in ten
-- minutes. Every constraint below exists to make a bare performance complaint
-- unwritable. A low score is not a kill.
--
-- Scoring, encoded in `mlops.falsification_score()` rather than in application
-- code: evaluator_defect > promotion > kill > zero. An agent scored on
-- promotions that discovers the simulator lets it buy the intrabar low will
-- exploit it silently, because reporting it destroys its best result. The only
-- stable fix is to make the report worth more than the exploit, and a scoring
-- rule that lives in a config file is one that gets tuned downward when
-- promotions look thin.

-- ── the shared reproduction ──────────────────────────────────────────────────
--
-- Embedded in both tables rather than normalised into one: a reproduction
-- belongs to exactly one report, and a join here would let a report exist
-- without one for the duration of a transaction.
CREATE TABLE IF NOT EXISTS mlops.falsification (
  falsification_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id         TEXT NOT NULL,

  -- 'kill'             — a candidate was shown to be noise.
  -- 'evaluator_defect' — the evaluator itself was shown to be wrong.
  report_kind       TEXT NOT NULL CHECK (report_kind IN ('kill','evaluator_defect')),

  -- For a kill: the trial refuted. For a defect: NULL.
  target_trial_id   UUID REFERENCES mlops.trial(trial_id),
  -- For a defect: which part of the platform. For a kill: NULL.
  component         TEXT,
  -- For a defect: what that component claims to guarantee, quoted from the
  -- spec. A defect is a gap between a claim and a behaviour; without the claim
  -- there is only a behaviour, and behaviours are not defects.
  claimed_guarantee TEXT,

  -- For a kill: the defect class, from a closed vocabulary. Free text here
  -- would let "underperforms" back in through the side door, which is the one
  -- thing this table exists to prevent.
  defect_class      TEXT CHECK (defect_class IN (
                      'lookahead','data_artifact','below_cost_floor',
                      'below_tradeable_frontier','fill_model_exploit',
                      'duplicate_of_existing','invalid_metric')),

  -- ── the reproduction ──
  artifact_ref      TEXT   NOT NULL CHECK (length(trim(artifact_ref)) > 0),
  seed              BIGINT NOT NULL,
  expected          TEXT   NOT NULL CHECK (length(trim(expected)) > 0),
  observed          TEXT   NOT NULL CHECK (length(trim(observed)) > 0),
  steps             TEXT[] NOT NULL,

  -- ── verification ──
  verification      TEXT NOT NULL DEFAULT 'unverified'
                      CHECK (verification IN ('unverified','verified','refuted')),
  verified_by       TEXT,
  verified_at       TIMESTAMPTZ,
  -- The regression test that now covers it. Verification closes the question;
  -- only a test closes the hole.
  regression_test   TEXT,

  reported_by       TEXT NOT NULL,
  reporter_kind     TEXT NOT NULL CHECK (reporter_kind IN ('human','agent','scheduler')),
  reported_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- Nothing was refuted if the expectation matched the observation. This is the
  -- constraint that makes "sharpe is only 0.4" unwritable: as a reproduction it
  -- reduces to expected = observed.
  CONSTRAINT chk_falsification_refutes_something
    CHECK (trim(expected) <> trim(observed)),

  -- `cardinality`, never `array_length`: array_length returns NULL for an empty
  -- array and a CHECK evaluating to NULL *passes*. Four constraints the spec
  -- calls REQUIRED were silently accepting empty arrays before AT-42 found it
  -- against a live database. A report a human cannot re-run in ten minutes
  -- scores zero, and one with no steps cannot be re-run at all.
  CONSTRAINT chk_falsification_has_steps
    CHECK (cardinality(steps) >= 1),

  -- Each kind carries its own fields and not the other's. A "kill" with a
  -- component and no target is a defect report filed under the wrong name, and
  -- it would score wrong.
  CONSTRAINT chk_falsification_kind_fields CHECK (
    (report_kind = 'kill'
       AND target_trial_id IS NOT NULL
       AND defect_class    IS NOT NULL
       AND component       IS NULL
       AND claimed_guarantee IS NULL)
    OR
    (report_kind = 'evaluator_defect'
       AND component         IS NOT NULL
       AND claimed_guarantee IS NOT NULL
       AND defect_class      IS NULL)
  ),

  -- A verdict without an author is a verdict nobody can be asked about.
  CONSTRAINT chk_falsification_verified_has_author CHECK (
    verification = 'unverified'
    OR (verified_by IS NOT NULL AND verified_at IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS idx_falsification_tenant_kind
  ON mlops.falsification(tenant_id, report_kind, reported_at DESC);
CREATE INDEX IF NOT EXISTS idx_falsification_target
  ON mlops.falsification(target_trial_id);
-- Open, verified defects with no regression test: the work queue.
CREATE INDEX IF NOT EXISTS idx_falsification_undischarged
  ON mlops.falsification(tenant_id)
  WHERE report_kind = 'evaluator_defect'
    AND verification = 'verified'
    AND regression_test IS NULL;

-- ── the report itself is append-only; the verdict on it is not ───────────────
--
-- A reporter must not be able to edit a claim after someone tried to reproduce
-- it. But verification has to be writable, or nothing can ever be verified. So
-- rather than the blanket mlops.refuse_mutation() used elsewhere, this refuses
-- an UPDATE that touches anything *except* the verification columns.
CREATE OR REPLACE FUNCTION mlops.falsification_claim_is_immutable()
RETURNS TRIGGER AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    RAISE EXCEPTION
      'falsification reports are append-only: a refuted report is kept, because a pattern of '
      'refuted reports is itself the signal that a reporter is guessing';
  END IF;

  IF ROW(NEW.tenant_id, NEW.report_kind, NEW.target_trial_id, NEW.component,
         NEW.claimed_guarantee, NEW.defect_class, NEW.artifact_ref, NEW.seed,
         NEW.expected, NEW.observed, NEW.steps, NEW.reported_by, NEW.reporter_kind)
     IS DISTINCT FROM
     ROW(OLD.tenant_id, OLD.report_kind, OLD.target_trial_id, OLD.component,
         OLD.claimed_guarantee, OLD.defect_class, OLD.artifact_ref, OLD.seed,
         OLD.expected, OLD.observed, OLD.steps, OLD.reported_by, OLD.reporter_kind)
  THEN
    RAISE EXCEPTION
      'the claim in a falsification report is immutable: only verification, verified_by, '
      'verified_at and regression_test may be updated. File a new report instead of editing '
      'this one after somebody tried to reproduce it.';
  END IF;

  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_falsification_claim_immutable ON mlops.falsification;
CREATE TRIGGER trg_falsification_claim_immutable
  BEFORE UPDATE OR DELETE ON mlops.falsification
  FOR EACH ROW EXECUTE FUNCTION mlops.falsification_claim_is_immutable();

-- ── the scoring rule, in the database ────────────────────────────────────────
--
-- Returns an integer whose ORDER is the contract: 3 evaluator_defect,
-- 2 promotion, 1 kill, 0 everything else. The magnitudes mean nothing; the
-- ordering means everything, and it is here rather than in application code so
-- that no scheduler can quietly re-weight it.
CREATE OR REPLACE FUNCTION mlops.falsification_score(
  p_report_kind TEXT,
  p_verification TEXT
) RETURNS INT AS $$
  SELECT CASE
    WHEN p_verification <> 'verified' THEN 0
    WHEN p_report_kind = 'evaluator_defect' THEN 3
    WHEN p_report_kind = 'kill' THEN 1
    ELSE 0
  END;
$$ LANGUAGE sql IMMUTABLE;

COMMENT ON FUNCTION mlops.falsification_score(TEXT, TEXT) IS
  'Falsification scoring. 3 = verified evaluator defect, 2 = promotion (scored elsewhere, '
  'listed here so the ordering is stated in one place), 1 = verified kill, 0 = unverified or '
  'refuted. A verified evaluator defect MUST outrank a promotion: an agent scored on promotions '
  'that finds a simulator bug will exploit it silently, because reporting it destroys its best '
  'result.';

-- ── the view the scheduler reads ─────────────────────────────────────────────
CREATE OR REPLACE VIEW mlops.falsification_scored AS
SELECT
  f.*,
  mlops.falsification_score(f.report_kind, f.verification) AS score,
  (f.report_kind = 'evaluator_defect'
     AND f.verification = 'verified'
     AND f.regression_test IS NOT NULL) AS is_discharged
FROM mlops.falsification f;

-- ── tenancy and grants ───────────────────────────────────────────────────────
--
-- Same shape as 0044: exactly one permissive policy per table, so `USING` and
-- `WITH CHECK` cannot be widened by a second policy OR-ing in beside it.
ALTER TABLE mlops.falsification ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.falsification FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.falsification;
CREATE POLICY tenant_isolation ON mlops.falsification
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The agent may file reports and read its own. It may **not** verify them: a
-- reporter that can mark its own report verified is a reporter that scores
-- whatever it likes, and the ordering in falsification_score() would be
-- decorative. Verification is an UPDATE, and agent_role has no UPDATE here.
GRANT SELECT, INSERT ON mlops.falsification TO agent_role;
GRANT SELECT, INSERT, UPDATE ON mlops.falsification TO app_role;
GRANT SELECT ON mlops.falsification_scored TO app_role, agent_role, internal_ml_role;
GRANT EXECUTE ON FUNCTION mlops.falsification_score(TEXT, TEXT)
  TO app_role, agent_role, internal_ml_role;
