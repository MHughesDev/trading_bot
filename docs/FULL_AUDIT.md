# Full-scope audit — playbook & checklist

**Purpose:** A **single, repeatable** document for running a **complete** audit pass across security, quality, performance, operations, and data. Use it in **any** repository (web API, batch jobs, mobile backends, ML systems, etc.). **Mark sections N/A** when they do not apply to your stack, but record *why*.

**How this differs from narrow audits:** A “code review audit” or “CVE scan” is one *slice*. A **full audit** walks **every category** below in one coordinated pass (or a scheduled rotation), updates **last-run dates**, and links **evidence** (reports, dashboards, tickets).

---

## 1. Audit record (update every full pass)

| Field | Value |
|--------|--------|
| **Repository / product** | `[NAME]` |
| **Version / commit audited** | `[GIT_SHA or TAG]` |
| **Full audit — last completed (UTC)** | **`YYYY-MM-DD`** — *(initial: not yet run)* |
| **Full audit — lead / role** | `[Name, team]` |
| **Scope notes** | *(e.g. production only, staging + prod, exclude `legacy/`)* |
| **Overall outcome** | ☐ Pass ☐ Pass with findings ☐ Blocked |

**Per-category last run (optional but recommended):** Update when you execute that category as part of *any* audit (full or partial). During a **full audit**, set each category’s date to the full-audit date if you completed it.

| Category | ID | Last run (UTC) | Outcome (P / PWF / B) | Evidence link / path |
|----------|----|----------------|------------------------|----------------------|
| Governance & scope | G | | | |
| Secrets & repository hygiene | SEC-REPO | | | |
| Supply chain & dependencies | SUP | | | |
| Static analysis & style | STATIC | | | |
| Correctness, safety & concurrency | CORR | | | |
| Testing & quality gates | TEST | | | |
| Application & API security | APPSEC | | | |
| Infrastructure & secrets management | INFRA | | | |
| Container & image security | CONT | | | |
| Data durability, backup & recovery | DATA | | | |
| Performance & capacity | PERF | | | |
| Reliability & resilience | REL | | | |
| Observability & alerting | OBS | | | |
| Operational readiness & DR | OPS | | | |
| Privacy & compliance | PRIV | | | |
| Documentation & architecture | DOC | | | |
| ML / AI / model governance *(if applicable)* | ML | | | |
| Release, change & incident process | RLS | | | |

*(Outcome: **P** = Pass, **PWF** = Pass with findings, **B** = Blocked.)*

---

## 2. How to run a full audit (instructions)

1. **Schedule:** Block time; a full audit is usually **multi-hour to multi-day** depending on system size.
2. **Freeze scope:** Decide environment(s), version/commit, and exclusions (e.g. `vendor/`, `legacy/`).
3. **Work top to bottom:** For each **§3** category, execute checks, capture artifacts, and fill the tables in **§1**.
4. **N/A discipline:** If a section does not apply, add one line: *“N/A — [reason]”* under that checklist.
5. **Track findings:** File issues with severity, owner, and due date; link them in **§4**.
6. **Sign-off:** Update **§1** full audit date and commit this file (or export a dated appendix).

**Automation:** Wire as many checks as practical into CI/CD; this document still captures **manual** verification (threat modeling, DR drills, SLO reviews).

---

## 3. Categories — definitions, tools, checklists

Below, **tools** are **examples**; substitute your stack’s equivalents (`eslint`, `golangci-lint`, `cargo audit`, `npm audit`, `terraform validate`, etc.).

### G — Governance & scope

**Goal:** Clear ownership, scope, and risk tolerance for the audit.

- [ ] **G-1** Stakeholders and system boundary documented (what is in / out of scope).
- [ ] **G-2** Threat model or risk register exists or was reviewed (even lightweight).
- [ ] **G-3** Regulatory / contractual obligations identified (PCI, HIPAA, SOC2, etc.) or explicitly N/A.

---

### SEC-REPO — Secrets & repository hygiene

**Goal:** No secrets in source; safe history and clone practices.

- [ ] **SEC-REPO-1** Scan for committed secrets (`gitleaks`, `trufflehog`, `git-secrets`, or platform secret scanning).
- [ ] **SEC-REPO-2** `.gitignore` / `.dockerignore` appropriate; no credential files tracked.
- [ ] **SEC-REPO-3** Pre-commit or CI blocks obvious secret patterns where feasible.

---

### SUP — Supply chain & dependencies

**Goal:** Known dependencies, known vulnerabilities, reproducible installs.

