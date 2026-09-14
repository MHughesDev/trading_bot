# ADR-0031: Harness conformance — capability profiles, tool budgets and containment

**Status:** Accepted
**Date:** 2026-09-11
**Supersedes:** nothing
**Amends:** ADR-0024 (agent runtime), ADR-0028 (skill registry)
**Relates to:** D-01, D-07, D-10, D-11

---

## Context

`AGENT_HARNESS_GUIDE_v3.md` is adopted as the design standard for this platform's
agent harness. Auditing the existing design and code against it found the platform
strong on containment and weak on the model-facing harness:

**Already conformant.** Isolation tier T1 with default-deny networking, no secrets in
the sandbox, ephemeral rebuildable environments and resource caps (guide 10.2); code
execution only inside the sandbox (11); persistent human-in-the-loop approvals that
survive a restart (14.3); an eval suite that ships with the application and gates
promotion (6.2); checkpointed background execution with resume (15.1).

**Not conformant, and the gaps are load-bearing:**

1. **No capability profile.** Model, effort, budget and tool-result caps were env
   vars and hardcoded constants. The guide's central mechanism — one harness whose
   behaviour is parameterised by a profile loaded at startup — did not exist.
2. **No tool budget and no risk taxonomy.** The MCP catalogue exposes **56 tools**;
   the internal-agent profile exposes **38**. The guide's frontier band is 20–40 and
   its local band is 2–6. No tool declared whether it reads, writes, destroys or
   sends, so the approval policy had nothing to key on.
3. **No provenance tagging.** This platform ingests Reddit and web-collector text.
   That content reaches a model that also holds private research data. Nothing marked
   it as untrusted, and nothing wrapped it in a data block.
4. **No recorded trifecta audit.** The outbound leg *is* cut (the container's network
   is `internal: true`), which is the correct answer — but the guide requires the
   audit to be written down in the agent's own config, and it was not.
5. **No workspace contract.** The agent had `/workspace` and a convention. The
   guide's `inbox/work/memory/outputs/logs` layout, the `deliver` tool that makes
   "done" verifiable, and per-workspace quotas did not exist.

## Decision

### 1. Adopt the guide, with one scope limit

Guide Parts 0–2, 4–5 (harness-side), 9–15 are adopted and implemented. Part 3
(constrained decoding, native chat templates) and Part 6.1 (local model selection)
are **implemented as profile surface and left unimplemented at the adapter**, because
no local model is wired yet. The profile fields exist, the startup canary hook
exists, and loading a local profile fails loudly rather than silently running
unconstrained.

### 2. D-01 and D-07 are not reopened

The guide's prime directive is one harness spanning frontier and local models. D-01
says frontier first and D-07 fixes the harness as the Claude Agent SDK. Those are
locked decisions and this ADR does not flip them.

They are also less in conflict than they look, because **the guide's value is not
conditional on running a local model.** Guide §0.4 states it directly: context rot and
tool overload degrade frontier models too, just later. Every gap listed above is a
real defect against the frontier model we run today — 56 undifferentiated tools with
no risk taxonomy is a bad harness at any tier.

The resolution is structural rather than a choice between them:

- **The Agent SDK remains the executor** for the `frontier` profile. It owns the
  observe-think-act loop, the chat template and the tool-call format inside the
  container.
- **Everything the guide calls harness-side moves to the platform**, where it is
  harness-neutral: the capability profile, the tool registry with its risk taxonomy
  and exposure budget, the validation ladder, the provenance and policy engines, the
  workspace contract, the budgets and the trace. D-07's own consequence column
  already required this ("the platform API, SDK and MCP layer stays harness-neutral")
  — this ADR is what makes that sentence true in code.
- **A local tier becomes a profile plus an adapter**, not a second harness. Adding
  one is a config entry and a `ModelAdapter` implementation; nothing above the
  adapter changes.

**What this does not decide:** whether to run local models at all. That is D-01's
territory. This ADR makes the decision cheap to revisit later instead of making it
now.

