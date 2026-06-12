# Paper trading — internal accounts per asset class

How simulated trading works without any external trade or account API.
Everything below lives in `crates/execution/src/paper/` and runs in-process:
fills, balances, positions, P&L, and transaction history are all computed and
stored locally. No venue credentials are read anywhere on this path.

## Components

| Layer | Module | Role |
|-------|--------|------|
| Fill simulators | `clob.rs`, `broker_quote.rs`, `amm_swap.rs`, `prediction.rs` | Price a fill at the mark per market structure (spread, slippage, fees) |
| Account policy | `policy.rs` | Ad hoc account model per asset class (table below) |
| Account | `account.rs` | Cash, positions (VWAP entry), realized P&L, buying-power checks |
| Journal | `ledger.rs` | Append-only cash-movement journal per account |
| Engine | `engine.rs` | `PaperTradingEngine`: one account per asset class, mark board, order store, resting limit orders |
| Broker view | `broker.rs` | `PaperBroker` — `Broker` impl per asset class over the engine |
| Account view | `account_source.rs` | `PaperAccountSource` — `AccountSource` impl (venue id `"paper"`), credentials ignored |

## Policy table (one ad hoc model per asset class)

| Asset class | Account kind | Leverage | Multiplier | Quote | Seed cash | Fill simulator |
|-------------|--------------|---------:|-----------:|-------|----------:|----------------|
| `crypto_spot_cex` | Cash (long-only) | 1 | 1 | USD | 100 000 | CLOB |
| `equity` | Cash (long-only) | 1 | 1 | USD | 100 000 | Broker quote |
| `etf` | Cash (long-only) | 1 | 1 | USD | 100 000 | CLOB |
| `bond` | Cash (long-only) | 1 | 1 | USD | 100 000 | CLOB |
| `crypto_spot_dex` | Cash + token-balance view | 1 | 1 | USDC | 100 000 | AMM swap |
| `nft` | Cash + token-balance view | 1 | 1 | ETH | 100 | AMM swap |
| `futures_expiring` | Margin (short OK) | 10 | 1 | USD | 100 000 | CLOB |
| `perpetual_swap` | Margin (short OK, funding) | 10 | 1 | USD | 100 000 | CLOB |
| `fx` | Margin (short OK) | 30 | 1 | USD | 100 000 | CLOB |
| `option` | Cash, premium × 100 | 1 | 100 | USD | 100 000 | Broker quote |
| `prediction_market` | Binary (settles 0/1) | 1 | 1 | USD | 10 000 | Prediction |

Semantics:

- **Cash** — buys debit `qty × price × multiplier + fee` up front and reject on
  insufficient cash; sells reject beyond the held quantity. Realized P&L is
  implicit in the cash legs and tracked as a stat.
- **Margin** — long and short. Orders that increase exposure must keep
  `equity − Σ(|qty| × mark × mult / leverage) ≥ 0`; reducing exposure is always
  allowed (closing risk never blocks) and settles realized P&L to cash.
  Perps additionally support `apply_funding(instrument, rate)`.
- **Binary** — prices clamped to `[0, 1]`; `settle_binary(instrument, won)`
  pays 1 per contract on the winning side. Futures/options expiry uses
  `settle_at_price(asset_class, instrument, price)`.

## Order lifecycle

```
OrderIntent ──RiskGate──▶ PaperBroker.submit ──▶ engine.submit(asset_class, intent)
                                                   │ no mark for instrument → Rejected
                                                   │ simulate fill at mark
                                                   ├─ filled → account.apply_fill (funds/margin check)
                                                   │            └─ insufficient → Rejected (account untouched)
                                                   ├─ unfilled limit, GTC/Day → rests
                                                   └─ unfilled, IOC/FOK       → Cancelled

hot path tick ──▶ engine.on_mark(instrument, price)
                    │ update mark board
                    └─ re-simulate resting orders on that instrument;
                       fills apply to the account, funds-rejects terminalize
```

- Submits are **idempotent** on `OrderIntent.idempotency_key` — a redelivered
  intent returns the original order id without executing.
- `query_order` / `query_open_orders` / `query_positions` read the same
  internal store; terminal order history is FIFO-bounded at 50 000.
- The mark board is fed by stage 2 of the hot path
  (`apps/platform/src/hot_path.rs::stage_bar_builder`), one mark per
  instrument — fills always price off the latest tick.

## Invariants

- `sum(ledger.cash_delta) == cash` per account at all times (verified in
  `tests/paper_accounts.rs`).
- A rejected order never mutates an account; rejections still journal a
  zero-delta `rejection` entry for audit.
- Asset-class accounts are fully isolated: one engine, eleven accounts,
  no shared balances.

## What stays external

Only market data (collectors) comes from outside. Live trading remains
opt-in via per-user credentials in the `venue_credentials` table (migration
0007); the venue adapters in `execution::account` / `execution::venues` are
unused unless those credentials exist.