- [ ] **SUP-1** Lockfile or reproducible resolution strategy documented (`package-lock.json`, `poetry.lock`, `Cargo.lock`, etc.).
- [ ] **SUP-2** CVE / advisory scan on dependencies (`npm audit`, `pip-audit`, `cargo audit`, Dependabot, Snyk, OSV).
- [ ] **SUP-3** SBOM generated or tooling in place (`cyclonedx`, Syft) for critical releases.
- [ ] **SUP-4** Review pinned vs floating deps for critical paths; document update policy.

---

### STATIC — Static analysis & style

**Goal:** Consistent style and machine-detectable bug classes.

- [ ] **STATIC-1** Linter / formatter enforced in CI (e.g. Ruff, ESLint, RuboCop, golangci-lint).
- [ ] **STATIC-2** Type checking if applicable (mypy, TypeScript `strict`, etc.).
- [ ] **STATIC-3** Security-oriented static rules where available (e.g. Semgrep, CodeQL, bandit for Python).

---

### CORR — Correctness, safety & concurrency

**Goal:** Logic matches intent; safe failure modes; no data races / deadlocks in critical paths.

- [ ] **CORR-1** Critical paths identified and reviewed (auth, payments, trading, PII, etc.).
- [ ] **CORR-2** Error handling: no silent swallow of exceptions in security-sensitive code without justification.
- [ ] **CORR-3** Concurrency: locks, async correctness, timeouts, cancellation reviewed where relevant.
- [ ] **CORR-4** Integer overflow, time zones, and unit confusion checked for domain-specific code.

---

### TEST — Testing & quality gates

**Goal:** Automated regression safety net aligned with risk.

- [ ] **TEST-1** Unit tests pass in CI on default branch.
- [ ] **TEST-2** Integration / contract tests for external services or APIs you own or mock.
- [ ] **TEST-3** Coverage or critical-path test policy documented (percentage is optional; absence of tests in hot paths is not).
- [ ] **TEST-4** End-to-end or smoke tests for release candidates (where applicable).

---

### APPSEC — Application & API security

**Goal:** OWASP-style controls for apps and HTTP APIs.

- [ ] **APPSEC-1** Authentication and authorization model documented; least privilege enforced.
- [ ] **APPSEC-2** Session / token handling (rotation, storage, CSRF if cookie-based web).
- [ ] **APPSEC-3** Input validation and output encoding; parameterized queries / ORM use.
- [ ] **APPSEC-4** CORS, CSP, HSTS, security headers reviewed for web surfaces.
- [ ] **APPSEC-5** Rate limiting / abuse controls for public or sensitive endpoints (or documented compensating controls, e.g. WAF).
- [ ] **APPSEC-6** Dependency surface for deserialization (`pickle`, unsafe YAML, etc.) avoided or isolated.

---

### INFRA — Infrastructure & secrets management

**Goal:** Safe configuration in all environments.

- [ ] **INFRA-1** Secrets only from vault / env / orchestrator secrets — not baked into images or repos.
- [ ] **INFRA-2** Least-privilege IAM / service accounts for deploy and runtime.
- [ ] **INFRA-3** Network segmentation and firewall rules reviewed (DB not public, etc.).
- [ ] **INFRA-4** TLS for data in transit where required; cert lifecycle managed.

---

### CONT — Container & image security

**Goal:** Minimal, patched images; no leaked secrets in layers.

- [ ] **CONT-1** Dockerfile lint (e.g. Hadolint) and non-root user where feasible.
- [ ] **CONT-2** Image scan for CVEs (Trivy, Grype) — policy for failing vs informational.
- [ ] **CONT-3** Base image update cadence defined.

---

### DATA — Data durability, backup & recovery

**Goal:** No silent data loss; recovery tested.

- [ ] **DATA-1** Backup strategy documented (frequency, retention, encryption).
- [ ] **DATA-2** Restore drill performed on schedule or evidence of last successful restore.
- [ ] **DATA-3** Under failure: DB replication / fsync / WAL expectations documented.
- [ ] **DATA-4** Migrations backward-compatible or rollback plan exists.

---

### PERF — Performance & capacity

**Goal:** Meet latency and throughput needs; avoid memory leaks; cost-aware.

- [ ] **PERF-1** **Profiling:** CPU hot paths identified (profiler appropriate to stack).
- [ ] **PERF-2** **Memory:** leak checks or heap snapshots for long-running processes.
- [ ] **PERF-3** **Latency:** SLOs or targets documented; measured against p50/p95/p99 where relevant.
- [ ] **PERF-4** **Strategies:** caching, batching, async I/O, query tuning — backlog for top issues.
- [ ] **PERF-5** Load or stress test for critical endpoints / pipelines (representative traffic).

