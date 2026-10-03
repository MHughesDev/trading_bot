# Phase 7 — Authoring — identical for people and models

**Completion: 71% (5 / 7 tasks).** U-7.1, U-7.2, U-7.4, U-7.5, U-7.6 DONE 2026-10-03; U-7.3 PARTIAL (not run over the platform's stored definitions; decision, LLM, ranking and universe-pipeline nodes refused by name); U-7.7 added (a deployment proposal a human decides).

**Requirement IDs:** SU-AUTH-01…06.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §10, §12; ADR-0045; AGENT-011.
**Depends on:** Phase 0; Phase 1 (the compiler) for validation.
**Blocks:** Phase 8.
**Migration range:** none.

A person in the editor and a model through tools produce the same canonical graph, get the same errors for the same mistake, and test through the same endpoints.

---

### U-7.1 SLv3 text ↔ graph — L
`DONE` 2026-10-03 — `strategy_lang::v3` (`lex.rs`, `parse.rs`, `print.rs`; the parser lowers straight
to the graph's wire JSON and lets `domain` deserialise it, so there is no separate AST or lowering
pass to drift). Keyword-led statements for the header, `param`, `constraint`, `input`, `universe`,
`let`, rules (`when … -> …`, `rule name group g when …`), `otherwise`, `select … where … top …
attribute …`, `triggers {…}` and `risk {…}`. Node values have a full expression grammar — `and`,
`or`, `not`, comparisons, arithmetic, negative literals folded — with positional call arguments
bound to the operator registry's port order; intents and other data use one tagged notation,
`tag(field: value)`, tagged by the field they sit in, which covers every intent without a grammar
per intent. FEAT-007 §10's example was rewritten in the grammar as built (its first-draft sugar is
noted there). Evidence: `v3::tests` 8/8 — both FEAT-007 graphs and a graph of every intent, size,
style, unit and anchor round-trip (`parse(print(g)) == g`, printing is a fixed point, same
revision); the spec's own ```` ```slv3 ```` block is read from the file, parsed and round-tripped;
a whitespace-squashed text has the same revision; mistakes are named with line, column and fix.

Extend `strategy-lang` from SLv2 to SLv3: `let` bindings for any operator, model calls with typed inputs, `when … -> intent` rules covering every intent, `select` blocks, triggers. Text and graph round-trip.

**Files:** `crates/strategy-lang/src/{lex,parse,ast,lower,print}.rs`.
**Acceptance:** every FEAT-007 example parses, lowers to the graph, prints back, and re-parses to the same graph; a formatting change does not change the revision hash.
**Traces:** ADR-0045; FEAT-007 §10.

### U-7.2 One validator — L
`DONE` 2026-10-03 — `strategy_lang::v3::validate`: text, raw JSON or a built graph → the same
findings for the same mistake. Text and JSON go through one structural check
(`StrategyGraph::from_json`, new, with a code per `GraphError`); then the compiler's diagnostics;
then the rules about a strategy as a strategy (empty name, no rules, a shadowed rule, an ATR period
nothing computes, a condition that can never fire, a warm-up longer than the window, a parameter
default outside its range — the compiler had never checked that); then one teaching table naming
the guide section. **Found and added:** a stop or target placed zero or a negative distance away
was caught by nothing — now `domain::strategy_graph::intent::check_literals`, with
`distance_not_positive` and `close_fraction_out_of_range` replacing the generic `bad_intent`.
Every one of `strategy-validator`'s 24 codes has an equivalent (`LEGACY`): 18 map to a code emitted
here for the same mistake with its fix; 6 cannot be written in a v3 graph and the table says why.
Evidence: `tests/v3_validate.rs` 5/5 (every legacy code covered and nothing else; each equivalent
emitted for its mistake with a fix and section; text and JSON give identical findings); the
builder door in `v3_builder.rs`. *Not yet:* `strategy-validator`'s grammar stays until Phase 8
retires the v1 path it validates.

Validation is the compiler's diagnostics plus the `teach.rs` fix/example table, served from `strategy-lang`. `strategy-validator`'s rules move here; its grammar is retired.

