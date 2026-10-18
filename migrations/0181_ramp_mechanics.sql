-- 0181 — ramp mechanics (Set T T-3.1, T-3.2, T-3.3; ADR-P2-27, SPEC §12.3 Gate 16).
--
-- Two quantities move, and they are kept apart because they are not the same
-- thing:
--
--   * the capital RUNG (`mlops.capital_authorisation`, 0053): 0 / 10 / 25 / 50 /
--     100 % of intended size. It goes up one rung at a time, only under a
--     capital-authorising profile, only on all sixteen gates;
--   * the SIZE of a deployment in the platform's own engine: 1, 1/2, 1/4, 1/8,
--     then zero. It can only go DOWN. A deployment that fails its review period
--     is halved at the period boundary, automatically, until it reaches zero
--     (ADR-P2-27: nothing sits at its current size indefinitely on the strength of
--     never having been measured).
--
-- Both are driven by one clock: `ramp_period`, whose boundaries are recorded, not
-- computed from wall-clock arithmetic. Each period ends in exactly one
-- `ramp_review`. The review is the EVENT: the ladder's history is the sequence of
-- reviews, and replays to the same state.
--
-- What the database refuses, rather than the worker declining to do:
--
--   * opening period n+1 before period n has been reviewed, or with a start that
--     is not period n's recorded end (so a failing period cannot be skipped over);
--   * a review of a period that has not ended;
--   * a failing period with any outcome but `halve` or `zero`; and a passing one
--     with either;
--   * `step_up` without all of: profile authorises capital, Gate 16 computed and
--     passed;
--   * a capital raise (`mlops.capital_authorisation`) that is not the consequence
--     of exactly one `step_up` review of that deployment, once.

CREATE TABLE IF NOT EXISTS ramp_period (
  period_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  deployment_id UUID NOT NULL REFERENCES strategy_deployment (deployment_id),
  period_no     INT NOT NULL CHECK (period_no >= 1),
  starts_at     TIMESTAMPTZ NOT NULL,
  ends_at       TIMESTAMPTZ NOT NULL,
  opened_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (ends_at > starts_at),
  UNIQUE (deployment_id, period_no)
);

CREATE TABLE IF NOT EXISTS ramp_review (
  review_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  deployment_id  UUID NOT NULL,
  period_no      INT NOT NULL,
  FOREIGN KEY (deployment_id, period_no) REFERENCES ramp_period (deployment_id, period_no),
  UNIQUE (deployment_id, period_no),
  profile_id     TEXT NOT NULL REFERENCES mlops.gate_profile (profile_id),
  -- hold: passed, size unchanged. step_up: passed under a capital-authorising
  -- profile and Gate 16 said so. halve / zero: the period failed.
  outcome        TEXT NOT NULL CHECK (outcome IN ('hold', 'step_up', 'halve', 'zero')),
  passed         BOOLEAN NOT NULL,
  -- Gate 16 could not be computed (an incomplete or unobserved period). Not a
  -- pass. A period that cannot be judged has not been measured, and ADR-P2-27
  -- does not let anything sit unmeasured at its current size.
  not_computable_reason TEXT,
  halvings_before SMALLINT NOT NULL CHECK (halvings_before BETWEEN 0 AND 4),
  halvings_after  SMALLINT NOT NULL CHECK (halvings_after BETWEEN 0 AND 4),
  -- The evidence the verdict was computed from, stored whole, with its hash:
  -- re-running the judge over `evidence` must reproduce `outcome`.
  evidence       JSONB NOT NULL CHECK (jsonb_typeof(evidence) = 'object'),
  evidence_hash  TEXT NOT NULL CHECK (evidence_hash ~ '^[0-9a-f]{64}$'),
  reason         TEXT NOT NULL CHECK (length(trim(reason)) > 0),
  decided_by     TEXT NOT NULL CHECK (length(trim(decided_by)) > 0),
  decided_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- A failing period steps down; a passing one does not. There is no third
  -- value, and no configuration key that adds one.
  CONSTRAINT chk_ramp_review_failure_steps_down
    CHECK ((passed AND outcome IN ('hold', 'step_up'))
        OR (NOT passed AND outcome IN ('halve', 'zero'))),
  CONSTRAINT chk_ramp_review_size_arithmetic
    CHECK ((outcome IN ('hold', 'step_up') AND halvings_after = halvings_before)
        OR (outcome = 'halve' AND halvings_before <= 2 AND halvings_after = halvings_before + 1)
        OR (outcome = 'zero'  AND halvings_before = 3 AND halvings_after = 4)),
  CONSTRAINT chk_ramp_review_step_up_is_computed
    CHECK (outcome <> 'step_up' OR not_computable_reason IS NULL)
);

-- ── period rules ────────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION ramp_period_is_continuous() RETURNS trigger AS $$
DECLARE
  prev ramp_period%ROWTYPE;
  prev_after SMALLINT;
BEGIN
  IF NEW.period_no = 1 THEN
    RETURN NEW;
  END IF;
  SELECT * INTO prev FROM ramp_period
   WHERE deployment_id = NEW.deployment_id AND period_no = NEW.period_no - 1;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'deployment %: period % cannot open before period %', NEW.deployment_id, NEW.period_no, NEW.period_no - 1;
  END IF;
  IF NEW.starts_at <> prev.ends_at THEN
    RAISE EXCEPTION 'deployment %: period % must start where period % ended (%), not %',
      NEW.deployment_id, NEW.period_no, prev.period_no, prev.ends_at, NEW.starts_at;
  END IF;
  SELECT halvings_after INTO prev_after FROM ramp_review
   WHERE deployment_id = NEW.deployment_id AND period_no = prev.period_no;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'deployment %: period % has not been reviewed; a period is not skipped', NEW.deployment_id, prev.period_no;
  END IF;
  IF prev_after = 4 THEN
    RAISE EXCEPTION 'deployment % has been stepped down to zero; no further period opens', NEW.deployment_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ramp_period_is_continuous ON ramp_period;
