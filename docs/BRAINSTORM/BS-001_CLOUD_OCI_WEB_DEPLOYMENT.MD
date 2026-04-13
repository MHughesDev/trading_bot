# Cloud-native OCI deployment — web app, Linux containers, portable hosting

**Brainstorm ID:** BS-001  
**Scope:** multi  
**Status:** draft  
**Author:** optional

**See also:** **[`README.md`](../README.md)** — **Deployment models (FB-CONT-007)** — Windows **`.bat`** path vs Linux **Docker/Compose/cloud**; **[`docs/DEPLOY_CLOUD.MD`](../DEPLOY_CLOUD.MD)** for VM/Fargate notes; **[`docs/ADR_MANAGED_DATA_SERVICES.MD`](../ADR_MANAGED_DATA_SERVICES.MD)** (**FB-CONT-008**) — managed vs self-hosted data services.

---

## One-liner

Standardize operator and production deployment on **browser-first access**, **Linux OCI images** for all long-running components, **host OS irrelevant** (any machine with a container runtime), and **easy lift to cloud** using the same images and patterns as local/homelab.

## Problem / opportunity

The repo historically documents **Windows-first** paths (`setup.bat`, optional `.exe` launcher, pywebview) for convenience. A separate **deployment north star** is needed: **one reproducible stack** that runs as a **web application** (no dependency on desktop executables), packages **data + app + observability** as **containers**, and can move from **laptop → home server → cloud** without redesigning the architecture.

---

## High level

### Vision / north star

Operators use **HTTPS** (or **VPN/private overlay** such as Tailscale) to reach **FastAPI** and **Streamlit**. **QuestDB**, **Redis**, **Qdrant**, and **observability** (Prometheus, Grafana, Loki) run as **Linux containers** with **persistent volumes**. The **trading runtime** (live loop, decision engine, **risk/signing**, execution adapters) and **ML inference** run inside **Linux OCI images** — typically the **same container** as the control plane for parity, or a split **API + worker** if scale demands. **Market data stays Kraken-only**; **execution** remains **Alpaca (paper)** / **Coinbase (live)** per existing settings. **Secrets never ship in images.**

### Workstreams or sub-initiatives

| # | Name | What it covers | Depends on |
|---|------|----------------|------------|
| 1 | **Container images** | Dockerfiles for app (FastAPI + Streamlit + live_service + models), image tags, CI build | — |
| 2 | **Compose / orchestration** | `docker-compose` profiles for dev; optional **ECS/K8s/Fly/Railway** manifests or docs | 1 |
| 3 | **Stateful data** | Named volumes locally; cloud block storage + **backup to object storage** | — |
| 4 | **Edge & access** | Caddy/Nginx TLS, optional Tailscale; bind to localhost vs public | 2 |
| 5 | **Observability** | Prometheus scrape, Grafana/Loki as today; env for prod | 2 |
| 6 | **Secrets & config** | `.env` → cloud secret manager; `NM_*` mapping | 1, 2 |

### Cross-cutting dependencies

- **Risk and signing** must remain on the **OrderIntent** path — no “cloud shortcut” around `risk_engine` / `intent_gate`.
- **Kraken-only market data** — no Alpaca data clients outside the execution adapter (existing repo rule).
- **Mode changes** (paper/live) that affect the live loop should follow **restart / controlled rollout**, not silent hot swap (aligns with [`WINDOWS_OPERATOR_UI.MD`](../WINDOWS_OPERATOR_UI.MD) and execution profile semantics).

### Phased rollout *(optional)*

- **Phase 1 — Parity:** Documented **Compose** stack: infra images (QuestDB, Redis, Qdrant, Prom/Grafana/Loki) + **one app image**; developer runs locally on **any OS with Docker Desktop** or Linux engine.
- **Phase 2 — Hardening:** **TLS**, **volume backups** to S3-compatible storage, **healthchecks**, resource limits.
- **Phase 3 — Cloud:** Same images on a **managed container service** or single cloud VM; optional **managed Redis** / DB swaps behind explicit queue items.

---

## Proposal *(core)*

### Access and networking

| Piece | Role |
|-------|------|
| **Browser** | Sole client for **Streamlit** + API tooling; **no** requirement for PyInstaller `.exe` or pywebview in this deployment model. |
| **Reverse proxy + TLS** | **Caddy** or **Nginx** — HTTPS termination, optional basic auth / IP allowlists. |
| **Private access (optional)** | **Tailscale** or **SSH tunnel** when the stack should not be on the public internet. |

### Application tier (OCI — Linux)

| Piece | Role |
|-------|------|
| **FastAPI** (`control_plane`) | Control plane API: `/status`, power, future **positions/PnL** aggregates. |
| **Streamlit** | Multipage dashboard; **`NM_CONTROL_PLANE_URL`** to API. |
| **Live runtime** | `live_service` / supervisor — Kraken loop, **`run_decision_tick`**, execution routing. |
| **Decision + risk** | Unchanged shared path — **risk final**, signing, validators. |
| **Execution adapters** | **Alpaca** paper, **Coinbase** live — venue logic stays in adapters. |
| **Models / ML** | **PyTorch**, checkpoints, conformal — run **inside the app container image** (same Linux userspace local and cloud). |

### Data and memory tier (containers)

| Product | Role |
|---------|------|
| **QuestDB** | Time series: traces, fills, bars, PnL history when persistence enabled. |
| **Redis** | Hot cache, bar TTL, pub/sub patterns per config. |
| **Qdrant** | Vector memory / retrieval (`memory` feature). |

