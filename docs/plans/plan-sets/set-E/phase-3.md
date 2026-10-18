# Phase 3 — Execution Venue Adapters

**Completion: 4 / 6 DONE (3.1, 3.2, 3.3, 3.6) · 3.5 PARTIAL (built to the signed request; live submit `BLOCKED:HUMAN`) · 3.4 `BLOCKED:HUMAN` — track PROD, 2026-10-03. None blocks unattended PAPER trading: only `PaperBroker` is ever constructed.**

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

### ☑ 3.1 [DONE 2026-10-03] Tradier `AccountSource` — S–M

> `account/tradier.rs`: balances, positions and history from the three REST
> endpoints, with the quirks handled (an empty collection is the string
> `"null"`; one item is a bare object; numbers arrive as numbers or strings).
> Average entry price is `cost_basis / (|qty| x multiplier)` (Tradier reports the
> total paid; OCC option symbols use 100). History events have no id, so one is
> derived from `date|type|amount|symbol` plus an occurrence index — stable
> across calls. A body that carries no cash figure is an error, not a zero
> balance. Tests: 4 unit (fixtures) + `tests/account_http.rs` (2: real HTTP
> against a local stand-in, asserting the bearer header sent and the venue's
> error body surfaced). **Unverified against a live account** (no credentials);
> history paging follows Tradier's docs.
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

### ☑ 3.2 [DONE 2026-10-03] Tradovate `AccountSource` — M

> `account/tradovate.rs`, locked decision 6: credential blob
> `username:password[:appId[:cid[:sec]]]`; the adapter performs the
> `/auth/accesstoken` exchange itself, caches the token per username, renews it
> 5 minutes before expiry, and on a 401 drops the token, exchanges again and
> retries **once**. A throttled login (`p-ticket`/`p-time`, returned as HTTP 200)
> is an error naming the wait, never an empty token. Balances from
> `account/list` + `cashBalance/getcashbalancesnapshot` (margin split out of
> available); positions from `position/list` with closed rows dropped and
> `contractId` resolved to a symbol via `contract/item` (cached); fills as
> `fill_buy`/`fill_sell` transactions, an unreadable action refused. Tests: 5
> unit + `tests/account_http.rs` (3: one exchange serves many calls, the 401
> renewal path makes exactly two exchanges and two list calls, a second 401 gives
> up after one retry, the throttle). **Unverified against a live account.**
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

### ☑ 3.3 [DONE 2026-10-03] AMM paper simulator: real 0x `/price` quotes — M

> `venues::zerox::ZeroXQuoter` (read-only: an API key and nothing that can move
> funds) calls `GET /swap/permit2/price` with `chainId`, token **addresses**
> (0x v2 does not take symbols — a default mainnet table is included), the
> `0x-version: v2` header and the exact-input amount in the sell token's real
> decimals. `ZeroXQuoter::fetch_quote` shapes the answer as a
> `FirmQuote` (`firm_quote_from_price`: effective price per base, network fee x
> a caller-supplied native-token USD price — never an invented one). The bps
> `simulate_fill` remains the offline fallback. A buy's quote-token input is
> sized `size x reference_price` because the endpoint is exact-input. Errors,
> never zero prices: no liquidity, a zero amount, no fee, an unknown token, more
> than 18 decimals. Tests: 4 unit + `tests/zerox_quote.rs` (4: wei vs 6-decimal
> USDC on the wire, headers, an unknown token, no liquidity). **Not wired into
> the paper engine's `submit`** — that call is synchronous and the engine's AMM
> class still fills from the bps model; an async quote hook there is the next
> step and belongs with whoever owns `paper/engine.rs` fills.
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

### ☑ 3.6 [DONE 2026-10-03] Retire `NotImplemented` once adapters land — S

> The `AccountSourceError::NotImplemented` variant is deleted; no adapter in
> `account/` is a stub. `tests/account_adapters.rs::no_account_adapter_is_a_stub`
> scans the directory so a stub cannot return under another spelling.
**Addresses cleanup.** `AccountSourceError::NotImplemented`
(`account_source.rs:78-79`) is the deliberate fail-loud sentinel and the
definition-of-done tracker for 3.1/3.2. Keep it until both land (it's matched in
`tests/account_source.rs`, `tests/account_adapters.rs`), then either retire it
or keep for future venues.
- **Files:** `crates/execution/src/account_source.rs`, related tests.
- **Verify:** no production adapter returns `NotImplemented`.

---

### ⛔ 3.4 [BLOCKED:HUMAN] 0x full on-chain submit + status poll — L — **real-money**

> **Not built, deliberately:** it needs a wallet signer key, a chain RPC and a
> `/security-review` of key handling, none of which exist here. What changed is
> the failure mode: `ZeroXBroker::submit` used to return `0x:{quote.to}` — the
> router contract's address, identical for every order — as a "broker order id"
> for a swap that was never broadcast (audit finding #10). It now **refuses**
> ("nothing was broadcast"), and `tests/zerox_quote.rs` pins that. The paper path
> quotes through `ZeroXQuoter` (3.3) and never reaches this adapter. To unblock:
> decide the signer's home (this crate or a dedicated one), provision a
> testnet key in secret storage, and run the review before any mainnet key.
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

### ◑ 3.5 [PARTIAL — everything up to the signed request; live submit BLOCKED:HUMAN] Coinbase live broker — L — **real-money**

> `coinbase.rs`: **ES256 JWT per request** signed with `ring` (no new crate in
> the lock — `ring`/`base64` were already there) from a CDP EC P-256 PEM key
> (`EC PRIVATE KEY` SEC1 with the `[0]` parameters field ring rejects is rebuilt
> to the PKCS#8 ring accepts; literal `\n` escapes from the JSON download are
> handled). Claims: `sub`, `iss=cdp`, `nbf`, `exp` (+120 s), `uri="METHOD
> host/path"` (no scheme, no query), `kid`, random `nonce` — so a token is bound
> to the endpoint it was minted for. `build_create_order` (market IOC; limit GTC
> / FOK / IOC-SOR; `client_order_id` = idempotency key; stops, Day expiry and
> OCO refused by name rather than approximated). `prepare_submit` /
> `prepare_cancel` return the **signed, unsent** request. `query_order` and
> `query_open_orders` run for real with the status mapping `OPEN→New`,
> `FILLED→Filled`, `CANCELLED→Cancelled`, partial by `filled_size`, side parsed
> strictly (`BUY`/`SELL`), absent fees `None`.
> **Blocked:** `submit` and `cancel` build and sign, then refuse to send
> ("BLOCKED:HUMAN"). No live key exists; locked decision 8 requires a
> `/security-review` of key handling first; migration 0121 also pins
> `orders.execution_mode = 'paper'`. Tests: 8 unit (JWT verifies under the key's
> public point and fails when tampered; SEC1 with parameters; bad keys refused
> without echoing them; order bodies; refusals; status/side) +
> `tests/coinbase_signed.rs` (3: the unsent POST's token verifies and is bound to
> the order endpoint with **zero** order/cancel requests reaching the stand-in;
> a read request's token verifies for exactly its endpoint; an unreadable open
> order fails the listing). Test keys are generated per run — none is committed.
> **Also found:** `account/coinbase.rs` authenticates with only a `CB-ACCESS-KEY`
> header, which Advanced Trade does not accept; it needs this JWT signer.
> Not done: sandbox round-trip (no sandbox credentials), `ExecRouter` wiring
> (nothing is constructed but `PaperBroker`), the ~30 req/s limiter.
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
