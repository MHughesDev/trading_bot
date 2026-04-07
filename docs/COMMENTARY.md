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

## Honest gaps (still “Not started” or shallow)

- **Coinbase live orders** remain unsigned / stub until CDP JWT work is done.
- **TFT** is still a **Ridge surrogate** in code; the spec says TFT — either implement PyTorch TFT or keep an explicit deviation note in `docs/` and in the issue log.
- **QuestDB** decision traces are **logged**, not yet **inserted** on every tick in the live runner.
- **Prefect + MLflow** nightly flow is still a stub file; promotion must stay manual when you turn automation on.
- **Grafana/Loki** are in compose but not fully wired to app config in code.

## How to use the issue log

`docs/ISSUE_LOG.md` uses **Not started**, **Pending**, **Completed**. Move items as you merge work. The epic stays **Pending** until everything that matters for your definition of V1 is **Completed**.

When in doubt, prefer **Pending** over **Completed** — “Completed” should mean you would defend the implementation in a production review.
