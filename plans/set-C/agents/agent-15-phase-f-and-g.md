# Agent Query — Alpaca Side Inference + Typed Error Chain (Phase F + G)
## Covers Issues: #47, #64, #65
## Phase: F + G
## Estimated Effort: 3–5 days
## Prerequisites: None (both are independent of Phase A/B/C/D)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The Alpaca equity collector always sets `trade.side = Side::Unknown` because the raw trade data doesn't include a maker/taker side field and no inference logic is implemented. This is a data quality issue affecting any strategy that filters or signals based on trade side (buy-initiated vs sell-initiated flow). Separately, 50+ error construction sites across the five account adapters eagerly call `.to_string()` on the underlying error to format it into the variant payload — these should use typed `thiserror` errors with `#[from]` conversions so that error formatting is lazy (deferred to display time). Issue #65 (web scraper URL/robots caching) is included here as a follow-up to agent-08 in case that agent didn't complete the per-domain caching step.

## Codebase Context

- `crates/collectors/src/equity/alpaca_data.rs` — around lines 117–118: `side: Side::Unknown` is always set for every trade, regardless of what information is available in the message.
- `crates/execution/src/account/alpaca.rs` — around line 44+: `.map_err(|e| AccountSourceError::Http(e.to_string()))` pattern appears 50+ times. `e.to_string()` allocates a heap String at error-construction time, even when the error is later discarded.
- `crates/execution/src/account/kraken.rs` — same pattern.
- `crates/execution/src/account/kalshi.rs` — same pattern.
- `crates/execution/src/account/oanda.rs` — same pattern.
- `crates/execution/src/account/coinbase.rs` — same pattern.
- `crates/collectors/src/web/scraper.rs` — may be missing per-domain `RobotsTxt` caching if agent-08 was not completed. Issue #65 requires this caching.

The problematic error pattern in `alpaca.rs`:
```rust
// account/alpaca.rs — repeated 50+ times across 5 files
.map_err(|e| AccountSourceError::Http(e.to_string()))     // ← e.to_string() = heap alloc
.map_err(|e| AccountSourceError::Parse(e.to_string()))    // ← same
.map_err(|e| AccountSourceError::Missing(e.to_string()))  // ← same
```

The problematic side assignment:
```rust
// alpaca_data.rs ~line 117
TradePayload {
    side: Side::Unknown,   // ← always Unknown, never inferred
    // ...
}
```

## Task

### Fix #47 — Alpaca trade side inference

**Problem:** `crates/collectors/src/equity/alpaca_data.rs` (around lines 117–118): `side: Side::Unknown` is always set. Alpaca WebSocket v2 trade messages include a `taker_side` field that identifies whether the trade was buy-initiated or sell-initiated.

**Solution:** Parse the `taker_side` field from the Alpaca WS v2 trade message. Map `"B"` → `Side::Buy`, `"S"` → `Side::Sell`, absent/`""` → `Side::Unknown`. If `taker_side` is absent, apply a Lee-Ready heuristic if a recent quote (bid/ask) is available.

**Implementation steps:**

1. Check the Alpaca WebSocket v2 API documentation for the trade message schema. The `taker_side` field is present in v2 trade messages and identifies the aggressor side:
   - `"B"` — buy-initiated (buyer was the aggressor/taker)
   - `"S"` — sell-initiated (seller was the aggressor/taker)
   - `""` or absent — unknown

2. Add `taker_side` to the WS trade message struct in `alpaca_data.rs`:
   ```rust
   #[derive(serde::Deserialize)]
   struct AlpacaTrade<'a> {
       #[serde(borrow)]
       symbol: &'a str,
       #[serde(borrow, default)]
       taker_side: &'a str,   // "B", "S", or "" — default empty if absent
       price: f64,
       size: f64,
       // ...
   }
   ```
   The `#[serde(default)]` ensures the field is `""` (empty) if absent from the message.

3. In the `normalize()` function, add side inference:
   ```rust
   let side = match msg.taker_side {
       "B" | "b" => Side::Buy,
       "S" | "s" => Side::Sell,
       _ => {
           // Fall back to Lee-Ready heuristic if recent quote is available
           if let Some(quote) = recent_quote {
               let mid = (quote.bid + quote.ask) / 2.0;
               if msg.price > mid { Side::Buy }
               else if msg.price < mid { Side::Sell }
               else { Side::Unknown }
           } else {
               Side::Unknown
           }
       }
   };
   ```

