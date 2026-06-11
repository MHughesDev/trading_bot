---
Type: Formal
Status: Complete
Completed: 2026-06-10
Derived From: C-055, C-056, C-058, C-068, C-073, C-086, C-087, C-088, C-089, C-090, C-092, C-093, C-105, C-114
---

# Phase 1 — Registry & Paper Engine

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the specs under [`../../specs/`](../../specs/) (especially
> [`../../specs/DATA-002-instrument-metadata.md`](../../specs/DATA-002-instrument-metadata.md) and
> [`../../specs/COMP-002-execution-and-risk-gate.md`](../../specs/COMP-002-execution-and-risk-gate.md)),
> and the existing codebase. `crates/domain` is already implemented (`Price`/`Size` newtypes,
> `EventEnvelope`, `Instrument`/`AssetClass`, `OrderIntent`/`OrderState`); read it before editing.

## Phase goal

After this phase the **backend foundation layer for the full 8-asset-class, multi-venue system
exists**: an authoritative `DataType` registry, a `SupportedVenue` enum, the final 8-member
`AssetClass` enum, Postgres seed/registry tables (`asset_class_registry`, `data_type_registry`,
`venue_credentials` with AES-256-GCM envelope encryption), the four internal paper fill simulators
plus a DEX paper wallet (as typed skeletons with a working CLOB simulator), the event-sourced ledger
tables and writer skeleton, the `AccountSource` trait skeleton, and the P&L lot schema. These are the
invariants every later phase builds on. The Alpaca paper adapter is marked for retirement (replaced
in Phase 4).

## Prerequisites

- Existing codebase compiles (`cargo check --workspace` green).
- Migrations `0001`–`0005` applied. New migrations in this phase start at `0006`.
- No decision gates — all decisions are settled (see conclusions referenced inline).

## Invariants this phase must respect

- **No `f64` on price or size** — use `domain::money::Price` / `Size`. Ledger and lot payloads carry
  `Decimal`-backed types only.
- **Internal paper only** (C-056): paper simulators never call an external venue.
- **Execution modes are `PAPER` or `LIVE_ROUTED` only** (C-058) — the enum you add must have exactly
  these two variants.
- **No risk UI** (C-114): do not add user-facing risk concepts; existing kill-switch machinery stays
  as an operator tool.
- **Credentials are never logged or returned** (C-093): the encrypt/decrypt service must expose no
  plaintext through `Debug`/`Display`/serde.
- **Append-only ledger** (C-088): ledger rows are insert-only; no UPDATE/DELETE in the writer.

---

## Tasks

### P1-T01 — `DataType` enum: authoritative data registry
- **Goal:** A single source-of-truth enum for every data primitive/derived capability, with dotted
  string serialization used as demand keys and manifest entries.
- **Files:** `crates/domain/src/data_type.rs` (new), register in `crates/domain/src/lib.rs`.
- **Context:** Per C-090 and the one principle (C-112): the `DataType` enum is the authoritative
  registry referenced by strategy manifests, collector capabilities, and demand keys. Variants cover
  at least: `MarketOhlcv` (`market.ohlcv`), `MarketTrade` (`market.trade`), `MarketQuote`
  (`market.quote`), `MarketFundingRate` (`market.funding_rate`), `MarketOpenInterest`
  (`market.open_interest`), `PredictionMarketPrice` (`prediction.price`), `DexQuote` (`dex.quote`),
  `SocialPost` (`social.post`), `WebPageSnapshot` (`web.page_snapshot`), `NewsArticle`
  (`news.article`). Serialize/deserialize via dotted string (`as_key()` / `FromStr`). The minimum
  baseline is `market.ohlcv` (C-129) — order book / DOM / tick-list types are explicitly **not**
  added.
- **Acceptance:** unit test `crates/domain/src/data_type.rs::tests` proves round-trip
  `DataType::MarketOhlcv.as_key() == "market.ohlcv"` and `"dex.quote".parse::<DataType>()` returns
  `DexQuote` for every variant; an unknown key returns a parse error.
- **Depends on:** none.

### P1-T02 — Finalize `AssetClass` to the 8 end-state classes
- **Goal:** The `AssetClass` enum has exactly the 8 final variants; `Bond` and `Nft` are removed.
- **Files:** `crates/domain/src/instrument.rs` (or wherever `AssetClass` is defined — grep for
  `enum AssetClass`).
