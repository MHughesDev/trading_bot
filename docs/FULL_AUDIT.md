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

**Using §3 prompts:** Each category in **§3** includes (1) an **Audit prompt** block—copy into an AI agent or paste into a ticket for a human reviewer; (2) a **Procedure** for hands-on steps; (3) **Pass criteria** checkboxes; (4) **Evidence** to attach. Run categories **in order** (G first) unless you are doing a **partial** audit—then still complete **G** minimally so scope stays explicit.

---

## 3. Categories — detailed audit prompts & procedures

Each subsection is usable **standalone**: copy the **Audit prompt** into an AI agent or human runbook, follow **Procedure**, and attach **Evidence** to §1 and §4. **Tools** are examples—map to your stack (`eslint`, `golangci-lint`, `cargo audit`, `npm audit`, `terraform validate`, etc.).

---

### G — Governance & scope

**Goal:** Establish *who* is accountable, *what* is in scope, *what* risks are accepted, and *how* this audit ties to business obligations—before deeper technical work.

#### Audit prompt (copy verbatim for an agent)

> You are performing a **Governance & scope** audit for a software system. Your job is **not** to find code bugs yet; it is to **frame** the audit: boundaries, stakeholders, risk tolerance, and compliance drivers.  
> **Inputs you must obtain:** repository name, product name, list of environments (dev/stage/prod), list of data types processed (PII, financial, health, none), and any existing risk register or architecture doc.  
> **Tasks:**  
> (1) Produce a **system boundary diagram** in text: components, trust zones, external systems, data flows. Mark what is **in scope** vs **out of scope** for *this* audit cycle.  
> (2) List **stakeholders** (engineering owner, security contact, operations, product/legal if PII).  
> (3) Identify **regulatory / contractual** obligations (GDPR, PCI, HIPAA, SOC2, customer DPAs, etc.) or state **explicitly N/A** with justification.  
> (4) Define **risk appetite** in one paragraph: what “acceptable” downtime, data loss window, and security incident response looks like for this product.  
> (5) Record **audit exclusions** (e.g. `legacy/`, vendor forks, third-party SaaS internals) and the **reason** each exclusion is safe.  
> **Outputs:** A short governance summary (max 2 pages), a table of in/out scope, and a list of **follow-on audit categories** that must run given this scope (e.g. if PII → privacy mandatory).  
> **Do not** assume—flag **unknown** items as gaps.

#### Procedure (human or hybrid)

1. **Kickoff:** Confirm audit lead, read-only vs change-allowed, and communication channel for findings.
2. **Inventory docs:** Open README, architecture docs, ADRs, security pages, vendor SLAs.
3. **Boundary:** Walk from user entry points (UI, API, batch, mobile) to persistence and third parties; draw or narrate trust boundaries.
4. **Compliance:** If the product touches regulated data, note which **controls** must appear later (encryption, access logs, subprocessors).
5. **Sign-off prep:** Ensure §1 “Scope notes” matches this section.

#### Pass criteria

- [ ] **G-1** Stakeholders and system boundary documented (in/out of scope explicit).
- [ ] **G-2** Threat model **or** risk register **or** equivalent risk narrative exists **or** was created in this pass.
- [ ] **G-3** Regulatory / contractual obligations identified **or** marked N/A with rationale.

#### Evidence to attach

- Link to diagram or doc; meeting notes; risk register export; redacted DPA checklist.

---

### SEC-REPO — Secrets & repository hygiene

**Goal:** Ensure secrets never live in source, images, or tickets in recoverable form; history and clone workflows are safe.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **secrets and repository hygiene**. Assume attackers have **read access to the repo** and **historical git history**.  
> **Tasks:**  
> (1) Run or describe **secret scanning** across the full tree and history (e.g. `gitleaks`, `trufflehog`, GitHub secret scanning). List every finding with **file, commit, and remediation** (rotate secret, purge history, or false positive justification).  
> (2) Review **`.gitignore`**, **`.dockerignore`**, and CI for leaked paths (`.env`, `*.pem`, `id_rsa`, cloud keys).  
> (3) Search for **high-entropy strings**, **API key patterns**, **private URLs** in code and docs.  
> (4) Verify **pre-commit** or **CI** blocks new secrets (grep hooks, detect-secrets baseline).  
> (5) Check **fork and clone** guidance: no instructions to copy real keys into docs.  
> **Outputs:** Pass/fail, list of findings by severity, rotation tickets for any live credential exposure.  
> **Constraints:** Do not print live secrets in outputs—redact or hash.

