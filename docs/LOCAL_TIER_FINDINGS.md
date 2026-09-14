# Local tier — measured findings

Empirical results from the dev box, taken before designing the local executor. Every
number here was produced by an actual request, not inferred from documentation.

**Measured on:** Ollama 0.33.3, GTX 1080 Ti (11 GB, Pascal), 2026-09-11.

**These measurements are from the dev box, which is NOT the design target.** The
deployment target is a single RTX 3090 (24 GB, Ampere), then 2×3090 via NVLink
(48 GB pooled), with a 128 GB unified-memory host as a stretch. The reference model
is `qwen3.6-35b-a3b` at Q4 (~21 GB resident) and multi-step-reliable tool calling is
a baseline assumption. The dev box exists here only to establish **protocol
behaviour**, which is hardware-independent, and to characterise the degraded tier.

---

## 1. Constrained decoding works, and it is `format`, not `tools`

Ollama accepts a JSON Schema in `format` and honours it. A 7B model at
`temperature: 0` produced a schema-conformant tool call on the first attempt,
enums respected:

```json
{"name": "read_bars", "arguments": {"instrument": "BTC-USD", "tf": "1h", "limit": 500}}
```

This satisfies guide §3.1 for the local tier: valid syntax becomes a sampling-time
guarantee rather than a parse-time hope.

## 2. `tools` and `format` do not compose — `format` wins

Passing both, `format` takes precedence and **`tool_calls` comes back `null`**; the
output lands in `message.content` as schema-conformant JSON.

| Request | `tool_calls` | `content` |
|---|---|---|
| `tools` only | populated, with ids | empty |
| `format` only | null | conformant JSON |
| **both** | **null** | **conformant JSON** |

So on Ollama it is either the native tool-call *serialization* or a grammar
guarantee, not both.

**This is a backend limitation, not a law.** vLLM refuses the choice: `guided_json`
constrains the generated text while `--enable-auto-tool-choice --tool-call-parser
hermes` renders the model's own template, so §3.1's guarantee and §3.3's native
template arrive in the same request. That is the finding that made vLLM the
production local backend and left Ollama as the dev-box one (ADR-0032).

Where Ollama is used, **the grammar wins the tradeoff**, because §3.1 is explicit
that the tool-call envelope is the thing to constrain, and because the alternative is
unenforced. §3.3's "native template" requirement is still met either way: Ollama
applies the model's own chat template from its Modelfile regardless — what changes is
only how the tool call is serialised out.

## 3. Tool-call training is real and visible

Same prompt, same tool schema, three models:

| Model | `tool_calls` | Verdict |
|---|---|---|
| `qwen2.5:7b-instruct` | populated, with ids | tool-trained |
| `llama3.2:3b` | populated, with ids | tool-trained |
| `qwen2.5-coder:7b` | **null** — JSON in content | **not** tool-trained |

This is guide §6.1 confirmed by measurement: a model not explicitly trained for tool
calling emits malformed calls regardless of harness quality. It is also the argument
for constraining rather than trusting the template — under `format`, the untrained
coder model produced correct output anyway.

*Incidental:* `crates/llm/src/ollama.rs`'s header comment says tool calls "carry no
ids — we synthesize `call_N`". Ollama 0.33.3 **does** return ids
(`call_q6ymnk1f`). The synthesised fallback is harmless, but the comment is stale.

## 4. The decisive one: a single-call `anyOf` envelope selects the wrong tool

Given one grammar covering three tools as `anyOf` branches, the model produced
**valid JSON naming the wrong tool, both times**:

| Task | Expected | Got |
|---|---|---|
| "Get 500 bars of BTC-USD at 1h" | `read_bars` | `finish_task` |
| "Report no edge, citing exp_1" | `finish_task` | `read_catalog` |

This is the worst available failure mode: it passes the validation ladder and does
the wrong thing. The likely cause is that the grammar commits to an `anyOf` branch
early in decoding, before the model has meaningfully chosen.

