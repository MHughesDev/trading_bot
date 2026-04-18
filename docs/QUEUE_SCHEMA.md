# Queue system — portable schema (any repository)

**Documentation last reviewed:** **2026-04-18** (`queue_top.sh` / `queue_close.sh` agent workflow — list order only in generator).

---

## What “queue system” means in this repo

The **queue system** is **all artifacts that define, store, and operate the work backlog machinery** — not only `QUEUE.MD`. When editing backlog **process**, **schema**, or **next-task** behavior, touch every relevant file below so nothing drifts.

**Agent session contract (repo-wide, not queue-only):** [`README.md`](../README.md) + [`AGENTS.md`](../AGENTS.md) must be read at the start of every agent session; **[`.cursorrules`](../.cursorrules)** (if present) reinforces that for Cursor. Queue steps in [`QUEUE.MD`](QUEUE.MD) step **0** match this.

**Agent workflow (token-efficient):** Do **not** load **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** or **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD)** in full to pick or close work. Run **`bash scripts/queue_top.sh`** (or `python3 scripts/print_next_queue_item.py`) for the next task; after implementation, run **`bash scripts/queue_close.sh --next`** (or `--id <ID>`) to mark **`Done`** and regenerate the CSV. Use **`docs_refs`** only when the task points you at specific docs.

**Template parity:** The canonical queue layout and filenames match the [MHughesDev/trading_bot](https://github.com/MHughesDev/trading_bot) template on GitHub (`docs/QUEUE*.MD`, `docs/QUEUE_STACK.csv`, `scripts/generate_queue_stack.py`, `scripts/ci_queue_consistency.py`, Cursor skills under `.cursor/skills/`). When changing the portable schema, keep this repo aligned with that source unless you intentionally fork behavior.

| Artifact | Role |
|----------|------|
| [`QUEUE.MD`](QUEUE.MD) | **Agent protocol** + **conventions** (Kind, IDs, how to add/close items). Small file; read for rules, not full history. |
| [`QUEUE_STACK.csv`](QUEUE_STACK.csv) | **Next-task stack** — machine-readable; **`agent_task`** per row; canonical for **which** task runs next. |
| [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) | **Narrative tables** — open detail, `IL-*`, `HG-*`, completed `FB-*` archive. |
| [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) | **This file** — portable schema + queue-system index. |
| [`CANONICAL_SPEC_INDEX.MD`](CANONICAL_SPEC_INDEX.MD) | **APEX canonical** specs index + precedence vs as-built (`docs/Specs/`) — program **FB-CAN-002**. |
| [`app/config/canonical_config.py`](../app/config/canonical_config.py) | **APEX canonical runtime config** (`CanonicalRuntimeConfig`) — **FB-CAN-003**; optional YAML `apex_canonical` merged over legacy projection. |
| [`decision_engine/trigger_engine.py`](../decision_engine/trigger_engine.py) | **APEX three-stage trigger** — **FB-CAN-005**; `TriggerOutput` in `ForecastPacket.forecast_diagnostics["trigger"]`. |
| [`decision_engine/auction_engine.py`](../decision_engine/auction_engine.py) | **APEX opportunity auction** — **FB-CAN-006**; `AuctionResult` in `ForecastPacket.forecast_diagnostics["auction"]`. |
| [`risk_engine/canonical_sizing.py`](../risk_engine/canonical_sizing.py) | **APEX canonical risk sizing** — **FB-CAN-007**; `RiskState.last_risk_sizing` after `RiskEngine.evaluate`. |
| [`execution/execution_logic.py`](../execution/execution_logic.py) | **APEX execution guidance** — **FB-CAN-008**; `OrderIntent.metadata.execution_guidance`. |
| [`app/contracts/replay_events.py`](../app/contracts/replay_events.py) | **Canonical replay run + events** — **FB-CAN-009**; `ReplayRunContract`, `ReplayEventEnvelope`. |
| [`AUTOMATION_QUEUE_SLICE_PROMPT.MD`](AUTOMATION_QUEUE_SLICE_PROMPT.MD) | Agent workflow: one slice → validate → PR → merge. |
| [`.cursor/skills/add-to-queue/SKILL.md`](../.cursor/skills/add-to-queue/SKILL.md) | Cursor **Add to Queue** skill for adding/updating items. |
| [`.cursor/skills/queue-one-at-a-time/SKILL.md`](../.cursor/skills/queue-one-at-a-time/SKILL.md) | Cursor **Queue One-at-a-Time** execution skill (take top Open row only, implement, validate, document, commit). |
| [`scripts/generate_queue_stack.py`](../scripts/generate_queue_stack.py) | Optional CSV **regenerator** (maintainer tool): edit the **`ROWS`** list **order** (append/reorder dicts), run **`python scripts/generate_queue_stack.py`** — **`stack_order`** is **auto** (**1…N**, sentinel **`9999`**); do **not** hand-edit numbers in Python. |
| [`scripts/ci_queue_consistency.py`](../scripts/ci_queue_consistency.py) | CI helper: **Open** rows in `QUEUE_STACK.csv` must appear in `QUEUE_ARCHIVE.MD` (see **FB-AUD-008**). |
| [`scripts/print_next_queue_item.py`](../scripts/print_next_queue_item.py) | **Agent helper:** print the next **`Open`** row (smallest **`stack_order`**) as one terminal string; optional **`--json`**. |
| [`scripts/queue_top.sh`](../scripts/queue_top.sh) | **Shell alias** for agents: same as `python3 scripts/print_next_queue_item.py` — **grab the top Open row** without opening the CSV in an editor. |
| [`scripts/close_queue_item.py`](../scripts/close_queue_item.py) | **Agent helper:** mark an item **`Done`** in `scripts/generate_queue_stack.py`, run **`generate_queue_stack.py`**, optionally flip **`Open` → `Done`** in a matching **`QUEUE_ARCHIVE.MD`** table row. |
| [`scripts/queue_close.sh`](../scripts/queue_close.sh) | **Shell alias:** `bash scripts/queue_close.sh --next` or `--id <ID>` — **move / archive closure** without loading the full CSV or archive. |

**Audit → backlog (optional):** [`docs/FULL_AUDIT.md`](FULL_AUDIT.md) **§8** audit report · [`.cursor/skills/draft-audit-report`](../.cursor/skills/draft-audit-report/SKILL.md) · [`.cursor/skills/audit-report-to-queue`](../.cursor/skills/audit-report-to-queue/SKILL.md) · [`docs/BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD`](BRAINSTORM/BS-006_AUDIT_TO_QUEUE_BRAINSTORM.MD)

**Related (not part of the core queue system but often updated together):** [`scripts/create_github_issues.sh`](../scripts/create_github_issues.sh) (optional GitHub mirror).

---

## Copying to another repository

Copy **`QUEUE.MD`** + **`QUEUE_STACK.csv`** together at minimum; add **`QUEUE_ARCHIVE.MD`** when you need full tables. See column definitions in [`QUEUE.MD` §0](QUEUE.MD#0-next-task-stack-queue_stackcsv).

**Minimum viable:** `QUEUE.MD` + `QUEUE_STACK.csv` only; fold archive tables into `QUEUE.MD` if you want a single file (higher token cost for agents).

**Regenerator:** maintain `scripts/generate_queue_stack.py` (**list order** = stack; regenerate CSV) or edit **`QUEUE_STACK.csv`** by hand (set **`stack_order`** explicitly).
