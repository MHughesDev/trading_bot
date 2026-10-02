# ADR-0045: One Language Front-End — the Graph Is Canonical; Text, Visual and Builder Are Views

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect, on the owner's direction that humans and AI "write strategies, use
them and test them exactly the same"
**Supersedes:** `strategy-validator`'s expression grammar; `strategy-runtime`'s interpreter and
bytecode compiler as grammar implementations; the domain EBNF as anything but documentation
**Carries forward:** ADR-0010 (three front doors, one canonical JSON) and ADR-0026's Layer 2
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §10; QA-4

## Context

The expression grammar is implemented in at least three places that disagree (the domain's frozen
EBNF, the validator's parser, the runtime's bytecode compiler and interpreter), with SLv2's compiler
on top. The agent authors through three surfaces (SLv2 text, JSON builder operations, raw JSON) and
people through a block editor, and each path validated differently: a sweepable strategy validated
in one place and was refused in another as recently as 2026-09-27.

## Decision

1. The **canonical graph** (`tbot.strategy/3`) is the strategy; its content hash is its revision.
2. **`strategy-lang` is the only front-end**: SLv3 text ↔ graph, the typechecker, the validator with
   teachable errors, the compiler to an `ExecutionPlan`, and the v1 translator. It reads three
   registries — operators, features (DATA-006), model contracts — and nothing else defines what is
   valid.
3. Three editors, one artefact: the visual graph editor, SLv3 text, and builder tool-calls
   (`add_node`, `connect`, `add_rule`) for small models that need a constrained grammar. All
   round-trip through the graph.
4. One API serves the UI and the agent identically (the MCP server stays a thin client, ADR-0022):
   validate, compile, explain, preview, backtest, run selection, propose deployment.

## Consequences

- Good: the same mistake produces the same error for a person and a model.
- Good: one place to add an operator; the palette, the text grammar and the agent catalogue follow.
- Bad: `strategy-validator` and parts of `strategy-runtime` are retired; their tests move or are
  rewritten against the new front-end.