**Files:** `crates/strategy-lang/src/validate.rs`, `crates/strategy-validator/`.
**Acceptance:** every error code `strategy-validator` emits today has an equivalent here with the same teachable fix; the same malformed strategy yields the same error through text, builder ops and raw JSON.
**Traces:** ADR-0045 decision 2.

### U-7.3 v1 → v3 translator — L
`PARTIAL` 2026-10-03 — `strategy_lang::v3::translate_v1`: conditions → nodes (the v1 expression
grammar rewritten into SLv3: `feature('x')`, `bar('close')`, `param('p')`, `{{p}}`), signal +
`place_order` → `order` rules (fixed, percent-of-balance and model sizing), exits → a `protect` rule
while in a position plus `close` rules for max-holding-bars and on-signal (a sigma distance becomes
an `abs` distance of `k × rolling_std`), `model_forecast` / `inference` / `sizing` → `model` nodes
with outputs read as fields, `combine` → `weighted_mean`/`max`/`min`/sum, parameters and constraints
unchanged, the per-minute order limit carried. Every translation lists what changes (next-bar-open
fills, the risk gate, missing values hold, models now called, exits as venue orders). Refused by
name, never approximated: a parameter inside a feature name, a units position limit or a
per-second limit (dropping either would loosen risk), `risk_unit` sizing, inputs on another
instrument, and the `decision`, `llm_inference`, `model_rank` and universe-pipeline nodes.
Evidence: `tests/v3_translate.rs` 5/5 over the repository's corpus (the runtime fixture and every
SLv2 skeleton compiled to v1); `backtest/tests/translated_v1.rs` — a translated v1 EMA cross with an
on-signal exit backtests on the one engine and round-trips. **Not met as written:** "every stored
definition" was not run — the platform's database is not here; and the four refused AI and
pipeline node kinds still need a translation (a decision model's class map, an LLM node's prompt
as a contract, ranking and pipelines as selection strategies).

Translate every stored v1.0–1.5 definition: conditions and signals to nodes and rules; `place_order` to `order` intents; exits to `bracket`/`protect`; AI nodes to model nodes with derived contracts and adapters; universe nodes to a selection strategy. Emit, per strategy, the list of behaviours that change (fills at next open, models now called, risk gate applied).

**Files:** `crates/strategy-lang/src/translate_v1.rs`.
**Acceptance:** every stored definition translates or is refused naming the unsupported construct; each translation carries its behaviour-change list; a translated EMA cross compiles and backtests.
**Traces:** FEAT-007 §12.

### U-7.4 One API and one tool surface — M
`DONE` 2026-10-03 — `crates/api/src/routes/strategies_v3.rs`: `POST /api/v3/authoring/{validate,
compile,explain,build}`, `POST /api/v3/strategies` (save), `GET /api/v3/strategies/{id}`,
`GET /api/v3/catalogue`. Preview, backtest and selection runs keep their single routes, which already
take a graph. Saving a v3 graph appends a revision through the same transactional write as a v1
definition (`strategies::write_revision`, extracted so there is one write path).
`strategy_lang::v3::build` — `new`, `add_node`, `connect`, `set_arg`, `remove_node`, `add_rule`,
`set_param`, `set_universe`, `set_select`. Seven MCP tools (`*_strategy_v3`,
`get_strategy_catalogue`, in-app and MCP) in `tools/strategy_v3.rs`. Evidence: the thin-client
source scan (the module imports no language, compiler or engine and calls only mounted
`/api/v3` routes); `tests/v3_builder.rs` 5/5 — builder steps produce the same graph and revision as
the equivalent text; route invariants (anonymous refusal, no shadowed sibling) green. *Not here:*
`propose_deployment` — see U-7.7.

`validate`, `compile`, `explain`, `preview`, `backtest`, `run_selection`, `propose_deployment`, and builder operations `add_node`, `connect`, `add_rule`, `set_param` — served once and called by the UI and the MCP server alike.