#### Procedure

1. Install/run scanner appropriate to org policy; include **full git history** if allowed.
2. Manually spot-check `docker compose`, `kubernetes` YAML, and `README` for example keys—ensure they are **obviously dummy**.
3. Confirm **secret backends** (Vault, cloud SM, env-only) are the **only** runtime path documented.

#### Pass criteria

- [ ] **SEC-REPO-1** Secret scan clean **or** all findings fixed/accepted with ticket IDs.
- [ ] **SEC-REPO-2** Ignore files and repo hygiene appropriate; no credential files tracked.
- [ ] **SEC-REPO-3** Automated prevention (pre-commit/CI) in place **or** gap documented with owner.

#### Evidence

- Scanner report (redacted), CI job name, baseline file if using detect-secrets.

---

### SUP — Supply chain & dependencies

**Goal:** Dependencies are **known**, **vetted**, **reproducible**, and **monitored** for vulnerabilities and license risk.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **supply chain and dependency management**.  
> **Inputs:** Lockfiles (`package-lock.json`, `poetry.lock`, `Cargo.lock`, `go.sum`, etc.), manifest files (`package.json`, `pyproject.toml`, `requirements.txt`), CI dependency jobs, Dependabot config.  
> **Tasks:**  
> (1) Confirm **reproducible installs**: same CI and prod resolution path; document if lockfile is optional and why.  
> (2) Run **CVE / advisory** scans (`npm audit`, `pip-audit`, `cargo audit`, OSV-Scanner, Snyk) on the audited commit; categorize by **reachability** and **severity**.  
> (3) Review **direct vs transitive** critical dependencies (crypto, HTTP, auth, parsing).  
> (4) Evaluate **SBOM** maturity: generate CycloneDX/SPDX for a release artifact **or** justify deferral.  
> (5) **License** compliance: flag GPL/AGPL in linked code if distribution model forbids.  
> (6) Define **update policy**: how often patch vs minor, who approves breaking upgrades.  
> **Outputs:** Table of vulnerabilities with remediation plan; dependency top-N list; policy paragraph.

#### Procedure

1. Run primary ecosystem scanner locally and compare to CI (if any).
2. Check **pinned versions** on security-sensitive libs (TLS, JWT, YAML, SQL drivers).
3. Review **git submodules** and **vendored** code for the same standards.

#### Pass criteria

- [ ] **SUP-1** Lockfile or reproducibility strategy documented.
- [ ] **SUP-2** Advisory scan run; critical issues **tracked** or **mitigated**.
- [ ] **SUP-3** SBOM path defined for releases **or** N/A with reason.
- [ ] **SUP-4** Pinning/update policy recorded for critical paths.

#### Evidence

- `pip-audit`/`npm audit` JSON output, Dependabot dashboard link, SBOM file path.

---

### STATIC — Static analysis & style

**Goal:** Enforce consistent style and catch **classes** of bugs statically (style, types, security patterns).

#### Audit prompt (copy verbatim for an agent)

> You are auditing **static analysis and code quality gates**.  
> **Tasks:**  
> (1) Identify **linters and formatters** in repo config (ESLint, Ruff, Prettier, clang-format, golangci-lint). Verify they run in **CI on every PR** to default branch.  
> (2) **Type safety:** If language supports it, run `mypy`, `pyright`, `tsc --strict`, or equivalents; document strictness level and exceptions file.  
> (3) **Security static rules:** Run Semgrep, CodeQL, or Bandit (Python) with a **reasonable rule set**; triage findings.  
> (4) **Dead code / complexity:** Note thresholds (cyclomatic complexity, TODO debt) if used.  
> (5) Confirm **editor/CI parity** so local dev matches CI.  
> **Outputs:** List of tools, versions, CI job names, failing rules, and backlog for false positives.

#### Procedure

1. Run the same commands CI runs; capture exit codes.
2. Review **suppressed rules** (`# noqa`, `eslint-disable`) in hot paths for abuse.
3. For monorepos, verify **each package** is covered.

