# specs/

Feature, component, data model, integration, and system-overview specification files. Specs define what something should do with enough precision for a developer (human or AI) to implement it, and for a tester to verify it.

---

## What Belongs Here

- Feature specs ("strategy system")
- Component specs ("execution and risk gate design")
- Data model specs ("event envelope schema")
- Integration specs ("MCP server front door")
- System overview specs ("full system architecture map")

## What Does Not Belong Here

- Architecture decisions → `adr/`
- Research that informed the spec → `research/`
- Project plans and milestones → `plans/`

---

## Spec Types

| Type | Code | Meaning |
|------|------|---------|
| Feature | `FEAT` | User-visible or system-level behavioral feature |
| Component | `COMP` | Internal system component: interface, responsibilities, constraints |
| Data | `DATA` | Data model or schema: fields, types, invariants, relationships |
| Integration | `INTG` | Integration with an external system or protocol |
| System-overview | `SYS` | Top-level map linking all component and feature specs for a complex system |

Numbering is per type (`FEAT-001` and `COMP-001` may both exist). Cite any line as `<SPEC-ID> §N` (e.g. `COMP-002 §3.1`).

---

## Lifecycle

`Draft` → `Ready for Review` → `Approved` → `Implemented`

Retirement states: `Deprecated` (nothing replaces it) or `Superseded by <SPEC-ID>`. Specs are never deleted — they retire so history stays readable.

---

## Index

| ID | File | Title | Status | ADR(s) | Success Conditions |
|----|------|-------|--------|--------|--------------------|
| SYS-001 | [SYS-001-system-overview.md](./SYS-001-system-overview.md) | System Overview | Draft | ADR-0001, ADR-0003 | SC-1 through SC-7 |
| DATA-001 | [DATA-001-event-envelope-and-payloads.md](./DATA-001-event-envelope-and-payloads.md) | Event Envelope and Payloads | Draft | ADR-0002, ADR-0009 | SC-1, SC-7 |
| DATA-002 | [DATA-002-instrument-metadata.md](./DATA-002-instrument-metadata.md) | Instrument Metadata | Draft | ADR-0001 | SC-5 |
| DATA-003 | [DATA-003-timestamps-and-identity.md](./DATA-003-timestamps-and-identity.md) | Timestamps and Identity | Draft | ADR-0008 | SC-3, SC-4 |
| DATA-004 | [DATA-004-strategy-definition-format.md](./DATA-004-strategy-definition-format.md) | Strategy Definition Format | Draft | ADR-0007, ADR-0010 | SC-3 |
| FEAT-001 | [FEAT-001-strategy-system.md](./FEAT-001-strategy-system.md) | Strategy System | Draft | ADR-0007, ADR-0008, ADR-0010, ADR-0011 | SC-2, SC-3, SC-4 |
| COMP-001 | [COMP-001-data-quality-and-ingestion.md](./COMP-001-data-quality-and-ingestion.md) | Data Quality and Ingestion | Draft | ADR-0003, ADR-0009, ADR-0011 | SC-3, SC-6 |
| COMP-002 | [COMP-002-execution-and-risk-gate.md](./COMP-002-execution-and-risk-gate.md) | Execution and Risk Gate | Draft | ADR-0005, ADR-0006 | SC-2, SC-6, SC-7 |
| COMP-003 | [COMP-003-ui-streaming-gateway.md](./COMP-003-ui-streaming-gateway.md) | UI Streaming Gateway | Draft | ADR-0001, ADR-0011 | SC-3 |
| COMP-004 | [COMP-004-storage-and-replay.md](./COMP-004-storage-and-replay.md) | Storage and Replay | Draft | ADR-0004, ADR-0008, ADR-0009 | SC-3, SC-4 |
| INTG-001 | [INTG-001-mcp-server.md](./INTG-001-mcp-server.md) | MCP Server | Draft | ADR-0010 | SC-2 |
