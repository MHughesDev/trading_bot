-- 0180 — the deployment path (Set T T-1.1, T-1.2, T-1.4, T-1.5).
--
-- Until this migration a strategy reached paper by anybody POSTing a graph to
-- /api/deployments. There was a record of the deployment (0142) and no record of
-- why it was allowed, who allowed it, or on what evidence. This is the "why".
--
--   deployment_proposal       what is being asked for: a strategy revision, the
--                             gate profile it passed, all sixteen verdicts, the
--                             divergence thresholds it will be held to, the paper
--                             capital and the review period. Created by anyone,
--                             including a research run (ring 0): asking costs a row.
--   deployment_decision       a person's yes or no, with the approval envelope.
--                             `decider_kind` admits one value.
--   deployment_authorisation  the join a deployment needs: it names the approved
--                             decision behind it, by composite foreign key.
--
-- Three rules live here rather than in the route, because the route is one
-- refactor away from not enforcing them:
--
--   1. a deployment exists only with an approved decision behind it
--      (`strategy_deployment_requires_authority`), once the switch below is armed;
--   2. `venue_class` has one value. A `paper_v1` pass authorises paper and nothing
--      else, and no row here can say otherwise (ADR-P2-14, D-T5);
--   3. a verdict about one revision cannot authorise another.
--
-- # Why a switch, and why it is one-way
--
-- Arming the proposal requirement before the deployments route asks for a
-- proposal would make every `POST /api/deployments` fail at the database. So the
-- trigger consults `deployment_gate_switch`, and a row in it is armed forever:
-- the table refuses UPDATE and DELETE, and `app_role` cannot insert (only the
-- owner connection can, from a migration or psql). It is a rollout step, not a
-- configuration key: nothing at runtime can turn it back off. See ADR-T1-01.
--
-- The same table holds the second switch, `legacy_unlogged_retired` (T-1.5),
-- which refuses any NEW trial under the `legacy_unlogged` policy once the Back
-- Testing UI no longer needs it. Existing rows are untouched: history is not
-- rewritten.

CREATE TABLE IF NOT EXISTS deployment_gate_switch (
  name      TEXT PRIMARY KEY
            CHECK (name IN ('deployments_require_a_proposal', 'legacy_unlogged_retired')),
  armed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  armed_by  TEXT NOT NULL CHECK (length(trim(armed_by)) > 0),
  note      TEXT NOT NULL CHECK (length(trim(note)) > 0)
);

DROP TRIGGER IF EXISTS trg_deployment_gate_switch_one_way ON deployment_gate_switch;
CREATE TRIGGER trg_deployment_gate_switch_one_way
  BEFORE UPDATE OR DELETE ON deployment_gate_switch
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── the proposal ────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS deployment_proposal (
  proposal_id           UUID PRIMARY KEY,
  -- Whose deployment it will be. The decider acts for them or is them.
  owner_user_id         TEXT NOT NULL CHECK (length(owner_user_id) > 0),
  proposed_by           TEXT NOT NULL CHECK (length(proposed_by) > 0),
  proposer_kind         TEXT NOT NULL CHECK (proposer_kind IN ('agent', 'human', 'scheduler')),
  strategy_name         TEXT NOT NULL CHECK (length(strategy_name) > 0),
  revision_hash         TEXT NOT NULL CHECK (revision_hash ~ '^[0-9a-f]{64}$'),
  -- One instrument, or `selection:<revision>` for a selection-bound deployment.
  instrument_id         TEXT NOT NULL CHECK (length(instrument_id) > 0),
  profile_id            TEXT NOT NULL REFERENCES mlops.gate_profile (profile_id),
  -- One value. Widening this CHECK is a migration, an ADR and roadmap gate H-4.
  venue_class           TEXT NOT NULL DEFAULT 'paper' CHECK (venue_class = 'paper'),
  -- Every one of the sixteen gates, each as {gate_no, passed, ...}. An array of
  -- fifteen is a verdict nobody finished.
  verdict               JSONB NOT NULL
                        CHECK (jsonb_typeof(verdict) = 'array' AND jsonb_array_length(verdict) = 16),
  -- The revision the verdict was computed for. A verdict is about one revision;
  -- it cannot be carried to another one (staleness, T-1.1).
  verdict_revision_hash TEXT NOT NULL,
  gates_passed          INT NOT NULL CHECK (gates_passed BETWEEN 0 AND 16),
  -- Declared before the first fill (SPEC §12.6): the detector's thresholds.
  thresholds            JSONB NOT NULL
                        CHECK (jsonb_typeof(thresholds) = 'object'
                               AND thresholds ?& ARRAY['max_abs_return_divergence',
                                                       'max_cum_return_divergence',
                                                       'max_position_divergence',
                                                       'min_signal_match']),
  starting_capital      NUMERIC(38,18) NOT NULL CHECK (starting_capital > 0),
  review_period_days    INT NOT NULL CHECK (review_period_days > 0),
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT chk_deployment_proposal_verdict_is_for_this_revision
    CHECK (verdict_revision_hash = revision_hash)
);

