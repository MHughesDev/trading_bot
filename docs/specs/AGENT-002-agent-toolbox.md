# AGENT-002: Agent Toolbox — `tbot` SDK/CLI and the MCP Surface

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0022 (thin client over `/api/*`, kept), ADR-0024 (agent runtime),
ADR-0025 (scopes)
**Derived from:** BS-007 [14_TOOLBOX](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/14_TOOLBOX.MD)
**Supersedes:** the tool list in INTG-001 §tools (transport and auth sections stand)
**Plan set:** L (core), then each owning set adds its commands
**Crates and packages:**
- `crates/api` (utoipa annotations);
- new `crates/api-contract` (capability registry);
- `crates/mcp-server` (generated tool definitions);
- new `sdk/python/tbot` (SDK + CLI);
- CI codegen check.

---

## 1. Purpose

Give the agent, human users and external MCP clients **one capability surface**.
- Every capability is a platform route first.
- The Python SDK and CLI are how the agent uses them from bash and code.
- MCP tools are generated from the same definitions, so the surfaces never drift.
- Outputs are compact answers with uncertainty and handles, never raw data dumps.

## 2. Principles (normative)

| # | Principle | Mechanism |
|---|---|---|
| P1 | **API first** | Every capability is an annotated `/api/*` route with a declared scope. SDK, CLI and MCP are generated or thin |
| P2 | **SDK/CLI first for the agent** | No platform tool schemas in the agent's context (AGENT-001 §8.2); chains run in code |
| P3 | **Answers, not data** | Server-side statistics with n and SE/CI; data goes to files as Parquet artifacts |
| P4 | **Handles** | Large inputs and outputs are `art_…` handles (COMP-005 §8) |
| P5 | **Progressive disclosure** | `detail=summary|standard|full` (default summary); `--json` for machine output |
| P6 | **Epistemic class on every number** | `fact | estimate | exploration | result | model_output`, in response metadata and in CLI output suffixes |
| P7 | **Jobs for slow work** | Anything that can exceed about 10 s is a job (COMP-005). Commands return a `job_id` unless `--wait` is given |
| P8 | **Errors that teach** | `{code, field, rule, message, fix}` ≤ 400 B; exit codes 0 success, 1 failure, 2 timeout, 3 usage, 4 scope denied |
| P9 | **Holdout-aware** | Clipping and ledger logging happen server-side (DATA-005) |
| P10 | **Stable surface** | Nothing loads or unloads mid-session. MCP clients get all tools declared, most deferred |
| P11 | **Human-readable ids** | Responses return symbols, slugs and names alongside ids |

## 3. Contract source of truth

- `crates/api` handlers are annotated with **utoipa**. OpenAPI 3.1 is generated at build
  time (`/api/openapi.json`).
- `crates/api-contract` holds a **capability registry**. Each entry has:
  - `name` (namespaced, e.g. `data.bars`);
  - `route`;
  - `scope`;
  - `kind: sync|job`;
  - `output_class`;
  - `summary_template`;
  - `phase`;
  - `status`.
- **Generated from the registry and OpenAPI:**
  - the Python SDK client (`sdk/python/tbot/_generated/`);
  - the CLI command table;
  - MCP tool definitions (`crates/mcp-server`, which replaces the hand-written
    `tool_definitions_for`).
- **CI check:** regenerate everything and fail on diff (`make contract-check`).

## 4. Python package `tbot`

```
sdk/python/tbot/
  __init__.py          # Client(api_url, token) from env TBOT_API_URL / TBOT_TOKEN
  data.py features.py analysis.py strategy.py bt.py exp.py datasets.py models.py
  text.py memory.py skills.py jobs.py art.py report.py proposals.py portfolio.py profile.py
  strategy_runtime.py  # @research_strategy, Param, target_vol_scale (FEAT-004 Layer 1)
  models_iface.py      # ModelInterface (FEAT-006)
  cli/                 # `tbot` entry point (click); compact renderers per output class
  _generated/          # client from OpenAPI (do not edit)
```

- **Return types:**
  - small results are dataclasses with `.summary()`;
  - data calls return `Extract(handle, manifest, path)` and download to
    `/workspace/data/` lazily (`.to_polars()`);
  - job calls return `Job(id)` with `.wait(timeout)` (event-driven, COMP-005 §6).
- **Versioning:** the SDK version is pinned in the agent image. The server rejects SDKs
  older than `min_sdk` with an upgrade message.

## 5. CLI conventions

- `tbot <group> <verb> [args] [--detail summary|standard|full] [--json] [--wait] [--timeout s]`
- Default output fits the §7 budgets. Tables are compact TSV with at most 10 rows plus
  `… N more (handle art_…)`.