#### Pass criteria

- [ ] **STATIC-1** Lint enforced in CI; violations documented if waived.
- [ ] **STATIC-2** Typecheck policy applied **or** N/A (e.g. pure JS without TS) with rationale.
- [ ] **STATIC-3** Security-oriented static analysis run **or** gap tracked.

#### Evidence

- CI log excerpt, config files (`ruff.toml`, `eslint.config.js`, `mypy.ini`).

---

### CORR — Correctness, safety & concurrency

**Goal:** Business logic and security invariants hold; failures are **detectable**; concurrency is **correct**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **correctness, safety, and concurrency** (manual + targeted code review).  
> **Inputs:** List of **critical paths** (auth, money movement, data deletion, admin APIs, trading, etc.).  
> **Tasks:**  
> (1) For each critical path, trace **happy path** and **main failure modes**; verify invariants (e.g. “balance never negative”, “idempotent webhook”).  
> (2) Review **error handling**: broad `except`, silent `pass`, swallowed errors in security-sensitive code—flag each with severity.  
> (3) **Concurrency:** identify shared mutable state, locks, async tasks, thread pools; check for races, deadlocks, re-entrancy.  
> (4) **Time and units:** timezone handling (UTC vs local), daylight saving, monetary rounding, integer overflow.  
> (5) **Idempotency:** retries must not double-charge or double-apply side effects.  
> **Outputs:** Per-path notes, list of correctness issues, recommended tests.  
> **Method:** Prefer reading code + tests together; run tests focused on critical modules.

#### Procedure

1. Build a **critical path list** with owners.
2. Use `rg`/IDE search for `except Exception`, `pass`, `TODO`, `FIXME` in those modules.
3. For async code, verify **cancellation** and **timeout** behavior.
4. Cross-check **unit tests** assert negative cases, not only golden paths.

#### Pass criteria

- [ ] **CORR-1** Critical paths enumerated and reviewed.
- [ ] **CORR-2** Error-handling risks documented; no silent failures in security-sensitive code without acceptance.
- [ ] **CORR-3** Concurrency/async reviewed where applicable.
- [ ] **CORR-4** Domain numeric/time issues checked.

#### Evidence

- Review notes, PR links for fixes, test names added.

---

### TEST — Testing & quality gates

**Goal:** Automated tests **match risk**: critical behavior is covered; CI is **trusted**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **testing strategy and CI quality gates**.  
> **Tasks:**  
> (1) Map **test pyramid**: unit vs integration vs E2E; identify gaps for critical domains.  
> (2) Run **full CI suite** locally or verify latest green run on audited commit for default branch.  
> (3) Review **test flakiness** history (quarantined tests, retries); document policy.  
> (4) **Coverage:** If coverage is tracked, note thresholds; **more importantly**, verify **critical modules** have meaningful tests, not only % lines.  
> (5) **Integration tests:** databases, queues, HTTP mocks—are they representative?  
> (6) **Contract tests** for internal APIs between services (if microservices).  
> **Outputs:** Gap analysis, list of untested critical behaviors, recommendations.

#### Procedure

1. `pytest` / `npm test` / `go test ./...` as per repo.
2. Open CI workflow YAML; confirm **required checks** for merge.
3. Spot-check **longest** or **skipped** tests for debt.

#### Pass criteria

- [ ] **TEST-1** Unit tests pass on audited commit (or documented exception).
- [ ] **TEST-2** Integration/contract coverage **adequate** or gaps documented.
- [ ] **TEST-3** Critical-path test policy written (even if “no global coverage %”).
- [ ] **TEST-4** E2E/smoke for release path **or** N/A with reason.

#### Evidence

- CI run URL, test command log, coverage report (if any).

---

### APPSEC — Application & API security

**Goal:** Apply **OWASP-style** thinking to the application layer: authn, authz, input handling, HTTP surface, abuse.

#### Audit prompt (copy verbatim for an agent)

