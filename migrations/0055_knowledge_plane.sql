-- 0055 — the knowledge plane (SPEC §5, checklist 3.3/3.5/3.7/3.10, ADR-P3-01…05).
--
-- Everything here is **recomputable from L0–L2**. No WORM, no retention
-- guarantee, no hash chain: this is what the platform has *learned*, and a
-- learned thing that cannot be recomputed from the facts is a claim nobody can
-- check. The ledger is the record; this is the index over it.
--
-- Three separations are structural, and each is a GRANT rather than a
-- convention:
--
--   1. **`regime_research` is not readable by anything a strategy runs as.**
--      Smoothed regime probabilities inflate Sharpe by roughly 2.2× because they
--      are the answer computed with hindsight. `regime_causal.regime_state`
--      holds `p_filtered` only, and that is the schema `backtest_role` can see
--      (AT-42 ⛔, R-07).
--   2. **`dr_estimate` never leaves the recommender.** A raw point estimate off
--      a sparse tensor is the number somebody will rank on; only the shrunk LCB
--      is granted out (AT-44 ⛔, R-06).
--   3. **Fingerprints are not features.** `asset_embedding` lives here and the
--      feature runtime cannot resolve a dimension of it by name (AT-68,
--      ADR-P3-01). That is a static test rather than a grant, because the
--      feature runtime is Rust, not SQL.
--
-- Vector columns need pgvector; the compose image and CI service are
-- `pgvector/pgvector:pg16` for exactly this migration.

CREATE EXTENSION IF NOT EXISTS vector;

CREATE SCHEMA IF NOT EXISTS knowledge;
CREATE SCHEMA IF NOT EXISTS regime_causal;
CREATE SCHEMA IF NOT EXISTS regime_research;

-- ── §5.3 asset embeddings ───────────────────────────────────────────────────
--
-- Venue is a first-class coordinate (R-10): BTC on two venues is two assets for
-- anything that trades them, and averaging them is how a funding-rate edge on
-- one becomes a claim about the other. `knowledge_time` is what stops
-- meta-leakage — a neighbour list computed from data the as-of date could not
-- have seen is a neighbour list from the future (AT-43).
CREATE TABLE IF NOT EXISTS knowledge.asset_embedding (
  instrument_id     BIGINT NOT NULL,
  venue_id          INT    NOT NULL,
  embedding_version TEXT   NOT NULL,
  window_spec       TEXT   NOT NULL,                   -- '21d@1m'
  valid_from        TIMESTAMPTZ NOT NULL,
  valid_to          TIMESTAMPTZ,
  knowledge_time    TIMESTAMPTZ NOT NULL,
  tier1             REAL[] NOT NULL,                   -- statistical fingerprint, incl. EDGE
  tier2             REAL[],                            -- reference-portfolio scores
  tier3             REAL[],                            -- learned encoder, gated adoption
  retrieval_vec     vector(48) NOT NULL,               -- whitened + reduced; the ONLY kNN target
  intrinsic_dim     REAL,
  hubness_corrected BOOLEAN NOT NULL DEFAULT TRUE,
  quality_flags     INT NOT NULL DEFAULT 0,
  info_class        TEXT NOT NULL DEFAULT 'market_public',
  PRIMARY KEY (instrument_id, venue_id, embedding_version, valid_from)
);
-- Deliberately NO ANN index (ADR-007, R-14). Exact kNN with metadata filters at
-- this scale is about a millisecond, and an approximate index's recall depends
-- on its own build state — a neighbour list that changes with the index is one
-- an attacker can probe. Add HNSW only on a measured p99 failure.
CREATE INDEX IF NOT EXISTS idx_asset_embedding_asof
  ON knowledge.asset_embedding (embedding_version, knowledge_time DESC);

CREATE TABLE IF NOT EXISTS knowledge.asset_cluster (
  cluster_id         INT NOT NULL,
  clustering_version TEXT NOT NULL,
  instrument_id      BIGINT NOT NULL,
  venue_id           INT NOT NULL,
  valid_from         TIMESTAMPTZ NOT NULL,
  valid_to           TIMESTAMPTZ,
  knowledge_time     TIMESTAMPTZ NOT NULL,
  stability_ari      REAL,
  PRIMARY KEY (clustering_version, instrument_id, venue_id, valid_from)
);
-- The clustering defines the outcome tensor's row space and nothing else. It is
-- not an allocator: "these assets cluster together" is a statement about their
-- statistics, not a recommendation to size them the same (R-04).

