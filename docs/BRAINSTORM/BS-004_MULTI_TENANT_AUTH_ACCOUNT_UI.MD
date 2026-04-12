# Multi-tenant users, auth, account settings, and web UI enhancements

**Brainstorm ID:** BS-004  
**Scope:** multi  
**Status:** promoted  
**Author:** —

---

## One-liner

**Signed-in users** with **standard authentication**, **per-user PnL and trading context**, an **Account** page for **Alpaca / Coinbase** API keys (**Kraken** remains the **shared market-data** path), **sign-in gating** with **redirect** for deep links, plus **dashboard/asset UI** enhancements previously listed in **BS-003** — tracked in [`QUEUE.MD`](../QUEUE.MD) **§2.4 `FB-UX-P0`**.

---

## Problem / opportunity

The control plane today assumes a **single operator** and **global** `.env` credentials. Product direction: **multiple accounts**, each seeing **only their** PnL, positions, and stored venue keys; **Kraken** stays **global** for **quotes/bars** (per [`AGENTS.md`](../../AGENTS.md)). The web app needs a **normal sign-in flow**, **route protection**, and **settings** UX (gear → Account).

---

## Vision / north star

- **Auth:** Email/password (or future OAuth); sessions; secure cookies or JWT; password hashing (e.g. bcrypt/argon2).  
- **Authorization:** Authenticated users access app pages; **unauthenticated** requests to **Dashboard**, **Asset**, etc. **redirect** to **Sign in**.  
- **Data isolation:** `user_id` scopes **PnL ledger**, **open positions cache**, **per-asset manifests/registry**, and **execution** credentials **per user**. **Kraken** REST/WS for market data uses **deployment-level** keys (`NM_*`), not per-user.  
- **Account page:** Form for **Alpaca** (paper) and **Coinbase** (live) API keys — **never** logged or committed; encrypted at rest in DB or vault-backed store.  
- **Chrome:** **Settings gear** opens Account / profile.  
- **UI extras (from BS-003):** Preflight strip, init job monitor, manifest card on Asset Page, CSV export, scheduler status, toasts, watchlist — see **FB-UX-008**–**FB-UX-014** in **QUEUE**.

---

## Workstreams

| # | Name | What it covers |
|---|------|----------------|
| 1 | **UX-AUTH** | User store, passwords, sessions, FastAPI dependencies, mutating routes |
| 2 | **UX-ACC** | Sign-in/up/out pages, redirect, gear, Account keys UI |
| 3 | **UX-DATA** | Per-user ledger, positions, registry paths, binding execution to user |
| 4 | **UX-UI** | Preflight, init monitor, model card, export, scheduler panel, notifications, watchlist |

---

## Constraints / alignment

- **Kraken-only market data** — do not route Kraken keys through per-user Account for **data plane**; execution venues (Alpaca/Coinbase) are **per user** for **orders**.  
- **Risk/signing** — `OrderIntent` path unchanged in spirit; signing secret may remain deployment-level or be scoped in a follow-up **ADR**.  
- **No secrets in git** — keys only in DB/secret store + runtime env for Kraken.  
- **Streamlit** has limited native **URL routing**; implementation may use **`st.query_params`**, **multi-page** with a **gate** in `Home.py`, or a **thin** FastAPI-served frontend later — spike if needed.

---

## Risks

- **Scope:** Per-user isolation touches **execution**, **ledger**, **registry** — coordinate with **FB-AP-P0** rows to avoid duplicate schemas.  
- **Migration:** Existing single-tenant `data/` layout may need a **one-time** migration to `data/users/{user_id}/…`.

---

## Promotion

**Promoted** to [`QUEUE.MD`](../QUEUE.MD) **§2.4 — Multi-tenant web operator UX (`FB-UX-P0`)** — rows **FB-UX-P0** (parent), **FB-UX-001**–**FB-UX-014**.

---

## Links

- [`QUEUE.MD`](../QUEUE.MD) §2.4  
- [`BS-003_ADDITIONAL_DASHBOARD_ASSET_UI_IDEAS.MD`](BS-003_ADDITIONAL_DASHBOARD_ASSET_UI_IDEAS.MD) — UI ideas partially absorbed into **FB-UX-008**–**014**  
- [`AGENTS.md`](../../AGENTS.md)
