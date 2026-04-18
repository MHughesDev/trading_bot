# AGENTS.md

Operational control for autonomous or semi-autonomous coding agents. **Not** general onboarding: boundaries, procedures, and verification.

**Human-facing landing page:** [`README.md`](README.md) — short overview, quick start, and links into [`docs/*.MD`](docs/). This file is the **agent/operator contract** (rules, map, CI, handoff).

---

## 0. Mandatory read for every agent session

**Binding rule:** Any autonomous or semi-autonomous agent (Cursor Agent, cloud agent, or similar) **must not** edit this repository, run project commands, or pick up tasks until both steps below are satisfied **in the current session** (after any context reset, new thread, or new run — treat each as a fresh session):

1. **Read [`README.md`](README.md) in full** — overview, quick start, stack summary, and links.
2. **Read this file (`AGENTS.md`) in full** — non-negotiable rules, repository map, testing commands, handoff format, and queue behavior.

**Re-read `AGENTS.md` every time** you start work in a new conversation or agent invocation, even if you believe you remember it. Operational detail (CI commands, queue protocol, risks) changes; memory is not a substitute for the current file.

If your environment supports project rules (for example `.cursorrules`), those rules exist to reinforce this section — they do not replace reading the two files above.

---

## 1. Repository purpose

**Trading Bot** (this repository) is a Python **multi-route AI crypto trading** codebase: **Kraken** for **all market data**, Alpaca for **paper execution only**, shared **decision + risk** path for live and replay, typed contracts (`app/contracts/`), and adapters under `execution/adapters/`. **Coinbase** appears only in the **live execution** adapter when configured — not for market data ingestion.

