# docs/plans/

Formal plans for the Python → Rust trading platform refactor. Each plan file is a self-contained, executable document with a stable task-ID structure, acceptance criteria, and phase exit criteria.

> **Start at [`ROADMAP.md`](./ROADMAP.md).** It is the one map from today's code to the North Star
> in [`../artifact.md`](../artifact.md), and it is the authority on which set runs when.
> [`EXECUTION-BOARD.md`](./EXECUTION-BOARD.md) is the live track assignment for agents working now.
>
> The A→B→…→7 phase ordering below applies to **Sets A and B only** (the Python → Rust rewrite),
> which Set B completed. The index rows marked `Pending` for Set A phases are historical: Set B
> superseded them. Everything from Set H onward is sequenced by the roadmap, not by that ordering.

All plan sets (A through J) live under `plan-sets/` in this directory. Set H
(`plan-sets/set-H/`) specifies the **AI Model Studio** — the end-state design for
creating, training, testing, evaluating, versioning, and deploying the models the
trading system uses. Set I (`plan-sets/set-I/`) builds the **probabilistic
forecasting core** on top of that spine — distributional outputs, walk-forward CV,
proper-scoring evaluation (CRPS/calibration/VaR), ensembles, the pipeline factory,
and forecast-quality monitoring — derived from
[`specs/AI_MODELS_SUITE_CAPABILITY_SPEC.md`](../specs/AI_MODELS_SUITE_CAPABILITY_SPEC.md).
Set J (`plan-sets/set-J/`) builds the **Backtest Suite honest-evaluation core** on
top of the existing `crates/backtest` Run engine — the Run/Study/Experiment object
model (content-addressed runs, sealed distributions, a monotonic trial counter, a
one-shot holdout vault), the Null Library, and the staged-gate funnel (0→4) — derived
from [`specs/BACKTEST_SUITE_CORE_SPEC.md`](../specs/BACKTEST_SUITE_CORE_SPEC.md).
Set L (`plan-sets/set-L/`) builds the **quant research agent's foundation** — the
Claude Agent SDK runtime in a per-project container, the LLM proxy, a durable job
service and artifact store, Data API v2 with the research cutoff, the `tbot` read
surface, the workspace UI v1 and the eval-suite skeleton — derived from
[`specs/AGENT-001-agent-runtime.md`](../specs/AGENT-001-agent-runtime.md) and its
siblings. Its **Phase 0 is independent and urgent**: it replaces the `market_bars`
table, whose sorting key silently destroys bars, and it must land before any bar
backfill. Set L also absorbs the two unshipped legs of Set K.

---

## What Belongs Here

- Formal phase plans and the master plan
- The execution record linking each plan to the specs and ADRs it implements

## What Does Not Belong Here

- Architecture decisions — record those in `adr/`
- Specifications — record those in `specs/`
- Research that informed the plans — record that in `research/`

---

## Index

| Plan | Type | Status | Derived From |
|------|------|--------|--------------|
| [ROADMAP.md](./ROADMAP.md) | Formal | **Current — read first** | artifact.md North Star; supersedes BS-007 17_BUILD_ORDER §2–§3 |
| [EXECUTION-BOARD.md](./EXECUTION-BOARD.md) | Formal | Current | ROADMAP.md; the live parallel-track assignments |
| [plan-sets/set-S/MASTER.md](./plan-sets/set-S/MASTER.md) | Formal | Current | AGENT-005…014, COMP-007, ADR-0034…0038 |
| [plan-sets/set-P/MASTER.md](./plan-sets/set-P/MASTER.md) | Formal | Current | DATA-005 §7/§8/§11/§12, 06_DATA §6, ADR-0027 |
| [plan-sets/set-T/MASTER.md](./plan-sets/set-T/MASTER.md) | Formal | Current | SPEC §12.3, ADR-P2-14/27, COMP-002, COMP-007 |
| [rust-rewrite-master-plan.md](./reference/rust-rewrite-master-plan.md) | Formal | Current | SYS-001, all ADRs |
| [phase-A-documentation.md](./plan-sets/set-A/phase-A-documentation.md) | Formal | Current | All specs/ADRs |
| [phase-B-bootstrap.md](./plan-sets/set-A/phase-B-bootstrap.md) | Formal | Pending | SYS-001, ADR-0001 |
| [phase-0-foundations.md](./plan-sets/set-A/phase-0-foundations.md) | Formal | Pending | DATA-001, DATA-002, DATA-003, DATA-004, ADR-0002, ADR-0007 |
| [phase-1-spine.md](./plan-sets/set-A/phase-1-spine.md) | Formal | Pending | COMP-001, COMP-004, ADR-0003, ADR-0006 |
| [phase-2-money-safety.md](./plan-sets/set-A/phase-2-money-safety.md) | Formal | Pending | COMP-002, ADR-0005, ADR-0006 |
| [phase-3-ui-streaming.md](./plan-sets/set-A/phase-3-ui-streaming.md) | Formal | Pending | COMP-003, ADR-0011 |
| [phase-4-strategies.md](./plan-sets/set-A/phase-4-strategies.md) | Formal | Pending | FEAT-001, DATA-004, ADR-0007, ADR-0008 |
| [phase-5-front-doors.md](./plan-sets/set-A/phase-5-front-doors.md) | Formal | Pending | DATA-004, INTG-001, ADR-0010 |
| [phase-6-second-asset.md](./plan-sets/set-A/phase-6-second-asset.md) | Formal | Pending | COMP-001, COMP-002, ADR-0006 |
| [phase-7-cutover.md](./plan-sets/set-A/phase-7-cutover.md) | Formal | Pending | All specs, all ADRs |
