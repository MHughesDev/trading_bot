# AGENT-006: The Tool Catalogue — what the research agent can call, how a tool is defined, and how the catalogue grows without growing the prompt

**Status:** Proposed (implementation spec; ready to build)
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), from the code in `crates/mcp-server`, `crates/harness/src/registry.rs`, `crates/api/src/agent/bridge.rs`, `crates/ledger/src/audit.rs`, and the research brief `docs/research/quant-agent-toolbox-uncommon-tools.md`
**Depends on:** AGENT-005 (context assembly: exposure budgets, description weight budget, `run_python`)
**Supersedes:** AGENT-002 §6 (the catalogue tables; the `tbot` SDK contract in §3–§5 stands and is out of scope here)
**Implementer notes:** sections marked **[DO]** are concrete changes with file and symbol anchors (line numbers as of 2026-09-14). Tool signatures are given as `name(params) -> {fields}`; the JSON Schema is derived from them in the manner of every existing definition in `crates/mcp-server/src/lib.rs:611`.

---

## 0. Why this spec exists (the measured state)

**What is in code.** `crates/mcp-server/src/taxonomy.rs:42` classifies 66 tools in 13 namespaces (plus `fs` from the harness, `crates/harness/src/registry.rs:145`). `INTERNAL_AGENT_TOOLS` (`lib.rs:495`) exposes 56 of them to the internal agent, plus the harness's own `search_tools`, `record_finding`, `finish_task` and four `fs` tools. Two files holding 13 of those tools (`tools/falsification.rs`, `tools/research_tools.rs`) are untracked in git: the newest part of the catalogue is not committed.

**What the spec says.** AGENT-002 §6 lists ~95 capabilities. Of those, the `analysis.*` suite (§6.3, 13 tools), `strategy.explain/diff/reconcile/preview_signals/forensics`, `bt.explain_trade/equity_curve`, every `exp.*` diagnostic beyond sweeps and gates (`robustness`, `alpha_chain`, `family_test`, `capacity`, `posterior`, `decompose`, `factors`, `decay`, `dossier`), `models.evaluate/compare/importance/calibrate`, `datasets.*`, `text.*`, `memory.*` and `portfolio.positions/paper_vs_backtest` are **not built**. Its status column is also stale the other way: it marks `models.train` and `skills.*` as N, and both exist (`train_model`, `create_skill`/`run_skill`).

**How a tool is registered today.** Four hand-maintained places must agree: the definition literal (`lib.rs:611` `all_tool_definitions`, or `tools/<file>.rs::definitions()` appended at `lib.rs:1103-1105`), the dispatch arm (`lib.rs:333` `dispatch_tool`), the taxonomy entry (`taxonomy.rs:42`), and the profile list (`lib.rs:495`). Three tests keep them honest (`taxonomy.rs:201` `every_tool_is_classified`, `:227` `the_taxonomy_has_no_entries_for_tools_that_do_not_exist`, `:247` `no_tool_is_classified_twice`) and an unclassified tool fails closed as `destructive` (`lib.rs:590` `stamp`). Approval gating is a fifth place: `crates/ledger/src/audit.rs:85` `Envelope::for_action` matches **action names** against `SPEND_ACTIONS`, `PROMOTION_ACTIONS`, `SEALED_HOLDOUT_ACTIONS`, `TIER3_MEMORY_ACTIONS`.

**Two defects found while reading.**
1. `PROMOTION_ACTIONS` lists `promote_model` (`audit.rs:98`); the tool is named `promote_model_version` (`taxonomy.rs:149`). `Envelope::for_action("promote_model_version")` returns `None`, so a promotion — an action AGENT-001 §15 says needs a human — is not gated on either driver. No test compares envelope lists to tool names.
2. Ten tool descriptions are 1.2–2.1 KB and one (`run_skill`) is 5.2 KB of source; they are documentation, not descriptions, and on the flat tier they are cut to their first sentence anyway (`registry.rs:594`). AGENT-005 §5.4 sets the budget; this spec says where the prose goes.

**Namespace size is a hard constraint.** `crates/api/src/agent/bridge.rs:355` asserts no namespace holds more than 16 tools, because a local step routes to one namespace and must be able to see all of it. Today's largest is `research` at 14. Every addition below respects the cap, and two namespaces are split to keep it.

## 1. Principles (normative)

- **P1 — The catalogue may be any size; the prompt may not.** Growth is absorbed by routing (AGENT-005 §5), not by exposure. A tool that "should always be visible" is a routing bug or a core-tool proposal, never an exception.
- **P2 — One definition, five consumers.** A tool is declared once; its taxonomy entry, profile membership, dispatch arm, approval envelope and documentation are derived or checked from that declaration. Hand-maintained parallel lists are what produced defect 1.
- **P3 — Answers, not data.** A tool returns a summary the model can reason over (≤ 400 tokens by default), the ids that make it citeable, and a handle to the full result. Never a table the model has to read.
- **P4 — Evidence is an id.** Every result that could support a `finish_task` claim carries ids (`experiment_id`, `study_id`, `run_id`, `ticket_id`, `audit_id`, `finding_id`). The termination check accepts only ids it has seen in this run (`crates/harness/src/drive.rs:1830` does this on the local tier; the hosted tier must too).
- **P5 — Spending is a trial.** Any tool that runs the simulator registers a trial before it runs (INV-16, the reason `create_backtest` was removed: `taxonomy.rs:78-80`). Adversarial re-runs are trials of `kind: falsification`; they raise N_eff, and the selection-bias arithmetic reads them.
- **P6 — Three implementation tiers, one contract.** A tool is *typed* (Rust, in `crates/mcp-server`), *templated* (a Python template under `run_python`, AGENT-005 §8), or a *workflow* (a fixed sequence of the other two run server-side and exposed as one tool). The model cannot tell which; the contract (§2.3) is identical.
- **P7 — Descriptions say when, the guide says how.** A description is ≤ 2 sentences: when to call it, what it returns, what it costs. Everything else lives in the served guide under `## Tool notes` and is fetched on demand.
- **P8 — Rigor in the tool.** Multiple-testing correction, null comparison, leakage checks and cost realism are inside the tool or the workflow, never left for the model to remember. A 7B calling `selection_bias_verdict` gets the same statistics a frontier model gets.

## 2. The catalogue model

### 2.1 Canonical namespaces (the routing unit)

