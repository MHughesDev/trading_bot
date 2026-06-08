# 00 — Taxonomy & Schema (the portable framework)

> **This file contains no domain-specific content.** It defines the vocabulary, the axes, and the
> record format used by every other file in this folder. It is designed to be copied, unchanged,
> into any software project at any level of abstraction — kernel, database, library, distributed
> service, embedded controller. If you ever find a trading term in this file, it is a bug.

---

## 1. The dependability chain (the anatomy of every failure)

Every failure mode, everywhere, has the same three-part anatomy:

```
   FAULT  ───activates──▶  ERROR  ───propagates──▶  FAILURE
(root cause)            (latent bad state)      (observable deviation)
```

- **Fault** — the adjudged or hypothesized cause of an error. May be **dormant** (present but not
  yet activated) or **active**. Faults are *developmental* (a bug, a wrong assumption, a missing
  check) or *operational* (a disk wears out, a packet drops, an operator fat-fingers a config).
- **Error** — the part of system state that is incorrect. An error is *latent* until it is used;
  it is *detected* when an internal check finds it, *effective* when it alters behavior.
- **Failure** — an event that occurs when the delivered service deviates from correct service, as
  seen at the **service interface** (by a user, an operator, or a depending component).

### The recursion that creates cascades

> **A failure at layer N is a fault at layer N+1.**

A dropped packet (failure of the network layer) is a fault for the RPC layer. A timed-out RPC
(failure of the RPC layer) is a fault for the service. A 500 from the service (failure of the
service) is a fault for the client. This single rule is the entire relational model. Cascades are
just fault→error→failure chains stitched across layer boundaries.

---

## 2. Axis 1 — LAYER (where the failure lives)

Layers are ordered from most abstract (the shape of the system) to most concrete (a specific
wire). A failure mode is filed under the **layer where its fault originates**, not where it is
ultimately observed (observation is a cascade, tracked separately).

| # | Layer | What lives here | Universal? |
|---|-------|-----------------|-----------|
| 01 | Architectural | Topology, boundaries, planes, partitioning, consensus, the "shape" | ✅ |
| 02 | State & Data | Correctness, consistency, durability, the data model, corruption | ✅ |
| 03 | Control & Concurrency | Ordering, races, clocks, idempotency, coordination, deadlock | ✅ |
| 04 | Resource & Capacity | Memory, CPU, disk, queues, backpressure, exhaustion, leaks | ✅ |
| 05 | Boundary & Integration | Contracts, serialization, versioning, any seam between two parts | ✅ |
| 06 | External Dependency | Network, third parties, hardware — anything past the trust boundary | ✅ |
| 07 | Lifecycle & Operations | Startup, shutdown, deploy, upgrade, crash recovery, config | ✅ |
| 08 | Security & Trust | Authn, authz, secrets, injection, supply chain, privilege | ✅ |
| 09 | Observability & Detection | The meta-failure: inability to *see* failures | ✅ |
| 10 | Human & Organizational | Operator error, knowledge gaps, process and runbook failures | ✅ |
| 11 | Domain | The system's reason for existing; its unique failures | ⛔ plug-in |

Layers 01–10 are reusable verbatim. Layer 11 is rewritten per system. A system may have more than
one domain layer (11, 12, …) if it spans multiple genuinely distinct domains.

---

## 3. Axis 2 — SEMANTICS (how the failure manifests)

Borrowed from the classical distributed-systems failure model and generalized. **Every failure,
at every layer, is one (or a composite) of these six.** This is the axis that proves the
recurrence: the same six kinds appear in a CPU, a kernel, a database, and a cluster.

