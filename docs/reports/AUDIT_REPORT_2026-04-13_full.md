# Audit report — trading_bot (full-scope)

**Report date (UTC):** **2026-04-13**  
**Audited commit:** `504bbe4dcfe290354290100b5b89eccb5f7dc6e2`  
**Playbook:** [`docs/FULL_AUDIT.md`](../FULL_AUDIT.md) (template v3.2)

---

## Metadata

| Field | Value |
|-------|--------|
| Repository | trading_bot |
| Audit lead | Automated full audit (agent) |
| Scope in | Repo root; CI (`.github/workflows/ci.yml`); Python app (`app/`, `control_plane/`, `execution/`, `data_plane/`); `infra/`; `docs/`; tests (`tests/`). |
| Scope out | Production runtime data; live exchange accounts; third-party SaaS beyond what CI exercises. |
| Playbook §2 | Embedded in-repo — see [`FULL_AUDIT.md`](../FULL_AUDIT.md) §2 (updated with this pass). |

---

## Executive summary

**Overall outcome: Pass with findings.**

This repository is a **Python 3.11+** trading stack with **FastAPI** (`control_plane/`), **Streamlit**, **live_service**, **QuestDB/Redis/Qdrant** integration, **Docker**, and **GitHub Actions** CI. Prior hardening work (API keys, CORS, rate limits, QuestDB metrics, spec compliance scripts) is **evident and documented**.

**Strengths:** Ruff + pytest + spec scripts in CI; pip-audit and Trivy (informational) present; queue-system documentation is mature; Kraken-only market-data policy is enforced in CI.

**Top gaps (promoted to backlog):** (1) **pip-audit** is **non-blocking** (`continue-on-error`) — policy should be explicit. (2) No **secret-scanning** (gitleaks/trufflehog) in CI. (3) No **Bandit** (Python SAST) in CI or dev docs.

**No Critical findings** from this documentation-and-config review. Residual gaps: no load/perf numbers, no formal pen-test.

---

## Category results

### G — Governance & scope

- **Verdict:** P  
- **Summary:** Scope is documented in AGENTS.md, queue system in QUEUE_SCHEMA.md; audit playbook present.  
- **Evidence:** `AGENTS.md`, `docs/QUEUE_SCHEMA.md`, `docs/FULL_AUDIT.md`

### SEC-REPO — Secrets & repository hygiene

- **Verdict:** PWF  
- **Summary:** `.env` gitignored; no hardcoded secrets found in `.github/workflows` sample review. No automated secret scanner in CI.  
- **Evidence:** `.gitignore`, `.github/workflows/ci.yml`

### SUP — Supply chain & dependencies

- **Verdict:** PWF  
- **Summary:** `pip-audit` runs in `lint-test` but **does not fail the job** (`continue-on-error: true`). Policy should be documented or tightened.  
- **Evidence:** `.github/workflows/ci.yml` (pip-audit step)

### STATIC — Static analysis & style

- **Verdict:** PWF  
- **Summary:** Ruff enforced in CI. No Bandit or similar Python security linter in CI.  
- **Evidence:** `.github/workflows/ci.yml` (Ruff step)

### CORR — Correctness, safety & concurrency

- **Verdict:** P  
- **Summary:** Large pytest suite; no deep concurrency audit in this pass.  
- **Evidence:** `tests/` (398+ tests at time of audit baseline)

### TEST — Testing & quality gates

- **Verdict:** P  
- **Summary:** pytest required in CI; integration job optional (`workflow_dispatch`).  
- **Evidence:** `.github/workflows/ci.yml`

### APPSEC — Application & API security

- **Verdict:** P  
- **Summary:** Control plane auth patterns documented (API key, sessions); prior FB-AUD items addressed in code review backlog.  
- **Evidence:** `docs/RUNBOOKS.MD`, `control_plane/api.py` (patterns from prior work)

### INFRA — Infrastructure & secrets management

- **Verdict:** P  
- **Summary:** Compose and env-driven config; secrets via `NM_*` and `.env`.  
- **Evidence:** `infra/docker-compose.yml`, `.env.example`

