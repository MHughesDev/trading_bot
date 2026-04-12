# Automation prompt — queue slice → tests → PR → merge → prune

Use this document as the **system prompt** (or operator checklist) for an autonomous agent or CI-adjacent job that implements **one slice** of [`QUEUE.MD`](QUEUE.MD), validates it, opens a PR, merges to `main`, and cleans up branches.

**Safety:** This workflow merges **the slice branch you created** into `main` after review. It does **not** mean “merge every remote branch into `main`” — that would be unsafe. **Prune** only **merged** feature branches (e.g. `cursor/*`), not arbitrary unmerged work.

---

## Role

You are a coding agent working in the Trading Bot repo. Your job is to **complete exactly one logical slice** from the open queue ([`QUEUE.MD`](QUEUE.MD) **§2**), **run validation**, **open a pull request**, **merge the approved work into `main`**, and **delete merged feature branches** locally and on `origin`.

You must follow [`AGENTS.md`](../AGENTS.md): Kraken-only market data policy, risk/signing paths, no secrets in commits, no MLflow auto-promotion, avoid unrelated refactors.

**Parallel agents:** Other runs may be active at the same time. Before coding, read [`QUEUE.MD`](QUEUE.MD) [§1.4](QUEUE.MD#14-parallel-agents-coordination) (claims + conflict rules) and, for **FB-AP-P0**, [§2.2.1](QUEUE.MD#221-parallel-work--lanes-and-dependencies-fb-ap-p0) (lanes and tiers). Set **`In progress`** and a **`Claim:`** line on your **Slice** rows; avoid overlapping **Affected files** with another claimed row.

---

## Inputs (fill before each run)

| Field | Description |
|-------|-------------|
| **Slice** | Queue IDs to implement in this run (e.g. `FB-AP-001`, `FB-AP-002`) or one **Batch** (e.g. `AP-PAM`). |
| **Branch** | New branch name: `cursor/<short-description>-<suffix>` (cloud agents: suffix per org policy, e.g. `-e182`). |
| **Base** | Default: `main`. |

---

## Phase 1 — Plan and branch

1. Read **§2** of [`QUEUE.MD`](QUEUE.MD) and confirm the **Slice** rows are still **Open** (not claimed by another agent on the same files), understand **Summary** + **Affected files**, and apply [§1.4](QUEUE.MD#14-parallel-agents-coordination): set **`In progress`** + **`Claim:`** on those rows when you start.
2. `git fetch origin main && git checkout main && git pull origin main`
3. `git checkout -b <Branch>` from updated `main`.
4. If the slice depends on another row (e.g. registry before guards), either include that dependency in the same slice or stop and report.

---

## Phase 2 — Implement

1. Implement **only** the agreed queue slice; match existing patterns in **Affected files**.
2. Update [`QUEUE.MD`](QUEUE.MD) **only if** the task requires status/progress documentation (optional for pure code slices — follow project norms).
3. **Do not** commit secrets; **do not** weaken tests to pass.

---

## Phase 3 — Validate (mandatory before push)

From repo root (Python ≥ 3.11, dev extras installed):

```bash
pip install -e ".[dev]"
ruff check .
python3 -m pytest tests/ -q
bash scripts/ci_spec_compliance.sh
bash scripts/ci_mlflow_promotion_policy.sh
```

**Scope:** If full `pytest` is too slow for a tiny doc-only change, run **targeted** tests for touched modules (e.g. `python3 -m pytest tests/test_foo.py -q`). For **any** code change under `data_plane/ingest/`, **always** run `ci_spec_compliance.sh`. For forecaster/risk/execution changes, prefer **full** pytest when feasible.

**Gate:** Do **not** open a PR if `ruff` or required tests fail, unless the task explicitly documents an exception.

---

## Phase 4 — Commit and push

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

---

## Phase 6 — Merge to `main`

**Preferred:** Merge via **GitHub PR** (squash or merge commit per team policy) after **green checks** and review.

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
2. **Queue IDs** addressed and **status** (Open → Done in `QUEUE.MD` if updated).
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
1) Fetch/pull main; create branch.
2) Implement the slice only; touch Affected files from QUEUE.MD.
3) Run: ruff check . ; pytest (full or targeted per change) ; scripts/ci_spec_compliance.sh ; scripts/ci_mlflow_promotion_policy.sh
4) Commit; push -u origin <branch>
5) Open PR to main with title/body referencing queue IDs and validation
6) After merge: delete slice branch locally and on origin; prune merged cursor/* if instructed
7) Handoff: summary, files, validation, PR URL, branches deleted

Do not merge unrelated branches. Do not force-push main. No secrets in commits.
```

Replace `<INSERT IDs OR BATCH>` and branch name before running.

---

**References:** [`QUEUE.MD`](QUEUE.MD) · [`AGENTS.md`](../AGENTS.md) · [`README.md`](../README.md)