- Every numeric line carries its class suffix, e.g. `vol_30d=0.612±0.041 (estimate, n=720)`.
- Long output goes to a file by the harness guard (AGENT-001 §12). The CLI itself also
  caps output at 8k tokens and writes the overflow to `/tmp/out/`.

## 6. Catalogue

Status: **E** exists (as an MCP tool today) · **X** extend · **N** new. The owner spec
defines the semantics; this table defines the surface. The phase is the plan-set phase
(BS-007 17).

### 6.1 Session, jobs, artifacts, reports

| Capability | CLI | Route | Scope | Status | Phase | Owner |
|---|---|---|---|---|---|---|
| jobs.* | `tbot jobs submit|get|list|wait|watch|cancel|logs` | `/api/jobs…` | research:jobs | N | 1 | COMP-005 |
| art.* | `tbot art get|show` | `/api/artifacts…` | research:artifacts | N | 1 | COMP-005 |
| report.submit | `tbot report submit <file>` | `POST /api/reports` | research:reports | N | 1 | AGENT-001 §15 |
| proposals.create | `tbot proposals create --kind …` | `POST /api/approvals` | research:proposals.create | N | 1 | AGENT-001 §16 |
| ask_user | in-process tool | orchestrator | — | N | 1 | AGENT-001 §8 |

### 6.2 Data and discovery (DATA-005)

| Capability | CLI | Status | Phase |
|---|---|---|---|
| data.catalog / coverage / search | `tbot data catalog|coverage|search` | X (`list_instruments`, `get_instrument`, `list_asset_classes`, `list_lanes`) | 1 |
| data.bars | `tbot data bars` | X (`get_bars` → Parquet) | 1 |
| data.trades / quotes / book | `tbot data trades|quotes|book` | N | 1 |
| data.universe | `tbot data universe` | N | 1 |
| data.qc | `tbot data qc` | N | 1 |
| data.live (Desk only) | `tbot data live` | N | 1 |
| data.synthetic | `tbot data synthetic` | N | 1 |
| data.funding / oi / option_chain / prediction_markets / fundamentals / macro | `tbot data …` | N | 5 |
| assets.init / init_status | `tbot assets init|status` | E (`init_asset`, `get_asset_init_job`) | 1 |

### 6.3 Market analysis (server-side; `crates/api` `analysis` module; `exploration` or `estimate`)

| Capability | Returns | Phase |
|---|---|---|
| analysis.describe_returns | Moments ± SE, JB, Ljung–Box (r, r²), ARCH-LM, Hill, max DD, jump share | 1 |
| analysis.volatility | PK/GK/RS/YZ, RV/BV/RQ, EWMA/GARCH/HARQ forecasts, percentile | 1 |
| analysis.regime | Filtered regime, occupancy, Hurst, VR, ADX | 1 / 3 |
| analysis.event_study | Forward paths vs matched random entries, block-bootstrap bands | 1 |
| analysis.liquidity_cost | EDGE spread (+ Roll/CS), Amihud, √-impact | 1 |
| analysis.correlation | Rolling correlation, beta, NLS covariance, MP band | 1 |
| analysis.seasonality | Hour/weekday with BH / Romano–Wolf | 1 |
| analysis.stationarity / cointegration | ADF+KPSS; EG/Johansen, OU half-life | 1 |
| analysis.compare_distributions | KS, Wasserstein, quantile deltas | 1 |
| analysis.analogs | Explicit-vector kNN | 6 |
| analysis.fit_process / simulate_paths / fit_regime_model | Process, path and label artifacts | 3 |
| profile.get | Instrument profile | 1 |

### 6.4 Features, strategies, backtests, evaluation

| Capability | Status | Phase | Owner |
|---|---|---|---|
| features.list / register | N | 2 | DATA-006 |
| strategy.validate / compile / save / get / list / versions | X (`validate_strategy`, `create_strategy`, `get_strategy`, `list_strategies`, `list_compatible_strategies`) | 4 | FEAT-004 |
| strategy.translate_v1 / explain / diff / reconcile / move | N | 4 | FEAT-004 |
| strategy.preview_signals / forensics | N | 2 | FEAT-004 |
| bt.run (definition or position_series; `experiment_id` required for agent tokens) | X (`create_backtest`) | 2 | FEAT-005 |
| bt.outputs / equity_curve / explain_trade | N | 2 | FEAT-005 |
| exp.hypothesis.register / list / get | N | 1 | BACKTEST_SUITE v2 |
| exp.create / get / list | E | 1 | Set J |
| exp.sweep / sweep_status / cancel / carried_forward | E (`run_sweep`, `get_sweep`, `cancel_sweep`, `get_carried_forward`) | 1 | FEAT-003 |
| exp.study / studies / diagnostics / surface | E / N (`get_surface`) | 1 | Set J |
| exp.null_picker / choose_null / funnel / advance_gate | E | 1 | Set J |
| exp.robustness / compare / benchmark | N | 2 | BACKTEST_SUITE v2 |
| exp.alpha_chain / family_test / capacity / posterior / decompose / factors / collider / decay / vs_random | N | 2–6 | BACKTEST_SUITE v2 |
| exp.dossier.build / show | N | 2 | BACKTEST_SUITE v2 |

