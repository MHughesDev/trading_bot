# Data Sources Reference

**Status:** Active  
**Conclusions:** C-074, C-075, C-076, C-077, C-038  
**Last updated:** 2026-06-10

Single source of truth for every venue and data source the platform uses or plans to use. All Phase 1+ collector implementations derive from this registry. When a venue's free-tier terms or rate limits change, update here first.

**Constraint (C-038):** All data sources and execution venues must have a genuine free or freemium tier. No paid-only APIs. Free-tier rate limits are acceptable if they support light trading activity.

---

## Quick Reference

| Phase | Asset Class | Data Venue | Execution Venue | Free Tier | WebSocket |
|-------|-------------|-----------|-----------------|-----------|-----------|
| 1 | Crypto Spot | Kraken | Coinbase Advanced Trade | YES | YES |
| 2 | Equities / ETFs | Alpaca (IEX) | Alpaca | YES (IEX) | YES |
| 3 | FX | OANDA | OANDA | DEMO ONLY | NO |
| 4 | Prediction Markets | Kalshi | Kalshi | YES | PARTIAL |
| 5 | Options | Tradier | Tradier / Alpaca | SANDBOX | YES |
| 6 | DEX / AMM | 0x | 0x aggregator | USAGE-BASED | NO |
| 7 | Perpetuals | Kalshi | Kalshi | YES (verified users) | PARTIAL |
| 8 | Futures | Tradovate / Databento | Tradovate | DEMO / $125 CREDIT | YES |

Backup data sources: CoinGecko (crypto), Alpha Vantage (equities), 1inch (DEX).

---

## Phase 1 — Crypto Spot

### Kraken (Market Data)

| Field | Value |
|-------|-------|
| **Venue** | Kraken |
| **Asset class** | Crypto Spot |
| **Role** | Market data only (not execution) |
| **Free tier** | YES — public WebSocket endpoints, no API key required |
| **Data types** | OHLC (1m, 5m, 15m, 30m, 1h, 4h, 1d), Trades, L2 Book (depth 10/25/100/500/1000), Ticker |
| **WebSocket** | YES — Kraken WebSocket v2; multi-symbol subscriptions supported |
| **Rate limits** | ~1 req/sec (public); WebSocket channel reuse for multiple symbols |
| **Auth model** | Public endpoints need no auth; private endpoints use API Key + Secret (HMAC-SHA512) |
| **Latency** | Real-time (sub-100ms tick delivery) |
| **Health check endpoint** | `GET https://api.kraken.com/0/public/SystemStatus` (C-077) |
| **Demo / sandbox** | N/A — public data is always real |
| **Caveats** | Rate limits are per-connection; server-wide admission control required (C-091) |
| **Collector crate** | `crates/collectors/src/crypto/kraken.rs` |

### Coinbase Advanced Trade (Execution)

| Field | Value |
|-------|-------|
| **Venue** | Coinbase Advanced Trade |
| **Asset class** | Crypto Spot |
| **Role** | Execution only (market data from Kraken) |
| **Free tier** | YES — API access free; trading fees apply |
| **Data types** | OHLCV, Trades, L2 Book (via WebSocket) |
| **WebSocket** | YES — 10 req/sec public |
| **Rate limits** | 10 req/sec (public), 30 req/sec (private) |
| **Auth model** | API Key + Secret (ECDSA-signed requests) |
| **Latency** | Real-time |
| **Health check endpoint** | `GET /products` (public, C-077) |
| **Demo / sandbox** | NO permanent sandbox — paper trading is internal (`PAPER` mode) |
| **Caveats** | Requires read + write credentials for live execution (C-019). Write scope required for order placement. |
| **Broker crate** | `crates/execution/src/brokers/coinbase.rs` |

---

## Phase 2 — Equities / ETFs

### Alpaca (Data + Execution)

| Field | Value |
|-------|-------|
| **Venue** | Alpaca Markets |
| **Asset class** | Equities / ETFs |
| **Role** | Market data + execution |
| **Free tier** | YES — IEX data free with paper/live account; unlimited paper trades |
| **Data types** | OHLCV (IEX source only on free tier), Trades, Quotes, News |
| **WebSocket** | YES — 200 req/min; stream trades/quotes/bars |
| **Rate limits** | 200 req/min (REST); WebSocket connection limits per subscription tier |
| **Auth model** | API Key + Secret (header-based) |
| **Latency** | Near real-time (IEX ~15-min delay on free tier for some data) |
| **Tier 2 cost** | $9/mo (Unlimited data plan) for SIP data |
| **Health check endpoint** | `GET /v2/account` (auth required, returns account status, C-077) |
| **Demo / sandbox** | Paper trading account available (separate base URL) — but internal `PAPER` mode is used instead (C-056) |
| **Caveats** | Free tier is IEX data only — not SIP consolidated. Some extended-hours data requires paid tier. Options data requires separate Alpaca Options account. |
| **Collector crate** | `crates/collectors/src/equity/alpaca_data.rs` |
| **Broker crate** | `crates/execution/src/brokers/alpaca.rs` |

