# Canonical monitoring and alerting (APEX)

This document maps **APEX Monitoring & Alerting Spec v1.0** domains to **Prometheus metrics** implemented in **`observability/canonical_metrics.py`** (queue slice **FB-CAN-010**) and **`observability/drift_calibration_metrics.py`** (**FB-CAN-039**).

**FB-CAN-056** adds a machine-checkable **domain checklist** in **`observability/monitoring_domain_checklist.py`** (eleven domains: system, data, state, trigger, auction, risk, execution, carry, drift, replay/shadow, governance). CI runs **`python3 scripts/ci_monitoring_domain_checklist.py`** from **`scripts/ci_canonical_gates.sh`** and fails if required metric **family** names disappear from the Prometheus registry after loading the canonical observability modules.

**FB-CAN-072** adds **`tb_canonical_lag_seconds`** (label **`domain`**: **`data_ingestion`**, **`decision`**, **`execution_feedback`**) in **`observability/lag_metrics.py`**, recorded from **`run_decision_tick`** alongside **`tb_decision_latency_seconds`**. See [§ FB-CAN-072 lag metrics](#fb-can-072-lag-metrics).

**FB-CAN-028** adds checked-in **Prometheus alert rules** (`infra/prometheus/alerts/canonical_apex.yml`), a **Grafana dashboard** (`infra/grafana/provisioning/dashboards/json/tb-canonical-health.json`, UID **`tb-canonical-health`**), and **`GET /governance/monitoring`** in the control plane for pointers. Docker Compose mounts alert rules into Prometheus (`infra/docker-compose.yml`); **`infra/prometheus/prometheus.yml`** references them via `rule_files`.

## Metric name prefix

All canonical slice metrics use the `tb_canonical_*` prefix (alongside existing `tb_*` metrics in `observability/metrics.py` and `observability/forecaster_metrics.py`).

## Domain coverage

| APEX domain (spec §3–4) | Instrument | Notes |
|-------------------------|------------|--------|
| State / safety | `tb_canonical_regime_confidence`, `tb_canonical_heat_score`, `tb_canonical_novelty`, `tb_canonical_reflexivity_score`, `tb_canonical_heat_component`, `tb_canonical_novelty_reason_total`, `tb_canonical_degradation_observations_total`, `tb_canonical_hard_override_total`, `tb_canonical_degradation_transition_count`, `tb_canonical_session_mode_total`, `tb_canonical_session_mode_transition_count`, `tb_canonical_session_reason`, `tb_canonical_safety_reason` (counter family; exposition may append `_total`) | From `RegimeOutput.apex` and **`RiskState`** (FB-CAN-033). **FB-CAN-042:** reflexivity histogram; per-component heat (`component` label `Hf`…`He`); novelty reason counters. **FB-CAN-073:** weekend / low-liquidity session mode rate + transition gauge + session reason codes. **FB-CAN-074:** exchange-risk / data-integrity **`state_*`** codes on `apex.safety_reason_codes`. |
| Trigger | `tb_canonical_trigger_stage_total`, `tb_canonical_trigger_missed_move_total`, `tb_canonical_trigger_setup_to_confirm_latency_ms` | From `forecast_diagnostics["trigger"]` (**FB-CAN-043:** deterministic stage timestamps + latency on `TriggerOutput`) |
| Auction / decision | `tb_canonical_auction_selected_score`, `tb_canonical_auction_suppressed_total`, `tb_canonical_auction_top_n_saturation`, `tb_canonical_auction_candidates_evaluated` | From `forecast_diagnostics["auction"]` (**FB-CAN-044:** throughput / top-N saturation) |
| Risk | `tb_canonical_risk_size_multiplier`, `tb_canonical_risk_final_notional_usd` | From `RiskState` |
| Execution (FB-CAN-047, FB-CAN-075) | `tb_canonical_execution_style_total`, `tb_canonical_execution_guidance_confidence` | From `last_decision_record.diagnostics.execution_guidance_preview` when present; **FB-CAN-075:** `execution_guidance.execution_confidence_terms` holds per-term qualities + normalized weights |
| Data quality (proxy) | `tb_canonical_data_age_seconds` | From `RiskState.data_age_seconds` |
| System lag (FB-CAN-072) | `tb_canonical_lag_seconds` | **`domain=data_ingestion`**: max age of feed vs bar timestamp vs now (event lag); **`domain=decision`**: `pipeline.step` + `risk_engine.evaluate` wall time; **`domain=execution_feedback`**: EMA fill latency from execution feedback memory (seconds). End-to-end tick duration: **`tb_decision_latency_seconds`** |
| Optional families (FB-CAN-050) | `tb_canonical_options_context_available`, `tb_canonical_stablecoin_flow_available`, `tb_canonical_options_context_fallback_active`, `tb_canonical_stablecoin_flow_fallback_active` | From effective feature row after boundary merge (availability + fallback when families enabled but upstream missing) |
| Drift / calibration | `tb_canonical_forecast_ood_score` | From `ForecastPacket.ood_score` |
| Drift / calibration (FB-CAN-039) | `tb_canonical_theoretical_edge_score`, `tb_canonical_edge_erosion_score`, `tb_canonical_trade_intent_execution_confidence` | Trade-intent ticks only; theoretical edge = `decision_confidence × trigger_confidence`, erosion = `max(0, theoretical − execution_confidence)` |
| Drift / calibration (FB-CAN-039) | `tb_canonical_feature_drift_penalty`, `tb_canonical_trigger_false_positive_memory` | From `canonical_exec_quality_penalty` feature + `RiskState.trigger_false_positive_memory` |
| Drift / calibration (FB-CAN-039) | `tb_canonical_confidence_calibration_gap`, `tb_canonical_conformal_interval_relative_width`, `tb_canonical_conformal_calibration_applied_total` | Route vs packet confidence gap; conformal band width when `conformal_applied` |
| Shadow / replay (FB-CAN-039) | `tb_canonical_shadow_divergence_rate`, `tb_canonical_shadow_comparison_within_thresholds`, `tb_canonical_shadow_rollback_recommended`, `tb_canonical_shadow_comparison_bars_compared` | Refreshed from persisted shadow comparison store on API startup and after **`POST /governance/shadow-comparison/run`** |
| Governance | `tb_canonical_active_config_version_info` | Set once from `AppSettings.canonical.metadata.config_version` |
| Governance / operator (**FB-CAN-065**) | `tb_governance_promotion_attempt`, `tb_governance_gate_outcome`, `tb_governance_gate_failure`, `tb_governance_config_drift_event`, `tb_governance_rollback_event` | From `evaluate_promotion_gates` (kind + target env), `POST /governance/release-evidence/diff` (semantic drift flags), release ledger writes with `current_stage=rolled_back` |
| Post-release probation (**FB-CAN-069**) | `tb_governance_probation_monitoring_active`, `tb_governance_probation_abort_recommended`, `tb_governance_probation_phase_total` | Live ticks only: rolling p95 over edge erosion, feature drift penalty, and false-positive memory vs `apex_canonical.domains.post_release_probation.thresholds`; see `GET /governance/probation-status` |
| Carry sleeve | `tb_canonical_carry_sleeve_active`, `tb_canonical_carry_target_notional_usd`, `tb_canonical_carry_funding_signal`, `tb_canonical_carry_trigger_confidence`, `tb_canonical_carry_decision_quality`, `tb_canonical_carry_reason_total`, `tb_canonical_carry_directional_suppression_total` | From `carry_sleeve` diagnostics / `RiskState.carry_sleeve_last` (**FB-CAN-064**): exposure (target notional), funding extremity, trigger confidence at evaluation, quality proxy `funding×trigger`, per-reason counter, isolation suppression |
| Shadow / replay | `tb_canonical_replay_shadow_divergence_total` | Incremented when divergence jobs emit events (legacy counter) |

## Emission point

Metrics are recorded at the end of **`run_decision_tick`** (`decision_engine/run_step.py`) via **`record_canonical_post_tick`**, so live and replay paths stay aligned when the full pipeline runs.

**FB-CAN-072:** Per-domain lag histograms (**`record_lag_seconds`**) are observed in **`run_decision_tick`** immediately after **`tb_decision_latency_seconds`** (same tick), so replay and live both emit when the full tick runs.

## Grafana

- **Overview:** **`infra/grafana/provisioning/dashboards/json/tb-overview.json`** — includes a few canonical panels.
- **Canonical health (FB-CAN-028):** **`infra/grafana/provisioning/dashboards/json/tb-canonical-health.json`** — data age, degradation, trigger missed-move, auction suppression, risk sizing/notional, carry sleeve, OOD, shadow divergence, active config version.

Example:

```promql
histogram_quantile(0.95, sum(rate(tb_canonical_regime_confidence_bucket[5m])) by (le, symbol))
```

## Alerting

### FB-CAN-028 alert rules

Logical rules live in **`infra/prometheus/alerts/canonical_apex.yml`** (loaded by **`infra/prometheus/prometheus.yml`** when using **`infra/docker-compose.yml`**). Categories include:

| Alert | Domain | Notes |
|-------|--------|--------|
| `CanonicalDataAgeHigh` | Data | p95 data age > 90s for 10m |
| `CanonicalDegradationNoTradeOccupancy` | State | elevated `no_trade` degradation ticks |
| `CanonicalTriggerMissedMoveSpike` | Trigger | missed-move counter burst |
| `CanonicalAuctionSuppressedSpike` | Auction | high suppression *rate* (tune for your symbol mix) |
| `CanonicalRiskSizeMultiplierLow` | Risk | p05 size multiplier persistently low |
| `CanonicalCarrySleeveActive` | Carry | carry active sustained (info) |
| `CanonicalCarryDirectionalSuppressionSpike` | Carry | isolation suppression rate elevated (**FB-CAN-064**) |
| `CanonicalCarryDecisionQualityLow` | Carry | p95 carry decision quality (funding×trigger) persistently low (**FB-CAN-064**) |
| `CanonicalShadowDivergenceEvents` | Shadow/replay | divergence counter increasing |
| `CanonicalEventLagHigh` | System (**FB-CAN-072**) | p95 event/data ingestion lag > 90s |
| `CanonicalDecisionProcessingLagHigh` | System (**FB-CAN-072**) | p95 `pipeline.step` + `risk_engine.evaluate` > 2s |
| `CanonicalDecisionCycleDurationHigh` | System (**FB-CAN-072**) | p95 **`tb_decision_latency_seconds`** > 5s |

### FB-CAN-039 alert rules (drift / calibration / shadow trends)

| Alert | Domain | Notes |
|-------|--------|--------|
| `CanonicalEdgeErosionHigh` | Drift / execution | p95 edge erosion score high over 15m |
| `CanonicalConfidenceCalibrationGap` | Calibration | p95 abs(route − forecaster confidence) high |
| `CanonicalShadowDivergenceRateHigh` | Shadow/replay | last persisted `trade_intent_divergence` rate above threshold |
| `CanonicalShadowRollbackRecommended` | Shadow/replay | critical when governance report sets `rollback_recommended` |

Tune `for:` windows and thresholds per environment; defaults aim for **actionable** signals with **low noise**.

See **`docs/Human Provided Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md`** for full requirement text.

### FB-CAN-069 post-release probation

After a **live** release candidate reaches `active_live` with `approved_at`, the process monitors **active** and **cooldown** phases (hours from `apex_canonical.domains.post_release_probation.windows`). During the **active** phase, if at least 32 **trade_intent** samples exist in the rolling window, p95 of edge erosion, `canonical_exec_quality_penalty` (drift), and `trigger_false_positive_memory` are compared to policy maxima; **`tb_governance_probation_abort_recommended`** goes to 1 when any breach fires (operator executes rollback per playbook — not automated ledger writes).

### FB-CAN-065 governance / operator monitoring

Module **`observability/governance_metrics.py`** registers Prometheus counters for promotion gate evaluations and config-diff semantics. **`GET /governance/monitoring`** lists **`governance_operator_metrics`** and **`governance_metrics_module`**. Grafana **tb-canonical-health** includes governance gate outcome and gate-failure panels. Alert group **`canonical_governance_operator_fb_can_065`** in **`infra/prometheus/alerts/canonical_apex.yml`** covers blocked promotions, config diffs requiring operator review, and rollback ledger events.

### FB-CAN-072 lag metrics

Per **`APEX_Monitoring_and_Alerting_Spec_v1_0.md`** §4.1 (event processing lag, decision cycle duration), **`tb_canonical_lag_seconds`** decomposes:

- **Event lag** (`domain=data_ingestion`): seconds from “now” to the freshest of feed last message time and decision `data_timestamp` (aligns with stale-input checks in **`risk_engine.engine`**).
- **Processing lag** (`domain=decision`): wall-clock seconds for **`pipeline.step`** + **`risk_engine.evaluate`** only (excludes post-tick **`record_canonical_post_tick`** / probation hooks).
- **Execution feedback** (`domain=execution_feedback`): **`latency_ms_ema` / 1000** from the per-symbol execution feedback memory bucket when present.

**End-to-end** tick duration remains **`tb_decision_latency_seconds`** (histogram in **`observability/metrics.py`**).

Grafana **tb-canonical-health** adds panels for per-domain lag p95 and decision-cycle p95. Alert group **`canonical_system_lag_fb_can_072`** in **`infra/prometheus/alerts/canonical_apex.yml`** fires on high event lag (p95 > 90s), high decision processing (p95 > 2s), or high end-to-end cycle (p95 > 5s).

### FB-CAN-064 carry sleeve monitoring

Replay **`decision_output_event`** payloads include **`carry_sleeve`** (already under `forecast_diagnostics`); each evaluation now records **`trigger_confidence`** and **`decision_quality`** (funding×trigger) on **`CarrySleeveDecision`** for operator forensics. Grafana **tb-canonical-health** adds panels for carry quality, trigger confidence, directional suppression rate, and reason-code rates.
