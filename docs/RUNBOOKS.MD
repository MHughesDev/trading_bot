# Operations runbooks (HG-12 / FB-I3)

## Control plane bind address

- Default **`NM_CONTROL_PLANE_HOST=0.0.0.0`** listens on all interfaces. On a single machine behind an untrusted LAN, prefer **`NM_CONTROL_PLANE_HOST=127.0.0.1`** so the API is only reachable from localhost.
- Keep **`NM_CONTROL_PLANE_API_KEY`** set for mutating routes (`POST /params`, `/system/mode`, `/flatten`) whenever the control plane is reachable beyond localhost.

## Preflight (before live or production-like paper)

- Run: `python scripts/preflight_check.py` — exits **1** if blocking issues (e.g. `execution_mode=live` without `NM_RISK_SIGNING_SECRET`, or `NM_ALLOW_UNSIGNED_EXECUTION=true` in live).
- Or call **`GET /status`** on the control plane — response includes **`preflight`** with `ok`, `severity`, `issues`, `warnings`.

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

## Market data gaps / flat synthetic OHLC

- The decision pipeline builds a **`ForecastPacket`** from **`ohlc_arrays_from_feature_row`**: if only the latest `close`/`volume` are present, history is **synthetic flat bars** — forecasts still run, but are **not** a substitute for a full rolling bar window after long WS outages. Prefer healthy WS + `enrich_bars_last_row` feature rows for production-like behavior.

## Model artifacts (master pipeline)

- **Serving lineage:** set `NM_MODELS_FORECASTER_CHECKPOINT_ID` to label `ForecastPacket.source_checkpoint_id` (operator-visible lineage).
- **Conformal on hot path:** set `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH` to a JSON file produced by `forecaster_model.calibration.conformal.save_conformal_state` after offline calibration; `DecisionPipeline` enables calibration when this path exists.
- **NumPy forecaster weights (FB-SPEC-02):** set `NM_MODELS_FORECASTER_WEIGHTS_PATH` to an **NPZ** produced with `forecaster_model.models.forecaster_weights.save_forecaster_weights` (e.g. from `capture_forecaster_weights_from_seed` after training selection). When present and readable, the hot path uses **fixed weights** instead of per-forward RNG draws; `forecast_diagnostics.weight_source` is `checkpoint`.
- **Policy MLP weights:** set `NM_MODELS_POLICY_MLP_PATH` to an NPZ from `MultiBranchMLPPolicy.save` / `PolicyNetwork.save`; `DecisionPipeline` then uses **`PolicyNetwork`** instead of **`HeuristicTargetPolicy`**.
- **Offline training artifacts:** `orchestration.nightly_retrain` / `training_campaign` write under `NM_TRAINING_ARTIFACT_DIR` (quantile forecaster joblib, reports). See [`docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD).
- **Runtime vs training:** campaign **QuantileRegressor** joblib is still **not** the same object as the NumPy `ForecasterModel` forward; NPZ weights align the **reference** forecaster with a saved draw. **PyTorch** checkpoints remain training-side until a torch export path is added (**FB-FR-P0**). On startup, look for log line `decision pipeline serving mode:` (`numpy_rng` vs `npz_weights`, `heuristic` vs `mlp_npz`; see [`README.md`](../README.md)).

## Rollback bundle (single host)

Before upgrading binaries or large config changes: copy **`models/artifacts_training/`** (or your `NM_TRAINING_ARTIFACT_DIR`), any **conformal JSON** referenced by `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH`, and snapshot the **QuestDB** Docker volume if you rely on `decision_traces` history (`infra/docker-compose.yml`).

## Alerting (minimal)

- **Forecast guard / abstain:** monitor `nm_forecast_packet_abstain_total` or logs for repeated guard failures after deploys.
- **Execution errors:** watch adapter logs and `nm_decision_latency_seconds` spikes; tune thresholds for your host (see **FB-AUDIT-09** in backlog for formal rules).

## Status payload: preflight (IL-105 / FB-SPEC-08)

- **`GET /status`** returns **`preflight`** (blocking issues: signing vs live, unsigned execution, adapter names, credentials) and **`production_preflight`** (signing secret, unsigned flag, venue API keys vs `execution_mode`). Use both before live trading; CLI: `python scripts/preflight_check.py`.

## Unified model artifacts (FB-SPEC-03)

- **`GET /status` → `model_artifacts`**: single JSON object describing **serving** paths (`NM_MODELS_*` for NPZ, conformal JSON, lineage id) with **`file_exists`** flags, plus a **training** note that `forecaster_quantile_real.joblib` is **campaign-only** (sklearn QuantileRegressor), not the NumPy hot path. Implemented in `app/config/model_artifacts.py`.