-- ── §5.4 regimes, split by what a strategy may see ──────────────────────────
CREATE TABLE IF NOT EXISTS regime_causal.regime_state (
  market_scope   TEXT NOT NULL,
  event_time     TIMESTAMPTZ NOT NULL,
  model_version  TEXT NOT NULL,                        -- the fit date (ADR-P3-02)
  -- Filtered probabilities: P(state | data up to t). The only regime state any
  -- strategy, feature or backtest may read.
  p_filtered     REAL[] NOT NULL,
  regime_vec     vector(16) NOT NULL,
  knowledge_time TIMESTAMPTZ NOT NULL,
  -- Labels are statistical, never narrative (ADR-P3-02): `high_vol`, not
  -- `risk_off`. A narrative label is a story the model did not learn.
  label          TEXT NOT NULL CHECK (label IN ('low_vol','mid_vol','high_vol')),
  PRIMARY KEY (market_scope, event_time, model_version)
);

CREATE TABLE IF NOT EXISTS regime_research.regime_state_smoothed (
  market_scope  TEXT NOT NULL,
  event_time    TIMESTAMPTZ NOT NULL,
  model_version TEXT NOT NULL,
  -- P(state | ALL data), including the future. Roughly 2.2× Sharpe inflation if
  -- a strategy ever sees it, which is why this table is in its own schema with
  -- its own grants rather than a column on the one above.
  p_smoothed    REAL[] NOT NULL,
  viterbi_path  INT,
  PRIMARY KEY (market_scope, event_time, model_version)
);

-- ── §5.5 the outcome tensor ─────────────────────────────────────────────────
--
-- The fact table is written from day one; the completion model is deferred until
-- there is something to fit it on (ADR-P3-03). A factorization of a few hundred
-- cells is noise with a version number.
CREATE TABLE IF NOT EXISTS knowledge.outcome_tensor_fact (
  tenant_id       TEXT NOT NULL,
  asset_cluster   INT NOT NULL,
  venue_id        INT NOT NULL,
  regime_id       INT NOT NULL,
  strategy_family TEXT NOT NULL,
  config_hash     TEXT NOT NULL,
  trial_id        UUID NOT NULL REFERENCES mlops.trial(trial_id),
  metric_name     TEXT NOT NULL,
  metric_value    DOUBLE PRECISION,
  -- MNAR correction input: what the platform's own policy did, not what it
  -- wishes it had done.
  propensity      DOUBLE PRECISION,
  -- Censored regression input. A stopped trial is an observation, and dropping
  -- it is how a search that works starts looking like one that does not (AT-45).
  censoring       TEXT NOT NULL,
  censor_at_step  INT,
  knowledge_time  TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (tenant_id, trial_id, metric_name)
);

CREATE TABLE IF NOT EXISTS knowledge.tensor_model (
  model_version      TEXT PRIMARY KEY,
  tenant_id          TEXT NOT NULL,
  rank               INT NOT NULL,
  -- The MNAR sensitivity coefficient, shipped as a health metric rather than
  -- hidden in a notebook (R-05): if `b1` is significantly non-zero the
  -- missingness is informative and the completion is not to be trusted.
  mnar_b1            DOUBLE PRECISION,
  mnar_b1_pvalue     DOUBLE PRECISION,
  trained_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  training_trial_ids UUID[] NOT NULL
);

