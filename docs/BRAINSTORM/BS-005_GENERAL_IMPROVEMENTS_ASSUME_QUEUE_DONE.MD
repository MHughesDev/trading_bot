# General improvements (assume current queue is implemented)

**Brainstorm ID:** BS-005  
**Scope:** multi  
**Status:** draft  

---

## One-liner

**Cross-cutting** quality and product ideas for **after** (or alongside) today’s **[`QUEUE.MD`](../QUEUE.MD)** roadmap — written as if **FB-AP-P0**, **FB-CONT-P0**, and **FB-UX-P0** are **already shipped**, so this file does **not** repeat per-asset UI, multi-tenant auth, container deployment, canonical bars, or other **already-queued** themes.

---

## Out of scope here (tracked in queue — treat as done for brainstorming)

- **Per-asset** product: manifests, state machine, init pipeline, charts, search, backfill, nightly rules, **FB-AP-*** rows.  
- **Multi-tenant** web UX: auth, Account keys, gear, per-user PnL isolation, dashboard extras — **FB-UX-***.  
- **OCI / Compose / TLS / backup / cloud** deployment — **FB-CONT-***.  
- Specific **fixes** and **compliance gates** — **IL-***, **HG-***.

---

## Engineering & developer experience

| Idea | Notes |
|------|--------|
| **One-command dev stack** | `make dev-up` or script: compose + seed data + optional sample `.env` — reduces onboarding friction beyond README bullets. |
| **Typed API client SDK** | OpenAPI-generated **Python/TS** client for `control_plane` — fewer hand-rolled `requests` in tests and tools. |
| **Contract tests against fixtures** | Golden JSON for `/status`, manifest payloads, decision outputs — catch drift without full integration. |
| **Structured logging schema** | Versioned JSON log fields (`correlation_id`, `user_id`, `symbol`) for Loki queries and alerts. |
| **Feature flags** | Lightweight toggles (file or Redis) for experimental paths without long-lived branches. |
| **Local load / soak harness** | Scripted replay N× speed to find memory leaks in ingest or decision loop. |

---

## Observability & operations

| Idea | Notes |
|------|--------|
| **SLO-style dashboards** | Grafana rows: “time to first bar after subscribe”, “order submit latency”, “risk reject rate” — not just raw metrics. |
| **Synthetic checks** | Cron job hits `/status` + one read-only data probe; page if unhealthy. |
| **Runbook-linked alerts** | Every alert annotation links to a **specific** `RUNBOOKS.MD` subsection. |
| **Cost visibility** | Rough monthly estimate doc: API call rates × venue pricing assumptions (even if approximate). |

---

## Product & UX (beyond queued UI)

| Idea | Notes |
|------|--------|
| **Guided first-run wizard** | After login: “add keys → pick symbol → initialize” with **skippable** steps and progress persistence. |
| **In-app glossary** | Short definitions: *conformal*, *regime*, *flatten*, *paper vs live* — reduces support load. |
| **Accessibility pass** | Keyboard nav, contrast, focus order on primary flows (WCAG-oriented spot check). |
| **Mobile-friendly read-only** | At least **view** PnL and read alerts on phone; trading stays desktop if desired. |
| **Release notes in app** | What changed this version — pulled from CHANGELOG or git tags. |

---

## Risk, safety, compliance (incremental)

| Idea | Notes |
|------|--------|
| **Kill switch tiers** | Separate “pause new entries” vs “flatten all” vs “read-only” — finer than single flatten when product supports it. |
| **Paper-to-live checklist gate** | UI forces confirmation steps before enabling live for a symbol (beyond phrase typing). |
| **Audit export** | Immutable **CSV/JSON** bundle of settings + last N orders for external review (not real-time surveillance). |

---

## Data & ML hygiene

| Idea | Notes |
|------|--------|
| **Dataset versioning** | DVC or simple manifest: which Parquet snapshot trained which model artifact. |
| **Experiment comparison** | Side-by-side metrics for two forecaster runs (same eval set). |
| **Data quality score** | Per-symbol score: gaps, outliers, stale bars — surfaces before training. |
| **Cold-start simulation** | Backtest “day 1” behavior with limited history to set expectations. |

---

## Ecosystem & integrations (optional futures)

| Idea | Notes |
|------|--------|
| **Webhook notifications** | Slack/Discord on fill, risk block, or nightly job failure (operator-configured URL + HMAC). |
| **Read-only public API** | API key with **GET-only** scope for portfolio summary — partners or personal scripts. |
| **Calendar export** | iCal of maintenance windows or scheduled retrains (if schedules become user-visible). |

---

## Open questions

- Which **two** themes from this list matter most after **FB-AP-P0** + **FB-UX-P0** stability?  
- Should any item be promoted to [`QUEUE.MD`](../QUEUE.MD) as its own **FB-** row, or stay **parked** here?

---

## Next step

- [ ] **Park** — revisit quarterly or after major releases.  
- [ ] **Promote** — cherry-pick 1–2 rows into **QUEUE** if they become priorities.

---

## Links

- [`QUEUE.MD`](../QUEUE.MD) — authoritative backlog (this brainstorm **does not** supersede it)  
- [`DEFERRED_ROADMAP.MD`](../DEFERRED_ROADMAP.MD) — deferred themes  
