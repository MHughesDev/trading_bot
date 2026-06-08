# ADR-0001: Rust Modular Monolith with Satellite Collectors

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

A money-handling trading platform run by a small team has a primary requirement beyond raw
performance: legibility. The ability to ask "what did the system believe and do, and why" and get
a straight answer is the most valuable operational property. Microservice meshes trade this
legibility for independent deployability at a scale cost this team has not yet incurred.

At the same time, certain subsystems have genuinely independent failure modes: market-data
collectors reconnect on their own rhythm when a venue WebSocket drops, and taking down the entire
system because Coinbase's feed hiccuped is unacceptable. These components must be able to crash
and restart without affecting the core.

The system is written in Rust, which gives strong compile-time enforcement of crate boundaries
without any runtime communication overhead — the same boundary-enforcement that drives a
microservice architecture is available for free inside a single process.

## Decision

Build the platform as a **modular monolith plus satellite collector processes**, organized as a
Cargo workspace with clear crate boundaries:

- **One main binary** contains the Axum REST API + auth, the UI Streaming Gateway, the Strategy
  Runtime, the Risk Gate, the Execution Engine, and the Demand Manager.
- **Satellite processes** (one per venue/source) run the market-data collectors. These are the
  only things that are architecturally separated into their own processes from day one.
- **Crate boundaries are enforced in code** even when deployed as one binary. A crate is only
  extracted into its own process when a specific, measured pressure (independent failure mode or
  scaling bottleneck) forces it.

## Rationale

Co-locating the API gateway, strategy runtime, risk gate, and execution engine in one process
eliminates network hops, serialization overhead, and distributed-systems failure modes (split
brain, partial failures) for the components that most need to agree on state. In a synchronous
risk gate, there is no room for a timeout between "strategy intent" and "risk check" — they must
be in the same memory space.

Collectors are separated because their failure mode is genuinely independent: a venue goes silent,
the collector reconnects, and the core continues processing the other venue's data normally. This
is the one early separation that pays for itself immediately.

Cargo workspaces enforce the same architectural discipline that microservices enforce (crate A
cannot call crate B's private internals) at zero operational cost. Extracting a crate to its own
process later requires changing its communication channel, not rearchitecting it.

NATS JetStream was chosen over Kafka for the same reason — one lightweight binary whose ops weight
is justified at this scope. If the bus is ever outgrown, it is a reversible choice.

## Consequences

**Positive:**
- Maximum legibility: the entire execution path is traceable in one process, one set of logs, one
  profiler session.
- No distributed-systems failure modes between the risk gate and the execution engine.
- Crate boundaries are compile-time enforced, so the architecture is documented in code.
- Incremental extraction path is clear: measure pressure, extract one crate at a time.
- Simpler deployment and operations for a small team.

**Negative:**
- A panic in any non-isolated crate takes down the main binary. The risk gate, execution engine,
  and strategy runtime all share one failure domain.
- Vertical scaling limits apply to the main binary; horizontal scaling requires extracting
  components into separate processes first.
- Memory pressure from one subsystem (e.g., a large strategy runtime) affects all co-located
  subsystems.

**Neutral:**
- The Cargo workspace layout must be kept disciplined; without process boundaries as an external
  forcing function, crate dependency rules must be enforced by convention and CI.
- Strategy runtime workers can optionally be extracted later for isolation without any change to
  their crate logic.

## Alternatives Considered

### Option A: Full Microservice Mesh
Each component (API, strategy runtime, risk gate, execution) runs as a separate deployed service
communicating over NATS or HTTP. Not chosen because it multiplies operational complexity and
distributed-systems failure modes (network partitions, timeouts, serialization mismatches) before
the team has validated any of the business logic. Legibility suffers immediately.

### Option B: Single Fat Binary with No Crate Boundaries
One crate, everything inlined. Not chosen because it provides no compile-time enforcement of
architectural boundaries, making the codebase increasingly entangled over time. Crate extraction
later becomes a refactor instead of a configuration change.

### Option C: Separate Process per Plane (Data / Control / Strategy)
Separating the data plane, control plane, and strategy runtime from day one. Not chosen because
the risk gate specifically must synchronously check every order; separating the strategy runtime
from the risk gate and execution engine without that being forced by a real scaling need adds
latency and distributed-systems risk to the most safety-critical path.

## References

- [spec/01-architecture.md](../../refactor_reference_docs/spec/01-architecture.md) — deployment
  philosophy, planes table, end-to-end shape, failure-mode posture
- [spec/09-tech-stack.md](../../refactor_reference_docs/spec/09-tech-stack.md) — monorepo crate
  layout, Cargo workspace structure
