-- 0053 — the capital ramp lives in the registry (SPEC §12.3 Gate 16, ADR-P2-14,
-- AT-65).
--
-- "The registry is the only path to capital." That is either a slogan or a
-- table, and this is the table. Every change to how much of its intended size a
-- strategy may trade is an append-only, hash-free but immutable row saying who
-- decided, under which gate profile, on how many passed gates, and in which
-- direction.
--
-- Three rules are enforced here rather than in the promotion code, because the
-- promotion code is one refactor away from not enforcing them:
--
--   1. the fraction is one of five rungs and nothing else — "we're running it at
--      37 %" is a number somebody picked, not a rung somebody earned;
--   2. a **raise** requires a profile whose `authorises_capital` is TRUE, so
--      sixteen passes under `paper_v1` still authorise nothing (AT-65);
--   3. a raise moves at most one rung, and only with all sixteen gates passed.
--
-- Lowering is unconstrained on purpose. Making a strategy smaller is never the
-- dangerous direction, and a reduction that needed approval is a reduction that
-- happens after the loss instead of before it.

CREATE TABLE IF NOT EXISTS mlops.capital_authorisation (
  authorisation_id  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id         TEXT NOT NULL,
  -- The strategy lineage this authorises. One ladder per lineage.
  subject           TEXT NOT NULL,
  profile_id        TEXT NOT NULL REFERENCES mlops.gate_profile(profile_id),
  previous_fraction NUMERIC(4,3) NOT NULL
                    CHECK (previous_fraction IN (0, 0.100, 0.250, 0.500, 1.000)),
  allowed_fraction  NUMERIC(4,3) NOT NULL
                    CHECK (allowed_fraction IN (0, 0.100, 0.250, 0.500, 1.000)),
  direction         TEXT NOT NULL CHECK (direction IN ('raise','hold','step_down','halt')),
  gates_passed      INT NOT NULL CHECK (gates_passed BETWEEN 0 AND 16),
  reason            TEXT NOT NULL CHECK (length(reason) > 0),
  decided_by        TEXT NOT NULL,
  decided_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- The direction has to match the arithmetic. A row claiming `step_down` while
  -- raising the number is the shape a mistake takes when nobody checks.
  CHECK (
    (direction = 'raise'     AND allowed_fraction > previous_fraction)
    OR (direction = 'hold'      AND allowed_fraction = previous_fraction)
    OR (direction = 'step_down' AND allowed_fraction < previous_fraction)
    OR (direction = 'halt'      AND allowed_fraction = 0)
  ),
  -- A raise is a promotion, and a promotion needs the whole stack.
  CHECK (direction <> 'raise' OR gates_passed = 16)
);

CREATE INDEX IF NOT EXISTS idx_capital_authorisation_current
  ON mlops.capital_authorisation (tenant_id, subject, decided_at DESC);

DROP TRIGGER IF EXISTS trg_capital_authorisation_immutable ON mlops.capital_authorisation;
CREATE TRIGGER trg_capital_authorisation_immutable
  BEFORE UPDATE OR DELETE ON mlops.capital_authorisation
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.capital_authorisation ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.capital_authorisation FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.capital_authorisation;
CREATE POLICY tenant_isolation ON mlops.capital_authorisation
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- ── the rules a code path cannot skip ───────────────────────────────────────
CREATE OR REPLACE FUNCTION mlops.check_capital_raise()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    authorises BOOLEAN;
    rungs      NUMERIC[] := ARRAY[0, 0.100, 0.250, 0.500, 1.000];
    prev_rung  INT;
    next_rung  INT;
    latest     NUMERIC;
BEGIN
    -- The ladder is continuous: a new row starts from where the last one left
    -- off, not from whatever the writer believes the current rung to be.
    SELECT allowed_fraction INTO latest
    FROM mlops.capital_authorisation
    WHERE tenant_id = NEW.tenant_id AND subject = NEW.subject
    ORDER BY decided_at DESC, authorisation_id DESC
    LIMIT 1;

    IF latest IS NULL THEN
        IF NEW.previous_fraction <> 0 THEN
            RAISE EXCEPTION
                'strategy % has no capital history; its ladder starts at 0, not %',
                NEW.subject, NEW.previous_fraction;
        END IF;
    ELSIF NEW.previous_fraction <> latest THEN
        RAISE EXCEPTION
            'strategy % is at %, not %; the ladder is continuous',
            NEW.subject, latest, NEW.previous_fraction;
    END IF;

    IF NEW.direction <> 'raise' THEN
        RETURN NEW;
    END IF;

    SELECT authorises_capital INTO authorises
    FROM mlops.gate_profile WHERE profile_id = NEW.profile_id;

    IF authorises IS NOT TRUE THEN
        RAISE EXCEPTION
            'profile % does not authorise capital; a pass under it is forward-test evidence, not permission',
            NEW.profile_id;
    END IF;

    prev_rung := array_position(rungs, NEW.previous_fraction);
    next_rung := array_position(rungs, NEW.allowed_fraction);
    IF next_rung <> prev_rung + 1 THEN
        RAISE EXCEPTION
            'a ramp climbs one rung at a time; % to % is not one step',
            NEW.previous_fraction, NEW.allowed_fraction;
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_capital_raise ON mlops.capital_authorisation;
CREATE TRIGGER trg_capital_raise BEFORE INSERT ON mlops.capital_authorisation
  FOR EACH ROW EXECUTE FUNCTION mlops.check_capital_raise();

-- What the risk gate (COMP-002) reads: the current rung per strategy.
CREATE OR REPLACE VIEW mlops.current_capital AS
SELECT DISTINCT ON (tenant_id, subject)
       tenant_id, subject, profile_id, allowed_fraction, direction, gates_passed,
       reason, decided_by, decided_at
FROM mlops.capital_authorisation
ORDER BY tenant_id, subject, decided_at DESC, authorisation_id DESC;

-- The agent may read what it is allowed to trade and may not write it. Raising
-- one's own allocation is not an operation the research agent has.
GRANT SELECT ON mlops.capital_authorisation TO agent_role;
GRANT SELECT ON mlops.current_capital TO agent_role;
GRANT SELECT, INSERT ON mlops.capital_authorisation TO app_role;
GRANT SELECT ON mlops.current_capital TO app_role;
