# Trading Platform — Architecture Documentation

This is the foundational documentation set for an **event-driven, data-intensive trading
platform** intended to run locally first (you + a small group of trusted users on a private
network) while being architected so that new asset classes and data sources can be added
**without redesign**.

## What this system is

A one-stop trading platform that:

- Streams large volumes of market and ancillary data from many sources at different rates.
- Lets users **submit orders manually** and **view data manually** in a React UI.
- Lets users **build strategies** through three front doors that all target one format:
  - a visual, n8n-style **strategy builder** in the UI,
  - a **JSON strategy API**,
  - a dedicated **MCP server** for agent-driven strategy authoring.
- Runs those strategies **automatically** in a strategy runtime, on multiple assets at once,
  while manual trading on the same assets continues to work.
- Supports **historical replay / backtesting** from the same stored event universe, using the
  *same* strategy code that runs live.

## v1 scope (decided)

- **Asset classes:** stocks and crypto, order-book driven.
- **Deployment:** local-first, single trusted group, no multi-tenant isolation.
- **Capital:** see [10-open-questions.md](./10-open-questions.md) — real-vs-paper is the first
  gating decision before the execution layer is built.

Everything beyond v1 (options, DEX/on-chain, ETFs, bonds, social/sentiment) is a **direction**,
not a v1 deliverable. The foundations are built so each of those is "a new collector + a new
payload type + rows in the instrument metadata table" — never a redesign.

## The one principle

> Build the cheapest **correct** system one small team can fully trust with real money, where
> the parts most expensive to get wrong (event schema, timestamp semantics, money/ledger model,
> the risk gate, the strategy definition format) are decided first and everything else stays
> changeable.

Correctness here is overwhelmingly a **data-quality** property, not an architecture property.
A perfect event fabric still loses money if a float touches a price or a late trade poisons a bar.

## Reading order

| # | Document | What it covers |
|---|----------|----------------|
| 00 | [00-overview.md](./00-overview.md) | High-level system shape, planes, the mental model |
| 01 | [01-architecture.md](./01-architecture.md) | Control/data/UI/storage/strategy/replay planes; service layout |
| 02 | [02-data-model.md](./02-data-model.md) | Event envelope, payloads, **instrument metadata**, timestamps, identity |
| 03 | [03-data-engineering.md](./03-data-engineering.md) | Schemas, decimals, dedup, watermarks, late data, quality, trust tiers |
| 04 | [04-strategy-system.md](./04-strategy-system.md) | Three front doors, definition format, runtime, world state |
| 05 | [05-execution-and-risk.md](./05-execution-and-risk.md) | Order flow, the risk gate, reconciliation, the kill switch |
| 06 | [06-ui-and-streaming.md](./06-ui-and-streaming.md) | UI gateway, subscriptions, throttling, lossy vs canonical |
| 07 | [07-storage-and-replay.md](./07-storage-and-replay.md) | Storage split, partitioning, replay determinism |
| 08 | [08-mcp-server.md](./08-mcp-server.md) | MCP server scope, tools, how it targets the same JSON |
| 09 | [09-tech-stack.md](./09-tech-stack.md) | Rust crates, infra choices, crate/monorepo layout |
| 10 | [10-open-questions.md](./10-open-questions.md) | What is still undecided and gates what |
| 11 | [11-roadmap.md](./11-roadmap.md) | Build order: what ships first and why |
| 12 | [12-glossary.md](./12-glossary.md) | Terms used consistently across the docs |

## Status

This is **initial documentation** — a design baseline, not a spec frozen in stone. The items in
[10-open-questions.md](./10-open-questions.md) must be answered before the corresponding code is
written. "Decided" in these docs means "design decided," not "built and tested."
