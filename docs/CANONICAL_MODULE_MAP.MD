# Canonical module map (APEX runtime alignment)

**Purpose:** Single **code-to-domain** map for the shared decision path after **FB-CAN-004…011**. Use it with **[`CANONICAL_SPEC_INDEX.MD`](CANONICAL_SPEC_INDEX.MD)** (what to build toward) and **[`docs/Specs/README.MD`](Specs/README.MD)** (as-built file index).

**Documentation last reviewed:** 2026-04-18 (FB-CAN-038: [`shadow_comparison.py`](../orchestration/shadow_comparison.py) paired replay divergence + governance APIs; FB-CAN-037: [`fault_injection_profiles.py`](../orchestration/fault_injection_profiles.py) named profiles + **`ReplayRunContract.fault_injection_profile_id`**; FB-CAN-036: [`decision_record.py`](../app/contracts/decision_record.py) + [`decision_engine/decision_record.py`](../decision_engine/decision_record.py) `build_decision_record`; FB-CAN-035: [`execution_feedback_memory.py`](../data_plane/memory/execution_feedback_memory.py) EMA + **`canonical_exec_quality_penalty`**; FB-CAN-034: auction **`D_corr`/`D_thesis`/`D_liq`** + **`AuctionResult.clustering_metadata`**; FB-CAN-033: `classify_hard_override` + transition/occupancy on **`RiskState`**; FB-CAN-032: [`signal_confidence.py`](../app/config/signal_confidence.py); FB-CAN-031: missed-move + `trigger_false_positive_memory`; FB-CAN-029: [`canonical_orchestrator.py`](../decision_engine/canonical_orchestrator.py); FB-CAN-023: [`Specs/CANONICAL_PRECEDENCE.MD`](Specs/CANONICAL_PRECEDENCE.MD) clarifies as-built Specs vs APEX target).

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
| Config / versioning | [`app/config/canonical_config.py`](../app/config/canonical_config.py), [`app/config/settings.py`](../app/config/settings.py), [`app/config/signal_confidence.py`](../app/config/signal_confidence.py), [`app/config/shadow_comparison.py`](../app/config/shadow_comparison.py) | `AppSettings.canonical` + YAML `apex_canonical` (**FB-CAN-032** `signal_confidence` / `feature_families`; **FB-CAN-038** `shadow_comparison`). |
| Decision step | [`decision_engine/run_step.py`](../decision_engine/run_step.py), [`decision_engine/decision_record.py`](../decision_engine/decision_record.py) | **`run_decision_tick`** — single entry for live and replay; **FB-CAN-036** `DecisionRecord` on `RiskState` + forecast diagnostics. |
| Typed input snapshots | [`app/contracts/decision_snapshots.py`](../app/contracts/decision_snapshots.py), [`app/contracts/snapshot_builders.py`](../app/contracts/snapshot_builders.py) | **FB-CAN-015:** boundary contracts + merge into effective feature row; diagnostics on packet. |
| Feature normalization | [`data_plane/features/canonical_normalize.py`](../data_plane/features/canonical_normalize.py) | **FB-CAN-016:** aliases, freshness/reliability, completeness; `apply_normalization_degradation` in `state_engine.py`. |
| Execution feedback memory | [`data_plane/memory/execution_feedback_memory.py`](../data_plane/memory/execution_feedback_memory.py) | **FB-CAN-035:** per-symbol EMA; boundary snapshot + `canonical_exec_quality_penalty` on feature row; replay updates after simulated fills. |
| Forecast / structure (canonical view) | [`app/contracts/canonical_structure.py`](../app/contracts/canonical_structure.py), [`app/contracts/structure_adapter.py`](../app/contracts/structure_adapter.py) | **FB-CAN-017:** `CanonicalStructureOutput` from `ForecastPacket`; diagnostics on packet; feeds trigger + auction. |
| Pipeline orchestration | [`decision_engine/pipeline.py`](../decision_engine/pipeline.py) | Boundary normalize + forecast packet build. |
| Canonical post-forecast sequence | [`decision_engine/canonical_orchestrator.py`](../decision_engine/canonical_orchestrator.py) | **FB-CAN-029** — structure → state → trigger → auction → carry (shared by `pipeline.step`). |
| State / regime / degradation | [`decision_engine/state_engine.py`](../decision_engine/state_engine.py), [`app/contracts/canonical_state.py`](../app/contracts/canonical_state.py), [`app/contracts/hard_override.py`](../app/contracts/hard_override.py) | **FB-CAN-041:** 5-class APEX regime vector (trend/range/stress/dislocated/transition) with `regime_confidence = max(R)−second_max(R)` and spec §7.3 transition probability; **`RegimeOutput.canonical_regime_probabilities`** + **`RegimeOutput.confidence`** align with canonical separation. **FB-CAN-042:** configurable **`apex_canonical.domains.state_safety_degradation`** weights for heat / novelty / reflexivity; propagation into trigger setup and auction penalties; **`RiskState.risk_novelty_score`**. **FB-CAN-033:** `classify_hard_override` (deterministic taxonomy); **`RiskState`** transition count + per-level occupancy; hard override forces **NO_TRADE** in orchestrator before auction. |
| Three-stage trigger | [`decision_engine/trigger_engine.py`](../decision_engine/trigger_engine.py), [`app/contracts/trigger.py`](../app/contracts/trigger.py) | **FB-CAN-031** missed-move gate; diagnostics: `forecast_diagnostics["trigger"]`. **FB-CAN-043:** stage ISO timestamps, `setup_to_confirm_latency_ms`, `stage_failure_codes` (setup / pretrigger / confirm). |
| Opportunity auction | [`decision_engine/auction_engine.py`](../decision_engine/auction_engine.py), [`app/contracts/auction.py`](../app/contracts/auction.py) | **FB-CAN-031** false-positive memory penalty **P**; **FB-CAN-034** diversification split (**`D_corr`**, **`D_thesis`**, **`D_liq`**) + **`clustering_metadata`**; diagnostics: `forecast_diagnostics["auction"]`. **FB-CAN-044:** **`apex_canonical.domains.auction`** (`top_n`, `max_candidates`, `saturation_warn_score`), **`AuctionResult`** throughput fields + **`clustering_metadata["fb_can_044"]`**, tie-break includes **`R`** penalty; suppression reason **`top_n_throughput_shortfall`** when **`top_n`** cannot be fully utilized in the single-symbol path. **FB-CAN-046:** **`RiskState.portfolio_thesis_buckets`** (USD notionals, keys `thesis_key` or `symbol::thesis_key`) + **`merge_symbol_position_into_thesis_buckets`**; thesis overlap fraction vs cap (hard reject **`thesis_overlap_cap`**); book exposure stress into **`B`**; defense / liquidation–stress dominance boosts **`G`**; **`clustering_metadata["fb_can_046"]`**; **`current_total_exposure_usd`** threaded from **`run_decision_tick`** → **`pipeline.step`** → auction. |
| Risk — trigger memory | [`app/contracts/risk.py`](../app/contracts/risk.py), [`decision_engine/state_engine.py`](../decision_engine/state_engine.py) | **`trigger_false_positive_memory`** updated in `merge_canonical_into_risk`. |
| Policy / proposal | [`decision_engine/spec_policy_proposal.py`](../decision_engine/spec_policy_proposal.py) | PolicySystem + trigger + auction; legacy `action_generator.py` removed (FB-CAN-019). |
| Risk + sizing | [`risk_engine/engine.py`](../risk_engine/engine.py), [`risk_engine/canonical_sizing.py`](../risk_engine/canonical_sizing.py) | Hard gates + `compute_canonical_notional`; **FB-CAN-045** composite degradation (`transition_probability` / `novelty` throttles) in `merge_canonical_into_risk`; `last_risk_sizing` + `canonical_degradation_sizing_terms` diagnostics; `apex_canonical.domains.risk_sizing` sizing knobs. |
| Execution guidance | [`execution/execution_logic.py`](../execution/execution_logic.py), [`execution/service.py`](../execution/service.py), [`app/contracts/execution_guidance.py`](../app/contracts/execution_guidance.py) | **FB-CAN-047:** configurable thresholds in **`apex_canonical.domains.execution`**; `build_execution_guidance` emits `style_rationale_codes`; `build_decision_record` fills `diagnostics.execution_guidance_preview` + trade intent style/urgency; metrics in `canonical_metrics.py`. |
| Carry sleeve | [`carry_sleeve/engine.py`](../carry_sleeve/engine.py), [`app/contracts/carry_sleeve.py`](../app/contracts/carry_sleeve.py) | **FB-CAN-018:** optional `RouteId.CARRY` proposal; isolation vs directional; `domains.carry` in canonical config. |
| Metrics | [`observability/canonical_metrics.py`](../observability/canonical_metrics.py) | `tb_canonical_*`; **FB-CAN-033** `tb_canonical_hard_override_total`, `tb_canonical_degradation_transition_count`; recorded from `run_decision_tick`. |

