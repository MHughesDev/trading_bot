# Operations runbooks (HG-12 / FB-I3)

**First-time setup:** see **[`READY_TO_RUN.MD`](READY_TO_RUN.MD)** (checklist: venv, `.env`, Docker, preflight, venues). This file adds secrets, incident response, and execution-mode restart detail beyond that checklist.

## Per-asset model registry (FB-AP-001 / FB-AP-002)

Persisted manifests and HTTP API: **[`PER_ASSET_OPERATOR.MD`](PER_ASSET_OPERATOR.MD)**. **`GET /status`** includes **`asset_model_registry.initialized_symbols`**. Mutating routes require **`X-API-Key`** when **`NM_CONTROL_PLANE_API_KEY`** is set.

## In-process scheduler (FB-AP-035)

The **control plane** process (`uvicorn control_plane.api:app`) can run an optional **background thread** that triggers **`run_nightly_training_job`** on an interval while the API is running. When you **stop** uvicorn, the thread exits — **no** scheduled training runs with the app closed.

- **Enable:** `NM_SCHEDULER_NIGHTLY_ENABLED=true` (or `scheduler.nightly_enabled: true` in `default.yaml`).
- **Interval:** `NM_SCHEDULER_NIGHTLY_INTERVAL_SECONDS` (default **86400**; minimum **1** second in code).
- **Per-asset forecaster (FB-AP-036):** When **`NM_SCHEDULER_NIGHTLY_PER_ASSET_FORECASTER=true`** (default), each tick runs the nightly **quantile forecaster** campaign **once per initialized symbol** (manifest in registry), writing under **`NM_TRAINING_ARTIFACT_DIR/nightly/<SYMBOL>/`**. Set to **`false`** for legacy single-symbol nightly (first market symbol only).
- **Conditional nightly RL (FB-AP-037):** When **`NM_SCHEDULER_NIGHTLY_RL_REQUIRES_TRADE=true`** (default), the **heuristic RL** block in each nightly campaign is **skipped** if **`data/trade_markers.jsonl`** has **no** marker for that symbol in the lookback window (default window = same days as **`NM_TRAINING_LOOKBACK_DAYS`** unless **`NM_SCHEDULER_NIGHTLY_RL_TRADE_LOOKBACK_DAYS`** is set). Forecaster step still runs.
- **Status:** `GET /status` → **`app_scheduler`** (`enabled`, `interval_seconds`, `running`, `last_tick_utc`, `last_error`).
- **Legacy global system power (optional):** With **`NM_SYSTEM_POWER_LEGACY_ENABLED=true`**, **`data/system_power.json`** and **`POST /system/power`** behave as before (OFF skips `run_training_campaign` / `run_decision_tick`). **Default is off** (**FB-AP-039**) — use **per-asset Stop** instead of a global hard stop.

## Per-asset Stop — flatten (FB-AP-032)

**`POST /assets/lifecycle/{symbol}/stop`** (with API key when configured) first attempts a **market** close for that symbol’s **open venue position** (Alpaca paper or Coinbase live per **per-symbol execution mode** / **`NM_EXECUTION_MODE`**), then transitions **`active` → `initialized_not_active`**. If the venue **position fetch** or **submit** fails, the API returns **502** and the symbol **stays active** — fix credentials or venue outage and retry. Residual size after partial fills may require another Stop or manual flatten via **`FLATTEN_ALL`** / **`POST /flatten`** (global).

## Control plane bind address

- Default **`NM_CONTROL_PLANE_HOST=0.0.0.0`** listens on all interfaces. On a single machine behind an untrusted LAN, prefer **`NM_CONTROL_PLANE_HOST=127.0.0.1`** so the API is only reachable from localhost.
- Keep **`NM_CONTROL_PLANE_API_KEY`** set for mutating routes (`POST /params`, `/system/mode`, `/flatten`) whenever the control plane is reachable beyond localhost.

## Execution mode (paper vs live)

- **`NM_EXECUTION_MODE`** remains the **process default** at startup (and `default.yaml` via `load_settings`). **Per-symbol** routing uses **`PUT /assets/execution-mode/{symbol}`** (sidecars) — **FB-AP-030** / **FB-AP-040**.
- **Legacy app-wide profile API (optional):** With **`NM_EXECUTION_PROFILE_LEGACY_API=true`**, `GET /status` includes the full **`execution_profile`** block (`active_execution_mode`, `pending_execution_mode`, `restart_required`), and **`POST /system/execution-profile`** can patch **`default.yaml`** / **`.env`**. **Default is off** — those endpoints return **410**; use per-asset API instead.
- **After Apply:** restart **uvicorn** (control plane), **`python -m app.runtime.power_supervisor`** / **`run.bat`**, and any **`live_service_app`** / **`python -m app.runtime.live_service`** so all processes reload settings.
- **Streamlit:** sidebar **Execution mode** — same POST; watch **restart required** until you restart.

