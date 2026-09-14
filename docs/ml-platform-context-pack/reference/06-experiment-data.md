# Experiment & Backtest Outcome Data: Structuring It So It Becomes Training Data

*Research note, September 2026. Scope: how to model, store, and "fixate" the outcomes of
training runs, hyperparameter sweeps, and backtests on a quant-ML platform so that the
resulting corpus is directly trainable — i.e. it becomes the supervised dataset for internal
models that steer the AI agents running the experiments.*

Companion notes: `01-mlops-infra.md` (tracking/lineage plumbing), `02-hpo-search.md` (search
algorithms that consume these surrogates), `03-financial-ml.md` (CPCV, PBO, DSR, embargo),
`04-agentic-ml.md` (agent loop that issues the decisions logged here).

---

## 0. Executive recommendation

**The one-line thesis.** Treat your experiment log as a *research dataset you are deliberately
collecting*, not as telemetry you happen to emit. The difference is almost entirely about five
things that production trackers (MLflow, W&B) do not give you by default:

1. **Store raw per-fold / per-bar prediction and P&L vectors, not just scalar metrics.** This is
   TabRepo's central insight, and it is worth more in quant than in AutoML. Scalars are a lossy
   projection; vectors let you recompute *any* metric, simulate *any* ensemble or portfolio
   overlay, and re-score history under a new cost model — for free, forever.
2. **Log the decision, the alternatives, and the propensity** at the moment an agent chooses what
   to run. Without logged propensities you can never do off-policy evaluation of a new search
   policy, and you will have permanently destroyed the counterfactual.
3. **Log the censoring.** Every killed, pre-empted, or gate-failed run is an observation with a
   *right-censored* label, not a missing row. Dropping them is the single largest source of bias
   in experiment databases.
4. **Fixate outcomes**: content-address the inputs, hash-chain the events, make the fact tables
   append-only and bitemporal. A result that can silently change is not training data.
5. **Make the task/context representation first-class.** A run is only meta-learnable if you
   stored *what problem it was solving* (asset universe, label definition, horizon, regime,
   dataset fingerprint, meta-features) in a structured, queryable way — not as a config blob.

**The recommended stack (2026):**

| Layer | Choice | Why |
|---|---|---|
| System of record (OLTP) | **PostgreSQL 17** | Runs, configs, artifacts, decisions, gates, lineage. Small row counts (10^5–10^7), needs transactions, FKs, and `SELECT ... FOR UPDATE`. |
| Append-only event spine | **Postgres `run_event` → NATS/Kafka → Iceberg** | Hash-chained, immutable. Source of every derived table. |
| Per-step metrics (hot) | **ClickHouse** (`MergeTree`, `ORDER BY (run_id, metric_id, step)`, `DoubleDelta`+`ZSTD`) | 10^9–10^10 rows, sub-second "compare 500 runs × 12 metrics". |
| Fact lake / archive | **Apache Iceberg v3 on S3** (Parquet, ZSTD) | Time travel, tags, branches (WAP), row lineage; engine-agnostic; cheap cold storage. |
| Analyst / training-set builder | **DuckDB** (local + `iceberg` extension), optionally **DuckLake** for the internal catalog | Zero-infra meta-learning feature engineering; reads the same Parquet. |
| Prediction / P&L vectors | **Parquet blobs in content-addressed object store**, indexed in Postgres/Iceberg | TabArena publishes 330 GB of predictions for 1,530 models × 211 datasets; your numbers will be similar and are worth every byte. |
| Meta-feature serving | **Feast-style offline/online split with as-of joins**, or roll your own on Iceberg + Redis | Point-in-time correctness at the *meta* level is the leakage trap nobody talks about. |

Do **not** make MLflow or W&B the system of record for meta-learning. Use them (if at all) as a
UI that reads from your tables. Their schemas are optimised for "show me this run", not for
"train a model over ten million runs".

---

## 1. Prior art: what the meta-learnable datasets store that trackers don't

### 1.1 The production trackers

**MLflow 3** models: `Experiment → Run → {Params, Metrics(step, timestamp), Tags, LoggedModel,
DatasetInput, Artifacts, Traces}`. MLflow 3 added `LoggedModel` with its own `model_id`, so
checkpoints within a run are addressable, and metrics can be attributed to a
(model, dataset) pair. Backends: filesystem, SQLAlchemy (Postgres/MySQL/SQLite), or managed.
Practical limits that bite: param values are stored as strings with a length cap, metric rows are
one-row-per-(key, step, timestamp) in a single `metrics` table, and the OSS backend has no
columnar storage — a few thousand runs × a few hundred thousand steps will make the tracking UI
unusable. `log_batch` helps ingest, not query.

**W&B** models: `Run` with `config` (immutable-ish dict), `summary` (last/best scalar per key),
`history` (the per-step time series), system metrics, plus `Artifact` with content-addressed,
deduplicated file entries and a lineage DAG. The `history` vs `summary` split is the right idea
and worth copying: **a dense time series plus a denormalised scalar rollup**, because 95% of
meta-learning queries hit the rollup and 5% need the curve.

**MLMD** (TFX ML Metadata) models everything as `Artifact` / `Execution` / `Context` with typed
properties and `Event` edges (INPUT/OUTPUT/DECLARED_INPUT/...). It is the cleanest *lineage*
abstraction of the four and its `Event` table is essentially an event-sourced spine. Its weakness
is that it is a lineage store with no opinion about metrics volume or experiment semantics, and
its momentum has faded post-TFX; treat it as a design reference rather than a dependency. If you
want the pattern without the dependency, it is three tables: `artifact`, `execution`,
`event(artifact_id, execution_id, type, path, ms_since_epoch)`.

### 1.2 OpenML: the best *public* entity model for meta-learning

OpenML's model is `Dataset → Task → (Flow, Setup) → Run → {Predictions, Trace, Evaluations}`.
The parts worth stealing:

- **`Task` is a separate entity from `Dataset`.** A task pins the target column, the estimation
  procedure (10-fold CV, repeats, holdout), and the evaluation measure. This means two runs are
  comparable *by construction*. This is the single most important schema decision on the list, and
  it maps directly onto quant: a task is (universe, label definition, horizon, CV scheme,
  embargo, cost model, evaluation window).
- **`Flow` vs `Setup`.** A *flow* is the algorithm/pipeline identity (name + external_version). A
  *setup* is a specific hyperparameter assignment of a flow — a deduplicated, hashed
  configuration. Two runs sharing a setup_id are exactly the same config. Deduplicating configs
  into a `setup` table is what makes "how does config X do across tasks?" a one-line join instead
  of a JSON-blob scan.
- **Predictions as a first-class ARFF file with a rigid column contract.** For supervised
  classification: `repeat, fold, row_id, prediction, confidence.<class>*`. The `row_id` anchors
  every prediction to a row of the frozen dataset, so *any* metric can be recomputed later and
  predictions from different runs can be aligned and ensembled.
- **Run trace** for HPO runs: `repeat, fold, iteration, evaluation, selected{True,False},
  parameter_<name>*`. Exactly one `selected=True` per (repeat, fold). This is the "inner loop"
  record that trackers throw away, and it is the training data for learning-to-rank and
  early-stopping models.

### 1.3 ASlib: the model for *failure* and *cost* accounting

ASlib (the Algorithm Selection Library format) is a small format with an outsized lesson. A
scenario is a directory of: `description.txt` (metadata, cutoffs, objective), `algorithm_runs.arff`,
`feature_values.arff`, `feature_costs.arff`, `feature_runstatus.arff`, `cv.arff`. The critical
fields:

- **`runstatus ∈ {ok, timeout, memout, not_applicable, crash, other}`** — recorded per
  (instance, algorithm). Failures are *labels*, not missing data.
- **`feature_costs`** — the cost of computing the meta-features themselves is recorded, because a
  selector that needs 60 s of features to save 30 s of solve time is a net loss.
- **`cv.arff`** — the cross-validation split over *instances* is frozen and shipped with the data,
  so every selector is evaluated on the same meta-level splits.