- **Context:** Per C-089: final variants are `CryptoSpotCex`, `Equity`, `Fx`, `PredictionMarket`,
  `Option`, `CryptoSpotDex`, `PerpetualSwap`, `FuturesExpiring`. Remove `Bond`/`Nft` and fix every
  resulting match arm across the workspace (the compiler will list them). Provide a `market_structure()
  -> MarketStructure` helper mapping each class to one of `{ Clob, BrokerQuote, AmmSwap,
  PredictionBinary }` (used by paper simulator selection in P1-T05).
- **Acceptance:** `cargo check --workspace` is green after removal; a unit test asserts
  `AssetClass::CryptoSpotCex.market_structure() == MarketStructure::Clob`,
  `Equity → BrokerQuote`, `CryptoSpotDex → AmmSwap`, `PredictionMarket → PredictionBinary`.
- **Depends on:** none.

### P1-T03 — `SupportedVenue` enum
- **Goal:** A typed enum of every venue the platform integrates, with capability metadata.
- **Files:** `crates/domain/src/venue.rs` (new), register in `crates/domain/src/lib.rs`.
- **Context:** Per C-055: variants `Kraken`, `Coinbase`, `Alpaca`, `Oanda`, `Kalshi`, `Tradier`,
  `ZeroX` (0x), `Tradovate`. Each variant exposes `provides_data() -> bool`,
  `provides_execution() -> bool`, `supported_asset_classes() -> &[AssetClass]`, and a stable
  `as_str()` slug (`"kraken"`, `"coinbase"`, `"zerox"`, …). Mapping per the vision: Kraken = crypto
  data; Coinbase = crypto live execution; Alpaca = equity data + execution; OANDA = FX (demo MVP);
  Kalshi = prediction markets + perpetuals; Tradier = options; 0x = DEX swap aggregation; Tradovate =
  futures (demo first).
- **Acceptance:** unit test asserts `SupportedVenue::Kalshi.supported_asset_classes()` contains both
  `PredictionMarket` and `PerpetualSwap`; `ZeroX.provides_execution()` is true and slug round-trips.
- **Depends on:** P1-T02.

### P1-T04 — Registry + credentials migration (`0006`)
- **Goal:** Postgres tables seeding asset classes and data types, plus the encrypted credential store.
- **Files:** `migrations/0006_registries_and_credentials.sql` (new).
- **Context:** Per C-089/C-090/C-093. Create:
  - `asset_class_registry(asset_class TEXT PRIMARY KEY, display_name TEXT, market_structure TEXT,
    is_24_7 BOOLEAN)` seeded with all 8 classes.
  - `data_type_registry(data_type_key TEXT PRIMARY KEY, description TEXT)` seeded with every
    `DataType` dotted key from P1-T01.
  - `venue_credentials(id UUID PK, user_id UUID, venue TEXT, ciphertext BYTEA, nonce BYTEA,
    wrapped_dek BYTEA, key_version INT, verified_at TIMESTAMPTZ, created_at, updated_at,
    UNIQUE(user_id, venue))`. No plaintext columns. `ciphertext` is the AES-256-GCM output;
    `wrapped_dek` is the data-encryption key wrapped by the operator KEK (envelope encryption).
- **Acceptance:** migration applies cleanly on a fresh DB; a query confirms `asset_class_registry` has
  8 rows and `data_type_registry` row count equals the number of `DataType` variants. A test in
  `crates/storage/tests/registry_seed.rs` asserts both counts match the enums.
- **Depends on:** P1-T01, P1-T02.

### P1-T05 — Paper fill simulator trait + 4 typed simulators + DEX wallet (skeletons)
- **Goal:** The internal paper-execution surface: one trait, four market-structure simulators, and a
  DEX paper wallet. CLOB simulator fully implemented; the other three implemented enough to fill a
  market order deterministically (full depth/behavior is Phase 4).
- **Files:** `crates/execution/src/paper/mod.rs`, `crates/execution/src/paper/clob.rs`,
  `crates/execution/src/paper/broker_quote.rs`, `crates/execution/src/paper/amm_swap.rs`,
  `crates/execution/src/paper/prediction.rs`, `crates/execution/src/paper/wallet.rs` (all new).