| Namespace | Holds | Exists | Cap-check after this spec |
|---|---|---|---|
| `core` | `get_authoring_guide`, plus harness-owned `search_tools`, `record_finding`, `finish_task` | yes | 4 |
| `discovery` | what exists: lanes, instruments, asset classes, initialised assets | yes | 5 |
| `data` | bars, asset init, data quality, provenance, survivorship, funding history | yes | 8 |
| `analysis` | statistics of a series: returns, volatility, regime, distributions, stationarity, cointegration, analogs, process fits | **new** (AGENT-002 §6.3, unbuilt) | 14 |
| `attribution` | what a signal *is*: factor spanning, style, half-life, decay, crowding, health check | **new** | 8 |
| `strategy` | validate/create/get/list, explain, diff, preview signals | yes | 8 |
| `builder` | the draft builder | yes, MCP-only | 10 |
| `backtest` | list/get/stop/rerun/delete/compare/wait, trade list | yes | 9 |
| `research` | experiments, sweeps, studies, funnel, nulls, gates | yes | 14 |
| `audit` | honesty arithmetic: deflated Sharpe, PBO, family-wise tests, holdout budget, leakage checks, metric integrity | **new** | 12 |
| `falsification` | refutation: kills, evaluator defects, oracle, frontier, similar trials, ablation, fragility, jitter, cost shock, regime failure, reshuffle | yes (+6) | 13 |
| `execution` | realism: venue constraints, funding costs, capacity curve, fill simulation, divergence attribution | **new** | 6 |
| `portfolio` | book state and construction: rollup, activity, status, orders, shrunk covariance, allocation, control limits | yes (+3) | 7 |
| `model` | registry, training, promotion, importance, calibration, drift | yes (+3) | 9 |
| `memory` | negative results, hypothesis dedup, signal registry | **new** | 4 |
| `report` | claims, rechecks, critique, dossier | **new** (harness-side) | 4 |
| `lifecycle` | the research stages: preregister, hypothesise, design, refine, compare, reproduce, propose, open_question, get_stage; plus the read-only session views get_session, list_agenda, get_digest | **new** (AGENT-013 §8, AGENT-014 §10) | 12 |
| `code` | `run_python` | **new** (AGENT-005 §8) | 1 |
| `skills` | create/list/run | yes | 3 |
| `web` | search, fetch | yes | 2 |
| `fs` | the agent's own folder | yes (harness) | 4 |
| `automation` | standing rules | yes, MCP-only | 4 |

Twenty-two namespaces, none above 16 (2026-09-14/15: AGENT-013 §8 adds `lifecycle`, which also holds AGENT-014 §10's three read-only session views; `research` gains only `define_campaign`/`get_campaign`, reaching the cap of 16 exactly, so any further `research` tool must retire one). The `analysis`/`attribution` and `audit`/`falsification` splits exist only because of the 16-cap; they also happen to be the right routing boundary (describing a series vs. explaining a signal; arithmetic about trials vs. attacks on a strategy).

[DO] Replace `taxonomy.rs`'s prose examples with this table in the module doc, and add a test `every_namespace_is_canonical` that rejects a tool whose namespace is not in this list.

### 2.2 Implementation tiers

| Tier | Where it lives | Runs as | Use it for |
|---|---|---|---|
| **Typed** | `crates/mcp-server/src/tools/<namespace>.rs`, backed by an API route in `crates/api/src/routes/` | Rust | anything that touches the ledger, the funnel, the registry, or needs a transaction |
| **Templated** | `apps/skill-runner/templates/<namespace>/<name>.py` | the existing skill runner (`crates/skills/src/runner.rs:152`), data pre-materialised by the harness to a parquet path in the payload (a skill has no DB and no network: `taxonomy.rs:285`) | statistics over a series: the whole `analysis` namespace, most of `attribution` and `audit` |
| **Workflow** | `crates/mcp-server/src/tools/workflows.rs` | server-side sequence of typed/templated calls with decision points; one trial family; one result | the multi-step batteries in §6 |
| **Harness-owned** | `crates/harness/src/registry.rs` (`with_core`) and `crates/api/src/agent/bridge.rs:34` (`HARNESS_OWNED`) | inside the loop, never dispatched | `search_tools`, `record_finding`, `finish_task`, `report.*` |

A templated tool is still a catalogue entry with a name, namespace, risk and schema. The schema is declared in the template's header (a JSON docstring) and a build-time test asserts the catalogue and the templates agree (§3.3). On profiles with `code_execution: denied` the templated tools are still exposed — the *harness* runs the template; only free-form `run_python` is hidden. That is what makes P8 hold on a 7B.

### 2.3 The per-tool contract

Every tool, in every tier, declares:

```rust
pub struct ToolSpec {
    pub name: &'static str,             // verb_noun, unique, ≤ 32 chars
    pub version: u32,                   // bumped when behaviour or the MEANING of a field changes; stamped on every trial ticket (§2.7)
    pub status: ToolStatus,             // Active | Deprecated { since, superseded_by, reason } (§2.7)
    pub namespace: &'static str,        // §2.1
    pub risk: Risk,                     // read | write | destructive | outbound (taxonomy.rs)
    pub description: &'static str,      // ≤ 2 sentences, ≤ 1,200 B rich; first sentence ≤ 400 B (AGENT-005 §5.4)
    pub input_schema: Value,            // rich; flat form is derived (registry::flatten), ≤ 2 optional params after flattening
    pub output: OutputBudget,           // summary tokens (default 400), hard cap = profile max_tool_result_bytes, handle on overflow
    pub evidence: &'static [&'static str], // which returned fields are ids the model may cite (P4); empty = not citeable
    pub spends_trial: TrialKind,        // None | Search | Falsification — registers a ledger trial before running (P5)
    pub envelope: Envelope,             // audit.rs: None | Spend | Promotion | SealedHoldout | Tier3Memory — DERIVED from name lists, asserted equal
    pub profiles: &'static [ToolProfile], // Mcp, InternalAgent
    pub tier: Tier,                     // Typed | Templated { template } | Workflow { steps } | HarnessOwned
    pub notes_section: Option<&'static str>, // heading in the guide's `## Tool notes` (P7)
}
```

[DO] Introduce `ToolSpec` in `crates/mcp-server/src/spec.rs`. Migrate the existing definitions file by file (`tools/<ns>.rs::specs() -> Vec<ToolSpec>`), and **generate** from the specs: `all_tool_definitions()` (name/description/inputSchema), `TAXONOMY` (name/namespace/risk), `INTERNAL_AGENT_TOOLS` (profiles), and the guide's tool-notes index. Keep the three taxonomy tests running against the generated arrays during migration; delete the hand-written arrays when the last file is migrated. Dispatch stays a match on name (`lib.rs:333`) — add a test that every spec name has an arm and every arm has a spec.

### 2.7 Versioning: evidence has to stay comparable

A trial ticket cites a tool. If that tool's behaviour changes and its name does not, every
earlier ticket now cites something that no longer exists, and the selection-bias arithmetic
that reads them (`deflated_sharpe`, `family_wise_test`) is silently comparing incomparable
runs. The ledger is append-only precisely so this cannot happen elsewhere; the catalogue
needs the same discipline.

- **`version` is stamped into the trial ticket** alongside the existing `tool_schema_hash`
  (ADR-P0-18). Bump it when behaviour or the meaning of an output field changes — not for a
  wording fix, and the description-budget test does not force a bump.
- **A statistic that spans tool versions is reported as spanning them.** `family_wise_test`
  and `probability_of_overfit` refuse a family whose members were produced by different
  major versions of the same tool, and say which. That refusal is the point: the honest
  answer is "these are not comparable", not a number.
- **Deprecation is a tombstone, never a deletion.** `Deprecated` keeps the tool in the
  catalogue and out of every profile's exposure, so old evidence still resolves and the desk
  can render what produced it. `superseded_by` gives the agent and the reader the path
  forward. A tool is removed from the source tree only when no ledger row cites it, which is
  a query, not a judgement.

### 2.4 The output contract (P3)

```json
{ "summary": "…≤ 400 tokens, a paragraph a trader would read…",
  "ids":     { "experiment_id": "…", "study_id": "…" },
  "n":       1234, "se": 0.031,
  "detail":  { …optional structured fields, bounded by the schema… },
  "handle":  "work/describe_returns-step7.json" }