4. For the Lee-Ready heuristic, the `AlpacaCollector` needs access to the most recent quote for each symbol. Add a `last_quote: HashMap<&'static str, QuoteSnapshot>` or use the `WorldState` if available in context. If no quote is available (collector startup), fall back to `Side::Unknown`.

5. Add a unit test in `crates/collectors/src/equity/alpaca_data.rs` (or a test module) that:
   - Tests side inference with `taker_side = "B"` → `Side::Buy`.
   - Tests side inference with `taker_side = "S"` → `Side::Sell`.
   - Tests Lee-Ready inference with a synthetic quote: price above midpoint → `Side::Buy`, below → `Side::Sell`.
   - Tests absence of `taker_side` field (should not panic; defaults to Unknown or Lee-Ready).

6. Verify that the changed side inference does not break any existing test that asserts `Side::Unknown` — update those tests to the correct expected side.

### Fix #64 — Typed error chain in account adapters

**Problem:** `.map_err(|e| AccountSourceError::Http(e.to_string()))` is repeated 50+ times across 5 account adapter files. `e.to_string()` is called at error-construction time, allocating a heap String even when the error is caught and handled (i.e., the String is never used). With `thiserror`'s `#[from]` conversions, the underlying error is stored without `.to_string()` and only formatted when `Display::fmt` is called (i.e., when printed to a log or returned to a user).

**Solution:** Define typed `AccountSourceError` enums in each adapter using `thiserror`. Use `#[from]` on each variant so that `?` automatically converts the underlying error type. No `.to_string()` at construction time.

**Implementation steps:**

1. Add `thiserror = "1"` to workspace `Cargo.toml`:
   ```toml
   [workspace.dependencies]
   thiserror = "1"
   ```
   Add `thiserror = { workspace = true }` to each account adapter's `Cargo.toml`:
   - `crates/execution/Cargo.toml` (or individual adapter Cargo.toml files if separate).

2. In `crates/execution/src/account/alpaca.rs`, define the typed error enum:
   ```rust
   #[derive(Debug, thiserror::Error)]
   pub enum AlpacaAccountError {
       #[error("HTTP request failed: {0}")]
       Http(#[from] reqwest::Error),

       #[error("JSON parse failed: {0}")]
       Parse(#[from] serde_json::Error),

       #[error("Missing required field: {0}")]
       MissingField(&'static str),

       #[error("Authentication failed: {status}")]
       Auth { status: reqwest::StatusCode },

       #[error("Rate limit exceeded")]
       RateLimit,
   }
   ```
   The `#[from]` attribute enables `?` conversion: `reqwest_result?` automatically converts `reqwest::Error` to `AlpacaAccountError::Http` without calling `.to_string()`.

3. Change all `.map_err(|e| AccountSourceError::Http(e.to_string()))` to just `?`:
   ```rust
   // Before:
   let resp = self.client.get(&url).send().await
       .map_err(|e| AccountSourceError::Http(e.to_string()))?;
   // After:
   let resp = self.client.get(&url).send().await?;
   // The `?` calls `AlpacaAccountError::from(reqwest::Error)` automatically via #[from]
   ```
   For `MissingField`, use the typed variant directly:
   ```rust
   // Before:
   field.ok_or_else(|| AccountSourceError::Missing("balance".to_string()))?;
   // After:
   field.ok_or(AlpacaAccountError::MissingField("balance"))?;
   // &'static str — no allocation
   ```

4. Apply the same pattern to each of the 5 adapter files:
   - `crates/execution/src/account/alpaca.rs` — `AlpacaAccountError`
   - `crates/execution/src/account/kraken.rs` — `KrakenAccountError`
   - `crates/execution/src/account/kalshi.rs` — `KalshiAccountError`
   - `crates/execution/src/account/oanda.rs` — `OandaAccountError`
   - `crates/execution/src/account/coinbase.rs` — `CoinbaseAccountError`

5. At the crate boundary (wherever these errors are collected into a single error type for the caller), add a `#[from]` on the outer error type:
   ```rust
   #[derive(Debug, thiserror::Error)]
   pub enum AccountSourceError {
       #[error(transparent)]
       Alpaca(#[from] AlpacaAccountError),
       #[error(transparent)]
       Kraken(#[from] KrakenAccountError),
       // ...
   }
   ```
   This propagates the error type without any `.to_string()`.

