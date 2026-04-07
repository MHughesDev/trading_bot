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