Copy all three. In your platform: `run_status ∈ {ok, oom, nan_divergence, timeout, preempted,
data_error, leakage_guard_trip, killed_by_scheduler, gate_fail, crash}`; `feature_cost_ms` for
every meta-feature; and a frozen, versioned meta-level split over *tasks* (not runs) so meta-models
are never evaluated on a task they saw.

### 1.4 The meta-learning datasets, and what makes them usable

| Dataset | Shape | Key design choice | Meta-learnable because |
|---|---|---|---|
| **TabRepo / TabArena** | 1,310 configs × 200 datasets × 3 folds (TabRepo); TabArena: 1,530 models × 211 datasets, 8-fold bagging, **330 GB of predictions**, ~7.75 M evaluations | Stores **raw validation and test prediction vectors** per (task, config), not only losses | Any ensemble (Caruana greedy) can be *simulated* post hoc; portfolio/zero-shot selection is a lookup, ~10,000× cheaper than retraining. Dense (all configs × all datasets) rather than sparse. |
| **YAHPO Gym** | 14 scenarios, >700 instances, ~20+ objectives | **Surrogate** (ResNet-style NN, ONNX-exported, multi-output) with **budget as an input dimension** | Continuous fidelity and continuous config space; no grid discretisation bias; multi-objective (perf + runtime + memory) in one model |
| **HPOBench** | 12 families, 100+ multi-fidelity problems | Ships **raw, tabular, and surrogate** variants of the same benchmark, each containerised | Lets you check whether a conclusion is an artefact of the surrogate |
| **LCBench** | 2,000 configs × 35 datasets, full per-epoch curves | Complete **learning curves** at every epoch, incl. train/val/test loss and accuracy | The only thing that makes learning-curve extrapolation trainable |
| **NAS-Bench-201** | 15,625 archs × 3 datasets, all trained | Complete tabular enumeration + per-epoch curves + multiple seeds | Exact ground truth; seeds let you separate noise from signal |
| **NAS-Bench-301** | 10^21-architecture space | **Surrogate** (GIN + LGBoost ensemble) with explicit **noise model** | Proves surrogates can beat tabular benchmarks in faithfulness for large spaces |
| **JAHS-Bench-201** | Joint architecture + HP + fidelity, XGBoost surrogates | Joint space, multi-fidelity, multi-objective incl. runtime | Realistic joint decisions |
| **PD1** (HyperBO) | Tens of thousands of configs across ~24 deep-learning workloads (image/text/protein) | Multi-task by construction; **learning curves + workload metadata** | Pre-training GP/transformer priors *across tasks*; gives ≥3× efficiency vs. cold-start BO |
| **LCDB** | 20 learners × 150+ datasets, anchors at powers of √2 (16, 24, 32, 45, …) | Learning curves over **training-set size** (not epochs) + full prediction vectors kept offline + precomputed meta-features | Sample-size extrapolation; anchor schedule is geometric so curves are comparable across datasets |

**Distilled: seven properties that separate a meta-learnable corpus from an archive.**

1. **Density.** All configs evaluated on all tasks (or a known, ignorable missingness mechanism).
   Sparse, opportunistically-collected logs give you selection bias you cannot undo.
2. **Alignment.** A frozen, addressable split (`row_id` / bar timestamp) so predictions from
   different runs can be stacked, compared, and ensembled.
3. **Raw outputs.** Prediction vectors / per-bar returns, not just scalars.
4. **Fidelity as a coordinate.** Epoch, training-set anchor, number of folds, universe size —
   stored as a column, so low-fidelity observations are usable rather than discarded.
5. **Cost and failure as labels.** Runtime, peak memory, and `runstatus` recorded for every cell,
   including failures and timeouts with their cutoffs.
6. **Replication.** ≥3 seeds on at least a subsample, so a meta-model can learn the noise floor
   and you can compute an irreducible-error bound for your surrogates.
7. **Meta-splits.** A versioned split over *tasks*, shipped with the data.

**What production trackers omit, concretely:** frozen task definitions; deduplicated config
identity (`setup_id`); raw prediction/P&L vectors; failure taxonomy; peak memory and cost;
propensity/decision context; the alternatives that were *not* chosen; censoring indicators; the
meta-level CV split; and the data snapshot fingerprint.

---

## 2. Core schema

Notation: PostgreSQL DDL for the system of record; ClickHouse for metrics; Iceberg/Parquet for
facts. All hashes are SHA-256 hex, truncated to 32 chars where used as identifiers.

### 2.1 Identity and fixation primitives

```sql
-- Content-addressed blob store index. The blob itself lives at s3://bucket/cas/<sha256[0:2]>/<sha256>.
CREATE TABLE artifact (
  content_hash    BYTEA PRIMARY KEY,            -- sha256 of bytes
  byte_size       BIGINT NOT NULL,
  media_type      TEXT   NOT NULL,              -- application/vnd.parquet, application/json, ...
  storage_uri     TEXT   NOT NULL,
  compression     TEXT,                         -- zstd-3, none
  first_seen_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  ref_count       INTEGER NOT NULL DEFAULT 0,
  legal_hold      BOOLEAN NOT NULL DEFAULT FALSE -- blocks GC for anything a published result cites
);

-- A frozen dataset snapshot: the *only* thing a run is allowed to train on.
CREATE TABLE data_snapshot (
  snapshot_id       TEXT PRIMARY KEY,           -- 'mkt.equities.bars1m@2026-08-31/v7'
  source_system     TEXT NOT NULL,
  as_of_knowledge   TIMESTAMPTZ NOT NULL,       -- system/knowledge time: when we knew this
  valid_from        TIMESTAMPTZ NOT NULL,       -- valid time: market data coverage start
  valid_to          TIMESTAMPTZ NOT NULL,
  iceberg_snapshot_id BIGINT,                   -- exact Iceberg snapshot, or
  delta_version     BIGINT,                     -- exact Delta version
  content_hash      BYTEA REFERENCES artifact,  -- manifest hash (Merkle root over file hashes)
  row_count         BIGINT,
  vendor_revision   TEXT,                       -- restatement/adjustment revision id
  UNIQUE (source_system, as_of_knowledge, vendor_revision)
);

-- Code + environment identity.
CREATE TABLE code_version (
  code_hash     BYTEA PRIMARY KEY,              -- Merkle root over tracked source tree
  git_commit    TEXT NOT NULL,
  git_dirty     BOOLEAN NOT NULL,
  image_digest  TEXT NOT NULL,                  -- sha256 of the OCI image actually used
  lockfile_hash BYTEA NOT NULL,                 -- uv.lock / poetry.lock / conda-lock
  cuda_version  TEXT, driver_version TEXT
);
```

### 2.2 Task: the comparability contract (steal from OpenML)

```sql
CREATE TABLE task (
  task_id           TEXT PRIMARY KEY,           -- hash of the canonical definition below
  task_family       TEXT NOT NULL,              -- 'xs_return_forecast' | 'vol_forecast' | 'exec_cost' | 'regime_clf'
  -- problem definition
  universe_id       TEXT NOT NULL,              -- 'sp500_liquid_top500'
  label_def_id      TEXT NOT NULL REFERENCES label_definition,
  horizon           INTERVAL NOT NULL,          -- '1 day', '5 day', '30 min'
  bar_frequency     TEXT NOT NULL,              -- '1m','5m','1d'
  -- evaluation protocol, FROZEN
  cv_scheme         JSONB NOT NULL,             -- {"type":"cpcv","n_groups":6,"k_test":2,"embargo_bars":390,"purge":true}
  cv_split_hash     BYTEA NOT NULL,             -- hash of the materialised fold boundaries
  eval_window       TSTZRANGE NOT NULL,         -- the OOS window this task scores on
  primary_metric    TEXT NOT NULL,              -- 'deflated_sharpe' | 'ic_mean' | 'pnl_after_cost'
  cost_model_id     TEXT NOT NULL REFERENCES cost_model,
  -- data binding
  data_snapshot_id  TEXT NOT NULL REFERENCES data_snapshot,
  -- meta
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  superseded_by     TEXT REFERENCES task(task_id)  -- tasks are immutable; you supersede, never edit
);

-- Precomputed, versioned meta-features describing the task. Cost is recorded (ASlib lesson).
CREATE TABLE task_meta_feature (
  task_id         TEXT NOT NULL REFERENCES task,
  feature_set_ver TEXT NOT NULL,                -- 'mf/v3'
  feature_name    TEXT NOT NULL,
  value           DOUBLE PRECISION,
  compute_ms      INTEGER NOT NULL,
  computed_as_of  TIMESTAMPTZ NOT NULL,         -- knowledge time: must be <= any decision that uses it
  PRIMARY KEY (task_id, feature_set_ver, feature_name)
);
```

