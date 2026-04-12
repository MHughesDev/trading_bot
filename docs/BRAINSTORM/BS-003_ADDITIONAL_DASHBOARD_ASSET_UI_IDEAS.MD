# Additional Dashboard & Asset Page UI ideas (beyond current queue)

**Brainstorm ID:** BS-003  
**Scope:** multi  
**Status:** superseded *(dashboard/Asset extras promoted to [`QUEUE.MD`](../QUEUE.MD) **§2.4** **FB-UX-008**–**FB-UX-014**; see [`BS-004`](BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD))*  

---

## One-liner

Extra **operator-facing** UI capabilities to consider **after or alongside** **`FB-AP-P0`** UI rows (**FB-AP-026**–**034**, **039**–**040**) — ideas **not** spelled out as distinct queue items today.

---

## Already covered elsewhere (do not duplicate as “new”)

**In [`QUEUE.MD`](../QUEUE.MD) §2.2:** symbol search API (**FB-AP-023**), chart + fills APIs (**FB-AP-024**–**025**), dashboard redesign + drop global controls (**FB-AP-026**), Asset Page routing/chart/markers (**FB-AP-027**–**029**), per-asset paper/live (**FB-AP-030**), Initialize/Start/Stop (**FB-AP-031**), slide-in active menu (**FB-AP-033**), realtime chart push (**FB-AP-034**), remove global power/mode (**FB-AP-039**–**040**).

**Implemented today:** Home P&L + positions, sidebar power + global mode, multipage Live/Routes/Models/Regimes/Logs/Emergency, observability links.

---

## Dashboard — additional ideas

| Idea | Why it helps | Backend / notes |
|------|----------------|-----------------|
| **Preflight / health strip** | One glance: signing, venue creds, execution mode, API reachability — from `GET /status` → `preflight`, `production_preflight`. | Already in API; pure UI composition. |
| **System mode + flatten state** | Clarify **RUNNING** vs **flatten requested** (`modes`) vs per-asset **Stop** (queue) so operators do not confuse global flatten with asset stop. | `GET /status` / `GET /system/mode`; may need copy help only. |
| **Initialized vs active summary** | Table: symbols **uninitialized / initialized / active** once **FB-AP-005** exists; until then, show **registry list** vs empty. | `asset_model_registry` + future state API. |
| **Init pipeline monitor** | Single place: running job **symbol**, **job_id**, step progress, link to poll **`GET /assets/init/jobs/{id}`** | Endpoints exist (**FB-AP-006**); UI not in queue as its own row. |
| **Nightly / scheduler status** | Next run, last run, last error per job (**FB-AP-035**–**037**) — “will train tonight?” without reading logs. | Needs small status API or log scrape; not a current **FB-AP-*** UI row. |
| **Risk & limits readout** | Display **effective** limits (max position, daily loss) if exposed from `risk_engine` / settings — operator trust. | May need read-only API surface. |
| **Recent activity feed** | Last N **decisions**, **fills**, or **risk blocks** (time, symbol, summary). | Requires agreed event source (QuestDB traces, ledger, new endpoint). |
| **Export** | Download **P&L summary** / **positions** as CSV for tax or review. | Client-side CSV from existing JSON responses. |
| **Watchlist / favorites** | Pin symbols for quick open of Asset Page — independent of **active** state (**FB-AP-033**). | `st.session_state` or small prefs file / API. |
| **Responsive / narrow layout** | Usable on tablet; collapsible sidebar. | Streamlit constraints; may be “good enough” or motivate future frontend. |
| **Operator audit trail** | Who toggled live, who flattened — if multi-user auth ever lands (`NM_CONTROL_PLANE_API_KEY` is not per-user today). | Longer-term; needs auth model. |
| **Dark / light theme** | Reduce eye strain for long sessions. | Streamlit theming or custom CSS. |

---

## Asset Page — additional ideas

| Idea | Why it helps | Backend / notes |
|------|----------------|-----------------|
| **Decision / trace inspector** | Panel: for a **time range** or **bar**, show last **route**, **regime**, **risk outcome** — deep link template to **QuestDB** or small API over `decision_traces`. | Partial overlap with “markers” (**FB-AP-029**) but **explainability**-focused. |
| **Model card block** | Show **forecaster / policy** paths, **last trained**, **version** from manifest — read-only. | `AssetModelManifest` + registry **GET**. |
| **Manual caution toggles** | **Pause new entries** / **reduce size** for this symbol only (if product adds “soft” flags in risk). | New contract + risk — large scope; idea only. |
| **Order / intent history** | Table of recent **OrderIntent** or venue orders for **this symbol** (paper vs live). | Needs symbol-scoped API beyond **FB-AP-025** shape. |
| **Liquidity / spread indicator** | Last bid/ask or spread from Kraken if surfaced — context before trading. | Market-data path must expose snapshot safely. |
| **Volume / secondary series** | Histogram under price; optional **volume** on chart. | Chart API may add a series column. |
| **Indicator overlays (optional)** | SMA/EMA toggles computed client-side from OHLC if API stays simple. | Pure UI math on loaded bars. |
| **Compare mode** | Two symbols side-by-side (read-only) for correlation view. | Heavy; optional power-user feature. |
| **Debug strip** | “Copy **curl** for **GET** chart” / last **request id** for support. | No secrets in copy; developer-only toggle. |
| **Backfill / data freshness** | “Data current through **ts**” and **gap** warning if registry/backfill (**FB-AP-018**–**019**) lags. | Needs canonical bar metadata API. |

---

## Cross-cutting (both surfaces)

| Idea | Notes |
|------|--------|
| **Unified global header** | Logo, environment badge (**paper** / **live** / **dev**), clock (UTC), link to **RUNBOOKS**. |
| **In-app notifications** | Toast when init completes, nightly train fails, or flatten completes — could use **st.toast** + polling. |
| **Keyboard shortcuts** | e.g. **/** focus search, **g** then **d** go dashboard — only if navigation model stabilizes (**FB-AP-027**). |
| **Role-based UI** | “Observer” read-only vs “operator” mutate — needs auth beyond shared API key. |
| **Embedded Grafana panel** | iframe single dashboard for **this** symbol — if Grafana templating supports it. |

---

## Suggested prioritization (opinion only)

1. **Preflight strip + init job monitor** — high leverage, mostly existing APIs.  
2. **Manifest / model card on Asset Page** — builds trust before advanced chart features.  
3. **Decision trace inspector** — differentiator for debugging without leaving the app.  
4. **Export + notifications** — cheap wins for operators.  
5. **Watchlist, theme, responsive** — polish once core **FB-AP-P0** UI ships.

---

## Constraints / alignment

- **Kraken** remains market data; UI must not imply Alpaca/Coinbase for quotes unless product changes.  
- **Risk is final** — any “override” or “soft pause” ideas require **`OrderIntent`** / risk design, not sidebar shortcuts.  
- No secrets in UI copy or export.

---

## Next step

- [x] **Promote** — **FB-UX-008**–**FB-UX-014** in [`QUEUE.MD`](../QUEUE.MD) §2.4 (**FB-UX-P0**); brainstorm **BS-004** holds the umbrella narrative.

**Child brainstorms:** —  

## Links

- [`QUEUE.MD`](../QUEUE.MD) §2.2 **FB-AP-P0**  
- [`BS-002_QUEUE_VS_UI_GAP_ANALYSIS.MD`](BS-002_QUEUE_VS_UI_GAP_ANALYSIS.MD)  
- [`PER_ASSET_OPERATOR.MD`](../PER_ASSET_OPERATOR.MD)
