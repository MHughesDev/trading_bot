# Observability and infrastructure

## Observability (`observability/`)

- **`metrics.py`** — Prometheus metrics (e.g. decision latency, feed stale blocks, normalizer unknowns)
- **`logging.py`** — Structured logging setup

## Infrastructure (`infra/`)

- **`docker-compose.yml`** — QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki, Promtail; named network `nautilus-monster`
- **`prometheus/prometheus.yml`** — Scrape config
- **`grafana/provisioning/`** — Datasources and dashboards as code
- **`promtail/promtail.yml`** — Log shipping config

Default ports and usage: see **`README.md`**.

## Graceful shutdown

- Live service registers SIGINT/SIGTERM where supported; see **`docs/GRACEFUL_SHUTDOWN.MD`** for conceptual notes and **`app/runtime/live_service.py`** for implementation