**This repo owns:** application code (runtime, data plane, models, decision/risk engines, backtesting, control plane, observability helpers), `infra/docker-compose.yml` for local stack, **`docs/*.MD`** including the **[queue system](docs/QUEUE_SCHEMA.md)** ([`docs/QUEUE.MD`](docs/QUEUE.MD), [`docs/QUEUE_STACK.csv`](docs/QUEUE_STACK.csv), [`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD), [`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`](docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD), [`.cursor/skills/add-to-queue/SKILL.md`](.cursor/skills/add-to-queue/SKILL.md), optional [`scripts/generate_queue_stack.py`](scripts/generate_queue_stack.py)), `scripts/` (CI guards, smoke tests).

**This repo does not own:** your brokerage accounts, cloud secrets stores, production deployment pipelines (unless added here), or external ERP/CRM. **Do not** assume access to live keys or paid APIs beyond what `.env` provides.

---

## 2. Agent mission

Agents working here should:

- For **next queue item** work, **run** **`bash scripts/queue_top.sh`** from repo root first (same as **`python3 scripts/print_next_queue_item.py`** — prints the full next **`Open`** row; optional **`--json`**). **Do not** load **[`docs/QUEUE_STACK.csv`](docs/QUEUE_STACK.csv)** or **[`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD)** in full for task selection or closure. Use the row’s **`agent_task`** + **`affected_files`** — read other docs only when **`docs_refs`** names them. To **close** an item after implementation, run **`bash scripts/queue_close.sh --next`** or **`--id <ID>`** (updates `scripts/generate_queue_stack.py`, regenerates the CSV, and archive tables when applicable). If **`queue_top.sh`** prints **`QUEUE_EMPTY:`**, **stop** and report — add or reprioritize per [**§6**](docs/QUEUE.MD#6-how-to-add-or-close-an-item); see [`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`](docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD) **Phase 1**.
- When updating the **queue generator** ([`scripts/generate_queue_stack.py`](scripts/generate_queue_stack.py)): **reorder or append** entries in **`ROWS`** only — **`stack_order`** is filled when you run **`python scripts/generate_queue_stack.py`** (keep **`_QUEUE_EMPTY_`** last).
- Preserve **non-negotiable rules** below unless the user task explicitly overrides.
- Keep **live vs replay** behavior aligned where the architecture expects it (`decision_engine/run_step.py` is the shared decision step).
- Update **docs** when behavior, env vars, or operator-facing flows change.
- Prefer **small diffs**; match existing style and patterns in touched modules.

### 2.1 Relevant procedures, skills, and docs (each user turn)

**Per turn** (for each new user request or follow-up, not only at session start): actively discover what to read before you edit code or run commands. Use whatever your environment provides for **semantic / vector similarity search** over this repository — for example **Cursor’s codebase or documentation search**, **@**-mentions of skills/rules, **embedding-based** “find similar” retrieval, or **ripgrep** with carefully chosen keywords when similarity search is unavailable.

**Where to look first**

- **[`.cursor/skills/`](.cursor/skills/)** — named workflows (queue, audit reports, etc.); read a skill when its description matches the task.
- **[`docs/`](docs/)** — runbooks, specs, queue system (`QUEUE_SCHEMA.md`, `QUEUE.MD`), architecture notes.
- **Skills / rules indexes** — if your tool lists project rules or skills, scan for matches to the user’s query.

**Goal:** find **any** procedures, skills, or documentation that are **relevant to the user’s query** and incorporate them; do not rely only on memory or a single file. This **adds** to §0 (mandatory `README.md` + `AGENTS.md` at session start); it does **not** replace it.

---

## 3. Non-negotiable rules

- **Kraken-only market data:** do not import Alpaca **data** clients outside the Alpaca **execution** adapter. Enforced by `scripts/ci_spec_compliance.sh` (excludes `legacy/`). The same script also fails on **`from legacy.`** / **`import legacy.`** outside the **`legacy/`** tree. If you touch market data ingestion, run that script.
- **Risk is final:** `OrderIntent` execution goes through the risk/signing path described in `execution/intent_gate.py` and settings — do not bypass for “quick tests” in production paths.
- **No raw text → trades:** keep metadata rules on `OrderIntent` consistent with existing validators.
- **No automatic MLflow model promotion** in code: do not add `transition_model_version_stage` / `set_registered_model_alias`; `scripts/ci_mlflow_promotion_policy.sh` enforces this.
- **Do not commit secrets:** `.env` is gitignored; never paste real API keys into code or docs.
- **Do not change `legacy/`** except to fix clear bugs in the snapshot; do not merge legacy patterns into the main V3 tree.
- **Dependency changes:** add to `pyproject.toml` with intent; avoid drive-by upgrades unrelated to the task.

---

## 4. Source of truth (priority when information conflicts)

1. **Explicit user / task instructions** for the current change.
2. **Tests** (`tests/`) and **runtime behavior** of the code being changed.
3. **[`README.md`](README.md)** — default entry (commands, stack summary); deep detail lives in **`docs/`**.
4. **Next task:** use **`bash scripts/queue_top.sh`** (output is derived from **[`docs/QUEUE_STACK.csv`](docs/QUEUE_STACK.csv)** — do not read the whole file for selection). **History / narrative tables:** **[`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD)** (do not load in full unless researching history). Conventions: **[`docs/QUEUE.MD`](docs/QUEUE.MD)** (do not treat narrative tables as executable spec unless the task says so).
5. Other **[`docs/*.MD`](docs/)** reference files (risk precedence, shutdown, Coinbase granularity, etc.).
6. Older comments or stale markdown — verify against code.

**Note:** All documentation filenames under `docs/` use **`.MD`** (uppercase extension).

---

## 5. Repository map

| Path | Role |
|------|------|
| `app/` | Runtime (`live_service`), config (`app/config/`), contracts |
| `data_plane/` | Ingest (Kraken WS/REST, normalizers), bars, features, memory, storage |
| `models/` | Regime, forecast, routing, MLflow registry stubs |
| `decision_engine/` | Pipeline, `run_step` (shared tick), policy/spec proposal path |
| `risk_engine/` | `RiskEngine`, signing |
| `execution/` | Router, service, **adapters** (Alpaca paper, Coinbase live) |
| `backtesting/` | Replay, simulator, portfolio |
| `control_plane/` | FastAPI `api.py`, Streamlit pages |
| `observability/` | Metrics, logging |
| `orchestration/` | Nightly retrain stub |
| `infra/` | `docker-compose.yml`, Prometheus config |
| `scripts/` | `ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`, `ci_canonical_contracts.sh`, `smoke_credentials.py`, `create_github_issues.sh` |
| `tests/` | Pytest suite |
| `docs/` | **[Queue system](docs/QUEUE_SCHEMA.md):** `QUEUE.MD`, `QUEUE_STACK.csv`, `QUEUE_ARCHIVE.MD`, `AUTOMATION_QUEUE_SLICE_PROMPT.MD`, `QUEUE_SCHEMA.md`; other reference `.MD` files |
| `legacy/cryptobot/` | Frozen snapshot; not part of main pipeline |

---

## 6. Required change workflow

1. **Identify** entry points: runtime (`app/runtime/live_service.py`), replay (`backtesting/replay.py`), or APIs (`control_plane/api.py`) as relevant.
2. **Trace** contracts and settings (`app/config/settings.py`, `app/config/default.yaml`, `NM_*` env).
3. **Find tests** under `tests/` for the module; add or extend if behavior changes.
4. **Make the smallest change** that satisfies the task; no unrelated refactors.
5. Run **ruff** and **pytest** (see §9).
6. Update **docs** if behavior, env vars, or operator steps changed (see §10).
7. **Handoff** using the format in §12.

---

## 7. Branch lifecycle (mandatory)

**Goal:** `main` stays current and correct; feature branches do **not** pile up on the remote.

When work uses a **feature branch** (e.g. `cursor/<task>-42e8`):

1. **Branch from `main`** — `git fetch origin main && git checkout -b <branch-name> main` (or equivalent).
2. **Implement and commit** on that branch.
3. **Test before merge** — from repo root, run the full checks in **§9** (`ruff`, `pytest`, `ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`, `ci_canonical_contracts.sh`). **Do not merge into `main` if tests fail** unless the task explicitly documents an exception and the risk.
4. **Merge into `main` only after tests pass** — e.g. `git checkout main && git merge <branch-name>` (or merge via PR in GitHub, then pull `main` locally).
5. **Push `main`** — `git push origin main` so the default branch is fully updated.
6. **Delete the feature branch** so it does not accumulate:
   - **Local:** `git branch -d <branch-name>` (use `-D` only if you intend to discard unmerged work).
   - **Remote:** `git push origin --delete <branch-name>` after the merge is on `main`.

**Queue slices (GitHub CLI):** Finish the slice on your branch first: **`QUEUE_STACK.csv`**, **`QUEUE_ARCHIVE.MD`**, and any **`QUEUE.MD`** §2 snapshot — run **`python3 scripts/ci_queue_consistency.py`**, commit, and **push**. **Only after** that queue work is on the remote PR branch, merge to `main` and delete the head branch in one step (requires `gh` auth and a mergeable PR):

```bash
gh pr merge --merge --delete-branch
```

Run from the repo root with the PR branch checked out, or pass the PR number: `gh pr merge <PR#> --merge --delete-branch`. If the PR is still a draft, run `gh pr ready` first. Do **not** merge until queue closure is pushed — otherwise `main` will lack the archived queue state for that slice. Afterward: `git fetch origin --prune`, `git checkout main && git pull`, and `git branch -d <branch-name>` locally if needed.

**Rule:** One short-lived branch per task is fine; **stale `cursor/*` branches that are already merged should be deleted** on `origin` to avoid clutter.

---

## 8. Engineering patterns (repo-specific)

- **Shared decision step:** `decision_engine/run_step.run_decision_tick` — keep live and replay calling this for parity.
- **Settings:** Pydantic `AppSettings` with `NM_` prefix; defaults in `app/config/default.yaml`.
- **Execution:** venue logic stays in adapters; router validates adapter names.
- **Features:** Polars in `data_plane/features/pipeline.py`; enrich bars consistently for live vs replay.
- **Imports:** follow existing package layout (`app`, `data_plane`, …) as in `pyproject.toml` `packages.find`.
- **Streamlit:** run as `python3 -m streamlit run control_plane/Home.py` (bare `streamlit` may not be on PATH in some environments).

---

## 9. Testing and validation

**Expectations**

- Run tests after logical changes to touched areas.
- For bug fixes, add or tighten a test when feasible.
- Do not weaken tests to greenwash failures.

**Commands** (from repo root, Python ≥3.11):

```bash
pip install -e ".[dev]"
python3 -m ruff check .
python3 -m pytest tests/ -q
bash scripts/ci_spec_compliance.sh
python3 scripts/ci_queue_consistency.py
bash scripts/ci_pip_audit.sh
bash scripts/ci_bandit.sh
bash scripts/ci_mlflow_promotion_policy.sh
bash scripts/ci_canonical_contracts.sh
bash scripts/ci_canonical_gates.sh
```

**Secret scanning (optional locally):** `docker run --rm -v "$PWD:/repo" zricethezav/gitleaks:v8.21.2 detect --source /repo --redact` (same image as CI **gitleaks** job).

Optional extras: `pip install -e ".[alpaca]"` for Alpaca adapter tests; `[dashboard]` for Streamlit.

---

## 10. Documentation rules

Update **when** the change affects:

- Operator-visible behavior, new/changed **`NM_*`** or config keys, or smoke/CI steps → **[`README.md`](README.md)** and/or relevant **[`docs/*.MD`](docs/)**.
- **Queue system** — backlog or process changes → keep **[`docs/QUEUE_SCHEMA.md`](docs/QUEUE_SCHEMA.md)** consistent and update **[`docs/QUEUE_STACK.csv`](docs/QUEUE_STACK.csv)** + **[`docs/QUEUE_ARCHIVE.MD`](docs/QUEUE_ARCHIVE.MD)** + **[`docs/QUEUE.MD`](docs/QUEUE.MD)** snapshot as needed (see schema); only if the task is to record work — otherwise a short PR/summary may suffice.
- **Full-scope audit** — follow **[`docs/FULL_AUDIT.md`](docs/FULL_AUDIT.md)**; end with a **§8** report under **`docs/reports/`**; use Cursor skills **`draft-audit-report`** / **`audit-report-to-queue`** (see **[`.cursor/skills/`](.cursor/skills/)**) to polish or promote findings to **`QUEUE_STACK.csv`**.

Do **not** duplicate long narratives across files; link to `docs/` instead.

---

## 11. Sensitive areas (extra caution)

| Area | Why |
|------|-----|
| `execution/adapters/` | Real money paths; Coinbase live is partially stubbed — do not fake production safety. |
| `risk_engine/` | Capital and gating logic; changes affect all execution. |
| `app/config/`, `NM_*` secrets | Misconfiguration can expose unsigned execution or break venues. |
| `infra/docker-compose.yml` | Binds ports and services; coordinate README if ports change. |

Prefer minimal edits; document assumptions in the handoff.

---

## 12. Task archetypes

**Bug fix:** Reproduce via test or trace; smallest fix; regression test if possible.

**Feature:** Check the **[queue system](docs/QUEUE_SCHEMA.md)** (`QUEUE_STACK.csv` / `QUEUE_ARCHIVE.MD`) for ID alignment; extend existing patterns; update tests and README/docs as needed.

**Refactor:** Behavior-preserving only; run full test suite; do not mix with feature work in the same commit when avoidable.

**Docs-only:** Keep [`docs/*.MD`](docs/) links and filenames consistent (`.MD` extension).

---

## 13. Required handoff format

When finishing work, report:

1. **What changed** (one short paragraph).
2. **Why** (tie to task).
3. **Files touched** (list).
4. **Assumptions / risks** (e.g. untested integration paths).
5. **Validation run** (`ruff`, `pytest`, CI scripts — state what passed).
6. **Follow-ups** (optional, only if clearly needed).

---

## 14. Local environment (Cursor / VM)

- **Docker:** `docker compose -f infra/docker-compose.yml up -d` — QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki (see `README.md` for ports).
- **Control plane:** `uvicorn control_plane.api:app --host 0.0.0.0 --port 8000`
- **`.env`:** gitignored; never commit.

This section is **operational**, not cultural: it prevents repeated setup mistakes.
