# Automation prompt — queue slice → tests → PR → merge → prune

**Documentation last reviewed:** **2026-04-19**.

**Queue system:** This prompt is part of the **[queue system](QUEUE_SCHEMA.md)** — see [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) for every file that implements backlog machinery.

Use this document as the **system prompt** (or operator checklist) for an autonomous agent or CI-adjacent job that implements **one slice** from [`QUEUE_STACK.csv`](QUEUE_STACK.csv) (task text in **`agent_task`**), validates it, opens a PR, merges to `main`, and cleans up branches.

**Safety:** This workflow merges **the slice branch you created** into `main` after review. It does **not** mean “merge every remote branch into `main`” — that would be unsafe. **Prune** only **merged** feature branches (e.g. `cursor/*`), not arbitrary unmerged work.

---

## Role

You are a coding agent working in the Trading Bot repo. Your job is to **complete exactly one logical slice** from the next **`Open`** row in **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** — use **`agent_task`**, **`affected_files`**, and optional **`docs_refs`** from that row only (do not read the full [`QUEUE.MD`](QUEUE.MD) or [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) unless **`docs_refs`** points you there). **Run validation**, **open a pull request**, **merge the approved work into `main`**, and **delete merged feature branches** locally and on `origin`.

**Session start (mandatory):** Read [`README.md`](../README.md) in full, then read [`AGENTS.md`](../AGENTS.md) in full — **every** new run or thread, including after context resets. Then follow `AGENTS.md` (Kraken-only market data policy, risk/signing paths, no secrets in commits, no MLflow auto-promotion, avoid unrelated refactors).

---

## Inputs (fill before each run)

| Field | Description |
|-------|-------------|
| **Slice** | Queue IDs to implement in this run (e.g. `FB-AP-001`, `FB-AP-002`) or one **Batch** (e.g. `AP-PAM`). |
| **Branch** | New branch name: `cursor/<short-description>-<suffix>` (cloud agents: suffix per org policy, e.g. `-e182`). |
| **Base** | Default: `main`. |

---

## Phase 1 — Plan and branch

