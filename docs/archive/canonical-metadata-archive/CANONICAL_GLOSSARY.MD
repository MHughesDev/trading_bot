# Canonical glossary — legacy vs APEX terminology (FB-CAN-040)

**Purpose:** Reduce naming drift during the **FB-CAN-*** migration. When docs or PRs discuss architecture, prefer **canonical APEX** vocabulary from `docs/Human Provided Specs/new_specs/canonical/`. This page maps **today’s repo language** to **target APEX language** and points to concrete code or contracts.

**Related:** [Canonical spec index](CANONICAL_SPEC_INDEX.MD) · [Gap audit](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md) · [APEX master spec](Human%20Provided%20Specs/new_specs/canonical/APEX_UNIFIED_Full_System_Master_Spec_v2_1_CANONICAL.md)

---

## 1) Rules of thumb

| Situation | Use |
|-----------|-----|
| Describing **intent** for the trading system after migration | APEX domain names (state, trigger, auction, risk sizing, execution guidance, …) |
| Describing **what runs in the repo today** | Current module/class names, and add “(legacy path)” only when it helps |
| **Specs / design decisions** | `docs/Human Provided Specs/new_specs/canonical/*.md` wins over `docs/Specs/*.MD` when they conflict on *target* behavior ([precedence](CANONICAL_SPEC_INDEX.MD)) |

---

## 2) Pipeline — old → new mapping

| Legacy / current (repo) | Canonical APEX term | Notes |
|-------------------------|----------------------|--------|
| **ForecastPacket** (`app/contracts/forecast_packet.py`) | **Structure forecast** (quantile bundle + diagnostics) | Canonical docs often say “forecast / structure” — output feeds **structure adapter** → `CanonicalStructureOutput` (`app/contracts/canonical_structure.py`). Still a **wire DTO**; not the same as “state engine”. |
| **PolicySystem** / policy MLP (`policy_model/`, `DecisionPipeline`) | **Route + opportunity auction** | Legacy stack picks a **route**; canonical adds **trigger** stages and a **candidate auction** with scoring/penalties. |
| **DecisionPipeline** (`decision_engine/pipeline.py`) | **Canonical decision sequence** (after forecast) | Orchestrates normalize → forecast → `run_canonical_decision_sequence_after_forecast` (structure → state → trigger → auction → carry). |
| **RouteDecision** (`app/contracts/decisions.py`) | **Selected route** (pre-auction / post-policy) | Canonical auction may **suppress** or **re-rank**; “route” remains a label, not the whole decision. |
| **RegimeOutput** + `apex` (`app/contracts/regime.py`, `CanonicalStateOutput`) | **State & safety** snapshot | Includes regime mix, **novelty**, **heat**, **reflexivity**, **degradation**. |
| **Trigger** dict in `forecast_diagnostics["trigger"]` | **Trigger / timing engine** (setup → pre-trigger → confirm) | See `decision_engine/spec_policy_proposal.py` / trigger engine — align language with APEX trigger spec. |
| **Auction** dict in `forecast_diagnostics["auction"]` | **Opportunity auction** | Candidate set, scores, diversification/overlap penalties — APEX “Decision / Opportunity Selection” domain. |
| **RiskState** (`app/contracts/risk.py`) | **Risk / portfolio state** | Holds **canonical degradation**, **size multipliers**, block codes, carry snapshot, etc. |
| **OrderIntent** / **TradeAction** (`execution/`) | **Trade intent** + **execution guidance** | Intent is signed/gated; guidance carries style, slippage, stress — see APEX execution spec. |
| **run_decision_tick** (`decision_engine/run_step.py`) | **Single shared decision step** (live + replay) | Same function for **live** and **replay** — do not fork semantics. |
| **Replay** (`backtesting/replay.py`) | **Replay / simulation** | Canonical **replay run contract** + event families (`app/contracts/replay_events.py`). |
| **AppSettings** + `default.yaml` | **Canonical runtime config** (`app/config/canonical_config.py`) | Migration toward versioned **config_version** / domain bundles; legacy flat keys still used. |

---

## 3) Docs and code locations — quick pointers

| Topic | Canonical doc (human specs) | Code / contracts |
|-------|-----------------------------|------------------|
| Master architecture | `APEX_UNIFIED_Full_System_Master_Spec_v2_1_CANONICAL.md` | `docs/CANONICAL_MODULE_MAP.MD` |
| Feature / snapshot inputs | `APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md` | `app/contracts/decision_snapshots.py` |
| State / degradation | `APEX_State_Regime_Logic_Detail_Spec_v1_0.md` | `app/contracts/canonical_state.py` |
| Trigger math | `APEX_Trigger_Math_Pseudocode_Detail_Spec_v1_0.md` | `decision_engine/trigger_engine.py` |
| Auction | `APEX_Auction_Scoring_Constraints_Detail_Spec_v1_0.md` | `decision_engine/auction_engine.py` (and diagnostics) |
| Execution | `APEX_Execution_Logic_Detail_Spec_v1_0.md` | `execution/` |
| Monitoring | `APEX_Monitoring_and_Alerting_Spec_v1_0.md` | `observability/`, `docs/MONITORING_CANONICAL.MD` |

---

## 4) What not to confuse

- **“Forecast”** in APEX often means **structure / leverage-flow view**, not “price predictor only.”
- **“Policy”** in old docs may mean **ML routing policy**; in APEX, **policy** is broader (governance + routing + auction rules) — prefer naming the **artifact** (e.g. “policy MLP route scores”) to avoid ambiguity.
- **Legacy `docs/Specs/*.MD`** describes **as-built** behavior; it is not the **target** contract during migration ([precedence](CANONICAL_SPEC_INDEX.MD)).

---

## 5) PR and doc hygiene

- New **architecture** or **FB-CAN-*** work: link or summarize which **APEX domain** you touch.
- If you must use a **legacy** name, pair it once with the **canonical** name (e.g. “`ForecastPacket` (structure forecast wire format)”).
- PR template in `.github/PULL_REQUEST_TEMPLATE.md` includes a checklist item pointing here.
