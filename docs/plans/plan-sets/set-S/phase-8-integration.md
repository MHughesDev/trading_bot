# Phase 8 — Integration and honesty

**Completion: 0% (0 / 9 tasks)**

**Requirement IDs:** TC-35…TC-40, RP-05…RP-08, EC-12.
**Specs:** [AGENT-006 §6](../../../specs/AGENT-006-tool-catalog.md),
[AGENT-008 §3.4–3.5](../../../specs/AGENT-008-reasoning-protocol.md),
[AGENT-005 §8](../../../specs/AGENT-005-context-assembly.md), AGENT-010 §10.
**Depends on:** Phases 2 (sub-runs, effect log), 3 (the tools the workflows compose), 4 (the
suites that grade them).
**Blocks:** nothing — this is the set's exit.
**Migration range:** 0085–0088.

The last phase is where the pieces stop being tools and start being a method. A workflow is
one call that runs a fixed battery with decision points and emits a verdict; a gate that
demands a workflow's ticket is a gate the model cannot argue past; a report that must carry
verified claims is a report a human can trust cold.

---

## Workflows

### S-8.1 The workflow tier — M
`NOT STARTED`

`crates/mcp-server/src/tools/workflows.rs`: a server-side sequence of typed and templated
calls with decision points, exposed as **one** tool, spending trials exactly as its steps
would, under one trial family. The model cannot tell a workflow from a typed tool — the
contract is identical.

**Acceptance:** a workflow's `ToolResult` carries every step's summary in `detail` and every
step's ticket in `ids`; its trial count equals the sum of its steps'.
**Traces:** AGENT-006 §2.2, §6; §10.6.

### S-8.2 The verdict workflows — L
`NOT STARTED`

`leakage_audit` (provenance → static scan → bar boundary → label shuffle → perturbation →
LLM contamination if text features exist; verdict `clean | suspect | leaking`, and **"no
edge" and "leaking" are different conclusions**), `selection_bias_verdict` (N_eff →
MinTRL → DSR → PBO → family-wise), `hypothesis_precheck` (dedup → registry → negative
results, where a prior `execution_infeasible` may be retried **only** with a cited capability
delta).

**Acceptance:** each emits one verdict from a closed set with the full evidence bundle sealed
into the trial ticket; the leakage audit's bundle is permanent, not transient.
**Traces:** AGENT-006 §6; research brief Themes 1–2.

### S-8.3 The capital workflows — L
`NOT STARTED`

`precapital_battery` (ablation → fragility → cost shock → reshuffle → jitter → regime →
synthetic stress on bootstrap/fitted generators **only** → red-team falsify), `signal_identity`
(half-life → spanning → style → cs/ts → health check → crowding), `go_live_readiness`
(capacity → venue constraints → funding → divergence → shadow session).

**Acceptance:** `precapital_battery` reaches `reject | reduced_size_paper | full_paper`;
a GAN/diffusion generator can never contribute to a pass verdict.
**Traces:** AGENT-006 §6; research brief Themes 3–5.

### S-8.4 Gates demand tickets — M
`NOT STARTED`

Amend the funnel: Gate 2 requires `leakage_audit = clean` **and**
`selection_bias_verdict = passes`; Gate 3 requires `precapital_battery ≠ reject`; the paper
gate requires `go_live_readiness ≠ no_go`. The gate refuses to advance without the ticket ids.

**Files:** `crates/backtest/src/gates/`, `crates/mcp-server/src/tools/research.rs`.
**Acceptance:** `advance_gate` to Gate 2 fails without both tickets, and the failure names
which is missing. Adversarial: a persuasive argument with no ticket is refused.
**Traces:** AGENT-006 §6; AGENT-008 P8.

---

## The report layer

### S-8.5 Claims, and rechecking them — M
`NOT STARTED`

`report.extract_claims` decomposes agent output into `{claim, evidence_refs, confidence,
verification_method}` by **post-hoc span alignment**, not by trusting the model to self-cite —
LLMs find relevant documents well and pinpoint the supporting span badly.
`direct_stat` is auto-assignable only when a number in the claim exactly matches a value in
the cited evidence; anything else is `derived` or `unverified`.
`report.recheck_claim` re-derives via a real tool call, and a failed recheck **blocks** the
report rather than logging.