6. Verify that all error paths are still reachable and that error messages are still displayed correctly (run tests that exercise error paths and verify the error message format).

### Fix #65 — Web scraper URL/robots caching (follow-up to agent-08)

**Problem:** If agent-08 was not fully completed, the web scraper may still be missing per-domain `RobotsTxt` caching. Issue #65 requires the parsed `RobotsTxt` to be cached per domain so that only one parse and one HTTP fetch occur per domain per session.

**Solution:** Confirm whether agent-08's `RobotsTxt` per-domain cache is in place. If not, implement it here.

**Implementation steps:**

1. Check `crates/collectors/src/web/scraper.rs` for a `robots_cache` field on the `Scraper` struct. If present, this fix is already done (skip to acceptance test).

2. If not present, add the cache:
   ```rust
   use std::collections::HashMap;
   use std::sync::Arc;
   use std::time::{Duration, Instant};

   struct CachedRobots {
       robots: Arc<RobotsTxt>,
       fetched_at: Instant,
   }

   pub struct Scraper {
       robots_cache: HashMap<String, CachedRobots>,
       robots_ttl: Duration,   // how long to cache (default: 24 hours)
       // ... other fields ...
   }
   ```

3. In the URL-check path, check the cache before fetching:
   ```rust
   let domain = extract_domain(&url)?;
   let robots = match self.robots_cache.get(&domain) {
       Some(cached) if cached.fetched_at.elapsed() < self.robots_ttl => {
           Arc::clone(&cached.robots)
       }
       _ => {
           let body = self.fetch_robots_txt(&domain).await?;
           let robots = Arc::new(RobotsTxt::parse(&body));
           self.robots_cache.insert(domain.clone(), CachedRobots {
               robots: Arc::clone(&robots),
               fetched_at: Instant::now(),
           });
           robots
       }
   };
   ```

4. If `DashMap` is available (agent-10 added it), use `DashMap<String, Arc<RobotsTxt>>` for thread-safe caching without a lock. Expiry can be tracked with a parallel `DashMap<String, Instant>` or embedded in the value.

**Acceptance test:**

- For #47: unit tests verifying `"B"` → `Side::Buy`, `"S"` → `Side::Sell`, and Lee-Ready heuristic. Verify the test file compiles and the tests pass.
- For #64: write a test that triggers an HTTP error and verifies the returned error is `AlpacaAccountError::Http(...)` without calling `.to_string()` (check that `format!("{e}")` on the error produces a non-empty string).
- For #65: unit test that calls `is_allowed()` twice for the same domain and verifies `fetch_robots_txt` is called only once (use a mock or counter).

## Overall Acceptance Criteria
- [ ] `AlpacaTrade` WS message struct has a `taker_side: &str` field with `#[serde(default)]`
- [ ] `side` in `AlpacaTradePayload` is correctly inferred from `taker_side` for `"B"` and `"S"`
- [ ] Lee-Ready heuristic applied as fallback when `taker_side` is absent and recent quote is available
- [ ] Unit tests for side inference pass (all three cases: B, S, absent-with-quote)
- [ ] Zero `.to_string()` calls in error construction across all 5 account adapter files
- [ ] `AccountSourceError` variants use `#[from]` for automatic `?` conversion
- [ ] `thiserror = "1"` added to workspace `Cargo.toml`
- [ ] `RobotsTxt` parsed result is cached per domain in the scraper (at most one fetch + parse per domain per session)
- [ ] All account adapter tests pass (error paths still reachable and produce meaningful messages)
- [ ] `cargo test` passes

## Files to Touch
- `crates/collectors/src/equity/alpaca_data.rs` — add taker_side field; implement side inference; add unit tests
- `crates/execution/src/account/alpaca.rs` — AlpacaAccountError with thiserror + #[from]; remove all .to_string() in map_err
- `crates/execution/src/account/kraken.rs` — KrakenAccountError; same pattern
- `crates/execution/src/account/kalshi.rs` — KalshiAccountError; same pattern
- `crates/execution/src/account/oanda.rs` — OandaAccountError; same pattern
- `crates/execution/src/account/coinbase.rs` — CoinbaseAccountError; same pattern
- `crates/collectors/src/web/scraper.rs` — add robots_cache if not present from agent-08
- `Cargo.toml` — add `thiserror = "1"` to workspace dependencies if not already present
