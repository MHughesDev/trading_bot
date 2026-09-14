# ADR-0032: The local tier — target hardware, the GOVERNOR loop, and the fence

**Status:** Accepted
**Date:** 2026-09-11
**Supersedes:** D-01 (frontier-first), which becomes D-18
**Amends:** ADR-0024 (agent runtime), ADR-0031 (harness conformance)
**Relates to:** D-07, D-10, D-18

---

## Context

ADR-0031 built the harness-side components — capability profiles, the tool registry,
the validation ladder, the policy engine, the context manager — and deliberately left
guide Part 3 (constrained decoding, the native chat template) as *profile surface with
no adapter behind it*. That was honest at the time: no local model was wired, and a
stub that passed `verify` and then decoded unconstrained would have been worse than
nothing.

This ADR wires it. Three things had to be settled first, and two of them were settled
by measurement rather than by argument.

### The hardware target is not the dev box

The machine this was developed on is a GTX 1080 Ti: 11 GB, Pascal, no usable tensor
cores. Designing against it would have baked an 11 GB budget and a set of FP32
fallbacks into the core path permanently, and every future host would have inherited
them.

The deployment target is:

| | |
|---|---|
| **Primary** | one RTX 3090 — 24 GB, Ampere, working FP16/BF16, flash-attention |
| **Near-term** | two 3090s over NVLink — 48 GB pooled |
| **Stretch** | a 128 GB unified-memory host for 120B-class MoE |

The dev box is a **degraded tier**: it runs what it can and refuses what it cannot.
Its constraints do not shape anything else.

### Constrained decoding and native tool templates are an either/or on Ollama

Measured on Ollama 0.33.3 (`docs/LOCAL_TIER_FINDINGS.md` §2): passing `format` and
`tools` together makes `format` win and `tool_calls` come back `null`. A backend
cannot give both.

vLLM can — `guided_json` applies to the generated text while
`--enable-auto-tool-choice --tool-call-parser hermes` renders the model's own
template. So the choice is not between a guarantee and a template; it is between
backends.

### A single grammar over many tools picks the wrong one

The most important measurement. Given one `anyOf` grammar covering three tools, the
model produced **valid JSON naming the wrong tool, twice out of two**
(`LOCAL_TIER_FINDINGS.md` §4). That output passes every rung of the validation ladder
and then does the wrong thing, which is the worst failure mode available to us.

Splitting the decode into two constrained calls fixed it, 3/3.

## Decision

### 1. The reference model is `qwen3.6-35b-a3b`, and it is the target, not an aspiration

`config/profiles/qwen3.6-35b-a3b.yaml` is the default local profile: ~21 GiB at Q4,
`tool_calling: multi_step`, `requires_target_architecture: true`. With the resolver's
20% headroom it does **not** fit a single 3090 and does fit two pooled — and the
resolver says which, at startup, rather than OOMing four hours into a session.

It is unreachable on the current dev box. That is expected and acceptable.
Multi-step-reliable tool calling is a baseline assumption here, not a capability to be
defensive about. No Pascal workaround, no FP32 fallback and no 11 GB constant appears
on the main path; `the_resolver_holds_no_dev_box_constants` in
`crates/harness/src/hardware.rs` fails the build if one does.

### 2. vLLM is the production backend; Ollama is dev-box only

`Provider::Vllm` is a distinct variant from `Provider::OpenAi` despite sharing the
wire format, because two things differ and both matter: it needs no API key, and
`guided_json` must **not** be sent to api.openai.com, which rejects unrecognised body
parameters with a 400 rather than ignoring them.

Ollama stays supported at `tool_calling: single_shot` for the degraded tier.

### 3. Two constrained calls per step, and the exposure budget becomes the grammar

1. **Select** — the grammar is `{"name": {"enum": [<tools routed this step>]}}`.
2. **Fill** — the grammar is the chosen tool's argument schema, choice fixed.

The reliability gain is the reason it was adopted; the structural property is the
reason it stays. **A tool that was not routed is unnameable, not merely rejected.**
The validation ladder's name-check rung becomes structurally unreachable rather than
enforced after the fact, and that only works because routing and decoding share one
object.

Cost: two model calls per step. Negligible on the target, and the right trade against
a wrong tool call that validates clean.

### 4. The loop is sans-IO

`harness::drive::Loop::next(&mut self, Input) -> Vec<Effect>` — synchronous, no
tokio, no reqwest, no provider names. `crates/api/src/agent/local_driver.rs` is a thin
pump that performs effects.

This is not a style preference. The failures that matter on a local tier are a backend
restarting mid-session, a model evicted from VRAM, guided decoding falling back, a
second GPU that is not actually pooled. In this shape each of those is a `Vec<Input>`,
and **every one of them is a test that runs green on a machine with no accelerator** —
which is the only reason they exist at all on this dev box.
`the_loop_names_no_runtime_no_client_and_no_provider` fails the build if the boundary
erodes.

### 5. The fence: one constraint violation ends the session

Every constrained reply is re-validated against the schema it was sent, with
`validation::conforms` — **strict, no coercion**.

That last part is the whole design. Coercing `"500"` to `500` is the right answer when
a model guessed a type; it is exactly the wrong answer when a *backend* returned a type
its grammar forbade, because the repair destroys the only evidence that constrained
decoding is not running. A violation produces
`Degradation::ConstraintIgnored` and fences the session.

