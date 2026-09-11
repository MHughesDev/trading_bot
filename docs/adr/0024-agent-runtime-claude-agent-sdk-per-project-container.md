# ADR-0024: Agent runtime — Claude Agent SDK in a per-project container, behind an LLM proxy

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes (with Claude)
**Supersedes (in part):** ADR-0022 §internal agent loop (the MCP thin-client decision stands)

## Context

ADR-0022 gave the platform an internal agent: a Rust loop in `crates/api/src/agent/`
that resends the whole transcript each turn over 38 MCP tools, with no prompt
caching, resume, sub-agents, files or code execution (BS-007 01 §8). BS-007 set
the goal of a single, long-running quant research agent that analyses data in
code, trains models, runs jobs for hours and learns skills. That is a
coding-agent problem. Rebuilding a coding-agent harness in Rust would be a large
project that would not beat existing harnesses. The user chose frontier models
first (BS-007 D-01), a sandboxed container per project (D-02) and the Claude
Agent SDK as the harness (D-07).

## Decision

1. **Harness.** The agent runs on the **Claude Agent SDK (Python)** inside a small
   `agent-host` process. It gets the SDK's built-in file, bash, grep, sub-agent,
   skill, compaction and hook machinery.
2. **One container per research project.** The container:
   - is built from a versioned image (`tbot-agent:<semver>`);
   - is rootless with a read-only root filesystem;
   - has a persistent `/workspace` volume;
   - has network egress only to the platform API, the LLM proxy and a package
     mirror.
3. **LLM proxy.** The container holds no long-lived secret. The SDK's model calls go
   to a platform **LLM proxy** (`/llm/*`). The proxy:
   - authenticates the session token;
   - injects the provider key from the encrypted credential store (migration
     0034);
   - enforces dollar budgets;
   - records per-request usage telemetry.
4. **Session orchestrator.** A new crate, `crates/agent-orchestrator`, manages the
   lifecycle of projects, sessions and containers. It also runs skill
   materialisation, the event bridge (SSE to the UI), steering, `ask_user`,
   budgets and resume-on-restart.
5. **Capabilities stay harness-neutral.** The agent reaches the platform only
   through `/api/*`, via the `tbot` SDK/CLI (and MCP for external clients). No
   platform capability depends on the harness.
6. **Pinned per project version:** the model, effort and system core. None of them
   change within a session.

## Rationale

- A coding-agent harness is exactly what the agent needs: files, code, background
  tasks, sub-agents, compaction, skills and hooks. Building it ourselves would
  cost months and trail the SDK.
- Keeping keys in the proxy satisfies "no secrets in the sandbox". It also puts
  budgets and telemetry where the agent can't bypass them (ADR-0025).
- Harness neutrality at the API boundary limits lock-in: the harness can be
  replaced without touching services.

## Consequences

- `crates/api/src/agent/{driver,prompt}.rs` and the `/agent` run loop are
  retired once AGENT-001 acceptance passes. Until then the driver gets an interim
  cache-safe elision fix.
- `crates/llm` stays, for `LlmInference` strategy nodes and model adapters, and it
  backs the proxy.
- New operational surface: container images, a container runtime on the dev box
  (Docker Desktop and WSL2; see the phantom-socket runbook), and per-project
  volumes.
- Provider neutrality is given up for the agent harness, and kept for strategy
  LLM nodes.

## Alternatives Considered

- **Claude Managed Agents with a self-hosted sandbox.** Rejected by the user in
  favour of local control and hooks; beta dependency.
- **Extend the custom Rust loop** (caching, compaction, context editing, tool
  search). Provider-neutral, but it rebuilds a harness from scratch.
- **Agent SDK without a container** (on the host). Rejected because the agent
  needs to run arbitrary code safely.

## References

- [AGENT-001](../specs/AGENT-001-agent-runtime.md)
- BS-007 [03_RUNTIME](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/03_RUNTIME.MD) and
  [04_CONTEXT_AND_COST](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/04_CONTEXT_AND_COST.MD)
- ADR-0022, ADR-0025
