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

## Model artifacts (master pipeline)

- **Serving lineage:** set `NM_MODELS_FORECASTER_CHECKPOINT_ID` to label `ForecastPacket.source_checkpoint_id` (operator-visible lineage; does not load weights by itself — see backlog FB-SPEC-02).
- **Conformal on hot path:** set `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH` to a JSON file produced by `forecaster_model.calibration.conformal.save_conformal_state` after offline calibration; `DecisionPipeline` enables calibration when this path exists.
- **Offline training artifacts:** `orchestration.nightly_retrain` / `training_campaign` write under `NM_TRAINING_ARTIFACT_DIR` (quantile forecaster joblib, reports). See [`docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD).
