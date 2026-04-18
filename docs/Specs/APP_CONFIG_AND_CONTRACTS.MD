# App configuration and contracts

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Configuration

- **Defaults:** `app/config/default.yaml`
- **Loader:** `app/config/settings.py` — Pydantic `AppSettings` with env prefix **`NM_`** and nested `__` for dicts (see `pydantic-settings`).
- **Overrides:** Environment and `.env` file; `NM_*` wins over YAML for mapped fields.
- **APEX canonical bundle (FB-CAN-003):** `app/config/canonical_config.py` — versioned **`CanonicalRuntimeConfig`** (`metadata` + per-domain bags). Optional YAML section **`apex_canonical`** in `default.yaml` merges over a **legacy projection** of flat `AppSettings` so runs can stamp **`settings.canonical`**. See [`APEX_Canonical_Configuration_Spec_v1_0.md`](../Human%20Provided%20Specs/new_specs/canonical/APEX_Canonical_Configuration_Spec_v1_0.md).
- **Risk limits in YAML (FB-CAN-022):** portfolio exposure / spread / stale thresholds are authored under **`apex_canonical.domains.risk_sizing`** (mirrored into flat `NM_RISK_*` fields). The deprecated top-level **`risk:`** key **fails fast** at load — use **`NM_RISK_*`** or **`apex_canonical`** only.

Notable groups in settings (non-exhaustive):

- **Execution:** `execution_mode` (`paper` | `live`), adapter names, position reconcile intervals
- **Market data:** `market_data_symbols`, provider string
- **Risk:** exposure, spread, stale thresholds, drawdown
- **Backtesting:** slippage, fees, solvency, optional `replay_available_cash`
- **QuestDB / Redis / Qdrant:** connection defaults, trace persistence, batching
- **Sentiment / news:** `sentiment_use_finbert`, `news_rss_feeds`, timeouts
- **Secrets:** `risk_signing_secret`, venue API keys, `control_plane_api_key` — never commit real values

## Contracts (`app/contracts/`)

Typed models used across layers:

- **`events.py`** — `BarEvent` (OHLCV + `schema_version`, validated OHLC consistency)
- **`orders.py`** — `OrderIntent`, `OrderSide`, validators (e.g. no raw news text in metadata)
- **`decisions.py`**, **`forecast.py`**, **`regime.py`**, **`risk.py`** — decision pipeline and risk mode enums
- **`audit.py`**, **`common.py`**, **`models.py`** — supporting structures

Execution adapters consume **`OrderIntent`** after risk approval, not ad-hoc dicts.
