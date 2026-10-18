-- 0214 — the regime strategy prior and the jump model's fits (Set R-b RB-4.2, RB-4.4;
-- FEAT-003 sections 9 and 10).
--
-- regime_strategy_prior is a contextual-bandit table, not a classifier: one row per
-- (instrument, regime, strategy family) holding what regime-sliced studies of that
-- family measured in that regime, so the first hypothesis of the next campaign can
-- cite which families did best where and how many observations each number rests on.
--
-- regime_jump_model records every fit of the statistical jump model: the penalty the
-- information criterion chose, the centroids and the standardisation, keyed by a
-- model version that is also the `model_version` of the labels it wrote to
-- regime_causal.regime_state. A label is reproducible from its fit and the stored
-- features, and a new fit is a new version, never an overwrite.
--
-- The labels themselves are written to regime_causal.regime_state (0055, 0117), the
-- table the rule tier already fills: this model replaces the internals, not the table.
--
-- Granted to app_role in this file.

CREATE SCHEMA IF NOT EXISTS regime_causal;

CREATE TABLE IF NOT EXISTS regime_causal.regime_strategy_prior (
  tenant_id       TEXT NOT NULL CHECK (btrim(tenant_id) <> ''),
  instrument      TEXT NOT NULL CHECK (btrim(instrument) <> ''),
  regime          TEXT NOT NULL CHECK (regime IN ('low_vol', 'mid_vol', 'high_vol')),
  strategy_family TEXT NOT NULL CHECK (btrim(strategy_family) <> ''),
  n_obs           INT NOT NULL DEFAULT 0 CHECK (n_obs >= 0),
  n_success       INT NOT NULL DEFAULT 0 CHECK (n_success >= 0),
  score_sum       DOUBLE PRECISION NOT NULL DEFAULT 0,
  score_sumsq     DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (score_sumsq >= 0),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (tenant_id, instrument, regime, strategy_family),
  CONSTRAINT successes_are_observations CHECK (n_success <= n_obs)
);

CREATE TABLE IF NOT EXISTS regime_causal.regime_jump_model (
  model_version TEXT PRIMARY KEY CHECK (btrim(model_version) <> ''),
  market_scope  TEXT NOT NULL CHECK (btrim(market_scope) <> ''),
  lambda        DOUBLE PRECISION NOT NULL CHECK (lambda > 0),
  n_train       INT NOT NULL CHECK (n_train > 0),
  -- The first observation the fit and every later labelling pass start from. The
  -- filter's state depends on where it starts, so reproducing a stored label means
  -- starting where the fit did.
  train_start   TIMESTAMPTZ NOT NULL,
  mean          DOUBLE PRECISION[] NOT NULL,
  sd            DOUBLE PRECISION[] NOT NULL,
  -- Row-major: k centroids of d standardised coordinates.
  centroids     JSONB NOT NULL CHECK (jsonb_typeof(centroids) = 'array'),
  criterion     JSONB NOT NULL CHECK (jsonb_typeof(criterion) = 'array'),
  fitted_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS regime_jump_model_scope ON regime_causal.regime_jump_model (market_scope, fitted_at DESC);

CREATE OR REPLACE FUNCTION regime_causal.refuse_model_mutation()
RETURNS TRIGGER AS $$
BEGIN
  RAISE EXCEPTION '% is append-only: a refit is a new model_version', TG_TABLE_NAME;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_regime_jump_model_append_only ON regime_causal.regime_jump_model;
CREATE TRIGGER trg_regime_jump_model_append_only
  BEFORE UPDATE OR DELETE ON regime_causal.regime_jump_model
  FOR EACH ROW EXECUTE FUNCTION regime_causal.refuse_model_mutation();

GRANT USAGE ON SCHEMA regime_causal TO app_role;
GRANT SELECT, INSERT, UPDATE ON regime_causal.regime_strategy_prior TO app_role;
GRANT SELECT, INSERT ON regime_causal.regime_jump_model TO app_role;
