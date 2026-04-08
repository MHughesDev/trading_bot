# Commentary — where we are vs Master Spec V3

This is a **running narrative** for humans and agents: what the spec asked for, what the code does today, and what still separates “scaffold” from “production.”

## What the spec is optimizing for

You asked for a **single Coinbase truth** for prices, **Alpaca only for paper fills**, the same **decision + risk** path for paper and live, **models as signals** (not magic strings that place orders), and a **risk engine that cannot be skipped**. The repo now encodes several of those as **mechanisms** (HMAC on `OrderIntent`, metadata ban on raw news text, CI grep for Alpaca data imports) rather than only as documentation.

## What landed in the latest push

- **Routing thresholds** live in `app/config/default.yaml` under `routing` (and `NM_ROUTING_*`), consumed by `DeterministicRouteSelector`. That closes the “magic numbers only in Python” gap for route selection.
- **Execution router** validates `execution_paper_adapter` / `execution_live_adapter` against **alpaca** / **coinbase** so misconfiguration fails fast.
- **Live loop** uses **message time** from tickers/trades for `data_timestamp` and infers **spread_bps** from ticker bid/ask or L2 when possible — so stale-data and spread limits in `RiskEngine` are tied to real feed shape, not `datetime.now()` only.
- **Redis** latest bar keys get a **TTL** (`redis.bar_ttl_seconds`) so keys do not grow forever.
- **Memory loop helper** `run_memory_retrieval_loop` implements the **60s cadence** pattern against Qdrant; you still need to start it alongside the live runner and pass a real query embedding when FinBERT/news encoders exist.
- **Sentiment feature hook** `FeaturePipeline.sentiment_features()` holds the three spec slots (FinBERT, frequency, shock) until NLP is wired.
- **Streamlit** multipage shell: `control_plane/Home.py` + `pages/` for Live, Regimes, Routes, Models, Logs, Emergency — each page is thin until you bind QuestDB/Loki.

## Rolling bars + risk modes (current batch)

- **`RollingMinuteBars`**: 1m OHLCV from ticks per symbol; **`enrich_bars_last_row`** matches replay.
- **Live**: merges Polars feature row with tick **overlay** (microstructure, memory placeholders).
- **Replay**: cumulative raw OHLCV slice → same `enrich_bars_last_row` + `run_decision_tick`; tracks position from simulated trades.
- **FLATTEN_ALL** / **REDUCE_ONLY**: position-aware via `position_signed_qty` (`Decimal`).

## Latest batch (queue continuation)

- **`run_decision_tick`** is the single decision+risk step for **live** and **replay** (`decision_engine/run_step.py`).
- **`RiskEngine`** checks **`feed_last_message_at`** first (before bar timestamp); **`nm_feed_stale_blocks_total`** counter.
- **Coinbase REST** retries with exponential backoff on 429/5xx.
- **`ProductMetadataCache`** + **`product_tradable`** gate in risk; **`live_service`** wires both.
- **QuestDB:** enable **`NM_QUESTDB_PERSIST_DECISION_TRACES`** to persist full JSON traces (`docs/QUESTDB_TRACES.md`).
- **Live loop:** `FeaturePipeline` + **`feature_row_from_tick`**, 60s memory **placeholder** task, SIGINT/SIGTERM stop (`docs/GRACEFUL_SHUTDOWN.md`).
- **Tests:** feed-stale risk, normalizer fixture, backtest/live parity imports.

## Honest gaps

- **Coinbase live** signed orders; **TFT** PyTorch; **Qdrant** real embeddings in the 60s loop; **Prefect**; **Grafana/Loki** wiring.
- **Risk modes:** PAUSE / MAINTENANCE matrix tests still outstanding (Issue 16).
- **Alpaca paper:** retries + symbol helpers + optional venue reconcile are in code; optional CI integration against the paper API still not added (Issue 18).

## Latest batch (Alpaca paper + live reconcile)

- **`execution/alpaca_util.py`:** `to_alpaca_crypto_symbol` / `from_alpaca_crypto_symbol`, `redact_secrets_for_log` for safe retry logs.
- **`AlpacaPaperExecutionAdapter`:** bounded retries with exponential backoff + jitter on transient failures; `fetch_positions` maps Alpaca symbols back to Coinbase-style `BTC-USD` keys.
- **`live_service`:** optional **position reconcile** in paper mode (`NM_POSITION_RECONCILE_ENABLED` / `execution.position_reconcile_*`): startup fetch + periodic refresh from Alpaca so `position_signed_qty` matches the broker when enabled. In-memory updates after fills still apply when reconcile is off.
- **README:** documents `python -m app.runtime.live_service` and reconcile env vars.

## Latest batch (backtest simulator — Issue 23)

- **`backtesting`:** `fee_bps`, `slippage_noise_bps`, `rng_seed`, `initial_cash_usd` in config (`NM_BACKTESTING_*`).
- **`replay_decisions(..., track_portfolio=True)`** applies simulated fill prices (slippage ± optional noise with seeded `Random`), fees on notional, and updates `PortfolioTracker`; rows include `portfolio_cash`, `equity_mark`, etc.
- **New gaps logged:** Issues **32** (multi-symbol replay), **33** (risk vs cash solvency), **34** (fee/slippage doc).

## How to use the issue log

`docs/ISSUE_LOG.md` uses **Not started**, **Pending**, **Completed**. Move items as you merge work. The epic stays **Pending** until everything that matters for your definition of V1 is **Completed**.

When in doubt, prefer **Pending** over **Completed** — “Completed” should mean you would defend the implementation in a production review.