```

- `summary` is **generated, not hand-written.** Each spec declares a summary template over
  its own `detail` fields (a small format string with named slots and a unit per slot), and
  the platform renders it. Sixty hand-written prose blobs would drift from the numbers they
  describe within a month, and a summary that disagrees with its own `detail` is worse than
  no summary — it is the one part of the result a flat-tier model reads first and a hurried
  human reads only. A template that references a field the schema does not define fails the
  build. Free prose is allowed only in a `note` slot for a genuine caveat.
- `summary` is mandatory and is what a flat-tier model sees first.
- Every numeric estimate that could be cited carries `n` and, where it has one, `se` (AGENT-001 §10.2's "every number with its n, SE and handle").
- `detail` is the typed body and is what `detail: "standard" | "full"` expands (AGENT-002 P5); default `"summary"`.
- `handle` appears whenever the full result exceeded the profile's cap; the harness writes the file (`drive.rs:1951`, and AGENT-005 §6.3 for the hosted tier).

[DO] Add `pub struct ToolResult` with these fields in `crates/mcp-server/src/spec.rs` and a `render(profile)` that applies the budget. Migrate typed tools to return it; templated tools return it as JSON from the template; workflows compose it.

### 2.4a The error contract

A result is either a `ToolResult` or a **typed error**, and the error shape is part of the
contract because half of what a model learns, it learns from being told no. The UI already
renders `error.code — error.fix` (`components/workspace/Panes.tsx`), and the platform already
returns that shape from several routes; it was never written down, so new tools invent their
own.

```json
{ "error": { "code": "window_exceeds_coverage",
             "message": "BTC-USD 1h has bars from 2024-03-01; you asked from 2023-01-01.",
             "fix": "start at 2024-03-01, or set auto_collect: true",
             "retryable": false,
             "retry_after_s": null,
             "ids": { "instrument_id": "BTC-USD" } } }
```

- `code` is from a closed per-namespace enum, so a harness can route on it and an eval can
  assert on it. Free-text codes are rejected by the same build-time test that checks schemas.
- `fix` is mandatory and is an **edit the caller can make**, not a restatement of the problem.
  "Invalid argument" is not a fix; "omit `venue` — it is inferred from the instrument" is.
- `retryable` distinguishes a transient failure from a wrong call. The loop's retry budget
  applies only to `retryable: true`; a wrong call is corrected, not retried, and three
  identical non-retryable errors change the approach (AGENT-008 P5).
- Errors obey the output budget: ≤ 400 bytes, no stack traces, no SQL.

### 2.4b Spending, previewing, and not double-writing

Three rules that keep an agent from burning a budget or corrupting state by accident.

**Per-tool call budgets.** `spends_trial` bounds simulator runs, but nothing bounded anything
else: a loop could call `run_sweep` fifty times and exhaust a dollar budget without ever
tripping the trial counter. Each `ToolSpec` carries
`max_calls_per_run: Option<u32>` (default `None` for cheap reads; small for anything
expensive), enforced by the harness. Exceeding it is a typed error naming the budget and the
count, not a silent refusal.

**Cost preview.** `estimate_cost(tool, arguments)` returns
`{trials, est_seconds, est_usd, est_rows, confidence}` for any tool that spends a trial or
dispatches a job, computed from the same cost model the job service uses for admission
(`crates/jobs/src/models.rs` `CostInputs`/`CostEstimate`). It spends nothing itself. A model
that can ask "what will this cost" before committing is a model that can plan inside a
budget rather than discovering it at the wall — and the number is what the human sees in the
approval card when the call needs one (§2.6).

**Idempotency on writes.** Every `Risk::Write` and `Risk::Destructive` call carries an
idempotency key derived from `(run_id, step, tool, canonical(arguments))` — the same
canonical-JSON hashing the job service already uses for submission dedup. A retry inside a
run, or a resume after a crash (AGENT-010 §5), presents the same key and the platform returns
the original result instead of performing the action twice. This is what makes the resume
rule safe in the general case rather than only for reads.

### 2.5 Evidence and trials (P4, P5)

[DO] `finish_task` on the hosted driver (`crates/api/src/agent/driver.rs:222` `parse_finish`) gains the same unattributable-id check the local loop has (`drive.rs:1830-1939`): ids named in `evidence` must have appeared in a tool result of this run. Implement by having the driver keep a `seen_ids: BTreeSet<String>` collected from every result's `ids` map (and, until all tools return `ToolResult`, from any string matching the platform's id shapes).

[DO] `spends_trial` is enforced in `dispatch_tool`: a tool declaring `Search` or `Falsification` is wrapped so the ledger row is written first (`crates/ledger`, the existing `TrialTicket` path used by `run_study`) and the result carries `ticket_id`. `Falsification` trials are tagged so `audit.selection_bias_verdict` counts them in N_eff. A test asserts that no tool with `Risk::Write` in `research`, `falsification` or `backtest` has `spends_trial: None` unless it is on an allow-list with a comment (e.g. `advance_gate` records a verdict, it does not run anything).

### 2.6 Risk and approval (fix defect 1)

[DO] `Envelope::for_action` (`audit.rs:85`) keeps matching on names, but the lists become **derived from `ToolSpec.envelope`** and a test asserts the reverse: every name in every list is a catalogue tool, and every tool whose spec declares an envelope is in the corresponding list. This fails immediately on `promote_model` vs `promote_model_version`; fix the list to the tool name. Also add the tools this spec introduces that need gating: `spend_holdout_look` → `SealedHoldout`; `allocate_book` with `apply: true` → `Promotion`; `run_python` → none (it is sandboxed, not privileged), but `Risk::Write`.

## 3. Registration procedure (the one path)

[DO] A new tool is one commit touching:
1. `crates/mcp-server/src/tools/<namespace>.rs` — its `ToolSpec` and, for typed tools, its handler; for templated tools the handler is the generic `templated::run(spec, args)`; for workflows, the step list.
2. Its backing route in `crates/api/src/routes/` (typed) **or** its template in `apps/skill-runner/templates/<namespace>/<name>.py` (templated).
3. The dispatch arm in `lib.rs:333` (until dispatch is also generated, which is a non-goal here).
4. Its `## Tool notes` entry in `crates/mcp-server/docs/authoring_guide.md` if `notes_section` is set.

