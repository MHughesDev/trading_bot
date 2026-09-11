# ADR-0030: One durable job service and a content-addressed artifact store

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes (with Claude)

## Context

Long-running work is handled ad hoc:
- the backtest manager (fixed 3-concurrent cap);
- sweeps and suites held in memory (FEAT-003 deferral);
- asset-init jobs;
- training runs.

A restart orphans compute and can double-count trials. The agent under ADR-0024
runs for hours and must wait on many kinds of slow work without polling. It needs
a handle to every large output.

## Decision

1. **One job service** (Postgres-backed: `jobs`, `job_events`, `job_artifacts`)
   for:
   - backtests, sweeps, studies, gate advances;
   - dataset builds, feature materialisation;
   - training, HPO, prediction series;
   - simulations, backfills, `data_qc`, profiles;
   - skill verification and eval tasks.
   Workers lease jobs and heartbeat. Queues (`agent | human | system`) share
   capacity by weighted fair share.
2. **Idempotent submission by manifest hash** within a project. A retried or
   duplicate submission returns the existing job and never a second trial.
3. **Evaluation-counted kinds are registered with the evaluation service at
   submission** (INV-1), whatever the client.
4. **Event-driven waiting:** state changes go over NATS → SSE. `wait` and `watch`
   block on events and never poll.
5. **Artifacts are content-addressed** (`art_<sha256>`) on the existing
   `ArtifactStore` backends (`crates/storage/src/artifacts`, fs/s3). Each carries a
   manifest (producer, inputs, query, schema, cutoff, `data_qc` grade, code
   hashes). Cited artifacts are pinned.

## Rationale

- One lifecycle makes everything slow durable, observable and budgetable.
- Idempotency closes the duplicate-trial hazard from compaction or resume.
- Content addressing makes every reported number reproducible.

## Consequences

- The backtest manager, sweep state, suite state and asset-init jobs migrate onto
  the service. The fixed backtest cap is removed.
- The existing xxh3 integrity hash stays. Handles use sha256.

## Alternatives Considered

- **Per-feature job tables** (status quo). Duplicated lifecycle and no common wait.
- **An external workflow engine** (Temporal etc.). Heavier to operate than the
  need; this can be revisited.

## References

- COMP-005 (planned)
- BS-007 [05_JOBS_AND_ARTIFACTS](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/05_JOBS_AND_ARTIFACTS.MD)