> You are performing an **application and API security** audit. Use OWASP ASVS/API Security Top 10 as a **mental checklist**, adapted to this stack.  
> **Tasks:**  
> (1) **Authentication:** How are users/services proven? Passwords, API keys, OAuth, mTLS—document flows and storage of secrets client-side.  
> (2) **Authorization:** Enforce **every** sensitive operation server-side; look for IDOR (object references), missing role checks, admin routes.  
> (3) **Sessions/tokens:** JWT validation (alg, exp, audience), cookie flags (`HttpOnly`, `Secure`, `SameSite`), refresh token rotation.  
> (4) **Input:** Validation, max sizes, content types; **SQL** parameterized; **command** injection; **SSRF** on URL-fetching features.  
> (5) **Output:** XSS for HTML/JS contexts; CSP where applicable.  
> (6) **Transport:** HTTPS everywhere for prod; HSTS.  
> (7) **CORS:** restrict origins in prod; no `*` with credentials.  
> (8) **Rate limiting / abuse:** brute force, scraping, DoS—app-level or edge.  
> (9) **Deserialization:** no unsafe `pickle`, `yaml.load`, XML XXE.  
> **Outputs:** Threat-oriented findings with severity, reproduction steps, fix guidance.

#### Procedure

1. Read API framework **middleware order** (auth before routes).
2. Enumerate **all** routes (OpenAPI, router files); sample high-risk ones.
3. For **Streamlit/SPA**, check **CSRF** and **clickjacking** if using cookies.

#### Pass criteria

- [ ] **APPSEC-1** Authn/z model documented; server-side checks on protected actions.
- [ ] **APPSEC-2** Session/token handling reviewed.
- [ ] **APPSEC-3** Injection classes mitigated (SQL, command, SSRF).
- [ ] **APPSEC-4** Browser security headers / CORS appropriate for deployment model.
- [ ] **APPSEC-5** Abuse controls **or** compensating controls documented.
- [ ] **APPSEC-6** Unsafe deserialization **absent** or isolated.

#### Evidence

- Burp/ZAP notes (if used), route list, redacted HTTP headers sample.

---

### INFRA — Infrastructure & secrets management

**Goal:** Runtime and deployment infrastructure **minimizes blast radius** and **protects secrets**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **infrastructure and secrets management** (cloud, Kubernetes, VMs, serverless).  
> **Tasks:**  
> (1) **Secrets:** Where do prod secrets live? Confirm **not** in git, not in `docker history`, not in plain env in CI logs.  
> (2) **IAM:** Least privilege for deploy roles, CI OIDC vs long-lived keys, runtime service accounts.  
> (3) **Network:** VPC/subnets, DB private-only, SSH/bastion patterns, **no public admin** ports.  
> (4) **TLS:** Certificates from ACM/Let’s Encrypt; auto-renewal; **min TLS version**.  
> (5) **IaC:** Terraform/CloudFormation/Pulumi **state** secured; `plan` in CI.  
> (6) **Backup access:** Who can snapshot DBs? MFA?  
> **Outputs:** Infra diagram, list of misconfigurations, severity, remediation.

#### Procedure

1. Read Terraform/k8s manifests or cloud console read-only review.
2. Verify **encryption at rest** for disks/DB where required by policy.
3. Check **public endpoints** list against expectation.

#### Pass criteria

- [ ] **INFRA-1** Secrets management **approved** path only.
- [ ] **INFRA-2** IAM least privilege **reviewed**.
- [ ] **INFRA-3** Network segmentation **reviewed**.
- [ ] **INFRA-4** TLS and cert lifecycle **addressed**.

#### Evidence

- Redacted architecture diagram, IAM role list, Terraform plan summary.

---

### CONT — Container & image security

**Goal:** Container images are **minimal**, **non-root**, **scanned**, and **rebuilt** on patches.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **container and image security**.  
> **Tasks:**  
> (1) Review **Dockerfile** (or equivalent): `USER`, multi-stage builds, no unnecessary packages, `COPY` scope minimal.  
> (2) Run **Hadolint** (or policy) on Dockerfile; fix or justify ignores.  
> (3) Build image from audited commit; run **Trivy/Grype** image scan; triage CRITICAL/HIGH.  
> (4) Check **base image** tag strategy (`:latest` vs pinned digest).  
> (5) Verify **no secrets** in layers (`docker history`, build args).  
> (6) **Runtime:** read-only root, `cap_drop`, seccomp/AppArmor if used.  
> **Outputs:** Scan report, policy for failing builds, patch cadence.

#### Procedure

