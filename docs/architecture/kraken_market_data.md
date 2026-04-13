# Kraken market data (V1)

**All market data** (live WebSocket, REST training pulls) uses **Kraken** public APIs. Execution may still use Coinbase for **live orders only** when `execution.live_adapter: coinbase` — that path does not ingest market data.

## Symbol mapping

Config symbols use **BASE-QUOTE** (e.g. `BTC-USD`). Internally:

- REST `pair`: `XBTUSD`, `ETHUSD`, … (`kraken_rest_pair`)
- WebSocket `pair`: `XBT/USD`, `ETH/USD`, … (`kraken_ws_pair`)

## Bar intervals

### Live (`app.runtime.live_service`)

Ticks are rolled into OHLCV with `RollingBars` and **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`** (default **1** second).

### Offline training (`orchestration.real_data_bars`)

- If **`NM_TRAINING_DATA_GRANULARITY_SECONDS`** matches Kraken **OHLC** intervals (60, 300, 900, 1800, 3600, 14400, 10080, 21600 minutes expressed in seconds), **`/0/public/OHLC`** is used with pagination.
- If it is a **multiple of 60** but not a native OHLC step: fetch **1m** OHLC and **resample** in Polars to the target size.
- Otherwise (e.g. **1 second**): **`/0/public/Trades`** is paginated and aggregated — accurate but **slow** for long lookbacks.

Kraken OHLC returns at most ~**720** candles per request; the client walks forward with `since`.

## Environment

- `KRAKEN_REST_*` — timeout, retries (`data_plane/ingest/kraken_rest.py`)
- `KRAKEN_WS_*` — reconnect backoff (`data_plane/ingest/kraken_ws.py`)

---

*Supersedes granular Coinbase-only candle notes for the data plane.*