Under a grammar the sampler cannot emit a token the grammar forbids. So a violating
reply is **not a model bug — it is proof the backend lied**, and continuing means
running a local tier for hours with none of §3.1's guarantees and nothing anywhere
saying so. The same bytes on an unconstrained tier are an ordinary retry; the
difference is not severity, it is what the failure proves.

### 6. The canary is adversarial, and has three outcomes

`crates/harness/fixtures/canary.json`. At startup each probe hands the backend a schema
**and a prompt that asks in plain language for output the schema forbids** — "answer
with the colour purple" against an enum of red/green/blue.

A probe a well-behaved model would satisfy anyway proves nothing: it passes just as
happily on a backend that dropped the grammar. The instruction-following that usually
helps is what makes this diagnostic.

| Outcome | Meaning | Severity |
|---|---|---|
| `Honoured` | the reply obeys | fine |
| `Rejected` | the backend refused the schema | fine — it *told* us |
| **`Ignored`** | accepted and ignored | **the one that ships to production** |

`Ignored` is reported ahead of `Rejected` when both occur: a loud failure is a good
failure, and the silent one is what the operator must read first.

### 7. Termination is typed at every tier

`finish_task` with required `result` and `evidence`, replacing the `FINAL:` prose
contract in the frontier driver as well. A model that can end a task by writing the
right words can end it by accident — quoting the contract back, or summarising a tool
result that happens to begin that way.

A run that stops calling tools without calling `finish_task` is recorded **`failed`,
not `completed`**, with the prose kept as the summary. Nothing is lost, and the record
does not claim a conclusion nobody can check.

## Consequences

- `agent_runs.status` gains `fenced` and `refused`. A fence is not folded into
  `failed` because the two call for different actions: `failed` means look at the
  task, `fenced` means look at the **backend**.
- `approval_requests` becomes run-scoped as well as project-scoped (migration 0040),
  so `AwaitingApproval` is one queue rather than two.
- A model with no profile keeps the legacy driver. Declaring a profile is the
  deliberate act that opts a model into the GOVERNOR loop.
- `registry::unflatten` was added: `flatten` shipped without an inverse, so
  `schema_style: flat` had been correct only for tools with no nested object in them.
- Pooling is **never inferred**. Two cards in a box are 48 GB only over NVLink with a
  runtime that pools them, and nothing in `nvidia-smi` says whether that is true, so
  detection reports `pooled: false` and pooling must be declared.

## Alternatives considered

**Design against the 1080 Ti and lift the target later.** Rejected on the user's
instruction, and they were right: an 11 GB budget and a set of Pascal workarounds in
the core path would outlive the card by years, and every host would inherit them.

**One constrained call with an `anyOf` envelope.** Rejected by measurement: 0/2 correct
tool selection, with output that passes validation. Cheaper and wrong.

**Native tool templates without a grammar.** Rejected: §3.1 makes constrained decoding
unconditional for local tiers, and the alternative is unenforced. vLLM makes this a
false choice anyway.

**Repair a constraint violation instead of fencing.** Rejected. It works — the
coercion path already exists and would fix most of them — which is exactly the problem:
it converts the only available evidence of a broken backend into a silent, successful
request.

**Keep `FINAL:` for the frontier tier since it works there.** Rejected: two spellings
of "done" is two termination contracts to keep in step, and the one that drifted would
be the one nobody was testing.

---

## Amendment, 2026-09-12: `multi_step` on a degraded tier becomes earnable

**Status:** Accepted. Amends D-18 as stated in this ADR.

D-18 as written forbade a `degraded` tier from declaring `multi_step` at all, and
`Profile::validate` enforced it. The intent was right and stands: a tier that
*asserts* capability it has never shown gets work admitted that then fails slowly,
which is the outcome the tier exists to prevent.

The rule was wrong about one thing. It made the claim **unreachable** rather than
**unearned**, by using hardware fitness for the reference model as a proxy for
tool-calling reliability. Those are different properties. What carries a chain in
this design is the two-step decode, the exposure budget being the decoding grammar,
and a harness-owned plan — none of which depend on the card. Measured on the dev box
with the model the profile already named: **10/10 fully clean** on an eleven-check,
six-step chain including recovery from an injected tool error and two state-carry
checks (`docs/LOCAL_TIER_FINDINGS.md` §9). The proxy cost the platform its entire
local tier: every conversational turn arrives as `TaskDemand::MultiStep`, so the
agent escalated or refused everything on that box.

So the claim is now admissible **on evidence**, via `profile::MultiStepEvidence`:

- `validate()` refuses it without an attestation naming **this** model, at least ten
  trials, every one clean.
- `Profile::effective_tool_calling(&hw)` re-checks the attested `device` against the
  machine at startup and **demotes to `single_shot`** when it does not match, routing
  through the ordinary `admit` path rather than a second failure mode. A config file
  travels; a GPU does not.
- `cargo test -p llm --test local_multistep_eval -- --ignored` is what produces an
  attestation, and it prints the block to paste — including the device name read from
  `nvidia-smi`, so a promotion cannot be granted by mistyping.

Refusal remains the default. Nothing about the tier's *name* changed: the machine
still cannot run the reference model, and no Pascal workaround, FP32 fallback or
11 GB-tuned constant appears in the core path.

**Also corrected here:** `hardware::fits` was checking models against **total** device
memory. On a workstation that is the wrong number — the desktop session held between
1.4 GB and 6 GB of VRAM depending on what was open — and the resulting failure is the
quiet one, because the backend offloads to host memory rather than refusing.
`Device::free_bytes` is measured when available and preferred, falling back to the
total when it is not.
