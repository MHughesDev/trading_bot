# Queue (QLOG) vs control-plane UI — gap analysis

**Brainstorm ID:** BS-002  
**Scope:** multi  
**Status:** draft  
**Author:** (session)

---

## One-liner

Compare what **`docs/QUEUE_ARCHIVE.MD`** (canonical backlog tables; legacy backlog lived in monolithic **`QUEUE.MD`**) promises for **FB-AP-P0** (per-asset product) with what operators can do today in **Streamlit** (`control_plane/Home.py` + `pages/`), and surface **API-only** capabilities until the two-page UI lands.

---

## Problem / opportunity

The queue describes a **two-page** operator product (Dashboard + Asset Page), **per-asset** execution mode, **state machine** buttons, and removal of **global** power and **app-wide** paper/live. The **FastAPI** layer already exposes large slices of **registry + init pipeline**; the **Streamlit** shell still reflects the **pre-epic** model (global power, global execution mode, multipage Live/Regimes/Routes/Models/Logs/Emergency). Without a written map, planning and prioritization drift.

---

## Definitions

| Term | In this repo |
|------|----------------|
| **QLOG / QueueLog** | **`docs/QUEUE.MD`** + **`docs/QUEUE_STACK.csv`** + **`docs/QUEUE_ARCHIVE.MD`** — do not create a separate `QueueLog.md` unless explicitly requested ([`.cursor/skills/add-to-queue/SKILL.md`](../../.cursor/skills/add-to-queue/SKILL.md)). |
| **UI** | **Streamlit** multipage app: `control_plane/Home.py`, `control_plane/pages/*.py`. |

---

## High level

### Vision / north star

**QUEUE** §2.2 **Brief**: Dashboard (holdings + PnL chart, no global start/stop) + Asset Page (per-symbol chart, markers, per-asset Paper/Live, Initialize / Start / Stop), canonical bars, universe search, push updates — with **FB-AP-039** / **FB-AP-040** removing global power and global execution mode from the operator mental model.

### Workstreams (queue batches vs UI touchpoints)

| Area | Queue batch(es) | In Streamlit today | Notes |
|------|-----------------|--------------------|-------|
| Per-asset manifest + registry | AP-PAM (**FB-AP-001** Done, **FB-AP-002** Done) | **Not exposed** | **`GET /assets/models`**, **`GET/PUT/DELETE /assets/models/{symbol}`**; **`GET /status` → `asset_model_registry.initialized_symbols`** |
| Init pipeline | AP-INI (**FB-AP-006**…**012** Done) | **Not exposed** | **`POST /assets/init/{symbol}`**, **`GET /assets/init/jobs/{job_id}`** |
| Runtime guards (forecaster / policy binding) | AP-ENF (**FB-AP-003**, **004** Open) | N/A | Backend |
| Per-asset state machine | AP-STA (**FB-AP-005** Open) | **Not exposed** | Buttons **Initialize / Start / Stop** blocked on state API |
| Global system power | AP-REM (**FB-AP-039** Open — remove) | **Still present** | Sidebar **System power** ON/OFF → **`/system/power`** |
| App-wide paper/live | AP-REM (**FB-AP-040** Open — remove) | **Still present** | Sidebar execution mode → **`/system/execution-profile`** |
| Dashboard redesign | AP-UI-D (**FB-AP-026** Open) | **Partial** | **`render_pnl_panel`**, **`render_positions_sidebar`** on **Home** — not the final “PnL-only + holdings + no global controls” layout |
| Asset Page (chart, markers, per-asset mode, buttons) | AP-UI-A (**FB-AP-027**…**031**, **033**) | **Absent** | No `asset_page.py`, no `/asset/<symbol>` route |
| Container / OCI deploy | **FB-CONT-P0** | N/A | No **`Dockerfile`** in repo yet (**FB-CONT-001** Open) |

### Cross-cutting dependencies

- **UI** work (**FB-AP-026**–**033**, **039**–**040**) depends on **API** and **state** rows (**FB-AP-005**, **FB-AP-024**–**025**, **FB-AP-023**) where noted in **QUEUE**.
- Operators can use **curl** / **HTTP client** against **`NM_CONTROL_PLANE_URL`** for registry and init today; **Streamlit** does not surface those flows yet.

---

## Proposal

- Treat this file as a **living map** while **FB-AP-P0** is open; **update** when **QUEUE** rows change status or when **Home.py** / pages materially change.
- When implementing **Asset Page**, wire **init** and **manifest** to the existing endpoints above rather than duplicating logic.
- **Incomplete user request (2026-04-12):** the prompt ended mid-sentence (“…between what the QLOG says and what we already have implemented in the”). **Fill in** the trailing intent here (e.g. “in the **Streamlit UI**”, “in the **API**”, “in **runtime**”) and extend the comparison table if needed.

---

## Options considered

| Option | Upside | Downside |
|--------|--------|----------|
| Document gaps only in **QUEUE** | Single file | Queue already long; UI/API gap narrative is not a work item row |
| **Brainstorm** (this file) + optional later **PER_ASSET** or **CONTROL_PLANE** doc update | Lightweight; links to **FB-*** IDs | Can drift if not refreshed when shipping UI |
| Full **SPEC** update now | Authoritative | User did not ask for spec edits; **QUEUE** is already detailed |

---

## Constraints / alignment

- [`AGENTS.md`](../../AGENTS.md): Kraken-only market data; risk/signing path unchanged.
- **API** keys: never document real secrets; reference **`NM_*`** only.

---

## Open questions

- Should **Streamlit** add a minimal **“Initialized assets”** panel that calls **`GET /assets/models`** and **POST /assets/init/{symbol}`** before the full Asset Page (**FB-AP-027**), as an incremental operator win?
- What is the exact completion of the user’s question (see **Proposal**)?

---

## Next step

- [ ] **Promote** — only if a single **QUEUE** row is needed for “gap documentation”; otherwise keep as **BS-002** reference.
- [x] **Park** — brainstorming / gap map until UI epic progresses.
- [ ] **Spike** — optional: automated check that **QUEUE** “Done” rows for **FB-AP-*** match **`grep`** for endpoints in **`api.py`**.

**Promotion / queue IDs (if any):** —

## Links

- [`docs/QUEUE_ARCHIVE.MD`](../QUEUE_ARCHIVE.MD) — **§2.2** **FB-AP-P0**
- [`docs/PER_ASSET_OPERATOR.MD`](../PER_ASSET_OPERATOR.MD) — manifest + API (partial operator doc)
- [`docs/Specs/CONTROL_PLANE.MD`](../Specs/CONTROL_PLANE.MD) — short spec (may lag **FB-AP-*** additions)
- [`control_plane/api.py`](../../control_plane/api.py) — asset routes + **`/status`**
- [`control_plane/Home.py`](../../control_plane/Home.py) — sidebar power + execution mode
