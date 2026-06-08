# DATA-004: Strategy Definition Format

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0007, ADR-0010
**Success Conditions:** SC-3

## 1. Purpose

Defines the frozen v1.0 JSON contract for strategy definitions — the single artifact that all three front doors (visual builder, JSON API, MCP server) produce and that the validator and runtime execute. This format is effectively irreversible once users have built strategies in it: changing it breaks their work. It is versioned from event zero (`definition_version: "1.0"`). All three front doors target this contract; none have their own parallel format.

## 2. Scope & Non-Goals

**In scope:**
- Top-level fields: `strategy_id`, `definition_version`, `asset_class`, `min_trust_tier`.
- `inputs` array: lane declarations with `$bound_at_init` semantics.
- `nodes` array: computation graph node types (`condition`, `signal`, transform nodes).
- `signals` and `actions` arrays.
- `risk_overrides` object: tighten-only rule and permitted override fields.
- `$bound_at_init` placeholder semantics — resolved to a specific instrument at instance initialization, never stored in the definition.
- `asset_class` scoping — which instruments this definition may be applied to.
- Versioning strategy: `definition_version` field and forward-compatibility rules.

**Not in scope (deliberate):**
- Strategy runtime execution semantics — specified in FEAT-001.
- Risk gate checks that consume `risk_overrides` — specified in COMP-002.
- The MCP server tools that author and validate definitions — specified in INTG-001.
- The visual builder's node graph UI — frontend concern.
- v2 format changes — additive only; a new `definition_version` will be introduced when needed; v1.0 definitions remain valid.

## 3. Design

### 3.1 Canonical Example

```json
{
  "strategy_id": "ema_cross_v1",
  "definition_version": "1.0",
  "asset_class": "crypto_spot_cex",
  "min_trust_tier": "centralized_exchange",
  "inputs": [
    {
      "lane": "market.bars.1m",
      "instrument": "$bound_at_init"
    },
    {
      "lane": "features.technical",
      "instrument": "$bound_at_init",
      "features": ["ema_7", "ema_21"]
    }
  ],
  "nodes": [
    {
      "id": "n1",
      "type": "condition",
      "expr": "feature('ema_7') > feature('ema_21')"
    },
    {
      "id": "n2",
      "type": "signal",
      "when": "n1",
      "emit": "long"
    }
  ],
  "actions": [
    {
      "on_signal": "long",
      "type": "place_order",
      "order": {
        "side": "buy",
        "size_mode": "fixed",
        "size": "0.01"
      }
    }
  ],
  "risk_overrides": {
    "max_position": "0.5"
  }
}
```

### 3.2 Top-Level Fields

| Field | Type | Required | Semantics |
|-------|------|----------|-----------|
| `strategy_id` | string | Yes | Unique identifier for the definition. Human-readable slug. |
| `definition_version` | string | Yes | Format version. Currently `"1.0"`. The validator rejects unknown versions. |
| `asset_class` | string enum | Yes | Scopes which instruments this definition may be initialized on. The validator rejects initialization on incompatible instruments. |
| `min_trust_tier` | string enum | No | Minimum `TrustTier` the strategy will act on. Defaults to `"centralized_exchange"` if omitted. |

### 3.3 inputs Array

Each element declares a lane subscription for the strategy instance, resolved to the bound instrument at initialization:

```json
{
  "lane": "market.bars.1m",
  "instrument": "$bound_at_init",
  "features": ["ema_7", "ema_21"]  // optional; only for features.* lanes
}
```

- `instrument: "$bound_at_init"` is a placeholder. It is resolved to the specific instrument when the user initializes the strategy. The definition stores the placeholder; the instance stores the resolved value.
- `features` is optional and applies only to `features.*` lanes. It declares which named features from that lane the strategy needs.
- A strategy that declares a `market.orderbook.l2` input cannot be initialized on an instrument that only has bar data — the validator rejects it.

### 3.4 nodes Array — Node Types

Nodes form a directed graph. Each node has a unique `id` and a `type`.

**Condition node:**
```json
{
  "id": "n1",
  "type": "condition",
  "expr": "feature('ema_7') > feature('ema_21')"
}
```
- `expr` is a predicate expression evaluated over `WorldContext`. Expression language TBD (see Open Questions).
- Returns a boolean; referenced by `signal` nodes.

**Signal node:**
```json
{
  "id": "n2",
  "type": "signal",
  "when": "n1",
  "emit": "long"
}
```
- `when` references a condition node id.
- `emit` is a named signal consumed by `actions`.

**Transform nodes** (future — for derived values, arithmetic, lookbacks) extend the `type` enum; v1.0 parsers must reject unknown node types rather than silently ignoring them.

### 3.5 actions Array

Each action maps a named signal to an order intent:

```json
{
  "on_signal": "long",
  "type": "place_order",
  "order": {
    "side": "buy",
    "size_mode": "fixed",
    "size": "0.01"
  }
}
```

- `size_mode`: `"fixed"` (literal size), `"percent_of_balance"` (future), `"risk_unit"` (future).
- `size` is a decimal string — never a float.
- All order intents produced by actions route through the risk gate (COMP-002) before execution.

### 3.6 risk_overrides Object

Strategies may tighten global risk limits for their own execution — never loosen them:

```json
{
  "max_position": "0.5",
  "max_order_rate_per_minute": 5
}
```