**Splitting the decode into two constrained calls fixes it — 3/3 correct:**

1. **Select.** Grammar is `{"name": {"enum": [<tools exposed this step>]}}`. One
   decision, nothing else to attend to.
2. **Fill.** Grammar is the chosen tool's argument schema, with the choice fixed.

| Task | Expected | Got | Args |
|---|---|---|---|
| 500 bars of BTC-USD at 1h | `read_bars` | `read_bars` | `{"instrument":"BTC-USD","tf":"1h","limit":500}` |
| Report no edge, cite exp_1 | `finish_task` | `finish_task` | `{"result":"BTC-USD has no edge","evidence":"exp_1"}` |
| What does the synthetic venue have | `read_catalog` | `read_catalog` | `{"venue":"synthetic"}` |

0/2 → 3/3, same model, same tasks. This matches guide §5.4 directly: *ask for ONE
decision per call.*

### Why this matters beyond reliability

**The exposure budget becomes the grammar.** Step 1's enum is exactly the tool set
`ToolRegistry::expose()` returned for this step, so a tool that was not routed is not
merely *rejected* when called — it is **unnameable**. The validation ladder's
name-check rung (§3.2 step 2) becomes structurally unreachable rather than enforced
after the fact.

That is the guide's "move intelligence out of the model and into the harness" at its
most literal, and it only works because routing and decoding share one object.

**Cost:** two model calls per step instead of one. Measured at ~0.6–1.3 s per
complete two-step decode on the dev box's 7B. On the target hardware this is not a
consideration; on any hardware it is the right trade against a wrong tool call that
validates clean.

---

## 5. Throughput, for the degraded tier only

| Measurement | Value |
|---|---|
| `qwen2.5:7b-instruct` decode | ~50 tok/s |
| `qwen2.5-coder:7b` decode | ~43–50 tok/s |
| Cold model load (first request) | ~100 s |
| Two-step decode, end to end | 0.6–1.3 s |

The 100 s cold load is a dev-box artifact (Pascal, spinning up from disk) and its
*magnitude* must **not** shape the core design — no prewarming hacks, no batch sizes
or context windows tuned to an 11 GB budget. It belongs in the degraded tier's honest
latency reporting (§15.3: surface expected duration rather than hiding slowness).

**One correction to an earlier reading of this.** An initial draft of this note put
`keep_alive` in the same category, as dev-box tuning to be kept out of the main path.
That was wrong, and the implementation does send it at every local tier. Telling a
backend to keep the weights resident is not a Pascal accommodation: a 21 GiB model
evicted between steps reloads from NVMe on a 3090 too, and a 15-step task pays for it
fifteen times. What is forbidden is tuning the *value* to this box's reload
cost. What is required is not letting a backend silently evict a model mid-task.

## 6. What the dev box cannot tell us

Everything above is **protocol** behaviour and transfers to the target. These do not:

- Whether a 32B-class MoE needs the two-step decode (it may select correctly in one
  call). The design uses two-step regardless: it is strictly more reliable, the cost
  is negligible on the target, and *the grammar-is-the-budget property is worth
  having on its own.* An eval on the target may later justify one-call for
  `local_high` — promotion by evidence, per §6.2.
- Multi-GPU pooling behaviour over NVLink.
- Unified-memory hosts where device and host memory are not distinct pools.
- Real throughput, context-growth behaviour, or OOM boundaries at 24 GB.

Each is a hole the target hardware fills, and none of them may be assumed away in
code that ships.

---

## 7. What the implementation settled

Recorded here because the measurements above framed two questions they could not
close on their own. Both are now decided in ADR-0032.

**The either/or in §2 was Ollama's, not the problem's.** vLLM applies `guided_json`
to the generated text while the model's own tool template still renders, so the
grammar guarantee and the native template arrive together. Ollama is the dev-box
backend; vLLM is the deployment one.

