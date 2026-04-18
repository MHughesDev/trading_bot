# Canonical monitoring and alerting (APEX)

This document maps **APEX Monitoring & Alerting Spec v1.0** domains to **Prometheus metrics** implemented in **`observability/canonical_metrics.py`** (queue slice **FB-CAN-010**) and **`observability/drift_calibration_metrics.py`** (**FB-CAN-039**).

**FB-CAN-056** adds a machine-checkable **domain checklist** in **`observability/monitoring_domain_checklist.py`** (eleven domains: system, data, state, trigger, auction, risk, execution, carry, drift, replay/shadow, governance). CI runs **`python3 scripts/ci_monitoring_domain_checklist.py`** from **`scripts/ci_canonical_gates.sh`** and fails if required metric **family** names disappear from the Prometheus registry after loading the canonical observability modules.

**FB-CAN-028** adds checked-in **Prometheus alert rules** (`infra/prometheus/alerts/canonical_apex.yml`), a **Grafana dashboard** (`infra/grafana/provisioning/dashboards/json/tb-canonical-health.json`, UID **`tb-canonical-health`**), and **`GET /governance/monitoring`** in the control plane for pointers. Docker Compose mounts alert rules into Prometheus (`infra/docker-compose.yml`); **`infra/prometheus/prometheus.yml`** references them via `rule_files`.

## Metric name prefix

All canonical slice metrics use the `tb_canonical_*` prefix (alongside existing `tb_*` metrics in `observability/metrics.py` and `observability/forecaster_metrics.py`).

## Domain coverage

| APEX domain (spec §3–4) | Instrument | Notes |
|-------------------------|------------|--------|
| State / safety | `tb_canonical_regime_confidence`, `tb_canonical_heat_score`, `tb_canonical_novelty`, `tb_canonical_reflexivity_score`, `tb_canonical_heat_component`, `tb_canonical_novelty_reason_total`, `tb_canonical_degradation_observations_total`, `tb_canonical_hard_override_total`, `tb_canonical_degradation_transition_count` | From `RegimeOutput.apex` and **`RiskState`** (FB-CAN-033). **FB-CAN-042:** reflexivity histogram; per-component heat (`component` label `Hf`…`He`); novelty reason counters. |
| Trigger | `tb_canonical_trigger_stage_total`, `tb_canonical_trigger_missed_move_total`, `tb_canonical_trigger_setup_to_confirm_latency_ms` | From `forecast_diagnostics["trigger"]` (**FB-CAN-043:** deterministic stage timestamps + latency on `TriggerOutput`) |
| Auction / decision | `tb_canonical_auction_selected_score`, `tb_canonical_auction_suppressed_total`, `tb_canonical_auction_top_n_saturation`, `tb_canonical_auction_candidates_evaluated` | From `forecast_diagnostics["auction"]` (**FB-CAN-044:** throughput / top-N saturation) |
| Risk | `tb_canonical_risk_size_multiplier`, `tb_canonical_risk_final_notional_usd` | From `RiskState` |
| Execution (FB-CAN-047) | `tb_canonical_execution_style_total`, `tb_canonical_execution_guidance_confidence` | From `last_decision_record.diagnostics.execution_guidance_preview` when present |
| Data quality (proxy) | `tb_canonical_data_age_seconds` | From `RiskState.data_age_seconds` |
| Optional families (FB-CAN-050) | `tb_canonical_options_context_available`, `tb_canonical_stablecoin_flow_available`, `tb_canonical_options_context_fallback_active`, `tb_canonical_stablecoin_flow_fallback_active` | From effective feature row after boundary merge (availability + fallback when families enabled but upstream missing) |
| Drift / calibration | `tb_canonical_forecast_ood_score` | From `ForecastPacket.ood_score` |
| Drift / calibration (FB-CAN-039) | `tb_canonical_theoretical_edge_score`, `tb_canonical_edge_erosion_score`, `tb_canonical_trade_intent_execution_confidence` | Trade-intent ticks only; theoretical edge = `decision_confidence × trigger_confidence`, erosion = `max(0, theoretical − execution_confidence)` |
| Drift / calibration (FB-CAN-039) | `tb_canonical_feature_drift_penalty`, `tb_canonical_trigger_false_positive_memory` | From `canonical_exec_quality_penalty` feature + `RiskState.trigger_false_positive_memory` |
| Drift / calibration (FB-CAN-039) | `tb_canonical_confidence_calibration_gap`, `tb_canonical_conformal_interval_relative_width`, `tb_canonical_conformal_calibration_applied_total` | Route vs packet confidence gap; conformal band width when `conformal_applied` |
| Shadow / replay (FB-CAN-039) | `tb_canonical_shadow_divergence_rate`, `tb_canonical_shadow_comparison_within_thresholds`, `tb_canonical_shadow_rollback_recommended`, `tb_canonical_shadow_comparison_bars_compared` | Refreshed from persisted shadow comparison store on API startup and after **`POST /governance/shadow-comparison/run`** |
| Governance | `tb_canonical_active_config_version_info` | Set once from `AppSettings.canonical.metadata.config_version` |
| Carry sleeve | `tb_canonical_carry_sleeve_active` | Placeholder gauge (0 until wired) |
| Shadow / replay | `tb_canonical_replay_shadow_divergence_total` | Incremented when divergence jobs emit events (legacy counter) |

## Emission point

Metrics are recorded at the end of **`run_decision_tick`** (`decision_engine/run_step.py`) via **`record_canonical_post_tick`**, so live and replay paths stay aligned when the full pipeline runs.

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
| `CanonicalShadowDivergenceEvents` | Shadow/replay | divergence counter increasing |

### FB-CAN-039 alert rules (drift / calibration / shadow trends)

| Alert | Domain | Notes |
|-------|--------|--------|
| `CanonicalEdgeErosionHigh` | Drift / execution | p95 edge erosion score high over 15m |
| `CanonicalConfidenceCalibrationGap` | Calibration | p95 abs(route − forecaster confidence) high |
| `CanonicalShadowDivergenceRateHigh` | Shadow/replay | last persisted `trade_intent_divergence` rate above threshold |
| `CanonicalShadowRollbackRecommended` | Shadow/replay | critical when governance report sets `rollback_recommended` |

Tune `for:` windows and thresholds per environment; defaults aim for **actionable** signals with **low noise**.

See **`docs/Human Provided Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md`** for full requirement text.