**Volumes:** **Named Docker volumes** or cloud **block storage**; fast SSD; **snapshots** or logical exports to **object storage**.

### Observability (containers)

| Piece | Role |
|-------|------|
| **Prometheus** | Metrics. |
| **Grafana** | Dashboards. |
| **Loki + Promtail** | Log aggregation. |

### Deployment pattern

| Layer | Method |
|-------|--------|
| **Artifact** | **OCI images** — app image from repo Dockerfile; upstream images for QuestDB, Redis, Qdrant, observability stack. |
| **Single host** | **Docker Compose** (extends [`infra/docker-compose.yml`](../../infra/docker-compose.yml) conceptually — may add **app service**). |
| **Cloud** | **ECS/Fargate, GKE, AKS, Cloud Run** (within limits), **Fly.io**, **Railway**, etc. — **same images**, different orchestration. |
| **Host OS** | **Irrelevant** at the ops abstraction: **Windows/macOS/Linux** with a **Linux container runtime**; workloads execute **Linux** inside containers/VM. |
| **Secrets** | Runtime injection — **`.env`** (dev), **cloud secret manager** (prod). |

### Relationship to Windows operator docs

[`WINDOWS_OPERATOR_UI.MD`](../WINDOWS_OPERATOR_UI.MD) remains valid for **desktop convenience** (`.bat`, optional launcher, pywebview). **BS-001** is the **server/cloud** counterpart: **web-only**, **OCI-first**.

---

## Options considered

| Option | Upside | Downside |
|--------|--------|----------|
| **A — OCI + Linux everywhere** | Portable to every major cloud; one build artifact; matches `infra/` services. | Requires Dockerfile maintenance; stateful services need backup discipline. |
| **B — Bare-metal Python per OS** | No container layer. | Poor reproducibility; host OS **not** irrelevant; painful cloud migration. |
| **C — Full PaaS “serverless”** | Minimal ops. | Poor fit for long-lived WebSocket/market loops and GPU/CPU-heavy inference without major rewrite. |
| **D — Kubernetes first** | Strong at scale. | Heavy for small deployments; still **containers** underneath — defer until needed. |

---

## Constraints / alignment

- [`AGENTS.md`](../../AGENTS.md): Kraken market data only; risk/signing path; no secrets in repo; no MLflow auto-promotion in code.
- **No secrets in images** — build args for non-secret metadata only.
- **Legacy `legacy/`** — not a target for this deployment work unless explicit bugfix.

---

## Open questions

- **Monolith image vs split** (API vs `live_service` worker) — start monolith unless scaling or failure isolation requires split.
- **Managed DBs** — when to adopt **managed Redis** or **managed TSDB** vs self-hosted QuestDB in cloud.
- **GPU** — if inference moves to GPU nodes, whether a separate **inference** image tier is needed.

---

## Risks *(multi-scope)*

- **Stateful migrations** — moving QuestDB/Redis/Qdrant data between regions needs runbooks and tested restore.
- **Cost** — always-on containers + block storage + egress; monitor vs homelab.
- **Latency** — cloud region vs exchange colocation; may matter for HFT-style strategies (document assumptions).

---

## Next step

Pick one:

- [x] **Promote** — row(s) added to [`QUEUE.MD`](../QUEUE.MD) as **FB-CONT-P0** and **FB-CONT-001 … FB-CONT-008** (see **Converting this brainstorm to the queue** below).
- [ ] **Spike** — prototype **one** `Dockerfile` for `control_plane` + minimal runtime smoke test.
- [ ] **Park** — no action for now.
- [ ] **Drop** — idea not pursued.

**Child brainstorms:** none.

**Promotion / queue IDs:** **FB-CONT-P0** (epic), **FB-CONT-001** … **FB-CONT-008** — see [`docs/QUEUE_ARCHIVE.MD`](../QUEUE_ARCHIVE.MD) §2.3.

---

## Converting this brainstorm to the queue

**Promoted 2026-04-12** — canonical rows live in **[§2.3 — Container / OCI cloud deployment (`FB-CONT-P0`)](../QUEUE_ARCHIVE.MD#23-container--oci-cloud-deployment-fb-cont-p0)** in **`QUEUE_ARCHIVE.MD`**. Use **FB-CONT-001 … FB-CONT-008** for traceability; link **BS-001** from epic Summary when editing related docs.

### Purpose

Turn the **multi-workstream** vision into **actionable queue items**. **Do not** duplicate the full brainstorm in `QUEUE.MD` — one row per slice with a pointer to **BS-001** or this file.

### Checklist after future splits

- [ ] Each new row has **Kind**, **Pri**, **Status**, **Summary** (with acceptance hints), **Affected files**.
- [ ] **BS-001** referenced in promoted rows for traceability.
- [ ] If **README** or **runbooks** change, update in the **same** PR as the queue entry (per `AGENTS.md`).
- [ ] **Contents** table in `QUEUE.MD` / `QUEUE_ARCHIVE.MD` updated only if a **new § anchor** is added (rare).

---

## Links

- [`README.MD`](../../README.md) — quick start  
- [`AGENTS.md`](../../AGENTS.md) — agent/operator contract  
- [`docs/QUEUE_ARCHIVE.MD`](../QUEUE_ARCHIVE.MD) — authoritative backlog tables (**FB-CONT-P0**)  
- [`infra/docker-compose.yml`](../../infra/docker-compose.yml) — current local stack  
- [`WINDOWS_OPERATOR_UI.MD`](../WINDOWS_OPERATOR_UI.MD) — Windows desktop / `.bat` brainstorm (complementary)
