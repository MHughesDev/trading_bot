# MLOps Infrastructure for a Quant-Finance ML Training & Experimentation Platform
### State of the art, September 2026 — orchestration, tracking, versioning, registry, reproducibility, resource management

**Context assumed:** production-grade ML training platform for systematic trading strategies, operated in part by an AI agent. Implications that shape every recommendation below:

- **Audit trail is a product requirement, not a nice-to-have.** US model-risk guidance was *replaced* on 2026-04-17 (see §5); "evidence must be produced as a byproduct of how models are built, not reconstructed after the fact."
- **Point-in-time correctness is existential.** A leaky as-of join in a feature pipeline produces a beautiful backtest and loses money. Everything in §3 exists to prevent this.
- **An AI agent operator changes the orchestration calculus.** The DAG is not statically known: the agent decides to fan out 200 more sweeps, kill a branch, escalate resources after an OOM. This kills static-DAG orchestrators (Argo, Kubeflow Pipelines, classic Flyte 1) and strongly favors *durable execution* with dynamic control flow.
- **Run volume is high, run duration is moderate.** Quant training is thousands of small-to-medium runs (feature sweeps, walk-forward folds, hyperparameter grids), not one 10,000-GPU pretraining job. This inverts several standard recommendations — cardinality and metadata throughput matter more than NVLink topology.

---

## 0. Executive recommendation (the stack)

| Layer | Pick | Why |
|---|---|---|
| Durable orchestration | **Temporal** for the control plane; **Ray** for the data/compute plane | Temporal gives crash-proof, resumable, cancellable, signalable agent-driven workflows with a real state machine. Ray gives gang-scheduled distributed training/tuning. Do not make one do the other's job. |
| Alternative single-system pick | **Flyte 2** (GA 2026-08) | If you want one system instead of two and accept a younger runtime. Genuinely the most interesting 2026 release in this space. |
| Batch job placement | **Kueue + JobSet** on Kubernetes, **Karpenter** for spot | Native, no second scheduler, quota + gang admission. Volcano/KAI only if you outgrow it. |
| Experiment tracking | **MLflow 3.x self-hosted** (Postgres + S3) as system of record; a **purpose-built metrics path** alongside it | MLflow 3's `LoggedModel` finally gives model-centric lineage. Its metric ingestion path is the weak point — route high-frequency scalars around it. |
| Model registry | **MLflow Model Registry** + **content-addressed OCI artifacts** | Registry for governance state machine; OCI/ORAS for immutable, digest-addressed bytes. |
| Dataset/feature versioning | **Apache Iceberg** (snapshot IDs + tags) as the substrate; **lakeFS** only if you need cross-table atomic branches | Snapshot ID in run metadata *is* the reproducibility link. Tags survive GC. |
| Feature layer | **Build the as-of join yourself on Iceberg**; Feast only for the online-store abstraction | Quant point-in-time semantics (bitemporal, restatements, vendor revisions) are stricter than any off-the-shelf feature store models. |
| Config | **Pydantic v2 models** as the schema + **Hydra** only for composition/sweeps | Hydra's `struct`-mode `DictConfig` is not a real type system; Pydantic validates, serializes, and hashes deterministically. |
| Env capture | **uv lockfile** inside a **pinned-by-digest container image** | `uv.lock` + image digest + git SHA = the reproducibility triple. |
| Lineage | **OpenLineage** events → your own metadata store (+ Marquez if you want the free UI) | It is the only standard with real adoption. MLMD is alive but a dead end. |
| UI streaming | **Hybrid: object-store/OLAP-backed snapshot + SSE deltas, fan-out via NATS JetStream** | Never one persistent connection per run. See §7. |

---

## 1. Durable workflow / job orchestration

### 1.1 The actual distinction: durable execution vs. task queue vs. DAG engine

Three different things get called "orchestration." They are not substitutes.

**(a) DAG engine (Airflow, Argo Workflows, Kubeflow Pipelines, Flyte 1, Dagster).**
The graph is compiled ahead of execution. The engine walks it, launching pods. Resumption means "re-run from the last successful node," which requires the engine to persist node-level outputs. Cancellation is pod deletion. Dynamic structure is bolted on (`dynamic` tasks, expansion, `map`).
*Failure mode for us:* an agent that decides mid-run to spawn 40 more folds cannot be expressed without a `dynamic` escape hatch that re-compiles a subgraph. You end up writing a workflow that writes workflows.

