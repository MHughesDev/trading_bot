# ADR: Managed vs self-hosted data services (FB-CONT-008)

**Status:** Accepted (2026-04-12)  
**Queue system:** [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) · **FB-CONT-008** in [`QUEUE_ARCHIVE.MD`](QUEUE_ARCHIVE.MD) · traceability **[`BS-001`](BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD)**  
**Scope:** Redis, time-series / QuestDB-shaped storage, Qdrant for **cloud** deployments — **not** a mandate to change `infra/docker-compose.yml` defaults.

---

## Context

The default stack runs **Redis**, **QuestDB**, and **Qdrant** as **containers** on one host (**`infra/docker-compose.yml`**), with **backup** guidance in **[`runbooks.md`](runbooks.md)** (**FB-CONT-005**). Cloud operators may consider **managed** equivalents (lower ops, higher recurring cost, different networking and latency).

---

## Comparison (qualitative)

| Dimension | Self-hosted (Compose on VM) | Managed (vendor Redis / TSDB / Qdrant) |
|-----------|------------------------------|----------------------------------------|
| **Cost** | VM + disk only; you operate upgrades | Per-hour + storage + egress; predictable SKUs but can exceed small VMs at scale |
| **Ops load** | You patch images, monitor disk, run backups | Vendor handles patching, HA options, some backup SLAs |
| **Latency** | App ↔ DB on **same Docker network** (microseconds–low ms) | Cross-AZ or cross-region adds **ms**; must co-locate app **region** with managed endpoints |
| **Migration** | **None** from current defaults | New connection strings (**`NM_REDIS_URL`**, **`NM_QUESTDB_*`**, **`NM_QDRANT_URL`**), possible **schema** / **wire** differences (QuestDB PG wire vs another TSDB), **data export/import** window |

**Redis:** Managed Redis (ElastiCache, Memorystore, Azure Cache) is **compatible** with `redis://` clients if TLS and ACLs are configured in URL — test **`NM_REDIS_URL`** in staging.

**QuestDB / TSDB:** **QuestDB Cloud** keeps the same **PostgreSQL wire** story as self-hosted QuestDB; other managed TSDBs (Timestream, ClickHouse Cloud, etc.) imply **different SQL**, **different** `QuestDBWriter` / ingest path — **out of scope** for a drop-in swap without a dedicated migration project.

**Qdrant:** **Qdrant Cloud** offers HTTP/gRPC endpoints; the Python client expects **`NM_QDRANT_URL`** — verify **TLS** and **API key** env vars per vendor docs.

---

## Decision

**Default recommendation:** **Keep self-hosted containers** on a **single Linux VM** (or small cluster) for **homelab → first cloud** moves — lowest change risk, matches **[`deploy_cloud.md`](deploy_cloud.md)** Path A and existing code paths.

**Consider managed** when: you need **vendor SLA**, **multi-AZ HA** without running it yourself, or **compliance** mandates managed encryption keys — **after** measuring **latency** from the app region to the managed endpoint and **pricing** at your retention size.

**Do not** swap the **canonical bar store** to a different TSDB without an explicit **migration + read path** project (this ADR does not authorize that).

---

## Follow-up

No new **QUEUE** rows unless product chooses a **managed TSDB other than QuestDB** (would require adapter work beyond **`NM_*`** URL changes).

---

## Related

- **[`architecture/adr/canonical_bar_storage.md`](architecture/adr/canonical_bar_storage.md)** — why QuestDB-shaped canonical bars exist.  
- **[`deploy_cloud.md`](deploy_cloud.md)** — VM vs Fargate sketch.  
- **[`runbooks.md`](runbooks.md)** — volume backup (**FB-CONT-005**).
