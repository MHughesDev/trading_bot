-- §3 Datasets and features. Content-addressed. One code path. [INV-12/13/14/15]

CREATE TABLE feature_def (
  feature_id       TEXT PRIMARY KEY,          -- 'realized_vol_21d_v3'
  version          INT  NOT NULL,
  code_hash        BYTEA NOT NULL,
  lookback_bars    INT  NOT NULL,             -- ENFORCED by windowed view [INV-13]
  knowledge_lag_ms BIGINT NOT NULL,
  output_dtype     TEXT NOT NULL,
  asset_classes    TEXT[] NOT NULL,
  deflators        TEXT[],                    -- e.g. crypto seasonal profile
  info_class       TEXT NOT NULL CHECK (info_class IN
                     ('platform_physics','methodology','market_public',
                      'strategy_content','performance_conditional','tenant_operational'))
);                                            -- [INV-24] firewall input

CREATE TABLE label_spec (
  label_spec_id      TEXT PRIMARY KEY,
  kind               TEXT NOT NULL CHECK (kind IN
                       ('triple_barrier','horizon_return','meta_label','custom')),
  horizon_bars       INT NOT NULL,            -- feeds embargo [INV-15]
  pt_sl_multiples    DOUBLE PRECISION[],
  vol_estimator      TEXT,
  min_return_threshold DOUBLE PRECISION,
  sample_weight_method TEXT NOT NULL CHECK (sample_weight_method IN
                       ('uniqueness','return_attribution','time_decay','none')),
  code_hash          BYTEA NOT NULL
);

CREATE TABLE split_spec (
  split_spec_id    TEXT PRIMARY KEY,
  kind             TEXT NOT NULL CHECK (kind IN
                     ('walk_forward','purged_kfold','cpcv','holdout','sealed')),
  n_folds          INT,
  n_test_groups    INT,                       -- CPCV
  train_window     TEXT NOT NULL,             -- 'expanding' | 'rolling:N'
  embargo_bars     INT NOT NULL,              -- COMPUTED, never typed [INV-15]
  embargo_override_reason TEXT,               -- non-null ⇒ surfaced in every comparison
  purge_on         TEXT NOT NULL DEFAULT 't1' CHECK (purge_on IN ('t1','t0')),
  purge_override_reason   TEXT,
  min_train_bars   INT,
  regime_stratified BOOLEAN NOT NULL DEFAULT FALSE,
  CHECK (purge_on = 't1' OR purge_override_reason IS NOT NULL)
);

CREATE TABLE dataset_spec (
  dataset_id            TEXT PRIMARY KEY,     -- blake3 of the canonical spec [INV-12]
  tenant_id             BIGINT NOT NULL,
  universe_spec         JSONB NOT NULL,
  instrument_ids        BIGINT[] NOT NULL,    -- RESOLVED at spec time, stored explicitly
  date_from             TIMESTAMP(9) NOT NULL,
  date_to               TIMESTAMP(9) NOT NULL,
  frequency             TEXT NOT NULL DEFAULT '1m',
  feature_set_id        TEXT NOT NULL,
  label_spec_id         TEXT NOT NULL REFERENCES label_spec(label_spec_id),
  split_spec_id         TEXT NOT NULL REFERENCES split_spec(split_spec_id),
  as_of_knowledge_time  TIMESTAMP(9) NOT NULL,   -- THE pit anchor
  quality_exclusion_mask INT NOT NULL,
  calendar_version      TEXT NOT NULL,
  adjustment_policy     JSONB NOT NULL,
  finality_policy       JSONB,                -- DeFi only
  runtime_image_digest  TEXT NOT NULL,
  non_reproducible      BOOLEAN NOT NULL DEFAULT FALSE,  -- back-adjusted futures [INV-08]
  iceberg_snapshot_id   BIGINT NOT NULL,
  iceberg_tag           TEXT NOT NULL,        -- [S-1]
  created_at            TIMESTAMPTZ NOT NULL
);

-- §3.3 One code path. Log every live serve; diff nightly. [INV-14]
CREATE TABLE feature_serving_log (
  serve_id        UUID PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  instrument_id   BIGINT NOT NULL,
  event_time      TIMESTAMP(9) NOT NULL,
  knowledge_time  TIMESTAMP(9) NOT NULL,
  feature_set_id  TEXT NOT NULL,
  values          BYTEA NOT NULL,             -- packed vector
  served_at       TIMESTAMP(9) NOT NULL
);

CREATE TABLE feature_consistency_diff (
  serve_id                UUID NOT NULL,
  feature_id              TEXT NOT NULL,
  served_value            DOUBLE PRECISION,
  recomputed_value        DOUBLE PRECISION,
  abs_diff                DOUBLE PRECISION,
  served_knowledge_time     TIMESTAMP(9),     -- carrying BOTH is what makes this useful
  recomputed_knowledge_time TIMESTAMP(9),
  diagnosis TEXT CHECK (diagnosis IN
    ('late_arrival','code_drift','nondeterminism','precision','ok')),
  PRIMARY KEY (serve_id, feature_id)
);
-- SLO: p99 abs rel diff < 1e-9 for deterministic features. Any 'code_drift' is P1.
