# ADR-0010: Three Front Doors, One Canonical Strategy JSON

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

The platform supports three distinct modes of strategy authorship:

1. **Visual builder (React UI):** A node-based graphical interface where users drag and connect nodes to define signals, actions, and risk parameters without writing code.
2. **JSON API:** A programmatic interface where developers or automated systems POST strategy definitions directly as structured JSON.
3. **MCP server (`crates/mcp-server`):** A Model Context Protocol server that exposes strategy authoring as tools, enabling AI agents to draft and submit strategies in natural language interactions.

These three surfaces serve very different user needs — visual composition for non-technical users, direct API access for developers, and AI-assisted authoring for future agent workflows. Without an explicit architectural decision, each front door might evolve its own internal format, its own validation logic, and its own special-case permissions, leading to:

- **Format fragmentation:** strategies authored in the visual builder may have subtle structural differences from strategies authored via the JSON API, making cross-tool compatibility unpredictable.
- **Privilege creep:** the MCP server or the internal API might acquire special-case permissions not available to the visual builder, creating security and auditability gaps.
- **Duplicated validation:** each front door reimplements portions of strategy validation, with inevitable divergence.
- **Strategy portability breakage:** a strategy saved by one front door cannot be loaded by another.

## Decision

All three front doors produce and consume the same **canonical strategy definition JSON format** as defined in `crates/domain/src/strategy_def.rs` (frozen at v1.0 per ADR-0007). No front door has elevated privileges relative to any other. All three front doors ultimately produce a valid strategy definition document and call `POST /strategies` (or the equivalent internal Rust API) to persist or execute it.

The MCP server is a thin translation layer: it accepts tool calls from AI agents, assembles a strategy definition JSON from the agent's instructions, validates it against the v1.0 schema, and submits it through the same API path used by the visual builder and JSON API. It has no privileged database access, no ability to bypass the risk gate, and no ability to submit malformed strategy definitions that the JSON API would reject.

The visual builder, JSON API, and MCP server are all **serializers/deserializers** of the canonical format. They are responsible for user experience — not for owning strategy semantics.

## Rationale

A single canonical format with no front-door-specific extensions guarantees that a strategy authored in the visual builder and a strategy authored via the JSON API are indistinguishable to the strategy runtime and the risk gate. This is a correctness property: the system's behavior cannot vary based on how a strategy was authored.

Uniform privilege between front doors is a security property. If the MCP server had elevated access (e.g., the ability to bypass risk checks for "agent-authored" strategies), a compromised AI agent or a prompt injection attack could exploit that elevation. Treating the MCP server as a thin, unprivileged front door eliminates this attack surface.

Validation being owned by the `domain` crate rather than by each front door means there is one authoritative validator. A strategy that passes the visual builder's client-side validation but fails the server-side `domain` validator is a bug in the front door's UI feedback, not a strategy that proceeds with invalid semantics.

This design also makes the front doors independently replaceable. If the visual builder UI is redesigned, or a new front door (e.g., a CLI tool, a Slack bot) is added, it simply produces the canonical JSON and calls the same endpoint. The runtime, risk gate, and storage layer do not require any changes.

## Consequences

**Positive:**
- Strategies are fully portable between front doors: a strategy authored in the visual builder can be downloaded as JSON and re-submitted via the API with identical semantics.
- The MCP server has no elevated privileges; AI-agent-authored strategies are subject to the same risk gate and validation as all other strategies.
- One validation implementation in `crates/domain` covers all three entry points; validation divergence is structurally impossible.
- Adding a fourth front door (CLI, Slack bot, etc.) requires only implementing the serialization layer and calling the existing API endpoint.

**Negative:**
- The visual builder's node-based representation must be serializable to and from the canonical JSON format. If the visual builder's internal graph model is richer than the canonical format allows, features must be deferred until the format supports them (which requires a versioned format change per ADR-0007 and must not break existing strategies).
- The MCP server cannot offer "smarter" strategy capabilities than the canonical format supports. AI agents are bounded by the format vocabulary, not by what the AI can express in natural language. This is a feature, not a bug — the boundary is explicit — but it does limit the MCP server's expressiveness.
- All three front doors share the same API rate limits and authentication requirements. The MCP server cannot have relaxed rate limits for "trusted agents."

**Neutral:**
- The MCP server's tools expose a natural-language interface that assembles strategy JSON; it is a UX layer over the canonical format, not a new strategy execution model.
- Front-door-specific UX validation (e.g., the visual builder highlighting an invalid connection in real time) is encouraged and valuable, but is advisory; the server-side `domain` validator is authoritative.
- The `schema_version: "1.0"` field in the canonical format applies equally to strategies submitted from all three front doors.

## Alternatives Considered

### Option A: Native Format per Front Door with Server-Side Translation
Each front door uses its own optimal format; the server translates each format to a canonical internal representation before executing.

Not chosen because: translation layers are a source of semantic loss and bugs. A visual builder format with richer graph semantics than the canonical format would either lose information in translation or require the canonical format to be extended to accommodate the visual builder — at which point the "separate format" has become the canonical format. Translation layers also mean three serialization formats to maintain and test.

### Option B: MCP Server with Direct Database and Risk-Gate Access
The MCP server bypasses the REST API and interacts directly with the database and strategy runtime, enabling more powerful agent workflows.

Not chosen because: direct database access from the MCP server bypasses authentication, authorization, and audit logging. Any AI agent (or prompt injection) that gains control of the MCP server would have unrestricted access to the platform's internal state. The thin-front-door model keeps the MCP server's blast radius limited to what the API allows.

### Option C: Visual Builder Format as the Canonical Format
Make the visual builder's node graph the canonical format and require the JSON API and MCP server to produce valid node graphs.

Not chosen because: node graphs are a presentation-layer concept. A JSON API that must express strategy logic as a visual node graph is cumbersome and inconsistent with how programmatic clients think about strategy definitions. The canonical format should be semantically clean, not shaped by the requirements of one particular user interface.

## References

- [spec/09-tech-stack.md](../../refactor_reference_docs/spec/09-tech-stack.md) — `crates/mcp-server` in workspace layout, `crates/domain/src/strategy_def.rs`
- ADR-0007 — Strategy definition format v1.0 freeze (the format these front doors target)
- [spec/10-open-questions.md](../../refactor_reference_docs/spec/10-open-questions.md) — Q3 strategy definition format, Q10 per-asset instances
