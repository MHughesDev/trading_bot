# Sub-agents and ensembles: what exists, what to build, what not to

Design note, 2026-09-14. Nothing here is built yet.

---

## 1. The headline

**Do not build a general "spawn N agents" primitive.** The platform already has a
coordination object for *many parallel attempts under one declared hypothesis
with one budget and a recorded draw policy* — `ledger::campaign` — and an agent
ensemble is that object with agents as the workers instead of sweeps.

Building a separate agent-fan-out path would mean reinventing, worse: the
pre-registered hypothesis, `delta_practical`, the four-dimensional budget, the
exploration floor, propensity logging, event-sourced state, and the gate stack.
More importantly it would reinvent them **outside the trial ledger**, and that
breaks the one thing this platform is built to guarantee.

The rest of this note is why, and what the actual gaps are.

---

## 2. What already exists

More than I expected. Roughly 60% of the machinery is there and unused.

| Piece | Where | State |
|---|---|---|
| Depth cap, declared and validated | `harness::profile::Orchestration::max_subagent_depth` | **Exists.** Defaults to 1; `>1` is refused below the frontier tier, with "needs eval evidence (guide §13.3)" |
| Parent/child tracing | `harness::trace` | **Works today.** `one_trace_id_spans_the_orchestrator_and_its_sub_agents` passes — a child shares its parent's `trace_id` and carries `parent_id` |
| Per-action policy | `harness::policy::decide` | **Exists**, and applies to a child unchanged — a child's `fetch_url` after untrusted content escalates exactly as a parent's does |
| Run lifecycle, orphan recovery, cancel | `api::agent::AgentManager` | **Exists**, one conversation → one run |
| Campaign: hypothesis, budget, exploration floor, dispatcher | `ledger::campaign` | **Exists, sealed, and unused by the agent layer** |
| Event-sourced campaign state | `mlops.campaign_event` | **Exists**; state is a fold, no status column |
| Campaign worker with termination checks | `api::campaign_driver::CampaignWorker` | **Exists**, `GateAdvance` only; other child workers unbuilt |
| Budget per task | `harness::profile` budgets | **Exists and is the wrong shape** — see §4.1 |

So the honest framing is not "build sub-agents". It is **"connect the agent
driver to the campaign machinery, and fix the three things that are genuinely
missing."**

---

## 3. The reason this is not an ordinary sub-agent feature

Every general agent framework treats fan-out as a latency-and-context problem:
more workers, more parallelism, smaller contexts. Here it is a **statistics**
problem, and getting it wrong silently destroys the platform's central claim.

### 3.1 Five agents asking one question is multiple testing

If five agents each run 200 trials against the same hypothesis and the best
result is promoted, that is **1,000 trials of selection**, not 200. Every
selection-adjusted statistic the platform computes — DSR, PBO, Romano–Wolf at
Gate 14, the N_eff that `Comparison` cannot be minted without — depends on the
trial count being the count of what was actually searched.

An ensemble that spawns children which register trials under *their own*
experiment ids inflates the effective trial count invisibly. Nothing errors.
The numbers just quietly become wrong, in the flattering direction, and
`Comparison` happily mints because it was handed the count it was given.

This is the single most important constraint on the design, and it has one
answer: **a child's trials register against the parent's campaign.** Not its own
experiment. Not a fresh ticket lineage. `Dispatcher::draw` already exists to make
that draw and log its propensity.

### 3.2 Diversity is the point, and diversity is also the danger

An ensemble is only worth running if its members differ — same prompt, same
model, same seed is five times the cost for one answer. But the moment they
differ, the best-of-N selection is a maximum over N draws, which is exactly the
distribution DSR corrects for.

So an ensemble must record **how** its members differ (prompt, seed, model,
search region), because that is what determines whether their results are
independent draws or one draw repeated. `candidate_set_hash` and `propensity` on
`mlops.trial` are already the right columns for this.

### 3.3 The falsifier pattern is the one that pays here

Given the falsification sprint, the highest-value ensemble on this platform is
not N proposers. It is **one proposer and one adversary**:

- The parent proposes a candidate.
- A child is spawned with the candidate, the trade list, the bars, and one
  instruction: *kill this*. It has `get_trade_list`, `probe_data_quality`,
  `find_similar_trials`, `run_oracle_test`, `check_tradeable_frontier` and
  `report_kill`.
- The child's score comes from producing a reproducible defect, never from
  agreeing.

That is a genuine ensemble (two roles, different objectives, aggregated verdict)
whose reward density is high — kills resolve in minutes — and which does **not**
multiply the trial count, because the adversary spends its budget on forensics
rather than on new backtests.

It is also the pattern where "spawn a sub-agent" earns its keep for a reason
other than throughput: the adversary needs a *clean context*. A model asked to
attack a candidate it just spent forty steps constructing is the same model with
a sunk cost; a fresh child has no such attachment. That is the strongest
argument for process isolation here, stronger than parallelism.

---

## 4. What is genuinely missing

Three things. The first is the one that will cause an incident if skipped.

### 4.1 Budget is per-task and must become per-tree

`harness::profile` budgets are `max_usd_per_task`, `max_wall_clock_s`,
`max_tokens_per_task`. A parent that spawns five children, each inheriting the
profile, spends **six times the declared budget** — and nothing notices, because
each run is individually within its limit.

The fix is a type, in the shape this codebase already uses for capital:

