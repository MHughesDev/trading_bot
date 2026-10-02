# ADR-0038: One Live Evaluation Path — Automations Declare, the Hot Path Evaluates

**Status:** Accepted

> **Extended 2026-10-01 by [ADR-0039](0039-one-deterministic-strategy-engine-environments-are-adapters.md):** one evaluation path for *every* environment — backtest, preview, paper and live — not only the live doors.
**Date:** 2026-09-15
**Deciders:** track 16 (the execution path to a fill)
**Supersedes:** the two-evaluator arrangement implied by Set E phase 5 task 5.4 and
the `automations` table's `armed` column
**Traces:** COMP-002 §3.1 (one chokepoint, no bypass), artifact SC-2, FM-2,
Set E locked decision 12, ROADMAP DONE-3

---

## Context

The platform had **two** ways to ask it to trade, and **neither worked**.

**Door one — `POST /api/strategies/:id/start`.** It inserted a `StrategyInstance`
into the API's `InstanceManager` and returned `201 running`. The in-process
pipeline's stage 3 held:

```rust
// No strategy loaded by default; placeholder for demand-manager integration.
let mut strategy: Option<StrategyInstance> = None;
```

Nothing ever wrote to that `Option`, and nothing ever read `InstanceManager`
outside tests. A started strategy therefore evaluated nothing, for ever, and
said `201` while doing it. `InstanceManager::dispatch` — the method that would
have run those instances — had no production caller at all.

**Door two — `POST /api/automations/:id/arm`.** It set `automations.armed = true`
and returned success. `storage::automation::armed_automations`, documented as
"used to resume automations at server startup", had **no caller anywhere in the
repository**. The spec was validated on save and then never evaluated.

Two doors, two successes reported, zero intents produced. That is a total outage
that reports success — twice — and it is the reason no fill has ever come out of
this platform.

The obvious repair is to write the missing automation evaluator. That is the
wrong repair.

## Decision

**There is exactly one live evaluator: stage 3 of the in-process pipeline.
Every door *declares* what should run; none of them runs anything.**

The declaration surface is `strategy_runtime::live::LiveStrategyRegistry`:

* `POST /api/strategies/:id/start` registers a `LiveStrategySpec` with
  `LiveSource::Strategy`.
* `POST /api/automations/:id/arm` resolves the automation's
  `execution_strategy_id` to a definition and registers it with
  `LiveSource::Automation`.
* Stage 3 owns the `StrategyInstance`s and re-syncs from the registry whenever
  its generation counter moves.

`InstanceManager` keeps exactly one job — declaring lane demand, so two users
wanting `BTC-USD market.bars.1m` get one pipeline. It is no longer an
evaluation path, and `dispatch` is documented as a replay/test facility.

**`pipeline` automations are refused, not silently accepted.** A multi-stage
discovery automation needs the shared universe feed (Set E 4.3), which does not
exist. Arming one returns `501` naming what is missing. It does not return
success.

### Consequences of the mechanism

* Stage 3 reads one `AtomicU64` per `WorldEvent` and takes the registry lock
  only when the generation changed — Set E locked decision 12's "no lock or
  allocation per event", satisfied without `arc-swap`.
* Instances survive a re-sync: adding a second strategy to an instrument does
  not reset the first one's indicator warm-up.
* The registry is a process-global `OnceLock`. The API handlers and the
  pipelines run in one binary and must see one registry; threading a handle
  from `apps/platform/src/main.rs` was not available to this track, and — more
  to the point — a second registry would reproduce the exact bug this ADR
  exists to remove.

## Alternatives considered

**Write an automation evaluator.** Rejected. It restores the property that
caused the outage: two code paths that can each be correct in isolation and
silently disagree about whether anything is running. When one of them broke,
nothing failed — a second one only doubles the number of places that can be
silently dead. One evaluator has one place to look.

**Make `InstanceManager` the evaluator and have stage 3 call `dispatch`.**
Rejected. `dispatch` takes `&mut self` behind an `Arc<Mutex<InstanceManager>>`
shared with every REST handler, so the hot path would take a contended lock per
`WorldEvent` — the thing Set E locked decision 12 specifically forbids — and a
slow handler would stall market-data evaluation.

**Delete automations.** Rejected. The `SingleInstrument` automation carries a
trigger spec and a time window that a bare strategy start does not, and its
"armed" state is durable across restarts, which a start is not. The concept
earns its place; the second evaluator did not.

## Consequences

* Arming an automation and starting a strategy now produce the same runtime
  object, so a bug in evaluation is visible from both doors at once.
* `armed_automations` finally has a caller: `routes::automations::resume_armed`,
  run once at startup, re-declares every armed `single_instrument` automation
  into the registry and requests its data pipeline. An armed automation whose
  strategy no longer exists or no longer materializes is **not** resumed and is
  logged at `error` — it stays armed in the database and is visibly not
  running, rather than being silently dropped or silently re-armed against a
  definition that changed underneath it.
* An automation whose `execution_strategy_id` does not resolve fails to arm
  rather than arming against nothing.