Quant meta-features worth storing per task (these are the covariates every meta-model will use):
n_assets, n_bars, effective N after purging, label autocorrelation, label skew/kurtosis,
cross-sectional dispersion, realised-vol regime percentile, turnover of the universe, fraction
missing, average bid-ask spread, feature count, feature-block identity, Hurst exponent of the
target, and time-since-last-regime-break. Record `compute_ms` for each.

### 2.3 Flow, config (setup), and run

```sql
CREATE TABLE flow (                              -- algorithm/pipeline identity
  flow_id       TEXT PRIMARY KEY,                -- hash(name, version, param_schema_hash)
  name          TEXT NOT NULL,                   -- 'lgbm_xsec_ranker'
  version       TEXT NOT NULL,
  family        TEXT NOT NULL,                   -- 'gbdt'|'linear'|'seq_nn'|'transformer'|'rule'
  param_schema  JSONB NOT NULL,                  -- typed search space: bounds, log-scale, conditionals
  code_hash     BYTEA NOT NULL REFERENCES code_version
);

-- Deduplicated hyperparameter assignment. This is OpenML's 'setup'.
CREATE TABLE config (
  config_id     TEXT PRIMARY KEY,                -- hash(flow_id, canonical_json(params))
  flow_id       TEXT NOT NULL REFERENCES flow,
  params        JSONB NOT NULL,                  -- canonicalised: sorted keys, normalised numerics
  params_vec    REAL[],                          -- dense encoding under flow.param_schema (for surrogates)
  n_active_dims SMALLINT,                        -- conditionals resolved
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON config USING GIN (params jsonb_path_ops);

CREATE TABLE run (
  run_id            UUID PRIMARY KEY,
  task_id           TEXT NOT NULL REFERENCES task,
  config_id         TEXT NOT NULL REFERENCES config,
  sweep_id          UUID REFERENCES sweep,
  parent_run_id     UUID REFERENCES run(run_id),   -- for resumed/forked (freeze-thaw) runs
  -- fixation
  run_fingerprint   BYTEA NOT NULL,                -- hash(task_id, config_id, code_hash, data_snapshot_id, seed, hw_class)
  seed              BIGINT NOT NULL,
  determinism_class TEXT NOT NULL,                 -- 'bitwise'|'numeric_tolerant'|'nondeterministic'
  hw_class          TEXT NOT NULL,                 -- 'a100_80g_x1', 'cpu_m7i_8xl'
  -- fidelity coordinates (the multi-fidelity axes; NULL = full)
  fid_epochs        INTEGER,
  fid_train_frac    REAL,
  fid_n_folds       SMALLINT,
  fid_universe_frac REAL,
  fid_bar_subsample REAL,
  -- lifecycle
  status            TEXT NOT NULL,                 -- see run_status taxonomy
  submitted_at      TIMESTAMPTZ NOT NULL,
  started_at        TIMESTAMPTZ,
  ended_at          TIMESTAMPTZ,
  -- CENSORING: the fields that make killed runs usable
  is_censored       BOOLEAN NOT NULL DEFAULT FALSE,
  censor_reason     TEXT,                          -- 'asha_rung_cut'|'preempted'|'budget_exhausted'|'agent_kill'|'manual'
  censor_at_epoch   INTEGER,
  censor_at_seconds DOUBLE PRECISION,
  budget_cap_seconds DOUBLE PRECISION,             -- the cutoff that produced the censoring
  -- resource outcomes (labels for cost models)
  wall_seconds      DOUBLE PRECISION,
  cpu_seconds       DOUBLE PRECISION,
  gpu_seconds       DOUBLE PRECISION,
  peak_host_mem_bytes BIGINT,
  peak_gpu_mem_bytes  BIGINT,
  cost_usd          NUMERIC(12,4),
  -- outcome rollup (denormalised 'summary'; the curve lives in ClickHouse)
  primary_metric_value DOUBLE PRECISION,
  primary_metric_se    DOUBLE PRECISION,           -- across folds/seeds; essential for surrogates
  UNIQUE (run_fingerprint)                         -- dedupe: identical work is never repeated
);
CREATE INDEX ON run (task_id, status, ended_at DESC);
CREATE INDEX ON run (sweep_id, primary_metric_value DESC NULLS LAST);
```

**`run_status` taxonomy** (learn this as a multiclass label, ASlib-style):
`ok`, `ok_censored`, `timeout`, `oom_host`, `oom_gpu`, `nan_divergence`, `loss_explosion`,
`data_missing`, `schema_mismatch`, `leakage_guard_trip`, `constraint_infeasible`,
`preempted`, `killed_by_scheduler`, `killed_by_agent`, `crash_unknown`, `gate_fail`.

### 2.4 The two vector tables that make this a research dataset

This is the TabRepo insight, translated. Scalars go in `run`; **vectors go in Parquet, addressed
by content hash, indexed here.**

```sql
-- (a) Model outputs aligned to a frozen split. Analogue of OpenML predictions.arff / TabRepo preds.
CREATE TABLE prediction_block (
  run_id         UUID NOT NULL REFERENCES run,
  fold           SMALLINT NOT NULL,
  split          TEXT NOT NULL,                  -- 'val' | 'test' | 'oos'
  n_rows         BIGINT NOT NULL,
  row_index_hash BYTEA NOT NULL,                 -- hash of the (asset_id, bar_ts) index => alignment key
  dtype          TEXT NOT NULL,                  -- 'float16' is almost always enough
  content_hash   BYTEA NOT NULL REFERENCES artifact,
  PRIMARY KEY (run_id, fold, split)
);

-- (b) The quant analogue that matters MORE: per-bar positions and returns.
CREATE TABLE backtest_series (
  backtest_id    UUID PRIMARY KEY,
  run_id         UUID NOT NULL REFERENCES run,
  task_id        TEXT NOT NULL REFERENCES task,
  cost_model_id  TEXT NOT NULL REFERENCES cost_model,
  bar_index_hash BYTEA NOT NULL,                 -- alignment key across all backtests on this task
  n_bars         BIGINT NOT NULL,
  -- one Parquet file with columns: bar_ts, [asset_id], target_weight, executed_weight,
  -- gross_ret, net_ret, turnover, cost_bps, slippage_bps, borrow_bps, exposure_*, factor_betas...
  content_hash   BYTEA NOT NULL REFERENCES artifact,
  gross_bytes    BIGINT NOT NULL
);
```

**Why (b) is the highest-leverage table you will build.** Store the per-bar *position and return
vector* of every strategy candidate and you get, for free and forever:

- **Recompute any metric.** New Sharpe convention, new drawdown definition, Deflated Sharpe with a
  different trial count, PBO with a different combinatorial split — no re-run.
- **Simulate any portfolio.** Combining 40 candidate strategies under 10 weighting schemes is a
  matrix multiply over stored return vectors, not 400 backtests. This is exactly TabRepo's
  Caruana-ensemble simulation, and it is what makes strategy-portfolio meta-learning affordable.
- **Re-cost.** Change the cost model (spread assumptions, impact exponent, borrow) and re-score a
  decade of candidates in seconds — provided you stored `target_weight`/`turnover` separately from
  `net_ret`, which is why the column list above splits gross from net.
- **Correlation-aware selection.** The selector model needs the *correlation matrix between
  candidates*, which is only computable from return vectors. A leaderboard of Sharpe ratios
  cannot tell you that your top 10 strategies are the same trade.
- **Honest multiple-testing control.** DSR/PBO need the full set of trials' return series,
  including the losers.