CREATE INDEX IF NOT EXISTS idx_deployment_proposal_owner
  ON deployment_proposal (owner_user_id, created_at DESC);

DROP TRIGGER IF EXISTS trg_deployment_proposal_immutable ON deployment_proposal;
CREATE TRIGGER trg_deployment_proposal_immutable
  BEFORE UPDATE OR DELETE ON deployment_proposal
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── the decision, and its envelope ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS deployment_decision (
  proposal_id   UUID PRIMARY KEY REFERENCES deployment_proposal (proposal_id),
  decision      TEXT NOT NULL CHECK (decision IN ('approved', 'rejected')),
  decided_by    TEXT NOT NULL CHECK (length(trim(decided_by)) > 0),
  -- One value. The type of a decider is not a column a writer chooses: a machine
  -- cannot approve a deployment (D-T2, ADR-0025).
  decider_kind  TEXT NOT NULL DEFAULT 'human' CHECK (decider_kind = 'human'),
  decided_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  note          TEXT NOT NULL DEFAULT '',
  -- Who decided, when, on what evidence, within what limits (migration 0054's
  -- shape): the proposal's verdict hash, the capital and period approved, and
  -- the thresholds the detector will use.
  envelope      JSONB NOT NULL
                CHECK (jsonb_typeof(envelope) = 'object'
                       AND envelope ?& ARRAY['decided_by', 'decided_at', 'evidence_hash',
                                             'starting_capital', 'review_period_days',
                                             'thresholds', 'profile_id']),
  UNIQUE (proposal_id, decision)
);

DROP TRIGGER IF EXISTS trg_deployment_decision_immutable ON deployment_decision;
CREATE TRIGGER trg_deployment_decision_immutable
  BEFORE UPDATE OR DELETE ON deployment_decision
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── the authorisation a deployment needs ────────────────────────────────────