1. `docker build` with CI-equivalent flags.
2. Compare CI **exit-code policy** for scanners (informational vs blocking).

#### Pass criteria

- [ ] **CONT-1** Dockerfile lint + hardening basics **reviewed**.
- [ ] **CONT-2** Image CVE scan performed; policy **defined**.
- [ ] **CONT-3** Base image update cadence **documented**.

#### Evidence

- Hadolint output, Trivy JSON, image digest.

---

### DATA — Data durability, backup & recovery

**Goal:** Data survives **component failures** and **operator mistakes**; recovery is **proven**, not theoretical.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **data durability, backup, and recovery**.  
> **Inputs:** Database types (SQL, NoSQL, object store, queues), replication, WAL, object storage.  
> **Tasks:**  
> (1) For each **durable store**, document **RPO/RTO** targets (even informal).  
> (2) **Backup:** frequency, retention, encryption, **off-site** copy, who can delete backups.  
> (3) **Restore:** last successful **restore test** date; step-by-step runbook; time to restore.  
> (4) **Failure modes:** single-node crash, AZ loss, corruption **detection** (checksums).  
> (5) **Migrations:** forward-only vs reversible; **order** with app deploys.  
> (6) **Queues:** message durability vs at-most-once semantics—document tradeoffs.  
> **Outputs:** Per-store table, gaps, mandatory drills.

#### Procedure

1. Read runbooks for each database/object store.
2. Ask **when** last restore was performed; if never, file **drill ticket**.
3. Verify **backup monitoring** (failed backup alerts).

#### Pass criteria

- [ ] **DATA-1** Backup strategy **documented** per critical store.
- [ ] **DATA-2** Restore **evidence** or scheduled drill.
- [ ] **DATA-3** Durability under failure **documented** (fsync, replication).
- [ ] **DATA-4** Migration/rollback **plan** exists.

#### Evidence

- Backup job logs, restore drill notes, migration playbook.

---

### PERF — Performance & capacity

**Goal:** System meets **latency**, **throughput**, and **resource** expectations under realistic load; performance **debt** is visible.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **performance and capacity**.  
> **Tasks:**  
> (1) **SLOs:** List target latency (p50/p95/p99), error rate, throughput; if missing, propose **draft** SLOs from business needs.  
> (2) **Profiling:** CPU profile for representative workload (API, batch job); identify top **5** hot functions.  
> (3) **Memory:** Heap snapshots or RSS monitoring over **hours/days**; leak **suspects** (unbounded caches, global lists).  
> (4) **I/O:** DB query plans, N+1 queries, slow logs; external API latency.  
> (5) **Caching:** Redis/CDN—correct invalidation, TTLs, stampedes.  
> (6) **Load testing:** `k6`, Locust, JMeter—**sustained** load on critical endpoints; compare to SLO.  
> (7) **Cost-performance:** largest infra line items vs **cheap** wins (batch size, connection pooling).  
> **Outputs:** Profile summaries, bottleneck list, prioritized backlog, before/after metrics if optimizations exist.

#### Procedure

1. Reproduce **worst-known** slow path from tickets/metrics.
2. Run **long soak** (memory) if service is long-running.
3. Align **test data** size with production order of magnitude (sanitized).

#### Pass criteria

- [ ] **PERF-1** Profiling done **or** N/A (batch-only trivial) with rationale.
- [ ] **PERF-2** Memory leak risk **assessed**.
- [ ] **PERF-3** SLOs **documented** and **measured** or gap logged.
- [ ] **PERF-4** Improvement backlog **from data**, not guesses.
- [ ] **PERF-5** Load/soak **representative** test **or** scheduled.

#### Evidence

- Flame graphs, k6 summary, Grafana dashboard snapshots.

---

### REL — Reliability & resilience

**Goal:** System **degrades** predictably and **recovers** without human heroics where possible.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **reliability and resilience**.  
> **Tasks:**  
> (1) **Outbound calls:** timeouts set; retries with **jitter**; **circuit breakers** where appropriate; **idempotency keys** for retries.  
> (2) **Graceful shutdown:** SIGTERM handling, drain period, in-flight request completion, queue consumers.  
> (3) **Dependencies:** behavior when DB, cache, or vendor is **down**—fail closed vs open, cached data staleness.  
> (4) **HA:** single points of failure listed; **multi-AZ/region** if claimed.  
> (5) **Chaos / game:** optional fault injection (kill pod, network partition) **or** documented **failure mode** table.  
> **Outputs:** Failure mode matrix (component × symptom × mitigation), list of code/config gaps.