---

### REL — Reliability & resilience

**Goal:** Degrade gracefully; recover automatically where possible.

- [ ] **REL-1** Timeouts and retries with backoff on outbound calls; idempotency where needed.
- [ ] **REL-2** Graceful shutdown (drain connections, finish in-flight work) documented and verified.
- [ ] **REL-3** Chaos or failure injection (optional) or documented failure modes.
- [ ] **REL-4** HA / failover assumptions explicit (single AZ vs multi-region).

---

### OBS — Observability & alerting

**Goal:** Debug production issues quickly; alert on symptoms not only “server down.”

- [ ] **OBS-1** Structured logging; no PII in logs or redaction policy.
- [ ] **OBS-2** Metrics (RED/USE or domain metrics) for critical paths.
- [ ] **OBS-3** Distributed tracing if microservices or async pipelines.
- [ ] **OBS-4** Alerts actionable; on-call runbook links; alert noise reviewed.

---

### OPS — Operational readiness & disaster recovery

**Goal:** Humans can operate the system safely.

- [ ] **OPS-1** Runbooks for common failures (deploy rollback, DB full disk, dependency down).
- [ ] **OPS-2** DR plan: RTO/RPO targets or explicit “none” with risk acceptance.
- [ ] **OPS-3** On-call rotation and escalation path defined for production.

---

### PRIV — Privacy & compliance

**Goal:** Handle personal data lawfully.

- [ ] **PRIV-1** Data inventory: what PII exists, where stored, retention.
- [ ] **PRIV-2** Legal basis / DPA / subprocessors documented or N/A.
- [ ] **PRIV-3** User rights process (export/delete) if applicable.

---

### DOC — Documentation & architecture

**Goal:** New engineers and auditors can understand the system.

- [ ] **DOC-1** Architecture diagram or narrative current.
- [ ] **DOC-2** ADRs or decision log for major choices.
- [ ] **DOC-3** README / onboarding steps accurate (install, test, deploy).

---

### ML — ML / AI / model governance *(if applicable)*

**Goal:** Safe, traceable model lifecycle for ML-heavy systems.

- [ ] **ML-1** Training / serving separation; no accidental training deps in minimal runtime images.
- [ ] **ML-2** Model versioning, promotion gates, rollback.
- [ ] **ML-3** Data leakage / evaluation hygiene for offline metrics.
- [ ] **ML-4** Fairness / safety review if product requires it.

---

### RLS — Release, change & incident process

**Goal:** Controlled change; learn from incidents.

- [ ] **RLS-1** Release process documented (canary, feature flags, migrations order).
- [ ] **RLS-2** Post-incident reviews filed for Sev-1/2 or equivalent.
- [ ] **RLS-3** Dependency and OS patch cadence defined.

---

## 4. Findings log (append each full audit)

| ID | Category | Severity | Summary | Tracking issue |
|----|----------|----------|---------|----------------|
| | | | | |

---

## 5. Sign-off

| Role | Name | Date |
|------|------|------|
| Engineering | | |
| Security / risk *(if applicable)* | | |
| Operations *(if applicable)* | | |

---

## 6. Repo-specific quick reference *(optional — customize per repository)*

Copy this block into your repo and fill it so auditors know **where** commands and policies live. Remove if you keep this file 100% generic.

| Topic | This repository |
|--------|-----------------|
| Primary languages | e.g. Python 3.12 |
| CI workflow | e.g. `.github/workflows/ci.yml` |
| Lint / test commands | e.g. `ruff check .`, `pytest` |
| Container build | e.g. `Dockerfile`, `docker compose -f infra/...` |
| Security scans in CI | e.g. Hadolint, Trivy (fs), *add pip-audit if adopted* |
| Integration tests | e.g. optional job, env `NM_INTEGRATION_SERVICES` |
| Runbooks | e.g. `docs/RUNBOOKS.MD` |
| Prior focused audit | e.g. `docs/AUDIT_CODE_REVIEW.MD` |

---

## 7. Reuse in other repositories

- **Copy** this file to `docs/FULL_AUDIT.md` (or `SECURITY/FULL_AUDIT.md`).
- **Fill §1** and **§6** first; **§3** is the universal checklist.
- **Trim** optional sections (e.g. ML) if permanently N/A.
- **Automate** what you can; keep this file as the **human** record of scope, dates, and evidence.

---

*Template version: 1.0 — structured for any application type; categories align with common SDLC, security, and SRE practice.*
