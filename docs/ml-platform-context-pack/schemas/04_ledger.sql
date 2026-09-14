-- §4 The Trial Ledger. The asset. [INV-16..21]

-- §4.3 Outcome is a VECTOR. There is deliberately no scalar `score`. [ADR-012, INV]
CREATE TYPE outcome_vector AS (
  auc DOUBLE PRECISION, logloss DOUBLE PRECISION, brier DOUBLE PRECISION,
  ece DOUBLE PRECISION, ic DOUBLE PRECISION, ic_ir DOUBLE PRECISION,
  sharpe_net DOUBLE PRECISION, sortino_net DOUBLE PRECISION, calmar DOUBLE PRECISION,
  psr DOUBLE PRECISION, dsr DOUBLE PRECISION, pbo DOUBLE PRECISION,
  max_dd DOUBLE PRECISION, dd_duration_days INT, turnover_annual DOUBLE PRECISION,
  capacity_usd DOUBLE PRECISION, breakeven_cost_multiple DOUBLE PRECISION,
  seed_sharpe_std DOUBLE PRECISION, regime_pnl_hhi DOUBLE PRECISION,
  cpcv_p05_sharpe DOUBLE PRECISION, bootstrap_p05_sharpe DOUBLE PRECISION,
  param_cliff_score DOUBLE PRECISION,
  alpha_t_stat DOUBLE PRECISION, factor_r2 DOUBLE PRECISION,
  inference_latency_p99_ms DOUBLE PRECISION, model_bytes BIGINT,
  train_gpu_seconds DOUBLE PRECISION
);

CREATE TABLE campaign (
  campaign_id     UUID PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  objective       JSONB NOT NULL,             -- maximize[] + subject_to{}
  benchmark       JSONB NOT NULL,
  delta_practical DOUBLE PRECISION NOT NULL,  -- REQUIRED, no default [§10]
  budget          JSONB NOT NULL,             -- usd, gpu_hours, wall_clock, max_trials
  exploration_floor DOUBLE PRECISION NOT NULL DEFAULT 0.05
                    CHECK (exploration_floor >= 0.05),   -- [INV-21]
  gates_profile   TEXT NOT NULL,
  state           TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL
);

CREATE TABLE trial (
  trial_id        UUID PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  campaign_id     UUID NOT NULL REFERENCES campaign(campaign_id),
  parent_trial_id UUID REFERENCES trial(trial_id),
  seq             BIGINT NOT NULL,
  prev_hash       BYTEA NOT NULL,             -- [INV-19]
  row_hash        BYTEA NOT NULL,

  config_hash     BYTEA NOT NULL,
  config          JSONB NOT NULL,
  dataset_id      TEXT  NOT NULL REFERENCES dataset_spec(dataset_id),
  split_spec_id   TEXT  NOT NULL,
  code_hash       BYTEA NOT NULL,
  image_digest    TEXT  NOT NULL,
  seed_set        INT[] NOT NULL,

  actor_kind      TEXT NOT NULL CHECK (actor_kind IN ('human','agent','scheduler')),
  actor_id        TEXT NOT NULL,
  on_behalf_of    BIGINT,                     -- Temporal principal attribution
  policy_id       TEXT,
  policy_version  INT,
  candidate_set_hash BYTEA,
  propensity      DOUBLE PRECISION,           -- [INV-20]
  exploration_flag BOOLEAN NOT NULL DEFAULT FALSE,
  hypothesis_id   UUID,

  prereg_hash     BYTEA NOT NULL,
  delta_practical DOUBLE PRECISION NOT NULL,

  state           TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL,
  started_at      TIMESTAMPTZ,
  ended_at        TIMESTAMPTZ,
  terminal_reason TEXT,
  censoring       TEXT NOT NULL DEFAULT 'none' CHECK (censoring IN
                    ('none','right_asha','right_budget','right_preempt',
                     'right_cancel','failed')),                 -- [INV-17]
  censor_at_step  INT,
  planned_steps   INT,

  gpu_seconds DOUBLE PRECISION, cpu_seconds DOUBLE PRECISION,
  peak_vram_bytes BIGINT, usd_cost DECIMAL(18,6),

  outcome       outcome_vector,
  gate_results  JSONB,

  artifacts_uri TEXT, metrics_uri TEXT,
  predictions_uri TEXT,                       -- per-fold predictions [INV-18]
  returns_uri     TEXT,                       -- full OOS series      [INV-18]

  supersedes     UUID REFERENCES trial(trial_id),
  knowledge_time TIMESTAMPTZ NOT NULL,

  CHECK (propensity IS NOT NULL OR policy_id = 'legacy_unlogged'),
  CHECK (state <> 'REGISTERED' OR (config_hash IS NOT NULL AND prereg_hash IS NOT NULL))
);
CREATE UNIQUE INDEX ON trial (tenant_id, seq);
CREATE INDEX ON trial (tenant_id, config_hash);   -- dedup lookup [§9]
-- NO UPDATE GRANT. Corrections append with `supersedes`. [INV-19, AT-20]

