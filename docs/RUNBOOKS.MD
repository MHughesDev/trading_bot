# Operations runbooks (HG-12 / FB-I3)

**First-time setup:** see **[`READY_TO_RUN.MD`](READY_TO_RUN.MD)** (checklist: venv, `.env`, Docker, preflight, venues). This file adds secrets, incident response, and execution-mode restart detail beyond that checklist.

## Docker Compose — app + data stack (FB-CONT-002)

Merge **`infra/docker-compose.yml`** (QuestDB, Redis, Qdrant, observability) with **`infra/docker-compose.app.yml`** (control plane + Streamlit, optional **`live`** profile). From the repo root, with **`.env`** present (gitignored; copy **`.env.example`**):

```bash
docker compose -f infra/docker-compose.yml -f infra/docker-compose.app.yml up -d --build
```

**Ports:** **8000** FastAPI (**`GET /status`**, **`GET /metrics`**), **8501** Streamlit. **`NM_CONTROL_PLANE_URL`** for the Streamlit container defaults to **`http://trading-bot-api:8000`**. Host bind mounts **`./data`** → **`/app/data`** for registry, lifecycle, PnL ledger, and universe SQLite paths. Kraken and venue keys remain **only** via **`NM_*`** in **`.env`** — nothing in the image.

## TLS edge — Caddy (FB-CONT-004)

Optional **HTTPS** in front of the same API + Streamlit containers (merge **`infra/docker-compose.edge.yml`** after the app file):

```bash
docker compose -f infra/docker-compose.yml -f infra/docker-compose.app.yml -f infra/docker-compose.edge.yml up -d
```