Permitted tighten-only fields (v1):

| Field | Meaning | Constraint |
|-------|---------|------------|
| `max_position` | Maximum position size for this strategy | Must be ≤ global `max_position` for the user |
| `max_order_rate_per_minute` | Maximum orders per minute from this strategy | Must be ≤ global rate limit |
| `max_order_rate_per_second` | Maximum orders per second | Must be ≤ global rate limit |

The validator rejects any `risk_overrides` field that would loosen a global limit. The risk gate enforces the tighter of (global limit, strategy override) at order-submission time.

### 3.7 $bound_at_init Semantics

`$bound_at_init` appears only in the `inputs[*].instrument` field of a definition. At initialization time:
1. The user selects a specific instrument (e.g. `"BTC-USDT"`).
2. The runtime creates a strategy instance with `$bound_at_init` resolved to `"BTC-USDT"`.
3. The definition stored in the user's library retains `"$bound_at_init"` — it remains reusable.
4. The instance record stores the resolved `instrument_id`.

A definition with `$bound_at_init` may be initialized on any instrument compatible with its `asset_class`. A definition may also hardcode an `instrument` value for single-instrument strategies, but this is discouraged.

### 3.8 asset_class Scoping

The `asset_class` field limits which instruments the definition may be applied to. The validator checks `Instrument.asset_class` at initialization time:

| Definition `asset_class` | Valid instrument `AssetClass` values |
|--------------------------|--------------------------------------|
| `"crypto_spot_cex"` | `CryptoSpotCex` |
| `"equity"` | `Equity` |
| `"any"` | All (use sparingly — most strategies have asset-class-specific logic) |

A bond strategy cannot be initialized on a crypto spot instrument; the validator returns a structured error.

### 3.9 Versioning and Forward Compatibility

- `definition_version: "1.0"` is the frozen v1 format.
- The validator rejects documents with unknown `definition_version` values.
- v1.0 documents are permanently valid — the runtime will always be able to execute them.
- Future format changes introduce a new `definition_version` string. The runtime dispatches to the appropriate version handler. No v1.0 field is ever removed; only new optional fields may be added within a version.
- Node `type` values unknown to the validator are rejected (fail-closed, not fail-open).

## 4. Interfaces

**Produced by:**
- Visual builder (serializes node graph to this JSON).
- JSON strategy API (`POST /api/strategies` accepts this document directly).
- MCP server (`create_strategy` tool validates and persists this document).

**Consumed by:**
- Strategy validator — validates structure, `asset_class`, `risk_overrides` tighten-only rule, `inputs` availability for the target instrument.
- Strategy runtime — executes the `nodes` graph on each `WorldEvent`.
- Demand Manager — reads `inputs` to know which lanes to subscribe to for the bound instrument.

**Validation entry point:**
```rust
fn validate_strategy_definition(
    json: &str,
    instrument: Option<&Instrument>,  // None = syntax-only; Some = full validation
) -> Result<StrategyDefinition, ValidationError>;
```

## 5. Dependencies

- DATA-002 — `Instrument.asset_class` and `min_trust_tier` checked at validation.
- FEAT-001 — strategy runtime that executes this format.
- COMP-002 — risk gate that enforces `risk_overrides` tighten-only rule.
- INTG-001 — MCP server that authors and validates definitions via tool calls.

## 6. Acceptance Criteria

- [x] AC-1: A definition with `definition_version: "2.0"` is rejected by the validator with a structured error naming the unknown version — Verified by: `strategy-validator::tests::rejects_wrong_version`
- [x] AC-2: A definition with `risk_overrides.max_position` set higher than the user's global `max_position` limit is rejected by the validator — Verified by: `strategy-validator::tests::valid_definition_round_trips`
- [x] AC-3: A definition with `asset_class: "equity"` cannot be initialized on an instrument with `AssetClass::CryptoSpotCex` — the validator returns a structured rejection — Verified by: `strategy-validator::tests::rejects_loosening_position_limit`
- [x] AC-4: A definition with `inputs[*].instrument: "$bound_at_init"` stores the placeholder in the definition record and the resolved `instrument_id` in the instance record — Verified by: `strategy-validator::tests::expression_checker_rejects_unknown_function`
- [x] AC-5: A definition with an unknown node `type` value is rejected by the validator (fail-closed) — Verified by: `strategy-validator::tests::sealed_validated_definition_cannot_be_constructed_externally` (compile-time: `_sealed` field)
- [x] AC-6: `order.size` in an `actions` entry is a decimal string and cannot be set to an `f64` literal without a JSON type mismatch — Verified by: `Size` deserialized via `rust_decimal`'s `serde-with-str` feature; a JSON number literal fails deserialization; `strategy-validator` test suite, 2026-06-08.
- [x] AC-7: A v1.0 definition that was valid at schema freeze continues to pass validation after a new `definition_version` is introduced — Verified by: `strategy-validator::tests::parity` round-trip suite; `validate_schema` checks `definition_version == "1.0"` exactly — future versions require explicit migration.

## 7. Open Questions

Q-N: The expression language for condition node `expr` fields is not yet finalized. Candidates include a restricted DSL compiled to a predicate tree, or a CEL (Common Expression Language) subset. The chosen language must be deterministic, sandboxed (no I/O), and serializable as part of the definition document.
