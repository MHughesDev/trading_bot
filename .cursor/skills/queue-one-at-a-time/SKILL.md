---
name: queue-one-at-a-time
description: Execute exactly one queue item at a time from docs/QUEUE_STACK.csv, implement it end-to-end, update docs, and commit before moving to the next item.
---

# Queue One-at-a-Time Execution Skill

Use this skill when the user asks to execute queue work in sequence.

## Core rule

**One queue item per iteration.**

Do not start item N+1 until item N is fully completed, documented, validated, and committed.

## Required workflow

0. Read `README.md` and `AGENTS.md` in full at the start of this session (re-read `AGENTS.md` after any context reset).
1. Open `docs/QUEUE_STACK.csv`.
2. Select the **first** row where:
   - `status == Open`
   - lowest `stack_order`
3. Read only files needed by that row:
   - `affected_files` first
   - then `docs_refs` if needed
4. Implement exactly that row’s `agent_task`.
5. Run relevant validation commands (and required repo checks when applicable).
6. Update docs impacted by the change (`README.md`, `docs/*.MD`, runbooks/spec pointers as needed).
7. Update queue artifacts (full closure before GitHub merge):
   - set row status or adjust queue per `docs/QUEUE.MD` §6
   - mirror queue status in `docs/QUEUE_ARCHIVE.MD` when applicable
   - refresh snapshots in `docs/QUEUE.MD` if open counts change
   - run `python3 scripts/ci_queue_consistency.py` and fix until OK
8. Commit and push **including** queue closure to the PR branch.
9. Open or update the PR to `main` when required; **only after** queue closure is pushed to the remote branch, run **`gh pr merge --merge --delete-branch`** (with `gh` authenticated) — see [`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`](../../../docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD) Phase 6 and [`AGENTS.md`](../../../AGENTS.md) section 7.
10. Stop and report completion of that single item.

## Constraints

- Never batch multiple Open queue items into one implementation pass unless explicitly instructed.
- Do not treat old docs as canonical if canonical docs conflict.
- Keep changes scoped to the chosen queue item.
- Ensure the commit message names the queue ID.

