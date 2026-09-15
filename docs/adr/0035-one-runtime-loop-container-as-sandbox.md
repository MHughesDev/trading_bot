# ADR-0035: One runtime loop for every tier; the container becomes the code sandbox

**Status:** Proposed
**Date:** 2026-09-14
**Spec:** `docs/Specs/AGENT-010-agent-runtime.md`
**Supersedes:** ADR-0024 §1–§3 (the agent runs on the Claude Agent SDK in a per-project container). ADR-0024's container image, network isolation and credential handling are retained for a different purpose.
**Amends:** ADR-0022 (the internal agent's Rust loop is no longer "the interim"), ADR-0033 (unchanged in substance; its "turn = run" model now has one runtime underneath it)

## Context

ADR-0024 judged that rebuilding a coding-agent harness in Rust "would not beat existing harnesses" and moved the research agent into a Docker container on the Claude Agent SDK. Since then the platform built, for the local tier, exactly the harness that judgement said not to build: `crates/harness` has routed exposure, constrained decoding, budgets, a policy engine, provenance tagging, approvals, a canary, and a replayable trace, and it is the only path that ever ran a model end to end on this platform's real hardware (`docs/LOCAL_TIER_FINDINGS.md`). The container path, meanwhile, is Claude-only, its image is built by nothing, and its loop is the vendor's: the platform can refuse its requests at a proxy but cannot budget its context, route its tools, verify its claims, or replay its steps.

The user's requirement is a single harness usable from a 7B local model to a frontier model. Two loops cannot satisfy it; three certainly cannot.

## Decision

1. **The `harness::drive::Loop` is the runtime for every tier.** The hosted `driver.rs` is deleted after a flagged migration. The frontier tier is a profile with a `native` transport, prompt caching and larger budgets, on the same loop.
2. **The container is repurposed as the code sandbox.** `infra/agent-image` keeps its research stack, non-root user, read-only root and internal network, drops the Claude Agent SDK, and runs a small executor that answers `run_python` calls at the `code_execution: full` tier. The agent's judgement never runs inside it; its code does.
3. **Model adapters declare capabilities** and are verified against profiles at load; all adapters go through the LLM proxy for usage, cache TTL and budget refusal.
4. **The effect log is the run.** It is written ahead of execution, replayed on resume and by the evaluation suite.

## Consequences

- One code path to make honest; one set of evals; one place budgets are enforced. The "which driver am I on" branch in `manager.rs` disappears.
- The SDK's built-in conveniences (compaction, sub-agents, hooks) are replaced by the platform's own, which the specs AGENT-005/007/008/010 define — deterministic, budgeted and tier-aware where the SDK's are neither.
- Sessions on the frontier tier gain approvals, steering, suspension on jobs, resume after a crash, and escalation with state, none of which the hosted driver had.
- Reversal cost is bounded by the flag in AGENT-010 §13 step 3 and by keeping `driver.rs` for one release.
- AGENT-001 is superseded by AGENT-010; HARNESS_CONFORMANCE deviation 1 ("the execution loop is the Agent SDK's, not ours") is closed rather than tolerated.

## Alternatives considered

- **Keep both loops and share the assembler** (AGENT-005's own non-goal): stops the bleeding but leaves two termination contracts, two approval paths and two evaluation targets. Rejected because the requirement is one harness.
- **Make the SDK container the runtime for every tier via an OpenAI-compatible shim for local models:** the SDK does not expose grammar-constrained decoding or per-step exposure, so the local tier would lose the mechanisms that made it work. Rejected on the local-tier findings.
