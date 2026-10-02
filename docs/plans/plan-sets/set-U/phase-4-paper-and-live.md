# Phase 4 — Paper and live

**Completion: 0% (0 / 6 tasks)**

**Requirement IDs:** SU-LIVE-01…06.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §9.5, §11; ADR-0038, ADR-0039, ADR-0042, ADR-0047.
**Depends on:** Phases 1 and 2.
**Blocks:** Phase 8; Set T Phase 1 (deployment record).
**Migration range:** 0141–0143.

The same engine, a different clock and a different venue. This phase is also where paper deployments are finally *recorded* — the missing writer that has kept forward evidence at zero.

---

### U-4.1 LiveRunner — L
`NOT STARTED`

Per deployment: compile, warm up from `BarStore`, subscribe to the live bus for every declared (instrument, timeframe), step the engine on decision ticks only (ADR-0042), route through the risk gate and execution engine to the venue, feed fills and order updates back.

**Files:** `apps/platform/src/live_runner.rs`.
**Acceptance:** an `ema(close, 7) > ema(close, 21)` strategy deployed to paper fires from live bars alone — no injected feature values (the defect FEAT-007 §1 #2 names); the engine is stepped once per closed bar, not per tick (counted).
**Traces:** FEAT-007 §9.5; ADR-0039; ADR-0042.

### U-4.2 Deployment record — M
`NOT STARTED`

Starting a deployment writes the deployment row (revision, target, environment, starting capital, pinned model versions, who started it); stopping closes it. This is the writer `mlops.paper_deployment` never had, shaped so Set T's proposal path can sit in front of it.

**Files:** `crates/storage/src/postgres/deployments.rs`, `crates/api/src/routes/deployments.rs`, migration 0141.
**Acceptance:** every running deployment has a row and every row a running or closed deployment; starting capital is recorded and read by reconciliation; Set T's `paper_deployment` readers find rows.
**Traces:** FEAT-007 §1 defect 10, §9.5; Set T T-1.1, T-1.3.

### U-4.3 Asynchronous model resolution — M
`NOT STARTED`

The LiveRunner resolves `NeedModels` concurrently through the model provider with each contract's timeout; a timeout resumes with `missing(model_timeout)`. The hot path never blocks on a model.

**Files:** `apps/platform/src/live_runner.rs`.
**Acceptance:** a model that takes longer than its timeout yields a `hold` with `model_timeout` in the journal, on time; two models in one layer are requested concurrently (timed).
**Traces:** FEAT-007 §8.2, §8.4; ADR-0043.

### U-4.4 Checkpoint and restore — M
`NOT STARTED`

Engine state (temporal operators, warm-up position, OCO bookkeeping) checkpoints after each decision; a restart restores and resumes instead of re-entering.

**Files:** `crates/strategy-engine/src/checkpoint.rs`, migration 0142.
**Acceptance:** kill the process mid-position, restart, and the next decision emits no duplicate entry (a test); a checkpoint from a different plan hash is refused.
**Traces:** FEAT-007 §9.5; ADR-0041.

### U-4.5 The decision journal — M
`NOT STARTED`

`strategy_decision_journal` stores one record per decision per deployment — input digest, model requests and responses, intents, planned orders, gate verdicts, missing reasons — with retention per the data budget.

**Files:** `crates/storage/src/postgres/decision_journal.rs`, migration 0143.
**Acceptance:** every LiveRunner decision writes exactly one record; a record carries enough to replay the step (asserted by Phase 8's harness); journal write failure is surfaced, never swallowed.
**Traces:** ADR-0047.

### U-4.6 v3 deployments through the one live registry — M
`NOT STARTED`

ADR-0038's registry declares v3 deployments; the LiveRunner is what runs them. v1 strategies keep hot-path stage 3 until Phase 8.

**Files:** `crates/strategy-runtime/src/live.rs`, `apps/platform/src/hot_path.rs`.
**Acceptance:** a v3 deployment is never also evaluated by stage 3 (asserted); stopping a deployment stops its runner within one decision tick.
**Traces:** ADR-0038; ADR-0039.