### 6.5 Datasets, models, text, memory, skills, portfolio

| Capability | Status | Phase | Owner |
|---|---|---|---|
| models.list / get | E | 1 | FEAT-006 |
| models.templates / train / evaluate / compare / importance / predict_series / calibrate / meta_labeler | N | 3 | FEAT-006 |
| datasets.build / show / validate | N | 3 | FEAT-006 |
| text.request / signal_series / contamination_probe | N | 5 | DATA-005 §10 |
| memory.add / search / get | N | 1 / 6 | AGENT-003 |
| skills.search / show / glossary / propose / patch / vote / deprecate | N | 1 (read) / 4–6 | AGENT-003 |
| portfolio.rollup / paper_activity / trading_status / order | E (`get_dashboard_rollup`, `get_paper_activity`, `get_trading_status`, `get_order`) | 1 | — |
| portfolio.positions / paper_vs_backtest | N | 1 / Track | — |

### 6.6 Retired

| Item | When | Why |
|---|---|---|
| Draft-builder MCP tools (`new_strategy_draft`, `add_*`, `finalize_strategy`) | End of Set L | Agents write files; SLv2 replaces them |
| `get_authoring_guide` (monolithic) | End of Set L | Becomes core skills (AGENT-003) |
| `wait_for_backtest`, the driver's `run_sweep` intercept | End of Set L | `jobs.wait` / `watch` |
| `ToolProfile::InternalAgent` | End of Set L | Scopes replace profiles; the internal agent uses the SDK |
| Automations tools in any agent-reachable scope | Never granted | D-12 |

## 7. Output budgets

| Class | Budget (summary) | Content |
|---|---|---|
| Discovery / metadata | ≤ 150 tokens | Compact TSV |
| Analysis | ≤ 400 tokens | Numbers ± SE, window, `as_of`, one-line interpretation, handle |
| Set J result | ≤ 400 tokens | Sealed distribution, ledger line, trials, effective N |
| Diagnostics section | ≤ 400 tokens | Drill down by section |
| List | Top 10 + total + handle | — |
| Error | ≤ 100 tokens | code, field, rule, fix |
| Hard cap | 8k tokens | Overflow goes to a file |

Summaries MUST include every number needed for the next decision, with n and SE. The
`detail=full` re-fetch rate is tracked (AGENT-004).

## 8. MCP surface (external clients)

- Transport and auth as in INTG-001.
- Tools are generated from the registry with **namespaced** names (`data_bars`,
  `exp_sweep`, …) and scope-filtered per token.
- **≤ 15 always-loaded tools:** discovery, data summary, experiments read, jobs, and
  tool search. All others are declared with deferred loading so clients using tool
  search find them without breaking prompt caches.
- Descriptions are generated from the registry. Each follows the pattern:
  - **when to use**;
  - **returns**;
  - **cost** (sync or job);
  - **class**.

## 9. Requirements mapping

| BS-007 ID | Where |
|---|---|
| TB-01 | §3 (routes and registry first) |
| TB-02 | §3–§6 (generation, CI check) |
| TB-03, TB-04 | §2 P5–P6, §5, §7 |
| TB-05 | §6.4 `bt.run` (experiment required) + COMP-005 §10 |
| TB-06 | §6.5 text (reader scope, DATA-005) |
| TB-07 | §6.2 `data.live` (Desk only) |
| TB-08 | §8 |
| TB-09 | §6.6 |
| TB-10 | §2 P3, §7 |

## 10. Acceptance

1. `tbot analysis volatility ETH-USD 1h --window 2y` returns ≤ 400 tokens with SEs,
   window, `as_of` and class in one call.
2. `make contract-check` fails if a route is added without a registry entry, scope or
   regenerated clients.
3. An agent token calling `tbot bt run` without `--experiment` gets exit 4 or 1 with
   `experiment_required` and a fix.
4. An MCP client sees ≤ 15 loaded tools, and finds `exp_family_test` through tool
   search.