| Code | Semantic | Definition | Example at 3 different layers |
|------|----------|------------|-------------------------------|
| **CRASH** | Stop | A component halts and does nothing further (fail-stop if detectable). | process exits · service dies · node powers off |
| **OMISSION** | Drop | A component fails to do something it should (send-omission, receive-omission, compute-omission). | dropped packet · lost message · skipped callback |
| **TIMING** | Too early / too late / never | Correct value, wrong time. Includes performance failure (too slow) and infinite hang. | slow disk · GC pause · deadline miss |
| **VALUE** | Wrong answer | A response is produced but it is incorrect (corruption, miscompute, wrong type). | bit flip · off-by-one · wrong serialization |
| **STATE** | Stale / divergent / lost | Internal state no longer matches reality or another replica. | stale cache · split-brain · lost write |
| **BYZANTINE** | Arbitrary / two-faced | Inconsistent or adversarial behavior; different observers see different things; may be malicious. | lying node · corrupted-but-valid message · compromised dependency |

Severity tends to rise left-to-right: CRASH is the *kindest* failure (it is honest and
detectable); BYZANTINE is the *cruelest* (it is dishonest and may be undetectable). **A robust
system tries to convert failures leftward** — e.g. turn a silent VALUE corruption into a loud
CRASH via an assertion, because a crash you can see beats corruption you cannot.

---

## 4. Axis 3 — POSTURE (how the system must respond)

For every failure mode, the response must be **decided in advance**, not improvised during an
incident. There are exactly three postures; choosing is mandatory.

| Posture | Meaning | When to choose |
|---------|---------|----------------|
| **HALT** | Stop the affected scope immediately; do not proceed on suspect state. | When proceeding could cause irreversible harm or corrupt the system of record. |
| **DEGRADE** | Continue with reduced function; isolate the failure; backfill/repair later. | When the affected function is non-critical and the rest can safely proceed. |
| **IGNORE** | Accept the failure; it is within tolerance. | When the failure is cosmetic, self-correcting, or cheaper to absorb than to handle. |

**The scope of HALT matters as much as the choice.** "Halt the instrument" ≠ "halt the process" ≠
"halt the whole system." A good posture names its **blast-radius scope** (see Axis 4). The cardinal
sin is *not deciding* — an undecided failure mode defaults to "improvise," which under load means
"cascade."

---

## 5. Axis 4 — BLAST RADIUS (how far it spreads)

The scope of state or service affected when the failure is effective. Ordered narrow → wide.

```
REQUEST  ◀  ENTITY  ◀  COMPONENT  ◀  PROCESS  ◀  NODE  ◀  SYSTEM  ◀  CROSS-SYSTEM
```

- **REQUEST** — one operation; retriable in isolation.
- **ENTITY** — one logical unit of the domain (one account, one file, one instrument, one session).
- **COMPONENT** — one module/crate/service within a process.
- **PROCESS** — one OS process / one deployable.
- **NODE** — one host/VM/container host.
- **SYSTEM** — the whole application.
- **CROSS-SYSTEM** — escapes into depending or peer systems (the worst — it is no longer yours).

Blast radius and posture interact: a SYSTEM-radius failure with an IGNORE posture is a latent
disaster; an ENTITY-radius failure with a HALT posture is healthy containment.

---

## 6. Axis 5 — DETECTION (how you find out, and how fast)

A failure you cannot detect cannot be handled; it can only be suffered. Each FM records both a
**mechanism** and a **latency class**.

**Mechanism** (non-exhaustive, universal): assertion / invariant check · type system · checksum
/ hash · timeout / watchdog · heartbeat / liveness probe · sequence-gap detection · reconciliation
/ cross-check · metric threshold / anomaly · audit-log replay · external report (a human, a
dependency, a customer).

**Latency-to-detect class:**

| Class | Meaning |
|-------|---------|
| `compile` | Caught before it can run (best). |
| `immediate` | Caught synchronously at the point of failure. |
| `seconds` | Caught by a watchdog/heartbeat within seconds. |
| `sweep` | Caught by the next periodic reconciliation/job. |
| `post-hoc` | Caught only after the fact, in audit/analysis. |
| `never` | Not currently detectable (a known blind spot — the most important class to record). |

`never` entries are the backlog for [`relations/detection-coverage.md`](./relations/detection-coverage.md).

---

## 7. The Universal Failure Mode Record (UFMR)

Every entry in every layer file uses this exact shape. The fields are layer-independent, which is
what makes a kernel fault and an application bug describable side by side.