#### Procedure

1. Read shutdown docs; test in staging if possible.
2. Trace **retry** logic for payment/order paths—no duplicate side effects.

#### Pass criteria

- [ ] **REL-1** Timeouts/retries/idempotency **reviewed** for critical IO.
- [ ] **REL-2** Graceful shutdown **documented and verified** or gap filed.
- [ ] **REL-3** Failure modes **documented**; chaos **optional** but ideal.
- [ ] **REL-4** HA assumptions **explicit**.

#### Evidence

- Failure mode doc, test logs, chaos experiment report.

---

### OBS — Observability & alerting

**Goal:** Operators can **answer** “what broke?” and “who is affected?” in minutes; alerts are **actionable**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **observability and alerting**.  
> **Tasks:**  
> (1) **Logging:** structured logs (JSON); **correlation IDs** across services; log levels; **no secrets** in logs.  
> (2) **Metrics:** RED (rate, errors, duration) for services; **USE** for resources; **business** metrics (orders, revenue).  
> (3) **Tracing:** OpenTelemetry spans for critical paths if distributed.  
> (4) **Dashboards:** Grafana/Datadog—do they cover **golden signals**?  
> (5) **Alerts:** On **symptom** (error rate spike, latency) not only **cause** (pod restart); **no paging** for non-prod unless intended.  
> (6) **Noise:** review last month’s alert volume; **top noisy** alerts fixed or tuned.  
> **Outputs:** Dashboard links, alert inventory, gaps (e.g. missing DB metrics).

#### Procedure

1. Walk through **one** simulated incident using only observability tools.
2. Verify **PII** redaction in logs and traces.

#### Pass criteria

- [ ] **OBS-1** Logging **structured**; PII policy **respected**.
- [ ] **OBS-2** Metrics **cover** critical paths.
- [ ] **OBS-3** Tracing **if** multi-service **or** N/A with reason.
- [ ] **OBS-4** Alerts **actionable**; runbooks linked; noise reviewed.

#### Evidence

- Dashboard URLs, alert rule export, example log line with trace ID.

---

### OPS — Operational readiness & disaster recovery

**Goal:** **Runbooks**, **DR**, and **on-call** make outages **short** and **boring**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **operational readiness and disaster recovery**.  
> **Tasks:**  
> (1) **Runbooks:** deploy, rollback, DB failover, disk full, certificate expiry, **vendor outage**.  
> (2) **DR:** region loss scenario; **backup region** or cold standby; **RTO/RPO** numbers or explicit “not targeted” with sign-off.  
> (3) **On-call:** rotation, escalation, **severity** definitions, **communication** template (status page).  
> (4) **Capacity:** what happens at traffic **2×**; **scale** plan (manual vs auto).  
> **Outputs:** Runbook coverage matrix, DR gaps, incident comms checklist.

#### Procedure

1. Read existing runbooks; **time** a rollback drill in staging.
2. Confirm **contact tree** is current.

#### Pass criteria

- [ ] **OPS-1** Runbooks **exist** for top failure modes.
- [ ] **OPS-2** DR **plan** + RTO/RPO **or** accepted risk.
- [ ] **OPS-3** On-call **defined** for prod.

#### Evidence

- Runbook links, DR table, last game-day or drill date.

---

### PRIV — Privacy & compliance

**Goal:** Personal data is **lawfully** processed, **limited**, and **deletable** where required.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **privacy and compliance** (not legal advice—flag for counsel).  
> **Tasks:**  
> (1) **Data inventory:** categories of PII, **where** stored, **retention**, **who** can access.  
> (2) **Legal basis** (GDPR) or equivalent; **consent** flows if applicable.  
> (3) **Subprocessors** and DPAs listed for customer contracts.  
> (4) **Rights:** access, deletion, portability—**process** and **SLA**.  
> (5) **Cross-border** transfers (SCCs, adequacy).  
> (6) **Minimization:** only collect what is needed; **pseudonymization** where possible.  
> **Outputs:** Data map, gaps, legal follow-ups.

