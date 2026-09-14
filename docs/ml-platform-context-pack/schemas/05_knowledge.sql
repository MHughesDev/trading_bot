-- §5 Knowledge plane. Fully recomputable from L0-L2. No WORM, no retention guarantees.

CREATE EXTENSION IF NOT EXISTS vector;

-- §5.3 Venue is a first-class coordinate. knowledge_time prevents meta-leakage. [R-10]
CREATE TABLE asset_embedding (
  instrument_id     BIGINT NOT NULL,
  venue_id          INT    NOT NULL,
  embedding_version TEXT   NOT NULL,
  window_spec       TEXT   NOT NULL,          -- '21d@1m'
  valid_from        TIMESTAMPTZ NOT NULL,
  valid_to          TIMESTAMPTZ,
  knowledge_time    TIMESTAMPTZ NOT NULL,     -- [INV-01, AT-43]
  tier1             REAL[] NOT NULL,          -- statistical fingerprint (incl. EDGE)
  tier2             REAL[],                   -- reference-portfolio scores
  tier3             REAL[],                   -- learned encoder, gated adoption
  retrieval_vec     vector(48) NOT NULL,      -- whitened+reduced; the ONLY kNN target
  intrinsic_dim     REAL,
  hubness_corrected BOOLEAN NOT NULL DEFAULT TRUE,
  quality_flags     INT NOT NULL DEFAULT 0,
  PRIMARY KEY (instrument_id, venue_id, embedding_version, valid_from)
);
-- NO ANN INDEX. Exact kNN with metadata filters. [ADR-007, R-14]
-- Add HNSW only on measured p99 failure.

-- §5.4 Filtered only for anything a strategy can see. Enforced by GRANT. [R-07]
CREATE SCHEMA IF NOT EXISTS regime_causal;
CREATE SCHEMA IF NOT EXISTS regime_research;

CREATE TABLE regime_causal.regime_state (
  market_scope   TEXT NOT NULL,
  event_time     TIMESTAMP(9) NOT NULL,
  model_version  TEXT NOT NULL,
  p_filtered     REAL[] NOT NULL,             -- causal. the ONLY strategy-visible state.
  regime_vec     vector(16) NOT NULL,
  knowledge_time TIMESTAMP(9) NOT NULL,
  PRIMARY KEY (market_scope, event_time, model_version)
);

CREATE TABLE regime_research.regime_state_smoothed (
  market_scope  TEXT NOT NULL,
  event_time    TIMESTAMP(9) NOT NULL,
  model_version TEXT NOT NULL,
  p_smoothed    REAL[] NOT NULL,              -- 2.2x Sharpe inflation if a strategy sees it
  viterbi_path  INT,
  PRIMARY KEY (market_scope, event_time, model_version)
);

CREATE TABLE asset_cluster (
  cluster_id        INT NOT NULL,
  clustering_version TEXT NOT NULL,
  instrument_id     BIGINT NOT NULL,
  venue_id          INT NOT NULL,
  valid_from        TIMESTAMPTZ NOT NULL,
  valid_to          TIMESTAMPTZ,
  knowledge_time    TIMESTAMPTZ NOT NULL,
  stability_ari     REAL,                     -- cluster stability over time
  PRIMARY KEY (clustering_version, instrument_id, venue_id, valid_from)
);
-- Hierarchical clustering defines the tensor row space ONLY. Not an allocator. [R-04]

-- §5.5 Outcome tensor. Sparse fact table + factorization sidecar.
CREATE TABLE outcome_tensor_fact (
  tenant_id      BIGINT NOT NULL,
  asset_cluster  INT NOT NULL,
  venue_id       INT NOT NULL,
  regime_id      INT NOT NULL,
  strategy_family TEXT NOT NULL,
  config_hash    BYTEA NOT NULL,
  trial_id       UUID NOT NULL REFERENCES trial(trial_id),
  metric_name    TEXT NOT NULL,
  metric_value   DOUBLE PRECISION,
  propensity     DOUBLE PRECISION,            -- MNAR correction input
  censoring      TEXT NOT NULL,               -- censored regression input [AT-45]
  censor_at_step INT,
  knowledge_time TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (tenant_id, trial_id, metric_name)
);

CREATE TABLE tensor_model (
  model_version   TEXT PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  rank            INT NOT NULL,
  mnar_b1         DOUBLE PRECISION,           -- ship as a health metric [R-05]
  mnar_b1_pvalue  DOUBLE PRECISION,           -- H0: b1 = 0
  trained_at      TIMESTAMPTZ NOT NULL,
  training_trial_ids UUID[] NOT NULL
);

-- Recommendations are LCB-ranked and shrunk. No raw point estimates, ever. [R-06, AT-44]
CREATE TABLE recommendation (
  rec_id          UUID PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  context_hash    BYTEA NOT NULL,
  candidate       JSONB NOT NULL,
  dr_estimate     DOUBLE PRECISION NOT NULL,  -- internal only; never returned
  lcb_score       DOUBLE PRECISION NOT NULL,  -- what ranking uses
  shrink_weight   DOUBLE PRECISION NOT NULL,  -- significance-tested
  default_policy_value DOUBLE PRECISION NOT NULL,
  returned_score  DOUBLE PRECISION NOT NULL,  -- shrunk(lcb, default)
  model_version   TEXT NOT NULL REFERENCES tensor_model(model_version),
  created_at      TIMESTAMPTZ NOT NULL
);

-- §5.6 Agent memory. Evidence required. Embeddings over claim text ONLY.
CREATE TABLE insight (
  insight_id        UUID PRIMARY KEY,
  tenant_id         BIGINT NOT NULL,
  tier              SMALLINT NOT NULL CHECK (tier IN (1,2,3)),
  scope             JSONB NOT NULL,           -- asset_cluster/regime/family/venue
  claim             TEXT NOT NULL,
  evidence_trial_ids UUID[] NOT NULL CHECK (cardinality(evidence_trial_ids) >= 1),
  support_n         INT NOT NULL DEFAULT 0,
  contradicted_n    INT NOT NULL DEFAULT 0,
  embedding         vector(768),              -- claim TEXT only. never numbers. [§5.6]
  created_at        TIMESTAMPTZ NOT NULL,
  last_confirmed_at TIMESTAMPTZ,
  decay_score       REAL NOT NULL DEFAULT 1.0,
  info_class        TEXT NOT NULL             -- firewall [INV-24]
);
