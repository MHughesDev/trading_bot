# ADR-0027: Free-only data sources for agent research

**Status:** Proposed
**Date:** 2026-09-11
**Deciders:** Mason Hughes

## Context

The agent needs more data reach than the platform has:
- deep history (BTC 1h had 22 bars at the 2026-09-08 check);
- news;
- fundamentals;
- macro;
- options chains;
- funding.

Paid vendors would give depth quickly, at a recurring cost. The user decided the
platform uses free sources only (BS-007 D-09).

## Decision

1. **Keep** the existing collectors as configured: Kraken, Alpaca, Tradier,
   Tradovate, OANDA, Kalshi, 0x, Reddit, web.
2. **Add only free public sources:**
   - deep backfill from free exchange and broker endpoints (Coinbase Exchange
     candles, Kraken public history, the Alpaca free plan);
   - Alpaca News and corporate actions;
   - SEC EDGAR;
   - FRED/ALFRED;
   - free public crypto options market data;
   - a free, US-accessible crypto funding/OI source;
   - Kalshi history exposure.

   Confirm each provider's current terms, rate limits and US access before building.
3. **Forward collection** where no free history exists: Tradier equity option
   chain snapshots, and L2 book snapshots.
4. **Out of scope:** paid equity options history, paid fundamentals, and purchased
   tick or L2 history. Adding any paid source requires a new ADR.

## Rationale

Keeps running costs at zero while unlocking the high-value gaps. Deep backfill
alone unblocks CPCV, walk-forward and regime work.

## Consequences

- Equity options research and order-flow (OFI) research depend on
  forward-collected data, and start slowly.
- Every report states its data limits (behaviour rule B-08).
- Backfill jobs must be rate-limit friendly and resumable.

## Alternatives Considered

- **Paid vendors now** (Polygon/Databento/OPRA, fundamentals vendors). Rejected on
  cost.
- **Free first, paid later.** Rejected: the user wants no paid dependency in the
  specs.

## References

- [DATA-005](../specs/DATA-005-data-api-v2.md)
- BS-007 [06 §6](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/06_DATA.MD)
