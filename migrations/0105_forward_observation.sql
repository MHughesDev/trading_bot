-- 0105 — forward evidence: what a deployed strategy actually did (SPEC §12.3
-- Gate 15, §12.6; Set T T-0.1, T-0.3).
--
-- Of the platform's sixteen gates, Gate 15 is the only one that cannot be
-- satisfied by computing harder. Every other gate asks whether the evidence is
-- internally sound. Gate 15 asks whether the simulator is telling the truth,
-- and the only way to answer it is to have traded forward, in observed time,
-- and compared the fills against what the backtest said. These three tables are
-- where that record lives.
--
-- Three things are enforced here rather than in application code, because all
-- three failure modes are silent:
--
--   1. `venue` is a one-element CHECK. Set T may not create a live-venue code
--      path, and a constraint is the version of that sentence the database can
--      enforce after the sentence has been forgotten.
--   2. `source` separates observed rows from backfilled ones, and only observed
--      rows are forward evidence (D-T1, ADR-P2-24). A backfilled row is not
--      evidence about the future however recent its timestamp — the entire
--      content of the claim is that nobody knew the answer when the order went
--      out. The gate filters on this column; the column is NOT NULL and has no
--      default, so a writer has to say which it is.
--   3. Divergence thresholds and kill criteria are columns on the deployment,
--      NOT NULL, written when it is deployed. §12.6: a stopping rule chosen
--      after watching the result is not a stopping rule. Post-hoc kill
--      decisions always arrive late, because the drawdown that should trigger
--      them always comes with a plausible excuse.