0. Already done mandatory read of [`README.md`](../README.md) and [`AGENTS.md`](../AGENTS.md) for this session (see **Role**).
1. From repo root run **`python3 scripts/print_next_queue_item.py`** — stdout is the full next **`Open`** row (smallest **`stack_order`**); use **`--json`** if you need machine parsing. Alternatively read **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** only — **not** [`QUEUE.MD`](QUEUE.MD) in full. If output is **`QUEUE_EMPTY:`** or **`id`** is **`_QUEUE_EMPTY_`**, **stop** — report empty stack and point to [**§6** — How to add or close an item](QUEUE.MD#6-how-to-add-or-close-an-item). Implement using **`agent_task`** + **`affected_files`** from that row. When the operator asks for **HIGH** priority only: if the printed row is not HIGH, **stop** or re-sort work per operator (script prints the stack order; filter CSV manually for HIGH-only).
2. `git fetch origin main && git checkout main && git pull origin main`
3. `git checkout -b <Branch>` from updated `main`.
4. If the slice depends on another row (e.g. registry before guards), either include that dependency in the same slice or stop and report.

---

## Phase 2 — Implement

1. Implement **only** the agreed queue slice; match existing patterns in **`affected_files`** from the CSV row.
2. **Before any merge to `main`** (on your feature branch): close out the queue item — update **`QUEUE_STACK.csv`** (**`status`** → `Done` or remove row; **or** edit **`ROWS`** in **`scripts/generate_queue_stack.py`** and run **`python scripts/generate_queue_stack.py`** — no manual **`stack_order`** renumbering in Python), mirror status into **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** if that ID has a §2 table row, and refresh the small snapshot in [`QUEUE.MD`](QUEUE.MD) §2 if needed. This is **not** optional for a completed slice: GitHub is updated **only after** this bookkeeping is done and pushed (see Phase 6).
3. **Do not** commit secrets; **do not** weaken tests to pass.

---

## Phase 3 — Validate (mandatory before push)

From repo root (Python ≥ 3.11, dev extras installed):

```bash
pip install -e ".[dev]"
ruff check .
python3 -m pytest tests/ -q
bash scripts/ci_spec_compliance.sh
python3 scripts/ci_queue_consistency.py
bash scripts/ci_pip_audit.sh
bash scripts/ci_bandit.sh
bash scripts/ci_mlflow_promotion_policy.sh
```

**Scope:** If full `pytest` is too slow for a tiny doc-only change, run **targeted** tests for touched modules (e.g. `python3 -m pytest tests/test_foo.py -q`). For **any** code change under `data_plane/ingest/`, **always** run `ci_spec_compliance.sh`. For forecaster/risk/execution changes, prefer **full** pytest when feasible.

**Gate:** Do **not** open a PR if `ruff` or required tests fail, unless the task explicitly documents an exception.

---

## Phase 4 — Commit and push

Include **all** slice work **and** queue closure from Phase 2 (stack + archive + `QUEUE.MD` snapshot if needed) in commits before you merge.

```bash
git add -A
git status   # verify no secrets / no accidental files
git commit -m "<type>: <queue-id(s)> <concise description>"
git push -u origin <Branch>
```

Retry push on transient network errors (bounded retries with backoff).

---

## Phase 5 — Pull request

1. Use GitHub **CLI** or the hosting UI:
   - `gh pr create --base main --head <Branch> --title "..." --body "..."`  
   Or the repository’s **PR tool**, with:
   - **Title:** imperative, includes queue IDs (e.g. `FB-AP-001: per-asset manifest schema`).
   - **Body:** what changed, why, queue reference, validation commands run and results, risks/follow-ups.
2. If the project uses **draft PRs** by default, keep draft until validation is green; then mark ready.
3. If queue updates were added **after** the first push, commit and **push again** so the PR shows the final **`QUEUE_STACK.csv`** / **`QUEUE_ARCHIVE.MD`** / snapshot state before you merge.

---

## Phase 6 — Merge to `main` (only after queue is closed on the branch)

**Order:** Implementation → validation (Phase 3) → **queue fully updated and archived on the branch** (Phase 2) → committed and pushed (Phase 4) → PR open and green (Phase 5) → **then** update GitHub’s default branch by merging.

Do **not** run **`gh pr merge`** until **`QUEUE_STACK.csv`**, **`QUEUE_ARCHIVE.MD`**, and any **`QUEUE.MD`** §2 snapshot edits for this slice are **committed and pushed** to the PR branch, and **`python3 scripts/ci_queue_consistency.py`** passes locally.

**One-step merge + delete head branch (GitHub CLI):** With `gh` installed and authenticated, from repo root (PR branch checked out) **after** the steps above:

```bash
gh pr merge --merge --delete-branch
```

Or: `gh pr merge <PR#> --merge --delete-branch`. Use `gh pr ready` first if the PR is draft. This merges with a merge commit and deletes the remote feature branch; then `git fetch origin --prune` and `git checkout main && git pull`, and `git branch -d <Branch>` locally if the branch still exists.

**Local merge (if allowed and PR is not used):**

```bash
git checkout main
git pull origin main
git merge --no-ff <Branch> -m "Merge <Branch>: <slice summary>"
git push origin main
```

**Do not** force-push to `main`. **Do not** merge unrelated branches.

---

## Phase 7 — Prune branches

After the slice is **merged into `main`**:

1. **Local:** `git branch -d <Branch>` (the slice branch).
2. **Remote:** `git push origin --delete <Branch>` (ignore error if already deleted).
3. **Optional cleanup:** List remote branches `git branch -r`, delete other **merged** `cursor/*` branches that are fully incorporated into `main` (use `git branch -r --merged main` to candidates). **Never** delete `main` or **unmerged** work without explicit instruction.

```bash
git fetch origin --prune
git branch -vv
```

---

## Phase 8 — Handoff (required)

Report:

1. **What changed** (one paragraph).
2. **Queue IDs** addressed and **status** (Open → Done in **`QUEUE_STACK.csv`** + **`QUEUE_ARCHIVE.MD`** if applicable).
3. **Files touched** (list).
4. **Validation** (exact commands and pass/fail).
5. **PR** URL and merge commit on `main`.
6. **Branches deleted** (names).

---

## Failure modes

| Situation | Action |
|-----------|--------|
| Tests fail | Fix or narrow scope; do not merge. |
| Merge conflict on `main` | Rebase or merge `main` into slice branch, resolve, re-run tests, push. |
| Slice too large | Stop after one PR; schedule follow-up slice. |
| “Merge all branches” requested | **Clarify:** only merge **this slice’s branch** + prune **merged** feature branches — not every remote branch. |

---

## One-shot copy-paste prompt for GPT / Cursor Agent

```
You are working in the Trading Bot repo per AGENTS.md.

Task: Implement queue slice: <INSERT IDs OR BATCH>.
Branch: cursor/<desc>-<suffix> from latest main.

Steps:
0) python3 scripts/print_next_queue_item.py  (or read QUEUE_STACK.csv for the next Open row)
1) Fetch/pull main; create branch.
2) Implement the slice only; touch paths from the CSV row **affected_files**.
3) Close queue on branch: QUEUE_STACK.csv + QUEUE_ARCHIVE.MD + QUEUE.MD snapshot if needed; python3 scripts/ci_queue_consistency.py
4) Run: ruff check . ; pytest (full or targeted per change) ; scripts/ci_spec_compliance.sh ; scripts/ci_mlflow_promotion_policy.sh
5) Commit (include queue closure); push -u origin <branch>
6) Open PR to main with title/body referencing queue IDs and validation; push again if queue docs changed after first push
7) Only after queue closure is on the remote branch: gh pr merge --merge --delete-branch (or UI); then git fetch origin --prune, delete local branch if needed
8) Handoff: summary, files, validation, PR URL, branches deleted

Do not merge unrelated branches. Do not force-push main. No secrets in commits.
```

Replace `<INSERT IDs OR BATCH>` and branch name before running.

---

**References:** [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) (queue system) · [`QUEUE_STACK.csv`](QUEUE_STACK.csv) · [`QUEUE.MD`](QUEUE.MD) · [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) · [`AGENTS.md`](../AGENTS.md) · [`README.md`](../README.md)
