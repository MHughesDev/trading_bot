# BS-007 / 05 — Job service and artifact store

**Spec status:** spec-ready draft · **Req prefix:** JB- · **Formal spec:** COMP-005 ·
**Plan set:** L · **Decisions:** D-02, D-05, D-10 · **Replaces:** the ad hoc
long-running paths (backtest manager, in-memory sweeps and suites, asset-init jobs;
01 §7, FEAT-003 deferrals)

---

## 1. Scope

Every slow thing on the platform becomes a job with one lifecycle: backtests, sweeps,
studies, gate advances, dataset builds, feature materialisation, training, batch
inference, simulations, backfills, `data_qc`, instrument profiles, and skill verification.
Every large output becomes a content-addressed artifact that is passed by handle.

## 2. Job model

```yaml
Job:
  id: job_01J…                  # ULID
  kind: backtest | sweep | study | gate_advance | dataset_build | feature_materialise
        | train | hpo | predict_series | simulate_paths | backfill | data_qc
        | instrument_profile | skill_verify | eval_task
  manifest: { … kind-specific spec … }      # canonical JSON
  manifest_hash: sha256(canonical(manifest) + code_hashes + data_snapshot)
  project_id, session_id?, user_id, submitted_by: agent | user | system
  experiment_id?                # set for evaluation-counted kinds
  queue: agent | human | system ; priority: 0–9
  state: queued → running → succeeded | failed | cancelled   (+ paused for gate approvals)
  progress: { pct, stage, message }          # coarse; no per-epoch spam
  estimate: { compute_s, gpu_s, cost_usd } ; actual: { … }
  result: { summary (≤ 1.5 KB for_model), artifacts: [art_…], metrics? }
  error: { code, field?, rule?, fix?, detail_ref? }   # ≤ 400 B; stack trace → artifact
  heartbeats, created_at, started_at, finished_at, attempts
```

- **Idempotent submission:** the same `manifest_hash` in the same project returns the
  existing job, whatever its state. A client retry, a resume after compaction or a
  duplicate agent call never creates a second run or a second trial (RT-17, hazard H13).
- **Code-carrying kinds:** Layer 1 strategy runs, bring-your-own-model training and
  `skill_verify` snapshot the referenced workspace files into the artifact store. Their
  hashes are part of the manifest.
- **Evaluation-counted kinds** (`backtest`, `sweep`, `study`, `gate_advance`, `train`,
  `hpo`) are registered with the evaluation service on submission, which increments the
  counters (INV-1, 11 §3). Counting is done by the service. It can't be skipped.

## 3. Lifecycle, workers and queues

- **Durable in Postgres:** `jobs`, `job_events`, `job_artifacts`. Workers heartbeat. A
  job without a heartbeat past its lease is re-queued if it's idempotent-safe, otherwise
  marked failed with `lost_worker`.
- **Workers by class:**
  - backtest (Rust, market_simulator);
  - research-runner (Python sandbox for Layer 1 strategies and analysis jobs);
  - trainer (Python, GPU optional);
  - data (backfill, `data_qc`, profiles);
  - eval.
  Worker pools are configurable. The fixed 3-concurrent backtest cap is replaced.
- **Queues:** `agent`, `human` and `system`, with priority and weighted fair share.
  Per-project concurrency limits come from the project tier. Campaign sub-agents share
  their project's share.
- **Cancellation** is cooperative, within a bounded time. Children (sweep members, CPCV
  paths) are cancelled with their parent.
- **Retries:** automatic only for infrastructure failures (worker lost, transient IO), at
  most 2. Logic failures return a structured error and are never retried automatically.
- **Suites and sweeps:** the in-memory sweep and suite state from FEAT-003 moves here. The
  gate funnel stops running under the suite lock (Set K item).

## 4. API and CLI

| REST | CLI (`tbot jobs …`) | Notes |
|---|---|---|
| `POST /api/jobs` | `submit <spec.json>` | Returns `{job_id, deduplicated: bool}` |
| `GET /api/jobs/{id}` | `get <id>` | Compact status plus summary |
| `GET /api/jobs?project&state&kind` | `list [--mine]` | Top-k plus count |
| `POST /api/jobs/{id}/cancel` | `cancel <id>` | — |
| `GET /api/jobs/{id}/logs?tail` | `logs <id> [--tail 50]` | Logs go to a file by default; `--tail` prints a bounded slice |
| `GET /api/jobs/events?project` (SSE) | `watch <ids…>` | Background; exits and prints one line per completion |
| — | `wait <ids…> [--timeout]` | Blocks on the event stream; prints one final summary line per job. On timeout it prints state and exits non-zero, and never loops |