-- ── The deployment ──────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS mlops.paper_deployment (
  deployment_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id         TEXT NOT NULL,
  -- The running strategy's id, as it appears on `OrderIntent.strategy_id`. This
  -- is the join between an order and its divergence halt.
  strategy_id       TEXT NOT NULL CHECK (length(strategy_id) > 0),
  -- The trial whose stored return series the realised series is compared
  -- against. NOT NULL: a forward record with no registered backtest to compare
  -- to is not Gate 15 evidence, it is a P&L statement.
  trial_id          UUID NOT NULL REFERENCES mlops.trial(trial_id),

  -- Paper only. ADR-0006 resolved the paper venue as Alpaca paper; roadmap gate
  -- H-4 (liability) stands between this table and anything else. Widening this
  -- CHECK is a migration, an ADR and a conversation — which is the point.
  venue             TEXT NOT NULL CHECK (venue IN ('alpaca_paper')),

  -- The unit both series are counted in. Without it, a USDT-denominated
  -- strategy's returns sum into a USD total and the arithmetic stays valid
  -- while the answer goes wrong by whatever the peg moved.
  numeraire         TEXT NOT NULL CHECK (numeraire IN ('usd','usdt','usdc','eur','btc','eth')),
  -- Bar alignment, in seconds. The realised and backtest series are joined on
  -- bar close; two series on different bar widths are not comparable and the
  -- divergence figure computed across them means nothing.
  bar_seconds       INT NOT NULL CHECK (bar_seconds > 0),

  -- ── Declared divergence thresholds (T-0.3) ────────────────────────────────
  -- Declared at deployment, recorded here, never edited: the table is
  -- append-only, so widening a threshold after a halt writes a new deployment
  -- row and leaves the old one legible.
  max_abs_return_divergence   DOUBLE PRECISION NOT NULL
                              CHECK (max_abs_return_divergence > 0),
  max_cum_return_divergence   DOUBLE PRECISION NOT NULL
                              CHECK (max_cum_return_divergence > 0),
  max_position_divergence     NUMERIC(38,18) NOT NULL
                              CHECK (max_position_divergence >= 0),
  min_signal_match            DOUBLE PRECISION NOT NULL
                              CHECK (min_signal_match > 0 AND min_signal_match <= 1),

  -- ── Pre-registered kill criteria (§12.6) ──────────────────────────────────
  -- Free text on purpose: SPRT/CUSUM parameters, MDD multiples, and whatever
  -- else the deployer commits to. What matters is that it is non-empty and
  -- written before the first fill, which the immutability trigger guarantees.
  kill_criteria     TEXT NOT NULL CHECK (length(trim(kill_criteria)) > 0),

  deployed_by       TEXT NOT NULL CHECK (length(deployed_by) > 0),
  deployed_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_paper_deployment_strategy
  ON mlops.paper_deployment (tenant_id, strategy_id, deployed_at DESC);

-- UPDATE is refused along with DELETE. A `retired_at` column that could be set
-- would be an UPDATE grant on this table, and the same grant would let somebody
-- widen `max_abs_return_divergence` after a halt fired — which is the one edit
-- that must leave a trace, because §12.6's whole point is that the stopping rule
-- predates the result. Retirement goes in its own table below.
DROP TRIGGER IF EXISTS trg_paper_deployment_immutable ON mlops.paper_deployment;
CREATE TRIGGER trg_paper_deployment_immutable
  BEFORE UPDATE OR DELETE ON mlops.paper_deployment
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.paper_deployment ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.paper_deployment FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.paper_deployment;
CREATE POLICY tenant_isolation ON mlops.paper_deployment
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON mlops.paper_deployment TO app_role;
GRANT SELECT ON mlops.paper_deployment TO agent_role, internal_ml_role, backtest_role;

-- A deployment that has stopped producing forward evidence. Its own table so
-- the deployment row stays immutable; a deployment with no row here is running.
CREATE TABLE IF NOT EXISTS mlops.paper_deployment_retirement (
  deployment_id UUID PRIMARY KEY REFERENCES mlops.paper_deployment(deployment_id),
  tenant_id     TEXT NOT NULL,
  retired_by    TEXT NOT NULL CHECK (length(trim(retired_by)) > 0),
  reason        TEXT NOT NULL CHECK (length(trim(reason)) > 0),
  retired_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS trg_paper_deployment_retirement_immutable
  ON mlops.paper_deployment_retirement;
CREATE TRIGGER trg_paper_deployment_retirement_immutable
  BEFORE UPDATE OR DELETE ON mlops.paper_deployment_retirement
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.paper_deployment_retirement ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.paper_deployment_retirement FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.paper_deployment_retirement;
CREATE POLICY tenant_isolation ON mlops.paper_deployment_retirement
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON mlops.paper_deployment_retirement TO app_role;
GRANT SELECT ON mlops.paper_deployment_retirement
  TO agent_role, internal_ml_role, backtest_role;

-- ── The observations ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS mlops.forward_observation (
  observation_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id         TEXT NOT NULL,
  deployment_id     UUID NOT NULL REFERENCES mlops.paper_deployment(deployment_id),
  -- Denormalised so an observation is legible, and queryable, without a join.
  strategy_id       TEXT NOT NULL,

  -- The alignment key: bar close. The realised series and the stored backtest
  -- series are joined on this and on nothing else.
  bar               TIMESTAMPTZ NOT NULL,

  realised_return   DOUBLE PRECISION NOT NULL CHECK (realised_return = realised_return),
  expected_return   DOUBLE PRECISION NOT NULL CHECK (expected_return = expected_return),
  numeraire         TEXT NOT NULL CHECK (numeraire IN ('usd','usdt','usdc','eur','btc','eth')),

  -- Observed, or reconstructed after the fact. No default: a writer must say.
  source            TEXT NOT NULL CHECK (source IN ('observed','backfill')),

  -- §12.6's four process measurements, per bar. Aggregated into Gate 15's
  -- verdict over the observation window.
  signal_matched            BOOLEAN NOT NULL,
  realised_slippage_bps     DOUBLE PRECISION NOT NULL
                            CHECK (realised_slippage_bps = realised_slippage_bps),
  modelled_slippage_bps     DOUBLE PRECISION NOT NULL
                            CHECK (modelled_slippage_bps = modelled_slippage_bps),
  realised_turnover         DOUBLE PRECISION NOT NULL CHECK (realised_turnover >= 0),
  expected_turnover         DOUBLE PRECISION NOT NULL CHECK (expected_turnover >= 0),
  model_attributable_rejects INT NOT NULL CHECK (model_attributable_rejects >= 0),

  observed_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

  -- One observation per deployment per bar per source. A second observed row
  -- for the same bar is a double-count, and a double-counted forward
  -- observation inflates exactly the number that is hardest to acquire.
  UNIQUE (deployment_id, bar, source)
);

CREATE INDEX IF NOT EXISTS idx_forward_observation_window
  ON mlops.forward_observation (tenant_id, deployment_id, source, bar);

DROP TRIGGER IF EXISTS trg_forward_observation_immutable ON mlops.forward_observation;
CREATE TRIGGER trg_forward_observation_immutable
  BEFORE UPDATE OR DELETE ON mlops.forward_observation
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.forward_observation ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.forward_observation FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.forward_observation;
CREATE POLICY tenant_isolation ON mlops.forward_observation
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

-- The platform writes; the agent reads. An agent that could write its own
-- forward observations could manufacture the one kind of evidence that exists
-- precisely because it cannot be manufactured.
GRANT SELECT, INSERT ON mlops.forward_observation TO app_role;
GRANT SELECT ON mlops.forward_observation TO agent_role, internal_ml_role, backtest_role;

-- ── The halts, and the human act of clearing one ────────────────────────────
--
-- Two append-only tables rather than one table with a `cleared_at` column: an
-- UPDATE-able halt is a halt that can be un-raised, and the whole value of this
-- record is that the raise survives whatever happened next. An active halt is a
-- raise row with no clearance.

CREATE TABLE IF NOT EXISTS mlops.divergence_halt (
  halt_id           UUID PRIMARY KEY,
  tenant_id         TEXT NOT NULL,
  strategy_id       TEXT NOT NULL CHECK (length(strategy_id) > 0),
  deployment_id     UUID REFERENCES mlops.paper_deployment(deployment_id),
  kind              TEXT NOT NULL CHECK (kind IN ('position','return','balance')),
  reason            TEXT NOT NULL CHECK (length(trim(reason)) > 0),
  raised_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_divergence_halt_strategy
  ON mlops.divergence_halt (tenant_id, strategy_id, raised_at DESC);

DROP TRIGGER IF EXISTS trg_divergence_halt_immutable ON mlops.divergence_halt;
CREATE TRIGGER trg_divergence_halt_immutable
  BEFORE UPDATE OR DELETE ON mlops.divergence_halt
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.divergence_halt ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.divergence_halt FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.divergence_halt;
CREATE POLICY tenant_isolation ON mlops.divergence_halt
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON mlops.divergence_halt TO app_role;
GRANT SELECT ON mlops.divergence_halt TO agent_role, internal_ml_role, backtest_role;

CREATE TABLE IF NOT EXISTS mlops.divergence_halt_clearance (
  -- One clearance per halt. The PK is the halt's own id, so a second clearance
  -- is a primary-key violation rather than a second row nobody notices.
  halt_id           UUID PRIMARY KEY REFERENCES mlops.divergence_halt(halt_id),
  tenant_id         TEXT NOT NULL,
  -- A person. Not a service account, not "system": resuming a strategy after a
  -- divergence is the one decision here that a machine must not make.
  cleared_by        TEXT NOT NULL CHECK (length(trim(cleared_by)) > 0),
  -- Why they believe it is safe to resume.
  note              TEXT NOT NULL CHECK (length(trim(note)) > 0),
  cleared_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

DROP TRIGGER IF EXISTS trg_divergence_halt_clearance_immutable ON mlops.divergence_halt_clearance;
CREATE TRIGGER trg_divergence_halt_clearance_immutable
  BEFORE UPDATE OR DELETE ON mlops.divergence_halt_clearance
  FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation();

ALTER TABLE mlops.divergence_halt_clearance ENABLE ROW LEVEL SECURITY;
ALTER TABLE mlops.divergence_halt_clearance FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON mlops.divergence_halt_clearance;
CREATE POLICY tenant_isolation ON mlops.divergence_halt_clearance
  USING (tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (tenant_id = current_setting('app.tenant_id', true));

GRANT SELECT, INSERT ON mlops.divergence_halt_clearance TO app_role;
GRANT SELECT ON mlops.divergence_halt_clearance TO agent_role, internal_ml_role, backtest_role;
