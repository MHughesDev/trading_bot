# Observability and infrastructure

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Observability (`observability/`)

- **`metrics.py`** — Prometheus metrics (e.g. decision latency, feed stale blocks, normalizer unknowns, **`tb_questdb_write_fail_total`** for QuestDB writer errors — FB-AUD-015)
- **`canonical_metrics.py`** — **FB-CAN-010:** APEX canonical families (`tb_canonical_*`) — state/regime, trigger, auction, risk, OOD, data-age proxy, config version; see **[`docs/MONITORING_CANONICAL.MD`](../MONITORING_CANONICAL.MD)**
- **`logging.py`** — Structured logging setup
- **`forecaster_metrics.py`** — `tb_forecast_build_*`, `tb_forecast_packet_abstain_total`, `tb_policy_shadow_*`, `tb_model_version_info`

**Grafana `tb-overview.json`** panels use `tb_decision_latency_seconds`, `tb_pnl_usd`, `tb_forecast_build_fallback_total`, `tb_policy_shadow_target_exposure_delta` — aligned with metric names above (FB-AUDIT-08). Add panels for `tb_canonical_*` per **`docs/MONITORING_CANONICAL.MD`**.

## Infrastructure (`infra/`)

- **`docker-compose.yml`** — QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki, Promtail; named network `trading-bot`
- **`prometheus/prometheus.yml`** — Scrape config
- **`grafana/provisioning/`** — Datasources and dashboards as code
- **`promtail/promtail.yml`** — Log shipping config

Default ports and usage: see **`README.md`**.

## Graceful shutdown

- Live service registers SIGINT/SIGTERM where supported; see **`docs/GRACEFUL_SHUTDOWN.MD`** for conceptual notes and **`app/runtime/live_service.py`** for implementation