### Live trading checklist (mirrors dashboard guardrails, FB-DASH-06)

Before switching **to live** from paper (or any non-live active mode), complete the same checks the Streamlit UI expects:

1. **Preflight:** `python scripts/preflight_check.py` or **`GET /status`** → **`preflight`** / **`production_preflight`** — resolve blocking issues (signing secret, `NM_ALLOW_UNSIGNED_EXECUTION`, venue keys for live).
2. **Secrets:** `NM_COINBASE_API_KEY` / `NM_COINBASE_API_SECRET` (CDP) for Coinbase live; **`NM_RISK_SIGNING_SECRET`** set; **`NM_ALLOW_UNSIGNED_EXECUTION=false`** for production-like operation.
3. **Control plane:** if **`NM_CONTROL_PLANE_API_KEY`** is set, mutating POSTs (including execution profile) require **`X-API-Key`** — the Streamlit app sends it when **`NM_CONTROL_PLANE_API_KEY`** is in the dashboard process environment.
4. **Dashboard confirmation:** selecting **live** and **Apply** opens a **Confirm live execution** dialog: accept the risk checkbox and type **`LIVE`** (uppercase). Re-applying while the active mode is already **live** does not require the phrase again. Set **`NM_STREAMLIT_LIVE_CONFIRM=false`** only for automation/dev (not recommended for operators).
5. If using **legacy** profile API, **restart** runtime processes after Apply so **`NM_EXECUTION_MODE`** loads; verify **`GET /status` → `execution_profile`** when **`legacy_api_enabled`** is true.

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

## Restart after config change (execution mode / venue)

Execution mode and venue credentials are read from **`AppSettings`** at process start. There is **no hot-swap** of paper vs live or of Alpaca vs Coinbase adapters while a long-lived loop is running.

After changing **`NM_EXECUTION_MODE`**, **`NM_EXECUTION_ADAPTER`**, or venue API keys in **`.env`**:

1. **Stop** the live decision loop: the process running `python -m app.runtime.live_service` or `uvicorn services.live_service_app:app` with **`NM_LIVE_SERVICE_APP_START_LOOP=true`** (SIGTERM/SIGINT).
2. If you use **`python -m app.runtime.power_supervisor`**, restart it so it spawns a fresh **`live_service_app`** with the new environment.
3. **Restart** the execution gateway or any service that caches `load_settings()` if you run split microservices.
4. Re-run **preflight:** `python scripts/preflight_check.py` or **`GET /status`** (`preflight`, `production_preflight`).

Streamlit and the control plane API can often stay up, but **`GET /status`** reflects what the **API process** loaded; the **live loop** must restart to trade under the new mode.

## QuestDB backup / restore

- **Data path:** Docker volume `questdb_data` (see `infra/docker-compose.yml`).
- **Backup:** stop or snapshot the volume; copy QuestDB’s data directory for the container (`/var/lib/questdb` in the official image).
- **Restore:** replace the volume data with a consistent backup while QuestDB is stopped; restart the container.
- **Retention:** QuestDB partition-by-day tables — set external retention policy (snapshot + drop old partitions) per your compliance needs; not automated in-repo.

## Grafana / Loki

- Default URLs (local): Grafana `http://127.0.0.1:3000`, Loki `http://127.0.0.1:3100`. Set `NM_GRAFANA_URL` / `NM_LOKI_URL` for Streamlit links.

## CI integration tests

- Optional: `NM_INTEGRATION_SERVICES=1` with Redis, QuestDB, Qdrant running (`docker compose -f infra/docker-compose.yml up -d`). See `tests/test_integration_optional_services.py` and `tests/test_integration_microservices_redis.py` (Redis + multi-process scaffold chains; includes **stub** and **`mock_alpaca_paper`** paths; skips if Redis is down). Control plane **`GET /microservices/health`** probes default scaffold ports when processes run on the host.

### Real Alpaca paper (microservices / gateway, optional smoke)

No broker keys belong in the repo. To exercise **paper** against Alpaca’s API from your machine:

1. `pip install -e ".[alpaca]"` and set **`NM_ALPACA_API_KEY`**, **`NM_ALPACA_API_SECRET`**, **`NM_EXECUTION_MODE=paper`**, **`NM_EXECUTION_ADAPTER=alpaca`** (or omit adapter to use paper default per settings).
2. Run Redis + **`uvicorn services.execution_gateway_service.main:app`** with **`NM_MESSAGING_BACKEND=redis_streams`**, **`NM_REDIS_URL`**, **`NM_RISK_SIGNING_SECRET`**, and **`NM_EXECUTION_GATEWAY_SUBMIT=true`**.
3. Drive the chain with **`tests/test_integration_microservices_redis.py`** patterns or manual **`POST`** to **`/ingest/risk-accepted`** / upstream services — expect network calls to Alpaca and real account state; use a **paper** account only.