Kind-specific helpers (`tbot bt run`, `tbot exp sweep`, `tbot models train`, …) are thin
wrappers that build manifests and call `submit` (14).

## 5. Events and waiting

- Job state changes are published on NATS → persisted → pushed over SSE to the UI and to
  the session's `agent-host` event bridge.
- **The agent never polls.**
  - Short waits: `tbot jobs wait` inside a Bash call.
  - Long waits: `tbot jobs watch` as a background task. Its completion re-invokes the
    agent.
  - Before a wait expected to exceed the cache TTL, the agent compacts first (04 §4).
- Checkpoint-worthy completions (gate passed, hypothesis killed, repeated failure) emit
  UI checkpoint events (02 §5).

## 6. Artifact store

- **Content-addressed:** `art_<sha256-prefix>`, with bytes in object storage (a local
  filesystem volume in dev) and metadata in Postgres (`artifacts`).
- **Types:**
  - `parquet_extract` (Data API);
  - `dataset`;
  - `feature_matrix`;
  - `prediction_series`;
  - `model_bundle`;
  - `run_outputs` (trades, fills, positions, equity, decision features);
  - `study_result`;
  - `path_set`;
  - `chart_spec`;
  - `report`;
  - `dossier`;
  - `skill_bundle`;
  - `code_snapshot`;
  - `log`.
- **Manifest on every artifact:**
  - producer (job id or Data API query);
  - inputs (handles);
  - query and parameters;
  - row counts;
  - schema;
  - cutoff applied;
  - `data_qc` grade;
  - code hashes;
  - created_at.

  Every number the agent reports is reproducible from a manifest.
- **Scope:** artifacts belong to a project. Cross-project reads need the artifact to be
  shared, or promoted to global (skills bundles, global feature definitions).
- **Retention:** artifacts cited by an Experiment, report, finding or skill are pinned.
  Unreferenced extracts expire after N days (configurable).
- **Access from the container:** `tbot art get <handle> [--to path]` downloads Parquet
  or other files into the workspace. `tbot art show <handle>` prints the manifest and a
  summary.

## 7. Exploration ledger

Every Data API read, `feature_materialise`, analysis job and research-runner job in a
project appends a ledger entry:
- time;
- instruments;
- window;
- what was computed;
- handle.

The ledger is reported with every verdict (D-13, 11 §3). It is also what drives the
`post_hoc` flag.

## 8. Requirements

| ID | Requirement | Pri |
|---|---|---|
| JB-01 | All §1 kinds MUST run through one durable job service with the §2 model | MUST |
| JB-02 | Submission MUST be idempotent by `manifest_hash` within a project | MUST |
| JB-03 | Evaluation-counted kinds MUST register with the evaluation service at submission; clients cannot bypass counting | MUST |
| JB-04 | Jobs MUST survive platform and worker restarts (lease plus re-queue or `lost_worker`) | MUST |
| JB-05 | Queues MUST implement agent, human and system classes with fair share and per-project limits; the fixed backtest cap MUST be removed | MUST |
| JB-06 | Job events MUST stream over SSE; `wait` and `watch` MUST be event-driven with bounded, one-line output | MUST |
| JB-07 | Errors MUST be structured and ≤ 400 B, with detail in an artifact; logic failures MUST NOT auto-retry | MUST |
| JB-08 | Jobs MUST carry estimated and actual compute cost, charged to the project budget | MUST |
| JB-09 | Artifacts MUST be content-addressed with the §6 manifest, scoped to a project, and pinned when cited | MUST |
| JB-10 | Code-carrying jobs MUST snapshot their code into an artifact and include its hash in the manifest | MUST |
| JB-11 | Every data read and analysis job MUST append to the project's exploration ledger | MUST |
| JB-12 | Sweep and suite state MUST move from memory into the job service | MUST |

## 9. Acceptance criteria

1. Submitting the same backtest manifest twice returns one job, and the Experiment's
   trial counter increases by one.
2. Killing a backtest worker mid-job re-queues it. Restarting the platform leaves no
   orphaned or double-counted jobs.
3. A 20-member sweep and an unrelated human backtest run concurrently under fair share.
4. `tbot jobs wait` on a 10-minute job prints exactly one summary line. `watch` re-invokes
   the agent on completion.
5. Every artifact cited in a `final_report` resolves to a manifest that reproduces its
   numbers.
