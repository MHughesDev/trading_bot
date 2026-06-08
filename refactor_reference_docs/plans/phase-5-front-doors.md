# Phase 5 — Authoring front doors (JSON API + visual builder + MCP server)

> **Self-contained execution doc.** You need only: this file, [`../file-structure.md`](../file-structure.md),
> and the specs — especially [`04-strategy-system.md`](../spec/04-strategy-system.md) and
> [`08-mcp-server.md`](../spec/08-mcp-server.md).

## Phase goal

After this phase there are **three front doors that all produce the same artifact** — the frozen
`1.0` strategy-definition JSON: a JSON strategy API, a visual n8n-style builder in the React SPA, and
a thin MCP server. All three target one shared **validator** and the one runtime; none has a
privileged path. A definition authored by any door round-trips through the others.

## Prerequisites

- Phase 0 (strategy format frozen at `1.0`) and Phase 4 (runtime + validator-consuming endpoints,
  backtest) complete.
- **Decision gate Q3** must be resolved (done in Phase 0): the expression language, node types,
  `$each` fan-out, and tighten-only override rule are fixed. Front doors only *target* the format;
  they must not extend it.
- `legacy_python/mcp_server/` and `legacy_python/strategies/custom_strategy_store.py` contain prior
  behavior — read for parity.

## Invariants this phase must respect

- **One format, one validator, one runtime.** Every door emits the same canonical JSON and passes it
  through `crates/strategy-validator`. No door defines its own strategy format or its own runtime —
  that is the divergence trap the spec forbids.
- **Validation is mandatory before create/apply.** A malformed or risk-loosening definition is
  rejected with structured errors the caller (human or agent) can act on.
- **`risk_overrides` can only tighten.** Enforced by the validator, not by trusting the caller.
- **MCP has no order-placement tool.** It defines/runs strategies; any order they emit still passes
  the risk gate.

---

## Tasks

### P5-T01 — Strategy validator
- **Goal:** The single validator all three doors target.
- **Files:** `crates/strategy-validator/src/{lib,schema,expressions,risk}.rs`,
  `crates/strategy-validator/tests/rejects_loosening.rs`.
- **Context:** Per [`../spec/04-strategy-system.md`](../spec/04-strategy-system.md) and
  [`../spec/08-mcp-server.md`](../spec/08-mcp-server.md): `validate(def) ->
  Result<ValidatedDefinition, Vec<ValidationError>>`. `schema.rs` checks structure against the frozen
  `1.0` format; `expressions.rs` parses/validates the `condition.expr` grammar frozen in Phase 0;
  `risk.rs` enforces **tighten-only** `risk_overrides` against the global gate (reusing the rule from
  `crates/risk`). Errors are structured (path + reason) so an agent can self-correct.
- **Acceptance:** `rejects_loosening` proves a loosening definition is rejected with a structured
  error; a malformed expression is rejected; a valid definition validates.
- **Depends on:** Phase 0 (format), Phase 2 (`crates/risk` tighten-only rule).

### P5-T02 — JSON strategy API
- **Goal:** Create/validate/apply/stop strategies over REST against the frozen format.
- **Files:** `crates/api/src/routes/strategies.rs` (extend), persistence via
  `crates/storage/src/postgres/strategies.rs`.
- **Context:** Per [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md) REST list:
  `POST /api/strategies` (create from JSON → **validate** → persist), `POST /api/strategies/{id}/start`
  (apply over `asset_universe` via the runtime), `POST /api/strategies/{id}/stop`,
  `GET /api/strategies/{id}/config`. All creation/apply goes through `strategy-validator` first.
- **Acceptance:** a definition POSTed as JSON validates, persists, starts on the runtime, and stops;
  an invalid one returns structured errors and is not persisted.
- **Depends on:** P5-T01, Phase 4 (runtime).

### P5-T03 — Visual builder (React)
- **Goal:** An n8n-style node-graph editor that serializes to/from the same canonical JSON.
- **Files:** `frontend/src/builder/{BuilderCanvas.tsx,serialize.ts}`,
  `frontend/src/builder/nodes/` (node components per node type), plus a `StrategyPanel` entry to open
  it.
- **Context:** Per [`../spec/04-strategy-system.md`](../spec/04-strategy-system.md) §front doors and
  [`../spec/08-mcp-server.md`](../spec/08-mcp-server.md) §relationship: the editor's node graph
  **serializes to the strategy-definition JSON** and deserializes it back (`serialize.ts` round-
  trips, preserving node ids and optional positions). Node types map 1:1 to the frozen format's node
  types — the builder must not introduce node types the format doesn't have. Submitting calls the
  Phase-5 JSON API (`POST /api/strategies`), so validation is shared.
- **Acceptance:** a strategy built visually serializes to canonical JSON, validates via the API, and
  a JSON definition (e.g. an MCP-authored one) opens in the builder and round-trips without loss.
- **Depends on:** P5-T02.

### P5-T04 — MCP server (thin front door)
- **Goal:** A thin MCP server that lets an agent author/apply the same canonical JSON via tools.
- **Files:** `crates/mcp-server/src/lib.rs`,
  `crates/mcp-server/src/tools/{mod,discovery,authoring,lifecycle,backtest}.rs`,
  `apps/mcp-server/src/main.rs`.
- **Context:** Per [`../spec/08-mcp-server.md`](../spec/08-mcp-server.md): implement the tool set —
  `list_lanes`, `list_instruments` (discovery); `validate_strategy`, `create_strategy` (authoring,
  via `strategy-validator`); `apply_strategy` (initialize strategy on a specific instrument),
  `stop_strategy`, `list_strategies` (lifecycle, via the runtime); `run_backtest`,
  `get_backtest_result` (via Phase 4 market_simulator adapter + REST endpoints). **Guardrails:** no
  order-placement tool; validation mandatory before create/apply; overrides can only tighten
  (validator enforces); `min_trust_tier` respected; `apply_strategy` requires an `instrument_id`
  binding (strategy definitions are not pre-bound; the MCP caller supplies the instrument). The MCP
  server is a translator from agent intent → canonical JSON, nothing more.
- **Acceptance:** an agent can list lanes/instruments, validate + create a strategy, apply and stop
  it, and run/fetch a backtest — all producing/consuming the same canonical JSON; there is no tool to
  place a raw order; a loosening definition is rejected via `validate_strategy`.
- **Depends on:** P5-T01, Phase 4 (runtime + backtest).

### P5-T05 — Round-trip + parity test
- **Goal:** Prove the three doors share one format/validator/runtime.
- **Files:** extend `tests/` with a round-trip test: author via MCP tool → open/edit shape in builder
  serialize logic → submit via JSON API → run on the runtime.
- **Context:** The single source of truth is the canonical JSON; this test guards against any door
  drifting into its own format.
- **Acceptance:** a definition authored through any door validates and runs identically through the
  others; node ids/positions survive the builder round-trip.
- **Depends on:** P5-T02, P5-T03, P5-T04.

---

## Phase exit criteria

- [ ] `crates/strategy-validator` and `crates/mcp-server` implemented; `apps/mcp-server` runs.
- [ ] JSON API create/validate/apply/stop works and always validates first.
- [ ] The visual builder round-trips canonical JSON without loss and submits through the same API.
- [ ] The MCP server exposes only the spec's tool set, has **no** order tool, and enforces tighten-
      only + mandatory validation.
- [ ] All three doors target one format, one validator, one runtime; the round-trip/parity test passes.