**The two-step decode's second property turned out to be the load-bearing one.**
§4 adopted it for reliability (0/2 → 3/3). In code it does something stronger: step
1's enum is exactly what `ToolRegistry::expose()` returned, so **the exposure budget
is the decoding grammar** and an unrouted tool is unnameable rather than rejected.
That property does not depend on model size, which is why §6's open question about
32B-class models no longer decides anything — even if a larger model selects
correctly in one call, one call gives this up.

**A finding that only appeared once there was code to break.** Under a grammar, a
non-conforming reply is impossible; the sampler has no token for it. So when one
arrives it is not the model failing to follow instructions — it is evidence the
grammar was never applied. The harness therefore re-validates every constrained reply
against the schema it sent, **with no coercion**, and one failure fences the session.
Coercion is the right answer for a model's guess and the wrong answer for a backend's
lie, because repairing it destroys the only evidence that constrained decoding is not
running.

The startup canary is built on the same reasoning, which is why its probes ask in
plain language for output the schema forbids. A probe a well-behaved model would
satisfy anyway passes just as happily on a backend that dropped the grammar, and would
have told us nothing.

---

## 8. The first live run of the GOVERNOR loop (2026-09-12)

Everything above was measured against Ollama directly. This section is the first
**end-to-end** run: the real hardware probe, the real canary, the real loop, the real
tool bridge, against this platform's own data.

**Setup:** `qwen2.5:7b-instruct` on the dev box (GTX 1080 Ti, 11 GB, Pascal), which
resolves to the `degraded` tier. Platform on `:7080`, Postgres and ClickHouse up.

### What it proved

| Mechanism | Result |
|---|---|
| Hardware probe | detected `NVIDIA GeForce GTX 1080 Ti (11 GiB)` via `nvidia-smi`, resolved `degraded` |
| Canary | **4/4 honoured**, including all three adversarial probes |
| Admission (D-18) | a `multi_step` task was **refused** with `escalate to claude-opus-5`, spending nothing |
| Two-step decode | select then fill, every step |
| Policy engine | `policy.allow` on a read, recorded with the risk it keyed on |
| Tool bridge | dispatched `list_instruments`, real coverage data came back |
| Termination | `finish_task` with `result` + `evidence`, status `completed` |

The canary result is the one worth stating plainly. Probe
`enum_under_pressure` tells the model, in words, to answer `purple`; the grammar allows
only red/green/blue. It returned `blue`. **The backend genuinely constrains** — which
is what every other guarantee on this tier rests on.

### Five bugs it found that the test suite did not

All five were invisible to 1310 passing tests, and all five are now fixed with tests.

1. **A truncated reply was read as a grammar violation.** The 7B wrote a long
   `finish_task` result, hit `reserve_for_output` at exactly 1000 tokens, and the JSON
   was cut mid-string. Unparseable output under a grammar normally *proves* the grammar
   was not applied — but not when the reply simply ran out of room. The loop fenced a
   healthy session. Truncation is now its own input (`Input::ModelTruncated`), because a
   variant cannot be forgotten the way a bool can, and repeated truncation now fences as
   `OutputTruncated { reserve_for_output }` — naming the number an operator would change
   rather than calling the model stuck.

2. **`finish_task` consumed the only step.** On `max_steps: 1` the loop did the lookup
   and then had no step left to report it, fencing with the answer already in hand.
   A spent budget now buys one last turn with exactly one tool exposed.

3. **The exposure budget hid the tool the model had just searched for.** Truncation was
   alphabetical over the union of routed namespaces, so `create_strategy` kept the slot
   and `list_instruments` — which `search_tools` had just found — did not. The model
   went and created a strategy instead, which reads as a stupid model and was the
   harness overruling it. Exposure now fills in router-priority order, and tools the
   model *named* are pinned ahead of namespace filling.