Sizing: 10 years × 252 days × 500 assets × float16 ≈ 2.5 MB per candidate for daily weights;
minute bars × 500 assets ≈ 390 MB raw, ~40–80 MB ZSTD — store minute-level only for promoted
candidates, daily for everything. For 100k daily candidates that is ~250 GB. TabArena spends
330 GB on far less economically valuable predictions.

### 2.5 Per-step metrics (ClickHouse)

```sql
CREATE TABLE metric_point
(
  run_id        UUID,
  metric_id     LowCardinality(String),          -- 'val/ic', 'train/loss', 'gpu/mem_gb'
  step          UInt32,
  epoch         UInt16,
  ts            DateTime64(3) CODEC(DoubleDelta, ZSTD(1)),
  value         Float32       CODEC(Gorilla, ZSTD(1)),
  fold          UInt8 DEFAULT 0,
  -- denormalised dims so the common query needs no join
  task_id       LowCardinality(String),
  sweep_id      UUID,
  flow_family   LowCardinality(String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (run_id, metric_id, step)
TTL toDateTime(ts) + INTERVAL 90 DAY TO VOLUME 'warm',
    toDateTime(ts) + INTERVAL 365 DAY TO VOLUME 'cold',
    toDateTime(ts) + INTERVAL 400 DAY
      GROUP BY run_id, metric_id
      SET value = avg(value)                     -- downsample tail history
SETTINGS index_granularity = 8192,
         storage_policy = 'hot_warm_cold';

-- Cross-run metric comparison is a different access path: add a projection rather than a 2nd table.
ALTER TABLE metric_point ADD PROJECTION p_by_metric
  (SELECT * ORDER BY (task_id, metric_id, step, run_id));
```

Design notes, verified against ClickHouse guidance:
- Order compound keys by **ascending cardinality**; `run_id` first is correct only because the
  dominant query is "give me these runs' curves". The projection covers the transpose
  ("this metric across all runs in a task"), and ClickHouse picks the cheaper one automatically.
- `LowCardinality(String)` for `metric_id` turns a text column into a dictionary-encoded UInt8/16.
- `DoubleDelta` on monotone timestamps/steps and `Gorilla` on float metric values are the standard
  time-series codecs; expect **1–3 bytes per point** end-to-end.
- TTL `... TO VOLUME` gives you hot NVMe → warm → S3 tiering; TTL `... GROUP BY` gives you
  automatic rollup instead of deletion. (Note: tiered volumes are a self-managed-cluster feature.)

**Volume math.** 1 M runs × 500 logged steps × 12 metrics = 6 × 10^9 rows. At ~2 bytes/point
compressed that is ~12 GB on disk plus index — trivially a single ClickHouse node. The same data
in MLflow's Postgres `metrics` table (≈ 60–100 bytes/row before index) is ~500 GB and unqueryable.
This is the whole argument for splitting OLTP from OLAP.

### 2.6 Sweeps, gates, and the decision log

```sql
CREATE TABLE sweep (
  sweep_id       UUID PRIMARY KEY,
  task_id        TEXT NOT NULL REFERENCES task,
  search_policy  JSONB NOT NULL,       -- {"algo":"asha","eta":3,"r_min":1,"R":81,"sampler":"tpe","version":"2.4.1"}
  policy_hash    BYTEA NOT NULL,       -- identity of the behaviour policy, for OPE
  budget_usd     NUMERIC(12,2),
  objective      TEXT NOT NULL,
  n_trials_planned INTEGER,
  opened_at      TIMESTAMPTZ NOT NULL,
  closed_at      TIMESTAMPTZ
);

-- Promotion gates: the explicit, versioned decision boundaries a candidate must clear.
CREATE TABLE gate_definition (
  gate_id       TEXT PRIMARY KEY,      -- 'g2_oos_walkforward'
  gate_order    SMALLINT NOT NULL,     -- 1=smoke, 2=cheap OOS, 3=CPCV, 4=full cost/capacity, 5=paper
  expr          TEXT NOT NULL,         -- 'dsr >= 0.95 AND pbo <= 0.30 AND max_dd <= 0.12 AND capacity_usd >= 5e6'
  expr_hash     BYTEA NOT NULL,
  expected_cost_usd NUMERIC(12,2),     -- cost of EVALUATING the gate -- needed for value-of-information
  valid_from    TIMESTAMPTZ NOT NULL,  -- gates change; bitemporal so old decisions stay explicable
  valid_to      TIMESTAMPTZ
);

CREATE TABLE gate_evaluation (
  gate_eval_id  UUID PRIMARY KEY,
  run_id        UUID NOT NULL REFERENCES run,
  gate_id       TEXT NOT NULL REFERENCES gate_definition,
  expr_hash     BYTEA NOT NULL,        -- which VERSION of the gate was applied
  passed        BOOLEAN NOT NULL,
  margin        DOUBLE PRECISION,      -- signed distance to the boundary: a far better label than the bit
  metrics       JSONB NOT NULL,        -- every quantity the expression referenced
  evaluated_at  TIMESTAMPTZ NOT NULL,
  eval_cost_usd NUMERIC(12,4)
);
```

**The decision log is the table that makes off-policy evaluation possible.** Write it at the
moment of choice, not after the outcome:

```sql
CREATE TABLE decision_log (
  decision_id     UUID PRIMARY KEY,
  decided_at      TIMESTAMPTZ NOT NULL,
  agent_id        TEXT NOT NULL,
  policy_id       TEXT NOT NULL,          -- behaviour policy identity
  policy_version  TEXT NOT NULL,
  decision_type   TEXT NOT NULL,          -- 'propose_config'|'continue_or_kill'|'promote_to_gate'|'allocate_budget'
  -- CONTEXT x: everything the policy saw, as of decided_at. Stored by reference for reproducibility.
  context_hash    BYTEA NOT NULL REFERENCES artifact,
  context_summary JSONB NOT NULL,         -- small, queryable subset (task_id, budget_left, n_runs_so_far, ...)
  -- ACTION a and the CANDIDATE SET it was drawn from
  action_ref      JSONB NOT NULL,         -- {"config_id":"...","fidelity":{"epochs":9}}
  candidate_set_hash BYTEA REFERENCES artifact,  -- the full slate considered  <-- do not skip this
  n_candidates    INTEGER NOT NULL,
  -- PROPENSITY  <-- the field that cannot be reconstructed later
  propensity      DOUBLE PRECISION NOT NULL CHECK (propensity > 0),
  propensity_kind TEXT NOT NULL,          -- 'exact'|'softmax_temp'|'epsilon_greedy'|'estimated'
  exploration_flag BOOLEAN NOT NULL,      -- was this a forced-exploration draw?
  -- OUTCOME, filled in later; NULL until known
  run_id          UUID REFERENCES run,
  reward          DOUBLE PRECISION,
  reward_def_id   TEXT REFERENCES reward_definition,
  reward_observed_at TIMESTAMPTZ
);
CREATE INDEX ON decision_log (policy_id, decided_at);
```

If you take one thing from section 2: **`propensity`, `candidate_set_hash`, and
`exploration_flag` are unrecoverable after the fact.** Every day you run agents without them is a
day of data that can support correlational analysis but not policy evaluation.

---

## 3. Fixation: event sourcing, content addressing, and bitemporality

### 3.1 Event-sourced vs snapshot: use both, but the event log is primary

The snapshot tables above (`run`, `gate_evaluation`, …) are **projections**. The source of truth
is an append-only, hash-chained event log. This is not architectural purity — it buys four
concrete things: (i) you can rebuild any derived table when you discover a bug in a metric
definition, (ii) you get the *timeline* of when each fact became known, which is exactly the
knowledge-time axis meta-models need, (iii) late-arriving corrections are new events rather than
destructive updates, and (iv) tamper-evidence.