### 3. The tool catalogue gets a risk taxonomy and an exposure budget

Every tool declares a namespace (`data.*`, `jobs.*`, `research.*`, …) and a risk
(`read | write | destructive | outbound`). Exposure per step is capped by the active
profile and enforced in code. The catalogue may be any size; simultaneous exposure is
what is budgeted. A `search_tools(query)` escape hatch keeps routing errors
recoverable.

**56 → the frontier profile's budget.** The reduction is by routing, not deletion:
the catalogue keeps every tool and the registry exposes the namespace the current
step needs.

### 4. Untrusted content is tagged and wrapped; the trifecta is recorded

Content entering context carries a provenance level. `external_untrusted` content is
wrapped in a delimited data block with a standing charter rule that data blocks are
never instructions. Any destructive or outbound action proposed in the step after
untrusted content was ingested escalates to human approval regardless of tier.

Each agent config records its trifecta legs explicitly. For the research agent:
private data **yes**, untrusted content **yes** (Reddit, web, and any third-party
tool description), outbound channel **cut** — the container's network reaches the
platform and nothing else, and publishing happens only through `deliver`.

### 5. The workspace becomes a contract

`inbox/ work/ memory/ outputs/ logs/`, with harness-side path canonicalisation on
every call, per-workspace quotas, and `outputs/` as the only thing a caller sees.
This is the concrete form of the scratchpad rule the design already relied on.

## Consequences

- A new crate, `crates/harness`, holds the six components as separable modules. It
  depends on nothing above it, so a future local adapter or a different executor
  reuses it unchanged.
- Profiles live in `config/profiles/*.yaml` and are loaded at startup. An unknown
  model defaults to the most conservative tier. **A profile value that nothing
  enforces is a bug**, and the conformance tests assert each one is read.
- The auditor suite gains fixtures for the new guards, so the tool budget, the risk
  policy and the provenance wrapper are checked on every PR like every other
  invariant.
- AGENT-001 §6 and AGENT-002 are amended; the tool catalogue grows a required
  `namespace` and `risk` per tool, which is a breaking change to the MCP descriptor
  shape and is made in one pass.

### 6. Appendix A is encoded, and deviations must be stated

Guide v3 adds a defaults table. It is encoded in `harness::appendix_a` rather than
left in the document, because a table nobody checks drifts. The guide says an eval
beats the table, so conformance *reports* deviations rather than refusing them — and
requires each one to carry a stated reason. An explained deviation is a decision; an
unexplained one is drift.

Two deviations are recorded on the frontier profile, both in
[docs/HARNESS_CONFORMANCE.md](../HARNESS_CONFORMANCE.md): `max_steps` (400 against a
default of 30, because a task here is a multi-hour research session rather than a
question) and `max_retries_per_call` (3 against 2, so one retry budget serves every
tier).

### 7. Failure injection is part of the eval suite

Guide v3 §6.2 adds it, and it is the half this platform was missing: the eval suite
measured whether the agent finds real edges and refuses false ones, not what happens
when a tool returns garbage or a web page contains an instruction addressed to the
agent. `crates/harness/tests/failure_injection.rs` covers malformed output, mid-chain
failures, over-budget contexts, and prompt-injection payloads — asserting the **policy
engine** blocks the resulting action rather than that the model resisted the text.

## Alternatives considered

**Adopt the guide wholesale, including a local-first harness.** Rejected for now: it
reopens D-01 and D-07, which are the user's to reopen, and it would trade a working
frontier runtime for a rebuild before a single live session has run.

**Treat the guide as advisory and fix only the security items.** Rejected: the tool
budget and the profile system are the items with the largest effect on reliability
today, and deferring them means the catalogue keeps growing against no limit.

**Put the harness components inside `agent-host` (the container).** Rejected: the
container is the thing being contained. Enforcement that lives inside it is
enforcement the agent can reach (D-10).
