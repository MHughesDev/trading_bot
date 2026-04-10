# App configuration and contracts

## Configuration

- **Defaults:** `app/config/default.yaml`
- **Loader:** `app/config/settings.py` — Pydantic `AppSettings` with env prefix **`NM_`** and nested `__` for dicts (see `pydantic-settings`).
- **Overrides:** Environment and `.env` file; `NM_*` wins over YAML for mapped fields.

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