Everything else — taxonomy, profile lists, envelope lists, description budget, namespace cap, template/schema agreement, dispatch coverage — is a test, and the commit does not merge until they pass. `AGENT-002 §10` acceptance is amended to point here.

### 3.1 [DO] Description rewrite for the ten over-budget tools

`create_skill`, `create_experiment`, `run_sweep`, `report_kill`, `run_study`, `check_tradeable_frontier`, `train_model`, `report_evaluator_defect`, `web_search`, `fetch_url`, and `run_skill`: keep the first sentence (when to use), add one sentence (returns + cost), move the rest verbatim into `## Tool notes` under a heading equal to the tool name. The content is good; it is in the wrong layer. Measured sizes are in AGENT-005 §5.4; the test there enforces the ceiling.

### 3.2 [DO] `get_authoring_guide(section)`

Per AGENT-005 §3.3: `section ∈ {protocol, definition_format, expressions, features, results, tool_notes}` with a `tool` filter for `tool_notes` (`get_authoring_guide(section: "tool_notes", tool: "run_sweep")`). Default `protocol`. Each section ≤ 4 KB or it is split further.

### 3.3 [DO] Templated tool mechanics

- Template header: a fenced JSON block in the module docstring with `{name, namespace, risk, description, input_schema, output, evidence}`; `tools/templated.rs` reads the templates directory at build time (`build.rs`) into `specs()`.
- Data: the harness materialises the series the template asks for (`instrument`, `timeframe`, `start`, `end`, and for multi-asset tools a list) into `<runner tmp>/data/<hash>.parquet` and passes `payload.data_path`; the template reads it with `pyarrow`/`pandas`. The runner's `env_clear()` and socket block (`skill_runner.py:92-133`) stay; no template may import `requests`, `socket`, `subprocess` (the existing static scan `crates/skills/src/lib.rs:218`).
- Limits: the runner's 30 s / 512 MB / 4 MB (`runner.rs:47-54`) are the defaults; a template may declare `"limits": {"wall_seconds": 120}` up to a per-namespace maximum (analysis 120 s; audit/attribution 300 s because block bootstraps are slow). The harness caps the *result* by profile as usual.
- Determinism: every template takes `seed` and prints the versions of numpy/pandas/statsmodels it used into `detail.provenance`.
- Windows dev boxes have no rlimits (`skill_runner.py:61-65`); the wall clock still holds. Unchanged.

## 4. The existing catalogue: keep, fix, retire

| Tool(s) | Decision |
|---|---|
| all 66 in `taxonomy.rs:42` | keep; migrate to `ToolSpec` (§2.3); descriptions per §3.1 |
| `get_authoring_guide` | add `section`/`tool` params (§3.2) |
| `promote_model_version` | gate as `Promotion` (§2.6) |
| `run_skill` | keep; its 5.2 KB description becomes `## Tool notes › run_skill`; add `evidence: ["skill_run_id"]` |
| builder tools (`new_strategy_draft` … `finalize_strategy`) | stay MCP-only (`lib.rs:479-481` rationale stands) |
| `automation.*` | stay MCP-only; `arm_automation` stays `Spend` |
| `backtest_strategy` | keep; declare `spends_trial: Search`; its notes say "prefer `run_study`" |
| `wait_for_backtest`, `run_sweep` | keep the driver-side inline handling (`driver.rs:667`); declare `output.summary` so the wait result is a paragraph, not a snapshot |
| `x_skill` | not a tool (test fixture at `research_tools.rs:368`); nothing to do |
| `find_similar_trials` | keep; becomes the typed core of `memory.check_hypothesis` (§5.8) |
| `get_trade_list` | keep; it is the concentration check `falsification.ablation_sweep` builds on |

## 5. Additions, by namespace

Priority is the research brief's (P0 first), effort S/M/L, tier T = typed, P = templated, W = workflow. "Composes" names existing tools. Signatures are the model-facing shape; every result is a `ToolResult` (§2.4).

### 5.1 `data` (+4)

| Tool | Tier | Risk | Signature | Composes | Effort | Pri |
|---|---|---|---|---|---|---|
| `probe_pit_provenance` | T | read | `(source_id, start, end) -> {is_bitemporal, restatement_count, worst_case_lag_days, sample_violations[]}` | `probe_data_quality` | M | P1 |
| `audit_survivorship` | T | read | `(universe_id, start, end) -> {assets_included, delisted_missing[], estimated_bias_bps?}` | `list_instruments` | S–M | P2 |
| `get_funding_history` | T | read | `(instrument, venue, start, end) -> {rows_handle, data_gap_pct, annualized_mean_bps}` | `get_bars` | S | P0 (needed by `execution.apply_funding_costs`) |
| `certify_source` | W | write | `(source_id) -> {certificate_id, known_gaps[], recheck_by}` = `probe_pit_provenance` + `audit_survivorship` over full history | — | S | P2 |

### 5.2 `analysis` (+13, all templated; this is AGENT-002 §6.3 built as templates)