```sql
CREATE TABLE run_event (
  event_id      BIGSERIAL PRIMARY KEY,
  run_id        UUID NOT NULL,
  seq           INTEGER NOT NULL,            -- per-run monotone
  event_type    TEXT NOT NULL,               -- 'submitted','started','epoch_end','checkpoint','metric_batch',
                                             -- 'rung_promoted','killed','failed','completed','gate_evaluated',
                                             -- 'metric_recomputed','result_retracted'
  occurred_at   TIMESTAMPTZ NOT NULL,        -- VALID time  (when it happened in the world)
  recorded_at   TIMESTAMPTZ NOT NULL DEFAULT now(),  -- SYSTEM/KNOWLEDGE time (when we learned it)
  payload       JSONB NOT NULL,
  payload_hash  BYTEA NOT NULL,
  prev_hash     BYTEA NOT NULL,              -- hash-chain over (prev_hash || payload_hash || seq)
  chain_hash    BYTEA NOT NULL,
  producer      TEXT NOT NULL,               -- worker id / agent id
  UNIQUE (run_id, seq)
);
```

Per-run chain hashes are periodically folded into a **Merkle root per (day, task)** and that root
is written into an Iceberg snapshot summary and (optionally) an external timestamping service.
Cost: negligible. Benefit: you can prove that the backtest number you showed the risk committee in
March is the number the system computed in March.

**Retraction, never mutation.** If a bug invalidates results, emit `result_retracted` events with
a reason and a replacement pointer. Never `UPDATE`. Meta-models then learn from a corpus where
`is_retracted` is a filterable column, and historical decisions remain explicable.

### 3.2 Bitemporality and slowly-changing dimensions

Three clocks matter and conflating them is the classic quant-platform bug:

| Clock | Meaning | Column |
|---|---|---|
| **Event/market time** | When the market bar occurred | `bar_ts` |
| **Valid time** | The period a fact is true of the world | `valid_from`, `valid_to` |
| **Knowledge/system time** | When *we* learned it | `recorded_at`, `as_of_knowledge` |

Financial data is restated: fundamentals get revised, corporate actions backfill, vendors reissue
histories. A run executed in 2025 on the then-current data is *not* reproducible against today's
snapshot, and that discrepancy is itself a signal (your leakage detector should flag candidates
whose edge appears only under restated data). So:

- Dimension tables (`task`, `gate_definition`, `cost_model`, `reward_definition`, `universe`) are
  **SCD Type 2**: `(entity_id, version, valid_from, valid_to, is_current, change_reason)`, never
  updated in place. Every fact row stores the *version* it used, not just the id.
- Fact tables carry both `occurred_at` and `recorded_at`. All meta-learning feature queries filter
  `recorded_at <= decision_time`. This is the meta-level point-in-time rule (§7).
- Data snapshots are content-addressed by a Merkle manifest so "the same snapshot id" is provable,
  not conventional.

```sql
-- SCD2 pattern used for every dimension
CREATE TABLE cost_model (
  cost_model_id TEXT NOT NULL,
  version       INTEGER NOT NULL,
  spec          JSONB NOT NULL,       -- {"spread_model":"...","impact":{"kind":"sqrt","coef":0.3},"borrow_bps":...}
  spec_hash     BYTEA NOT NULL,
  valid_from    TIMESTAMPTZ NOT NULL,
  valid_to      TIMESTAMPTZ NOT NULL DEFAULT 'infinity',
  is_current    BOOLEAN GENERATED ALWAYS AS (valid_to = 'infinity') STORED,
  change_reason TEXT,
  PRIMARY KEY (cost_model_id, version),
  EXCLUDE USING gist (cost_model_id WITH =, tstzrange(valid_from, valid_to) WITH &&)
);
```

### 3.3 Table-format time travel: Iceberg vs Delta vs DuckLake

- **Iceberg v3** is the default recommendation for the fact lake. Relevant v3 additions over v2:
  **row lineage** (`_row_id`, `_last_updated_sequence_number`) so row-level provenance survives
  compaction; **deletion vectors** (single binary vector per data file, replacing positional delete
  files) making corrections cheap; **variant** type for the semi-structured config/metrics blobs;
  plus v2-era **branches and tags** which give you Write-Audit-Publish: land new experiment facts
  on a branch, run consistency audits (no duplicate `run_fingerprint`, no negative costs, all
  `prediction_block` hashes resolvable), then fast-forward to `main`. Tag the state used for every
  published research result (`tag: research/alpha-42/2026-09-01`) — tags pin snapshots against
  expiration.
- **Delta Lake** time travel (`VERSION AS OF` / `TIMESTAMP AS OF`) is equivalent in spirit, but
  note the retention trap: default `delta.logRetentionDuration` = 30 days and
  `deletedFileRetentionDuration` = 7 days, and `VACUUM` plus log cleanup will silently make old
  versions unreadable. If you use Delta as your fixation layer you **must** raise both to your
  compliance horizon (e.g. 7 years) or, better, materialise fixed results into the CAS rather than
  relying on table history.
- **DuckLake** (released 2025; catalog-in-a-SQL-database rather than metadata-files-in-blob-store)
  is genuinely interesting for this workload because experiment ingestion is *high-frequency,
  small-write*: a snapshot is a few rows in Postgres, it supports thousands of transactions/second,
  and it can inline small changes into the catalog instead of writing small Parquet files. If your
  agents commit results every few seconds, DuckLake removes the small-file problem that Iceberg
  compaction otherwise creates. Reasonable 2026 posture: DuckLake for the hot internal catalog,
  Iceberg for the long-term, externally-readable archive; both over the same Parquet.

### 3.4 Parquet layout for the fact lake

```
s3://quant-exp/facts/
  runs/                 dt=2026-09-12/task_family=xs_return_forecast/part-*.parquet
  gate_evaluations/     dt=.../gate_order=3/
  decisions/            dt=.../decision_type=propose_config/
  metric_rollups/       task_id=<hash>/metric_id=val_ic/
  predictions/          cas/<sha256[0:2]>/<sha256>.parquet     <-- content-addressed, never partitioned by time
  backtest_series/      cas/<sha256[0:2]>/<sha256>.parquet
```

- Target **256–512 MB files**, **128 MB row groups**, page size 1 MB, ZSTD-3 (ZSTD-9 for cold).
- **Sort within file** by the dominant predicate (`task_id, config_id` for runs; `run_id, step` for
  metrics) so min/max column statistics actually prune. Unsorted Parquet defeats predicate
  pushdown and is the most common reason "our lake is slow".
- Enable **column-level bloom filters** on `run_id`, `config_id`, `task_id`.
- Prediction/backtest blobs are **content-addressed and immutable** — never partitioned by date,
  never rewritten, deduplicated automatically (two runs producing byte-identical predictions share
  one object, which happens more than you would think and is itself a useful duplicate detector).
- Hive-partition facts by `dt` + one low-cardinality dimension; avoid partitioning by `run_id`
  (millions of tiny partitions is the canonical failure).

### 3.5 Storage tiering

| Tier | Contents | Medium | Retention |
|---|---|---|---|
| Hot | last 30 d metric points, open runs, all `run`/`decision_log` rows | NVMe (ClickHouse) + Postgres | 30–90 d full resolution |
| Warm | 90 d–2 y metrics at full resolution, all fact Parquet | S3 Standard | 2 y |
| Cold | downsampled metrics (per-epoch mean/min/max/last), archived predictions | S3 Infrequent Access / Glacier IR | 7 y |
| Frozen | anything cited by a promoted strategy or published result | S3 with Object Lock + `legal_hold` | indefinite |

Downsampling rule: keep **full resolution for any run that was ever promoted past gate 2, or that
is in a meta-model's training set**; downsample everything else after 90 days to (per-epoch
last, min, max, mean, count). Learning-curve models need per-epoch, not per-step, so this loses
almost nothing. Never downsample `prediction_block` or `backtest_series` — they are the crown
jewels and they are small relative to the metric stream.

---

## 4. Analytical store choices, 2026

### 4.1 The split

**OLTP system of record: PostgreSQL.** Everything in §2.1–2.4 and §2.6 except metric points.
Row counts are modest (10^6 runs, 10^7 decisions, 10^7 gate evaluations), and you need
transactions, foreign keys, uniqueness on `run_fingerprint`, advisory locks for the scheduler, and
`LISTEN/NOTIFY` for the agent loop. Postgres 17 with `pg_partman` on the event tables handles this
to ~10^8 rows comfortably. Add `pg_cron` for projection refresh.

**OLAP analysis store: ClickHouse for metrics, Iceberg+DuckDB for facts.** The load is:
- *Metric stream*: 10^9–10^10 rows, high write rate, queries are "N runs × M metrics × steps".
  ClickHouse is unambiguously right here.
