-- 0190 — the ramp's two chosen numbers, and the fleet's three ceilings (Set T,
-- T-3.3 and T-2.1; ADR-T3-04, ADR-T2-04).
--
-- Two decisions that were placeholders in code ("14 days", "20 bars") or missing
-- altogether (the ceilings) become rows, so that "what was the policy when this
-- period was judged" is answerable and changing it is a recorded act.
--
-- Both tables are append-only: a new row supersedes, the old one stays legible.
-- `app_role` can read and cannot write `ramp_policy` (REVOKE below): the numbers a
-- review is judged by are not something the process under review gets to edit.

-- ── ramp_policy ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS ramp_policy (
  policy_id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  -- The review period, in days, for a deployment that declared none (a
  -- `legacy_ungated` one).
  legacy_review_period_days  INT NOT NULL CHECK (legacy_review_period_days BETWEEN 1 AND 90),
  -- The fewest `observed` forward bars a period needs before it can be judged at
  -- all. Below it the period is NotComputable.
  min_observed_bars          INT NOT NULL CHECK (min_observed_bars >= 1),
  decided_by                 TEXT NOT NULL CHECK (length(trim(decided_by)) > 0),
  decided_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
  basis                      TEXT NOT NULL CHECK (length(trim(basis)) > 0)
);

DROP TRIGGER IF EXISTS trg_ramp_policy_immutable ON ramp_policy;
CREATE TRIGGER trg_ramp_policy_immutable
  BEFORE UPDATE OR DELETE ON ramp_policy
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

CREATE OR REPLACE VIEW current_ramp_policy AS
SELECT policy_id, legacy_review_period_days, min_observed_bars, decided_by, decided_at, basis
  FROM ramp_policy
 ORDER BY decided_at DESC, policy_id DESC
 LIMIT 1;

-- ADR-T3-04. 14 days: two weeks is long enough to span a weekend and a weekly
-- cycle for the 1h bars the first deployments run on, and short enough that a
-- deployment which cannot be measured is flagged inside a fortnight rather than
-- a quarter. 20 bars: the smallest sample from which a mean signal match, a
-- slippage ratio and a turnover deviation are more than one or two coin flips;
-- the profile's own Gate 15 asks for more, so this is the floor for being
-- *judged*, not for passing.
INSERT INTO ramp_policy (legacy_review_period_days, min_observed_bars, decided_by, basis)
SELECT 14, 20, 'adr:ADR-T3-04',
       '14-day review period for a deployment that declared none; 20 observed forward bars before a period can be judged. Both are floors, not targets: the profile''s Gate 15 thresholds still decide pass or fail. Rejected: 7 days (a single weekend dominates the sample) and 30 days (an unmeasurable deployment would run a month before it is even flagged).'
 WHERE NOT EXISTS (SELECT 1 FROM ramp_policy);

-- 0044 gave `app_role` full DML on every new public table through default
-- privileges, so "cannot write" has to be a REVOKE, not an absence of a GRANT
-- (0167 found the same for the alert inbox). The immutability trigger already
-- refuses UPDATE and DELETE; this is what refuses the INSERT of a policy row that
-- would lower a floor.
REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON ramp_policy FROM app_role;
GRANT SELECT ON ramp_policy, current_ramp_policy TO app_role;
GRANT SELECT ON ramp_policy, current_ramp_policy TO agent_role;

-- ── the three fleet ceilings (T-2.1) ────────────────────────────────────────
--
-- 0182 left these undeclared on purpose, so that an approval fails closed until a
-- person chose. ADR-T2-04 is that choice, recorded with its reason. They bound
-- PAPER deployments only (the venue class is a CHECK of one value), so a wrong
-- number costs a refused approval, not money; raising one is a new row.

INSERT INTO deployment_ceiling (name, value, declared_by, note)
SELECT v.name, v.value, 'adr:ADR-T2-04', v.note
  FROM (VALUES
    ('max_concurrent', 10::numeric,
     'ten concurrent paper deployments: enough running strategies for Gate 15 evidence to accrue at a useful rate, few enough that a person can read every one of them on the deployments page'),
    ('max_aggregate_capital', 100000::numeric,
     'summed paper capital across all running deployments, in quote currency; ten deployments at the 10k paper start the first deployments use'),
    ('max_instrument_exposure', 25000::numeric,
     'per instrument, aggregated across every instance of a revision (capacity::Exposure); a quarter of the aggregate, so one instrument cannot be the whole fleet')
  ) AS v(name, value, note)
 WHERE NOT EXISTS (SELECT 1 FROM deployment_ceiling c WHERE c.name = v.name);
