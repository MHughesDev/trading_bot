# Phase 3 — Execution Venue Adapters

**Completion: 0% (0 / 6 tasks complete)**

**Goal:** Complete the account-source adapters, real DEX quoting **and on-chain
execution**, and the **live Coinbase broker**, following the existing complete
adapters as templates. **Addresses:** #7, #8, #9, #10 (full), #11.

> **Real-money paths (3.4, 3.5):** both touch live funds. Keys/secrets live in
> secure storage (env/KMS) — **never** config or repo — and each is gated by a
> `/security-review` of key handling before live/mainnet keys are enabled.

> **Templates already in-repo (read these first):** `Broker` →
> `execution/src/alpaca.rs` (full CRUD); complete venue brokers exist at
> `venues/{oanda,kalshi,tradier,tradovate}.rs`. `AccountSource` →
> `account/alpaca.rs`, `account/coinbase.rs`, `account/oanda.rs` (full).
> Credentials arrive **already decrypted** as `VenueCredentials.plaintext`
> (AES-256-GCM upstream); adapters just parse the packed `key:secret` /
> `token:account_id` form.

---

## Tasks

### ☐ 3.1 Tradier `AccountSource` — S–M — **best first win**
**Addresses #8 (CL tradier).** `parse_creds` works (`account/tradier.rs:27-40`,
`api_token:account_id`) but all three methods `Err(NotImplemented)`. Endpoints
and Bearer auth are already proven in the companion `venues/tradier.rs` broker.
- Template: `account/oanda.rs` (identical `token:account_id` + Bearer +
  `serde_json::Value` navigation).
- `GET /accounts/{id}/balances` → `Balance`; `/positions` (reuse the array walk
  from `venues/tradier.rs:250-269`) → positions; `/history` (filter by type,
  `start` from `since`) → `VenueTransaction`. Add `auth_headers`; propagate via
  `?` and `HttpStatus(resp.text())`. Handle Tradier's single-vs-array JSON quirk.
- **Files:** `crates/execution/src/account/tradier.rs`.
- **Verify:** adapter tests against recorded fixtures; no `NotImplemented`.

### ☐ 3.2 Tradovate `AccountSource` — M
**Addresses #9 (CL tradovate).** No `parse_creds` at all; methods ignore creds
and return `NotImplemented`. Tradovate uses a **token-exchange flow**
(`/auth/accesstoken`), and positions report numeric `contractId`, not symbols.
- **Locked decision 6:** the credential blob is **`username:password`** (matches
  the frontend form); the adapter performs the **`/auth/accesstoken` exchange
  in-adapter** and handles token renewal.
- Template: `account/alpaca.rs` + host/shape from `venues/tradovate.rs:36-47,
  237-273`. Add `parse_creds` (`username:password[:appId]`) + an
  `access_token()` exchange/refresh helper; `/cashBalance/...` → `Balance`;
  `/position/list` (reuse the mapping) → positions; `/fill/list` → transactions.
  Add a `contractId`→symbol resolve step.
- **Files:** `crates/execution/src/account/tradovate.rs`.
- **Verify:** adapter tests; token-expiry path exercised.

### ☐ 3.3 AMM paper simulator: real 0x `/price` quotes — M
**Addresses #11 (CL amm_swap).** The simulator works but `FirmQuote` is
caller-supplied/mocked (`paper/amm_swap.rs:25-36`). Paper-only — no signer/wallet.
- Add async `fetch_quote(intent) -> FirmQuote` calling 0x `GET /swap/permit2/price`
  (borrow the request/parse pattern from `venues/zerox.rs:80-105`): map
  `buyAmount`→`out_amount`, `price`→`effective_price`, `gas*gasPrice`→`fee_usd`.
  Use **real token decimals** (the 0.2 fix), not `10^18`. Keep the bps
  `simulate_fill` as the offline/backtest fallback.
- **Files:** `crates/execution/src/paper/amm_swap.rs`,
  `crates/execution/src/venues/zerox.rs` (shared request helper).
