# Phase 1 — Strategy Builder Tools

**Completion: 0% (0 / 3 tasks)**

**Goal:** Give an AI agent the ability to construct a `StrategyDefinition`
incrementally via discrete MCP tool calls, then validate and persist it in one
final `finalize_strategy` call. The builder holds server-side draft state so
the agent never has to assemble raw JSON itself.

---

## Background

The existing `create_strategy` tool accepts a fully-formed strategy JSON string.
That's correct for code-generation agents, but makes iterative construction
cumbersome. The builder approach lets the agent call focused tools
(`add_condition_node`, `add_action`, etc.) and get validation feedback
incrementally, matching the way the strategy builder UI works.

Draft state is keyed by a UUID (`draft_id`) that the agent receives from
`new_strategy_draft` and carries through subsequent calls. Drafts are
in-memory only (lost on server restart). A `discard_draft` tool allows
cleanup; drafts that are finalized are also removed from the store.

---

## Tasks

### ☐ F-1.1 Add StrategyDraft store to McpContext + draft lifecycle tools — S

**What:** Introduce a `StrategyDraft` type and add a `draft_store` field to
`McpContext`. Add two draft management tools: `new_strategy_draft` and
`discard_draft`.

**`StrategyDraft` shape** (internal, `crates/mcp-server/src/tools/builder.rs`):
```rust
pub struct StrategyDraft {
    pub strategy_id: Option<String>,
    pub definition_version: String,         // always "1.0"
    pub asset_class: Option<String>,
    pub min_trust_tier: Option<String>,
    pub inputs: Vec<InputDeclaration>,
    pub nodes: Vec<Node>,
    pub actions: Vec<Action>,
    pub risk_overrides: RiskOverrides,
}
```

**`McpContext` changes** (`crates/mcp-server/src/lib.rs`):
```rust
pub draft_store: Arc<Mutex<HashMap<Uuid, StrategyDraft>>>,
```
Add to both `new()` and `new_without_db()` constructors.

**New MCP tools:**
- `new_strategy_draft` — no params → returns `{ "draft_id": "<uuid>" }`
- `discard_draft` — params: `draft_id` → returns `{ "discarded": true/false }`

**Files:**
- `crates/mcp-server/src/lib.rs`
- `crates/mcp-server/src/tools/builder.rs` (new file)
- `crates/mcp-server/src/tools/mod.rs` (add `pub mod builder`)

**Acceptance criteria:**
- `new_strategy_draft` returns a unique UUID each call.
- `discard_draft` with a valid draft_id removes it and returns `discarded: true`.
- `discard_draft` with an unknown id returns `discarded: false` (not an error).
- `McpContext` compiles and all existing tests still pass.

---

### ☐ F-1.2 Implement 7 step-by-step builder tools — M

**What:** Add tools that mutate an existing draft. All require a `draft_id`
param; all return the current draft summary on success, or an error if
`draft_id` is unknown.

**Tools:**

| Tool | Params | Effect |
|------|--------|--------|
| `set_strategy_meta` | `draft_id`, `strategy_id` (slug), `asset_class`, `min_trust_tier?` | Sets top-level strategy fields |
| `add_strategy_input` | `draft_id`, `lane`, `instrument` (default `"$bound_at_init"`), `features?` (array) | Appends an `InputDeclaration` |
| `add_condition_node` | `draft_id`, `node_id`, `expr` | Appends `Node { id, kind: Condition { expr } }` |
| `add_signal_node` | `draft_id`, `node_id`, `when` (condition node_id), `emit` (signal name) | Appends `Node { id, kind: Signal { when, emit } }` |
| `add_strategy_action` | `draft_id`, `on_signal`, `side` ("buy"/"sell"), `size_mode` ("fixed"), `size` (decimal string) | Appends a `PlaceOrder` action |
| `set_risk_overrides` | `draft_id`, `max_position?`, `max_order_rate_per_minute?`, `max_order_rate_per_second?` | Overwrites `RiskOverrides` |
| `get_draft_summary` | `draft_id` | Returns current draft as JSON (no mutation) |

**Response shape** (mutation tools):
```json
{
  "draft_id": "<uuid>",
  "strategy_id": "ema_cross_v1",
  "inputs_count": 2,
  "nodes_count": 2,
  "actions_count": 1
}
```

**Error shape** (unknown draft_id):
```json
{ "error": "draft_not_found", "draft_id": "<uuid>" }
```

**Files:**
- `crates/mcp-server/src/tools/builder.rs` (extend from F-1.1)
- `crates/mcp-server/src/lib.rs` (add all 7 to `dispatch_tool` and `tool_definitions`)

**Acceptance criteria:**
- Each tool round-trips correctly: add an input, call `get_draft_summary`,
  see it reflected.
- `add_condition_node` with a duplicate `node_id` returns
  `{ "error": "duplicate_node_id" }` (not a panic).
- `add_signal_node` with a `when` that references a non-existent condition
  is allowed at this stage (deferred to `finalize_strategy` validation).
- All 7 appear in `tool_definitions()` with correct `inputSchema`.

---

### ☐ F-1.3 `finalize_strategy` — assemble draft → validate → persist — S

**What:** The terminal builder tool. Assembles the draft into a
`StrategyDefinition`, runs it through `strategy_validator::validate`, and on
success calls the existing `create_strategy` logic to persist it. On failure,
returns structured `ValidationError` items so the agent can fix them and retry
without starting over.

**Tool:** `finalize_strategy`
- Params: `draft_id`
- On **validation success**: persists the strategy, removes the draft, returns:
  ```json
  { "store_id": "<uuid>", "strategy_id": "ema_cross_v1", "valid": true }
  ```
- On **validation failure**: draft is **not** removed; returns:
  ```json
  {
    "valid": false,
    "errors": [
      { "path": "/nodes/1/when", "message": "references unknown condition node 'n0'" }
    ]
  }
  ```

**Implementation note:** Reuse `tools::authoring::create_strategy_from_def(ctx,
def)` (extract from the existing `create_strategy` which parses JSON before
persisting). `finalize_strategy` bypasses the JSON-parse step since it has the
typed `StrategyDefinition` directly.

**Files:**
- `crates/mcp-server/src/tools/authoring.rs` — extract `create_strategy_from_def`
- `crates/mcp-server/src/tools/builder.rs` — add `finalize_strategy`
- `crates/mcp-server/src/lib.rs` — add to dispatch + definitions

**Acceptance criteria:**
- A complete valid draft round-trips: `new_strategy_draft` → builder tools →
  `finalize_strategy` → draft removed, `store_id` returned.
- A draft with a bad expression (e.g. `"feature('ema_7') > >"`) returns
  validation errors and leaves the draft intact.
- `list_strategies` (existing tool) returns the newly persisted strategy.
- Attempting to `finalize_strategy` with an unknown `draft_id` returns
  `{ "error": "draft_not_found" }`.