CREATE TABLE IF NOT EXISTS deployment_authorisation (
  deployment_id UUID PRIMARY KEY,
  proposal_id   UUID NOT NULL UNIQUE,
  decision      TEXT NOT NULL DEFAULT 'approved' CHECK (decision = 'approved'),
  -- The composite key is the enforcement: only an APPROVED decision can be named.
  FOREIGN KEY (proposal_id, decision) REFERENCES deployment_decision (proposal_id, decision),
  venue_class   TEXT NOT NULL DEFAULT 'paper' CHECK (venue_class = 'paper'),
  authorised_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS trg_deployment_authorisation_immutable ON deployment_authorisation;
CREATE TRIGGER trg_deployment_authorisation_immutable
  BEFORE UPDATE OR DELETE ON deployment_authorisation
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- ── legacy_ungated (T-1.4, ADR-P2-27) ───────────────────────────────────────
--
-- A fact shown beside a deployment's numbers. It is never a permission and never
-- an input to a threshold: nothing that evaluates a strategy holds a grant on
-- this table (see the GRANTs below, and `legacy_marker_is_unreadable_by_gates`).

CREATE TABLE IF NOT EXISTS deployment_legacy_marker (
  deployment_id UUID PRIMARY KEY REFERENCES strategy_deployment (deployment_id),
  marker        TEXT NOT NULL DEFAULT 'legacy_ungated' CHECK (marker = 'legacy_ungated'),
  marked_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  basis         TEXT NOT NULL CHECK (length(trim(basis)) > 0)
);

DROP TRIGGER IF EXISTS trg_deployment_legacy_marker_immutable ON deployment_legacy_marker;
CREATE TRIGGER trg_deployment_legacy_marker_immutable
  BEFORE UPDATE OR DELETE ON deployment_legacy_marker
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

-- A deployment that has an authorisation is not legacy, and the marker would be
-- a lie about it.
CREATE OR REPLACE FUNCTION deployment_legacy_marker_only_unauthorised() RETURNS trigger AS $$
BEGIN
  IF EXISTS (SELECT 1 FROM deployment_authorisation WHERE deployment_id = NEW.deployment_id) THEN
    RAISE EXCEPTION 'deployment % has a decided proposal behind it; it is not legacy_ungated', NEW.deployment_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_deployment_legacy_marker_only_unauthorised ON deployment_legacy_marker;
CREATE TRIGGER trg_deployment_legacy_marker_only_unauthorised
  BEFORE INSERT ON deployment_legacy_marker
  FOR EACH ROW EXECUTE FUNCTION deployment_legacy_marker_only_unauthorised();

-- Everything running today. Backfilled once: a deployment started later without
-- a proposal is refused outright once the switch is armed, so it never needs one.
INSERT INTO deployment_legacy_marker (deployment_id, basis)
SELECT deployment_id,
       'running before the proposal path existed (migration 0180); ADR-P2-27: nobody is grandfathered, the marker is a fact and not a permission'
  FROM strategy_deployment
 WHERE stopped_at IS NULL
ON CONFLICT (deployment_id) DO NOTHING;

-- ── the door ────────────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION strategy_deployment_requires_authority() RETURNS trigger AS $$
DECLARE
  armed BOOLEAN;
  a     RECORD;
BEGIN
  SELECT EXISTS (SELECT 1 FROM deployment_gate_switch
                  WHERE name = 'deployments_require_a_proposal') INTO armed;
  IF NOT armed THEN
    RETURN NEW;
  END IF;

  -- A selection member runs under the authority its parent was given. The parent
  -- is a deployment like any other and had to clear this trigger itself, or be
  -- one of the legacy ones that predate it.
  IF NEW.parent_deployment_id IS NOT NULL THEN
    IF EXISTS (SELECT 1 FROM deployment_authorisation
                WHERE deployment_id = NEW.parent_deployment_id)
       OR EXISTS (SELECT 1 FROM deployment_legacy_marker
                   WHERE deployment_id = NEW.parent_deployment_id) THEN
      RETURN NEW;
    END IF;
    RAISE EXCEPTION 'strategy_deployment %: its parent % has no authority to inherit',
      NEW.deployment_id, NEW.parent_deployment_id;
  END IF;

  SELECT p.revision_hash, p.owner_user_id, p.starting_capital, p.instrument_id
    INTO a
    FROM deployment_authorisation au
    JOIN deployment_proposal p USING (proposal_id)
   WHERE au.deployment_id = NEW.deployment_id;
  IF NOT FOUND THEN
    RAISE EXCEPTION 'strategy_deployment %: no decided proposal is behind it. A deployment reaches paper through a proposal a person approved, and through no other door.', NEW.deployment_id;
  END IF;
  IF NEW.revision_hash <> a.revision_hash THEN
    RAISE EXCEPTION 'strategy_deployment %: the approved proposal is for revision %, not %',
      NEW.deployment_id, a.revision_hash, NEW.revision_hash;
  END IF;
  IF NEW.user_id <> a.owner_user_id THEN
    RAISE EXCEPTION 'strategy_deployment %: the approved proposal belongs to another user', NEW.deployment_id;
  END IF;
  IF NEW.starting_capital > a.starting_capital THEN
    RAISE EXCEPTION 'strategy_deployment %: starting capital % exceeds the approved %',
      NEW.deployment_id, NEW.starting_capital, a.starting_capital;
  END IF;
  IF NEW.target IS NULL AND NEW.instrument_id <> a.instrument_id THEN
    RAISE EXCEPTION 'strategy_deployment %: the approved proposal is for % not %',
      NEW.deployment_id, a.instrument_id, NEW.instrument_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_strategy_deployment_requires_authority ON strategy_deployment;
CREATE TRIGGER trg_strategy_deployment_requires_authority
  BEFORE INSERT ON strategy_deployment
  FOR EACH ROW EXECUTE FUNCTION strategy_deployment_requires_authority();

-- ── T-1.5: nothing new is registered as legacy_unlogged, once armed ─────────

CREATE OR REPLACE FUNCTION mlops_trial_refuses_new_legacy() RETURNS trigger AS $$
BEGIN
  IF NEW.policy_id = 'legacy_unlogged'
     AND EXISTS (SELECT 1 FROM deployment_gate_switch WHERE name = 'legacy_unlogged_retired') THEN
    RAISE EXCEPTION 'policy legacy_unlogged is retired (OQ-08 option A): a new trial registers under an Experiment or a campaign. Existing rows are history and stay.';
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_trial_refuses_new_legacy ON mlops.trial;
CREATE TRIGGER trg_trial_refuses_new_legacy
  BEFORE INSERT ON mlops.trial
  FOR EACH ROW EXECUTE FUNCTION mlops_trial_refuses_new_legacy();

-- ── grants ──────────────────────────────────────────────────────────────────
-- The platform's role reads and writes the path; it can only READ the switch, so
-- the platform cannot disarm or arm itself. The research agent reads proposals
-- and nothing else. Nothing that evaluates a strategy (agent_role,
-- backtest_role, internal_ml_role) holds a grant on the legacy marker.
GRANT SELECT, INSERT ON deployment_proposal      TO app_role;
GRANT SELECT, INSERT ON deployment_decision      TO app_role;
GRANT SELECT, INSERT ON deployment_authorisation TO app_role;
GRANT SELECT, INSERT ON deployment_legacy_marker TO app_role;
GRANT SELECT         ON deployment_gate_switch   TO app_role;
GRANT SELECT         ON deployment_proposal      TO agent_role;