| Tool | Signature (all take `instrument, timeframe, start, end, seed`) | Effort | Pri |
|---|---|---|---|
| `describe_returns` | `-> {moments{mean,sd,skew,kurt}±se, jb_p, ljung_box{r,r2}, arch_lm_p, hill_alpha, max_dd, jump_share, n}` | S | P0 |
| `estimate_volatility` | `(estimators=[pk,gk,rs,yz,rv,bv,rq], forecast=[ewma,garch,harq]) -> {per_estimator{value,se}, forecast{h1,h5,h20}, percentile_1y}` | M | P0 |
| `detect_regime` | `(method=vol_tercile|hmm|hurst_adx) -> {current, occupancy{}, hurst, variance_ratio, adx, transition_matrix?}` | M | P0 |
| `event_study` | `(entries: [ts], horizon_bars, n_bootstrap) -> {forward_path{mean,p5,p95}, matched_random{…}, excess{value,se}, n}` | M | P0 |
| `estimate_liquidity_cost` | `-> {edge_spread_bps, roll_bps, cs_bps, amihud, sqrt_impact_coef}` — wraps what `check_tradeable_frontier` already computes for one horizon | S | P1 |
| `rolling_correlation` | `(instruments[], window) -> {latest{}, beta{}, nls_cov_handle, mp_band{lower,upper}}` | M | P1 |
| `seasonality` | `(by=hour|weekday, correction=bh|romano_wolf) -> {cells[{key,mean,se,p_adj}], survivors[]}` | S | P1 |
| `stationarity` | `-> {adf_p, kpss_p, verdict}` | S | P1 |
| `cointegration` | `(instruments[2..], method=eg|johansen) -> {stat, p, hedge_ratio, ou_half_life_bars}` | S | P1 |
| `compare_distributions` | `(a_window, b_window) -> {ks_p, wasserstein, quantile_deltas[]}` | S | P2 |
| `find_analogs` | `(vector: [..], k) -> {neighbours[{ts, distance, forward_return}]}` | M | P2 |
| `fit_process` | `(family=gbm|garch|jump) -> {params, aic, process_artifact_id}` | M | P2 |
| `simulate_paths` | `(process_artifact_id, n_paths, horizon) -> {paths_handle, summary{ret,vol,max_dd}}` | S | P2 |

### 5.3 `attribution` (+7)

| Tool | Tier | Signature | Composes | Effort | Pri |
|---|---|---|---|---|---|
| `factor_spanning_test` | P | `(signal_ref, factor_set=ff5_mom|crypto_proxy_v1|custom, custom[]?, nw_lags=5) -> {loadings[], residual_alpha{value,t,annualized}, r2, verdict: spanned|genuine}` | — | S–M (crypto proxy set from free funding/OI/BTC-beta is the work) | P0 |
| `signal_half_life` | P | `(signal_ref, horizons=[1,5,15,60,240,1440]) -> {ic_curve[], half_life_bars, recommended_rebalance_bars, decay_model}` | `cost_shock_curve` for the breakeven cross-check | S | P0 |
| `alpha_beta_timing` | P | `(run_id, benchmark, method=tm|hm) -> {static_beta, timing, residual_alpha, timing_t}` | — | S–M | P1 |
| `alpha_health_check` | T | `(signal_ref, self_corr_threshold=0.7) -> {self_correlation{max, vs}, sub_universe[], fitness, sign_flip, overall, failure_reasons[]}` — the pool is the trial ledger | `find_similar_trials` | M | P1 |
| `cs_ts_decompose` | P | `(run_id, universe[]) -> {variance_share{auto_cov, cross_serial, mean_var}, dominant_driver}` | — | M | P2 |
| `crowding_decay_overlay` | T | `(signal_ref, asset_class, proxy) -> {benchmark_decay{oos:26,post_pub:58}, observed_vs_benchmark, proxy_series_handle, alert}` — labelled "reference class: US equity anomalies" for crypto | `get_funding_history` | M | P2 |
| `style_detector` | P | `(signal_ref) -> {loadings{momentum,carry,short_vol,liquidity}, dominant_style, residual{value,t}, verdict}` | `factor_spanning_test` | S | P2 |

### 5.4 `audit` (+11)

| Tool | Tier | Risk | Signature | Composes | Effort | Pri |
|---|---|---|---|---|---|---|
| `deflated_sharpe` | T | read | `(run_id, trial_family_id, benchmark=0) -> {dsr, psr_p, n_eff_used, skew, kurt, verdict}` — family is fixed at hypothesis registration, never at call time | ledger `NEff` | S | P0 |
| `min_track_record` | T | read | `(observed_sharpe, n_eff, skew, kurt, confidence=0.95) -> {min_btl_bars, min_trl_days, have_days, gap_days}` | funnel (hard gate before holdout) | S | P0 |
| `check_holdout_budget` | T | read | `(experiment_id, partition_id) -> {looks_used, looks_allowed, exhausted, prior_looks[]}` | — | S | P0 |
| `spend_holdout_look` | T | **write, `SealedHoldout`** | `(experiment_id, partition_id, reason) -> {recorded, new_count, blocked, metrics?}` — the **only** tool that may read holdout rows; every other tool's query scope excludes them | funnel holdout→paper | S–M | P0 |
| `probability_of_overfit` | T | read | `(trial_family_id, n_splits=16, metric=sharpe) -> {pbo, is_winner_oos_percentile, logits_handle}` — refuses below 10 distinct configs | ledger return series | M | P1 |
| `family_wise_test` | P | read | `(trial_family_id, method=romano_wolf|spa|reality_check, n_bootstrap=5000) -> {adjusted_p{}, rejected_at_5pct[]}` — block length from `describe_returns` diagnostics | null library block bootstrap | M | P1 |
| `future_perturbation_test` | T | read | `(strategy_id, timeframe, shift_bars=1) -> {has_bias, biased_indicators[], biased_signal_count, max_divergence}` — rerun-and-diff, Freqtrade-style; needs a "shift/mask column X" mode in the strategy runtime | `run_study` | M | P0 |
| `bar_boundary_check` | T | read | `(strategy_id, resample_pairs[]) -> {leak_detected, leaking_pairs[]}` — against the 1m aggregation consumers | — | S | P0 |
| `feature_leakage_scan` | T | read | `(strategy_id) -> {flagged[{feature, issue: full_window_stat|future_correlation|label_transform, severity}]}` — static, in `crates/strategy-validator` | `validate_strategy` | M | P1 |
| `forking_paths_audit` | W | write (trials) | `(strategy_id, specification_space, n_samples=200) -> {m_value, percentile_of_reported, verdict}` — space hash must predate the family's first trial or the tool refuses | `run_study` | L | P2 |
| `metric_integrity` | T | read | `(ticket_id) -> {flags[selective_window|metric_recomputed_inconsistently|null_comparison_skipped], recomputed, matches_reported}` — recomputes from platform-logged calls (`agent_messages`), not from the agent's text | — | M | P0 |

### 5.5 `falsification` (+6, joining the seven that exist)

