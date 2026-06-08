# ADR-0007: Freeze Strategy Definition Format v1.0

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

The platform has three planned front doors for strategy authoring: a visual node-based builder (React UI), a JSON API (for programmatic strategy creation), and an MCP server (for AI-agent-driven strategy authoring). All three front doors produce and consume the same canonical strategy definition format.

Open question Q-3 from the platform design identified a critical sequencing risk: if any of these front doors is built before the strategy definition format is pinned to a stable v1.0, changes to the format after the fact will break existing users' strategies. Strategies are user data — they represent investment logic that users depend on. Breaking the format mid-development destroys trust and creates migration burden.

The format as sketched in the design docs includes several under-specified areas that could each independently break backward compatibility if changed after front doors exist:
- The expression language used in signal conditions
- Node type vocabulary (which node types exist and their required/optional fields)
- The semantics of `$bound_at_init` instrument binding
- The `asset_class` scoping rules (which asset classes a strategy can declare it operates on)
- How `risk_overrides` are validated as tighten-only (what fields exist and what tighter-only means for each)

These design questions interact: the expression language used in a `signal` node's condition references the node types and instrument fields that are available. Changing any one can require cascading changes to the others.

## Decision

The strategy definition format is frozen at **v1.0** before any of the three front doors (visual builder, JSON API, MCP server) are built. The v1.0 specification pins:

- The **expression language** for signal conditions (specific operators, literals, references to field paths)
- The **node type vocabulary**: the complete set of valid node types and their required and optional fields
- The **`$bound_at_init` semantics**: a strategy definition is not bound to a specific instrument at definition time; it is bound at instance creation time when a user selects an instrument. The format expresses this through `$bound_at_init` placeholders in instrument references.
- The **`asset_class` scoping rules**: a strategy declares the asset class(es) it operates on; the validator rejects initialization against an instrument whose asset class is not in the declared set
- The **`risk_overrides` schema and tighten-only validation**: the set of fields that can appear in `risk_overrides`, and the validator rule that each override value must be strictly more restrictive than the global limit it addresses

The frozen format lives in `crates/domain/src/strategy_def.rs`. A `schema_version: "1.0"` field is included in every strategy definition so the validator can reject or migrate definitions that predate the freeze.

This decision resolves Q-3.

## Rationale

A strategy definition is a user artifact that outlives any individual session. Unlike a UI layout preference, a strategy definition persists in the database and is reloaded on every system restart. If the format changes incompatibly after users have saved strategies, those strategies are either silently wrong (if the parser ignores unknown fields), explicitly broken (if the parser rejects them), or require a migration that the team may not be resourced to write.

Building the front doors first and freezing the format later inverts the dependency. The front doors are the consumers of the format — they must target a stable specification. Specifying the format first is the correct sequencing, identical to how an API schema is finalized before client SDKs are generated.

The `schema_version` field enables forward evolution: a future v1.1 or v2.0 format can be introduced while maintaining a migration path. Without a version field, format changes are silent and undetectable.

`$bound_at_init` binding (rather than baking instrument IDs into the definition) means a single strategy definition is reusable across instruments of the same asset class. The front doors emit the same parameterizable definition regardless of which instrument the user will initialize against.

## Consequences

**Positive:**
- All three front doors target an identical, stable format. There is no risk of the visual builder and JSON API producing subtly different formats.
- Users' saved strategies survive platform updates without migration.
- The `schema_version` field enables future format evolution with explicit migration paths.
- `$bound_at_init` semantics make strategy definitions portable across instruments within an asset class.
- `risk_overrides` tighten-only validation is enforced at strategy initialization time, before any order is submitted.

**Negative:**
- The format freeze requires upfront design effort before any front door code can be written. Underspecifying and iterating is not available as a workflow.
- Format freeze creates pressure to anticipate future use cases (options strategies, multi-leg strategies, sentiment-driven signals) at v1.0 design time. Features that are not designed in at v1.0 require a versioned format upgrade to add later.
- A mistake in the frozen format (e.g., an expression language operator that turns out to be ambiguous) cannot be corrected without a breaking change and a migration.

**Neutral:**
- The `crates/domain/src/strategy_def.rs` file is the single authoritative source for the format. Changes to this file require explicit versioning and migration planning. This constraint is the point.
- The visual builder, JSON API, and MCP server all have zero elevated privileges relative to the format — they all produce the same JSON document and all are validated by the same `strategy_def` validator.

## Alternatives Considered

### Option A: Freeze the Format After the Visual Builder Is Prototyped
Build the visual builder first as a design tool, use it to discover the format requirements, then freeze.

Not chosen because: the visual builder, once built, creates user expectations and potentially persisted strategy documents. Even a "prototype" becomes a de facto commitment. The cost of discovering format requirements through the visual builder is borne by users whose strategies break during the "discovery" phase.

### Option B: Versioned Format with No Freeze (Continuous Evolution)
Adopt semantic versioning for the format and migrate strategies automatically on each update.

Not chosen because: continuous format evolution without a stable version means the front doors must always target a moving spec. Automated migration is only reliable for additive changes; structural changes (expression language syntax, node type renames) require semantic understanding of the user's intent that cannot be automated. A v1.0 freeze provides the stability that migrations cannot.

### Option C: Separate Formats per Front Door (Builder Format, API Format)
Let the visual builder use its own internal format, translate to a canonical format on export.

Not chosen because: two formats with a translation layer between them creates a permanent maintenance burden and a source of subtle semantic divergence. The canonical format must be the only format. Front doors are serializers/deserializers of that format, not owners of their own.

## References

