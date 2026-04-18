<div align="center">

# 🤖 Trading Bot

**A Python playground for AI-assisted crypto trading:** one codebase for **research**, **paper trading**, and **careful live execution** — with **Kraken** for market data and **Alpaca (paper)** / optional **Coinbase (live)** for orders.

[![Python 3.11+](https://img.shields.io/badge/python-3.11+-3776AB?style=flat&logo=python&logoColor=white)](https://www.python.org/downloads/)
[![Code style: Ruff](https://img.shields.io/badge/code%20style-ruff-261230.svg?style=flat)](https://github.com/astral-sh/ruff)

</div>

---

## For AI agents and autonomous tools

**Before any code changes, commands, or task work:** read this **`README.md`** in full, then read **[`AGENTS.md`](AGENTS.md)** in full. **`AGENTS.md`** is the mandatory repository contract (rules, safety boundaries, CI, queue workflow, handoff). **Re-read `AGENTS.md` at the start of every new agent session or thread** — including after context resets — not only on first clone.

**Queue work:** from repo root run **`bash scripts/queue_top.sh`** (or **`python3 scripts/print_next_queue_item.py`**) — prints the full next **`Open`** queue row (optional **`--json`**). To close an item after a slice, run **`bash scripts/queue_close.sh --next`**. See [`AGENTS.md`](AGENTS.md) and [`docs/QUEUE.MD`](docs/QUEUE.MD).

---

## ⚡ Easy start (do this first)

Right after you clone, these scripts set up a virtualenv, install the app, bring up the local data stack (Docker), and copy **`.env.example` → `.env`** if you do not have one yet.

### 🐧 Linux / macOS

```bash
git clone https://github.com/MHughesDev/trading_bot.git
cd trading_bot
chmod +x setup.sh run.sh    # once
./setup.sh                  # install + infra (runs package-index preflight first)
./run.sh                    # API + supervisor + dashboard
./doctor.sh                 # optional env doctor (full audit readiness checks)
```

### 🪟 Windows

```bat
git clone https://github.com/MHughesDev/trading_bot.git
cd trading_bot
setup.bat
run.bat
doctor.bat                  REM optional env doctor
```

**Tips**

- Want to skip Docker for a run? Set **`NM_SKIP_DOCKER=1`** (venv and pip still run). See [`.env.example`](.env.example).
- Local baseline: use **Python 3.12** for parity with CI (project minimum remains 3.11+).
- Deeper checklist (keys, Docker, preflight): [`docs/READY_TO_RUN.MD`](docs/READY_TO_RUN.MD) · Windows UI notes: [`docs/WINDOWS_OPERATOR_UI.MD`](docs/WINDOWS_OPERATOR_UI.MD).

---

## 🛰️ When the system is *actually watching* an asset for you

Think of it as a little factory line — not every bell and whistle, just the happy path while the live loop is on and a symbol is in scope:

| Step | What happens |
|:---:|:---|
| 📡 | **Kraken** streams trades, ticker, and book updates over the WebSocket. |
| 🧹 | Messages are **normalized** into clean snapshots (ticks, spreads, depth). |
| 📊 | **Bars roll** and the **feature pipeline** turns price action into signals the models understand. |
| 🧠 | The **decision engine** runs the same **`run_decision_tick`** path as replay — so paper and live stay honest with each other. |
| 🛡️ | **Risk + signing** get the last word before anything leaves the building. |
| 📝 | **Orders** go to **Alpaca (paper)** by default, or **Coinbase** when you have configured live mode and keys. |
| 💾 | Along the way, **QuestDB / Redis / Qdrant** (via Docker) back bars, cache, and optional memory — so the app is not reinventing a database in RAM. |

**One-liner vibe:** *market noise in → features → decision → risk → (maybe) trade.*

---

## 🎁 The boring stuff you still want

| | |
|:---:|:---|
| 🔑 | Put secrets in **`.env`** — never commit them. App settings use the **`NM_`** prefix (see [`.env.example`](.env.example)). Risk limits in YAML live under **`apex_canonical.domains.risk_sizing`** in **`app/config/default.yaml`** (not a top-level **`risk:`** key). |
| 🧩 | **Microservices runtime bridge** (`NM_MICROSERVICES_RUNTIME_BRIDGE_ENABLED`): enabling the bridge with **`NM_MICROSERVICES_EXECUTION_GATEWAY_MODE=in_process`** duplicates the live decision path unless you set **`apex_canonical.domains.runtime_cutover.migration_shadow_allowed: true`** in YAML (FB-CAN-059). Use **external** gateway mode or disable the bridge if you are not in an intentional migration-shadow window. |
| 📛 | **Suppression / no-trade reason codes** (FB-CAN-063): stable prefixed strings (**`trg_*`**, **`auc_*`**, **`exe_*`**, **`pip_*`**, **`ovr_*`**, **`state_*`**, plus existing **`risk_*`** blocks) in decision records and diagnostics — see [`app/contracts/reason_codes.py`](app/contracts/reason_codes.py). |
| ⚙️ | **Canonical metadata** (`apex_canonical.metadata` in **`default.yaml`**): must include **config_version**, **config_name**, **created_at**, **created_by**, **notes**, and **enabled_feature_families** (APEX §4, FB-CAN-061). With **`NM_EXECUTION_MODE=live`** or **`NM_CANONICAL_CONFIG_STRICT=1`**, **environment_scope** must not stay **unspecified** — set **research**, **simulation**, **shadow**, or **live**. |
| 📦 | Default execution mode is **paper**; go **live** only when you mean it and keys are set. |
| 🧪 | Dev quickies: `python3 -m ruff check .` and `python3 -m pytest tests/ -q` (after `pip install -e ".[dev]"`). |
| 🏛️ | **Governance metrics** (FB-CAN-065): Prometheus **`tb_governance_*`** counters for promotion gates, config-diff drift flags, and rollback ledger events — see [`observability/governance_metrics.py`](observability/governance_metrics.py) and [`docs/MONITORING_CANONICAL.MD`](docs/MONITORING_CANONICAL.MD) (`GET /governance/monitoring`). |
| ⏱️ | **Post-release probation** (FB-CAN-069): after a live **`active_live`** release, **`tb_governance_probation_*`** gauges reflect elevated monitoring and automatic **abort recommended** when rolling risk-quality proxies breach policy — see [`docs/MONITORING_CANONICAL.MD`](docs/MONITORING_CANONICAL.MD) and **`GET /governance/probation-status`**. |
| 📊 | **Canonical coverage matrix** (FB-CAN-070): machine-readable gap-domain ↔ queue ↔ code ↔ tests map — [`docs/reports/CANONICAL_SPEC_COVERAGE_MATRIX.json`](docs/reports/CANONICAL_SPEC_COVERAGE_MATRIX.json), validated by **`scripts/ci_canonical_coverage_matrix.py`**. |
| ⏳ | **Per-domain lag** (FB-CAN-072): Prometheus **`tb_canonical_lag_seconds`** (event lag, decision processing, execution-feedback EMA latency) plus **`tb_decision_latency_seconds`** — see [`docs/MONITORING_CANONICAL.MD`](docs/MONITORING_CANONICAL.MD). |
| 📅 | **Weekend / low-liquidity session** (FB-CAN-073): **`apex_canonical.domains.state_safety_degradation.session_mode`** throttles sizing and triggers; metrics **`tb_canonical_session_mode`** — see [`docs/MONITORING_CANONICAL.MD`](docs/MONITORING_CANONICAL.MD). |
| ⚠️ | **Exchange risk / data integrity** (FB-CAN-074): boundary **`SafetyRegimeSnapshot`** hints → **`apex_exchange_risk_level_code`** / **`apex_data_integrity_alert`** in the merged feature row; degradation, hard overrides (**`data_integrity_alert`**, **`exchange_risk_critical`**), sizing throttle (**`apex_canonical.domains.risk_sizing.exchange_risk`**), metrics **`tb_canonical_safety_reason`** — see [`docs/MONITORING_CANONICAL.MD`](docs/MONITORING_CANONICAL.MD). |
| ⏲️ | **Execution latency / reliability confidence** (FB-CAN-075): **`apex_canonical.domains.execution.execution_confidence`** weights + latency/reliability scalars; **`ExecutionGuidance.execution_confidence_terms`**; feature row **`canonical_exec_latency_ms_ema`** / **`canonical_exec_execution_trust`**; replay fault **`execution_latency_ms_add`** — see [`execution/execution_logic.py`](execution/execution_logic.py). |
| 📉 | **Edge-budget monitoring** (FB-CAN-076): Prometheus **`tb_canonical_edge_budget_headroom`** / **`tb_canonical_edge_budget_stress`**, **`tb_canonical_auction_edge_penalty_max`**, **`tb_canonical_edge_budget_escalation`**; thresholds in **`apex_canonical.domains.monitoring.edge_budget_escalation`** — see [`docs/MONITORING_CANONICAL.MD`](docs/MONITORING_CANONICAL.MD). |
| 🔗 | **Immutable run binding** (FB-CAN-077): each **`DecisionRecord`** carries **`run_binding`** (config/logic/dataset/seed + tamper-evident hash); live **`apex_canonical.domains.replay.live_dataset_id`**; optional **`strict_run_binding`** — see [`app/contracts/run_binding.py`](app/contracts/run_binding.py). |

**Want the full map?** [`docs/SYSTEM_WALKTHROUGH.MD`](docs/SYSTEM_WALKTHROUGH.MD) · **Canonical target architecture (APEX):** [`docs/CANONICAL_SPEC_INDEX.MD`](docs/CANONICAL_SPEC_INDEX.MD) · **Legacy vs canonical naming:** [`docs/CANONICAL_GLOSSARY.MD`](docs/CANONICAL_GLOSSARY.MD) · **Code ↔ APEX domains:** [`docs/CANONICAL_MODULE_MAP.MD`](docs/CANONICAL_MODULE_MAP.MD) · **Removed paths (tombstones):** [`docs/CANONICAL_TOMBSTONE_INDEX.MD`](docs/CANONICAL_TOMBSTONE_INDEX.MD) · **Typed decision inputs:** [`app/contracts/decision_snapshots.py`](app/contracts/decision_snapshots.py) · **Release gating / experiment registry:** [`docs/GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD`](docs/GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD) · **Repo contract for contributors:** [`AGENTS.md`](AGENTS.md).

---

<div align="center">

*Have fun, measure twice, trade responsibly.*

</div>