4. **The planner's namespace was trusted over the catalogue.** The 7B planned
   `{goal: "list_instruments", namespace: "strategy"}` — naming the exact tool it wanted
   and filing it under the wrong namespace. Guide §2.2 step 2a says route
   *deterministically from the plan step where possible*; we were asking the weakest
   model in the system instead. The harness now searches its own catalogue with the
   step's goal and treats the planner's namespace as a fallback. This removed the
   `search_tools` round-trip entirely: the right tool is exposed on the first try.

5. **`GET /api/instruments/{id}` returned 500 on every call.** Not a harness bug — the
   handler selected `symbol` and `is_active`, neither of which exists on the table
   (`active`, and no `symbol` at all), and mapped the error away so nothing was logged.
   The agent found it by routing correctly and getting an opaque `http_500` back.

### One that is not a bug

The corrective message for a missing `finish_task` evidence field said "naming the
handles or ids that support it". On a lookup task there *are* no ids, so the model
could not comply, failed identically three times, and was fenced for the harness's
vagueness. The requirement is right and stays; the message now names what the session
actually produced ("cite what you actually used this session: `list_instruments`").

The run went from three identical failures to success on the next attempt.

### Measured latencies, degraded tier

| | |
|---|---|
| Cold load, `qwen2.5:7b-instruct` | ~169 s |
| Tool selection (step 1 of the decode) | 0.2–0.4 s |
| Argument fill | 0.2–2.0 s |
| A 1000-token capped reply | ~22 s |
| Whole run, warm | ~40 s |

The cold load is why `keep_alive` is sent at every local tier, and the correction in
§5 stands: that is not dev-box tuning.

---

## 9. The degraded tier was promoted, and what that did and did not establish (2026-09-12)

The tier declared `single_shot` from the day it was written. That declaration was
never measured. It was inherited from "this card cannot run the reference model" —
which is a statement about **memory and tensor cores** — and used as a proxy for
**tool-calling reliability**. Those are different properties, and the proxy was wrong
in the direction that costs capability: every conversational turn arrives as
`TaskDemand::MultiStep` (`manager.rs`), so on this box the agent escalated or refused
*everything*. The local tier was, in practice, unreachable.

What actually carries a chain here is the two-step decode (§4), the exposure budget
being the decoding grammar, and a harness-owned plan holding the state the model
would otherwise have to. None of the three depends on the card.

### The measurement

`cargo test -p llm --test local_multistep_eval -- --ignored --nocapture`, driving the
real `LlmClient` against a real Ollama. Eleven checks per trial over a six-step
chain, with the three-tool exposure cap the profile actually sets:

| What it checks | Why it is in there |
|---|---|
| correct first selection against two near-miss distractors | selection must be a discrimination, not a lookup |
| recovery from an injected tool error | re-read the message, halve the limit, retry — the behaviour a single-shot tier cannot have |
| `realised_vol` chosen over `implied_vol` | one word apart |
| instrument carried from step 1's **result** | not derivable from the prompt |
| `0.0417` carried from step 3's **result** into step 4's arguments | ditto |
| `delete_strategy` never called | it is exposed for the last three steps |
| final answer carries the computed figure | the chain has to produce an answer, not just run |

**Result: 10/10 fully clean, twice, ~7–9 s per run warm**, on `qwen2.5:7b-instruct` —
the model the profile already named. The card was never the thing in the way.

### The eval bug that nearly became a finding

The first hardened run scored **0/10** and looked like a clean vindication of
`single_shot`: the model failed to retry `read_bars` after the injected error, then
invented a volatility of `0.01` and sized a position off it. That is a textbook
"fails slowly" trace, and it was **the eval's fault**. Exposure was driven off the
call *count*, so `read_bars` was retired the moment it was tried and was not in the
exposed set on the step where recovery had to happen. The check was unsatisfiable by
construction.

Worth recording because of what it nearly cost. The trace agreed with the
hypothesis, and a result that confirms what you already believe is the one nobody
re-reads. Progress is now measured by *results* — the first step of the chain not yet
satisfied by a **successful** call — which is also what the real router does.

