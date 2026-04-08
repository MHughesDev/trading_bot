# Coinbase candle granularity (V1 assets)

Advanced Trade public candles use `granularity` in **seconds**. Allowed values (typical): **60**, **300**, **900**, **3600**, **21600**, **86400**.

| Bar use | `granularity_seconds` |
|--------|------------------------|
| 1m features | 60 |
| 5m | 300 |
| 15m | 900 |
| 1h | 3600 |

V1 symbols: **BTC-USD**, **ETH-USD**, **SOL-USD** — pass each as `product_id` to `CoinbaseRESTClient.get_public_candles`.

REST client retries with backoff on 429 / 5xx / transport errors (`COINBASE_REST_MAX_RETRIES`, `COINBASE_REST_RETRY_BACKOFF_BASE_SECONDS`).

## Advanced Trade 401 / 403 vs public Exchange API (Issue 35)

Some environments return **401** or **403** on Advanced Trade **GET** `/products` or `/products/{id}/candles` without a CDP JWT. `CoinbaseRESTClient` then **falls back** to the legacy **Coinbase Exchange** public API (`https://api.exchange.coinbase.com`) for the same read-only data: **GET /products** and **GET /products/{product_id}/candles**. Exchange candle rows are `[time, low, high, open, close, volume]` (Unix seconds).

For **signed** Advanced Trade access (optional), use CDP API keys and JWT — not wired in this client yet; the fallback keeps scripts and tests working without secrets.
