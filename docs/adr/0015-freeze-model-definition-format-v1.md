# ADR-0015: Freeze Model Definition Format v1.0

**Status:** Accepted
**Date:** 2026-06-15
**Deciders:** Platform team

## Context

The AI Model Studio (Set-H) introduces a first-class model identity, training, evaluation, and promotion
lifecycle to the platform. Like strategies (ADR-0007), models are user artifacts that outlive any
individual session: they are persisted in Postgres, versioned, and referenced by strategies at runtime.

Before any Model Studio front door (visual builder, JSON API, MCP server) is built, the model definition
format must be frozen. The same sequencing risk that applied to strategies applies here: if any front door
is built against an unstable format, format changes after the fact will break saved model definitions and
destroy trust.

The format contains several under-specified areas that could independently break backward compatibility:
- The `model_kind` vocabulary (which kinds exist and which are trainable vs. external adapters)
- The `framework x kind` compatibility matrix (which frameworks are valid for which kinds)
- The `target` schema (field, horizon, transform)
- The `adapter` schema (provider, model, endpoint, cost) used only by `external_llm_adapter` kind
- The `runtime` enum (Python vs. Rust sidecar)
- The `schema_version` evolution rule (when to increment and how to migrate)

These areas interact: the `framework` field is only valid for certain `model_kind` values, and the
presence of `target` vs. `adapter` depends on whether the kind is trainable.

ADR-0007 established the precedent for this pattern. ADR-0015 mirrors it exactly for the model domain.

## Decision

The model definition format is frozen at **v1.0** before any Set-H front door is built.
The frozen format lives in `crates/domain/src/model_def/`.

The v1.0 specification pins:

- **The `model_kind` vocabulary**: `forecaster`, `signal_ranker`, `trade_decision`, `risk_sizing`,
  `embedding`, `external_llm_adapter`. All kinds except `external_llm_adapter` are trainable.

- **The `framework x kind` compatibility matrix**: `external_llm_adapter` and `embedding` kinds
  require `framework: external_api`. All other trainable kinds require a non-`external_api` framework
  (`xgboost`, `lightgbm`, `sklearn`, `torch`). Mixed combinations are rejected by the validator.

- **The `target` schema**: a `TargetSpec` carries `field` (return/price/volatility/direction/action/
  score/size_fraction), `horizon` (ISO-8601-ish token e.g. `"1h"`, `"4h"`, `"1d"`), and an optional
  `transform` (none/logret/zscore, default none). Required for trainable kinds, forbidden for adapters.

- **The `adapter` schema**: an `AdapterSpec` carries `provider`, `model`, `endpoint`,
  `default_params` (arbitrary JSON), and `cost_per_1k_tokens` stored as a decimal string (ADR-0002).
  Required for `external_llm_adapter`, forbidden for trainable kinds.

- **The `runtime` enum**: `python` (default) or `rust`. Determines which sidecar is invoked in Phase 2.

- **The `schema_version` evolution rule**: the field is required; the validator rejects any value other
  than the current `DEFINITION_VERSION` constant. A future format change must bump the constant and
  provide an explicit migration path, identical to ADR-0007's strategy format rule.

This decision is a prerequisite for all Set-H phases.

## Rationale

A model definition is a user artifact that is reloaded on every system restart, referenced by strategies
at runtime, and tracked through a version history. If the format changes incompatibly after users have
saved model definitions, those definitions are either silently wrong, explicitly broken, or require a
migration that the team may not be resourced to write.

The same argument that applied to strategy definitions in ADR-0007 applies here without modification.
The only new consideration is the `trainable vs. adapter` duality: trainable kinds need a `target` spec
and no `adapter` block; adapter kinds need an `adapter` block and no `target`. Pinning this distinction
at v1.0 prevents future ambiguity about which fields are required for which kinds.

ADR-0002 (no f64 for money/price) applies to `cost_per_1k_tokens`: this is a monetary quantity and must
be a decimal string, not a float. `confidence` (0..1 calibration metric) is not money and may remain f64.

## Consequences

**Positive:**
- All Set-H front doors target an identical, stable format.
- Users' saved model definitions survive platform updates without migration.
- The `schema_version` field enables future format evolution with explicit migration paths.
- The `target` / `adapter` mutual exclusivity is enforced at definition time, before any training run
  is submitted.
- ADR-0002 compliance is enforced structurally: `cost_per_1k_tokens` is a `String`, not f64.

**Negative:**
- The format freeze requires upfront design effort before any front door code can be written.
- Unanticipated model kinds or frameworks require a versioned format upgrade to add later.
- A mistake in the frozen `framework x kind` matrix cannot be corrected without a breaking change.

**Neutral:**
- `crates/domain/src/model_def/` is the single authoritative source for the format.
- Changes to this module require explicit versioning and migration planning.
- The visual builder, JSON API, and MCP server all produce the same `ModelDefinition` JSON and are all
  validated by the same `validate::validate` function.

## Alternatives Considered

### Option A: Embed Model Definition in Strategy Definition
Extend `StrategyDefinition` with an optional embedded model block instead of a separate definition type.

Not chosen because: model definitions have an independent lifecycle (training, evaluation, promotion)
that does not map to strategy instance lifecycle. A model may be shared by multiple strategies. A
separate type with its own `schema_version` is the correct encoding.

### Option B: Use serde_json::Value for the Entire Definition
Store the definition as an opaque JSON blob with no typed schema.

Not chosen because: an opaque blob cannot be validated at definition time. The compatibility matrix
(kind x framework), the `target`/`adapter` mutual exclusivity, and the `schema_version` check all
require typed access to the fields. An opaque blob defers all errors to runtime.

### Option C: Separate Schemas per Kind
Define a separate Rust struct for each `model_kind`, using a serde tagged union.

Not chosen because: the format would not be representable as a single flat JSON document. Users and
front doors would need to know the kind before deserializing, creating a two-pass parse. A single
`ModelDefinition` struct with optional `target` and `adapter` fields is simpler and compatible with
the existing strategy definition pattern.

## References

- ADR-0007: Freeze Strategy Definition Format v1.0 (precedent and structural template)
- ADR-0002: Decimal Money Newtypes -- No f64 (applies to `cost_per_1k_tokens`)
- `crates/domain/src/model_def/` -- frozen format implementation