### CONT — Container & image security

- **Verdict:** P  
- **Summary:** Dockerfile + hadolint + Trivy fs scan (informational `exit-code: 0`).  
- **Evidence:** `.github/workflows/ci.yml` (docker-image job)

### DATA — Data durability, backup & recovery

- **Verdict:** P  
- **Summary:** RUNBOOKS covers volume backup; QuestDB writer failure metric exists.  
- **Evidence:** `docs/RUNBOOKS.MD`, `observability/metrics.py`

### PERF — Performance & capacity

- **Verdict:** N/A — *no load/SLO evidence in-repo for this pass; recommend scheduled perf review.*

### REL — Reliability & resilience

- **Verdict:** P  
- **Summary:** Graceful shutdown notes; optional integration tests.  
- **Evidence:** `docs/GRACEFUL_SHUTDOWN.MD` (referenced in specs), CI optional integration

### OBS — Observability & alerting

- **Verdict:** P  
- **Summary:** Prometheus metrics; Grafana/Loki in compose.  
- **Evidence:** `observability/`, `infra/docker-compose.yml`

### OPS — Operational readiness & DR

- **Verdict:** P  
- **Summary:** RUNBOOKS, DEPLOY_CLOUD, preflight scripts.  
- **Evidence:** `docs/RUNBOOKS.MD`, `scripts/preflight_check.py`

### PRIV — Privacy & compliance

- **Verdict:** N/A — *no dedicated compliance review; operator PII in user store documented in auth flows.*

### DOC — Documentation & architecture

- **Verdict:** P  
- **Summary:** README, Specs index, queue system docs aligned.  
- **Evidence:** `README.md`, `docs/Specs/README.MD`

### ML — ML / AI / model governance

- **Verdict:** PWF  
- **Summary:** Training/inference paths exist; MLflow promotion policy in CI — no full model governance audit this pass.  
- **Evidence:** `scripts/ci_mlflow_promotion_policy.sh`

### RLS — Release, change & incident

- **Verdict:** P  
- **Summary:** CI on PR/main; no formal release checklist in this report.  
- **Evidence:** `.github/workflows/ci.yml`

---

## Findings

| Finding ID | Category ID | Severity | Location | Summary | Remediation | Status |
|------------|---------------|----------|----------|---------|-------------|--------|
| AUD-SUP-001 | SUP | Medium | `.github/workflows/ci.yml` | `pip-audit` uses `continue-on-error: true`, so vulnerable deps may not block merges. | Make failing the default **or** document release-blocking policy explicitly in README/RUNBOOKS. | Open |
| AUD-SEC-REPO-001 | SEC-REPO | Medium | CI (missing) | No gitleaks/trufflehog (or equivalent) secret scan on PR/push. | Add optional CI step + local run docs in AGENTS/README. | Open |
| AUD-STATIC-001 | STATIC | Low | CI / dev docs | No Bandit (or equivalent) Python SAST in CI. | Add `bandit` to dev docs and/or optional CI job. | Open |

---

## N/A categories

- **PERF** — N/A for this pass — *no benchmark artifacts or SLO tests reviewed.*

---

## Residual gaps

- Did not run `pip-audit` / `bandit` locally with full output in this session (CI behavior reviewed from workflow YAML).
- No review of cloud account IAM, secrets rotation, or production Grafana dashboards.

---

## Recommended next steps

1. Triage **FB-AUD-018** (pip-audit policy) — highest operational clarity win.  
2. Implement **FB-AUD-019** (secret scanning) before next external contributor surge.  
3. Add **FB-AUD-020** (bandit) when convenient — low cost, incremental coverage.

---

## Sign-off

| Role | Name | Date |
|------|------|------|
| Engineering | *(pending)* | 2026-04-13 |

---

*Findings promoted to [`docs/QUEUE_STACK.csv`](../QUEUE_STACK.csv) as **FB-AUD-018** … **FB-AUD-020**.*
