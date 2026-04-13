# Audit report — trading_bot (full-scope)

**Report date (UTC):** **2026-04-13**  
**Audited commit:** `WORKTREE_UNCOMMITTED`  
**Playbook:** [`docs/governance/full_audit.md`](../full_audit.md) (template v3.2)

---

## Metadata

| Field | Value |
|-------|--------|
| Repository | trading_bot |
| Audit lead | Automated full audit (agent) |
| Scope in | Repo root, CI (`.github/workflows/ci.yml`), runtime packages (`app/`, `data_plane/`, `execution/`, `decision_engine/`, `risk_engine/`, `control_plane/`), `infra/`, docs, and tests. |
| Scope out | Live exchange accounts, production cloud IAM/secrets manager state, and external SaaS internals not represented in-repo. |
| Runtime used for this pass | Local `python3` = **3.12.12** |

---

## Executive summary

**Overall outcome: Pass with findings (PWF).**

This pass was executed on a CI-aligned interpreter baseline (Python 3.12), and core policy guards passed (`ruff`, Kraken-only spec compliance, queue consistency, MLflow promotion policy). I added package-index preflight checks to the setup/audit scripts in this cycle (`scripts/env_preflight.py`, `setup.sh`, `setup.bat`, `scripts/ci_pip_audit.sh`) so environment failures now fail-fast with precise diagnostics. The remaining blockers are external to repository code: local `pytest` collection still fails because required project dependencies are not installable in the current runtime, and security tooling checks (`pip-audit`, `bandit`) cannot complete due proxy/index restrictions and missing module installation.

---

## Category results

### G — Governance & scope
- **Verdict:** P  
- **Evidence:** `AGENTS.md`, `docs/governance/full_audit.md`, `docs/QUEUE_SCHEMA.md`.

### SEC-REPO — Secrets & repository hygiene
- **Verdict:** PWF  
- **Summary:** Secret-scanning capability exists in CI (`gitleaks` job), but this local pass did not execute it.
- **Evidence:** `.github/workflows/ci.yml`.

### SUP — Supply chain & dependencies
- **Verdict:** PWF  
- **Summary:** `ci_pip_audit.sh` is release-blocking by policy; it now fails-fast via `scripts/env_preflight.py` and in this container reports proxy tunnel `403 Forbidden` to package index.
- **Evidence:** `scripts/ci_pip_audit.sh`, local command output, `.github/workflows/ci.yml`.

### STATIC — Static analysis & style
- **Verdict:** PWF  
- **Summary:** Ruff passed locally; Bandit check failed because `bandit` module is not installed in this local environment.
- **Evidence:** `scripts/ci_bandit.sh`, local command output.

### CORR — Correctness, safety & concurrency
- **Verdict:** PWF  
- **Summary:** Full test suite failed during collection due missing runtime dependencies (`pydantic`, `fastapi`, `yaml`, `httpx`, `polars`, `numpy`) in the local environment.
- **Evidence:** `pyproject.toml`, local `pytest` output.

### TEST — Testing & quality gates
- **Verdict:** PWF  
- **Summary:** Test gate exists in CI, but local `pytest` cannot provide signal until dependencies are installed.
- **Evidence:** `.github/workflows/ci.yml`, local `pytest` output.

### APPSEC — Application & API security
- **Verdict:** P  
- **Evidence:** CI controls + architecture/docs inspection.

### INFRA — Infrastructure & secrets management
- **Verdict:** P  
- **Evidence:** `infra/docker-compose.yml`, env-driven settings model.

### CONT — Container & image security
- **Verdict:** P  
- **Evidence:** `.github/workflows/ci.yml` docker-image job (hadolint + build + Trivy).

### DATA — Data durability, backup & recovery
- **Verdict:** P  
- **Evidence:** runbooks and storage/observability coverage in repo docs/modules.

### PERF — Performance & capacity
- **Verdict:** N/A — no benchmark/SLO artifact set produced in this pass.

### REL — Reliability & resilience
- **Verdict:** P  
- **Evidence:** graceful operation/recovery docs and scripts in repository.

### OBS — Observability & alerting
- **Verdict:** P  
- **Evidence:** `observability/` modules and local infra stack declarations.

### OPS — Operational readiness & DR
- **Verdict:** P  
- **Evidence:** `docs/operations/runbooks.md` and operational scripts.

### PRIV — Privacy & compliance
- **Verdict:** N/A — no dedicated compliance evidence package assembled in this pass.

### DOC — Documentation & architecture
- **Verdict:** P  
- **Evidence:** README and `docs/*.MD` provide broad operator coverage.

### ML — ML / AI / model governance
- **Verdict:** P  
- **Summary:** MLflow auto-promotion guard passed locally.
- **Evidence:** `scripts/ci_mlflow_promotion_policy.sh`.

### RLS — Release, change & incident
- **Verdict:** P  
- **Evidence:** CI policy/jobs on `main`/PR.

---

## Findings

| Finding ID | Category | Severity | Location | Summary | Recommended remediation | Status |
|---|---|---|---|---|---|---|
| AUD-TEST-2026-04-13-001 | CORR / TEST | High | local environment + `pyproject.toml` | `pytest` cannot collect due missing core dependencies (`pydantic`, `fastapi`, `httpx`, `polars`, `yaml`, `numpy`). | Run `pip install -e ".[dev]"` (or `./setup.sh`/`setup.bat`) in the active Python 3.12 environment, then rerun full tests. | Open |
| AUD-SUP-2026-04-13-001 | SUP | Medium | `scripts/env_preflight.py`, `scripts/ci_pip_audit.sh` | Package-index connectivity check fails with proxy tunnel `403 Forbidden`, so pip-audit bootstrap cannot run in this container. | Point `PIP_INDEX_URL` at a reachable mirror (or repair proxy allowlist/credentials), then rerun full audit checks. | Open |
| AUD-STATIC-2026-04-13-001 | STATIC | Medium | `scripts/ci_bandit.sh` | Bandit check failed locally: `No module named bandit`. | Install dev extras in the active env and rerun `bash scripts/ci_bandit.sh`. | Open |

---

## Validation log (this pass)

- ✅ `python3 --version` *(3.12.12)*
- ✅ `python3 -m ruff check .`
- ❌ `python3 -m pytest tests/ -q`
- ✅ `bash scripts/ci_spec_compliance.sh`
- ✅ `python3 scripts/ci_queue_consistency.py`
- ❌ `bash scripts/ci_pip_audit.sh` *(fails-fast at env preflight: proxy tunnel `403 Forbidden`)*
- ❌ `bash scripts/ci_bandit.sh`
- ✅ `bash scripts/ci_mlflow_promotion_policy.sh`

---

## Residual gaps

- No successful local full-test result yet due missing dependencies.
- No successful local pip-audit/bandit artifact due environment/network/tooling limitations.
- No local gitleaks artifact captured in this run (CI job exists).

---

## Recommended next actions

1. Bootstrap dependencies in active Python 3.12 environment (`pip install -e ".[dev]"` or `./setup.sh`).
2. Re-run `pytest`, `ci_pip_audit.sh`, and `ci_bandit.sh` and attach artifacts.
3. Optionally add `make audit-local` to standardize prerequisite checks and command order.
