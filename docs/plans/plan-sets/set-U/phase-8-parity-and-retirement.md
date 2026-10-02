# Phase 8 — Parity, proven; old paths retired

**Completion: 0% (0 / 5 tasks)**

**Requirement IDs:** SU-PAR-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §11, §12, §15; ADR-0047.
**Depends on:** Phases 3, 4, 7.
**Blocks:** nothing — this closes the set.
**Migration range:** none.

Sharing code makes parity true; this phase makes it *measured*, and then removes every evaluator that is not the engine.

---

### U-8.1 Replay harness — M
`NOT STARTED`

Feed a paper journal's recorded inputs — bars, portfolio views, model responses — through the engine as the backtest environment builds it; compare decisions byte for byte; report the first differing step.

**Files:** `crates/backtest/src/replay.rs`.
**Acceptance:** a journal replays identically; a deliberately perturbed engine fails naming the step and the field.
**Traces:** ADR-0047 decision 2; QA-1.

### U-8.2 Nightly parity job — S
`NOT STARTED`

Run the harness nightly over the previous day's paper journals; a difference raises the same severity as a feature-consistency code drift.

**Files:** `apps/platform/src/parity_jobs.rs`.
**Acceptance:** the job is registered at boot (source-scan guard, the pattern of `fill_stream_tests`); a seeded difference produces an alert.
**Traces:** ADR-0047.

### U-8.3 Retire the old evaluators — M
`NOT STARTED`

Delete `sim.rs`'s evaluation loop, `preview.rs`'s loop, `interpreter.rs` and hot-path stage 3's `StrategyInstance` once every stored strategy runs on v3. A source-scan guard fails the build if any reappears.

**Files:** `crates/backtest/src/`, `crates/strategy-runtime/`, `apps/platform/src/hot_path.rs`.
**Acceptance:** `strategy-engine` is the only crate that evaluates a strategy (guard); the workspace builds and every suite passes without the retired code.
**Traces:** ADR-0039; FEAT-007 §15 criterion 1.

### U-8.4 Retire the duplicate grammars — M
`NOT STARTED`

Remove `strategy-validator`'s expression parser and the runtime bytecode compiler as grammar implementations; the domain EBNF becomes documentation of v1 only; DATA-004, FEAT-001 and FEAT-005 are updated as-built.

**Files:** `crates/strategy-validator/`, `crates/strategy-runtime/src/bytecode.rs`, `docs/specs/`.
**Acceptance:** exactly one parser for strategy expressions exists in the workspace (guard); the as-built specs describe the engine that runs.
**Traces:** ADR-0045.

### U-8.5 Migrate stored strategies — M
`NOT STARTED`

Translate every stored strategy, re-run its latest backtest on the engine, and publish old-versus-new with the translator's behaviour-change list, so no result changes silently.

**Files:** `crates/api/src/bin/migrate_strategies_v3.rs`, `docs/reports/`.
**Acceptance:** every stored strategy has a v3 revision or a named refusal; every re-run's difference is attributed to a listed behaviour change.
**Traces:** FEAT-007 §12.