---

## Replay / simulation

| Concern | Modules |
|---------|---------|
| Run contract + events | [`app/contracts/replay_events.py`](../app/contracts/replay_events.py) — **FB-CAN-037** optional **`fault_injection_profile_id`** (named canonical profile). |
| Replay step + fault injection | [`backtesting/replay_core.py`](../backtesting/replay_core.py), [`backtesting/fault_injection.py`](../backtesting/fault_injection.py), [`orchestration/fault_injection_profiles.py`](../orchestration/fault_injection_profiles.py), [`backtesting/replay_events.py`](../backtesting/replay_events.py) |
| Bar replay API | [`backtesting/replay.py`](../backtesting/replay.py) |

---

## Governance (operator tooling)

| Concern | Modules / docs |
|---------|----------------|
| Release gating | [`orchestration/release_gating.py`](../orchestration/release_gating.py) (**FB-CAN-037** `fault_stress_*` + **`fault_stress_evidence`**; **FB-CAN-038** **`shadow_comparison_passed`** for shadow/live), [`scripts/validate_release_gates.py`](../scripts/validate_release_gates.py), [`scripts/ci_canonical_contracts.sh`](../scripts/ci_canonical_contracts.sh) |
| Release evidence JSON | [`orchestration/release_evidence.py`](../orchestration/release_evidence.py) — **`fault_stress_run_ids`** / **`fault_profile_ids_satisfied`** (**FB-CAN-037**); **`shadow_comparison_report`** (**FB-CAN-038**). |
| Shadow comparison | [`orchestration/shadow_comparison.py`](../orchestration/shadow_comparison.py), [`models/registry/shadow_comparison_store.py`](../models/registry/shadow_comparison_store.py), **`GET/POST /governance/shadow-comparison*`** |
| Experiment registry | [`models/registry/experiment_registry.py`](../models/registry/experiment_registry.py) |
| Runbooks | [`GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD`](GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD), [`MONITORING_CANONICAL.MD`](MONITORING_CANONICAL.MD) |

---

## Removed / superseded paths

See **[`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD)**. The **as-built** tree under `docs/Specs/` is **not** deleted here: it stays the engineer-facing map of **current** modules; canonical specs define **target** behavior.

---

## Migration note

Ongoing hard deletions of legacy-only modules are tracked in the queue (**FB-CAN-019** onward) and the gap audit; this file does **not** replace [`reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`](reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md).