---

## Phase 3 — FX

### OANDA v20 (Data + Execution)

| Field | Value |
|-------|-------|
| **Venue** | OANDA |
| **Asset class** | FX |
| **Role** | Market data + execution |
| **Free tier** | DEMO ONLY — demo/practice account free, no permanent free live tier |
| **Data types** | OHLC bars (via `/instruments/{}/candles`), Streaming bid/ask prices |
| **WebSocket** | NO — pricing is a long-poll streaming REST endpoint (`/accounts/{}/pricing/stream`), not WebSocket |
| **Rate limits** | 30 req/sec per account; pricing stream throttled to 1 update per 250ms max |
| **Auth model** | Bearer token (Personal Access Token from OANDA dashboard) |
| **Latency** | Near real-time (pricing stream ~250ms throttle) |
| **Tier 2 cost** | Standard account (live) — spread-based, no monthly fee but requires funded account |
| **Health check endpoint** | `GET /v3/accounts/{account_id}` (Bearer token, C-077) |
| **Demo / sandbox** | YES — full demo/practice account with real-time market data |
| **Caveats** | No permanent free tier. MVP uses demo account only. Live upgrade requires funded OANDA account. No WebSocket — polling or streaming REST only. Rate-limit budget requires server-side admission control. |
| **AccountSource crate** | `crates/execution/src/account/oanda.rs` (planned) |

---

## Phase 4 — Prediction Markets

### Kalshi (Data + Execution)

| Field | Value |
|-------|-------|
| **Venue** | Kalshi |
| **Asset class** | Prediction Markets (and Phase 7 Perpetuals) |
| **Role** | Market data + execution |
| **Free tier** | YES — API access free for verified users; 0% trading fees for early adopters |
| **Data types** | Markets/Events listing, YES/NO order book, OHLC (where available), Quotes, Greeks (for perpetuals) |
| **WebSocket** | PARTIAL — WebSocket available for some market data; not all endpoints have WS equivalents |
| **Rate limits** | 10 req/sec (free tier) |
| **Auth model** | API Key (header `Authorization: Token <key>`) |
| **Latency** | Near real-time |
| **Health check endpoint** | `GET /portfolio` (API key, C-077) |
| **Demo / sandbox** | YES — demo environment available |
| **Caveats** | U.S. residents fully eligible (CFTC-regulated). Free tier is 10 req/sec — server-wide admission control required. Some WS endpoints only available at higher tiers. Kalshi also launched CFTC-approved perpetual futures (May 29, 2026) covering 13+ crypto contracts (C-078). |
| **Collector crate** | `crates/collectors/src/prediction/kalshi.rs` (planned) |
| **Broker crate** | `crates/execution/src/brokers/kalshi.rs` (planned) |

---

## Phase 5 — Options

### Tradier (Data + Execution)

| Field | Value |
|-------|-------|
| **Venue** | Tradier |
| **Asset class** | Options |
| **Role** | Market data + execution |
| **Free tier** | SANDBOX — full sandbox with real-time data is free; funded account required for live execution |
| **Data types** | Option chains, Greeks (delta, gamma, theta, vega — sourced from ORATS), IV rank, OHLCV for underlying and options, Open interest |
| **WebSocket** | YES — streaming quote/trade data |
| **Rate limits** | 120 req/min (sandbox) |
| **Auth model** | Bearer token (OAuth token from Tradier developer account) |
| **Latency** | Real-time |
| **Tier 2 cost** | $0/mo Sandbox, $0/mo live (commission-based only) |
| **Health check endpoint** | `GET /v1/accounts` (Bearer token, C-077) |
| **Demo / sandbox** | YES — sandbox with full Greek/IV data is free |
| **Caveats** | Funded account required for live order placement. Single-leg options only for MVP (C-108). Vendor Greeks are primary source; internal Black-Scholes is fallback/display only. Multi-leg spreads deferred to Phase 9+. |
| **Collector crate** | `crates/collectors/src/options/tradier.rs` (planned) |
| **Broker crate** | `crates/execution/src/brokers/tradier.rs` (planned) |

---

## Phase 6 — DEX / AMM

### 0x Swap API v2 (Data + Execution Routing)