### What the promotion is, precisely

Narrower than the tier name suggests. It says: chains of the shape the eval measured
hold together on **this model, on this device, at this exposure cap and this context
budget**, ten times out of ten. It is not a claim that a 7B is a frontier model, and
`escalate_to` is deliberately kept.

The mechanism is `harness::profile::MultiStepEvidence`. `validate()` refuses the
claim without evidence naming this model at ten-for-ten; `Profile::effective_tool_
calling` re-checks the attested `device` against the machine at startup, so a config
file that travels **loses** its promotion rather than silently keeping it. The
default is still refusal. What changed is that the claim became *earnable* rather
than *unreachable*.

### Two things the box taught us that are not about tiers

1. **`fits()` was measuring the wrong number.** It fitted models against **total**
   device memory. On a workstation that is a different number from what is
   available: measured here, the desktop session held 1.4 GB of VRAM at idle and
   6 GB with a browser and two Electron apps open. The failure mode is the quiet one
   — the backend does not refuse, it offloads layers to host memory and runs an
   order of magnitude slower, which reads as the agent hanging. `Device::free_bytes`
   is now measured and `usable_model_bytes` prefers it, falling back to the total
   when it was not measured.

2. **`OLLAMA_HOST` is a trap for clients.** It is Ollama's *server bind* variable and
   is commonly `0.0.0.0:11434` — no scheme, and an address you listen on rather than
   connect to. Passing it to a client yields a reqwest `builder error` that reads
   like a network fault. The platform never read it; the first draft of the eval did.

### Two bugs that only existed once the tier could run

Both were reachable the whole time and unreachable in practice, because a tier that
refuses every multi-step task never executes a second step. Promoting it ran them
immediately.

**1. `flatten` collapsed arrays to strings and `unflatten` never put them back.**
A flat-tier model is *sent* `backtest_ids: {type: string}` — that is deliberate,
§2.4 promises primitives — so it decoded a string and returned `"[]"`. The harness
then validated that against the **rich** schema's `type: array` and rejected it:

```
`backtest_ids` must be a array; got "[]"
```

Three identical times, then a fence. The model was doing exactly what it was told
and was being graded against a schema it never saw — the same shape of failure as
§8's corrective-message bug, one level down. Nesting had an inverse (`unflatten`
rebuilds `filter_symbol` into `filter: {symbol}`); the array collapse did not.
`unflatten` now reads the list back, accepting JSON (`["a","b"]`) or comma-separated
(`a, b`), coercing elements to `items.type`; and the flat schema now *says* which
form it wants instead of leaving the model to invent one.

**2. `ToolDef::idempotent` was set on every tool and read by nothing.**
By this crate's own rule that is a bug — it read as a guarantee and behaved as a
comment — and what it should have been preventing was happening in the open. Asked
"which instruments do you have data for", the agent got the answer at step 1 and then
called `list_instruments` **eleven more times**, because the planner had emitted
*reasoning* steps ("count the number of instruments", "name two from the list") as if
they were tool steps and the model dutifully reached for the instrument tool each
time. It answered correctly only because a spent budget buys one last turn.

An identical idempotent call is now answered from what the session already holds,
with an explicit "you already called this — use it or finish". Failed calls are
recorded too, and that is the case that was actually costing budget: a tool error is
an ordinary observation rather than something the retry budget counts, so eleven
identical `compare_backtests` calls each returning `invalid_request` cost eleven
steps. One retry is allowed there, because the harness cannot tell a deterministic
rejection from a transient one.

Same question, before and after: **13 steps and 12 tool calls → 3 steps and 1 tool
call**, same 32 seconds wall clock, same correct answer.

The planner emitting non-tool steps is the deeper cause and is still open. Re-running
a read that cannot have changed is never the right answer to it, which is why the
guard belongs here regardless.
