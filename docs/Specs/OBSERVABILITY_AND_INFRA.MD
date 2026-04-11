# Observability and infrastructure

## Observability (`observability/`)

- **`metrics.py`** — Prometheus metrics (e.g. decision latency, feed stale blocks, normalizer unknowns)
- **`logging.py`** — Structured logging setup
- **`forecaster_metrics.py`** — `nm_forecast_build_*`, `nm_forecast_packet_abstain_total`, `nm_policy_shadow_*`, `nm_model_version_info`

**Grafana `nm-overview.json`** panels use `nm_decision_latency_seconds`, `nm_pnl_usd`, `nm_forecast_build_fallback_total`, `nm_policy_shadow_target_exposure_delta` — aligned with metric names above (FB-AUDIT-08).

## Infrastructure (`infra/`)

- **`docker-compose.yml`** — QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki, Promtail; named network `nautilus-monster`
- **`prometheus/prometheus.yml`** — Scrape config
- **`grafana/provisioning/`** — Datasources and dashboards as code
- **`promtail/promtail.yml`** — Log shipping config

Default ports and usage: see **`README.md`**.

## Graceful shutdown

- Live service registers SIGINT/SIGTERM where supported; see **`docs/GRACEFUL_SHUTDOWN.MD`** for conceptual notes and **`app/runtime/live_service.py`** for implementation