| Field | Value |
|-------|-------|
| **Venue** | 0x Protocol |
| **Asset class** | DEX / AMM |
| **Role** | Quote aggregation + execution routing (not a custodian) |
| **Free tier** | USAGE-BASED (~5 req/sec free tier as of 2026; earlier "discontinued" note superseded) |
| **Data types** | Swap quotes (`/price` indicative, `/quote` firm with route/gas/minBuyAmount), Liquidity sources |
| **WebSocket** | NO — REST only |
| **Rate limits** | ~5 req/sec (free tier) |
| **Auth model** | API Key (header `0x-api-key`), version header `0x-version: v2` |
| **Latency** | Sub-second (quote generation ~200–500ms typical) |
| **Health check endpoint** | `GET /swap/allowance-holder/quote` (any pair, returns 200 or rate-limit error, C-077) |
| **Demo / sandbox** | NO — always real quotes; paper mode simulates fills internally (C-087) |
| **Supported chains (MVP)** | Ethereum (1), Base (8453), Arbitrum One (42161), Optimism (10), Polygon (137), Avalanche C-Chain (43114), BNB Smart Chain (56) |
| **Caveats** | Platform never holds private keys (C-093). Paper mode: use `/quote` (not `/price`) for paper fills — firm quote required. Quote freshness: reject quotes older than 15 seconds. Slippage: credit `minBuyAmount` not `buyAmount`. Gas cost is charged from paper wallet. MEV protection required for live swaps above 50bps impact threshold (C-109). |
| **Aggregator quote crate** | `crates/collectors/src/dex/zerox.rs` (planned) |

---

## Phase 7 — Perpetuals

See **Kalshi** entry in Phase 4. Same venue covers both prediction markets and CFTC-regulated perpetuals.

**Kalshi Perpetuals specifics (C-078):**
- Launched: May 29, 2026
- Contracts: 13+ (BTC, ETH, SOL, XRP, ADA, LTC, DOT, DOGE, LINK, AAVE, UNI, MATIC, AVAX)
- Data: OHLC, Quotes, Order book, Greeks
- U.S. eligible: YES (no state restrictions — CFTC-regulated)
- API: REST + WebSocket (partial) + FIX

---

## Phase 8 — Futures

### Tradovate (Data + Execution)

| Field | Value |
|-------|-------|
| **Venue** | Tradovate |
| **Asset class** | Futures |
| **Role** | Market data + execution (demo/sim first) |
| **Free tier** | DEMO — free demo account with simulated trading and real-time market data |
| **Data types** | OHLC, DOM/depth, Quotes, Charts, Histograms |
| **WebSocket** | YES — REST + WebSocket API; demo account has real-time data |
| **Rate limits** | 5,000 req/hour (demo) |
| **Auth model** | OAuth2 — `POST /auth/accesstokenrequest` (C-077) |
| **Latency** | Real-time (demo) |
| **Health check endpoint** | `POST /auth/accesstokenrequest` (returns token or 401, C-077) |
| **Demo / sandbox** | YES — full demo/simulated account with real market data |
| **Caveats** | Live futures execution is gated behind explicit enablement (C-107): requires FCM API integration, validated contract/margin rules, risk acknowledgments, and proven paper telemetry. Demo/sim ships first. |
| **Collector crate** | `crates/collectors/src/futures/tradovate.rs` (planned) |
| **Broker crate** | `crates/execution/src/brokers/tradovate.rs` (planned) |

### Databento (Historical / Tick Data)

| Field | Value |
|-------|-------|
| **Venue** | Databento |
| **Asset class** | Futures (and potentially other asset classes) |
| **Role** | Historical and tick data |
| **Free tier** | $125 credit for new users |
| **Data types** | Tick-level futures data, OHLC, historical depth |
| **WebSocket** | NO — REST / file-based delivery |
| **Health check endpoint** | N/A — credit-based; no auth health-check endpoint |
| **Caveats** | Credit-based, not permanently free. Use for Phase 8 data bootstrap / backtesting seeds only. |

---

## Backup Data Sources

| Source | Asset class | Use |
|--------|-------------|-----|
| CoinGecko API | Crypto Spot | Fallback price/metadata if Kraken is rate-limited |
| Alpha Vantage | Equities | Fallback OHLCV if Alpaca is unavailable |
| 1inch | DEX / AMM | Secondary DEX aggregator if 0x is rate-limited |

---

## Health Check Pattern (C-077)

Each `AccountSource` and `Collector` implementation must expose a `health_check()` method that hits the venue-specific endpoint listed above. For authenticated venues: fetch a lightweight account endpoint. For public endpoints: query product/system status. All return HTTP 200 on success or specific error codes on failure.

Credentials are verified before save — they are never stored in a non-verified state (C-124).

---

## Rate-Limit Budget Summary (C-074)

| Venue | Free tier limit | Notes |
|-------|----------------|-------|
| Kraken | ~1 req/sec public WS | Multi-symbol subscriptions reduce req count |
| Coinbase | 10 req/sec public, 30 req/sec private | |
| Alpaca | 200 req/min | Basic free subscription limits apply |
| OANDA | 30 req/sec, pricing stream ≤ 1 update/250ms | |
| Kalshi | 10 req/sec | Covers prediction + perpetuals |
| Tradier | 120 req/min (sandbox) | |
| 0x | ~5 req/sec | Server-wide admission required |
| Tradovate | 5,000 req/hour (demo) | |

Rate limits are a **server-wide budget** (C-091). Free-tier budgets are shared across all users. When a venue budget is exhausted, admission control rejects or degrades new demand — it does not spawn per-user duplicate collectors. Upgrade path is operator-level venue/API tier upgrade.
