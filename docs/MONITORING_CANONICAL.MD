# Canonical monitoring and alerting (APEX)

This document maps **APEX Monitoring & Alerting Spec v1.0** domains to **Prometheus metrics** implemented in **`observability/canonical_metrics.py`** (queue slice **FB-CAN-010**).

**FB-CAN-028** adds checked-in **Prometheus alert rules** (`infra/prometheus/alerts/canonical_apex.yml`), a **Grafana dashboard** (`infra/grafana/provisioning/dashboards/json/tb-canonical-health.json`, UID **`tb-canonical-health`**), and **`GET /governance/monitoring`** in the control plane for pointers. Docker Compose mounts alert rules into Prometheus (`infra/docker-compose.yml`); **`infra/prometheus/prometheus.yml`** references them via `rule_files`.

## Metric name prefix

All canonical slice metrics use the `tb_canonical_*` prefix (alongside existing `tb_*` metrics in `observability/metrics.py` and `observability/forecaster_metrics.py`).

## Domain coverage

| APEX domain (spec §3–4) | Instrument | Notes |
|-------------------------|------------|--------|
| State / safety | `tb_canonical_regime_confidence`, `tb_canonical_heat_score`, `tb_canonical_novelty`, `tb_canonical_degradation_observations_total`, `tb_canonical_hard_override_total`, `tb_canonical_degradation_transition_count` | From `RegimeOutput.apex` and **`RiskState`** (FB-CAN-033) |
| Trigger | `tb_canonical_trigger_stage_total`, `tb_canonical_trigger_missed_move_total` | From `forecast_diagnostics["trigger"]` |
| Auction / decision | `tb_canonical_auction_selected_score`, `tb_canonical_auction_suppressed_total` | From `forecast_diagnostics["auction"]` |
| Risk | `tb_canonical_risk_size_multiplier`, `tb_canonical_risk_final_notional_usd` | From `RiskState` |
| Data quality (proxy) | `tb_canonical_data_age_seconds` | From `RiskState.data_age_seconds` |
| Drift / calibration | `tb_canonical_forecast_ood_score` | From `ForecastPacket.ood_score` |
| Governance | `tb_canonical_active_config_version_info` | Set once from `AppSettings.canonical.metadata.config_version` |
| Carry sleeve | `tb_canonical_carry_sleeve_active` | Placeholder gauge (0 until wired) |
| Shadow / replay | `tb_canonical_replay_shadow_divergence_total` | Placeholder counter for future divergence jobs |

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

Tune `for:` windows and thresholds per environment; defaults aim for **actionable** signals with **low noise**.

See **`docs/Human Provided Specs/new_specs/canonical/APEX_Monitoring_and_Alerting_Spec_v1_0.md`** for full requirement text.