- **Local dev:** add **`127.0.0.1 api.localhost ui.localhost`** to **`hosts`**. Open **https://api.localhost:8443** and **https://ui.localhost:8443** — Caddy uses **`tls internal`** (self-signed); accept the browser warning or trust Caddy’s local CA (dev only). Streamlit still calls the API over the Docker network at **`http://trading-bot-api:8000`**.
- **Public / production:** edit **`infra/caddy/Caddyfile`** with real DNS names, remove **`tls internal`**, and expose **80**/**443** (or use a host firewall) so Caddy can obtain **Let’s Encrypt** certificates automatically.

**LAN without TLS** remains the default **HTTP** compose path above (ports **8000** / **8501**).

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

## CORS and optional rate limit (FB-AUD-009 / FB-AUD-004)

- **`NM_CONTROL_PLANE_CORS_ALLOW_ORIGINS`**: comma-separated browser origins, or **`*`** (default) for permissive dev + Streamlit/SSE (**FB-AP-034**). **Public deployments:** set explicit origins (e.g. `https://dashboard.example.com`) and terminate TLS at **[TLS edge — Caddy](#tls-edge--caddy-fb-cont-004)** or your LB.
- **`NM_CONTROL_PLANE_RATE_LIMIT_ENABLED`** / **`NM_CONTROL_PLANE_RATE_LIMIT_PER_MINUTE`**: when enabled, applies a **per-client-IP** sliding window before handlers (429 when exceeded). Default **off** for localhost; use **on** when the API is exposed without an edge limiter, or prefer **Caddy/Nginx `rate_limit`** in **`infra/docker-compose.edge.yml`**.
- **Startup warning (FB-AUD-003):** if **`NM_CONTROL_PLANE_API_KEY`** is unset, **`NM_AUTH_SESSION_ENABLED`** is false, and the bind address is **not** loopback-only, uvicorn logs a **WARNING** at startup — fix before exposing the API.

## Production / network exposure (FB-AUD-001)

**Threat model:** The control plane can **start training**, change **params**, **flatten** positions, and drive **lifecycle** APIs. If **`NM_CONTROL_PLANE_API_KEY`** is **unset** and **`NM_AUTH_SESSION_ENABLED`** is **false**, **mutating** routes still succeed (local-dev ergonomics). **Do not** expose that configuration to the internet or an untrusted LAN.

**Checklist for any non-localhost deployment:**

1. **Authentication:** Set **`NM_CONTROL_PLANE_API_KEY`** (clients send **`X-API-Key`**) **or** enable **`NM_AUTH_SESSION_ENABLED`** with operator accounts and session cookies (see **`.env.example`**).
2. **TLS:** Terminate HTTPS in front of the API (e.g. **[TLS edge — Caddy](#tls-edge--caddy-fb-cont-004)**), cloud load balancer, or **Tailscale**/VPN — not raw **HTTP** on a public IP.
3. **Bind / firewall:** Prefer **`NM_CONTROL_PLANE_HOST=127.0.0.1`** if only a reverse proxy on the same host connects upstream; otherwise restrict **security groups** / **iptables** so **:8000** is not world-open unless behind a proxy you control.
4. **CORS:** set **`NM_CONTROL_PLANE_CORS_ALLOW_ORIGINS`** to an explicit allowlist for public hostnames (see [CORS and optional rate limit](#cors-and-optional-rate-limit-fb-aud-009--fb-aud-004)); default **`*`** is for dev + Streamlit/SSE (**FB-AP-034**).
5. **Streamlit:** For shared dashboards, enable the **route guard** and align session env with the API — see **[Streamlit route guard (FB-AUD-002)](#streamlit-route-guard-fb-aud-002)** below.

**Related:** **`README.md`** — *Production control plane hardening* · *Streamlit route guard*; **`docs/AUDIT_CODE_REVIEW.MD`**.

## Streamlit route guard (FB-AUD-002)

**Code:** **`control_plane/streamlit_util.py`** — **`require_streamlit_app_access()`** (called after **`st.set_page_config`** on gated pages; **`pages/0_Login.py`** must **not** call it).

**When to enable:** Set **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=true`** if **:8501** is reachable beyond a **single trusted operator on localhost** (shared LAN, remote host, or dashboard behind a reverse proxy).

**Behavior:**

- **Guard off** (**default `false`**): no redirect — same ergonomics as an unauthenticated API in dev.
- **Guard on** and **`NM_CONTROL_PLANE_API_KEY`** is **set** in the **Streamlit** process: guard **skips** redirect (automation / headless / scripts that use the API key on **`httpx`** calls).
- **Guard on** and **no** API key in Streamlit: redirect to login **unless** **`st.session_state` holds an operator token** and **`GET {NM_CONTROL_PLANE_URL}/auth/me`** returns **200** with the **`Cookie`** header built from that token.

**Session env (must match the FastAPI process):** **`NM_AUTH_SESSION_ENABLED=true`** for login UI; **`NM_CONTROL_PLANE_URL`** points to the API that serves **`/auth/login`** and **`/auth/me`**; **`NM_AUTH_SESSION_COOKIE_NAME`**, **`NM_AUTH_SESSION_TTL_SECONDS`**, **`NM_AUTH_SESSION_COOKIE_SECURE`**, **`NM_AUTH_SESSION_COOKIE_SAMESITE`** — see **`.env.example`**. Misaligned values cause **`/auth/me`** to fail and repeated redirects to login.

**HTTPS vs HTTP:** set **`NM_AUTH_SESSION_COOKIE_SECURE=true`** when the operator uses **HTTPS** (reverse proxy or TLS edge) so browsers only send the cookie over TLS. For **localhost HTTP** development, **`false`** is typical. **`SameSite=lax`** is the default; use **`none`** only with **`Secure`** and HTTPS.

**Note:** Deep links to a Streamlit app page may still show the login flow first; the UI stores the session token in **`session_state`** after **`POST /auth/login`**.

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

## Stateful Docker volumes — backup & restore (FB-CONT-005)

Compose file **`infra/docker-compose.yml`** declares named volumes: **`questdb_data`**, **`redis_data`**, **`qdrant_data`** (plus observability volumes). Docker names them **`<project>_<volume>`** — set a fixed project name so backup scripts stay stable:

```bash
export COMPOSE_PROJECT_NAME=trading-bot
docker compose -f infra/docker-compose.yml --project-directory . config >/dev/null
```

**Cold backup (simple, copy-paste)** — stop writers, archive volume contents, restart:

```bash
export COMPOSE_PROJECT_NAME=trading-bot
cd /path/to/repo
mkdir -p backup/archives

docker compose -f infra/docker-compose.yml stop questdb redis qdrant

docker run --rm \
  -v trading-bot_questdb_data:/data:ro \
  -v "$(pwd)/backup/archives:/out" \
  alpine:3.20 \
  tar czf "/out/questdb-$(date -u +%Y%m%dT%H%MZ).tgz" -C /data .

docker run --rm \
  -v trading-bot_redis_data:/data:ro \
  -v "$(pwd)/backup/archives:/out" \
  alpine:3.20 \
  tar czf "/out/redis-$(date -u +%Y%m%dT%H%MZ).tgz" -C /data .

docker run --rm \
  -v trading-bot_qdrant_data:/data:ro \
  -v "$(pwd)/backup/archives:/out" \
  alpine:3.20 \
  tar czf "/out/qdrant-$(date -u +%Y%m%dT%H%MZ).tgz" -C /data .

docker compose -f infra/docker-compose.yml start questdb redis qdrant
```

If your host uses a different **`COMPOSE_PROJECT_NAME`**, replace **`trading-bot_*`** with the names from **`docker volume ls`**.

**Restore drill (same host)** — **destructive** on the target volume; use a staging VM or copy volumes first.

1. **`docker compose -f infra/docker-compose.yml stop questdb`** (or redis / qdrant).
2. Clear the volume *or* `docker volume rm …` and recreate by **`compose up`** once (empty).
3. **`docker run --rm -v trading-bot_questdb_data:/data -v "$(pwd)/backup/archives:/in" alpine:3.20 tar xzf /in/questdb-YYYYMMDD…tgz -C /data`** (adjust archive name).
4. **`docker compose … start questdb`**. Repeat pattern for Redis / Qdrant.

**Order:** restore **QuestDB** / **Redis** / **Qdrant** before starting app processes that depend on them (**`infra/docker-compose.app.yml`**).

**Object storage:** upload **`backup/archives/*.tgz`** to your S3-compatible bucket (SSE-KMS, versioning, lifecycle rules — no secrets in object keys). **Retention:** keep at least **7** daily and **4** weekly copies per compliance comfort; **test restore** at least **quarterly** on a non-prod host.

**QuestDB note:** tables are partition-oriented — cold **`tar`** of **`/var/lib/questdb`** is acceptable when the **questdb** container is **stopped** so files are consistent.

**Redis note:** with **`appendonly yes`**, the AOF under the Redis volume is included in the archive; restore replaces full history for that instance.

**Qdrant note:** snapshot is the volume **`storage`** tree; large collections → expect large archives.

## Outbound HTTP (`httpx`) — timeouts and TLS (FB-AUD-012)

| Area | Notes |
|------|--------|
| **Streamlit → API** (`control_plane/streamlit_util.py`, chart fetch helpers) | Explicit **`timeout=`** on requests (typically **15–60s**). Default TLS **`verify=True`**. |
| **Coinbase Advanced** (`execution/coinbase_advanced_http.py`) | **`httpx.AsyncClient(timeout=...)`** from settings — do not disable verify for production. |
| **Edge** | For public APIs, prefer **reverse-proxy** rate limits + TLS over tuning client timeouts alone. |

## QuestDB writer failures — metrics (FB-AUD-015)

- Prometheus counter **`tb_questdb_write_fail_total`** with label **`operation`** (`insert_bar`, `insert_decision_trace`, `flush_decision_traces`) increments when the QuestDB writer raises. The live loop still logs **`questdb insert_bar failed`** with stack traces; **`GET /metrics`** exposes the counter for alerting.

## Multi-tenant data scoping (`NM_MULTI_TENANT_DATA_SCOPING`) — review checklist (FB-AUD-016)

- When **true**, session-bound **`tenant_ctx`** scopes paths under **`data/users/<user_id>/`** (registry, lifecycle, PnL, markers, watermarks). **Kraken** shared data plane stays deployment-wide per **AGENTS.md**. When adding new persisted artifacts, resolve paths through **`app/runtime/tenant_context`** (or equivalent) so tenant **A** never reads tenant **B** files.

## Silent `except` patterns — inventory note (FB-AUD-014)

- Broad **`except: pass`** hides failures; prefer **`logger.exception`** on unexpected errors. Quick inventory: `rg "except\\s*:" --glob '*.py' app control_plane execution` and review. Hot paths (live loop) already log most failures; treat **`pass`** only for optional UX (e.g. best-effort logout).

## CI security checks (FB-AUD-018…020)

- **`bash scripts/ci_pip_audit.sh`** — Creates **`.audit-venv/`** (gitignored), `pip install -e ".[dev]"`, runs **`pip-audit -l`** on that environment only. **Merges to `main` require this to pass** in GitHub Actions (same script). This avoids scoring unrelated **system** Python packages (e.g. apt-installed tools).
- **`bash scripts/ci_bandit.sh`** — **Bandit** High-severity-only on application packages (`tests/` excluded). Config: **`pyproject.toml`** `[tool.bandit]`.
- **Gitleaks** — CI runs **`zricethezav/gitleaks:v8.21.2`** in Docker on the full repo. Locally:  
  `docker run --rm -v "$PWD:/repo" zricethezav/gitleaks:v8.21.2 detect --source /repo --verbose --redact`

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
