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
7. Update queue artifacts:
   - set row status or adjust queue per `docs/QUEUE.MD` §6
   - mirror queue status in `docs/QUEUE_ARCHIVE.MD` when applicable
   - refresh snapshots in `docs/QUEUE.MD` if open counts change
8. Commit changes to the current branch.
9. Open a PR to `main` when required; after the slice is done and queue docs are updated on the branch, prefer **`gh pr merge --merge --delete-branch`** (with `gh` authenticated) to merge and remove the remote head branch in one step — see [`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`](../../../docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD) Phase 6 and [`AGENTS.md`](../../../AGENTS.md) section 7.
10. Stop and report completion of that single item.

## Constraints

- Never batch multiple Open queue items into one implementation pass unless explicitly instructed.
- Do not treat old docs as canonical if canonical docs conflict.
- Keep changes scoped to the chosen queue item.
- Ensure the commit message names the queue ID.

