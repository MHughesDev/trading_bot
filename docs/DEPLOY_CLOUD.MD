# Cloud deployment (FB-CONT-006)

Opinionated paths for running the **same** container image and Compose layout as homelab. See also **[`BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD)** and the container epic **FB-CONT-P0** in **[`QUEUE.MD`](QUEUE.MD)**. **Managed Redis / QuestDB / Qdrant vs self-hosted:** **[`ADR_MANAGED_DATA_SERVICES.MD`](ADR_MANAGED_DATA_SERVICES.MD)** (**FB-CONT-008**).

**Non-negotiable:** Kraken remains market data; venue keys and **`NM_RISK_SIGNING_SECRET`** never belong in git — load from a secret store into **`.env`** or process environment at runtime.

---

## Path A (primary): single Linux VM + Docker Compose + systemd

**When it fits:** You want one host, full Docker Compose parity with **`infra/docker-compose.yml`** + **`infra/docker-compose.app.yml`**, optional **`infra/docker-compose.edge.yml`**.

1. **VM shape (starting point, not a SLA):** **4 vCPU / 16 GiB RAM** / **100+ GiB** SSD for the data-plane volumes (QuestDB/Redis/Qdrant grow with use). Scale vCPU/RAM if you run **live_service** + nightly training + heavy Streamlit concurrently.
2. **OS:** Recent **Ubuntu LTS** or **Debian** with **Docker Engine** + **Compose v2** (`docker compose`).
3. **Disk:** Attach a **persistent block volume** (or large root disk) and mount it at a stable path (e.g. **`/srv/trading-bot/data`**). Map Compose **`../data`** host paths in **`docker-compose.app.yml`** to that mount, or keep default **`./data`** under the repo checkout on the volume.
4. **Networking — inbound:** Restrict **SSH** (key-only, non-default port optional), **8000** / **8501** (or **443** behind Caddy) to **office IP / VPN** only unless the UI must be public. **Outbound:** allow **HTTPS** for Kraken, Alpaca, Coinbase, and any registry pulls.
5. **Secrets:** Store **`NM_*`** in **AWS Secrets Manager**, **GCP Secret Manager**, **Azure Key Vault**, or **HashiCorp Vault**; at boot, materialize **`.env`** (mode **0600**) or export env vars before **`docker compose up`**. No secrets in Compose YAML beyond **`env_file`** pointing at a protected file.
6. **Run:** Clone repo (or deploy a release artifact), **`docker build`**, then:

   ```bash
   export COMPOSE_PROJECT_NAME=trading-bot
   docker compose -f infra/docker-compose.yml -f infra/docker-compose.app.yml up -d --build
   ```

7. **systemd:** Use **`Restart=always`** **`Type=simple`** units that **`WorkingDirectory=`** the repo and **`ExecStart=`** the **`docker compose up`** line (or a small wrapper script). After **`git pull`** / image updates, **`systemctl daemon-reload && systemctl restart trading-bot-stack`** (unit names are yours to define).
8. **Backups:** Follow **[`RUNBOOKS.MD`](RUNBOOKS.MD)** (**FB-CONT-005**) for volume **`tar`**; copy archives to **S3-compatible** storage in the same region as the VM to reduce egress charges on restore drills.

---

## Path B (sketch): AWS Fargate + EFS

**When it fits:** You need AWS-managed compute and are willing to split **one container per service** (API, Streamlit, live loop) and point stateful services to **managed** QuestDB/Redis/Qdrant **or** run data plane elsewhere.

- **EFS:** Mount **read/write** at **`/app/data`** for registry JSON, SQLite sidecars, JSONL files — tune **EFS throughput** if many small writes.
- **Secrets:** **ECS task secrets** from **Secrets Manager** → env; task role for **S3** backups if used.
- **Networking:** **ALB** → API (**8000**) and Streamlit (**8501**) on separate target groups, or one ALB with path/host rules; **security groups** least-privilege.
- **QuestDB / Redis / Qdrant:** Not covered by the default Fargate task — run **RDS/ElastiCache/managed vector** equivalents **or** keep data plane on a **small EC2** / **ECS EC2** node and connect via **`NM_QUESTDB_HOST`**, **`NM_REDIS_URL`**, **`NM_QDRANT_URL`** over **private networking**.

This path is **more moving parts** than Path A; use it when org standards require Fargate, not for minimal lift-and-shift.

---

## Egress and cost

- **Market data and venues:** Steady **HTTPS** to Kraken, Alpaca, Coinbase — budget **egress** if the VM is in a different region than APIs (usually small for JSON).
- **Backups:** Uploading **`tar`** archives to object storage in-region avoids cross-region egress.

---

## Related

- **TLS at the edge:** **[`infra/docker-compose.edge.yml`](../infra/docker-compose.edge.yml)** + **[`infra/caddy/Caddyfile`](../infra/caddy/Caddyfile)** (**FB-CONT-004**).
- **Volume backup commands:** **[`RUNBOOKS.MD`](RUNBOOKS.MD)** (**FB-CONT-005**).