- *Fact/meta-learning*: 10^6–10^8 rows, read-mostly, complex joins, needs reproducible snapshots
  and Python-native access. Iceberg Parquet read by DuckDB (locally, in the training job) is
  right here, and it means your meta-model training script has no database dependency at all.

**Where the alternatives land:**

| Option | Verdict for this workload |
|---|---|
| **DuckDB alone** | Excellent as the *analysis and training-set-construction* engine; not a concurrent multi-writer store. Single-writer semantics make it wrong as the ingestion target. Use it embedded in every meta-model training job, reading Iceberg/Parquet. |
| **ClickHouse** | Right for the metric stream and for interactive cross-run analytics. Weak on frequent single-row updates and on foreign keys — which is fine because facts are append-only. Watch out: `ORDER BY` key design is the whole game; get it wrong and you rewrite the table. |
| **Iceberg + engine** | Right for the durable, engine-neutral archive and for fixation (tags, branches, row lineage). Not a low-latency store; expect 100 ms–seconds. |
| **TimescaleDB** | Tempting because it keeps everything in Postgres, and hypertables + continuous aggregates fit per-step metrics well. Realistic ceiling ~10^9 rows with careful chunking; compression is good but columnar scan performance is ~3–10× behind ClickHouse. Choose it only if operational simplicity dominates and you are confident you will stay under ~10^9 metric points. |
| **Postgres + columnar (Hydra/citus columnar, pg_mooncake, pg_duckdb)** | The 2026 "one database" story is much better than it was. `pg_duckdb`/`pg_mooncake` let Postgres query Parquet/Iceberg directly, which is a very clean way to serve the fact lake without a second cluster. Still not a substitute for ClickHouse on the metric firehose. Good pragmatic starting point for a small team: Postgres + `pg_duckdb` now, add ClickHouse when metric points cross ~10^8. |

### 4.2 Cardinality discipline

The failure mode that kills experiment-metrics systems is **unbounded metric-name cardinality**
(`val/ic/asset=AAPL/fold=3/regime=highvol` as a metric name). Rules:

1. **A metric id is a registered enum, not a free string.** Maintain a `metric_definition` table;
   reject unregistered names at ingest with a clear error. Target ≤ 500 distinct `metric_id`
   values platform-wide.
2. **Dimensions are columns, not name suffixes.** `fold`, `asset_id`, `regime` are columns.
3. **Per-asset metrics do not go in the metric stream.** They belong in `backtest_series` Parquet.
   A 500-asset × 12-metric per-step stream is 6,000 points/step and will bankrupt you; the same
   information is 1 Parquet file.
4. **Log at geometric step intervals** for long runs (every step ≤ 100, then every 10, then every
   100), or better, log per-epoch plus a per-step ring buffer flushed only on failure. LCDB's
   powers-of-√2 anchor schedule is the principled version of this.
5. Budget: aim for **≤ 10^4 metric points per run**. At 10^6 runs that is 10^10 points — the top of
   what one ClickHouse node handles gracefully.

### 4.3 The canonical query patterns, and how each is served

```sql
-- Q1. "Compare 500 runs across 12 metrics" (the interactive one). ClickHouse, uses the base key.
SELECT run_id, metric_id,
       argMax(value, step)                   AS final_value,
       max(value)                            AS best_value,
       argMax(step, value)                   AS best_step,
       quantileExactWeighted(0.5)(value, 1)  AS med
FROM metric_point
WHERE run_id IN (...500 ids...)
  AND metric_id IN ('val/ic','val/sharpe', ...12...)
GROUP BY run_id, metric_id;
-- ~6M points scanned, <200 ms on one node.

-- Q2. "Learning curves for all runs of task T, aligned on epoch" -- uses the projection.
SELECT run_id, epoch, avgIf(value, metric_id='val/ic') AS ic
FROM metric_point
WHERE task_id = {t:String} AND metric_id = 'val/ic'
GROUP BY run_id, epoch ORDER BY run_id, epoch;

-- Q3. Meta-learning training set assembly (DuckDB over Iceberg, in the training job).
SELECT r.task_id, c.params_vec, m.feature_vec, r.fid_epochs,
       r.primary_metric_value, r.primary_metric_se,
       r.wall_seconds, r.peak_gpu_mem_bytes, r.status,
       r.is_censored, r.censor_at_epoch, r.budget_cap_seconds
FROM iceberg_scan('s3://quant-exp/facts/runs', snapshot_id => 8472...) r
JOIN config c USING (config_id)
JOIN task_meta_feature_wide m USING (task_id)
WHERE r.recorded_at <= {as_of:Timestamp}          -- knowledge-time cut
  AND r.task_id NOT IN (SELECT task_id FROM meta_split WHERE split='test' AND split_ver='v3');
```

Rule of thumb: **anything an agent needs in < 100 ms is served from Postgres or a materialised
rollup; anything a human explores interactively is ClickHouse; anything that trains a model is
DuckDB over a pinned Iceberg snapshot.** The pinned snapshot id belongs in the meta-model's own
config so the meta-model is itself reproducible.

---

## 5. Labels: what exactly are you predicting?

This is where most attempts at "train a model on our experiment history" fail — not on plumbing.

### 5.1 The label menu

| Label | Definition | Good for | Pathologies |
|---|---|---|---|
| **Final validation metric** | `primary_metric_value` at full fidelity | Config surrogates, BO | Only observed for uncensored runs → massive selection bias; scale differs per task |
| **Within-task rank / normalised regret** | rank of config among all configs on the same task, or `(y - y_best)/(y_worst - y_best)` | Cross-task meta-learning, LTR | Depends on which other configs happened to be run (composition bias); use a fixed reference portfolio to stabilise |
| **Learning-curve trajectory** | the vector `y_1..y_T` | Early-stopping value models, freeze-thaw BO | Curves are non-monotone and noisy; needs alignment on a common fidelity axis |
| **Gate outcome (pass/fail)** | `gate_evaluation.passed` | Cheap pre-screening before expensive backtests | Extremely imbalanced (often 1–5% pass); gate definitions drift → must condition on `expr_hash` |
| **Gate margin** | signed distance to the boundary | Same, but far more sample-efficient | Requires the gate expression to be differentiable/continuous — design gates that way |
| **Time-to-target** | wall-clock or epochs until `y ≥ τ` | Scheduling, budget allocation | **Right-censored by construction**; the natural home of survival models |
| **Cost-adjusted utility** | `u = y − λ·cost`, or `y / cost`, or Pareto rank over (y, cost, memory) | Budget-aware agents | λ is a business choice; store `reward_definition` as a versioned entity so you can relabel |
| **Realised forward performance** | live/paper P&L after promotion | The only label that isn't a proxy | Months of latency; tiny n; the ultimate target for a final calibration layer |

**Recommendation: store the raw ingredients and compute labels at training time from a versioned
`reward_definition`.** Never bake a scalar reward into the fact table.

```sql
CREATE TABLE reward_definition (
  reward_def_id TEXT PRIMARY KEY,
  version       INTEGER NOT NULL,
  expr          TEXT NOT NULL,  -- 'dsr - 0.15 * log1p(cost_usd) - 0.5 * max(0, pbo - 0.3)'
  expr_hash     BYTEA NOT NULL,
  valid_from    TIMESTAMPTZ NOT NULL, valid_to TIMESTAMPTZ NOT NULL DEFAULT 'infinity',
  notes         TEXT
);
```

### 5.2 Censoring — the biggest correctness issue

Under ASHA/Hyperband/agent kills, **most runs never finish**. A run stopped at epoch 9 of 81 with
`val/ic = 0.021` does not have label "0.021 is its final performance"; it has the label
"its final performance is unknown, and we observed the process up to epoch 9". Three treatments,
in increasing order of correctness:

1. **Naive (wrong, common):** drop censored runs. Induces the strongest possible selection bias —
   the surviving set is exactly the set that looked good early, so your surrogate learns
   "everything works" and systematically overestimates mediocre configs' ceilings.