For **no-network** CI, the suite uses **`NM_EXECUTION_ADAPTER=mock_alpaca_paper`** instead (see **`test_redis_external_gateway_mock_alpaca_paper_path`**).

### Microservices compose (`infra/docker-compose.microservices.yml`)

- **`live_runtime` (8208):** default **`NM_LIVE_SERVICE_APP_START_LOOP=false`** — HTTP probes only. Set **`NM_LIVE_SERVICE_APP_START_LOOP=true`** and mount a configured **`.env`** when you want the full Kraken + decision loop inside the container (same requirements as host `live_service`).
- **`market_data_service` (8206):** set **`NM_MARKET_DATA_KRAKEN_WS=true`** to publish **`market.tick.normalized.v1`** from Kraken public WebSocket (needs outbound network). **`GET /messaging`** reports **`kraken_ws`** on/off.
- **QuestDB audit (`FB-MS-03`):** start the base stack so QuestDB is reachable, then set **`NM_QUESTDB_PERSIST_MICROSERVICE_EVENTS=true`** on **`observability_writer`** and point **`NM_QUESTDB_HOST`** (e.g. **`questdb`**) at the DB service if you join networks. Events land in **`microservice_events`** (see `data_plane/storage/schemas.py`).

## Market data gaps / flat synthetic OHLC

- The decision pipeline builds a **`ForecastPacket`** from **`ohlc_arrays_from_feature_row`**: if only the latest `close`/`volume` are present, history is **synthetic flat bars** — forecasts still run, but are **not** a substitute for a full rolling bar window after long WS outages. Prefer healthy WS + `enrich_bars_last_row` feature rows for production-like behavior.

## Model artifacts (master pipeline)