**Files:** `crates/harness/src/report/`.
**Acceptance:** an unsupported claim blocks finalisation; a claim whose number does not match
its evidence cannot be `direct_stat`.
**Traces:** AGENT-006 §5.8; AGENT-008 §3.4 rungs 6–7; research brief Theme 9.

### S-8.6 The critic, in a fresh session — M
`NOT STARTED`

`report.adversarial_critique` runs as a sub-run (S-2.12) with a **fresh context**, the
evidence bundle only, read-only tools, and never the producer's reasoning trace — separated
*sessions*, not just separated prompts, because same-context self-critique measurably
underperforms. Smaller tiers get a deterministic checklist critic instead: every acceptance
criterion has an evidence id, every trial is in a family, no `suggestive` finding is
presented as established, the null comparison ran. Track reviewer precision over time.

**Acceptance:** the critic cannot read the producer's transcript, asserted structurally; a
`blocking` flag sends the draft back rather than annotating it.
**Traces:** AGENT-008 §3.4 rung 8; AGENT-006 §5.8; research brief Theme 3 (naive debate
underperforms).

### S-8.7 The dossier — M
`NOT STARTED`

`report.compile_dossier` with mandatory "Unresolved questions" and "Reviewer critique"
sections that must say "none found" **explicitly** rather than being omitted — an omitted
section reads as suspicious to an experienced reviewer. Plus the API the desk reader needs:
`GET /api/dossiers/{ticket}`, `…/verify/{claim}`, `…/comments`.

**Files:** `crates/harness/src/report/`, `crates/api/src/routes/dossiers.rs`,
`migrations/0085_dossiers.sql`.
**Acceptance:** a dossier with nothing unresolved still renders the section with "none found";
`verify` re-runs a claim's recheck live.
**Traces:** AGENT-006 §5.8; AGENT-012 §5.

---

## Code, and the close

### S-8.8 `run_python` and the container's new job — L
`NOT STARTED`

One tool, namespace `code`, risk `write`, gated by the existing
`orchestration.code_execution` field (`profile.rs:164`, already enforced at `:398`):
`denied`/`templated` tiers run templates on the host skill runner; `full` runs the
**container** from ADR-0024, repurposed — `infra/agent-image` keeps its research stack,
non-root user, read-only root and internal network, **drops the Claude Agent SDK**, and runs
a small executor accepting `{script, payload, data_path}`. Started per conversation, mounting
that conversation's workspace, torn down with it. **Build the image in CI** — nothing builds
it today — and add an `agent-sandbox` compose service.

**Files:** `infra/agent-image/`, `.github/workflows/ci.yml`, `docker-compose.yml`,
`crates/mcp-server/src/tools/code.rs`.
**Acceptance:** `run_python` on the `full` tier executes in the sandbox with the
conversation's workspace mounted and **no provider credential present**; on a `denied` tier
the tool is not exposed while the templated tools still are.
**Traces:** AGENT-005 §8; AGENT-006 §5.9; AGENT-010 §10; ADR-0035.

### S-8.9 Close the set — M
`NOT STARTED`

Run every suite; publish the scorecards; fill the last `Verified by:` slots across the nine
specs; update `docs/parity-matrix.md` with the new behaviours and their evidence; move the
nine specs from `Proposed` to `Implemented` in `docs/specs/README.md`; mark AGENT-001…004 and
COMP-006 superseded; record in `PROGRESS.md` what is enforced by mechanism and, honestly,
what is not.

**Acceptance:** all 75 acceptance criteria carry evidence or a recorded, justified exception;
`COMPLACENCY_LOG.md` has an entry for every exception with the phase that would close it.
**Traces:** `docs/procedures/execute-plan.md` §3c; MASTER §6.

---

## Exit criteria

- A battery is one call, one trial family, one verdict.
- A gate refuses to advance without the tickets that license it.
- A report cannot be finalised with an unsupported claim, and the critic that checks it
  cannot see the reasoning that produced it.
- `run_python` exists at the `full` tier, in a sandbox the agent's judgement does not run in,
  and the image is built by CI.
- Every one of the 75 criteria is closed with evidence, or its exception is recorded where
  the next person will find it.