- **Verify:** a mocked HTTP response yields a `FirmQuote` with correct decimals;
  the offline fallback still works with no network.

### ☐ 3.6 Retire `NotImplemented` once adapters land — S
**Addresses cleanup.** `AccountSourceError::NotImplemented`
(`account_source.rs:78-79`) is the deliberate fail-loud sentinel and the
definition-of-done tracker for 3.1/3.2. Keep it until both land (it's matched in
`tests/account_source.rs`, `tests/account_adapters.rs`), then either retire it
or keep for future venues.
- **Files:** `crates/execution/src/account_source.rs`, related tests.
- **Verify:** no production adapter returns `NotImplemented`.

---

### ☐ 3.4 0x full on-chain submit + status poll — L — **real-money**
**Addresses #10 (full); locked decision 7.** Builds on the 0.2 fail-honest pass.
- `submit` signs and broadcasts the 0x swap and returns the **real tx hash**;
  `query_order` polls `eth_getTransactionReceipt` and maps confirmations →
  `New` (0 conf) / `Filled` (success) / `Rejected` (reverted). `cancel` stays an
  error (atomic swaps); `query_open_orders`/`query_positions` stay empty.
- **New component:** a signer + RPC client (wallet key + chain RPC endpoint).
  Keys come from secure secret storage (env/KMS), **never** config/repo. Decide
  whether the signer lives in this crate or a small dedicated module.
- Template: `alpaca.rs` for the query→state mapping; 0x request style already in
  `zerox.rs`.
- **Files:** `crates/execution/src/venues/zerox.rs`, new signer/RPC module,
  `crates/execution/Cargo.toml` (alloy/ethers + signing deps).
- **Verify:** against a **testnet/fork** — a broadcast swap returns a real hash
  and `query_order` reflects the receipt; a reverted tx → `Rejected`. **Gated by
  a `/security-review` of key handling before mainnet enablement.**

### ☐ 3.5 Coinbase live broker — L — **real-money**
**Addresses #7 (CL coinbase); locked decision 8.** Replace the `coinbase.rs:1`
stub with a full `Broker` against Coinbase Advanced Trade for `CryptoSpotCex`
(`venue.rs:77`, `provides_execution() == true`).
- **Auth:** ES256 JWT per request (add `jsonwebtoken`); handle clock-skew
  rejection.
- `POST /api/v3/brokerage/orders` (`client_order_id = idempotency_key`,
  `product_id`, `order_configuration.{market_market_ioc|limit_limit_gtc}`);
  batch-cancel via `POST /orders/batch_cancel`; `query_order` →
  `GET /orders/historical/{order_id}`; positions from the portfolio breakdown.
  Map `OPEN`→New, `FILLED`→Filled, `CANCELLED`→Cancelled, partial via
  `filled_size`.
- Template: `alpaca.rs` (Broker shape) + `account/coinbase.rs` (host/product
  conventions); signed-header pattern from `venues/kalshi.rs`. Register via
  `ExecRouter::register`.
- **Files:** `crates/execution/src/coinbase.rs`, `crates/execution/Cargo.toml`,
  exec-router wiring.
- **Verify:** Coinbase **sandbox** order submit/query/cancel round-trip;
  idempotency on retry (query-not-resubmit per the Broker contract,
  `broker.rs:70-71`). **Gated by a `/security-review` before live keys.** Respect
  Coinbase private-endpoint rate limits (~30 req/s).

---

## Definition of Done
Tradier and Tradovate account state queries return real balances/positions/
transactions (no `NotImplemented`); paper DEX fills use real 0x indicative
quotes with correct token decimals; the 0x adapter executes real on-chain swaps
with receipt-polled status; and the Coinbase live broker round-trips orders on
sandbox. The `NotImplemented` sentinel is retired or consciously retained. Both
real-money paths (3.4, 3.5) pass a key-handling `/security-review` before any
live/mainnet keys are enabled.