#### Procedure

1. If **no PII**, document **N/A** and **what** would trigger privacy review if added.
2. Cross-check **analytics** and **logs** for accidental PII.

#### Pass criteria

- [ ] **PRIV-1** Data inventory **or** N/A.
- [ ] **PRIV-2** Legal/subprocessor **documentation** or N/A.
- [ ] **PRIV-3** User rights **process** if PII **or** N/A.

#### Evidence

- Data map diagram, DPA list (internal), policy links.

---

### DOC — Documentation & architecture

**Goal:** New engineers and auditors **ramp quickly**; docs match **reality**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **documentation and architecture**.  
> **Tasks:**  
> (1) **Architecture:** C4 or equivalent—context, containers, key components; **update** date.  
> (2) **ADRs:** major decisions recorded with **status** (accepted/superseded).  
> (3) **README:** clone, install, test, run, deploy—**execute** steps on clean machine **or** list friction.  
> (4) **API docs:** OpenAPI/Swagger current; **examples** work.  
> (5) **Onboarding:** “day one” path < X hours achievable?  
> **Outputs:** Doc freshness table, **broken links**, **drift** between code and docs.

#### Procedure

1. Click through every **major** doc link from README.
2. Compare **deploy** section to actual CI/CD pipeline names.

#### Pass criteria

- [ ] **DOC-1** Architecture **current** or **refresh** ticket filed.
- [ ] **DOC-2** ADRs **or** equivalent decision log.
- [ ] **DOC-3** README/onboarding **verified** or issues listed.

#### Evidence

- Doc review notes, list of fixed links.

---

### ML — ML / AI / model governance *(if applicable)*

**Goal:** Models are **traceable**, **safe to promote**, and **separated** from minimal runtime.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **ML/AI governance** (skip if no ML—state N/A).  
> **Tasks:**  
> (1) **Separation:** training code/deps **not** required in production image unless justified.  
> (2) **Versioning:** model artifacts versioned (MLflow, DVC, registry); **immutable** deploy.  
> (3) **Promotion:** gates (metrics thresholds, peer review); **rollback** path.  
> (4) **Evaluation:** train/val/test split; **no leakage** from future data; **offline** vs **online** metrics.  
> (5) **Safety:** prompt injection (LLMs), toxicity, PII in training data.  
> (6) **Fairness:** if regulated or product requires—bias testing **or** documented waiver.  
> **Outputs:** Model card summary, promotion checklist, gaps.

#### Procedure

1. Trace **one** model from training artifact to **production** load path.
2. Compare **Dockerfile** stages for training vs inference.

#### Pass criteria

- [ ] **ML-1** Train/serve separation **reviewed**.
- [ ] **ML-2** Versioning + promotion **documented**.
- [ ] **ML-3** Evaluation hygiene **checked**.
- [ ] **ML-4** Fairness/safety **per product** requirements **or** N/A.

#### Evidence

- Model registry screenshot, promotion runbook, test reports.

---

### RLS — Release, change & incident process

**Goal:** Changes are **controlled**; incidents produce **learning**.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **release, change, and incident management**.  
> **Tasks:**  
> (1) **Release:** frequency, **canary**, **feature flags**, **migration order** vs app rollout.  
> (2) **Change control:** who approves prod deploys; **emergency** path.  
> (3) **Incidents:** Sev definitions; **postmortems** for Sev-1/2; **action items** tracked.  
> (4) **Patching:** OS, language runtime, dependency **cadence**; **SLA** for critical CVEs.  
> (5) **Freeze:** holiday/code freeze policy if any.  
> **Outputs:** Process summary, links to templates, gaps.

#### Procedure

1. Review last **3** incidents or postmortems (if any).
2. Read **CONTRIBUTING** / release checklist.

#### Pass criteria

- [ ] **RLS-1** Release process **documented**.
- [ ] **RLS-2** Post-incident practice **verified** or **improvement** ticket.
- [ ] **RLS-3** Patch cadence **defined**.

#### Evidence

- Postmortem links, release checklist, patch calendar.

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

*Template version: 2.0 — includes per-category **audit prompts**, procedures, pass criteria, and evidence; portable across application types.*