**Files:** `crates/api/src/routes/strategies_v3.rs`, `crates/mcp-server/src/tools/strategy_v3.rs`.
**Acceptance:** every MCP strategy tool is a thin client of an API route (source scan); builder operations on a small-model grammar produce the same graph as the equivalent SLv3 text.
**Traces:** ADR-0045 decision 4; ADR-0022.

### U-7.5 Graph editor — L
`DONE` 2026-10-03 — `frontend/src/pages/StrategyGraphPage.tsx` at `/strategy/graph` (linked from the
v1 builder), with `components/strategy/v3/{graph.ts,GraphOpNode.tsx}` and `api/strategyV3.ts`. The
palette is `GET /api/v3/catalogue` (cross-sectional operators only in a selection); ports carry the
registry's types and `canWire` refuses a wire whose types do not fit, into a literal port, or that
would close a loop — React Flow's `isValidConnection` asks it before a wire exists; rules and the
selection sink are forms; the SLv3 view and the canvas are kept one strategy through
`/api/v3/authoring/build` (the browser never parses a strategy); Save is `POST /api/v3/strategies`.
Evidence: `graph.test.ts` 8/8 (typed ports, loop refusal, canvas ↔ graph round trip, the save
route) and `StrategyGraphPage.test.tsx` 3/3 rendered against a stand-in API (palette from the
catalogue, server text and findings shown, applied text replaces the graph, Save calls the agent's
route); typecheck and lint clean. Not exercised in a signed-in browser (no running backend here).

The UI edits the canonical graph: a palette generated from the operator, feature and model-contract registries; typed ports that refuse incompatible wires; rules and select sinks as forms; a synchronised SLv3 text view; validation inline; preview and backtest from the editor.

**Files:** `frontend/src/components/strategy/`.
**Acceptance:** a wire between incompatible ports cannot be made; editing the text view updates the graph and vice versa; the editor's save is the same API call the agent makes.
**Traces:** ADR-0045 decision 3; FEAT-007 §10.

### U-7.6 Agent catalogue and guide — M
`DONE` 2026-10-03 — `strategy_lang::v3::catalogue`: operators straight from the registry (it already
serialised), feature families from DATA-006, model versions with contracts from the registry
snapshot; served as `GET /api/v3/catalogue` and `get_strategy_catalogue`. `docs/agent/slv3-guide.md`
— hand-written sections, one per place a finding points, and operator and feature tables generated
between markers (`guide_block`). Evidence: `the_catalogue_is_the_registries_and_nothing_else`
(every operator and family, none twice, every family's example resolves); `tests/v3_guide.rs` —
the tables equal the registries (regenerate with `SLV3_GUIDE_WRITE=1`), and every section a finding
names is a heading.

### U-7.7 A deployment proposal a human decides — M
`NOT STARTED`

Found closing U-7.4. FEAT-007 §10 ends authoring in "a deployment proposal a human decides", and the
tool list names `propose_deployment`; the agent can save, validate, preview and backtest a v3
strategy, and `POST /api/deployments` deploys one directly — but there is no proposal for it. The
approval queue (`agent_lifecycle.proposal`, `routes/proposals.rs`) has `deploy_paper` for v1
automations only. Add a kind whose payload is a deployment spec (revision, target, capital), a
`propose_deployment` route and tool, and perform it on approval by recording and declaring the
deployment as `POST /api/deployments` does.

**Files:** `crates/ledger/src/proposal.rs`, `crates/api/src/routes/proposals.rs`, a migration for the
kind constraint, `crates/mcp-server/src/tools/strategy_v3.rs`.
**Acceptance:** an agent's proposal is inert until a human approves it; approval starts exactly the
proposed deployment; an agent token cannot approve.
**Traces:** FEAT-007 §10; AGENT-013 §6.8.

Regenerate the agent's strategy catalogue and authoring guide from the registries — operators with *use when*, feature families, models with their contracts — so the agent can only be told about what exists.

**Files:** `crates/mcp-server/src/tools/strategy_v3.rs`, `docs/agent/`.
**Acceptance:** every operator, family and contract in the registries appears in the catalogue and nothing else does (a test diffs them).
**Traces:** AGENT-011; ADR-0045.