```markdown
### FM-<LAYER>-<NNN> — <short name>

- **Semantics:**   <CRASH | OMISSION | TIMING | VALUE | STATE | BYZANTINE> (may be composite)
- **Fault:**       <root cause — the dormant/triggering defect or condition>
- **Error:**       <the latent incorrect internal state the fault produces>
- **Failure:**     <the observable deviation at the service interface>
- **Detection:**   <mechanism> — <latency class>
- **Blast radius:** <REQUEST | ENTITY | COMPONENT | PROCESS | NODE | SYSTEM | CROSS-SYSTEM>
- **Posture:**     <HALT(scope) | DEGRADE | IGNORE>
- **Recovery:**    <how correct state/service is restored>
- **Propagates-to:** [FM-X-NNN, ...]   (what this becomes a fault for, one layer up/over)
- **Proven-by:**   <the adversarial test that triggers this and verifies the mitigation; or "—" if none yet>
```

### Field rules

1. **ID is stable forever.** `FM-<LAYER>-<NNN>` where `<LAYER>` is the two-digit layer number's
   short code (e.g. `ARCH`, `DATA`, `CONC`, `RES`, `BND`, `EXT`, `LIFE`, `SEC`, `OBS`, `HUM`,
   `DOM`). Never renumber; deprecate instead.
2. **One fault per record.** If a record needs "and also," it is two records linked by
   `Propagates-to`.
3. **Posture must name a scope** when it is HALT.
4. **`Proven-by` is a promise, not a wish.** `—` is honest and means "this is an untested
   mitigation" — a first-class backlog item, not a gap to hide.
5. **`Propagates-to` is the only relational glue.** Keep it accurate; the cascade graph is
   generated from it.

---

## 8. The conceptual model in one picture

```
            SEMANTICS  (how it manifests)
            CRASH  OMISSION  TIMING  VALUE  STATE  BYZANTINE
          ┌──────┬─────────┬───────┬──────┬──────┬──────────┐
   01 ARCH│      │         │       │      │ ▓▓▓  │          │
   02 DATA│      │         │       │ ▓▓▓  │ ▓▓▓  │   ▓▓     │
   03 CONC│      │   ▓▓    │ ▓▓▓   │      │ ▓▓   │          │   each cell = a
L  04 RES │ ▓▓   │   ▓▓▓   │ ▓▓▓   │      │      │          │   failure mode,
A  05 BND │      │   ▓▓    │       │ ▓▓▓  │      │   ▓▓▓    │   tagged with a
Y  06 EXT │ ▓▓▓  │   ▓▓▓   │ ▓▓▓   │      │ ▓▓   │   ▓▓     │   POSTURE and a
E  07 LIFE│ ▓▓   │         │       │      │ ▓▓▓  │          │   BLAST RADIUS,
R  08 SEC │      │         │       │      │      │   ▓▓▓▓   │   detected with a
   09 OBS │ ▓▓   │   ▓▓▓   │ ▓▓    │      │      │          │   MECHANISM at a
   10 HUM │      │         │       │ ▓▓   │      │   ▓▓     │   LATENCY class.
   11 DOM │      │   ▓▓    │       │ ▓▓▓  │ ▓▓▓  │          │
          └──────┴─────────┴───────┴──────┴──────┴──────────┘
```

The grid is the system. Empty cells are either genuinely impossible (good) or unexamined
(dangerous). Filling the grid honestly — including the `never`-detected and the `—`-proven cells —
is the point of the whole folder.

---

## 9. How to instantiate this framework for a new system

1. Copy this file and the README unchanged.
2. Copy layer files `01`–`10`; **delete the example entries**, keep the headings and the
   layer-intro prose (it is generic).
3. Write layer `11` (and `12`…) for your domain from scratch — this is the only creative work.
4. Walk every component of your system; for each, ask the six SEMANTICS at the relevant LAYERS;
   write a UFMR for each plausible cell.
5. Decide a POSTURE for each — *never leave one undecided.*
6. Fill `Propagates-to` to build the cascade graph.
7. Generate the three relational views in [`relations/`](./relations/).
8. Every `Proven-by: —` is a test you owe.