CREATE TRIGGER trg_ramp_period_is_continuous
  BEFORE INSERT ON ramp_period
  FOR EACH ROW EXECUTE FUNCTION ramp_period_is_continuous();

CREATE OR REPLACE FUNCTION ramp_review_rules() RETURNS trigger AS $$
DECLARE
  p        ramp_period%ROWTYPE;
  prev_after SMALLINT;
  authorises BOOLEAN;
BEGIN
  SELECT * INTO p FROM ramp_period
   WHERE deployment_id = NEW.deployment_id AND period_no = NEW.period_no;
  IF now() < p.ends_at THEN
    RAISE EXCEPTION 'deployment % period % ends at %; it is not reviewed before it has ended',
      NEW.deployment_id, NEW.period_no, p.ends_at;
  END IF;
  -- The ladder is continuous: this review starts from where the last one left off.
  SELECT halvings_after INTO prev_after FROM ramp_review
   WHERE deployment_id = NEW.deployment_id AND period_no = NEW.period_no - 1;
  IF FOUND THEN
    IF NEW.halvings_before <> prev_after THEN
      RAISE EXCEPTION 'deployment % is at % halvings, not %', NEW.deployment_id, prev_after, NEW.halvings_before;
    END IF;
  ELSIF NEW.period_no = 1 AND NEW.halvings_before <> 0 THEN
    RAISE EXCEPTION 'deployment % starts at full size, not % halvings', NEW.deployment_id, NEW.halvings_before;
  END IF;
  IF NEW.outcome = 'step_up' THEN
    SELECT authorises_capital INTO authorises FROM mlops.gate_profile WHERE profile_id = NEW.profile_id;
    IF authorises IS NOT TRUE THEN
      RAISE EXCEPTION 'profile % does not authorise capital; a period passed under it is evidence, not a rung', NEW.profile_id;
    END IF;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_ramp_review_rules ON ramp_review;
CREATE TRIGGER trg_ramp_review_rules
  BEFORE INSERT ON ramp_review
  FOR EACH ROW EXECUTE FUNCTION ramp_review_rules();

DROP TRIGGER IF EXISTS trg_ramp_period_immutable ON ramp_period;
CREATE TRIGGER trg_ramp_period_immutable
  BEFORE UPDATE OR DELETE ON ramp_period
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

DROP TRIGGER IF EXISTS trg_ramp_review_immutable ON ramp_review;
CREATE TRIGGER trg_ramp_review_immutable
  BEFORE UPDATE OR DELETE ON ramp_review
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── a capital raise is the consequence of a review ──────────────────────────

ALTER TABLE mlops.capital_authorisation
  ADD COLUMN IF NOT EXISTS ramp_review_id UUID REFERENCES ramp_review (review_id);

-- One review, one raise.
CREATE UNIQUE INDEX IF NOT EXISTS uq_capital_authorisation_one_raise_per_review
  ON mlops.capital_authorisation (ramp_review_id) WHERE ramp_review_id IS NOT NULL;

CREATE OR REPLACE FUNCTION capital_raise_needs_a_review() RETURNS trigger AS $$
DECLARE
  r ramp_review%ROWTYPE;
BEGIN
  IF NEW.direction <> 'raise' THEN
    RETURN NEW;
  END IF;
  IF NEW.ramp_review_id IS NULL THEN
    RAISE EXCEPTION 'a capital raise is the consequence of a completed review period (Gate 16); it names none';
  END IF;
  SELECT * INTO r FROM ramp_review WHERE review_id = NEW.ramp_review_id;
  IF NOT FOUND OR r.outcome <> 'step_up' OR NOT r.passed THEN
    RAISE EXCEPTION 'review % is not a passed step_up; a raise needs one', NEW.ramp_review_id;
  END IF;
  IF r.deployment_id::text <> NEW.subject OR r.profile_id <> NEW.profile_id THEN
    RAISE EXCEPTION 'review % is for another deployment or profile than this raise', NEW.ramp_review_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_capital_raise_needs_a_review ON mlops.capital_authorisation;
CREATE TRIGGER trg_capital_raise_needs_a_review
  BEFORE INSERT ON mlops.capital_authorisation
  FOR EACH ROW EXECUTE FUNCTION capital_raise_needs_a_review();

-- ── what the platform reads ─────────────────────────────────────────────────

-- A deployment's size, as halvings: 0 = full, 1 = half, 2 = quarter, 3 = eighth,
-- 4 = zero. A deployment with no review is at full size.
CREATE OR REPLACE VIEW deployment_size AS
SELECT DISTINCT ON (deployment_id)
       deployment_id, period_no, halvings_after AS halvings, outcome, decided_at
  FROM ramp_review
 ORDER BY deployment_id, period_no DESC;

GRANT SELECT, INSERT ON ramp_period TO app_role;
GRANT SELECT, INSERT ON ramp_review TO app_role;
GRANT SELECT         ON deployment_size TO app_role;
GRANT SELECT         ON ramp_period, ramp_review, deployment_size TO agent_role;