CREATE TABLE IF NOT EXISTS knowledge.recommendation (
  rec_id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id            TEXT NOT NULL,
  context_hash         TEXT NOT NULL,
  candidate            JSONB NOT NULL,
  -- Internal only. Never granted out, never in an API type (AT-44).
  dr_estimate          DOUBLE PRECISION NOT NULL,
  lcb_score            DOUBLE PRECISION NOT NULL,
  shrink_weight        DOUBLE PRECISION NOT NULL CHECK (shrink_weight BETWEEN 0 AND 1),
  default_policy_value DOUBLE PRECISION NOT NULL,
  returned_score       DOUBLE PRECISION NOT NULL,
  model_version        TEXT REFERENCES knowledge.tensor_model(model_version),
  created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- What a caller may see. The point estimate is not in it, so a widened table
-- grant later cannot leak it by accident.
CREATE OR REPLACE VIEW knowledge.recommendation_public AS
SELECT rec_id, tenant_id, context_hash, candidate, lcb_score, shrink_weight,
       default_policy_value, returned_score, model_version, created_at
FROM knowledge.recommendation;

-- ── §5.6 agent memory ───────────────────────────────────────────────────────
--
-- One store, two spec names: this is the pack's `insight` table and AGENT-003's
-- durable memory (ADR-P3-04). Evidence is REQUIRED — a remembered claim with no
-- trial behind it is a rumour the platform will act on later.
CREATE TABLE IF NOT EXISTS knowledge.insight (
  insight_id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id          TEXT NOT NULL,
  tier               SMALLINT NOT NULL CHECK (tier IN (1,2,3)),
  scope              JSONB NOT NULL,
  claim              TEXT NOT NULL CHECK (length(claim) > 0),
  -- `cardinality`, not `array_length(x, 1)`. For an empty array `array_length`
  -- returns NULL, `NULL >= 1` is NULL, and a CHECK that evaluates to NULL
  -- **passes** — so the obvious spelling of "evidence is required" accepts an
  -- insight with no evidence at all. Caught by AT-42's live test, not by review.
  evidence_trial_ids UUID[] NOT NULL CHECK (cardinality(evidence_trial_ids) >= 1),
  support_n          INT NOT NULL DEFAULT 0 CHECK (support_n >= 0),
  contradicted_n     INT NOT NULL DEFAULT 0 CHECK (contradicted_n >= 0),
  -- Over the claim TEXT only, never over numbers (§5.6). An embedding of a
  -- metric is a similarity metric nobody designed.
  embedding          vector(768),
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_confirmed_at  TIMESTAMPTZ,
  decay_score        REAL NOT NULL DEFAULT 1.0 CHECK (decay_score BETWEEN 0 AND 1),
  info_class         TEXT NOT NULL DEFAULT 'derived'
);
CREATE INDEX IF NOT EXISTS idx_insight_scope
  ON knowledge.insight (tenant_id, tier, decay_score DESC);

-- ── tenancy ─────────────────────────────────────────────────────────────────
DO $$
DECLARE t TEXT;
BEGIN
  FOREACH t IN ARRAY ARRAY['outcome_tensor_fact','tensor_model','recommendation','insight'] LOOP
    EXECUTE format('ALTER TABLE knowledge.%I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE knowledge.%I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format('DROP POLICY IF EXISTS tenant_isolation ON knowledge.%I', t);
    EXECUTE format(
      'CREATE POLICY tenant_isolation ON knowledge.%I
         USING (tenant_id = current_setting(''app.tenant_id'', true))
         WITH CHECK (tenant_id = current_setting(''app.tenant_id'', true))', t);
  END LOOP;
END $$;

-- ── the grants that are the design ──────────────────────────────────────────
GRANT USAGE ON SCHEMA knowledge, regime_causal, regime_research TO app_role;
GRANT USAGE ON SCHEMA knowledge, regime_causal TO agent_role, internal_ml_role, backtest_role;

-- 1. Filtered regimes are readable by everything that runs a strategy.
GRANT SELECT ON regime_causal.regime_state
  TO app_role, agent_role, internal_ml_role, backtest_role;
GRANT INSERT ON regime_causal.regime_state TO app_role;

-- 2. Smoothed regimes are readable by research, and by nothing that evaluates.
--    `backtest_role` is deliberately absent, and so is `agent_role`: an agent
--    that can read the smoothed path can put it in a feature (AT-42 ⛔).
REVOKE ALL ON ALL TABLES IN SCHEMA regime_research FROM PUBLIC;
REVOKE USAGE ON SCHEMA regime_research FROM agent_role, internal_ml_role, backtest_role;
GRANT SELECT, INSERT ON regime_research.regime_state_smoothed TO app_role;

-- 3. Embeddings and clusters: readable, written by the platform.
GRANT SELECT ON knowledge.asset_embedding, knowledge.asset_cluster
  TO app_role, agent_role, internal_ml_role;
GRANT INSERT, UPDATE ON knowledge.asset_embedding, knowledge.asset_cluster TO app_role;

-- 4. The tensor and its model.
GRANT SELECT, INSERT ON knowledge.outcome_tensor_fact TO app_role;
GRANT SELECT ON knowledge.outcome_tensor_fact TO internal_ml_role;
GRANT SELECT, INSERT ON knowledge.tensor_model TO app_role, internal_ml_role;

-- 5. Recommendations: the internal model writes them and reads its own working;
--    everyone else sees the view, which has no point estimate in it (AT-44 ⛔).
GRANT SELECT, INSERT ON knowledge.recommendation TO internal_ml_role;
GRANT INSERT ON knowledge.recommendation TO app_role;
GRANT SELECT ON knowledge.recommendation_public TO app_role, agent_role;
REVOKE SELECT ON knowledge.recommendation FROM app_role, agent_role;

-- 6. Memory: the agent reads and writes its own findings, evidence enforced by
--    the CHECK above.
GRANT SELECT, INSERT, UPDATE ON knowledge.insight TO app_role, agent_role;