**(b) Task queue (Celery, RQ, Hatchet's non-durable mode, SQS+workers).**
At-least-once dispatch of independent units. No memory of multi-step progress. You implement resumption yourself via idempotency keys and explicit checkpoints in your own DB.
*Correct verdict:* this is genuinely sufficient for a surprising amount of ML. Hatchet's write-up is right that "many stacks typically have great use-cases for traditional task queues" without durable-execution overhead. If your pipeline is `prepare → train → evaluate → register` with no branching, a task queue plus a `runs` table with a status column is less machinery and easier to debug.

**(c) Durable execution (Temporal, Restate, Inngest, DBOS, Flyte 2).**
Workflow code is *replayed* against an append-only event history. Three properties, per Hatchet's framing:
1. **Automatic retry / crash recovery** — process dies, another worker replays history to reconstruct in-memory state, continues.
2. **Exactly-once dispatch of subtasks** — idempotency keys prevent duplicate side effects.
3. **Deterministic ordering** — the append-only log guarantees invocation order is invariant across replays.

**The cost of (c) is determinism constraints, and it is real.** Workflow code may not:
- iterate over unordered collections (Go maps; in Python, anything relying on set/dict-iteration nondeterminism across processes),
- perform side effects (network, clock, RNG, filesystem) inline — all must go through activities or SDK-provided deterministic wrappers (`workflow.now()`, `workflow.random()`),
- change the sequence of activity invocations for workflows already in flight — this is the **versioning problem**, and it is the #1 operational tax.

For ML this constraint is *cheap*, because the real work is already in coarse-grained external steps ("launch a Ray job," "wait for it"). Your workflow function is glue, ~200 lines. Determinism restrictions bite hard for fine-grained business logic; they barely bite here.

### 1.2 Temporal — what actually shipped (Replay 2026)

Temporal is the mature choice and 2026 closed its biggest ML-relevant gaps:

- **Worker Versioning — GA.** Pins in-flight workflows to their original worker build ID. This is the fix for the versioning tax above: instead of `workflow.patched()` branches accumulating forever, you deploy a new build ID, new workflows go to it, old ones drain on the old one. **This is the single most important Temporal feature for a platform where an agent is editing pipeline code frequently.**
- **Standalone Activities** (public preview Go/Python/.NET). Durable job processing *without* a wrapping workflow — closes the gap with plain task queues so you don't need two systems for one-shot work.
- **Task Queue Priority & Fairness — GA.** Urgency levels plus proportional compute distribution across tenants. Directly usable for "research sweeps must not starve the nightly production retrain."
- **Workflow Streams** (preview). Durable streaming built on Signal & Update, explicitly aimed at "token batches and application-level updates to power responsive UIs." Relevant to §7, though not a replacement for a metrics pipeline.
- **External Payload Storage** (preview, Python/Go) — offload large payloads to S3. **You need this.** Temporal's default payload/history size limits will otherwise force you to pass S3 URIs by hand everywhere.
- **Nexus** — GA in Python. Cross-namespace durable calls; how you let a research namespace invoke a production-governed retraining service without sharing a task queue.
- **Principal Attribution** (pre-release): non-spoofable field recording who invoked a workflow. **For an agent-operated platform under model-risk audit this is close to mandatory** — it is the difference between "a training run happened" and "agent X, acting for user Y, initiated this run."
- Multi-region replication GA (20-min RTO), OpenMetrics GA, Rust SDK preview.

**Primitives that matter, mapped to our needs:**

| Need | Temporal primitive |
|---|---|
| Long-running (days) training DAG | Workflow + `continue-as-new` before history grows unbounded |
| Resumable across control-plane restarts | Event-history replay (free) |
| Resumable *training* across GPU preemption | Activity heartbeats + `heartbeatDetails` carrying checkpoint URI |
| Cancellable | `workflow.cancel()` → cancellation scope → activity sees `CancelledError`, runs cleanup |
| Retryable with escalation | `RetryPolicy` + catch `ActivityError` → re-dispatch to larger-GPU task queue |
| Human/agent approval gate | `Signal` (fire-and-forget) or `Update` (request/response with validation) |
| Agent interrogating a live run | `Query` (read-only, no history mutation) |
| Timeouts | `start_to_close`, `schedule_to_start`, `heartbeat_timeout` — set all three, always |

**Concrete shape:**

```python
@workflow.defn
class TrainStrategyWorkflow:
    def __init__(self) -> None:
        self._cancel_requested = False
        self._state = "PENDING"

    @workflow.run
    async def run(self, spec: TrainSpec) -> TrainResult:
        # Deterministic: derived from workflow ID, stable across replays.
        run_id = workflow.info().workflow_id
        dataset = await workflow.execute_activity(
            materialize_dataset, spec.data,
            start_to_close_timeout=timedelta(hours=2),
            retry_policy=RetryPolicy(maximum_attempts=3),
        )  # returns {"iceberg_snapshot_id": 8395021456789, "uri": "s3://..."}

        gpu_class = spec.gpu_class
        for attempt in range(spec.max_escalations):
            try:
                ckpt = await workflow.execute_activity(
                    launch_ray_train, {"dataset": dataset, "gpu": gpu_class},
                    start_to_close_timeout=timedelta(days=3),
                    heartbeat_timeout=timedelta(minutes=5),   # detects silent GPU death
                    retry_policy=RetryPolicy(maximum_attempts=10,
                                             non_retryable_error_types=["ConfigError"]),
                )
                break
            except ActivityError as e:
                if "CUDA out of memory" not in str(e.cause):
                    raise
                gpu_class = escalate(gpu_class)   # resource-aware retry
        ...
    @workflow.signal
    def request_cancel(self) -> None: self._cancel_requested = True

    @workflow.query
    def state(self) -> str: return self._state    # agent polls this, costs nothing
```

Note the **resource-aware retry** in the `except` block. This is the pattern Flyte 2 builds in natively ("dynamically rerun with more memory after OOM"); in Temporal you write it, which is five lines and arguably clearer.

**Temporal failure modes:**
- **History bloat.** Every activity result is persisted. A workflow that loops 50,000 times over folds will hit event-history limits. Fix: `continue-as-new` on a counter, and External Payload Storage for anything > ~100 KB.
- **Long activities with no heartbeat are invisible.** A hung NCCL all-reduce looks identical to slow progress until `start_to_close` fires days later. **Always heartbeat, and carry the checkpoint URI in the heartbeat details** so a retry resumes rather than restarts.
- **Determinism violations discovered in production.** Use the replay test harness in CI against recorded histories of real workflows. Non-negotiable.
- **It is not a scheduler.** Temporal will happily start 500 workflows that each want 8 GPUs. Quota lives in Kueue (§6), not Temporal — though Task Queue Fairness now handles the worker-side share.

### 1.3 Flyte 2 — the most interesting release of 2026

Flyte 2 went **GA on 2026-08-04** (SDK `flyte` 2.7.2 as of 2026-09-08, Apache-2.0, Python ≥3.10). It is a ground-up rebuild that repositions Flyte from "DAG orchestrator" to "durable AI runtime," and the pitch is precisely the two axes we care about:

1. **The `@workflow` DSL is gone.** Tasks call tasks in plain Python. Loops, conditionals, `try/except` work anywhere. Arithmetic on task outputs works (in Flyte 1, outputs were `Promise` objects and `result / 100` was a compile error).
2. **`asyncio` is the parallelism model**, with the orchestrator as the event loop. `asyncio.gather()` distributes real compute across the cluster, not just I/O.

```python
env = flyte.TaskEnvironment(name="research", image=Image.from_debian_base().with_pip("torch"))

@env.task
async def sweep(specs: list[Spec]) -> list[Result]:
    return await asyncio.gather(*[train(s) for s in specs])   # fan out to N containers

@env.task(retries=3, cache="auto")
def train(s: Spec) -> Result:
    ...
```

Differentiators vs. Temporal:
- **Infrastructure-aware recovery** is first-class: retries can *modify the resource request*, not merely replay the code path. Flyte explicitly frames this as recovering from "failures caused by infrastructure, not the code."
- **Built-in content-addressed caching** across tasks (`cache="auto"`) — in Temporal you build this.
- **Orchestration sandboxes** for LLM-generated pipelines, network-blocked by default; **human-in-the-loop as a first-class construct**; MCP support for agent/IDE integration. For an agent-operated platform this is squarely on-target.
- **Spot with automatic on-demand fallback** and scale-to-zero built in.
- `flyte start devbox` gives a full local backend at `localhost:30080` — the local/prod parity story is better than Temporal's.

**Risks:** GA is one month old. PyPI lists 2 core contributors. Union.ai is a single commercial sponsor with an obvious BYOC upsell. Flyte 1 → 2 is a rewrite, not a migration flag. The durability internals are less battle-tested than Temporal's decade of event-sourcing in production.

**Verdict:** if you are greenfield, willing to be an early adopter, and want one system rather than two, Flyte 2 is a legitimate bet and the closest thing to a purpose-built answer to "agent-driven, resource-aware, crash-proof ML orchestration." If the platform must be boring and auditable from day one, Temporal + Ray.

### 1.4 The rest of the field, briefly

**Prefect / Dagster — consolidated.** **Prefect acquired Dagster Labs, announced 2026-07-13.** Both brands, OSS licenses, and roadmaps continue independently; Prefect Cloud and Dagster+ pricing unchanged; Dagster founders Nick Schrock and Pete Hunt moved to strategic advisor roles. Prefect states it has been profitable for a year. Read this as: the category consolidated, and betting your platform on Dagster's *long-term* roadmap now carries acquirer risk even with the stated commitments. Dagster's asset model is genuinely excellent for data-lineage-shaped problems (freshness policies, asset checks, table-level lineage) and genuinely awkward for "run 400 hyperparameter trials." Prefect 3 is the more ML-flexible of the two (work pools, event-driven automation, `@materialize` for optional asset tracking) but its durability model is weaker than Temporal's — retries and caching, not event-sourced replay.

**Airflow 3.x** — 3.0 GA in 2025; 3.3 by 2026 added asset-based scheduling, snapshot-aware partitioning, and a Task Execution API that allows remote workers in any language. It is the largest ecosystem and the right answer for *data* orchestration. It is the wrong answer for a training control plane: DAG-centric, scheduler-heartbeat-driven, high operational overhead, and the failure mode the community itself names — **"task success as data success," green dashboards over stale tables** — is exactly the class of silent error that kills a quant platform.

**Argo Workflows** — solid, K8s-native, YAML/CRD-based. Good as a *backend* that something else generates. Authoring multi-stage training DAGs directly in Argo YAML with an agent in the loop is a bad time. Use it if you already run it.

**Kubeflow** — the *Pipelines* component keeps losing ground; treat it as legacy. But **Kubeflow Trainer v2 is genuinely good and worth adopting independently**. v2.2 (2026) ships one `TrainJob` CRD across PyTorch/JAX/XGBoost/DeepSpeed/MPI runtimes, built on JobSet, with `activeDeadlineSeconds` timeouts and — importantly — **progress/metrics reported into `TrainJob.status`**, so you don't scrape logs to know where a job is. Breaking changes to note: `PodTemplateOverrides` → `RuntimePatches`, `numProcPerNode` removed from Torch MLPolicy, and **the `ElasticPolicy` API was removed pending redesign** — so as of v2.2 you do *not* get elastic training from Kubeflow Trainer. Roadmap targets DRA integration and native gang scheduling without third-party plugins.

**Ray (Core/Train/Tune/Data/Serve) + KubeRay** — the compute plane, and the recommendation is unambiguous: **use Ray for distributed training and HPO, not for durable orchestration.** Ray is ~2.58 as of Sept 2026. Relevant 2026 state:
- **Ray Train V2** is the default: decoupled controller, **asynchronous checkpointing** (uploads on separate CPU threads so GPUs never idle on I/O), **asynchronous validation** (validation as parallel Ray tasks on different, autoscaled hardware — materially cheaper for walk-forward validation where you have many folds), a local debugging mode that runs the train function in-process, and a `JaxTrainer` for TPU.
- **Elastic training shipped in Ray Train** (core functionality + user guides + multi-host TPU) in the 2.55 line.
- Fault tolerance is `FailureConfig(max_failures=N)` on `RunConfig`: worker-group restart from the latest checkpoint. **The semantics are "restart the whole worker group from checkpoint," not "replace the dead worker in place."** For 8-GPU jobs this is fine. For 500-GPU jobs it is not — see torchft in §6.
- Ray Summit 2026 was dominated by RL/physical AI; infra highlights were topology-aware placement on GB300 (+13% throughput), `SlicePlacementGroup`/`SubslicePlacementGroup` for TPU topology, and the **RELAY batch proxy for the Ray GCS: 2.85× faster actor creation, 16.4× lower RPC latency at 32,000 workers** — which tells you where the old scaling wall was.

**Metaflow** — **Anaconda acquired Outerbounds (announced 2026-04-29)**; `outerbounds.com` now redirects to `anaconda.com`. Metaflow remains the nicest *developer ergonomics* in the category: `@retry`, `@resume` (re-enter a flow at the failed step reusing prior artifacts), `@checkpoint` for spot-survivable training, automatic artifact persistence of every `self.x`. The artifact model — every instance attribute is content-addressed and versioned automatically — is the single best reproducibility ergonomic in the field and worth stealing regardless of what you adopt. Weakness: the control plane is a step-function-ish DAG, not durable execution; dynamic agent-driven topology is awkward (`foreach` only).

**SkyPilot** — not an orchestrator; a **multi-cloud GPU broker**. Managed Jobs give you a controller that provisions the cheapest available GPU across clouds/regions and auto-recovers preempted spot jobs by relaunching and resuming from checkpoint. Correct role in the stack: the thing a Temporal activity *calls* to get GPUs, if you are multi-cloud or scavenging spot capacity. Do not make it your job graph.

**Determined AI** — HPE acquired it in 2021; it now lives as **HPE Machine Learning Development Environment**. The OSS project has not been a growth story. Its ideas (built-in adaptive ASHA, transparent checkpoint/pause/resume, cluster-level fair-share scheduling) were good and have been absorbed elsewhere. **Do not build on it in 2026.**

### 1.5 What's winning in 2026

The pattern that consolidated: **durable control plane + specialized compute plane + K8s quota layer.** Nobody credible is building new platforms on a monolithic DAG engine. The interesting fight is whether the durable layer is general-purpose (Temporal) or ML-native (Flyte 2), and 2026 is the first year the ML-native option is real.

---

## 2. Experiment tracking & model registry

### 2.1 Data models compared

| System | Core entities | Lineage model | Notes |
|---|---|---|---|
| **MLflow 3.x** | Experiment → Run → **LoggedModel**; Trace; Dataset; Prompt; RegisteredModel/Version | Run *produces* LoggedModel; eval Run *consumes* LoggedModel; metrics attach to `(model_id, dataset_digest)` | The 3.x redesign. See below. |
| **W&B** | Project → Run → Artifact (typed, versioned v0/v1/…); Registry → Collection → linked ArtifactVersion | Artifact DAG: runs consume/produce artifacts; registry entries are *pointers*, not copies | Best artifact graph in the field. |
| **Neptune** | Project → Run (attribute-tree namespaces); fork/branch of runs | Attribute paths, run forking | Built explicitly for foundation-model-scale metric volume; overkill for our run shape. |
| **Comet** | Workspace → Project → Experiment → Asset; Model Registry | Experiment→model registration | Mature, unremarkable. |
| **Aim** | Run with `Sequence`s (typed metric streams), Repo | Minimal | Fast local UI, embedded RocksDB. No registry, no governance. |
| **ClearML** | Task (typed: training/testing/data_processing) → Artifacts/Models; Pipelines from Tasks | Task parent/child; task cloning | Task-as-unit is elegant; the platform sprawls (agents, queues, data, serving, now a "Platform Management Center" for cost). Also a Dell AI Ecosystem partner. |
| **DVC / DVCLive** | Git commit *is* the experiment; `dvc.lock` pins stage I/O by hash; DVCLive writes metric files | Git DAG + content hashes | The only genuinely git-native model. Iterative has shifted energy toward **DataChain**; treat DVC as stable-but-not-growing. |

### 2.2 MLflow 3 — what changed and why it matters

MLflow 3 (June 2025) made **models first-class entities** rather than blobs inside a run's artifact directory. As of September 2026 the line is at **3.16.x** (3.15.2 released 2026-08-25), shipping roughly monthly.

The core change:

- Models live at `experiments/<experiment_id>/models/<model_id>/`, **not** under `runs/<run_id>/artifacts/`.
- `log_model()` no longer requires an active run and takes `name=` instead of `artifact_path=`.
- A `LoggedModel` carries `model_id`, `name`, `params`, `status`, `artifact_uri`, `creation_time`, `last_updated_time`, plus links to metrics, datasets, and traces.
- **Metrics attach to `(model, dataset)` pairs**, which is the actually important bit:

```python
mlflow.log_metrics({"sharpe": 1.84, "max_dd": -0.12},
                   model_id=logged_model.model_id,
                   dataset=validation_dataset)      # dataset carries name + digest
```

- Query in both directions:

```python
mlflow.search_logged_models(
    filter_string="metrics.sharpe >= 1.5",
    datasets=[{"dataset_name": "oos_2019_2024", "dataset_digest": "a1b2c3"}],
    order_by=[{"field_name": "metrics.sharpe", "ascending": False}])
mlflow.search_runs(filter_string="models.model_id = '<id>'")
```

**Why this is the right model for quant:** a strategy model is evaluated on many datasets (in-sample, OOS, each walk-forward fold, each regime slice, each stress scenario). MLflow 2's "metrics belong to a run" forced you to either create a run per evaluation (losing the model as the organizing entity) or flatten metric names into `sharpe_oos_2019_fold3` (destroying queryability). MLflow 3 lets you ask *"show me every model whose Sharpe ≥ 1.5 on this exact dataset digest"* — which is the question a validator asks.

Deep-learning checkpoints become separate `LoggedModel`s with `step=epoch`, so checkpoint-level comparison is native.

**Breaking changes to plan for:** MLflow Recipes removed; `fastai`/`mleap`/`diviner`/`gluon` flavors removed; the old AI Gateway config format and deployment server removed; `run_uuid` → `run_id`; `mlflow.gitBranchName` / `mlflow.gitRepoURL` tags gone (**set your own git tags — you need them for the reproducibility triple**). 2.x artifacts are readable by 3.x but not vice versa. Upgrade client and server together.

Recent releases are heavily GenAI-weighted (MCP Registry, LLM judges, Review Queues, `mlflow agent setup`, durable tracing for coding agents) but 3.13 shipped two things that matter to us regardless: **RBAC with an admin UI**, and **automatic trace archival to object storage** — plus an official Helm chart.

### 2.3 What they get wrong at scale

This is where most platforms break, and the failure is always the same shape: **the tracking server's relational backend becomes a time-series database it was never designed to be.**

**MLflow specifically.** The `metrics` table is `(run_uuid, key, value, timestamp, step)`. Every scalar you log at every step is a row. A 100k-step run logging 20 scalars = 2M rows. A thousand such runs = 2B rows in Postgres. Observed consequences, consistent across practitioner reports: query latency decaying from milliseconds → seconds → minutes; `search_runs` with tag/param filters degenerating into multi-way self-joins on a key-value schema where "indexing became a black art"; and the genuinely dangerous one — **the tracking server becoming a deployment dependency**, where an overloaded metadata service blocks model-artifact retrieval and you cannot ship.

**Mitigations, in order of leverage:**
1. **Do not log high-frequency scalars to MLflow.** Log them to a time-series path (see §7). Log to MLflow only: final/best metrics, per-epoch summaries, and the metrics a validator will query. Rule of thumb: if no human will ever filter on it, it does not belong in the RDBMS.
2. **Batch.** `log_batch` / `MlflowClient.log_batch` — one round trip per N metrics, never one HTTP call per scalar. Async logging where available.
3. **Partition and archive.** Cold runs → Parquet on S3, partitioned by `(date, project)`; query with DuckDB/Trino. Keep the RDBMS as the hot index only.
4. **Never pre-aggregate destructively.** A well-documented regret pattern: teams downsample for dashboard speed, then need per-step granularity for a post-mortem that no longer exists. **Keep raw in object storage, aggregate for serving.**
5. **Separate the registry read path from the tracking write path.** Model resolution for deployment must not share a connection pool with sweep ingestion.

**W&B publishes hard numbers** (multi-tenant cloud recommended ceilings), which are the most useful calibration data available for *any* tracker:

| Dimension | Recommended limit |
|---|---|
| Runs per project | 10,000 |
| Steps per run | 500,000 |
| **Distinct metrics (cardinality)** | **100,000** |
| Log frequency | 1,000 rows/min |
| Throughput | 100,000 values/min |
| Single logged value | < 1 MB |
| Single `log()` call | < 25 MB |
| Run config | < 10 MB |
| Files per run | < 1,000 |

And the key sentence: **"performance issues are often caused by logging too many distinct metrics, not by logging too many steps."** High cardinality — not volume — is what kills trackers. The quant-specific trap: logging per-instrument metrics (`pnl/AAPL`, `pnl/MSFT`, … × 3,000 tickers × N runs) blows past 100k distinct keys immediately. **Log per-instrument results as a single artifact (a Parquet table), not as 3,000 metric series.** This one rule will save you more pain than any other in this document.

W&B rate-limits per project and returns `RateLimit-Limit` / `RateLimit-Remaining` / `RateLimit-Reset` headers; the SDK retries. Plan for it.

**Neptune** is the one tracker architected from the start for extreme metric throughput (`neptune-client-scale`), with **run forking** as a first-class concept — fork a run at step N and branch the experiment, with the UI understanding shared history. That is a genuinely good primitive that maps well to "restart this walk-forward from fold 7 with a different regularizer." Their public material is more positioning than architecture, so verify throughput claims against your own load before committing.

### 2.4 Registry recommendation

Split the two concerns that every vendor conflates:

1. **Governance state machine** — `Draft → Validated → Approved → Staging → Production → Retired`, with approvals bound to a specific version, role-gated transitions, and immutable transition history. Use MLflow Model Registry (with 3.13+ RBAC) or build a thin table. Under the 2026 MRM guidance, **validator sign-off must be bound to a specific model version** — this is a hard requirement, not a preference.
2. **Bytes** — content-addressed, immutable, digest-verifiable. Use **OCI artifacts via ORAS**. Docker's model spec shows the pattern: uncompressed, one-file-per-layer blobs identified by media type, so identical model files yield identical reusable layer blobs and inference engines can `mmap` them directly. You get for free: digest-addressed immutability, registry replication, signing (cosign), and RAM/policy controls. Store the OCI digest as a tag on the MLflow model version. That digest is your tamper-evident anchor.

---

## 3. Data & feature versioning, point-in-time correctness

### 3.1 The substrate: Iceberg

The table-format question is settled enough to act on. The useful 2026 framing: *"the table format war is over, and the table formats are not"* — **Iceberg won as the neutral interoperability standard**, and the others persist as specializations (Hudi for keyed CDC, Paimon for LSM streaming, DuckLake for metadata-in-RDBMS simplicity). Delta remains dominant inside Databricks, publishes Iceberg metadata via UniForm, and a proposed Delta 5.0 would adopt Iceberg's v4 metadata tree — convergence at the structural level.

**Iceberg v3** (spec matured 2025; AWS shipped deletion vectors + row lineage support Nov 2025; engine support still uneven — Spark 4.0 / iceberg-spark-runtime 1.7+ leads, Trino/Flink/Dremio catching up):
- **Deletion vectors** — compact bitmaps replacing V2's proliferation of positional delete files. Large I/O win on high-mutation tables.
- **Row lineage** (`_row_id`, `_last_updated_sequence_number`) — row-level provenance, which is directly useful for "which restatement changed this training row."
- **Variant type** — shredded binary semi-structured storage with predicate pushdown, instead of JSON-as-string.
- **`timestamp_ns` / `timestamptz_ns`** — nanosecond precision, called out explicitly for financial trading. **If you touch market microstructure data, this alone justifies v3.** Microsecond truncation silently reorders events within a nanosecond-resolution feed.
- Multi-argument partition transforms.
- **The upgrade is irreversible.** Validate your whole engine stack first.

### 3.2 How reproducibility is actually achieved

**The mechanism is one integer.** Every Iceberg write creates an immutable snapshot. Record the snapshot ID in run metadata and the training dataset is exactly reconstructable:

```sql
SELECT * FROM features.equity_daily VERSION AS OF 8395021456789;
-- or FOR SYSTEM_VERSION AS OF / FOR SYSTEM_TIME AS OF, per engine
```

```python
mlflow.log_param("iceberg_snapshot_id", 8395021456789)
mlflow.log_input(mlflow.data.from_pandas(df, name="equity_daily",
                                         digest=str(snapshot_id)))
```

**Then pin it against GC.** Snapshots expire. A tag is kilobytes of metadata and survives expiration:

```sql
ALTER TABLE features.equity_daily
  CREATE TAG model_v2_training_data AS OF VERSION 8395021456789;
```

**Failure mode, and it is the single most common way ML reproducibility silently dies:** `expire_snapshots` runs on a maintenance schedule, reclaims the files, and a model you must reproduce for a regulator six months later is gone. **Policy: every registered model version's training snapshot gets an Iceberg tag at registration time, created transactionally with the registry write.** Untagged snapshots expire on the normal schedule; tagged ones never do.

**Write-Audit-Publish** is the companion pattern. Write to a branch, run data-quality checks against the branch, fast-forward `main` only on pass. This prevents the corrupted-feature-table-trains-a-model failure — which in quant means a bad vendor file silently teaching the model to trade noise:

```sql
ALTER TABLE t CREATE BRANCH audit_20260912;
-- SET spark.wap.branch=audit_20260912; write; validate;
CALL catalog.system.fast_forward('t', 'main', 'audit_20260912');
```

### 3.3 lakeFS vs. Nessie vs. Iceberg-native

- **Iceberg-native branching/tagging** — per-table. Free, no extra service. **Sufficient for most needs.**
- **Nessie** — Git-like catalog giving **cross-table atomic commits and branches**. If a "dataset version" spans 12 tables that must be mutually consistent, this is the reason to adopt it. Now a REST-catalog implementation among several (Polaris, Lakekeeper, Unity).
- **lakeFS** — branches over the *object store*, format-agnostic. Covers Parquet, raw tick files, CSVs from vendors, model checkpoints — everything Iceberg doesn't. Cost: another stateful service in the data path, and it versions bytes rather than understanding tables.
- **DVC** — git-native, hash-pinned stage I/O (`dvc.lock`). Excellent for small-to-medium research repos, poor above ~TB scale, and Iterative's attention has moved to DataChain. **Use DVC for research-artifact versioning if the team already likes it; do not make it the backbone of a TB-scale feature store.**

**Recommendation:** Iceberg tags as the default. Add lakeFS *only* for the raw-vendor-file landing zone where there is no table format. Skip Nessie unless cross-table atomicity is a demonstrated requirement.

### 3.4 Point-in-time correctness — how it is actually solved

Training-serving skew is "the #1 cause of model performance degradation in production" (Feast's own framing, and it's correct). The mechanism is always the same: **an as-of (temporal) join between an entity-timestamp spine and feature values, where feature rows are admitted only if their event time ≤ the spine timestamp**, with an optional TTL floor.

```sql
-- Conceptually, per feature view:
SELECT s.entity_id, s.event_ts, f.value
FROM   spine s
ASOF JOIN features f            -- DuckDB/Snowflake ASOF; elsewhere a window fn
  ON  f.entity_id = s.entity_id
  AND f.event_ts <= s.event_ts
  AND f.event_ts >  s.event_ts - INTERVAL '30 days'   -- TTL
```

**Every off-the-shelf feature store models exactly one time dimension: event time.** That is insufficient for finance, where you need **bitemporality**:

- `event_time` — when the fact happened (the quarter the earnings covered).
- `knowledge_time` / `as_of_time` — when *you could have known it* (when the filing hit the wire; when the vendor delivered the file; when the restatement landed).

A model trained on `event_time` alone will train on restated fundamentals, revised economic releases, and survivorship-cleaned universes that did not exist at decision time. The backtest will be excellent and the strategy will not work. **This is the defining data-correctness problem of the domain and it is why I recommend owning the as-of join rather than delegating it.**

Concretely:

- Every feature row carries `(entity_id, event_time, knowledge_time, value, source_version)`.
- The training join filters on **both**: `f.event_time <= s.event_time AND f.knowledge_time <= s.decision_time`.
- Restatements are **appends with a later `knowledge_time`**, never updates. Iceberg v3 row lineage helps you prove this.
- The universe/point-in-time-constituents table is itself bitemporal. Survivorship bias enters through the universe definition far more often than through the price data.

**Offline/online split.** The canonical architecture: offline store (columnar, historical, point-in-time joins, high throughput) + online store (KV, sub-ms, latest value only) + a shared feature *definition* used by both. Skew is prevented not by the stores but by **a single transformation definition compiled to both paths**.

Verification is the part people skip and shouldn't. **Chronon's online/offline consistency measurement is the best idea in the feature-store space**: log every online fetch request (keys + timestamps), backfill those *exact* keys and timestamps offline, diff the results, and publish consistency metrics. That converts "we believe there's no skew" into a monitored number. **Steal this pattern regardless of what you build on.** (Chronon itself: Airbnb OSS, ~1,134 commits, last release v0.0.101 July 2025, adopters include Airbnb/Stripe/OpenAI/Netflix/Uber. Scala/Spark-centric and thinly staffed — good ideas, risky dependency.)

**Feature store verdict:**
- **Feast** (≥0.58, joined the **PyTorch Ecosystem 2026-01-22**) — the sane OSS choice, now with OTel instrumentation, OIDC auth, K8s RBAC, a permission system, and lineage UI. Crucial caveat, in their own words: *"You're responsible for writing features to the online store. Feast does not run your transformation logic on incoming events."* Feast is a **registry + retrieval abstraction, not a compute engine**. That is fine — it is exactly the layer worth taking.
- **Tecton** — best managed streaming/real-time; cost and lock-in via a proprietary transformation language. Justifiable if you need sub-second features and don't want to build; otherwise no.
- **Hopsworks** — middle ground, batch+streaming native, self-hostable.
- **Featureform** — thin virtual layer over your existing infra; low adoption, low risk, low value.

**Recommendation:** own the offline point-in-time join (bitemporal, on Iceberg, in your own code, exhaustively tested against golden fixtures). Use Feast for the online-store abstraction and registry if you need online serving. Implement Chronon-style consistency measurement as a scheduled job.

---

## 4. Config & reproducibility

### 4.1 The reproducibility triple, made concrete

Every run record must carry, as first-class indexed fields:

| Leg | Field | Captured as |
|---|---|---|
| **Code** | `git_sha`, `git_dirty`, `diff_blob_uri` | SHA + **refuse to launch on a dirty tree outside dev mode**; if allowed, store the diff as an artifact |
| **Data** | `iceberg_snapshot_id` per table, `dataset_digest` | Snapshot ID + tag created at registration |
| **Env** | `image_digest` (sha256, **not** a tag), `uv_lock_hash`, `cuda_version`, `driver_version`, `gpu_model` | Image by digest; `uv.lock` hashed and stored |
| **Config** | `config_hash` | Canonical JSON of the fully-resolved config, sha256 |
| **Seed** | `seed`, `determinism_level` | See §4.3 |
| **Actor** | `initiated_by` (human or agent identity) | Temporal Principal Attribution, or your own auth context |

MLflow 3 **removed** the automatic `mlflow.gitBranchName` / `mlflow.gitRepoURL` tags — set them yourself or lose the code leg.

### 4.2 Config: Hydra + Pydantic, with a clear division of labor

Hydra status check: **`hydra-core` 1.3.6 released 2026-08-29**, and the project has moved from `facebookresearch/hydra` to the **`hydra-ecosystem`** org (Omry Yadan still primary maintainer), with 1.4.0.dev pre-releases in flight. So: alive, community-stewarded rather than Meta-funded, low velocity. Note that PyPI still advertises **Python 3.7–3.11** — verify against your interpreter before committing.

The honest assessment:
- **Hydra is good at**: hierarchical config composition (`defaults:` lists), CLI overrides, **multirun sweeps** (`-m model.lr=1e-3,1e-4`), and output directory management. These are real and hard to replicate.
- **Hydra is bad at**: types. `DictConfig` with structured configs gives you dataclass-backed validation, but `Any` leaks constantly, `ListConfig`/`DictConfig` are not the types your functions expect, and interpolation (`${...}`) resolution errors surface at access time — hours into a run.

**The pattern that works** (the one Helsing and others converged on):

```python
class TrainConfig(BaseModel):                    # Pydantic v2: the real schema
    model_config = ConfigDict(extra="forbid", frozen=True)
    lr: PositiveFloat
    universe: Literal["sp500", "r3000"]
    lookback_days: int = Field(ge=1, le=2520)

@hydra.main(config_path="conf", config_name="train", version_base="1.3")
def main(cfg: DictConfig) -> None:
    raw = OmegaConf.to_container(cfg, resolve=True, throw_on_missing=True)
    config = TrainConfig.model_validate(raw)     # fail fast, at t=0
    run(config)
```

Hydra composes and sweeps; **the first thing the entrypoint does is resolve to a plain dict and validate into a frozen Pydantic model.** Everything downstream is typed. `extra="forbid"` catches the classic typo'd-override-silently-ignored bug. `frozen=True` makes the config hashable and prevents mid-run mutation.

`config_hash = sha256(json.dumps(config.model_dump(mode="json"), sort_keys=True))` — deterministic, canonical, and the key for cache lookups and duplicate-run detection. **Make the agent operator pass configs as validated JSON against the Pydantic JSON Schema**, not as CLI override strings. Schema-validated structured output is the whole point.

### 4.3 Determinism — what is actually achievable

Be honest in the docs: **PyTorch does not guarantee reproducibility across releases, platforms, or CPU↔GPU.** What you can guarantee is *bitwise reproducibility within a pinned environment on identical hardware*. State that as the SLA.

```python
def set_determinism(seed: int, strict: bool = True) -> None:
    os.environ["PYTHONHASHSEED"] = str(seed)
    os.environ["CUBLAS_WORKSPACE_CONFIG"] = ":4096:8"   # MUST be set before CUDA init
    random.seed(seed); np.random.seed(seed); torch.manual_seed(seed)
    torch.cuda.manual_seed_all(seed)
    if strict:
        torch.use_deterministic_algorithms(True)        # raises on nondeterministic ops
        torch.backends.cudnn.deterministic = True       # convolutions only
        torch.backends.cudnn.benchmark = False          # algo autotuning → nondeterminism

def seed_worker(worker_id: int) -> None:
    s = torch.initial_seed() % 2**32
    np.random.seed(s); random.seed(s)

g = torch.Generator(); g.manual_seed(seed)
DataLoader(ds, num_workers=8, worker_init_fn=seed_worker, generator=g)
```

Points that bite in practice:
- `CUBLAS_WORKSPACE_CONFIG` must be in the environment **before CUDA initializes** — set it in the container entrypoint, not in Python after `import torch`.
- `cudnn.deterministic` covers **only convolutions**; `use_deterministic_algorithms(True)` is the broad switch and it *raises* rather than silently degrading. Run with `strict=True` in CI and `warn_only=True` in exploratory work.
- **Some backward passes remain nondeterministic even with all flags set** (certain RNN paths, several SDPA attention backends — different backends accumulate in different orders and give different numbers). If you use attention, pin the SDPA backend explicitly.
- Determinism costs throughput. Measure it; typical cost is 5–20% depending on op mix.
- `num_workers` changes data order unless seeded as above. So does changing world size — **the effective batch composition is a function of world size**, so "same seed, different GPU count" is a different experiment. Record world size as a config field.
- Non-determinism across *reruns of the same config* is a bug detector: run a nightly job that re-executes a fixed canary config and asserts bitwise-identical final weights. It catches driver upgrades, base-image drift, and accidental `benchmark=True` regressions before they contaminate research.

### 4.4 Environment capture: uv + digest-pinned images

The 2026 consensus: **`uv` for pure-Python, `pixi` when you need conda-channel binaries** (CUDA toolkits, MKL, proprietary market-data SDKs, C/C++ libs). Both produce real cross-platform lockfiles; both are Rust-fast; conda/mamba are legacy for new projects.

Non-negotiables:
1. `uv.lock` (or `pixi.lock`) committed, and **CI fails if the lock is stale relative to `pyproject.toml`**.
2. `uv sync --frozen` in the image build. Never resolve at image build time from ranges.
3. **Reference images by `sha256:` digest everywhere** — in the TaskEnvironment, the K8s PodSpec, and the run record. A mutable tag like `:latest` or even `:v1.4.2` destroys the env leg of the triple silently.
4. Record CUDA runtime version, NVIDIA driver version, and GPU model in run metadata. A driver bump changes numerics.
5. Record the base image digest separately from the app layer digest, so you can answer "which runs used the bad cuDNN?"

### 4.5 Content-addressed artifacts

Everything the platform produces should be addressed by content hash, not by path:

```
s3://artifacts/cas/sha256/<first2>/<hash>        # the bytes, written once
s3://artifacts/runs/<run_id>/manifest.json       # {logical_name -> hash, size, media_type}
```

Benefits that compound: automatic dedup (1,000 walk-forward folds sharing one feature matrix store it once), free integrity verification, trivially correct caching (`config_hash + input_hashes → output_hash`), and tamper evidence for audit. Metaflow's automatic artifact persistence and DVC's `dvc.lock` are both this idea; OCI/ORAS is this idea with a distribution protocol and a signing story attached. For models specifically, prefer OCI.

---

## 5. Lineage, metadata standards, and governance

### 5.1 OpenLineage is the only standard worth adopting

**OpenLineage** — an event-based spec, explicitly modeled on OpenTelemetry's design (extensible facets, pluggable transports, instrumentation-in-the-source). The `RunEvent` shape:

```json
{
  "eventType": "START",                 // START | RUNNING | COMPLETE | ABORT | FAIL | OTHER
  "eventTime": "2026-09-12T10:00:00Z",
  "run":  {"runId": "<UUIDv7>", "facets": {...}},
  "job":  {"namespace": "quant.research", "name": "train_momentum_v3",
           "facets": {"sourceCodeLocation": {...}, "jobType": {...}}},
  "inputs":  [{"namespace": "s3://lake", "name": "features.equity_daily",
               "facets": {"schema": {...}, "version": {"datasetVersion": "8395021456789"},
                          "dataQualityMetrics": {...}}}],
  "outputs": [{"namespace": "s3://models", "name": "momentum_v3",
               "facets": {"columnLineage": {...}}}],
  "producer": "https://github.com/yourorg/platform/v1.2.0"
}
```

Why it wins: **column-level lineage via the `columnLineage` facet** (which the 2026 MRM guidance effectively demands), integrations across Spark/Airflow/Flink/dbt, HTTP/Kafka/file transports, and **custom facets** — which is how you attach `iceberg_snapshot_id`, `model_id`, `config_hash`, and `image_digest` to a lineage graph without forking the spec. **Marquez** is the reference backend/UI; use it to start, and be prepared to ingest the same events into your own store because Marquez's UI is a lineage explorer, not an ML console.

**ML Metadata (MLMD)** — alive (v1.21.0, 2026-06-09) but a dead end. 85% C++, tied to TFX's declining orbit, Windows support dropped after 1.14. Its `Artifact / Execution / Context / Event` quadruple is a sound conceptual model and worth borrowing as a schema. Do not take the dependency.

**OpenTelemetry for ML** — OTel graduated CNCF; the **GenAI semantic conventions are NOT stable** as of 2026 despite widespread marketing to the contrary. Verify the current status before building on specific attribute names. For *training* there is no meaningful semconv at all. Practical approach: use OTel for what it's good at — **traces and metrics for the platform's own services** (API latency, scheduler decisions, activity durations, queue depths) — and OpenLineage for data/model lineage. Do not try to model training runs as OTel spans; the cardinality and duration profiles are wrong for span storage.

### 5.2 Regulatory reality — both regimes moved in 2026

**United States — SR 11-7 was replaced.** On **2026-04-17**, federal regulators replaced SR 11-7 / OCC 2011-12 and related issuances with a **risk-based, principles-driven framework, effective immediately**. Direct implications for platform design:

| Requirement | Platform feature that satisfies it |
|---|---|
| Risk-based tiering; proportionate controls | `risk_tier` field on the model entity; tier drives required approvals, validation depth, monitoring frequency |
| Lifecycle governance as **one auditable chain with continuous lineage** | OpenLineage events from ingest → feature → train → validate → deploy → retire, in one store |
| **Effective challenge as versioned, reproducible artifacts** | Validator notebooks/challenger models are themselves runs, registered and linked to the model version under review |
| **Column-level data lineage and provenance** | OpenLineage `columnLineage` facet |
| **Versioned feature definitions ensuring train-serve consistency** | Feature definitions in git, hashed, referenced by ID in run metadata; Chronon-style consistency metric |
| **Validator sign-off bound to a specific model version** | Registry transition records with `(model_version, validator_identity, timestamp, artifacts)` — immutable |
| Continuous drift/performance monitoring tied to risk tier | Monitoring jobs registered against model versions, thresholds per tier |
| Living documentation tied to production versions | Model card generated **from** metadata at registration, not hand-written |

The load-bearing sentence: *"evidence must be produced as a byproduct of how models are built, not reconstructed after the fact."* Design the metadata path so that **the only way to train is the way that produces the audit trail.** If there is a side door — a notebook that can train and register without going through the workflow — you will discover during an exam that it was used.

**European Union — timelines slipped substantially.** The Digital Omnibus on AI, agreed **2026-05-07**:

| Obligation | Old date | New date |
|---|---|---|
| **Annex III high-risk (use-based — includes creditworthiness/financial)** | 2026-08-02 | **2027-12-02** |
| Annex I high-risk (product-regulated) | 2027-08-02 | 2028-08-02 |
| Synthetic content marking | 2026-08-02 | 2026-12-02 |
| National regulatory sandboxes | 2026-08-02 | 2027-08-02 |

Also: new prohibitions on NCII/CSAM generation from 2026-12-02; a narrow exception permitting special-category data processing for **bias detection** in high-risk systems; the EU AI Office gains exclusive enforcement over GPAI-based systems; and **Articles 25(2)/(4) were strengthened** — upstream providers must share technical documentation, failure-mode information, and testing access with downstream providers, with fines to **3% of worldwide turnover**.

Relevance depends on whether your models are Annex III (credit/insurance decisioning are; proprietary trading generally is not — a strategy model deciding your own firm's positions is not making decisions *about* natural persons). **Article 12 record-keeping (automatic logging over the system's lifetime) is the requirement to design for regardless**, because it is nearly identical to what MRM already demands, and the extra 16 months is schedule relief, not an exemption.

**Model cards.** Generate them; don't write them. A model card is a *view* over metadata you already hold (training snapshot IDs, feature list, eval metrics per dataset, known limitations, intended use, validator sign-offs). Hand-written cards drift from reality within one release. Template against EU AI Act Annex IV fields even if not currently in scope — it costs nothing extra and makes the field set future-proof.

---

## 6. Resource management

### 6.1 GPU sharing: MIG vs. MPS vs. time-slicing

| | Time-slicing | MPS | MIG |
|---|---|---|---|
| Mechanism | CUDA context switching | Multi-Process Service daemon | Hardware partitioning |
| Memory isolation | **None** | Limited (software) | **Yes, dedicated** |
| Fault isolation | **None** | **None** — a fatal client fault can put the MPS server into FAULT state and take down other clients | **Yes, hardware-bounded** |
| Hardware | Most NVIDIA GPUs | Most NVIDIA GPUs | **Ampere+ (CC 8.0+)**: A100, A30, H100, H200, B200, GB200 — up to 7 instances (A30: 4) |
| K8s request | `nvidia.com/gpu.shared: 1` | via device plugin config | `nvidia.com/mig-1g.5gb: 1` |

```yaml
# time-slicing (device plugin config)
sharing:
  timeSlicing:
    renameByDefault: true
    resources:
      - name: nvidia.com/gpu
        replicas: 4
```

**Recommendation for a quant platform:**
- **MIG for shared research capacity.** Quant feature models are frequently small (gradient-boosted trees, shallow nets, linear factor models) and a full H100 is absurd for them. `1g.10gb` slices with hard memory isolation let 7 researchers share one GPU without one person's OOM killing six others. This is the highest-utilization win available and it is under-used.
- **Time-slicing for notebooks and dev only.** Never for anything whose failure matters, and never across trust boundaries.
- **MPS rarely.** The shared fault domain is disqualifying for multi-tenant research; a single bad kernel takes down the server. Only within one trusted workload.
- **Full GPUs for production training.**

Failure modes: MIG profiles are static and **require the GPU to be idle to reconfigure** — so a cluster split into fixed profiles can simultaneously have idle `1g` slices and a queued job needing a full GPU. Plan node pools per profile rather than reconfiguring reactively.

### 6.2 Gang scheduling and quota

**Gang scheduling is mandatory, and the reason is economic, not aesthetic:** *"if seven of eight requested worker pods schedule while the eighth stays pending, the active workers hold GPU allocations at near-zero utilization waiting for the missing peer."* Seven H100s burning money on a barrier. At scale this deadlocks — two half-placed jobs each waiting for the other's GPUs.

**The 2026 recommendation is Kueue + JobSet, not Volcano.** Kueue acts as an **admission gate**: workloads stay `suspend: true` until quota permits, then are unsuspended for the default scheduler to place. JobSet provides gang coordination — "a controller for managing coordinated groups of Jobs with shared failure and success handling." No second scheduler, everything kubectl-native and upgradeable with the cluster.

Kueue's model:
- `ResourceFlavor` — a class of hardware (h100-spot, h100-ondemand, a100-mig-1g).
- `ClusterQueue` — quota per flavor, with **borrowing within a `Cohort`** (research borrows idle production capacity, preempted when production needs it back).
- `LocalQueue` — namespace-scoped entry point.
- `Workload` — the admission unit.
- **Topology-Aware Scheduling**: a `Topology` CRD referenced from `ResourceFlavor.spec.topologyName`, plus pod annotations `kueue.x-k8s.io/podset-required-topology` (hard) / `podset-preferred-topology` (soft) / `podset-unconstrained-topology`. Also **hot-swap** (finds replacement nodes on failure), balanced placement across domains, up to 3 simultaneous constraint layers, and ClusterAutoscaler integration via provisioning admission checks. **Cost: higher memory footprint and scheduling latency, since it tracks all pods and nodes.**

**Switch to Volcano (or NVIDIA KAI) when:** multi-node DDP/MPI needs true gang *placement* correctness rather than gang admission; tenant count grows past ~20 and you need hierarchical fair-share; or you need GPU-sharing-aware and topology-aware bin-packing in the scheduler itself. **Critical operational note: Kueue and Volcano are concurrent layers, not a pipeline — submitting directly to Volcano bypasses Kueue quota entirely.** If you run both, enforce single-entry submission.

**KAI Scheduler** (NVIDIA) runs as a secondary scheduler with gang scheduling, fair-share queuing with GPU quotas, priority preemption, and bin-packing. **Grove** adds `PodCliqueSet`/`PodClique`/`PodCliqueScalingGroup`/`ClusterTopology`/`PodGang` for declarative multi-component inference topologies — inference-oriented, not training.

**DRA (Dynamic Resource Allocation)** is the strategic direction: alpha in 1.26, redesigned in 1.31, beta in 1.32 (v1beta1), v1beta2 in 1.33; production guidance targets **1.33+**. The NVIDIA DRA driver auto-creates a `DeviceClass` named `gpu.nvidia.com`. `ResourceClaimTemplate` selects devices with CEL:

```yaml
device.attributes["gpu.nvidia.com"].productName.startsWith("H100") &&
device.attributes["gpu.nvidia.com"].memory >= 80737418240
```

The shift that matters: **scheduling granularity moves from "whole GPU only" to "GPU, MIG slice, or fraction."** Kubeflow Trainer and Kueue both have DRA integration on their roadmaps. Adopt when your cluster is ≥1.33 and your GPU operator supports it; it is the correct long-term substrate for mixed MIG/full-GPU fleets.

### 6.3 Spot, preemption, and checkpoint resume

Economics (2026): AWS Spot 60–90% off with a **2-minute** notice; GCP Spot 60–91% off with a **30-second** notice; Azure varies by region. Realistic blended outcome is **35–55% off the steady-state compute bill**; training runs capture the best of it at **65–75%**, inference far less because it needs on-demand reserve. The discipline: **spot is wrong wherever the cost of an interrupt exceeds the savings.**

The pattern that works:
1. **Karpenter** with `price-capacity-optimized` allocation across **30+ instance types and multiple AZs** — diversity is what actually reduces interruption rate, more than any single knob. Pod Disruption Budgets to survive consolidation.
2. **Kueue** for prioritization, quota, and gang admission on top.
3. **Checkpoint cadence tuned to interruption economics**, not to a round number. Optimal interval ≈ `sqrt(2 × checkpoint_cost × MTBF)`. With 30-second GCP notices you cannot write a 40 GB checkpoint reactively — **you must checkpoint proactively on a timer** and use the notice only for a graceful `SIGTERM` → flush-what-you-have → update-heartbeat.
4. **Asynchronous checkpointing** (Ray Train V2 does this natively: uploads on separate CPU threads so GPUs never stall on I/O). This is what makes frequent checkpointing affordable.
5. **Checkpoint URI in the durable heartbeat.** Temporal activity heartbeat details, or Metaflow's `@checkpoint`, or SkyPilot managed-job state. On retry, read it and resume. The checkpoint must include **optimizer state, LR-scheduler state, RNG states (torch/numpy/python, per-rank), dataloader position, and the current step** — not just weights. Partial checkpoints are the classic silent-divergence bug: the run "resumes" and quietly trains differently.
6. **Capacity Blocks / training plans** (AWS) for the rare job that genuinely cannot tolerate interruption.

### 6.4 Elastic training

- **`torchrun` elastic** (`--nnodes=MIN:MAX`, c10d rendezvous): the baseline. On membership change it **restarts all workers** from the last checkpoint. Simple, works, and the restart cost dominates at scale.
- **torchft** (`meta-pytorch/torchft`) is the 2026 answer for large jobs. Architecture: a **Lighthouse** coordination server doing per-step heartbeating, plus **Manager** and reconfigurable **ProcessGroup** wrappers (e.g. `ProcessGroupGloo`) that "report errors sanely and be reinitialized gracefully," with checkpoint transport from healthy peers. A quorum protocol allows **membership changes at training-step granularity** — no stop-the-world. Supports fault-tolerant DDP and HSDP (across replicated dimensions, composable with TP/PP); LocalSGD and DiLoCo are experimental. The PyTorch blog demo: **Llama training with ~2,000 synthetic failures every ~15 seconds and no checkpoints**, on Crusoe L40S.
- **Kubeflow Trainer v2.2 removed `ElasticPolicy`** pending redesign — do not plan on it there right now.
- **Ray Train** shipped elastic training in the 2.55 line, including multi-host TPU.

**Recommendation:** for typical quant jobs (1–16 GPUs), `torchrun` elastic + frequent async checkpoints is correct and simple — torchft's complexity isn't repaid. Adopt torchft only if you run jobs where a full restart costs more than a few minutes.

### 6.5 Cost attribution and budget enforcement

Sources of truth:
- **DCGM Exporter** (via GPU Operator) → Prometheus: per-GPU utilization, memory, SM occupancy, power, XID errors. Join `DCGM_FI_DEV_*` series to pod/namespace labels via `kube_pod_*` metrics to get per-job GPU-seconds.
- **OpenCost** for allocation. **1.121.0 (2026-08-05)** added inference cost tracking integrated with llm-d/vLLM, publishing `llm_total_hourly_cost` and `llm_cost_per_million_tokens` labeled by model, version, namespace, cost basis, and workload type. The conceptual contribution generalizes beyond LLMs and is the key idea for a training platform:
  - **Allocation-based cost** — everything attributed to holding the resource (GPU memory reserved, idle capacity between steps). Reconciles with the bill.
  - **Usage-based cost** — only infrastructure consumed doing productive work.
  - **The gap between them is your utilization problem, quantified in dollars.** OpenCost's build-vs-buy guidance — always compare on *allocation* cost — applies directly to "should we keep this reserved cluster."

**Enforcement architecture** (attribution alone changes no behavior):

```
Every workload carries: team, project, strategy_id, run_id, cost_center, initiated_by
   ↓
DCGM + OpenCost → GPU-seconds × instance price → cost per run_id (allocation and usage basis)
   ↓
Aggregate to team/month → compare against budget
   ↓
Enforce at admission: Kueue ClusterQueue nominalQuota per team
   + soft gate: budget-consumed > 80% → new workloads admitted at lower priority
   + hard gate: > 100% → only borrow idle cohort capacity, preemptible
```

The essential property: **enforcement happens at Kueue admission, before GPUs are allocated**, not by killing running jobs after the fact. Killing a 30-hour job at hour 29 for budget reasons destroys value and destroys researcher trust in the platform.

**For an agent operator, add a per-workflow budget as a first-class config field** (`max_gpu_hours`, `max_cost_usd`) that the orchestrator enforces by refusing to launch further activities once exceeded. An agent that can spawn sweeps without a hard budget ceiling is an unbounded-spend incident waiting to happen. Make the ceiling a required argument with no default.

Also instrument **cost per unit of research output** — dollars per completed walk-forward evaluation, per strategy candidate promoted to validation. Raw GPU spend is not a decision-relevant number; cost per validated hypothesis is.

---

## 7. Event streaming for the live UI

### 7.1 The wrong architecture (and why everyone builds it first)

Every training run opens a WebSocket to the API server; the API server holds it for the run's lifetime and pushes every logged scalar to whichever browsers are watching.

This breaks at three places simultaneously: connection count scales with *runs* not *viewers*; a server restart drops every connection and every in-flight metric; and it conflates three genuinely separate concerns — durable metric ingestion, live fan-out, and historical query.

### 7.2 The architecture that scales

**Separate ingestion from fan-out from query.** Four planes:

```
┌──────────────┐  batched HTTP/gRPC   ┌──────────────┐
│ Training job │ ───────────────────▶ │  Ingest API  │  (stateless, autoscaled)
│ (SDK buffers │  every 1–5 s,        └──────┬───────┘
│  in memory)  │  gzip, ≤N points            │ ① durable append
└──────────────┘                             ▼
                                   ┌────────────────────┐
                                   │ Parquet on S3      │  ← system of record
                                   │ part. (date, proj) │
                                   └────────────────────┘
                                             │ ② publish delta
                                             ▼
                                   ┌────────────────────┐
                                   │ NATS JetStream     │  subject:
                                   │                    │  runs.<proj>.<run_id>.metrics
                                   └─────────┬──────────┘
                                             │ ③ subscribe only for runs
                                             ▼    a browser is actually watching
                                   ┌────────────────────┐   SSE    ┌─────────┐
                                   │  SSE Gateway       │ ───────▶ │ Browser │
                                   │  (stateless, N×)   │          └─────────┘
                                   └────────────────────┘
                                             ▲ ④ initial snapshot
                                   ┌─────────┴──────────┐
                                   │ OLAP (ClickHouse/  │
                                   │ DuckDB over S3)    │
                                   └────────────────────┘
```

**The load-bearing property: connections scale with *viewers*, not with *runs*.** Ten thousand concurrent runs with three dashboards open is three SSE connections and three JetStream subscriptions — not ten thousand of anything. This inverts the naive design's scaling curve and is the whole trick.

**Why SSE over WebSockets** for this: training metrics are unidirectional server→client. SSE gives you automatic reconnection with `Last-Event-ID` (resume exactly where you dropped — critical across deploys), plain HTTP semantics (works through every proxy, load balancer, and corporate firewall; no upgrade handshake to misconfigure), gzip compression for free, and HTTP/2 multiplexing so the classic 6-connection-per-origin limit doesn't apply. The recurring 2026 industry judgment — "SSE beats WebSockets for ~95% of real-time cloud applications" — is overstated as a universal claim but exactly right for metric streaming. Reserve WebSockets for genuinely bidirectional surfaces: an interactive terminal into a running container, or the agent-operator console.

**Snapshot + delta, always.** On page load, fetch history from the OLAP/object-store path (one HTTP request, server-side downsampled to ~2,000 points per series using LTTB or time-bucketed min/max/avg — a 4K screen cannot render 500,000 points and a browser will die trying). *Then* open SSE for deltas from the snapshot watermark. Never replay history over the streaming channel.

**Why NATS JetStream over Kafka here:** subject-based routing with wildcards (`runs.*.*.metrics`) maps directly onto the access pattern; ephemeral consumers are cheap enough to create per browser tab; operational footprint is a single Go binary versus a Kafka cluster; and latency is lower. Kafka wins if you need long retention, exactly-once semantics into a warehouse, or already run it — in which case use it. **Redis pub/sub is the trap:** it is fire-and-forget with no replay, so any gateway restart silently loses messages and your chart develops holes nobody notices until someone questions a Sharpe.

### 7.3 Details that decide whether it works

- **Batch in the SDK.** Buffer in memory, flush every 1–5 s or every N points, whichever first. One HTTP round trip per scalar per step is the #1 cause of training-loop stalls from telemetry — and, at ~1,000 steps/sec, of rate-limit bans. W&B's own guidance ("batch related metrics into single log calls") reflects this.
- **Never block the training loop.** Telemetry goes on a background thread with a bounded queue and a **drop-oldest** policy. **If the queue is full, drop metrics — do not backpressure the trainer.** A monitoring system that can stall a 3-day GPU job is worse than no monitoring. Emit a `telemetry_dropped_total` counter so the loss is visible rather than silent.
- **Offline-first.** Write to a local WAL (SQLite or append-only file) and ship asynchronously. A network partition must not lose the run. This is the design W&B's offline mode and `wandb sync` implement, and the reason it exists.
- **Downsample server-side, adaptively by zoom level.** Pre-compute rollups at several resolutions on ingest; serve the one matching the requested time range.
- **Coalesce in the gateway.** Even at 1,000 points/sec arriving, push to the browser at most 4–10×/sec with accumulated deltas. The screen refreshes at 60 Hz; the human reads at ~2 Hz.
- **Cap fan-out per run.** A popular run watched by 50 people should have the gateway maintain one upstream subscription and fan out locally.
- **Logs are a different pipeline from metrics.** Logs are high-volume, low-value-per-byte, and want tail semantics. Ship them to Loki/CloudWatch/S3 and let the UI tail that; do not put log lines through the metric path.

**Temporal's Workflow Streams** (preview) — durable streaming over Signal/Update, aimed at "application-level updates to power responsive UIs." Right tool for **low-frequency, high-value workflow state transitions** that must not be lost (`stage: training → validating`, `checkpoint_saved`, `approval_required`). Wrong tool for scalar metrics — every signal is persisted to workflow history and you would bloat it into failure within an hour. **Use both: Temporal streams for the state machine, NATS for the firehose.**

---

## 8. Consolidated failure modes

Ranked by frequency × damage, in this domain.

1. **Look-ahead bias via single-timestamp features.** Backtest is great, live trading isn't. Fix: bitemporal `(event_time, knowledge_time)` everywhere; restatements are appends, never updates; golden-fixture tests for every as-of join.
2. **Snapshot expiry destroys reproducibility.** `expire_snapshots` deletes the files behind a registered model's training data. Fix: tag transactionally at registration; audit monthly that every non-retired model version's tag resolves.
3. **Metric cardinality explosion.** Per-instrument metric keys × runs → tracker unusable. Fix: hard limit on distinct metric keys per run, enforced in the SDK; per-entity results go to Parquet artifacts, not metric series.
4. **Tracking DB becomes a deployment dependency.** Overloaded metadata service blocks model retrieval; you cannot deploy. Fix: separate read path, cache resolutions, serve registry from content-addressed storage that doesn't need the RDBMS.
5. **Mutable image tags.** `:latest` silently changes; the env leg of the triple is fiction. Fix: `sha256:` digests everywhere, enforced by admission policy.
6. **Partial checkpoints.** Weights saved, optimizer/RNG/dataloader state not. Resumed run diverges silently and nobody notices because the loss curve looks plausible. Fix: single `save_state()`/`load_state()` pair covering everything, with a resume-equivalence test in CI.
7. **No gang scheduling.** Partially-placed distributed jobs hold GPUs at ~0% utilization; two such jobs deadlock. Fix: Kueue + JobSet, all-or-nothing admission.
8. **Silent GPU failure.** XID errors, thermal throttling, a dead NVLink — the job hangs rather than crashes, and `start_to_close` doesn't fire for days. Fix: activity heartbeats with step progress; Node Problem Detector auto-tainting nodes; alert on throughput derivative, not just liveness.
9. **Telemetry backpressures training.** Fix: bounded queue, drop-oldest, drop counter.
10. **"Task success as data success."** Every DAG node green, tables stale, model retrained on last week's data. Fix: assert on *data* freshness and snapshot advancement, not task exit codes.
11. **Non-deterministic "reproductions."** Fix: nightly canary config asserting bitwise-identical outputs; catches driver/base-image drift.
12. **Agent-driven unbounded spend.** Fix: mandatory `max_gpu_hours` / `max_cost_usd` per workflow, enforced at admission, no default value.
13. **Determinism violations found in prod (Temporal).** Fix: replay tests against recorded histories in CI; Worker Versioning for in-flight pinning.
14. **Temporal history bloat.** Fix: `continue-as-new`; External Payload Storage.
15. **Kueue bypass.** Jobs submitted straight to Volcano/kube-scheduler evade quota. Fix: single submission path, validating webhook rejecting unqueued GPU pods.

---

## 9. Recommended architecture, concretely

```
┌─────────────────────────────────────────────────────────────────────┐
│ AGENT OPERATOR / RESEARCHER UI                                      │
│  Pydantic-validated JSON configs · SSE dashboards · approval gates  │
└────────────────────────────┬────────────────────────────────────────┘
                             │ Temporal Client (Signal / Update / Query)
┌────────────────────────────▼────────────────────────────────────────┐
│ CONTROL PLANE — Temporal                                            │
│  Workflows: TrainStrategy, WalkForward, Sweep, Promote, Retrain     │
│  Worker Versioning · Task Queue Fairness · External Payload Storage │
│  Principal Attribution (who/what initiated — audit requirement)     │
└────────┬──────────────────────────────────────┬─────────────────────┘
         │ activities                           │ activities
┌────────▼──────────────┐            ┌──────────▼──────────────────────┐
│ DATA PLANE            │            │ COMPUTE PLANE                   │
│ Spark / DuckDB / Ray  │            │ Ray Train V2 + Tune on KubeRay  │
│ Iceberg v3 tables     │            │ async checkpoint + async valid. │
│ bitemporal as-of join │            │ torchrun elastic (torchft @scale)│
│ WAP branches          │            │ Kueue+JobSet · Karpenter spot   │
└────────┬──────────────┘            └──────────┬──────────────────────┘
         │                                      │
         └──────────────┬───────────────────────┘
                        ▼
┌─────────────────────────────────────────────────────────────────────┐
│ METADATA PLANE                                                      │
│  MLflow 3.x (Postgres + S3): Experiment/Run/LoggedModel/Registry    │
│  OpenLineage events → lineage store (+ Marquez UI)                  │
│  CAS artifacts on S3 · models as OCI artifacts (ORAS + cosign)      │
│  NATS JetStream → SSE gateway → live UI                             │
└─────────────────────────────────────────────────────────────────────┘
```

**Build order:**
1. Reproducibility triple + CAS artifacts + run record schema. *Everything else is worthless without this, and it is impossible to retrofit.*
2. Bitemporal as-of join on Iceberg, with golden-fixture tests. *The domain-critical correctness property.*
3. Temporal control plane, one workflow type, one activity, end to end.
4. MLflow 3 with the metric-routing discipline of §2.3 from day one.
5. Kueue + Karpenter + spot + checkpoint/resume.
6. OpenLineage emission + auto-generated model cards + registry state machine.
7. NATS/SSE live UI.
8. MIG partitioning for research capacity; cost attribution and budget gates.

Steps 1–2 are where the platform is won or lost. Steps 7–8 are what people want to build first.

---

## Sources

**Orchestration**
- [Union.ai — Introducing Flyte 2.0](https://www.union.ai/blog-post/introducing-flyte-2-0-dynamic-crash-proof-resource-aware-ai-orchestration)
- [Union.ai — Flyte 2 Is Generally Available](https://www.union.ai/blog-post/flyte-2-is-generally-available-the-durable-open-source-ai-runtime)
- [GlobeNewswire — Flyte 2 GA announcement (2026-08-04)](https://www.globenewswire.com/news-release/2026/08/04/3338397/0/en/Union-ai-Announces-General-Availability-of-Flyte-2-Bringing-Durable-Runtime-to-Open-Source.html)
- [Flyte 2 platform page](https://flyte.org/platform)
- [Union.ai Docs — From Flyte 1 to 2](https://www.union.ai/docs/v2/byoc/user-guide/flyte-2/)
- [PyPI — flyte 2.7.2](https://pypi.org/project/flyte/)
- [Temporal — Replay 2026 product announcements](https://temporal.io/blog/replay-2026-product-announcements)
- [Temporal — The definitive guide to Durable Execution](https://temporal.io/blog/what-is-durable-execution)
- [Temporal — 9 ways to use Temporal in your AI workflows](https://temporal.io/blog/nine-ways-to-use-temporal-in-your-ai-workflows)
- [Hatchet — How to think about durable execution](https://hatchet.run/blog/durable-execution)
- [Morningstar/BusinessWire — Prefect acquires Dagster (2026-07-13)](https://www.morningstar.com/news/business-wire/20260713065285/prefect-acquires-dagster-uniting-the-two-leading-modern-orchestrators)
- [Pulse2 — Prefect acquiring Dagster Labs](https://pulse2.com/prefect-acquiring-dagster-labs/)
- [Data Lakehouse Hub — Orchestration in 2026: Airflow 3 vs Dagster vs Prefect vs Event-Driven](https://datalakehousehub.com/blog/orchestration-in-2026/)
- [Anaconda — Anaconda Acquires Outerbounds (2026-04-29)](https://www.anaconda.com/press/anaconda-acquires-outerbounds)
- [The New Stack — Anaconda acquires Outerbounds](https://thenewstack.io/anaconda-ai-outerbounds-python-metaflow/)
- [Outerbounds — Indestructible training with @checkpoint](https://outerbounds.com/blog/indestructible-training-with-checkpoint)
- [Anyscale — Ray Summit 2026 recap](https://www.anyscale.com/blog/ray-summit-2026-recap)
- [Anyscale — Ray Train V2](https://www.anyscale.com/blog/ray-train-v2-unified-distributed-training-on-ray)
- [Ray docs — Handling Failures and Node Preemption](https://docs.ray.io/en/latest/train/user-guides/fault-tolerance.html)
- [Ray docs — Saving and Loading Checkpoints](https://docs.ray.io/en/latest/train/user-guides/checkpoints.html)
- [Ray 2.55.0 release notes](https://github.com/ray-project/ray/releases/tag/ray-2.55.0)
- [Kubeflow Blog — Trainer v2.2 release](https://blog.kubeflow.org/kubeflow-trainer-v2.2-release/)
- [GitHub — kubeflow/trainer](https://github.com/kubeflow/trainer)
- [SkyPilot — Managed Jobs docs](https://docs.skypilot.co/en/v0.11.1/examples/managed-jobs.html)
- [HPE Machine Learning Development Environment (Determined AI) docs](https://hpe-mlde.determined.ai/latest/)
- [Apache Airflow 3 GA announcement](https://airflow.apache.org/blog/airflow-three-point-oh-is-here/)

**Tracking & registry**
- [MLflow 3 docs](https://mlflow.org/docs/latest/genai/mlflow-3/)
- [MLflow 3 migration guide](https://mlflow.org/docs/latest/ml/mlflow-3/)
- [MLflow v3.1.0 release notes](https://github.com/mlflow/mlflow/releases/tag/v3.1.0)
- [MLflow releases index (through 3.15.2 / 3.16)](https://mlflow.org/releases/)
- [MLflow Model Registry docs](https://mlflow.org/docs/latest/ml/model-registry/)
- [Databricks/Azure — Track and compare models using MLflow Logged Models](https://learn.microsoft.com/en-us/azure/databricks/mlflow/logged-model)
- [Databricks — MLflow 3.0 announcement](https://www.databricks.com/blog/mlflow-30-unified-ai-experimentation-observability-and-governance)
- [W&B — Logging at scale and performance limits](https://docs.wandb.ai/models/track/limits)
- [W&B — Registry overview](https://docs.wandb.ai/guides/core/registry/)
- [W&B Registry product page](https://wandb.ai/site/registry/)
- [Neptune — Building the most scalable experiment tracker for foundation models](https://neptune.ai/blog/building-the-most-scalable-experiment-tracker-for-foundation-models)
- [GitHub — neptune-client-scale](https://github.com/neptune-ai/neptune-client-scale)
- [Experiment Tracking at Scale — Deep Dive](https://adhdecode.com/mlops/experiment-tracking/experiment-tracking-at-scale-thousands-runs/)
- [ClearML — Platform Management Center launch](https://clear.ml/blog/clearml-launches-platform-management-center-to-bring-financial-clarity-to-enterprise-ai-infrastructure)
- [DVCLive docs](https://doc.dvc.org/dvclive)

**Data & feature versioning**
- [Apache Iceberg Table Spec](https://iceberg.apache.org/spec/)
- [Apache Iceberg — Branching and Tagging](https://iceberg.apache.org/docs/latest/branching/)
- [Dremio — Apache Iceberg V2 vs V3](https://www.dremio.com/blog/apache-iceberg-v2-vs-v3-what-changed-and-what-it-means-for-your-tables/)
- [Databricks — Apache Iceberg v3: Moving the Ecosystem Towards Unification](https://www.databricks.com/blog/apache-icebergtm-v3-moving-ecosystem-towards-unification)
- [AWS — Iceberg V3 deletion vectors and row lineage support](https://aws.amazon.com/about-aws/whats-new/2025/11/aws-apache-iceberg-v3-deletion-vectors-row-lineage)
- [Dremio — Apache Iceberg Machine Learning: Solving Data Versioning for AI](https://www.dremio.com/blog/apache-iceberg-machine-learning-solving-data-versioning-for-ai/)
- [AWS — Write-Audit-Publish with Iceberg branching and Glue Data Quality](https://aws.amazon.com/blogs/big-data/build-write-audit-publish-pattern-with-apache-iceberg-branching-and-aws-glue-data-quality/)
- [Lakehouse Table Formats in 2026: Iceberg, Delta, Hudi, Paimon, DuckLake](https://amdatalakehouse.substack.com/p/lakehouse-table-formats-in-2026-iceberg)
- [Delta Lake — Delta Kernel and the new UC Delta APIs (2026-08-20)](https://delta.io/blog/2026-08-20-simplifying-your-open-lakehouse-with-the-delta-kernel-and-the-uc-delta-apis/)
- [Dremio — Data Lakehouse Versioning: Nessie vs Iceberg vs lakeFS](https://www.dremio.com/blog/data-lakehouse-versioning-comparison-nessie-apache-iceberg-lakefs/)
- [PyTorch Blog — Feast Joins the PyTorch Ecosystem (2026-01-22)](https://pytorch.org/blog/feast-joins-the-pytorch-ecosystem/)
- [Feast docs — Scaling Feast](https://docs.feast.dev/how-to-guides/feast-snowflake-gcp-aws/scaling-feast)
- [GitHub — airbnb/chronon](https://github.com/airbnb/chronon)
- [Feature Store Comparison 2026: Feast, Tecton, Hopsworks](https://mlopsplatforms.com/posts/feature-store-comparison-2026/)
- [Point-in-Time Correctness for Training Data](https://apxml.com/courses/feature-stores-for-ml/chapter-3-data-consistency-quality/point-in-time-correctness)

**Config & reproducibility**
- [PyTorch — Reproducibility / randomness notes](https://docs.pytorch.org/docs/main/notes/randomness.html)
- [PyPI — hydra-core 1.3.6 (2026-08-29)](https://pypi.org/project/hydra-core/)
- [GitHub — hydra-ecosystem/hydra](https://github.com/facebookresearch/hydra)
- [Helsing — Strongly-typed structured configuration in Hydra](https://blog.helsing.ai/posts/strongly-typed-structured-configuration-in-hydra/)
- [Towards Data Science — Configuration management with Pydantic and Hydra](https://towardsdatascience.com/configuration-management-for-model-training-experiments-using-pydantic-and-hydra-d14a6ae84c13/)
- [pydevtools — uv vs pixi vs conda for Scientific Python](https://pydevtools.com/handbook/explanation/uv-vs-pixi-vs-conda-for-scientific-python/)
- [Docker — Why Docker Chose OCI Artifacts for AI Model Packaging](https://www.docker.com/blog/oci-artifacts-for-ai-model-packaging/)
- [CNCF — How OCI Artifacts will drive future AI use cases](https://www.cncf.io/blog/2025/08/27/how-oci-artifacts-will-drive-future-ai-use-cases/)
- [ORAS — OCI artifact concepts](https://oras.land/docs/concepts/artifact/)

**Lineage & governance**
- [OpenLineage — Getting Started](https://openlineage.io/getting-started/)
- [GitHub — OpenLineage/OpenLineage](https://github.com/OpenLineage/OpenLineage)
- [OpenLineage — How OpenLineage takes inspiration from OpenTelemetry](https://openlineage.github.io/blog/openlineage-takes-inspiration-from-opentelemetry/)
- [GitHub — google/ml-metadata (v1.21.0, 2026-06-09)](https://github.com/google/ml-metadata)
- [OpenTelemetry GenAI semantic conventions — what actually shipped in 2026](https://dev.to/azena-ai/opentelemetrys-genai-semantic-conventions-are-not-stable-yet-heres-what-actually-shipped-in-2026-3mke)
- [Databricks — Model risk management in 2026: revised interagency guidance](https://www.databricks.com/blog/model-risk-management-2026-bankers-guide-revised-interagency-guidance)
- [ModelOp — SR 11-7 Model Risk Management](https://www.modelop.com/ai-governance/ai-regulations-standards/sr-11-7)
- [Inside Privacy — EU AI Act Update: Timeline Relief, Targeted Simplification, and New Prohibitions](https://www.insideprivacy.com/artificial-intelligence/eu-ai-act-update-timeline-relief-targeted-simplification-and-new-prohibitions/)
- [Help Net Security — What the EU AI Act requires for AI agent logging](https://www.helpnetsecurity.com/2026/04/16/eu-ai-act-logging-requirements/)
- [TrueScreen — EU AI Act Article 12 record-keeping requirements](https://truescreen.io/insights/ai-act-record-keeping-requirements/)
- [arXiv — TechOps: Technical Documentation Templates for the AI Act](https://arxiv.org/html/2508.08804v1)

**Resource management**
- [ScaleOps — GPU Sharing in Kubernetes: MIG vs MPS vs Time-Slicing](https://scaleops.com/blog/kubernetes-gpu-sharing/)
- [Kubesimplify — NVIDIA MIG on Kubernetes](https://blog.kubesimplify.com/slicing-gpus-in-kubernetes-with-nvidia-mig)
- [Spheron — Kubernetes GPU Orchestration in 2026: DRA, KAI Scheduler, Grove](https://www.spheron.network/blog/kubernetes-gpu-orchestration-2026/)
- [CloudOptimo — Kubernetes AI Infrastructure in 2026: GPU Scheduling & Production Realities](https://www.cloudoptimo.com/blog/kubernetes-ai-infrastructure-in-2026-gpu-scheduling-and-production-realities/)
- [Kueue — Topology-Aware Scheduling](https://kueue.sigs.k8s.io/docs/concepts/topology_aware_scheduling/)
- [Kueue — Features overview](https://kueue.sigs.k8s.io/docs/overview/)
- [Kueue 1.3 + JobSet: skip the second scheduler](https://bex.co/blog/2026/09/10/kueue-jobset-native-batch-stack)
- [InfraCloud — Batch Scheduling on Kubernetes: YuniKorn, Volcano, Kueue](https://www.infracloud.io/blogs/batch-scheduling-on-kubernetes/)
- [GitHub — meta-pytorch/torchft](https://github.com/meta-pytorch/torchft)
- [PyTorch Blog — Fault Tolerant Llama: 2000 synthetic failures every ~15s, no checkpoints](https://pytorch.org/blog/fault-tolerant-llama-training-with-2000-synthetic-failures-every-15-seconds-and-no-checkpoints-on-crusoe-l40s/)
- [PyTorch docs — torchrun (Elastic Launch)](https://docs.pytorch.org/docs/stable/elastic/run.md)
- [PyTorch tutorials — Fault-tolerant Distributed Training with torchrun](https://docs.pytorch.org/tutorials/beginner/ddp_series_fault_tolerance.html)
- [Spot Instance Strategies in 2026: Karpenter, Kueue, and the Interrupt Patterns](https://pdpspectra.com/blog/spot-instances-strategies-2026/)
- [AWS — EC2 Capacity Blocks for ML and SageMaker training plans](https://aws.amazon.com/blogs/machine-learning/secure-short-term-gpu-capacity-for-ml-workloads-with-ec2-capacity-blocks-for-ml-and-sagemaker-training-plans/)
- [CNCF — OpenCost 1.121.0: Kubernetes inference cost tracking (2026-08-05)](https://www.cncf.io/blog/2026/08/05/opencost-1-121-0-first-of-a-kind-kubernetes-inference-cost-tracking/)
- [NVIDIA — DCGM Exporter documentation](https://docs.nvidia.com/datacenter/cloud-native/gpu-telemetry/latest/dcgm-exporter.html)
- [Spheron — GPU Cloud FinOps for AI Teams](https://www.spheron.network/blog/gpu-cloud-finops-ai-teams-cost-allocation-chargeback-budgeting/)

**Event streaming / UI**
- [Tinybird — 7 Real-Time App Architecture Approaches](https://www.tinybird.co/blog/build-real-time-apps)
- [Server-Sent Events vs WebSockets in 2026](https://blog.codercops.com/blog/server-sent-events-vs-websockets-2026)
- [Scaling Real-Time APIs to 100k+ Concurrent Connections](https://dev.to/dzakiamriz/scaling-real-time-apis-to-100k-concurrent-connections-websockets-sse-and-redis-pubsub-5hhm)
- [NATS vs Redis vs Kafka: Message Broker Comparison 2026](https://www.index.dev/skill-vs-skill/nats-vs-redis-vs-kafka)
- [Real-Time Event Streaming: Kafka vs Redis Streams vs NATS in 2026](https://dev.to/young_gao/real-time-event-streaming-kafka-vs-redis-streams-vs-nats-in-2026-34o1)

**Domain**
- [MLOps best practices for quantitative trading teams](https://medium.com/@online-inference/mlops-best-practices-for-quantitative-trading-teams-59f063d3aaf8)
- [Backtesting Quantitative Trading Strategies: From Research Bias to Production Reality](https://ernie55ernie.github.io/trading/2026/08/27/backtest.html)
- [arXiv — A Rigorous Walk-Forward Validation Framework for Market Microstructure Signals](https://arxiv.org/html/2512.12924v1)
