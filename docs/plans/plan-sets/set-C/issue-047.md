# Issue #047 — Alpaca trades: side always Unknown

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | F |
| Pattern | Data modeling |
| Quick Win | No |
| Latency Impact | Information loss; no side detection |
| Location | `crates/collectors/src/equity/alpaca_data.rs:117-118` |

## Problem
Trade side is always set to `Unknown` in the Alpaca data collector. This means every equity trade event has no side information, making it impossible for the strategy runtime to correctly compute directional P&L, apply side-based filters, or make side-aware decisions for equity trades. This is a data quality issue that degrades strategy accuracy.

## Root Cause
The Alpaca market data WebSocket (`/v2/stocks/{symbol}/trades`) does not provide a "buy" or "sell" side field for exchange-reported trades. The collector sets `side = Unknown` because the raw data does not include side. However, side can be inferred.

## Implementation Plan
### Step 1 — Research Alpaca trade condition codes
Review the Alpaca trade conditions documentation. Certain condition codes (e.g., `@` for regular session, `F` for intermarket sweep) can indicate whether a trade was buyer- or seller-initiated based on tick test rules (uptick/downtick relative to previous price).

### Step 2 — Implement tick-test side inference
Apply the Lee-Ready algorithm or a simplified tick test:
- If `price > prev_price`: trade is buyer-initiated (Buy)
- If `price < prev_price`: trade is seller-initiated (Sell)
- If `price == prev_price`: use last tick direction (carry forward)

This requires maintaining a per-instrument `prev_price` state in the collector.

### Step 3 — Cross-reference with order book snapshot (optional)
If the Alpaca order book snapshot is available, compare trade price to mid-price: above mid = buy-initiated, below mid = sell-initiated. More accurate than tick test but requires an order book subscription.

### Step 4 — Document the inference method
Add a comment explaining which method is used and its limitations (tick test is imperfect; condition codes are approximate).

### Step 5 — Update tests
Add a test case: given a sequence of trades with ascending/descending prices, verify the inferred sides are correct.

## Acceptance Criteria
- [ ] Trade side is not always Unknown for Alpaca equity trades
- [ ] Tick-test inference implemented with per-instrument state
- [ ] Trade condition codes documented and used where applicable
- [ ] Inference method documented in code comments
- [ ] Unit test: trade sequence with known expected sides passes

## Files to Change
- `crates/collectors/src/equity/alpaca_data.rs` — implement tick-test side inference at lines 117-118; add per-instrument previous-price state
