# Testing and CI

## Tests (`tests/`)

- Pytest; **`asyncio_mode = auto`** in `pyproject.toml`
- Coverage areas include: decision/risk, replay (single and multi-asset), rolling bars, Coinbase REST fallback, control-plane auth, routing, normalizers, QuestDB batching, integration optional (Redis/QuestDB/Qdrant when `NM_INTEGRATION_SERVICES=1`)

## Scripts (`scripts/`)

- **`ci_spec_compliance.sh`** — Alpaca market-data import guard (excludes `legacy/`)
- **`ci_mlflow_promotion_policy.sh`** — Blocks forbidden MLflow promotion APIs in main code
- **`smoke_credentials.py`** — Optional connectivity smoke (no secrets in repo)

## GitHub Actions

- **`.github/workflows/ci.yml`** — Ruff, pytest, both CI scripts above
- Optional integration job can be triggered manually (`workflow_dispatch`) when services are available

## Optional integration tests (local / pre-release)

Before a release or when changing storage paths, start the stack and run integration tests:

```bash
docker compose -f infra/docker-compose.yml up -d
NM_INTEGRATION_SERVICES=1 python3 -m pytest tests/test_integration_optional_services.py -q
```

Requires Redis, QuestDB, and Qdrant reachable at the URLs in `app/config` defaults.
