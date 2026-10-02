# Phase 7 — Authoring — identical for people and models

**Completion: 0% (0 / 6 tasks)**

**Requirement IDs:** SU-AUTH-01…06.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §10, §12; ADR-0045; AGENT-011.
**Depends on:** Phase 0; Phase 1 (the compiler) for validation.
**Blocks:** Phase 8.
**Migration range:** none.

A person in the editor and a model through tools produce the same canonical graph, get the same errors for the same mistake, and test through the same endpoints.

---

### U-7.1 SLv3 text ↔ graph — L
`NOT STARTED`

Extend `strategy-lang` from SLv2 to SLv3: `let` bindings for any operator, model calls with typed inputs, `when … -> intent` rules covering every intent, `select` blocks, triggers. Text and graph round-trip.

**Files:** `crates/strategy-lang/src/{lex,parse,ast,lower,print}.rs`.
**Acceptance:** every FEAT-007 example parses, lowers to the graph, prints back, and re-parses to the same graph; a formatting change does not change the revision hash.
**Traces:** ADR-0045; FEAT-007 §10.

### U-7.2 One validator — L
`NOT STARTED`

Validation is the compiler's diagnostics plus the `teach.rs` fix/example table, served from `strategy-lang`. `strategy-validator`'s rules move here; its grammar is retired.

**Files:** `crates/strategy-lang/src/validate.rs`, `crates/strategy-validator/`.
**Acceptance:** every error code `strategy-validator` emits today has an equivalent here with the same teachable fix; the same malformed strategy yields the same error through text, builder ops and raw JSON.
**Traces:** ADR-0045 decision 2.

### U-7.3 v1 → v3 translator — L
`NOT STARTED`

Translate every stored v1.0–1.5 definition: conditions and signals to nodes and rules; `place_order` to `order` intents; exits to `bracket`/`protect`; AI nodes to model nodes with derived contracts and adapters; universe nodes to a selection strategy. Emit, per strategy, the list of behaviours that change (fills at next open, models now called, risk gate applied).

**Files:** `crates/strategy-lang/src/translate_v1.rs`.
**Acceptance:** every stored definition translates or is refused naming the unsupported construct; each translation carries its behaviour-change list; a translated EMA cross compiles and backtests.
**Traces:** FEAT-007 §12.

### U-7.4 One API and one tool surface — M
`NOT STARTED`

`validate`, `compile`, `explain`, `preview`, `backtest`, `run_selection`, `propose_deployment`, and builder operations `add_node`, `connect`, `add_rule`, `set_param` — served once and called by the UI and the MCP server alike.

**Files:** `crates/api/src/routes/strategies_v3.rs`, `crates/mcp-server/src/tools/strategy_v3.rs`.
**Acceptance:** every MCP strategy tool is a thin client of an API route (source scan); builder operations on a small-model grammar produce the same graph as the equivalent SLv3 text.
**Traces:** ADR-0045 decision 4; ADR-0022.

### U-7.5 Graph editor — L
`NOT STARTED`

The UI edits the canonical graph: a palette generated from the operator, feature and model-contract registries; typed ports that refuse incompatible wires; rules and select sinks as forms; a synchronised SLv3 text view; validation inline; preview and backtest from the editor.

**Files:** `frontend/src/components/strategy/`.
**Acceptance:** a wire between incompatible ports cannot be made; editing the text view updates the graph and vice versa; the editor's save is the same API call the agent makes.
**Traces:** ADR-0045 decision 3; FEAT-007 §10.

### U-7.6 Agent catalogue and guide — M
`NOT STARTED`

Regenerate the agent's strategy catalogue and authoring guide from the registries — operators with *use when*, feature families, models with their contracts — so the agent can only be told about what exists.

**Files:** `crates/mcp-server/src/tools/strategy_v3.rs`, `docs/agent/`.
**Acceptance:** every operator, family and contract in the registries appears in the catalogue and nothing else does (a test diffs them).
**Traces:** AGENT-011; ADR-0045.
