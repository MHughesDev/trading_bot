# Code review audit — functionality, correctness, quality, consistency

**Audit passes:** **2026-04-17** (initial), **2026-04-17** (second pass), **2026-04-17** (third pass — Streamlit/config/SQL/TLS spot checks)  
**Scope:** Static review of critical paths, security-relevant defaults, dependency and subprocess usage, alignment with `AGENTS.md`. **Not** performance tuning, new features, load testing, or full penetration testing. **Not** exhaustive line-by-line coverage of the whole repo (~400 test files / full tree).

**Method (pass 1):** Read `execution/intent_gate.py`, `execution/service.py`, `control_plane/api.py` (auth, CORS), `decision_engine/run_step.py`, `control_plane/preflight.py`; pattern scan `except Exception`, TODO/FIXME.

**Method (pass 2):** `rg` for `CORSMiddleware` / `allow_origins`, `shell=True`, `eval(`, dangerous `pickle`, `subprocess` usage; spot-check `bare except` / `pass` in hot paths; verify audit doc completeness (out-of-scope list, sign-off, backlog).

**Method (pass 3):** `control_plane/streamlit_util.py` (route guard, `NM_STREAMLIT_ROUTE_GUARD_ENABLED`, API key bypass); `app/config/settings.py` for **`yaml.safe_load`**; `rg` for **`verify=False`**, **`yaml.load(`** (unsafe), **`pickle`** in `app/`; sample **`data_plane/storage/questdb.py`** for parameterized SQL; consider **rate limiting** and **Streamlit** exposure.

---

## Executive summary

| Theme | Assessment |
|-------|------------|
| **Trading path** | **Strong:** `RiskEngine` → signed `OrderIntent` → `ExecutionService.submit_order` → `require_execution_allowed` → adapter. |
| **Live vs replay** | **Strong:** `run_decision_tick` is the shared decision step; `system_power` consulted early. |
| **Operator safety** | **Mixed:** Preflight and `/status` expose configuration health; **API** can run **without** API key in dev (see A-1); **CORS** is fully permissive (see A-6). |
| **Streamlit** | **Mixed:** **`require_streamlit_app_access`** enforces login when **`NM_STREAMLIT_ROUTE_GUARD_ENABLED`** and no **`NM_CONTROL_PLANE_API_KEY`** in the Streamlit process (see A-8); guard **off** by default — same “trusted local operator” assumption as the API. |
| **Maintainability** | **Mixed:** Broad `except Exception` and bare `pass` in several modules — resilience vs debuggability tradeoff. |
| **Repo policy** | **Enforced in CI:** Kraken-only market data — `ci_spec_compliance.sh` blocks **Alpaca *data*** imports outside the paper adapter; MLflow promotion (`ci_mlflow_promotion_policy.sh`). |
| **Config loading** | **Good:** YAML parsed with **`yaml.safe_load`** in `AppSettings` (pass 3). |

---

## What this audit does **not** cover (explicit)

- Performance profiling, latency SLOs, memory leaks  
- Full security audit (OWASP-style), dependency CVE scan (use `pip-audit` / GitHub Dependabot separately)  
- Coinbase / Alpaca API correctness against live vendor docs  
- QuestDB / Redis / Qdrant data durability under failure  
- Streamlit UX beyond error-handling and route-guard patterns  
- Contents of `legacy/` except to note policy: **frozen snapshot** per `AGENTS.md`  
- **Secrets in git history** — use `git-secrets` / pre-commit hooks separately  
- **API rate limiting / abuse protection** — not reviewed as implemented (assume **reverse proxy** or **WAF** in production if needed)  
- **PII / GDPR** — not applicable to this audit scope  

---

## Strengths

| Area | Notes |
|------|--------|
| **Execution gate** | `require_execution_allowed` centralizes HMAC checks; `ExecutionService.submit_order` always invokes it before adapters. |
| **Shared decision step** | `run_decision_tick` documented as shared live/replay path; `system_power` sync early. |
| **Preflight** | Live mode flags missing signing secret and unsigned execution; adapter name validation. |
| **Per-asset execution** | `adapter_for_symbol` / `_adapter_for_intent` keep routing aligned with `OrderIntent.symbol`. |
| **Tests** | Large `tests/` suite; CI scripts enforce Kraken market-data policy and MLflow promotion policy. |
| **Subprocess hygiene** | `power_supervisor` uses `subprocess.run` with argument list (no `shell=True` in sampled code). |
| **Multi-tenant context** | `TenantContextMiddleware` + `ContextVar` for request-scoped user id (FB-UX-007 pattern). |
| **YAML config** | `AppSettings` loads defaults via **`yaml.safe_load`** (not unsafe `yaml.load`). |
| **SQL (sampled)** | QuestDB paths use **parameterized** `execute(sql, params)` patterns — reduces injection risk vs string concatenation. |