-- §4.4 Not every decision produces a trial. Pruning and rejecting are training data.
CREATE TABLE decision (
  decision_id     UUID PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  campaign_id     UUID NOT NULL,
  decision_kind   TEXT NOT NULL CHECK (decision_kind IN
                    ('propose','select','prune','reallocate','stop','promote','reject')),
  actor_kind      TEXT NOT NULL,
  actor_id        TEXT NOT NULL,
  on_behalf_of    BIGINT,
  context_hash    BYTEA NOT NULL,
  context_uri     TEXT,
  candidate_set   JSONB NOT NULL,             -- ALL options considered
  chosen          JSONB NOT NULL,
  propensity      DOUBLE PRECISION NOT NULL,  -- [INV-20]
  policy_id       TEXT NOT NULL,
  policy_version  INT NOT NULL,
  exploration_flag BOOLEAN NOT NULL,
  decision_tier   TEXT NOT NULL CHECK (decision_tier IN
                    ('rule','shrunk','learned')),   -- cold-start ladder [§13.1]
  rationale       TEXT,
  decided_at      TIMESTAMPTZ NOT NULL,
  prev_hash BYTEA NOT NULL, row_hash BYTEA NOT NULL
);

-- §14.6 Log for fine-tuning now; it is impossible to retrofit.
CREATE TABLE agent_trajectory (
  traj_id          UUID NOT NULL,
  step_idx         INT  NOT NULL,
  tenant_id        BIGINT NOT NULL,
  campaign_id      UUID,
  tool_name        TEXT NOT NULL,
  tool_schema_hash BYTEA NOT NULL,            -- corpus half-life measurement
  tool_semver      TEXT NOT NULL,
  arguments        JSONB,
  result_summary   JSONB,
  error            JSONB,
  latency_ms       INT, tokens_in INT, tokens_out INT,
  critic_label     TEXT CHECK (critic_label IN
                     ('good','unnecessary','mistake','recover')),
  propensity       DOUBLE PRECISION,
  exploration_flag BOOLEAN,
  outcome_trial_id UUID,
  label_available_at TIMESTAMPTZ,
  PRIMARY KEY (traj_id, step_idx)
);

-- §4.6 Fixation
CREATE TABLE ledger_anchor (
  anchor_date DATE PRIMARY KEY,
  tenant_id   BIGINT NOT NULL,
  max_seq     BIGINT NOT NULL,
  chain_head  BYTEA NOT NULL,
  signature   BYTEA NOT NULL,
  worm_uri    TEXT NOT NULL
);

CREATE TABLE sealed_holdout_call (              -- rate limit, enforced by ledger [§12.7]
  strategy_lineage_id UUID PRIMARY KEY,
  trial_id   UUID NOT NULL,
  called_at  TIMESTAMPTZ NOT NULL,
  result     JSONB NOT NULL,
  n_attempts INT NOT NULL DEFAULT 1
);