2. **Censoring-aware regression.** Model `P(y_final | curve_{1..t}, config, task)` and fit with a
   likelihood that handles censoring:
   - **Tobit / censored Gaussian likelihood** when the censoring is a known threshold on the label.
   - **Accelerated Failure Time (AFT)** models for time-to-target. XGBoost and LightGBM both ship
     survival objectives (`survival:aft`, with `label_lower_bound`/`label_upper_bound`), which is
     the single easiest production-grade option: an uncensored run is `[t, t]`, a right-censored
     run is `[t, +inf)`, a run that hit the target between logged steps is `[t_i, t_{i+1}]`
     (interval censoring — which is what step-sampled logging actually gives you).
   - **Cox PH / discrete-time hazard** for "will this run ever reach target τ", where the hazard
     formulation naturally handles time-varying covariates (the curve itself).
3. **Learning-curve extrapolation as imputation.** Use a curve model (§6a) to produce a posterior
   over the final value and train downstream models on the *distribution*, not a point. This is
   what freeze-thaw BO does internally; doing it explicitly at the data layer means every
   downstream meta-model benefits.

Schema requirement: `is_censored`, `censor_at_epoch`, `censor_at_seconds`, `budget_cap_seconds`,
`censor_reason`. Distinguish **informative** censoring (an agent killed it *because it looked bad*
— depends on the outcome) from **non-informative** (spot preemption, cluster maintenance — random
w.r.t. outcome). Only the latter is ignorable. Tag it: `censor_informative BOOLEAN`.

### 5.3 Selection bias and survivorship

Three distinct mechanisms, each needing a different fix:

- **Configuration selection bias.** Configs were proposed by a BO/agent policy, so the observed
  (config, outcome) pairs are drawn from a policy-dependent distribution, not uniformly.
  *Fix:* (a) reserve **5–10% of every sweep's budget for uniform random exploration** — this is
  cheap insurance and it is the only source of unbiased coverage you will ever have; (b) weight
  training examples by inverse propensity when fitting surrogates meant to generalise off-policy;
  (c) always record the proposing policy so you can condition on it.
- **Fidelity selection bias.** Only promising configs get long runs, so high-fidelity observations
  are a biased subsample. *Fix:* model fidelity explicitly (§2.3 `fid_*` columns), fit a joint
  model over (config, fidelity) rather than separate models per fidelity, and include the censored
  low-fidelity observations.
- **Survivorship in the experiment database.** Failed/crashed runs get deleted, "bad" sweeps get
  cleaned up, and an intern purges old artifacts. *Fix:* deletion is forbidden; retraction is an
  event; garbage collection is driven by `ref_count` + `legal_hold` and never removes a `run` row.

**Quant-specific compounding:** the same survivorship problem exists one level down in your market
data (delisted tickers) and one level up in your research narrative (only the strategies that
worked get written up). Sections in `03-financial-ml.md` on PBO/DSR are the correct treatment for
the last one; the key data-engineering requirement is that **the denominator of the multiple-testing
correction is knowable**, i.e. you must be able to count *every* trial ever run against a task.
That is only true if failures and abandoned sweeps are in the database. `n_trials` for DSR comes
from `SELECT count(*) FROM run WHERE task_id = ? AND recorded_at <= ?` — which is wrong the moment
anyone deletes anything.

### 5.4 Practical label construction

```sql
-- The meta-learning "gold table". Materialised nightly into Iceberg; one row per (run, label_def).
CREATE VIEW meta_label AS
SELECT
  r.run_id, r.task_id, r.config_id, r.sweep_id,
  -- regression target, with censoring bounds for AFT/Tobit
  CASE WHEN r.is_censored THEN NULL ELSE r.primary_metric_value END AS y_final,
  r.primary_metric_value                                            AS y_observed,
  r.is_censored, r.censor_informative, r.censor_at_epoch,
  -- within-task normalisation against a FIXED reference portfolio (stabilises across sweeps)
  (r.primary_metric_value - ref.y_p10) / NULLIF(ref.y_p90 - ref.y_p10, 0) AS y_norm,
  percent_rank() OVER (PARTITION BY r.task_id ORDER BY r.primary_metric_value) AS y_rank,
  -- time-to-target (interval-censored)
  tt.t_lower, tt.t_upper,
  -- classification targets
  (r.status = 'ok')                                                 AS y_success,
  r.status                                                          AS y_status,
  g.passed                                                          AS y_gate_pass,
  g.margin                                                          AS y_gate_margin,
  -- cost targets
  r.wall_seconds, r.peak_gpu_mem_bytes, r.cost_usd,
  -- provenance / PIT
  r.recorded_at, r.run_fingerprint
FROM run r
LEFT JOIN reference_stats ref USING (task_id)
LEFT JOIN LATERAL (...time-to-target computation over metric_point...) tt ON TRUE
LEFT JOIN gate_evaluation g ON g.run_id = r.run_id AND g.gate_id = 'g3_cpcv';
```

---

## 6. Internal decision models: what to train, when, and how to evaluate

For each: data requirement (the *n* at which it reliably beats the obvious heuristic), model class,
evaluation protocol, and cold-start. The thresholds below are engineering rules of thumb calibrated
against the public meta-learning literature (LCBench: 2,000 configs × 35 datasets; PD1: tens of
thousands of configs across ~24 workloads; TabRepo: 1,310 × 200) and should be treated as order-of-
magnitude guidance, not guarantees. Signal-to-noise in financial targets is far worse than in
image classification, so for anything whose label is a *backtest* metric, multiply by 2–5×.

### (b) Runtime / memory / cost predictors — **build this first**

- **Beats heuristic at:** ~200–500 completed runs per (flow_family, hw_class).
- **Why first:** the label is cheap, low-noise, observed for *every* run including failures, and
  nearly deterministic given (config, data size, hardware). It is the easiest win and it
  immediately pays for itself in scheduling, bin-packing, and "will this OOM before I submit it".
- **Model:** gradient-boosted trees (LightGBM) on log-target; quantile objectives at τ = 0.5 and
  0.9 so the scheduler can reserve the P90, not the mean. Features: config params, `n_rows`,
  `n_features`, `n_assets`, batch size, model size (parameter count — compute it analytically, do
  not learn it), sequence length, precision, hardware class.
- **Evaluation:** grouped CV by task; report MAPE and, more importantly, **P90 coverage** and
  *under-prediction rate* (an under-predicted memory estimate causes an OOM; an over-predicted one
  wastes a GPU — asymmetric loss, use pinball loss with an asymmetric τ).
- **Cold start:** analytic model. Peak activation memory is calculable from architecture; runtime
  ≈ FLOPs / (achieved TFLOPs of the hw class). The learned model corrects the analytic prior —
  fit residuals, not absolutes.

### (c) Failure classifiers (OOM, divergence, NaN, leakage)

- **Beats heuristic at:** ~300–1,000 runs *with ≥50 examples of each failure class you care about*.
  In practice OOM and divergence dominate; the rest stay rule-based longer.
- **Model:** two-stage. (i) A cheap **pre-flight rule/analytic check** (memory estimate vs device
  capacity, LR vs known stability envelope). (ii) A multiclass GBDT over `run_status` for the
  residual cases, plus an **online** divergence detector on the metric stream (see (h)).
- **Evaluation:** precision@fixed-recall on held-out *tasks*. The operating point matters: blocking
  a valid config is expensive (you lose a possibly-great strategy), so run at high precision for
  the "block" action and use "warn + reduce batch size" as the low-precision action.
- **Cold start:** the analytic checks plus a hand-written rule table. Genuinely, ~80% of OOMs are
  predictable arithmetically and you should never need ML for those.
- **Leakage classifier** deserves special mention: features that predict leakage are structural
  (does the feature pipeline touch data after the label timestamp? does the CV scheme have an
  embargo shorter than the label horizon? does the run's `data_snapshot` post-date the eval
  window?) — encode these as **hard invariants checked at submit time**, and use the learned model
  only to rank suspicious-but-legal cases for human review. Label source: post-hoc discovered
  leaks, which you must record as `result_retracted` events with `reason='leakage'`.

### (a) Config → performance surrogates and learning-curve predictors

