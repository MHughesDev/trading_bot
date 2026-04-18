# Testing and CI

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Tests (`tests/`)

- Pytest; **`asyncio_mode = auto`** in `pyproject.toml`
- Coverage areas include: decision/risk, replay (single and multi-asset), rolling bars, Kraken REST/backfill paths, control-plane auth, routing, normalizers, QuestDB batching, integration optional (Redis/QuestDB/Qdrant when `NM_INTEGRATION_SERVICES=1`)

## Scripts (`scripts/`)

- **`ci_spec_compliance.sh`** — Fails if `alpaca.data` / Alpaca historical data imports appear outside the paper adapter (excludes `legacy/`) — keeps market data Kraken-only
- **`ci_mlflow_promotion_policy.sh`** — Blocks forbidden MLflow promotion APIs in main code
- **`ci_canonical_contracts.sh`** — **FB-CAN-013:** runs `validate_release_gates.py` on `tests/fixtures/canonical_release_candidate_live.json` so merge gates exercise APEX promotion evidence checks; canonical domain unit tests live under `tests/test_canonical_*.py`, `test_trigger_engine.py`, `test_auction_engine.py`, etc., and run in the main pytest job
- **`ci_canonical_gates.sh`** (**FB-CAN-025**) — runs `ci_canonical_config_gates.py` (required `apex_canonical` metadata + `domains.risk_sizing` in `default.yaml`, valid loaded bundle), `ci_forbidden_legacy_paths.sh` (deleted modules must not reappear; no `get_execution_adapter` under `execution/`), and `ci_replay_determinism.py` (canonical replay events stable across two runs)
- **`ci_canonical_magic_constants.py`** (**FB-CAN-067**) — AST scan of `decision_engine/`, `risk_engine/`, `execution/`, `app/config/` for new untracked float literals in assignments/comparisons; baseline lines in `scripts/ci_canonical_magic_constants_allowlist.txt`; same-line `# noqa: canonical-magic` to document an intentional literal; prefer moving policy thresholds to `apex_canonical` YAML
- **`ci_queue_consistency.py`** — **FB-CAN-025:** Open rows with batch `CAN-*` must appear in the §2.7 canonical program table in `QUEUE_ARCHIVE.MD`
- **FB-CAN-024:** removed overlapping “master pipeline only” tests; `test_pipeline_forecast_packet.py` + `test_decision_risk.py` assert the shared tick including `canonical_structure` on the packet and canonical fields on `RiskState` after `DecisionPipeline.step`
- **`smoke_credentials.py`** — Optional connectivity smoke (no secrets in repo)

## GitHub Actions

- **`.github/workflows/ci.yml`** — Ruff, pytest, spec compliance, queue consistency, MLflow policy, **canonical release-gate fixture** (`ci_canonical_contracts.sh`)
- Optional integration job can be triggered manually (`workflow_dispatch`) when services are available

**Systemic CI failures (all builds red):** use the formal prompt in **[`docs/CI_ROOT_CAUSE_ANALYSIS_PROMPT.md`](../CI_ROOT_CAUSE_ANALYSIS_PROMPT.md)** — single root-cause analysis aligned to this workflow.

## Optional integration tests (local / pre-release)

Before a release or when changing storage paths, start the stack and run integration tests:

```bash
docker compose -f infra/docker-compose.yml up -d
NM_INTEGRATION_SERVICES=1 python3 -m pytest tests/test_integration_optional_services.py -q
```

Requires Redis, QuestDB, and Qdrant reachable at the URLs in `app/config` defaults.