- **Active model set (FB-SPEC-06):** point **`NM_MODELS_ACTIVE_SET_PATH`** at a JSON file whose keys mirror `models:` in `default.yaml` (`forecaster_checkpoint_id`, `forecaster_conformal_state_path`, `forecaster_weights_path`, `policy_mlp_path`). When the file exists, those fields **override** the corresponding `NM_MODELS_*` values for this process (after YAML + env). Optional **`label`** and integer **`version`** in the JSON are echoed in **`GET /status` → `model_artifacts.active_model_set`**. Optional env-only labels: `NM_MODELS_ACTIVE_SET_LABEL`, `NM_MODELS_ACTIVE_SET_MANIFEST_VERSION`.
- **Serving lineage:** set `NM_MODELS_FORECASTER_CHECKPOINT_ID` to label `ForecastPacket.source_checkpoint_id` (operator-visible lineage).
- **Conformal on hot path:** set `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH` to a JSON file produced by `forecaster_model.calibration.conformal.save_conformal_state` after offline calibration; `DecisionPipeline` enables calibration when this path exists.
- **NumPy forecaster weights (FB-SPEC-02):** set `NM_MODELS_FORECASTER_WEIGHTS_PATH` to an **NPZ** produced with `forecaster_model.models.forecaster_weights.save_forecaster_weights` (e.g. from `capture_forecaster_weights_from_seed` after training selection). When present and readable, the hot path uses **fixed weights** instead of per-forward RNG draws; `forecast_diagnostics.weight_source` is `checkpoint`.
- **Policy MLP weights:** set `NM_MODELS_POLICY_MLP_PATH` to an NPZ from `MultiBranchMLPPolicy.save` / `PolicyNetwork.save`; `DecisionPipeline` then uses **`PolicyNetwork`** instead of **`HeuristicTargetPolicy`**.
- **Offline training artifacts:** `orchestration.nightly_retrain` / `training_campaign` write under `NM_TRAINING_ARTIFACT_DIR` (quantile forecaster joblib, reports). See [`docs/Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD).
- **Runtime vs training:** campaign **QuantileRegressor** joblib is still **not** the same object as the NumPy `ForecasterModel` forward; NPZ weights align the **reference** forecaster with a saved draw. **PyTorch MLP** path: install **`[models_torch]`**, run **`tb-train-forecaster-distill`** (or `forecaster_model.training.distill_mlp.train_distilled_mlp_forecaster`) to write **`forecaster_torch.pt`** + manifest under your artifact dir, then set **`NM_MODELS_FORECASTER_TORCH_PATH`** — `DecisionPipeline` loads it and logs **`pytorch_mlp`** (precedence over NPZ / RNG). Full spec-depth VSN/xLSTM **PyTorch** training is optional future work beyond this MLP distill loop. On startup, look for log line `decision pipeline serving mode:` (`pytorch_mlp` vs `npz_weights` vs `numpy_rng`; see [`README.md`](../README.md)).

## Rollback bundle (single host)

Before upgrading binaries or large config changes: copy **`models/artifacts_training/`** (or your `NM_TRAINING_ARTIFACT_DIR`), any **conformal JSON** referenced by `NM_MODELS_FORECASTER_CONFORMAL_STATE_PATH`, and snapshot the **QuestDB** Docker volume if you rely on `decision_traces` history (`infra/docker-compose.yml`).

## Alerting (minimal)

- **Forecast guard / abstain:** monitor `tb_forecast_packet_abstain_total` or logs for repeated guard failures after deploys.
- **Execution errors:** watch adapter logs and `tb_decision_latency_seconds` spikes; tune thresholds for your host (see **FB-AUDIT-09** in backlog for formal rules).

## Status payload: preflight (IL-105 / FB-SPEC-08)

- **`GET /status`** returns **`preflight`** (blocking issues: signing vs live, unsigned execution, adapter names, credentials) and **`production_preflight`** (signing secret, unsigned flag, venue API keys vs `execution_mode`). Use both before live trading; CLI: `python scripts/preflight_check.py`.

## Unified model artifacts (FB-SPEC-03)

- **`GET /status` → `model_artifacts`**: single JSON object with **`active_model_set`** (manifest path / existence / label / version from `models/registry/active_set.py`), **serving** paths (`NM_MODELS_*` for NPZ, conformal JSON, lineage id) with **`file_exists`** flags, plus a **training** note that `forecaster_quantile_real.joblib` is **campaign-only** (sklearn QuantileRegressor), not the NumPy hot path. Implemented in `app/config/model_artifacts.py`.

## Active model set file (FB-SPEC-06)

- Default **`models/registry/active_model_set.json`** — declare intended paths / lineage; **`model_artifacts.registry`** compares to live **`NM_*`** and lists **`drift_vs_env`** when they differ.
- Override path: **`NM_MODELS_ACTIVE_REGISTRY_PATH`**.

## Model registry and version labels (FB-SPEC-06 — partial)

- **In-repo today:** `models/registry/mlflow_registry.py` — optional MLflow **params/metrics** logging when `mlflow` is installed; **`promote()` is intentionally a no-op** (manual promotion only; aligns with CI “no auto MLflow stage” policy). Training campaigns can log under your tracking URI; there is **no** automated “champion vs candidate” pipeline yet (**FB-AUDIT-03**).
- **Operator-facing labels:** set **`NM_MODELS_FORECASTER_CHECKPOINT_ID`** for `ForecastPacket` lineage; use **`POST /models/version`** on the control plane to expose **`component` + `version`** to Prometheus (`tb_model_version_info`). Together with **`model_artifacts`** and artifact paths, this is the current “active model set” surface — a single MLflow-backed promotion workflow remains backlog.

## Live trading checklist (mirrors UI guardrails — FB-DASH-06-02)

Future Streamlit/UI flows may require typing **LIVE** before enabling live execution (**FB-DASH-06-01**). Until that ships, use this **manual** checklist before **`NM_EXECUTION_MODE=live`** and Coinbase (or any live venue):

1. Complete **[`OPERATOR_READY_CHECKLIST.MD`](OPERATOR_READY_CHECKLIST.MD)** (venv, `.env`, Docker if needed).
2. **`NM_RISK_SIGNING_SECRET`** set; **`NM_ALLOW_UNSIGNED_EXECUTION=false`** for production-like runs.
3. **`python scripts/preflight_check.py`** exits **0**; review **`GET /status`** → `preflight` + `production_preflight`.
4. **Venue keys** match intent: Coinbase CDP keys for live adapter; not using Alpaca for market data.
5. **Restart** the live loop after any mode or key change (see [Restart after config change](#restart-after-config-change-execution-mode--venue)).
6. **Monitor** first session: Grafana/Loki, `tb_decision_latency_seconds`, adapter errors.

## Multi-symbol runtime (FB-SPEC-07)

- See [`docs/Specs/MULTI_ASSET_PORTFOLIO.MD`](Specs/MULTI_ASSET_PORTFOLIO.MD): independent per-symbol ticks with **shared** risk caps — no built-in cross-symbol optimizer.
- **As implemented:** `market_data_symbols` drives **independent per-symbol** decision ticks (shared `RiskEngine` / capital where wired, but **no** cross-symbol correlation-aware allocator or risk-parity layer). Adding many symbols increases load and concurrent exposure; size each symbol’s risk budget explicitly in config and monitor aggregate notional.
- **Stretch (open in backlog, overlaps FB-SPEC-07 epic):** cross-symbol capital allocation, correlation-aware sizing, or formal risk-parity — requires `risk_engine/` / `decision_engine/` work.
