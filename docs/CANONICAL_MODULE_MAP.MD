# Canonical module map (APEX runtime alignment)

**Purpose:** Single **code-to-domain** map for the shared decision path after **FB-CAN-004…011**. Use it with **[`CANONICAL_SPEC_INDEX.MD`](CANONICAL_SPEC_INDEX.MD)** (what to build toward) and **[`docs/Specs/README.MD`](Specs/README.MD)** (as-built file index).

**Documentation last reviewed:** 2026-04-18 (FB-CAN-018: `carry_sleeve/`).

---

## How to read this

| Layer | Role |
|-------|------|
| **APEX canonical specs** | `docs/Human Provided Specs/new_specs/canonical/*.md` — target behavior. |
| **This map** | Where implemented in Python packages **today**. |
| **As-built specs** | `docs/Specs/*.MD` — module responsibilities and entry points (updated when code changes). |

Legacy **ForecastPacket → PolicySystem → proposal** remains the **forecast/routing** spine; **state → trigger → auction** wrap and gate that path before risk. Do not bypass **`run_decision_tick`** for live vs replay parity.

---

## End-to-end tick (`shared live + replay`)

| Step | Primary modules | Notes |
|------|-----------------|--------|
| Config / versioning | [`app/config/canonical_config.py`](../app/config/canonical_config.py), [`app/config/settings.py`](../app/config/settings.py) | `AppSettings.canonical` + optional YAML `apex_canonical`. |
| Decision step | [`decision_engine/run_step.py`](../decision_engine/run_step.py) | **`run_decision_tick`** — single entry for live and replay. |
| Typed input snapshots | [`app/contracts/decision_snapshots.py`](../app/contracts/decision_snapshots.py), [`app/contracts/snapshot_builders.py`](../app/contracts/snapshot_builders.py) | **FB-CAN-015:** boundary contracts + merge into effective feature row; diagnostics on packet. |
| Feature normalization | [`data_plane/features/canonical_normalize.py`](../data_plane/features/canonical_normalize.py) | **FB-CAN-016:** aliases, freshness/reliability, completeness; `apply_normalization_degradation` in `state_engine.py`. |
| Forecast / structure (canonical view) | [`app/contracts/canonical_structure.py`](../app/contracts/canonical_structure.py), [`app/contracts/structure_adapter.py`](../app/contracts/structure_adapter.py) | **FB-CAN-017:** `CanonicalStructureOutput` from `ForecastPacket`; diagnostics on packet; feeds trigger + auction. |
| Pipeline orchestration | [`decision_engine/pipeline.py`](../decision_engine/pipeline.py) | Regime, forecast, policy, canonical layers. |
| State / regime / degradation | [`decision_engine/state_engine.py`](../decision_engine/state_engine.py), [`app/contracts/canonical_state.py`](../app/contracts/canonical_state.py) | `RegimeOutput.apex`, `RiskState` canonical fields. |
| Three-stage trigger | [`decision_engine/trigger_engine.py`](../decision_engine/trigger_engine.py), [`app/contracts/trigger.py`](../app/contracts/trigger.py) | Diagnostics: `forecast_diagnostics["trigger"]`. |
| Opportunity auction | [`decision_engine/auction_engine.py`](../decision_engine/auction_engine.py), [`app/contracts/auction.py`](../app/contracts/auction.py) | Diagnostics: `forecast_diagnostics["auction"]`. |
| Policy / proposal | [`decision_engine/spec_policy_proposal.py`](../decision_engine/spec_policy_proposal.py), [`decision_engine/action_generator.py`](../decision_engine/action_generator.py) | Uses trigger + auction when enabled. |
| Risk + sizing | [`risk_engine/engine.py`](../risk_engine/engine.py), [`risk_engine/canonical_sizing.py`](../risk_engine/canonical_sizing.py) | Hard gates + `compute_canonical_notional`; `last_risk_sizing` diagnostics. |
| Execution guidance | [`execution/execution_logic.py`](../execution/execution_logic.py), [`execution/service.py`](../execution/service.py), [`app/contracts/execution_guidance.py`](../app/contracts/execution_guidance.py) | Prepare/sign path before adapters. |
| Carry sleeve | [`carry_sleeve/engine.py`](../carry_sleeve/engine.py), [`app/contracts/carry_sleeve.py`](../app/contracts/carry_sleeve.py) | **FB-CAN-018:** optional `RouteId.CARRY` proposal; isolation vs directional; `domains.carry` in canonical config. |
| Metrics | [`observability/canonical_metrics.py`](../observability/canonical_metrics.py) | `tb_canonical_*`; recorded from `run_decision_tick`. |

---

## Replay / simulation

| Concern | Modules |
|---------|---------|
| Run contract + events | [`app/contracts/replay_events.py`](../app/contracts/replay_events.py) |
| Replay step + fault injection | [`backtesting/replay_core.py`](../backtesting/replay_core.py), [`backtesting/fault_injection.py`](../backtesting/fault_injection.py), [`backtesting/replay_events.py`](../backtesting/replay_events.py) |
| Bar replay API | [`backtesting/replay.py`](../backtesting/replay.py) |

---

## Governance (operator tooling)

| Concern | Modules / docs |
|---------|----------------|
| Release gating | [`orchestration/release_gating.py`](../orchestration/release_gating.py), [`scripts/validate_release_gates.py`](../scripts/validate_release_gates.py), [`scripts/ci_canonical_contracts.sh`](../scripts/ci_canonical_contracts.sh) |
| Experiment registry | [`models/registry/experiment_registry.py`](../models/registry/experiment_registry.py) |
| Runbooks | [`GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD`](GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD), [`MONITORING_CANONICAL.MD`](MONITORING_CANONICAL.MD) |

---

## Removed / superseded paths

See **[`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD)**. The **as-built** tree under `docs/Specs/` is **not** deleted here: it stays the engineer-facing map of **current** modules; canonical specs define **target** behavior.

---

## Migration note

Ongoing hard deletions of legacy-only modules are tracked in the queue (**FB-CAN-019** onward) and the gap audit; this file does **not** replace [`reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md).
