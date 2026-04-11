# Operations runbooks (HG-12 / FB-I3)

## Secrets

- Store **only** in `.env` or your secret manager; never commit keys. Prefix application settings with `NM_` (see `app/config/settings.py`).
- **Risk signing:** set `NM_RISK_SIGNING_SECRET` in production so only `RiskEngine`-signed `OrderIntent` values reach venues.
- **Control plane mutations:** set `NM_CONTROL_PLANE_API_KEY`; clients send `X-API-Key` on POST `/params`, `/system/mode`, `/flatten`.
- **Alpaca paper:** `NM_ALPACA_API_KEY`, `NM_ALPACA_API_SECRET`.
- **Coinbase live (CDP):** `NM_COINBASE_API_KEY`, `NM_COINBASE_API_SECRET` (JWT built in adapter).

## Incident response

1. **Pause new entries:** POST `/system/mode` with `{"mode": "PAUSE_NEW_ENTRIES"}` (requires API key if configured).
2. **Flatten:** POST `/flatten` or set mode `FLATTEN_ALL` via `/system/mode`.
3. **Stop runtime:** SIGTERM/SIGINT to `python -m app.runtime.live_service` (graceful shutdown depends on platform signal support).
4. **Review traces:** QuestDB `decision_traces` table when `NM_QUESTDB_PERSIST_DECISION_TRACES=true`; Grafana/Loki for logs.

## QuestDB backup / restore

- **Data path:** Docker volume `questdb_data` (see `infra/docker-compose.yml`).
- **Backup:** stop or snapshot the volume; copy QuestDB’s data directory for the container (`/var/lib/questdb` in the official image).
- **Restore:** replace the volume data with a consistent backup while QuestDB is stopped; restart the container.
- **Retention:** QuestDB partition-by-day tables — set external retention policy (snapshot + drop old partitions) per your compliance needs; not automated in-repo.

## Grafana / Loki

- Default URLs (local): Grafana `http://127.0.0.1:3000`, Loki `http://127.0.0.1:3100`. Set `NM_GRAFANA_URL` / `NM_LOKI_URL` for Streamlit links.

## CI integration tests

- Optional: `NM_INTEGRATION_SERVICES=1` with Redis, QuestDB, Qdrant running (`docker compose -f infra/docker-compose.yml up -d`). See `tests/test_integration_optional_services.py`.

## Model artifacts and forecast routing (FB-PL-PG9)

- **Regime / Ridge forecast joblib:** point `NM_MODELS_REGIME_PATH` and `NM_MODELS_FORECAST_PATH` at known-good `.joblib` files (or clear the env vars to use in-process bootstrap models). Restart the runtime after changing paths.
- **Routing source:** `NM_MODELS_DECISION_FORECAST_ROUTING_SOURCE` = `ridge` (default) uses the Ridge surrogate `ForecastOutput` for routing; `packet` derives routing from the methodology `ForecastPacket` (see `decision_engine/forecast_packet_adapter.py`). Use `ridge` to “roll back” behavior if packet routing misbehaves.
- **Forecast packet diagnostics:** `NM_MODELS_DECISION_FORECAST_PACKET_ENABLED=true` builds a `ForecastPacket` each pipeline step for logging/policy experiments without changing routing (unless `routing_source` is `packet`).
- **Conformal state (offline):** `ForecasterConfig.calibration_enabled` with `build_forecast_packet_methodology(..., conformal_state_path=...)` loads persisted JSON from `forecaster_model.calibration.conformal.load_conformal_state`; use `save_conformal_state` after training-window updates (FB-FR-PG3).
- **Spec decision pipeline (default):** **`ForecastPacket` + `PolicySystem`** — `NM_MODELS_DECISION_PIPELINE_MODE=spec_policy` (default). Set to **`legacy`** to revert to Ridge + router (see [`docs/MIGRATION_TO_SPEC_PIPELINE.MD`](MIGRATION_TO_SPEC_PIPELINE.MD)).
- **Trained weights vs stub/heuristic (IL-103):** In **`spec_policy`** mode the hot path still uses **`build_forecast_packet_stub`** for the packet and **`HeuristicTargetPolicy`** in `PolicySystem` until **FB-SPEC-02** / **FB-FR-P0** / **FB-PL-P0** load PyTorch checkpoints. The runtime logs a **WARNING** once at `DecisionPipeline` startup when `spec_policy` is active. Do not assume neural ensemble weights are live without configuring those artifacts.

## Production preflight (IL-105)

- **GET `/status`** includes **`production_preflight`**: signing secret present, unsigned execution disallowed for live, and venue API keys present for the configured execution mode. Use it before enabling live trading; see **FB-SPEC-08** for broader deployment checks.