---

## Issues found

| ID | Severity | Topic | Description |
|----|----------|--------|-------------|
| A-1 | **Medium** | Control plane auth | When **`NM_CONTROL_PLANE_API_KEY`** is **unset** and **`NM_AUTH_SESSION_ENABLED`** is false, **`require_mutate_operator`** allows mutating routes **without** credentials. Intentional for local dev; **unsafe** if the API is exposed without TLS + firewall or another auth layer. |
| A-2 | **Low** | Error handling | Many **`except Exception:`** blocks across `live_service.py`, Streamlit, ingestion, and adapters. Reduces crashes but can **hide** programming errors; logs may not always include enough context. |
| A-3 | **Low** | Intent gate semantics | If **`risk_signing_secret`** is **unset**, **`execution_allowed`** returns **True** (gate open) even when **`allow_unsigned_execution`** is false — because the “no secret” branch short-circuits. Preflight warns for **live**; still easy to misread. |
| A-4 | **Info** | Queue maintenance | **`docs/QUEUE_STACK.csv`** must stay aligned with **`QUEUE_ARCHIVE.MD`** **Open** rows (and the §2 snapshot in **`QUEUE.MD`**) — **drift** if only one file is edited. |
| A-5 | **Info** | Default config | **`app/config/default.yaml`** uses **`execution.mode: live`** — safe operation depends on **`.env`** and operator discipline. |
| A-6 | **Medium** | CORS | **`control_plane/api.py`** registers **`CORSMiddleware`** with **`allow_origins=["*"]`** (and permissive methods/headers) for Streamlit/SSE (FB-AP-034). Correct for **LAN dev**; for **public** or **multi-tenant** deployment, **narrow origins** and review headers. |
| A-7 | **Low** | Dependency hygiene | No automated **CVE** scan called out in-repo in this audit — rely on **`pip-audit`**, lockfiles, or GitHub **Dependabot** for ongoing supply-chain review. |
| A-8 | **Low** | Streamlit route guard | **`NM_STREAMLIT_ROUTE_GUARD_ENABLED`** defaults **off**; when **off**, Streamlit pages do not enforce **`require_streamlit_app_access`**. Align deployment docs: enable guard + sessions for shared or exposed UIs. **`NM_CONTROL_PLANE_API_KEY`** in the Streamlit process is a **bypass** for automation (documented in code). |

---

## Non-issues / verified (passes 2–3)

| Check | Result |
|-------|--------|
| **`shell=True` in subprocess** | No problematic hits in sampled `rg` (PyTorch `model.eval()` is unrelated). |
| **`eval(` / unsafe `yaml.load` / `pickle` in `app/`** | No unsafe YAML load; no `pickle` in `app/` from scan; **`yaml.safe_load`** used for settings. |
| **`verify=False` (TLS)** | No hits in `*.py` from pass 3 scan. |
| **`power_supervisor` terminate** | Uses `taskkill` with fixed args on Windows; `logger.exception` on failure — reasonable. |
| **Kraken market data policy** | Documented in `AGENTS.md`; enforced by **`scripts/ci_spec_compliance.sh`**. |
| **SQLite in `platform_supported_universe`** | Static SQL strings with bound params / controlled attach path — acceptable; not user-controlled raw SQL from HTTP. |

---

## Recommended changes (not implemented by this audit doc)