| Tool | Tier | Signature | Trial | Effort | Pri |
|---|---|---|---|---|---|
| `ablation_sweep` | T | `(strategy_id, run_config, components[]?) -> {baseline, components[{ref, deltas, pct_trades_affected, verdict: load_bearing|redundant|harmful}]}` — deltas must clear the null-library CI, not raw point deltas | Falsification | S | P1 |
| `parameter_fragility_map` | T | `(strategy_id, params[], perturbation_pct=0.2, n_samples=200) -> {surface_handle, fragility_index, chosen_point_percentile}` | Falsification | M | P1 |
| `trade_reshuffle_bootstrap` | P | `(run_id, mode=reshuffle|bootstrap|block, n_paths=5000, capital_levels[]) -> {max_dd_pctiles, tuw_pctiles, ruin_prob_by_capital, endpoint_dispersion?}` — refuses to label reshuffle-only output "worst case" | none | S | P0 |
| `timing_jitter_sensitivity` | T | `(strategy_id, jitter_bars=[0,1,2,3,5], fill_delay_ms=[0,250,1000]) -> {decay_curve[], lookahead_suspicion}` | Falsification | S–M | P1 |
| `cost_shock_curve` | P | `(run_id, multipliers=[1,1.5,2,3,5], live_cost_bps?) -> {curve[], breakeven_multiplier, safety_margin?}` — default grid starts from measured venue spreads | none | S | P0 |
| `regime_failure_search` | T | `(run_id, method, n_regimes=3, min_obs=250) -> {regimes[], worst, unrepresented_risk}` — BH/Romano-Wolf across regimes | none | M | P1 |

### 5.6 `execution` (+6)

| Tool | Tier | Signature | Effort | Pri |
|---|---|---|---|---|
| `get_venue_constraints` | T | `(venue, tier?) -> {rate_limit{model, params}, stp_modes[], margin_model, order_types{}, verified_at}` — a versioned table; flags entries not re-verified in 90 days | S | P0 |
| `apply_funding_costs` | T | `(instrument, venue, positions_handle, source=historical|synthetic_from_basis) -> {cashflows_handle, annualized_carry_bps, data_gap_pct}` — gap > 10% downgrades confidence, never interpolates silently | S–M | P0 |
| `attribute_divergence` | T | `(strategy_id, window) -> {sharpe_backtest, sharpe_paper, decay_pct, attribution{signal,execution,timing,data}, verdict}` — extends the existing paper-vs-backtest tracking | M | P0 |
| `estimate_capacity_curve` | T | `(strategy_id, aum_grid[], cost_model) -> {curve[], optimal_aum, collapse_aum}` — composes the in-flight `crates/ledger/src/capacity.rs` and `crates/backtest/src/costs.rs` | M | P1 |
| `simulate_fill` | T | `(instrument, side, type, limit?, size, submit_ts, queue_model, latency_ms{}) -> {fill_events[], unfilled, queue_position, confidence}` — needs trade-tape retention beyond 1m bars | L | P1 |
| `run_shadow_session` | T, `Spend` | `(strategy_id, venue, max_notional, hours) -> {session_id, fills, slippage_p50_p95, latency_p50_p95, fill_model_bias}` — reports execution quality only, never P&L | M–L | P1 |

### 5.7 `portfolio` (+3) and `model` (+3)

| Tool | Tier | Signature | Effort | Pri |
|---|---|---|---|---|
| `portfolio.shrunk_covariance` | P | `(strategy_ids[], lookback_days, method=linear_lw|nonlinear_lw) -> {cov_handle, raw_vs_shrunk_disagreement, condition_number, shrinkage}` — raw always reported beside shrunk | S | P1 |
| `portfolio.allocate_book` | T, `Promotion` when `apply` | `(cov_handle, capacity_caps{}, method, constraints, apply=false) -> {weights{}, risk_contribution{}, binding[]}` — max single-strategy weight enforced outside the objective | M | P1 |
| `portfolio.check_control_limits` | T, **read-only** | `(strategy_id, window=30d|60d|90d) -> {status, breaching[], recommended_action}` — the *monitor* is a separate scheduled service (AGENT-005 §8 is not it; see ADR-0034 §2.4 principle: the kill switch is not a tool the agent can skip). This tool only reads its state | M | P0 |
| `model.time_aware_importance` | P | `(model_id, features[], window, block_size, method=winit|block_perm) -> {importance[], by_regime{}}` | S–M | P2 |
| `model.conformal_calibrate` | P | `(model_id, calibration_days, coverage, method=aci|enbpi) -> {interval, nonconformity, realized_coverage, drift_warning}` — never vanilla split-conformal on market data | M | P1 |
| `model.detect_drift` | P | `(model_id, features[], reference_window, live_window, method=psi|ks|mmd) -> {by_feature[], alert, est_perf_impact?}` — gates live inference | S–M | P0 |

### 5.8 `memory` (+4) and `report` (+4, harness-owned)

| Tool | Tier | Signature | Effort | Pri |
|---|---|---|---|---|
| `memory.record_negative_result` | T | `(ticket_id, failure_class: weak_effect|null_indistinguishable|capacity_insufficient|execution_infeasible|data_quality, notes) -> {entry_id}` — class derived from the funnel stage that failed, not free text | S | P0 |
| `memory.query_negative_results` | T | `(hypothesis_text, k=10) -> {entries[{ticket_id, class, why}]}` | S | P0 |
| `memory.check_hypothesis` | T | `(hypothesis_text, strategy_json) -> {duplicates[{ticket_id, similarity, verdict}], recommendation: block|warn|proceed}` — structural normaliser over the strategy DSL + embedding; never hard-blocks, override is logged | M | P1 |
| `memory.lookup_signal_registry` | T | `(signal_spec, threshold=0.85) -> {matches[], novelty_score}` — backfilled from the open-source factor libraries named in the brief | M | P1 |
| `report.extract_claims` | H | `(draft) -> {claims[{claim, evidence_refs[], confidence, method: direct_stat|derived|unverified}], unsupported[]}` — `direct_stat` only when a number in the claim equals a number in the cited result | M | P0 |
| `report.recheck_claim` | H | `(claim, via: rerun_stat|requery_ledger|recompute_null) -> {verified, discrepancy?, corrected?}` — a failed recheck blocks the report | S | P0 |
| `report.adversarial_critique` | H | `(artifact_ref) -> {flaws[{severity, description, gap}], verdict}` — runs in a **fresh** session; reviewer precision tracked over time | M | P1 |
| `report.compile_dossier` | H | `(ticket_id, claims, critique, integrity_flags) -> DossierDocument` — "Unresolved questions" and "Reviewer critique" sections are mandatory and say "none found" explicitly | S | P1 |

### 5.9 `code` (+1) — AGENT-005 §8

`run_python(script, payload?, data?: {instrument, timeframe, start, end}) -> ToolResult` on `code_execution: full`; hidden otherwise. Same runner, same limits, same run record as `run_skill`.

### 5.10 Explicitly not added (from the brief's reject list)

GAN/diffusion synthetic-market generators as evidence (only a `generator_trust_tier` flag on `simulate_paths`); matrix-profile motif discovery as a signal source; wavelet/EMD decomposition; VPIN without its incremental-power benchmark; same-context multi-agent "debate" as verification.

### 5.11 `lifecycle` (+12), `research` (+2) — added 2026-09-14 by AGENT-013 / AGENT-014 (revised 2026-09-15)

