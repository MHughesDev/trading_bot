# BRAINSTORM

**Ideas and rough plans only.** Everything here is **non-binding** until it moves into [`../QUEUE.MD`](../QUEUE.MD) or another official doc.

## What belongs here

| Content | Example |
|---------|---------|
| New integrations or vendors | Exchange, data feed, auth provider |
| Product / UX directions | Flows you might build later |
| Architecture “what if?” | ADR drafts before a decision |
| Spikes and experiments | Things to validate before committing |
| **Multi-part or program-level** integrations | Several systems, phases, or teams — still one **`BS-NNN`** doc; use **Scope: multi** in the template |

**One template:** [`TEMPLATE_BRAINSTORM.MD`](TEMPLATE_BRAINSTORM.MD) covers **single** (one change, one integration) and **multi** (larger programs). Set **`Scope:`** in the metadata; for **multi**, fill **Vision**, **Workstreams**, **Dependencies**, **Phased rollout**, and **Risks** as needed — leave optional sections blank for **single** ideas.

## What does *not* belong here

- **Tracked work** — use [`../QUEUE.MD`](../QUEUE.MD) (IDs, status, owners).
- **Operator runbooks** — use [`../RUNBOOKS.MD`](../RUNBOOKS.MD) or a dedicated `docs/*.MD` once the idea ships.
- **Secrets** — never paste keys or real credentials.

## Brainstorm IDs

Each idea file gets a stable **`BS-NNN`** id (e.g. **`BS-001`**, **`BS-007`**):

- Assign the **next free** number when you create a file from the template (search this folder for `**Brainstorm ID:** BS-` or the string `BS-` in `.MD` files to find the highest in use).
- Put the id in the **metadata line** at the top of the file (see template). **Optional:** repeat it in the filename, e.g. `BS-012_IDEA_KRAKEN_FUTURES.MD`, so lists sort by id.
- When you **promote** to [`QUEUE.MD`](../QUEUE.MD), you may reference **`BS-NNN`** in the queue Summary or Affected files for traceability.

IDs are **not** the same as **`FB-*` / `IL-*`** queue IDs.

## Workflow

1. **Copy** [`TEMPLATE_BRAINSTORM.MD`](TEMPLATE_BRAINSTORM.MD) → rename to something descriptive (`IDEA_KRAKEN_FUTURES.MD`, `INTEGRATION_ALPACA_OPTIONS.MD`, or `BS-NNN_….MD` if you use id-in-filename).
2. **Assign** the next **`BS-NNN`** and **fill** the sections; keep it short enough to skim in one pass.
3. **End state:** either **promote** (new `QUEUE` row + link from this file’s “Promotion” section), **merge** into a real design doc, or **leave parked** / delete when obsolete.

## Naming files

Use **UPPER_SNAKE** and a **prefix** so lists sort usefully:

- `IDEA_…` — product / UX / vague direction  
- `INTEGRATION_…` — external system  
- `SPIKE_…` — time-boxed technical question  
- `ADR_DRAFT_…` — architecture decision not yet decided  
- `EPIC_…` or `PROGRAM_…` *(optional)* — hints that the file is **multi**-scope (still use **`Scope: multi`** in the body)

Use **`.MD`** (uppercase extension) to match other docs in this repo.

**Splitting large ideas:** You may use one **`BS-NNN`** for the umbrella and **link** other **`BS-MMM`** files from **Child brainstorms** in the template — or keep everything in one file if it stays readable.

## Current brainstorm files

| ID | File |
|----|------|
| **BS-001** | [`BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD) — OCI/Linux containers, web-first deployment, cloud portability. **Promoted** to [`QUEUE.MD`](../QUEUE.MD) §2.3 **FB-CONT-P0** / **FB-CONT-001**… |
| **BS-002** | [`BS-002_QUEUE_VS_UI_GAP_ANALYSIS.MD`](BS-002_QUEUE_VS_UI_GAP_ANALYSIS.MD) — **QUEUE** (QueueLog) vs Streamlit UI gap map for **FB-AP-P0**. |
| **BS-003** | [`BS-003_ADDITIONAL_DASHBOARD_ASSET_UI_IDEAS.MD`](BS-003_ADDITIONAL_DASHBOARD_ASSET_UI_IDEAS.MD) — Extra Dashboard / Asset Page UI ideas (many **promoted** to **FB-UX-008**–**014** in **QUEUE** §2.4). |
| **BS-004** | [`BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD`](BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD) — Multi-tenant auth, Account page, per-user PnL — **promoted** to [`QUEUE.MD`](../QUEUE.MD) §2.4 **FB-UX-P0**. |
| **BS-005** | [`BS-005_GENERAL_IMPROVEMENTS_ASSUME_QUEUE_DONE.MD`](BS-005_GENERAL_IMPROVEMENTS_ASSUME_QUEUE_DONE.MD) — General improvements **assuming** current queue epics are done; **no overlap** with tracked **FB-AP** / **FB-UX** / **FB-CONT** rows. |

## Related

- [`../QUEUE.MD`](../QUEUE.MD) — authoritative backlog  
- [`../DEFERRED_ROADMAP.MD`](../DEFERRED_ROADMAP.MD) — deferred themes (when promoted from “someday”)