- **Context:** Per C-056/C-058/C-086/C-087. Define `trait PaperFillSimulator { fn simulate_fill(&self,
  intent: &OrderIntent, mark: Price) -> PaperFill; }`. Implementations:
  - `CLOBFillSimulator` — crypto/futures/perps. Marketable fill at the 1-min bar close mark, applies a
    configurable spread/slippage in ticks. **Fully implement** including limit-order resting logic
    (limit fills only when mark crosses the limit).
  - `BrokerQuoteFillSimulator` — equities/options/FX. Fills at a synthetic NBBO derived from the mark
    plus a half-spread.
  - `AMMQuoteSwapSimulator` — DEX. Fills against a 0x firm quote (skeleton: takes a `FirmQuote` input,
    returns the quoted out-amount; real 0x wiring is Phase 4).
  - `PredictionMarketFillSimulator` — Kalshi YES/NO binary; fills at the binary price in [0,1].
  - `wallet.rs` — `DexPaperWallet` tracking simulated token balances, debiting/crediting on swap fills.
  Selection is by `AssetClass::market_structure()` (P1-T02). **No external calls** in any simulator.
- **Acceptance:** `crates/execution/tests/paper_clob.rs` proves a market buy fills at mark+slippage
  and a resting limit fills only after the mark crosses; `crates/execution/tests/paper_dispatch.rs`
  proves `market_structure → simulator` selection returns the correct type for all 8 asset classes.
- **Depends on:** P1-T02, P1-T03.

### P1-T06 — Mark Alpaca paper adapter for retirement
- **Goal:** The Alpaca paper adapter is no longer wired as the paper path; paper goes through the
  internal simulators.
- **Files:** `crates/execution/src/alpaca.rs`, `crates/execution/src/lib.rs`.
- **Context:** Per C-056. Do **not** delete `alpaca.rs` yet (Phase 4 fully removes paper usage and may
  reuse the live-data client). Add a module-level doc comment marking it deprecated for paper, and
  ensure no code path selects it for `PAPER` execution mode. The Alpaca *equity-data collector* stays
  untouched.
- **Acceptance:** a grep/review shows no `PAPER`-mode code constructs the Alpaca broker; `cargo check
  --workspace` green. Add `#[deprecated(note = "paper now uses internal simulators (C-056)")]` on the
  paper-broker constructor.
- **Depends on:** P1-T05.

### P1-T07 — Ledger event tables (`0007`)
- **Goal:** Append-only event-sourced ledger schema for every money event.
- **Files:** `migrations/0007_ledger.sql` (new).
- **Context:** Per C-088/C-068. Create `ledger_events(id UUID PK, seq BIGSERIAL, user_id UUID,
  account_mode TEXT CHECK (account_mode IN ('PAPER','LIVE')), venue TEXT, asset_class TEXT,
  instrument_id TEXT, strategy_id UUID NULL, event_type TEXT, payload JSONB, usd_value NUMERIC,
  context JSONB, occurred_at TIMESTAMPTZ, recorded_at TIMESTAMPTZ DEFAULT now())`. `event_type` covers
  `fill`, `fee`, `margin`, `funding_payment`. `payload` is the typed asset-specific body; `context`
  carries venue, strategy, before/after state, USD conversion. Add an index on
  `(user_id, account_mode, occurred_at)`. **Insert-only**: no UPDATE/DELETE permitted by the writer.
- **Acceptance:** migration applies cleanly; a test inserts one row per `event_type` and confirms `seq`
  is monotonic and `payload`/`context` round-trip as JSON.
- **Depends on:** P1-T04.