1. **A-1 / A-6:** Document in **`README.md`** / **`RUNBOOKS.MD`**: production **must** use **TLS**, **firewall**, **`NM_CONTROL_PLANE_API_KEY`** and/or **session auth**, and **restrict CORS origins** when exposing the API beyond localhost. Optional: env-driven `allow_origins` list defaulting to `["*"]` only in dev.
2. **A-2:** Prefer **`logging.exception`** or structured logs in hot paths; narrow exceptions where the failure mode is known.
3. **A-3:** Add a short comment in **`intent_gate.py`** explaining the “no secret → allow” branch vs preflight.
4. **A-4:** Ensure **`AGENTS.md`** §10 or contributor checklist mentions **`QUEUE_STACK.csv`** + **`QUEUE_ARCHIVE.MD`** when editing queue workflow (may already be partially covered).
5. **A-5:** Product decision: default **`execution.mode: paper`** in `default.yaml` for safer first clone — optional.
6. **A-7:** Add optional **`pip-audit`** or document it in **`README.md`** CI / operator section.
7. **A-8:** Document **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=true`** for any deployment where the dashboard is reachable beyond a single trusted operator.

---

## Files reviewed (cumulative, non-exhaustive)

- `execution/intent_gate.py`, `execution/service.py`, `execution/router.py` (indirect)  
- `control_plane/api.py` (auth, CORS, middleware order)  
- `control_plane/preflight.py`  
- `control_plane/streamlit_util.py` (route guard, pass 3)  
- `decision_engine/run_step.py`  
- `app/config/settings.py` (YAML `safe_load`, pass 3)  
- `app/runtime/power_supervisor.py` (subprocess sample)  
- `data_plane/storage/questdb.py` (SQL execute pattern, pass 3)  
- `app/config/default.yaml` (spot)  
- Pattern scans: `except Exception`, `pass`, `CORSMiddleware`, `shell=True`, `verify=False`, `yaml.load`, `pickle`

---

## Sign-off

| Item | Status |
|------|--------|
| Critical trading path (risk → signed intent → gate → adapter) | **Reviewed** (architecture consistent) |
| Control plane exposure (auth + CORS) | **Reviewed** — **action items** for production deployment |
| Streamlit vs API auth model | **Reviewed** (guard optional; document for production) |
| CI policy scripts | **Referenced** — not re-run as part of this document |
| **Residual risk** | Operators **must** configure secrets and network boundaries; codebase assumes **trusted network** unless configured otherwise. |

---

## Queue promotion (2026-04-18)

The granular backlog below is **promoted** to the live queue as **§2.5** in **[`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD#25-post-audit-hardening-fb-aud)** — IDs **`FB-AUD-001`** … **`FB-AUD-017`**, with **`DOC-A1`** … **`LEGACY-A1`** traceability preserved in each row. **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** lists the same work in **`stack_order`** with self-contained **`agent_task`** (implement top to bottom unless reprioritized). Conventions: **[`QUEUE.MD`](QUEUE.MD)**.

**Done slices:** **`FB-AUD-001`** (2026-04-19) — **`README.md`** + **`RUNBOOKS.MD`** production exposure checklist; **`FB-AUD-002`** (2026-04-19) — Streamlit route guard tables + runbook + **`streamlit_util`** docstrings; see §2.5 rows.

---

## Source table (audit IDs → queue IDs — historical)

| Audit ID | Queue ID | Theme |
|----------|------------|-------|
| DOC-A1 | FB-AUD-001 | Production API hardening (docs) |
| DOC-A2 | FB-AUD-002 | Streamlit route guard (docs) |
| OPS-A1 | FB-AUD-003 | Startup warning (bind + API key) |
| OPS-A2 | FB-AUD-004 | Rate limiting |
| QA-A1 | FB-AUD-005 | `live_service` exception logging |
| QA-A2 | FB-AUD-006 | Streamlit exception logging |
| CODE-A1 | FB-AUD-007 | `intent_gate` comment |
| CI-A1 | FB-AUD-008 | `QUEUE_STACK` vs §2 validation |
| SEC-A1 | FB-AUD-009 | CORS allowlist |
| SEC-A2 | FB-AUD-010 | Session cookie secure docs |
| SEC-A3 | FB-AUD-011 | `pip-audit` |
| SEC-A4 | FB-AUD-012 | `httpx` inventory |
| TEST-A1 | FB-AUD-013 | `require_mutate_operator` tests |
| DEBT-A1 | FB-AUD-014 | Silent except / pass inventory |
| DATA-A1 | FB-AUD-015 | QuestDB observability |
| MT-A1 | FB-AUD-016 | Multi-tenant paths |
| LEGACY-A1 | FB-AUD-017 | `legacy/` isolation |

---

## Document revision history

| Date | Change |
|------|--------|
| 2026-04-17 | Pass 1 — initial audit (trading path, API auth, preflight, exceptions). |
| 2026-04-17 | Pass 2 — CORS, dependency CVE note, backlog table, sign-off. |
| 2026-04-17 | Pass 3 — Streamlit guard (A-8), YAML `safe_load`, TLS/sql/pickle spot checks, expanded non-issues, DOC-A2/OPS-A2/SEC-A4 backlog items. |
| 2026-04-19 | Docs consistency: `SYSTEM_WALKTHROUGH`, `Specs/SYSTEM_OVERVIEW`, `QUEUE` HG-01, `RISK_PRECEDENCE`, `ci_spec_compliance` message — Kraken market data vs Alpaca *data* CI guard (aligned with code). |

---

*This file is an audit artifact; it does not change runtime behavior. Update this document after major architecture or auth changes.*
