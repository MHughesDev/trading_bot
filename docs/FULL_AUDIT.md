# Full-scope audit — playbook & checklist

**Purpose:** A **single, repeatable**, **repository-neutral** document for running a **complete** audit pass across security, quality, performance, operations, and data. Copy this file **verbatim** into any repo (`docs/FULL_AUDIT.md`, `SECURITY/AUDIT_PLAYBOOK.md`, or `.github/AUDIT_FULL.md`)—**do not** fork per-language variants; instead fill **§0** and map tools in **§0.4**.

**Audience:** Human auditors, **AI agents**, and hybrid workflows. The **§0.3** master prompt + per-category **Audit prompt** blocks (**§4**) are the **agentic** interface.

**How this differs from narrow audits:** A “code review audit” or “CVE scan” is one *slice*. A **full audit** walks **every applicable category** in one coordinated pass (or a rotation), updates **last-run dates**, links **evidence**, and ends with a **standalone audit report** (**[§8](#8-audit-report-deliverable-mandatory-end-state)**) — optionally polished with **`draft-audit-report`**, then promoted to **`QUEUE_STACK.csv`** with **`audit-report-to-queue`**.

**Mark sections N/A** when they do not apply to your stack, but record *why* (see **§0.5**). **Never** treat N/A as “skip documentation”—state the reason in the audit record.

---

## 0. Repository profile & agent bootstrap *(fill once per repo; copy-paste friendly)*

Complete **§0.2** before the first audit. Prepend **§0.3** to every category prompt in **§4** when using an AI agent.

### 0.1 What “generalized” means

| Principle | Rule |
|-----------|------|
| **No default stack** | Do not assume web, containers, a particular cloud, or a language. Discover from the repo. |
| **Examples ≠ requirements** | Tool names in this file are **illustrative**. Map them using **§0.4**. |
| **N/A is explicit** | If a category does not apply, output **N/A — [reason]** and still update the **§2** row for that category with outcome **N/A**. |
| **Evidence, not vibes** | Findings must cite **paths**, **CI jobs**, **commands**, or **tickets** (redacted where needed). |
| **Shape-agnostic** | Applies to: libraries, CLIs, mobile apps, games, firmware, data pipelines, ML systems, static sites, internal tools. |

### 0.2 Repository profile *(paste into the agent session or a ticket)*

| Field | Your value |
|-------|------------|
| **Repository URL / name** | |
| **Primary languages / runtimes** | |
| **What this repo produces** | *(library, service binary, OCI image, site bundle, mobile app, mixed)* |
| **Entry points** | *(HTTP, gRPC, CLI, worker, cron, GUI, plugin API, none)* |
| **Data stores owned here** | *(none, files, SQLite, RDBMS, object store, queue, …)* |
| **How configuration & secrets are provided** | *(env, files, vault, platform SM, compile-time, …)* |
| **CI / CD** | *(vendor, key workflow paths, or “none”)* |
| **Regulated / sensitive data** | *(none, PII, financial, health, gov, unknown)* |
| **Exposure** | *(public internet, internal only, offline, mixed)* |
| **Paths / subtrees excluded from this audit** | |
| **Commit SHA / tag under audit** | |

### 0.3 Master system prompt *(prepend verbatim before each §4 “Audit prompt” block)*

```text
You are an expert software and systems auditor working from a generalized playbook.

Repository profile (fill from the project):
[PASTE §0.2 TABLE OR A SHORT SUMMARY]

Rules:
1. Infer the project’s actual stack from the repository (file tree, CI configs, docs). Do not assume tools not present.
2. If a playbook category does not apply (e.g. no containers, no ML), state N/A with a clear reason; do not force checks.
3. Never print secrets, tokens, keys, or unredacted PII. Use placeholders and describe where they live (e.g. “environment variable X”).
4. Tie claims to evidence: file paths, workflow names, command names, dashboard titles (non-sensitive URLs only).
5. Map generic playbook terms to this project (e.g. “artifact” → whatever this build produces).
6. Severity for findings: Critical / High / Medium / Low / Info.
7. Finish with the Outputs and Evidence structure required by that category’s prompt.
```

### 0.4 Tool & artifact substitution guide *(examples only—pick what exists)*

| Playbook term | Example implementations *(non-exhaustive)* |
|---------------|---------------------------------------------|
| Dependency manifests & locks | `package.json` + lockfile, `go.mod`, `Cargo.toml` + `Cargo.lock`, `pom.xml` / Gradle, `Gemfile.lock`, `pyproject.toml`, `composer.lock`, `uv.lock`, `flake.nix` |
| Lint / format | ESLint, Biome, Ruff, Black, Prettier, golangci-lint, `clang-format`, RuboCop, `cargo fmt`, Spotless |
| Type / static checks | `tsc`, `mypy`, `pyright`, Kotlin compiler, Swift strict, `go vet`, Sonar |
| Tests | `pytest`, `npm test`, `cargo test`, `go test`, `mvn test`, `dotnet test`, XCTest |
| Security SAST | Semgrep, CodeQL, Bandit, Brakeman, SonarQube |
| Secret detection | gitleaks, trufflehog, git-secrets, platform-native scanning |
| Dependency vulnerabilities | `npm audit`, `pip-audit`, `cargo audit`, OSV-Scanner, Dependabot, Renovate, Snyk |
| Container / OCI | Dockerfile, Containerfile, Buildpacks, Ko, Nix-generated images |
| Image / filesystem scanning | Trivy, Grype, ECR/Azure/GCR scanners |
| Policy / config tests | conftest, OPA, Sentinel |
| Infra as code | Terraform, Pulumi, CloudFormation, Bicep, Helm, Ansible |
| API contracts | OpenAPI, AsyncAPI, GraphQL schema, Protobuf, WSDL |
| Observability | OpenTelemetry, Prometheus, Grafana, Datadog, New Relic, CloudWatch, Splunk |
| Load / perf tools | k6, Locust, JMeter, Gatling, wrk, browser perf tools |

### 0.5 When a category is usually N/A *(document in §2)*

| ID | Typical N/A when… |
|----|-------------------|
| **CONT** | No OCI/container build; pure firmware or desktop app with no container story in this repo |
| **INFRA** | Repo is only a library or spec with no deployment or cloud config here |
| **ML** | No models, training, prompts, or inference code in scope |
| **DATA** | No durable state owned by this codebase |
| **APPSEC** | No remotely reachable surface and no security-sensitive library API *(still review public API design if library)* |
| **OBS** | Impossible only for throwaway prototypes—otherwise define **minimal** observability (even logs to stderr) |

### 0.6 Standard output shape *(append to every category result)*

Every category audit (human or agent) should end with:

1. **Verdict:** Pass / Pass with findings / Blocked / N/A (+ reason if N/A).
2. **Findings:** table of ID, severity, location, summary, remediation hint.
3. **Evidence:** list of artifacts produced (report paths, CI run IDs, ticket links).
4. **Residual gaps:** missing access, tooling, or time.

---

## 2. Audit record (update every full pass)

| Field | Value |
|--------|--------|
| **Repository / product** | **trading_bot** |
| **Version / commit audited** | **`fc39ea984587d4a513aa532326945e80d082530d`** |
| **Full audit — last completed (UTC)** | **2026-04-13** |
| **Full audit — lead / role** | Automated full audit (agent) |
| **Scope notes** | Repo + CI + docs; exclude production runtime secrets |
| **Overall outcome** | ☑ Pass with findings ☐ Pass ☐ Blocked |

**Per-category last run (optional but recommended):** Update when you execute that category as part of *any* audit (full or partial). During a **full audit**, set each category’s date to the full-audit date if you completed it.

| Category | ID | Last run (UTC) | Outcome (P / PWF / B) | Evidence link / path |
|----------|----|----------------|------------------------|----------------------|
| Governance & scope | G | 2026-04-13 | P | [`reports/AUDIT_REPORT_2026-04-13_full.md`](reports/AUDIT_REPORT_2026-04-13_full.md) |
| Secrets & repository hygiene | SEC-REPO | 2026-04-13 | PWF | same |
| Supply chain & dependencies | SUP | 2026-04-13 | PWF | same |
| Static analysis & style | STATIC | 2026-04-13 | PWF | same |
| Correctness, safety & concurrency | CORR | 2026-04-13 | PWF | same |
| Testing & quality gates | TEST | 2026-04-13 | PWF | same |
| Application & API security | APPSEC | 2026-04-13 | P | same |
| Infrastructure & secrets management | INFRA | 2026-04-13 | P | same |
| Container & image security | CONT | 2026-04-13 | P | same |
| Data durability, backup & recovery | DATA | 2026-04-13 | P | same |
| Performance & capacity | PERF | 2026-04-13 | N/A | same |
| Reliability & resilience | REL | 2026-04-13 | P | same |
| Observability & alerting | OBS | 2026-04-13 | P | same |
| Operational readiness & DR | OPS | 2026-04-13 | P | same |
| Privacy & compliance | PRIV | 2026-04-13 | N/A | same |
| Documentation & architecture | DOC | 2026-04-13 | P | same |
| ML / AI / model governance *(if applicable)* | ML | 2026-04-13 | PWF | same |
| Release, change & incident process | RLS | 2026-04-13 | P | same |

*(Outcome: **P** = Pass, **PWF** = Pass with findings, **B** = Blocked.)*

---

## 3. How to run a full audit (instructions)

1. **Schedule:** Block time; a full audit is usually **multi-hour to multi-day** depending on system size.
2. **Freeze scope:** Decide environment(s), version/commit, and exclusions (e.g. `vendor/`, `legacy/`).
3. **Work top to bottom:** For each **§4** category, execute checks, capture artifacts, and fill the tables in **§2**.
4. **N/A discipline:** If a section does not apply, add one line: *“N/A — [reason]”* under that checklist (see **§0.5**).
5. **Track findings:** File issues with severity, owner, and due date; link them in **§5**.
6. **Sign-off:** Update **§2** full audit date and commit this file (or export a dated appendix).
7. **Audit report:** Produce the **standalone deliverable** in **[§8](#8-audit-report-deliverable-mandatory-end-state)** (`docs/reports/...`); use **`draft-audit-report`** skill to polish; optionally **`audit-report-to-queue`** to promote findings to **`QUEUE_STACK.csv`**.

**Automation:** Wire as many checks as practical into CI/CD; this document still captures **manual** verification (threat modeling, DR drills, SLO reviews).

**Using §4 prompts:** Each category in **§4** includes (1) an **Audit prompt** block—prepend **§0.3**, then paste into an AI agent or ticket; (2) a **Procedure** for hands-on steps; (3) **Pass criteria** checkboxes; (4) **Evidence** to attach. Run categories **in order** (G first) unless you are doing a **partial** audit—then still complete **G** minimally so scope stays explicit.

---

## 4. Categories — detailed audit prompts & procedures

**Before each category:** Prepend **§0.3** (master system prompt) and inject the **§0.2** repository profile. Each **Audit prompt** is written in **neutral language**; replace illustrative tools with what **§0.4** maps to for this project.

Each subsection is usable **standalone**: copy the **Audit prompt** into an AI agent or human runbook, follow **Procedure**, and attach **Evidence** to **§2** and **§5**. Finish with **§0.6** (standard output shape).

**Surface-type note:** Where a prompt says **“HTTP/API”**, **“browser”**, or **“routes”**, interpret it for this repo’s actual surface (e.g. **gRPC**, **GraphQL**, **CLI flags**, **plugin ABI**, **desktop UI**, **mobile app**). If there is no network surface, pivot to **trust boundaries** and **caller-supplied input** for that component.

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
5. **Sign-off prep:** Ensure **§2** “Scope notes” matches this section.

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
> (2) Review **ignore files** (e.g. `.gitignore`, container ignore files, IDE exclusions) and CI for leaked paths (`.env`, `*.pem`, `id_rsa`, cloud keys).  
> (3) Search for **high-entropy strings**, **API key patterns**, **private URLs** in code and docs.  
> (4) Verify **pre-commit** or **CI** blocks new secrets (grep hooks, detect-secrets baseline).  
> (5) Check **fork and clone** guidance: no instructions to copy real keys into docs.  
> **Outputs:** Pass/fail, list of findings by severity, rotation tickets for any live credential exposure.  
> **Constraints:** Do not print live secrets in outputs—redact or hash.

#### Procedure

1. Install/run scanner appropriate to org policy; include **full git history** if allowed.
2. Manually spot-check **compose/orchestration manifests**, **CI YAML**, and `README` for example keys—ensure they are **obviously dummy**.
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
> (2) Review **error handling**: swallowed errors, empty catch-all handlers, or ignored return values in security-sensitive code—flag each with severity (use language-appropriate patterns).  
> (3) **Concurrency:** identify shared mutable state, locks, async tasks, coroutines, thread pools, or GPU/parallel sections; check for races, deadlocks, re-entrancy.  
> (4) **Time and units:** timezone handling (UTC vs local), daylight saving, monetary rounding, integer overflow.  
> (5) **Idempotency:** retries must not double-charge or double-apply side effects.  
> **Outputs:** Per-path notes, list of correctness issues, recommended tests.  
> **Method:** Prefer reading code + tests together; run tests focused on critical modules.

#### Procedure

1. Build a **critical path list** with owners.
2. Use repo search / IDE for “swallow” patterns appropriate to the language (e.g. empty catch, TODO/FIXME in hot paths).
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

1. Run the project’s documented test command(s) (see **§0.2** / **§7**).
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

> You are performing an **application security** audit for the **actual attack surface** of this repository (HTTP APIs, gRPC, WebSockets, CLIs, libraries consumed by untrusted callers, desktop/mobile IPC, etc.). Use OWASP ASVS / API Security / **relevant** CWE categories as a **mental checklist**, adapted to this stack.  
> **Tasks:**  
> (1) **Authentication:** How are users/services proven? Passwords, API keys, OAuth, mTLS—document flows and storage of secrets client-side.  
> (2) **Authorization:** Enforce **every** sensitive operation server-side; look for IDOR (object references), missing role checks, admin routes.  
> (3) **Sessions/tokens:** JWT validation (alg, exp, audience), cookie flags (`HttpOnly`, `Secure`, `SameSite`), refresh token rotation.  
> (4) **Input:** Validation, max sizes, content types; **SQL** parameterized; **command** injection; **SSRF** on URL-fetching features.  
> (5) **Output:** XSS for HTML/JS contexts; CSP where applicable.  
> (6) **Transport:** HTTPS everywhere for prod; HSTS.  
> (7) **CORS:** restrict origins in prod; no `*` with credentials.  
> (8) **Rate limiting / abuse:** brute force, scraping, DoS—app-level or edge.  
> (9) **Deserialization / parsing:** no unsafe object deserialization or untrusted structured-data parsing (e.g. unsafe YAML/XML/binary) on attacker-influenced input.  
> **Outputs:** Threat-oriented findings with severity, reproduction steps, fix guidance.

#### Procedure

1. Read framework **middleware / plugin / filter order** (auth before sensitive handlers).
2. Enumerate **all** externally reachable operations (OpenAPI, router files, RPC services, CLI subcommands, IPC); sample high-risk ones.
3. For **browser-based UIs** using cookies, check **CSRF** and **clickjacking**; for **native/mobile**, check **secure storage** and **deep links** per platform guidance.

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
> (1) **Secrets:** Where do prod secrets live? Confirm **not** in git, not in **build artifacts** or **image layers** (if applicable), not in plain env in CI logs.  
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

- Redacted architecture diagram, IAM role list, IaC plan summary or equivalent.

---

### CONT — Container & image security

**Goal:** OCI/container images (if any) are **minimal**, run as **non-root** where appropriate, **scanned**, and **rebuilt** on patches. **If this repo does not produce container images,** state **N/A** per **§0.5** and skip detailed checks.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **container and image security**.  
> **Tasks:**  
> (1) Review **Dockerfile** (or equivalent): `USER`, multi-stage builds, no unnecessary packages, `COPY` scope minimal.  
> (2) Run **Hadolint** (or policy) on Dockerfile; fix or justify ignores.  
> (3) Build image from audited commit; run **Trivy/Grype** image scan; triage CRITICAL/HIGH.  
> (4) Check **base image** tag strategy (`:latest` vs pinned digest).  
> (5) Verify **no secrets** in image layers or build args (inspect build history or equivalent).  
> (6) **Runtime:** read-only root, `cap_drop`, seccomp/AppArmor if used.  
> **Outputs:** Scan report, policy for failing builds, patch cadence.

#### Procedure

1. Build with the same **toolchain** CI uses (e.g. `docker build`, `podman`, `buildpacks`, cloud build).
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
> (6) **Load testing:** use a **sustained** load tool appropriate to the surface (HTTP k6/Locust/JMeter, gRPC tooling, batch replay, game/bot simulation)—compare to SLO.  
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

- Flame graphs or profiles, load-test summary, metrics/dashboard snapshots (any backend).

---

### REL — Reliability & resilience

**Goal:** System **degrades** predictably and **recovers** without human heroics where possible.

#### Audit prompt (copy verbatim for an agent)

> You are auditing **reliability and resilience**.  
> **Tasks:**  
> (1) **Outbound calls:** timeouts set; retries with **jitter**; **circuit breakers** where appropriate; **idempotency keys** for retries.  
> (2) **Graceful shutdown:** termination signal handling for the runtime (e.g. SIGTERM, host shutdown hooks), drain period, in-flight work completion, queue consumers.  
> (3) **Dependencies:** behavior when DB, cache, or vendor is **down**—fail closed vs open, cached data staleness.  
> (4) **HA:** single points of failure listed; **multi-AZ/region** if claimed.  
> (5) **Chaos / game:** optional fault injection (kill pod, network partition) **or** documented **failure mode** table.  
> **Outputs:** Failure mode matrix (component × symptom × mitigation), list of code/config gaps.

#### Procedure

1. Read shutdown docs; test in staging if possible.
2. Trace **retry** logic for **financial / idempotent** or **side-effect** paths—no duplicate side effects.

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
> (2) **Metrics:** RED (rate, errors, duration) for request-style services where applicable; **USE** for resources; **business** or domain metrics (throughput, jobs completed, revenue, etc.).  
> (3) **Tracing:** OpenTelemetry spans for critical paths if distributed.  
> (4) **Dashboards:** do they cover **golden signals** for this stack (any vendor or self-hosted)?  
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
> (4) **Public contract docs:** OpenAPI/Swagger, AsyncAPI, GraphQL schema, Rustdoc/Javadoc, CLI `--help`—**current**; **examples** work.  
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
2. If containers exist, compare **build stages** for training vs inference; otherwise compare **packages** or **artifacts**.

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

## 5. Findings log (append each full audit)

| ID | Category | Severity | Summary | Tracking issue |
|----|----------|----------|---------|----------------|
| AUD-SUP-001 | SUP | Medium | Dedicated venv pip-audit — release-blocking | Fixed 2026-04-13 — [`FB-AUD-018`](../QUEUE_STACK.csv) Done |
| AUD-SEC-REPO-001 | SEC-REPO | Medium | Gitleaks CI job | Fixed 2026-04-13 — [`FB-AUD-019`](../QUEUE_STACK.csv) Done |
| AUD-STATIC-001 | STATIC | Low | Bandit in CI | Fixed 2026-04-13 — [`FB-AUD-020`](../QUEUE_STACK.csv) Done |

---

## 6. Sign-off

| Role | Name | Date |
|------|------|------|
| Engineering | | |
| Security / risk *(if applicable)* | | |
| Operations *(if applicable)* | | |

---

## 7. Repo-specific quick reference *(optional — customize per repository)*

Copy this block into your repo and fill it so auditors know **where** commands and policies live. **This is the only place** project-specific paths should appear—keep **§4** prompts unchanged.

| Topic | This repository |
|--------|-----------------|
| Primary languages / runtimes | Python ≥3.11 |
| CI / CD entry points | `.github/workflows/ci.yml` |
| Lint / format / typecheck commands | `python3 -m ruff check .` |
| Test commands | `python3 -m pytest tests/ -q` |
| Container / image build *(if any)* | Root `Dockerfile`; `docker build` in CI |
| Security / supply-chain scans in CI | Ruff, pytest, `ci_spec_compliance.sh`, `pip-audit` (informational), Trivy fs (informational), queue consistency script |
| Integration / E2E triggers | `integration-optional` job (`workflow_dispatch`) |
| Runbooks / ops docs | `docs/RUNBOOKS.MD`, `docs/READY_TO_RUN.MD` |
| Prior audit reports *(optional)* | [`docs/reports/AUDIT_REPORT_2026-04-13_full.md`](reports/AUDIT_REPORT_2026-04-13_full.md) |
| **Queue system** *(if used)* | [`docs/QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) · [`QUEUE_STACK.csv`](QUEUE_STACK.csv) · [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) |

---

## 8. Audit report deliverable *(mandatory end state)*

After completing **§4** categories (or a defined subset), the agent **must** produce a **standalone audit report** file—**not** only inline notes in this playbook. The report is the durable artifact for humans, PRs, and **downstream promotion** to the backlog (see **§8.4** and the **`audit-report-to-queue`** skill).

### 8.1 Where to save

| Convention | Path *(adapt per repo)* |
|------------|-------------------------|
| **Default** | `docs/reports/AUDIT_REPORT_<YYYY-MM-DD>_<short-slug>.md` |
| **Alternative** | `reports/audit-<YYYY-MM-DD>.md` |

Create the directory if needed. **Do not** overwrite prior reports without archiving or renaming.

### 8.2 Required sections *(in order)*

The audit report **must** include these sections (headings at exactly this level unless your org template says otherwise):

1. **`# Audit report`** — title line with **product/repo name** and **UTC date** of report completion.
2. **`## Metadata`** — table: repository, commit SHA/tag audited, audit lead, scope (in/out), link to this **`FULL_AUDIT.md`** copy, link to **§2** row in playbook if embedded in repo.
3. **`## Executive summary`** — 5–15 lines: overall posture, top 3 risks, top 3 strengths, **Pass / Pass with findings / Blocked**.
4. **`## Category results`** — one **`### <ID> — <Name>`** subsection per **§4** category executed (G, SEC-REPO, SUP, …). For each: **Verdict** (P / PWF / B / N/A), **Summary** (2–6 sentences), **Evidence** (paths, commands, CI jobs—no secrets).
5. **`## Findings`** — unified table: **Finding ID** (e.g. `AUD-G-001`, `AUD-APPSEC-002`), **Category ID**, **Severity**, **Location**, **Summary**, **Remediation**, **Status** *(Open / Accepted risk / Fixed)*. Map 1:1 to **§5** in this playbook when both exist.
6. **`## N/A categories`** — bullet list of **§4** categories marked N/A with **one-line reason** each (or *“None”*).
7. **`## Residual gaps`** — tooling limits, missing access, time-box caveats.
8. **`## Recommended next steps`** — ordered list: quick wins vs scheduled work; call out **promotable** items for **`audit-report-to-queue`** (**§8.4**).
9. **`## Sign-off`** *(optional)* — same roles as **§6** in this file; can mirror or reference **§6**.

### 8.3 Quality bar

- **Evidence-backed:** every **High** or **Critical** finding cites a **file path**, **config key**, **workflow name**, or **repro step**.
- **Actionable:** each finding has a **remediation** hint a developer can turn into a task.
- **Traceable:** finding IDs stay stable if the report is split into queue rows later (**audit-report-to-queue** skill).

### 8.4 Skills *(optional but recommended)*

| Skill | When |
|-------|------|
| **`draft-audit-report`** ([`.cursor/skills/draft-audit-report`](../.cursor/skills/draft-audit-report/SKILL.md)) | Polish raw §4 outputs into the **§8.2** structure; align tone and tables before sign-off. |
| **`audit-report-to-queue`** ([`.cursor/skills/audit-report-to-queue`](../.cursor/skills/audit-report-to-queue/SKILL.md)) | Promote **Open** findings into **`QUEUE_STACK.csv`** with **`agent_task`** rows. |

---

## 9. Reuse in other repositories

- **Copy** this file **verbatim**—do not strip **§0**; it is what makes the playbook portable.
- **Fill §0.2** and **§7** first; then run **§4** categories; finish with **§8** audit report.
- **Trim** nothing from **§4** in the shared template; use **N/A** in the audit record instead. Optionally delete **§7** if you embed repo facts only in **§0.2**.
- **Automate** what you can in CI; keep **§2** / **§5** as the durable record of dates and evidence; keep **§8** reports in `docs/reports/` (or equivalent).
- **Version:** bump the footer when you change structure so forks know they are stale.

---

*Template version: 3.2 — **§8** audit report deliverable; **§9** reuse; **§0** repository profile + master agent prompt + substitution guide; **§4** category prompts unchanged across repos.*

**Documentation last reviewed (this repo copy):** **2026-04-13** — **§2** audit record + **§5** findings log updated from [`reports/AUDIT_REPORT_2026-04-13_full.md`](reports/AUDIT_REPORT_2026-04-13_full.md); **§7** quick reference filled.
