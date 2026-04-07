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

## Latest batch (queue continuation)

- **`run_decision_tick`** is the single decision+risk step for **live** and **replay** (`decision_engine/run_step.py`).
- **`RiskEngine`** checks **`feed_last_message_at`** first (before bar timestamp); **`nm_feed_stale_blocks_total`** counter.
- **Coinbase REST** retries with exponential backoff on 429/5xx.
- **`ProductMetadataCache`** + **`product_tradable`** gate in risk; **`live_service`** wires both.
- **QuestDB:** enable **`NM_QUESTDB_PERSIST_DECISION_TRACES`** to persist full JSON traces (`docs/QUESTDB_TRACES.md`).
- **Live loop:** `FeaturePipeline` + **`feature_row_from_tick`**, 60s memory **placeholder** task, SIGINT/SIGTERM stop (`docs/GRACEFUL_SHUTDOWN.md`).
- **Tests:** feed-stale risk, normalizer fixture, backtest/live parity imports.

## Honest gaps

- **Coinbase live** signed orders; **TFT** PyTorch; **Polars rolling bars** in live; **Qdrant** real embeddings in the 60s loop; **FLATTEN** with positions; **Prefect**; **Grafana/Loki** wiring.

## How to use the issue log

`docs/ISSUE_LOG.md` uses **Not started**, **Pending**, **Completed**. Move items as you merge work. The epic stays **Pending** until everything that matters for your definition of V1 is **Completed**.

When in doubt, prefer **Pending** over **Completed** — “Completed” should mean you would defend the implementation in a production review.
