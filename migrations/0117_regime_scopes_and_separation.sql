-- 0117 — the regime scope registry, and where the separation test's answer lives
-- (SPEC §5.4, §12.3 Gates 6 and 11, ADR-P3-07, ADR-P3-08, ADR-P3-09).
--
-- Three things, and the reason they are one migration is that the second and
-- third are only meaningful once the first exists.
--
--   1. `regime_causal.regime_scope` — the configuration ADR-P3-07 refused to
--      give a default. "The market" is a different series for crypto, equities
--      and futures, so the platform cannot pick one; but it *can* refuse to
--      accept a scope that does not say which series it is and why. The
--      rationale is a NOT NULL column with a length CHECK, so a scope arrives
--      with its reasoning or it does not arrive.
--
--   2. `regime_causal.regime_state.series_log_return` — the observable the
--      label was computed from, stored beside the label. Causal: the return of
--      the bar the tercile was computed through, known at that bar's close.
--      Gate 11 needs a *market* series (ADR-P2-29) and this is the one the
--      labeller already read; making the gate re-read ClickHouse to recover it
--      would be a second reader of the same number.
--
--   3. `regime_research.regime_separation` — the out-of-sample
--      regime-conditional separation test's result. It lives in the research
--      schema, which nothing that evaluates holds USAGE on, for a reason that
--      is not bureaucratic: the result is a full-sample statement about which
--      regime paid. A strategy or a gate that could read it would be reading a
--      summary of its own future (ADR-P3-09).

-- ── 1. the scope registry ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS regime_causal.regime_scope (
  -- The scope's name. It is NOT the symbol: the whole point of a scope is that
  -- it names a market, and the series is how that market is measured.
  market_scope           TEXT PRIMARY KEY,
  asset_class            TEXT NOT NULL,
  -- The series the regime is computed from.
  series_symbol          TEXT NOT NULL,
  -- NULL means the bar reader's own best-quality-venue rule. Naming a venue
  -- pins it; blending venues is never an option (SPEC §1.7).
  series_venue           TEXT,
  -- The stored timeframe that is read, and the bucket the returns are taken
  -- over. Both, because the platform stores minutes and the regime is daily,
  -- and a rollup through the PIT reader is not the same thing as a stored
  -- daily bar.
  series_base_period_secs INT NOT NULL,
  series_period_secs      INT NOT NULL,
  -- The instruments this scope may label. Empty means "every instrument of the
  -- asset class", which is a claim; listing the symbols is the honest default
  -- while the traded universe is small, because adding an instrument then
  -- forces a decision about whether this really is its market.
  applies_to             TEXT[] NOT NULL DEFAULT '{}',
  -- Why this series is the market for this scope. A column, not a comment:
  -- ADR-P3-07 made scope configuration a decision, and a decision with no
  -- recorded reason is a default with extra steps.
  rationale              TEXT NOT NULL,
  enabled                BOOLEAN NOT NULL DEFAULT TRUE,
  configured_by          TEXT NOT NULL,
  configured_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT chk_regime_scope_rationale CHECK (length(btrim(rationale)) >= 80),
  CONSTRAINT chk_regime_scope_periods
    CHECK (series_base_period_secs > 0
           AND series_period_secs >= series_base_period_secs
           AND series_period_secs % series_base_period_secs = 0)
);

CREATE INDEX IF NOT EXISTS idx_regime_scope_asset_class
  ON regime_causal.regime_scope (asset_class) WHERE enabled;

-- ── 2. the observable beside the label ──────────────────────────────────────
ALTER TABLE regime_causal.regime_state
  ADD COLUMN IF NOT EXISTS series_log_return DOUBLE PRECISION;

COMMENT ON COLUMN regime_causal.regime_state.series_log_return IS
  'The scope series log return for this bar — the observable the tercile was '
  'computed from. Causal: known at the bar close that stamps knowledge_time. '
  'Nullable because rows written before 0117 cannot have it reconstructed '
  'without re-reading, and a fabricated number is worse than a null.';