```rust
/// A share of a budget, which can be split and never multiplied.
///
/// No public constructor and no `Clone`. `split` consumes the parent lease and
/// returns the remainder alongside the child's share, so the arithmetic is
/// enforced by ownership rather than by care.
pub struct BudgetLease { /* usd, tokens, wall_clock, trials */ }

impl BudgetLease {
    pub fn split(self, n: usize) -> Result<(Vec<BudgetLease>, Self), BudgetError>;
}
```

`AllowedFraction` and `RampAuthority` are the precedent: the violation should be
unrepresentable, not validated. A `Clone` on this type is the bug, so it does not
derive one.

`campaign::Budget` already has the four dimensions (`max_trials`, `gpu_hours`,
`usd`, `wall_clock_hours`). A lease is a claim against one of those.

### 4.2 The spawn itself, with the caps enforced rather than declared

`max_subagent_depth` is declared and nothing reads it at spawn time, because
nothing spawns. What a `spawn_subagents` tool needs to refuse, by mechanism:

- **Depth** beyond the profile's cap. Depth lives on the run row and is
  `parent.depth + 1`; a child at the cap cannot spawn.
- **Fan-out** beyond a per-level cap. Unbounded fan-out at depth 2 is
  exponential, and the failure mode is a bill rather than an error.
- **Budget** that does not divide. If the remaining lease cannot be split `n`
  ways above a floor, spawn fewer or refuse — never scale each child down to
  something that cannot finish, which produces `n` truncated transcripts and no
  answer.
- **Tool surface**: a child's tools are a **subset** of its parent's, never a
  superset. This must be enforced at spawn rather than in the child's profile, or
  privilege escalation is one prompt away.

The last one deserves emphasis. A child that can call `report_kill` when its
parent cannot is a way for a parent to launder an action it was refused.

### 4.3 Aggregation has to be a declared rule, not a summary

The tempting design is "collect the children's transcripts and let the parent
read them". That is the worst option: the parent becomes a selector over N
results with no record of the selection, which is §3.1 with extra steps.

Aggregation should be one of a closed set, declared **before** the spawn:

| Rule | Semantics | Multiplicity |
|---|---|---|
| `all` | Every child's result is returned, none chosen | None — nothing was selected |
| `first_success` | First child to meet a pre-declared criterion | One draw, ordered |
| `consensus(k)` | At least `k` of `n` children agree | A vote, recordable |
| `adversarial` | Parent's claim survives if no child refutes it | **Negative** selection — the good case |
| `best_by(metric)` | Maximum over children | **Full multiplicity.** Must register `n` against the campaign |

`best_by` is the one that needs the trial-count treatment, and the table exists
so that choosing it is visibly different from choosing the others.

---

## 5. What not to build

- **No unbounded recursion.** Depth 2 covers proposer → adversary and
  planner → executors. Depth 3 has no use case here and turns a cost bug into an
  exponential one.
- **No shared mutable scratch between siblings.** Two children writing the same
  workspace file is a race with a plausible-looking result. Each child gets its
  own workspace; the parent merges.
- **No child-spawns-child before depth 2 has evidence.** The profile validator
  already says this ("needs eval evidence, guide §13.3") and it is right.
- **No "swarm" that reads each other's partial output.** That is one agent with
  extra latency, and it destroys the independence that makes an ensemble worth
  aggregating.
- **No sub-agent with a wider tool surface than its parent.**

---

## 6. Proposed shape

```
spawn_subagents(
  role:        "adversary" | "explorer" | "executor",
  count:       1..=8,
  brief:       [ per-child instruction ],
  aggregation: "all" | "first_success" | "consensus" | "adversarial" | "best_by",
  campaign_id: <required for `best_by`>,
) -> { children: [...], lease_spent: {...}, aggregated: {...} }
```

Backed by:

- `mlops.agent_tree` — parent, child, depth, role, lease granted, lease spent,
  aggregation rule, declared **before** results arrive (the same
  resolve-then-dispatch ordering as the audit trail: a rule chosen after seeing
  the answers is not a rule).
- `agent_runs.parent_run_id` and `.depth` — two columns on a table that exists.
- Children's trials registered through `Dispatcher::draw` against the parent's
  campaign, so `N_eff` counts what was actually searched.

---

## 7. Sequence

1. **`BudgetLease`** — the type, with `split` and no `Clone`. Nothing else is
   safe to build first; without it the first ensemble is a bill.
2. **`agent_runs.parent_run_id` + `.depth`**, and depth enforcement at spawn.
3. **The adversary role only.** One role, `aggregation: adversarial`, fan-out
   1–3. It is the highest-value pattern, it does not multiply the trial count,
   and it exercises the whole path.
4. **Measure it** against the planted/null worlds in `procedure_null` before
   adding a second role — does proposer+adversary have a better ROC than
   proposer alone? That is a measurable question and the answer decides whether
   ensembles are worth anything here.
5. `best_by` and campaign-registered fan-out, only if (4) says yes.

---

## 8. Confidence

- **Established in this repo:** everything in §2 — I read it.
- **Engineering inference:** §3.1's multiplicity argument follows from how
  `Comparison` and Gate 14 already work; it is not a literature claim, and I have
  not verified it against published multi-agent evaluation work.
- **Weakly evidenced:** the claim in §3.3 that a fresh child attacks a candidate
  more effectively than the parent that built it. It is plausible, it matches the
  sunk-cost framing in the external review, and **nobody has measured it here**.
  Step 4 exists to measure it rather than assume it.
- **Not researched:** published work on LLM ensemble aggregation, debate, and
  self-consistency. I have not read it for this note and the design should be
  checked against it before step 5 — that is exactly what `web_search` is now
  for, and running it is the obvious first use of the tool.
