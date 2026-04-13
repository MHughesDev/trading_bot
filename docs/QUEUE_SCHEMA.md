# Queue system — portable schema (any repository)

Copy these three artifacts together when reusing the pattern:

| File | Role |
|------|------|
| **`QUEUE.MD`** | Short **agent protocol** + **conventions** (Kind, IDs, how to add items). Keep under ~200 lines. |
| **`QUEUE_STACK.csv`** | **Machine-readable next tasks** — one row per actionable item; required **`agent_task`** (self-contained instructions). |
| **`QUEUE_ARCHIVE.MD`** (optional) | **Human-readable history** — large tables; agents skip unless auditing. |

**Minimum viable:** `QUEUE.MD` + `QUEUE_STACK.csv` only; fold archive tables into `QUEUE.MD` if you want a single file (larger agent cost).

**CSV columns (this repo):** see [`QUEUE.MD` §0](QUEUE.MD#0-next-task-stack-queue_stackcsv).

**Regenerator:** `scripts/generate_queue_stack.py` — optional; edit the `ROWS` list or maintain CSV by hand.