-- ── 3. the separation test's answer ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS regime_research.regime_separation (
  separation_id     UUID PRIMARY KEY,
  tenant_id         TEXT NOT NULL,
  market_scope      TEXT NOT NULL,
  -- Re-runs are keyed on this: a new regime model is a new question, and its
  -- answer never overwrites the old one's.
  model_version     TEXT NOT NULL,
  -- blake3 over the labels and the return series that went in. The job is a
  -- timer, so this is what stops it writing the same answer every day.
  input_digest      TEXT NOT NULL,
  computed_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- What went in.
  series_count      INT NOT NULL,
  observation_count INT NOT NULL,
  labelled_count    INT NOT NULL,
  regimes_present   INT NOT NULL,
  -- Per label: n, mean, sd, annualized Sharpe. JSONB because the label set is
  -- the model's, and a column per label would have to change with the model.
  per_regime        JSONB NOT NULL,
  -- best-minus-worst regime mean return, and the same in Sharpe terms.
  spread_mean       DOUBLE PRECISION,
  spread_sharpe     DOUBLE PRECISION,
  -- Kruskal–Wallis, reported but not decisive: it assumes independent
  -- observations and daily returns inside a volatility regime are not
  -- independent, so its p-value is anti-conservative here (ADR-P3-10).
  kruskal_h         DOUBLE PRECISION,
  kruskal_p         DOUBLE PRECISION,
  -- The decisive one: a circular block permutation of the label sequence,
  -- which preserves the autocorrelation of both the labels and the returns.
  permutation_p     DOUBLE PRECISION,
  block_length      INT,
  permutations      INT NOT NULL,
  -- Pre-registered before the first run and not tuned afterwards.
  alpha             DOUBLE PRECISION NOT NULL,
  -- NULL is a third state and it is not a pass: "not computable" is what a
  -- sample too short to answer the question deserves.
  separates         BOOLEAN,
  verdict           TEXT NOT NULL,
  CONSTRAINT uq_regime_separation_inputs
    UNIQUE (tenant_id, market_scope, model_version, input_digest)
);

CREATE INDEX IF NOT EXISTS idx_regime_separation_scope
  ON regime_research.regime_separation (market_scope, model_version, computed_at DESC);

-- ── the grants, which are the design ────────────────────────────────────────
--
-- The scope registry is readable by everything that reads a regime — knowing
-- which series is the market is not a signal. It is writable by `app_role`
-- alone: an agent that can add a scope can choose the market its own result is
-- measured against, which is gate-hacking with a configuration table
-- (SPEC §12.7).
GRANT SELECT ON regime_causal.regime_scope
  TO app_role, agent_role, internal_ml_role, backtest_role;
GRANT INSERT, UPDATE ON regime_causal.regime_scope TO app_role;
REVOKE INSERT, UPDATE, DELETE ON regime_causal.regime_scope
  FROM agent_role, internal_ml_role, backtest_role;

-- The separation result stays inside the research schema, whose USAGE was
-- revoked from every evaluating role in 0055 and is revoked again here so the
-- revocation is a property of this table's own migration rather than of an
-- earlier one somebody might reorder.
REVOKE ALL ON regime_research.regime_separation FROM PUBLIC;
REVOKE USAGE ON SCHEMA regime_research FROM agent_role, internal_ml_role, backtest_role;
GRANT SELECT, INSERT ON regime_research.regime_separation TO app_role;

-- ── the configuration itself (ADR-P3-08) ────────────────────────────────────
--
-- One scope, because one market is what this platform trades and has bars for.
-- Equities are deliberately absent: `instruments` holds AAPL/SPY/MSFT, and the
-- bar store holds nothing for any of them, so an equity scope would be a
-- configured market that produces no labels — and a scope that exists but never
-- labels is exactly the authoritative-looking nothing ADR-P3-07 refused.
INSERT INTO regime_causal.regime_scope
  (market_scope, asset_class, series_symbol, series_venue,
   series_base_period_secs, series_period_secs, applies_to,
   rationale, configured_by)
VALUES
  ('crypto_btc_usd', 'crypto', 'BTC-USD', NULL,
   60, 86400, ARRAY['BTC-USD','ETH-USD'],
   'BTC-USD is the dominant common factor in the crypto assets this platform '
   'trades, and it is the only instrument with enough stored history to take a '
   '21-day volatility window over. The series is read as minute bars rolled up '
   'to a day through the point-in-time reader, because the platform stores '
   'minutes and has no stored daily bar. ETH-USD is listed explicitly rather '
   'than the scope claiming all of crypto: when a third instrument is added, '
   'somebody has to decide whether BTC is its market.',
   'track-12/regime-scopes-and-separation')
ON CONFLICT (market_scope) DO NOTHING;
