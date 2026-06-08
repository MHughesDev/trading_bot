# Failure Modes — a portable system, instantiated for this platform

> **Read this first.** This folder is two things at once:
>
> 1. **A reusable framework** for cataloguing the failure modes of *any* software system — an
>    operating system, a database, a web app, a distributed trading platform, a single function.
>    The framework is in [`00-taxonomy-and-schema.md`](./00-taxonomy-and-schema.md). It does not
>    mention trading. You could lift it into any repository unchanged.
> 2. **An instantiation** of that framework for *this* system. The numbered layer files
>    (`01`–`11`) and the [`relations/`](./relations/) folder are this platform's specific failure
>    modes, expressed in the framework's vocabulary.

## The core idea: failure semantics recur at every layer

The reason a single framework can describe an OS and a trading platform is that **the *kinds* of
failure are universal — only the *layer* and the *blast radius* change.**

A dropped CPU-cache line, a lost network packet, and a missed broker fill-acknowledgement are the
**same semantic failure** — *omission* — occurring at three different layers of abstraction. A
stale TLB entry, a stale DNS record, and a stale position cache are the same failure — *state
divergence* — at three layers. A scheduler priority inversion, a database deadlock, and two
strategies fighting over the same instrument are the same failure — *coordination* — at three
layers.

This recurrence is the whole insight. If you organize failure modes by three orthogonal axes —

```
LAYER       (where it lives — the abstraction level)
   ×
SEMANTICS   (how it manifests — crash / omission / timing / value / byzantine / state)
   ×
POSTURE     (how the system must respond — halt / degrade / ignore)
```

— then the axes are **system-independent** and only the populated cells change from one system to
the next. You keep the skeleton; you swap the organs.

## Why ground this in dependability theory

The framework uses the **fault → error → failure** chain from the Avizienis/Laprie dependability
taxonomy (the canonical academic model for dependable computing). This is deliberate: it gives
every entry the same three-part anatomy regardless of layer, which is what makes a kernel
page-fault and a backtest-lookahead-bug describable by the same record format.

- **Fault** — the root cause (the dormant defect or the triggering condition).
- **Error** — the latent incorrect internal state the fault produces.
- **Failure** — the observable deviation from correct service that the error eventually causes.

A fault may lie dormant. When activated it produces an error. An error, when it propagates to the
service interface, becomes a failure. **A failure at one layer is a fault at the next layer up.**
That single sentence is the entire relational model — it is how cascades work (see
[`relations/cascade-chains.md`](./relations/cascade-chains.md)).

## Folder structure

```
failure-modes/
├── README.md                          ← you are here (the thesis)
├── 00-taxonomy-and-schema.md          ← THE universal, portable framework (no domain content)
│
│   ── universal layers (reusable for any system) ──
├── 01-architectural.md                ← topology, boundaries, planes, partition, consensus
├── 02-state-and-data.md               ← correctness, consistency, durability, corruption
├── 03-control-and-concurrency.md      ← ordering, races, clocks, idempotency, coordination
├── 04-resource-and-capacity.md        ← memory/CPU/disk, queues, backpressure, exhaustion
├── 05-boundary-and-integration.md     ← contracts, serialization, versioning, inter-component
├── 06-external-dependency.md          ← network, third parties, hardware, the world outside
├── 07-lifecycle-and-operations.md     ← startup/shutdown/deploy/upgrade/recovery/config
├── 08-security-and-trust.md           ← authn, authz, secrets, injection, supply chain
├── 09-observability-and-detection.md  ← the meta-failure: being unable to see failures
├── 10-human-and-organizational.md     ← operator error, knowledge gaps, process failures
│
│   ── the plug-in layer (system-specific) ──
├── 11-domain-money-and-trading.md     ← where THIS system's unique failures live
│
│   ── the relational layer (cross-cutting) ──
├── relations/
│   ├── cascade-chains.md              ← fault→error→failure propagation across layers
│   ├── posture-matrix.md             ← the halt/degrade/ignore decision for every FM
│   └── detection-coverage.md         ← which mechanism catches which FM; the blind spots
│
└── open-questions.md                  ← unresolved failure-mode questions
```

### The split that makes it portable

Layers **01–10 are universal**. Their *structure and their abstract failure modes* apply to any
system. We have instantiated them with this platform's specifics, but the headings, the semantics,
and most of the abstract entries would survive being copied into a compiler project or a kernel.

Layer **11 is the plug-in point.** Every system has exactly one (or a few) of these: the layer
where the system's *reason for existing* introduces failures nothing else has. For an OS it would
be `11-scheduling-and-memory-protection`. For a bank it would be `11-money-and-ledger`. For us it
is `11-domain-money-and-trading`. **To reuse this framework for a different system, you keep 00–10
and rewrite only 11.**

## How to read an entry

Every failure mode, at every layer, uses the **Universal Failure Mode Record (UFMR)** defined in
[`00-taxonomy-and-schema.md`](./00-taxonomy-and-schema.md). Every entry has a stable ID
(`FM-<LAYER>-<NNN>`), the fault→error→failure anatomy, a detection mechanism, a blast radius, a
decided posture, a recovery path, links to what it propagates into, and a pointer to the
adversarial test that proves the mitigation fires.

That last field connects this folder to the project's standing rule (see
[`../spec/10-open-questions.md`](../spec/10-open-questions.md)): **every decided mechanism gets a
test that proves it fires.** This folder is the catalogue of what those tests must defend against.

## How to use it

- **Designing a component?** Read the layers it touches; every UFMR is a design constraint.
- **Writing tests?** The `Proven-by` field is your adversarial-test backlog.
- **An incident happened?** Find the failure, walk its `Propagates-to` chain backward to the root
  fault, and check whether the posture and detection held.
- **Adding an asset class / engine?** New domain failures go in layer 11; new integration shapes
  go in 05/06. The other layers should not need new entries — if they do, that is a signal the
  abstraction is leaking.
- **Reusing for another project?** Copy `00` and `01`–`10`, delete the instantiated entries you
  don't need, rewrite `11`.
