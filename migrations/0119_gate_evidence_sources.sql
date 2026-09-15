-- 0119 — the evidence gates 3, 4, 10 and 12 judge on (SPEC §12.3, HANDOFF §5.3).
--
-- Eleven of the sixteen gates judge on real numbers. Four of the remaining five
-- were dark for the same reason: the evaluators exist and are correct, and
-- *nothing writes what they read*. `mlops.trial_statistic` (0057) carries the
-- scalars — a Sharpe, a PBO, a breakeven multiple — but these four gates need a
-- *shape*: a ladder, a curve, a panel, a cross-section. A scalar cannot be
-- interpolated, and a shape cannot be squeezed into one.
--
-- Why tables rather than metric series: §16.1 forbids per-instrument results as
-- metric series outright (3 000 tickers × N runs blows the cardinality ceiling
-- and turns the tracking store into a deployment dependency). These are facts
-- about one trial, read once by one gate, and they belong beside the trial.
--
-- Why append-only and immutable like the rest of `mlops`: a gate verdict that
-- cannot be re-derived from what it was computed on is not evidence, it is an
-- assertion. Every row carries `produced_by` for the same reason.
--
-- The rule these tables exist to keep: **an absent row is not a zero.** Every
-- reader treats a missing row as "this gate cannot be evaluated", never as a
-- neutral value. An honest `inconclusive` is strictly better than a confident
-- wrong verdict, and a column that is always populated with a number that does
-- not mean what the gate thinks it means is the worst outcome available.