The stage tools are specified in AGENT-013 §8 with their signatures and envelopes: `lifecycle.preregister`, `hypothesise`, `design`, `refine`, `compare_candidates`, `reproduce`, `propose`, `open_question` (typed) and `get_stage` (harness-owned). `lifecycle.get_session`, `lifecycle.list_agenda`, `lifecycle.get_digest` are read-only views of a continuous research session (AGENT-014 §10). `research.define_campaign` and `research.get_campaign` expose the built `mlops.campaign` (SPEC §10) to the `optimise` workflow; `define_campaign` takes the run's sealed `Preregistration` as its DEFINE and refuses any other input. Two rules carried from those specs: `lifecycle.propose{promote_strategy|change_limits}` is `Promotion` and `{deploy_paper}` is `Spend`; **no other tool with `Risk::Write` may target a registry table** — a static test asserts it (AGENT-013 §8.6).

## 6. Workflows (composed tools; one call, one trial family, one verdict)

Each is a `Workflow` tier entry in `tools/workflows.rs`; steps run server-side with the decision points below; the result is a `ToolResult` whose `detail` carries every step's summary and whose `ids` carry every ticket. A workflow spends trials exactly as its steps would.

| Workflow | Namespace | Sequence and decision points | Verdict |
|---|---|---|---|
| `leakage_audit` | `audit` | `probe_pit_provenance` (any violation → leaking) → `feature_leakage_scan` (high → leaking) → `bar_boundary_check` (leak → leaking) → null-library label shuffle (inside null → *suspect*, not leaking) → `future_perturbation_test` (bias → leaking) → `llm_contamination_check` if text features exist | `clean | suspect | leaking`, bundle sealed into the ticket |
| `selection_bias_verdict` | `audit` | N_eff from ledger → `min_track_record` (short → insufficient_data) → `deflated_sharpe` (fails α → indistinguishable_from_selection) → if N_eff ≥ 10 `probability_of_overfit` (> 0.5 → likely_overfit) → `family_wise_test` vs siblings | `passes | insufficient_data | indistinguishable_from_selection | likely_overfit | not_significant_vs_siblings` |
| `precapital_battery` | `falsification` | `ablation_sweep` (one rule > 50% → downgrade) → `parameter_fragility_map` (knife-edge → reject) → `cost_shock_curve` with measured spreads (margin < 2× → reject) → `trade_reshuffle_bootstrap` (ruin > 5% at 1× → reduced size) → `timing_jitter_sensitivity` (collapse at 1 bar → reject, route to `leakage_audit`) → `regime_failure_search` (current regime = worst → hold) → `simulate_paths` bootstrap/fitted only → `red_team_falsify` (a fresh-context run of the falsification tools over the bundle) | `reject | reduced_size_paper | full_paper` |
| `signal_identity` | `attribution` | `signal_half_life` (< 1 bar → inconclusive) → `factor_spanning_test` (spanned → stop) → `style_detector` (loading > 0.5 & weak residual → repackaged) → `cs_ts_decompose` if multi-asset → `alpha_health_check` (self-corr > 0.7 → redundant) → `crowding_decay_overlay` | `novel_clean | novel_confirm | repackaged_style | redundant | inconclusive` |
| `go_live_readiness` | `execution` | `estimate_capacity_curve` (intended AUM > optimal → reject) → `get_venue_constraints` (requote rate vs limits) → `apply_funding_costs` (carry > 50% of edge → reject) → `attribute_divergence` (execution_problem → recalibrate first; alpha_problem → reject) → `run_shadow_session` 24–72 h (bias > 5 bps → recalibrate, repeat) | `go | go_reduced | no_go`, with `optimal_aum` as the ceiling |
| `hypothesis_precheck` | `memory` | `check_hypothesis` (block → stop) → `lookup_signal_registry` → `query_negative_results` (a prior `execution_infeasible` may be retried only with a cited capability delta) | `proceed_new | proceed_as_variant | reject_duplicate` |
| `report_finalization` | `report` (harness) | `extract_claims` (unsupported → back) → `recheck_claim` on every `direct_stat` → `metric_integrity` (any flag → human, overriding all else) → `adversarial_critique` fresh session (blocking → back) → `compile_dossier` → HITL classification | the dossier, or a block with the reasons |

The funnel (`advance_gate`, `get_funnel`) is amended so that Gate 2 requires a `leakage_audit = clean` and a `selection_bias_verdict = passes` ticket, Gate 3 a `precapital_battery ≠ reject`, and the paper gate a `go_live_readiness ≠ no_go`. The gate refuses to advance without the ticket ids; the model cannot argue its way past (P8).

## 7. Profile membership and exposure

- `Mcp` profile: everything except `report.*` and harness-owned tools (they only exist inside a loop).
- `InternalAgent` profile: everything except `builder.*` and `automation.*` (existing rule, `lib.rs:479`) and, on profiles with `code_execution: denied|templated`, `run_python`.
- Per-step exposure is AGENT-005 §5.1 (24 / 8 / 6). With 21 namespaces averaging seven tools, a routed step sees one namespace whole on every tier except `local_small`, where `analysis` (14) and `research` (14) exceed the 6-slot budget and rely on goal search ordering (`registry.rs:429` fills in router order, never alphabetical). Acceptable, and measured by the eval suite (AGENT-004) rather than assumed.
- `search_tools` (`registry.rs:527`) is upgraded to search `notes_section` headings too, so "how do I check leakage" finds `leakage_audit` by its notes as well as its name.

## 8. Documentation the model can fetch

[DO] `crates/mcp-server/docs/authoring_guide.md` gains `## Tool notes`, one `###` per tool with a `notes_section`, generated from the specs' `notes` (long prose moved out of descriptions in §3.1, plus the guardrail text from the brief for every §5 tool). `get_authoring_guide(section: "tool_notes", tool: …)` serves one entry; the section index is ≤ 2 KB. A test asserts every `notes_section` resolves to a heading.

## 9. Build order

