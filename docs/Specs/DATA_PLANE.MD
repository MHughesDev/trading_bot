# Data plane

## Ingest

- **`coinbase_ws.py`** — WebSocket client; feeds rolling bars and tick normalization
- **`coinbase_rest.py`** — Advanced Trade public REST; **fallback** to Coinbase Exchange public API on 401/403 (unauthenticated candle/product calls)
- **`normalizers.py`** — Maps WS JSON to `TickerSnapshot`, `TradeTick`, `OrderBookLevel2Snapshot`, optional `BarEvent`
- **`product_cache.py`** — Product metadata (e.g. tradable flag) from REST
- **`news_ingest.py`** — RSS-backed headline fetch (when feeds configured), dedup, `aggregate_sentiment_for_symbols_async` with optional FinBERT (`sentiment_nlp.py`)
- **`rss_news.py`** — RSS/Atom parse helpers

## Bars and features

- **`bars/rolling.py`** — Rolling minute bars per symbol
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

All **market data** for the trading pipeline is sourced from **Coinbase** paths above; Alpaca is **not** used for market data in the main package tree.
