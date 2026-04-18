# Data plane

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Ingest

- **`kraken_ws.py`** — Kraken WebSocket v1; feeds rolling bars and tick normalization
- **`kraken_rest.py`** — Kraken public REST (`/0/public/OHLC`, `/Trades`, `AssetPairs`); training uses `orchestration/real_data_bars.py`
- **`normalizers.py`** — Maps WS JSON to `TickerSnapshot`, `TradeTick`, `OrderBookLevel2Snapshot`, optional `BarEvent`
- **`product_cache.py`** — Product metadata (e.g. tradable flag) from REST
- **`news_ingest.py`** — RSS-backed headline fetch (when feeds configured), dedup, `aggregate_sentiment_for_symbols_async` with optional FinBERT (`sentiment_nlp.py`)
- **`rss_news.py`** — RSS/Atom parse helpers

## Bars and features

- **`bars/rolling.py`** — Rolling OHLCV bars per symbol (bucket size in seconds, default 1s in live)
- **`bars/aggregator.py`** — Aggregation helpers
- **`features/pipeline.py`** — Polars feature pipeline (RSI, MACD, ATR, etc.)

## Memory

- **`memory/qdrant_memory.py`** — Vector store for news context; payload includes `schema_version`
- **`memory/embeddings.py`** — Deterministic embedding helper for queries
- **`memory/retrieval_loop.py`** — Periodic retrieval per symbol in live loop

## Storage

- **`storage/questdb.py`** — `QuestDBWriter` async PostgreSQL wire to QuestDB; bars + decision traces; batched trace inserts + flush
- **`storage/schemas.py`** — Table DDL helpers
- **`storage/redis_state.py`** — Redis helpers where used

## Policy

All **market data** for the trading pipeline is sourced from **Kraken** paths above; Alpaca is **not** used for market data in the main package tree.
