# Observability and infrastructure

## Observability (`observability/`)

- **`metrics.py`** — Prometheus metrics (e.g. decision latency, feed stale blocks, normalizer unknowns, **`tb_questdb_write_fail_total`** for QuestDB writer errors — FB-AUD-015)
- **`logging.py`** — Structured logging setup
- **`forecaster_metrics.py`** — `tb_forecast_build_*`, `tb_forecast_packet_abstain_total`, `tb_policy_shadow_*`, `tb_model_version_info`

**Grafana `tb-overview.json`** panels use `tb_decision_latency_seconds`, `tb_pnl_usd`, `tb_forecast_build_fallback_total`, `tb_policy_shadow_target_exposure_delta` — aligned with metric names above (FB-AUDIT-08).

## Infrastructure (`infra/`)

- **`docker-compose.yml`** — QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki, Promtail; named network `trading-bot`
- **`prometheus/prometheus.yml`** — Scrape config
- **`grafana/provisioning/`** — Datasources and dashboards as code
- **`promtail/promtail.yml`** — Log shipping config

Default ports and usage: see **`README.md`**.

## Graceful shutdown

- Live service registers SIGINT/SIGTERM where supported; see **`docs/operations/graceful_shutdown.md`** for conceptual notes and **`app/runtime/live_service.py`** for implementation