-- ── Gate 3: the cost ladder ───────────────────────────────────────────────────
--
-- The gate's question is not "is it profitable after costs" but "how wrong does
-- the cost model have to be before the edge disappears". That is a curve through
-- (multiple, net metric), and the breakeven is interpolated between the two
-- rungs that bracket zero. Storing only the interpolated scalar would leave the
-- gate's own arithmetic unre-derivable.
CREATE TABLE IF NOT EXISTS mlops.trial_cost_rung (
  rung_id     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id   TEXT NOT NULL,
  trial_id    UUID NOT NULL REFERENCES mlops.trial(trial_id),
  -- The multiple of *modelled* costs this rung applied. Strictly positive: a
  -- zero-cost rung is not a cost assumption.
  multiple    DOUBLE PRECISION NOT NULL CHECK (multiple > 0 AND multiple = multiple),
  -- The net metric earned at that multiple. May be negative — that is the point.
  net_metric  DOUBLE PRECISION NOT NULL CHECK (net_metric = net_metric),
  produced_by TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_trial_cost_rung_lookup
  ON mlops.trial_cost_rung (tenant_id, trial_id, recorded_at DESC);

-- ── Gate 4: the AUM ladder, and the header saying what it is a ladder of ──────
--
-- Split in two because `base_aum_usd` is one fact about the trial and the curve
-- is many, and a base AUM repeated on every point is a base AUM that can
-- disagree with itself.
CREATE TABLE IF NOT EXISTS mlops.trial_capacity (
  trial_id      UUID PRIMARY KEY REFERENCES mlops.trial(trial_id),
  tenant_id     TEXT NOT NULL,
  -- The AUM the headline result was earned at.
  base_aum_usd  DOUBLE PRECISION NOT NULL CHECK (base_aum_usd > 0),
  -- Single-venue ADV understates the real one, which makes Gate 4 conservative
  -- rather than wrong — but the verdict has to say so, so the fact is stored.
  single_venue  BOOLEAN NOT NULL,
  produced_by   TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS mlops.trial_capacity_point (
  point_id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id       TEXT NOT NULL,
  trial_id        UUID NOT NULL REFERENCES mlops.trial(trial_id),
  aum_usd         DOUBLE PRECISION NOT NULL CHECK (aum_usd > 0),
  net_sharpe      DOUBLE PRECISION NOT NULL CHECK (net_sharpe = net_sharpe),
  -- The counted trial that produced this rung. INV-16 has no exception for
  -- "runs we did to satisfy a gate": a capacity sweep that dodged the ledger
  -- would be exactly the kind of uncounted look this platform exists to make
  -- impossible, so every rung names the trial it cost.
  member_trial_id UUID NOT NULL REFERENCES mlops.trial(trial_id),
  produced_by     TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_trial_capacity_point_lookup
  ON mlops.trial_capacity_point (tenant_id, trial_id, aum_usd);

-- ── Gates 4 and 12: the per-instrument cross-section ──────────────────────────
--
-- Gate 12 asks whether one instrument carried the result; Gate 4 asks what share
-- of ADV the largest day took. Both read the same cross-section, so it is stored
-- once.
--
-- `numeraire` is not decoration. Summing a USDT P&L into a USD one produces a
-- concentration share that is arithmetic over two different units, and the
-- reader has no way to see it happened. Aggregation refuses to cross it.
--
-- `adv_usd` and `daily_vol` are NULLable *on purpose*: they are measured from
-- the run's own bars, over the run's own window, by whoever had the bars. A gate
-- measuring them at gate time would be reading today's liquidity against a 2023
-- backtest, which is a non-PIT read. Absent means Gate 4 cannot assess capacity
-- on that instrument, and Gate 4 says exactly that rather than guessing.
CREATE TABLE IF NOT EXISTS mlops.trial_instrument_pnl (
  row_id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id           TEXT NOT NULL,
  trial_id            UUID NOT NULL REFERENCES mlops.trial(trial_id),
  instrument          TEXT NOT NULL CHECK (length(instrument) > 0),
  numeraire           TEXT NOT NULL CHECK (length(numeraire) > 0),
  net_pnl             DOUBLE PRECISION NOT NULL CHECK (net_pnl = net_pnl),
  traded_notional     DOUBLE PRECISION NOT NULL CHECK (traded_notional >= 0),
  peak_daily_notional DOUBLE PRECISION NOT NULL CHECK (peak_daily_notional >= 0),
  adv_usd             DOUBLE PRECISION CHECK (adv_usd IS NULL OR adv_usd > 0),
  daily_vol           DOUBLE PRECISION CHECK (daily_vol IS NULL OR daily_vol >= 0),
  produced_by         TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_trial_instrument_pnl_lookup
  ON mlops.trial_instrument_pnl (tenant_id, trial_id, instrument);

-- ── Gate 12: the neighbourhood's shape ────────────────────────────────────────
--
-- A flat optimum generalizes and a spike does not, and that is a statement about
-- a sequence of (parameter value, metric) pairs in parameter order — not about
-- any one of them. The centre is marked rather than inferred from position: a
-- neighbourhood truncated by a budget no longer has its centre in the middle.
CREATE TABLE IF NOT EXISTS mlops.trial_neighbourhood_point (
  point_id    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id   TEXT NOT NULL,
  trial_id    UUID NOT NULL REFERENCES mlops.trial(trial_id),
  param       TEXT NOT NULL CHECK (length(param) > 0),
  param_value DOUBLE PRECISION NOT NULL CHECK (param_value = param_value),
  metric      DOUBLE PRECISION NOT NULL CHECK (metric = metric),
  is_center   BOOLEAN NOT NULL,
  produced_by TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_trial_neighbourhood_lookup
  ON mlops.trial_neighbourhood_point (tenant_id, trial_id, param, param_value);

-- ── Gate 10: the factor battery's return series ───────────────────────────────
--
-- One row per factor, carrying its own timestamps so the attribution regression
-- aligns period-for-period with the strategy's stored return series rather than
-- assuming the two were sampled the same way. A misaligned regression answers a
-- different question.
--
-- The battery itself is declared *per asset class on the profile*
-- (`mlops.gate_profile_asset_class`, ADR-P2-15); nothing here chooses factors.
-- Gate 10 refuses a battery that does not match the declared one, so a series
-- stored under a name the profile never declared changes no verdict.
CREATE TABLE IF NOT EXISTS mlops.trial_factor_return (
  row_id      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id   TEXT NOT NULL,
  trial_id    UUID NOT NULL REFERENCES mlops.trial(trial_id),
  factor      TEXT NOT NULL CHECK (length(factor) > 0),
  ts          TIMESTAMPTZ[] NOT NULL,
  returns     DOUBLE PRECISION[] NOT NULL,
  -- Where the series came from: which universe, over which window, from which
  -- bars. A factor series with no provenance is a column of numbers.
  produced_by TEXT NOT NULL CHECK (length(produced_by) > 0),
  recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- `cardinality`, not `array_length`: `array_length` is NULL on an empty array
  -- and a NULL CHECK passes, which would let an empty factor series through.
  CONSTRAINT chk_factor_return_shape
    CHECK (cardinality(ts) = cardinality(returns) AND cardinality(ts) > 0)
);
CREATE INDEX IF NOT EXISTS idx_trial_factor_return_lookup
  ON mlops.trial_factor_return (tenant_id, trial_id, factor, recorded_at DESC);

-- ── immutability, isolation, grants ───────────────────────────────────────────
--
-- The platform writes; the agent reads. Writing one of these is how a candidate
-- would hand a gate its own answer, which is §12.7's gate-hacking in its purest
-- form — so `agent_role` gets SELECT and nothing else, exactly as on
-- `mlops.trial_statistic`.
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY[
    'trial_cost_rung', 'trial_capacity', 'trial_capacity_point',
    'trial_instrument_pnl', 'trial_neighbourhood_point', 'trial_factor_return'
  ] LOOP
    EXECUTE format('DROP TRIGGER IF EXISTS trg_%s_immutable ON mlops.%I', t, t);
    EXECUTE format(
      'CREATE TRIGGER trg_%s_immutable BEFORE UPDATE OR DELETE ON mlops.%I
         FOR EACH ROW EXECUTE FUNCTION mlops.refuse_mutation()', t, t);
    EXECUTE format('ALTER TABLE mlops.%I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE mlops.%I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON mlops.%I', t);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON mlops.%I
         USING (tenant_id = current_setting(''app.tenant_id'', true))
         WITH CHECK (tenant_id = current_setting(''app.tenant_id'', true))', t);
    EXECUTE format('GRANT SELECT, INSERT ON mlops.%I TO app_role', t);
    EXECUTE format('GRANT SELECT ON mlops.%I TO agent_role, internal_ml_role, backtest_role', t);
  END LOOP;
END $$;