### P1-T08 — Ledger writer skeleton
- **Goal:** A typed append-only writer that records ledger events; never mutates.
- **Files:** `crates/storage/src/ledger.rs` (new), register in `crates/storage/src/lib.rs`.
- **Context:** Per C-088. `LedgerWriter::append(event: LedgerEvent) -> Result<LedgerEventId>` inserts
  one row. `LedgerEvent` is a typed enum (`Fill`, `Fee`, `Margin`, `FundingPayment`) each with an
  asset-specific payload struct using `Price`/`Size`/`Decimal` (no `f64`). The writer exposes no
  update/delete method. USD value is supplied by the caller (computed in Phase 4's rollup engine).
- **Acceptance:** `crates/storage/tests/ledger_append.rs` proves appending two events yields two rows
  with increasing `seq` and that the public API surface has no mutate method (compile-time: there is
  no such fn).
- **Depends on:** P1-T07.

### P1-T09 — `AccountSource` trait skeleton
- **Goal:** The per-user, per-venue balance/position/transaction fetch abstraction (on-demand).
- **Files:** `crates/execution/src/account_source.rs` (new), register in `crates/execution/src/lib.rs`.
- **Context:** Per C-017/C-092. `#[async_trait] trait AccountSource { async fn fetch_balances(&self,
  creds: &VenueCredentials) -> Result<Vec<Balance>>; async fn fetch_positions(...) ->
  Result<Vec<VenuePosition>>; async fn fetch_transactions(...) -> Result<Vec<VenueTransaction>>; }`.
  This is the trait + DTOs only (concrete per-venue REST adapters land in Phase 4). It fires on-demand
  when the user navigates to Dashboard — no polling loop here. Balances/positions use `Price`/`Size`.
- **Acceptance:** the trait + DTOs compile; a `MockAccountSource` test impl in
  `crates/execution/tests/account_source.rs` returns canned balances, proving the contract is usable.
- **Depends on:** P1-T03.

### P1-T10 — P&L lot schema (`0008`)
- **Goal:** Event-sourced FIFO lot tables backing the USD P&L rollup (engine itself is Phase 4).
- **Files:** `migrations/0008_pnl_lots.sql` (new).
- **Context:** Per C-073/C-105. Create `pnl_lots(id UUID PK, user_id UUID, account_mode TEXT,
  instrument_id TEXT, open_event_id UUID, open_qty NUMERIC, remaining_qty NUMERIC, open_price NUMERIC,
  open_usd_rate NUMERIC, opened_at TIMESTAMPTZ)` and `pnl_closes(id UUID PK, lot_id UUID, close_event_id
  UUID, close_qty NUMERIC, close_price NUMERIC, realized_usd NUMERIC, closed_at TIMESTAMPTZ)`. FIFO lot
  matching: a close consumes the oldest open lots first. Realized P&L recorded on close; unrealized is
  computed at query time against current mark. Win rate is position-level (% of closed positions
  profitable).
- **Acceptance:** migration applies cleanly; a test in `crates/storage/tests/pnl_schema.rs` inserts a
  lot, a partial close, and confirms `remaining_qty` and a `pnl_closes` row are consistent with FIFO.
- **Depends on:** P1-T07.

### P1-T11 — Credential encryption service (AES-256-GCM envelope)
- **Goal:** Encrypt/decrypt venue credentials with envelope encryption; verify-before-save hook point.
- **Files:** `crates/api/src/credentials/mod.rs`, `crates/api/src/credentials/crypto.rs` (new), register
  in `crates/api/src/lib.rs` or routes module.
- **Context:** Per C-093. Use `aes-gcm` crate. Flow: generate a random 256-bit **DEK**, encrypt
  plaintext credential bytes with AES-256-GCM (random 96-bit nonce), wrap the DEK with the operator
  **KEK** (from config/env `CRED_KEK`), persist `{ciphertext, nonce, wrapped_dek, key_version}` into
  `venue_credentials`. Decrypt reverses it. The plaintext struct must **not** derive `Debug`/`Display`
  in a way that leaks, and must never be serialized to logs or API responses (C-093). The actual
  health-check verification call is wired per-venue in Phase 2 (P2 health endpoints) / Phase 6 (UI);
  expose a `verify_then_store(creds, verifier)` signature that takes a verifier closure so save is
  gated on a successful check.
- **Acceptance:** `crates/api/tests/cred_crypto.rs` proves encrypt→decrypt round-trips to the original
  plaintext, that two encryptions of the same plaintext produce different ciphertext (random nonce),
  and that the plaintext type's `Debug` output contains no credential bytes (redacted).
- **Depends on:** P1-T04.

---

## Phase exit criteria
- [ ] `DataType` enum exists with dotted-key round-trip; `data_type_registry` seeded to match it.
- [ ] `AssetClass` has exactly the 8 end-state variants (no `Bond`/`Nft`); `market_structure()` maps
      each correctly; `cargo check --workspace` green.
- [ ] `SupportedVenue` enum exists with capability metadata for all 8 venues.
- [ ] Migration `0006` seeds `asset_class_registry` (8 rows) and `data_type_registry`, and creates the
      encrypted `venue_credentials` table.
- [ ] The 4 paper fill simulators + DEX wallet compile; CLOB simulator fully works (market + resting
      limit); dispatch-by-market-structure test passes; no simulator makes an external call.
- [ ] Alpaca paper adapter is marked deprecated and is unreachable from `PAPER`-mode execution.
- [ ] Ledger tables (`0007`) + insert-only `LedgerWriter` exist; append test green; no mutate method.
- [ ] `AccountSource` trait + DTOs compile with a working mock.
- [ ] P&L lot schema (`0008`) applies; FIFO partial-close test green.
- [ ] Credential encryption service round-trips with envelope encryption; redaction test green.
- [ ] `cargo check --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
      --all-features` all green.
