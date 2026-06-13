# Phase 3 — Execution Venue Adapters

**Completion: 0% (0 / 4 tasks complete; 2 deferred-by-design)**

**Goal:** Complete the read-only account-source adapters and real DEX quoting,
following the existing complete adapters as templates. **Addresses:** #8, #9,
#11 (+ #10 full, #7 — deferred).

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

## Deferred-by-design (tracked, not scheduled here)

### ⏸ 3.4 0x full on-chain submit + status poll — L
**Addresses #10 (full).** The safe bug-fix pass is **0.2**. Real `submit`
broadcasting a tx + `query_order` polling `eth_getTransactionReceipt` needs a
**signer wallet + RPC component that does not exist in this crate** — and the
code comment (`zerox.rs:56-59`) already defers broadcast to an external signer.
**Locked decision 7: quote-only + external signer** — no private keys/broadcast
in this crate; the full on-chain submit/poll is a cross-repo signer/RPC
component. **No live order broadcast.** Deferred.

### ⏸ 3.5 Coinbase live broker — L
**Addresses #7 (CL coinbase, HIGH/post-Phase 6).** Empty stub
(`coinbase.rs:1`). Needs Coinbase Advanced Trade **ES256 JWT per-request
signing** (new `jsonwebtoken` dep) and is a live-money path. **Locked decision 8:
out of scope** — no ES256 JWT signing this cycle, **no live trading**. Schedule
separately when crypto live execution is in scope.

---

## Definition of Done
Tradier and Tradovate account state queries return real balances/positions/
transactions (no `NotImplemented`); paper DEX fills use real 0x indicative
quotes with correct token decimals; the `NotImplemented` sentinel is retired or
consciously retained. 0x on-chain execution and Coinbase live trading remain
explicitly deferred with their blockers recorded.