- **Beats heuristic (random search) at:** per-task GP, ~20–50 trials on that task. Cross-task
  (transfer) surrogate: ~**30+ tasks × ~50 runs each ≈ 1,500–5,000 runs**, which is roughly the
  scale at which PD1-style GP pre-training and TabRepo-style portfolio learning start to pay.
- **Model progression:**
  1. *0 runs*: prior-data-fitted network out of the box. **ifBO / FT-PFN** is trained purely on a
     synthetic prior over learning curves, runs in a single forward pass (10–100× faster than deep
     GP / deep-ensemble surrogates), and needs zero in-house data. This is the correct cold-start:
     it is a genuine zero-shot learning-curve extrapolator.
  2. *10^2–10^3 runs/task-family*: GP with a learned mean function, or a random-forest surrogate
     (SMAC-style) which handles conditionals and categoricals gracefully.
  3. *10^3–10^4 runs across ≥30 tasks*: **deep kernel / transformer surrogate conditioned on task
     meta-features**, or fine-tune the PFN on your own curves. This is where transfer starts to
     dominate.
  4. *10^4+*: the TabRepo move — stop predicting and start **retrieving**. With dense enough
     coverage, "zero-shot portfolio" (a fixed, greedily-selected sequence of configs known to be
     complementary across tasks) beats a learned surrogate at a fraction of the complexity.
- **Evaluation:** never random-split. Use **leave-one-task-out** (TabRepo explicitly excludes the
  test dataset when computing portfolios "to avoid potential leakage"), and report the metric that
  matters: *simple regret after k evaluations* when the surrogate drives a search, not RMSE.
  Additionally report **rank correlation (Spearman ρ)** — YAHPO Gym's inclusion criterion of
  ρ > 0.7 on held-out data is a sane bar for "this surrogate is usable at all", with ρ ≥ 0.9 for
  metrics you will act on aggressively.
- **Financial caveat:** a surrogate over (config → backtest Sharpe) is fitting a function whose
  noise floor is enormous. Always fit to the **multi-seed mean with its standard error**
  (`primary_metric_se`) and use heteroscedastic likelihoods. Without replication you will build a
  very confident model of noise.

### (d) Early-stopping / continuation value models

- **Beats heuristic (ASHA) at:** ~1,000–3,000 *curves* with enough of them run to completion to
  anchor the extrapolation (target ≥ 20% uncensored, which requires deliberately running a random
  subset to full budget — budget ~5% of compute for this; it is the price of an unbiased anchor).
- **Model:** learning-curve posterior (from (a)) + a decision rule. The clean formulation is
  **value of information**: continue if `E[max(0, y_final − y_incumbent)] > marginal_cost × λ`.
  With FT-PFN-style posteriors this is a closed-form expected-improvement computation over the
  extrapolated curve, which is exactly ifBO's MFPI acquisition.
- **Evaluation:** **replay** against historical sweeps where full curves exist — simulate the
  policy's stop/continue decisions on stored curves and measure (final regret, compute saved). This
  is honest *only* for runs that were actually run to completion; for censored ones you need OPE
  (§8). Report the Pareto frontier of (regret, GPU-hours), never a single number.
- **Cold start:** ASHA with η=3. It is a strong baseline and your model must beat it on the
  frontier, not on a cherry-picked budget.

### (e) Algorithm / strategy-family selectors

- **Beats heuristic (always use the best-on-average family) at:** ~**100+ distinct tasks** with
  ≥10 configs per family per task. This is the ASlib-scale requirement and it is about *task
  count*, not run count — 10,000 runs on 5 tasks teaches a selector nothing.
- **Task supply is the bottleneck in quant.** Deliberately manufacture task diversity: cross
  (universe × horizon × label definition × rebalance frequency × cost regime × time period).
  Time-period slicing is the cheapest axis and also the most honest, since it directly produces
  the regime variation the selector must generalise over.
- **Model:** pairwise/listwise ranker over (task meta-features → family), or cost-sensitive
  classification with the **regret** of choosing wrong as the cost (the ASlib standard). Include
  feature computation cost in the objective.
- **Evaluation:** leave-one-task-out, reported as **closed gap**:
  `(oracle − selector) / (oracle − single_best_family)`. A selector that closes < 20% of the gap is
  not worth deploying. Also report performance under the **frozen meta-split** shipped with the
  data (ASlib's `cv.arff` discipline) so results are comparable over time.
- **Cold start:** the "single best family" default plus an explicit exploration schedule that runs
  a small fixed portfolio of 3–5 complementary families on every new task. That portfolio is
  itself learnable (TabRepo's zero-shot portfolio) once you have ~30 tasks.

### (f) Gate-outcome predictors (pre-screening before expensive backtests)

- **Beats heuristic at:** ~**500–2,000 gated candidates with ≥100 passes**. Below ~100 positives,
  a calibrated logistic model on 5–10 hand-chosen features beats anything fancier.
- **Why this is the highest-ROI model on the list for a quant platform:** gate 3 (CPCV over a decade
  of minute bars) can cost dollars-to-tens-of-dollars and minutes-to-hours per candidate. A
  screener that cuts 70% of candidates at 95% recall multiplies your effective research throughput
  by ~3× with no change to search quality.
- **Model:** GBDT on cheap features — gate-1/gate-2 metrics, learning-curve shape, config, task
  meta-features, *and the correlation of this candidate's return vector with already-promoted
  strategies* (computable only because you stored `backtest_series`). Train on **margin**
  (regression) rather than the pass bit; it converges with ~5× less data.
- **Evaluation:** **recall at fixed screening rate.** Set the operating point by explicit cost
  arithmetic: `E[cost] = screening_rate × gate_cost + (1 − recall) × value_of_missed_candidate`.
  Evaluate under **temporal** splits (train on candidates proposed before T, test after) because
  both the gate definitions and the agent's proposal distribution drift.
- **Critical schema dependency:** you can only train this if failed candidates' gate evaluations
  are retained. The natural instinct is to keep only the winners. Don't.
- **Cold start:** a monotone rule derived from the gate expression itself (e.g. gate-2 Sharpe as a
  single-feature screener) with the threshold set to achieve 99% recall on whatever data you have.

### (g) Learning-to-rank over candidate experiments

- **Beats heuristic at:** ~**100 sweeps × ~30 candidates ≈ 3,000 labelled items in query groups**.
  LTR needs *groups*, and a sweep is a natural group (a query = a task + a budget state).
- **Model:** LambdaMART (`lambdarank` in LightGBM) or a listwise transformer over the candidate
  slate. Target NDCG@k where k = the number of candidates you can actually afford to run.
- **Evaluation:** NDCG@k and **regret@k** on held-out sweeps, plus an OPE estimate (§8) of the
  induced selection policy. Because the historical candidate slates were generated by the old
  policy, pure LTR metrics overstate deployed performance — always pair with OPE.
- **Cold start:** rank by surrogate posterior mean + κ·σ (i.e. just use (a) as the ranker).

### (h) Anomaly detectors on metric streams

- **Usable at:** ~300–500 curves; unsupervised, so it is available very early. Practically, deploy
  at day 1 with fixed rules and swap in the learned version once you have a few hundred curves.
- **Model:** per-(task, metric) normalised curve bank + a distance/reconstruction score. Concretely:
  z-score of the current step against the empirical distribution of curves at the same step from the
  same task family, plus a change-point detector (CUSUM / BOCPD) on loss, gradient norm, and GPU
  utilisation. NaN/Inf and monotone-loss-increase are hard rules, not ML.
- **Evaluation:** time-to-detection vs false-alarm rate, with the cost model "a false alarm wastes a
  restart, a miss wastes the remaining budget".
- **What it catches that nothing else does:** silent data-pipeline corruption (features suddenly
  all-zero → loss drops implausibly fast → this is *leakage*, not success), and distribution shift
  in the feature store.

### Cross-cutting: the "cold-start ladder"

Every model above should be shipped as a **three-tier fallback chain** with the tier recorded in
the decision log: `analytic/rule → shrunk-to-prior learned model → full learned model`. The agent
logs which tier produced the decision so you can measure, in production, whether the learned tier
actually beat the rule tier — an A/B that costs nothing and settles arguments.