| Phase | Items | Why in this order |
|---|---|---|
| **1 — the contract** (≈ 2 days) | `ToolSpec` + `ToolResult` (§2.3–2.4); generate taxonomy/profile/envelope lists; fix `promote_model_version` gating; description rewrite (§3.1); `get_authoring_guide(section)`; hosted `seen_ids` check (§2.5) | Everything after this is "add a spec", and the two defects are closed |
| **2 — templated tier** (≈ 3 days) | `templated.rs` + build-time template loading (§3.3); the `analysis` namespace (§5.2); `trade_reshuffle_bootstrap`, `cost_shock_curve`, `deflated_sharpe`, `min_track_record` | Delivers the unbuilt AGENT-002 §6.3 suite at template cost and the four cheapest P0 honesty tools |
| **3 — honesty and leakage** (≈ 3 days) | `audit` typed tools (§5.4 P0/P1), `memory` (§5.8), `leakage_audit`, `selection_bias_verdict`, `hypothesis_precheck`; funnel amendments (§6) | The gates become evidence-backed; the agent stops re-testing dead ideas |
| **4 — adversarial and execution** (≈ 4 days) | `falsification` additions, `execution` P0s, `precapital_battery`, `go_live_readiness`; `Falsification` trial kind | Composes with the in-flight `falsification.rs`, `capacity.rs`, `costs.rs` |
| **5 — attribution, portfolio, model** (≈ 3 days) | §5.3, §5.7, `signal_identity` | Needs the templated tier and the crypto factor proxy set |
| **6 — report layer** (≈ 3 days) | `report.*` in the harness, `report_finalization`, reviewer precision tracking | Needs AGENT-005 §5.2 (fresh-context sessions on the hosted tier) |
| **7 — code** | `run_python` per AGENT-005 §8 | Last, because by then the templated tier has shown which analyses still need free-form code |

## 10. Acceptance

> **Form note (Set S task S-0.4, 2026-09-15).** Converted from a plain numbered
> list to the repository's acceptance form so `docs/procedures/execute-plan.md`
> step 3c can close the chain requirement -> criterion -> evidence. No criterion
> was added, removed, reworded or softened; each kept its position, so item *n* of
> the old list is `AC-n`. An unticked box with `Verified by: [--]` means nothing
> has been proved, which is the honest default.
>
> The plan's S-0.4 gives the target form as `6.1.A [ ] ... - Verified by: [--]`.
> The form actually used across the tracked specs (DATA-003, COMP-001..004,
> FEAT-001, SYS-001) is `- [ ] AC-n: ... - Verified by: [--]`, and that is what
> execute-plan step 3c ticks. This uses the tracked form; the plan's example is
> stale.

- [x] AC-1: One `ToolSpec` per tool; `TAXONOMY`, `INTERNAL_AGENT_TOOLS` and the four envelope lists are generated, and `every_tool_is_classified` / `the_taxonomy_has_no_entries_for_tools_that_do_not_exist` / `no_tool_is_classified_twice` still pass against the generated arrays. - Verified by: `mcp-server-lib::spec::tests::every_catalogue_tool_has_exactly_one_spec`, `spec::tests::a_tool_declared_twice_is_rejected_naming_both_sites` (adversarial), and the three taxonomy tests running against the generated rows (`taxonomy::tests::{every_tool_is_classified,the_taxonomy_has_no_entries_for_tools_that_do_not_exist,no_tool_is_classified_twice}`). The four envelope lists are reconciled rather than generated - see AC-2.
- [ ] AC-2: `Envelope::for_action(name)` equals `spec.envelope` for every catalogue tool; `promote_model_version` is `Promotion`; `spend_holdout_look` is `SealedHoldout`. - Verified by: `invariants::static_invariants::every_tool_envelope_matches_the_ledger` (both directions) plus `every_envelope_entry_is_a_real_action`, `the_envelope_name_check_rejects_a_name_no_tool_answers_to` and `the_old_promotion_spelling_matches_no_envelope` (adversarial). **Partial:** `spend_holdout_look` does not exist yet, so its `SealedHoldout` half is unproved.
- [ ] AC-3: Every description ≤ 1,200 B rich and ≤ 400 B flat (AGENT-005 §5.4 test); every `notes_section` resolves. - Verified by: `mcp-server-lib::catalogue_tests::{no_tool_descriptor_exceeds_the_rich_budget,every_first_sentence_fits_the_flat_budget,every_notes_section_resolves_to_a_heading}`. **Partial:** the flat bound is asserted on the first sentence, not on `registry::render(_, Flat)`.
- [x] AC-4: No namespace > 16 tools (`bridge.rs:355`), and every namespace is in §2.1. - Verified by: `mcp-server-lib::catalogue_tests::no_namespace_holds_more_than_sixteen_tools` and `taxonomy::tests::every_namespace_is_canonical`.
- [ ] AC-5: Every templated tool's header schema equals its catalogue schema (build-time test); every template runs under the runner's limits on the reference dataset with `seed` fixed and produces byte-identical `detail` twice. - Verified by: [--]
- [ ] AC-6: Every tool that runs the simulator has `spends_trial ≠ None`, and a run of `precapital_battery` raises the family's N_eff by exactly the number of simulator runs it made. - Verified by: [--]
- [ ] AC-7: `finish_task` on both drivers refuses an `evidence` id that no tool result of the run produced. - Verified by: [--]
- [ ] AC-8: `advance_gate` to Gate 2 fails without `leakage_audit = clean` and `selection_bias_verdict = passes` tickets; to Gate 3 without `precapital_battery`; to paper without `go_live_readiness`. - Verified by: [--]
- [ ] AC-9: Every §5 tool returns a `ToolResult` whose `summary` ≤ 400 tokens on the reference dataset and whose `handle` is set whenever the raw result exceeds the smallest profile's cap (6,144 B). - Verified by: [--]
- [ ] AC-10: AGENT-002 §6 tables are replaced by a pointer to this spec; `docs/Specs/README.md` lists it. - Verified by: [--]
- [ ] AC-11: The `lifecycle` namespace is in §2.1 at 12 tools, the cap test passes at twenty-two namespaces with `research` at exactly 16, the three session views are read-only on every profile, and the static test finds no `Risk::Write` tool targeting a registry table outside `lifecycle.propose` (AGENT-013 §8.6). - Verified by: [--]
## 11. Non-goals

- Generating the dispatch arms (kept as a match with a coverage test; a macro can come later).
- The `tbot` Python SDK surface (AGENT-002 §3–§5) — it will consume the same specs, but that is its own work.
- Sub-agent tool grants (AGENT-001 §13; SUBAGENTS-DESIGN.md).
- Building the kill-switch *monitor service*; this spec only exposes its read-only state.
- LLM-derived text features and `llm_contamination_check` — specified in the brief, deferred until a text feature exists.

## 12. Open questions (record, do not solve)

- Whether `analysis` should split again once `fit_process`/`simulate_paths` grow variants; the 16-cap decides.
- The structural normaliser for `check_hypothesis` over the strategy DSL (which parameter deltas count as "the same idea") has no published recipe; ship with a conservative similarity and let `memory.query_negative_results` hits calibrate it.
- Whether templated tools should expose `detail: "full"` to flat tiers at all, or only the handle; measure on the 7B eval.
- `run_shadow_session` needs a venue minimum-size table and a risk-gate carve-out; it is specified here but its approval envelope (`Spend`) may need a fifth category ("live but capped") once the risk engine grows one.
