# ADR-0002: Decimal Money Newtypes — No f64

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

Financial calculations require exact decimal arithmetic. IEEE 754 floating-point (f64) is
incapable of representing many common decimal values exactly — `0.1 + 0.2 != 0.3` in binary
floating-point. For a trading platform, this means prices, sizes, and computed monetary values
can silently accumulate rounding errors, which translates directly to incorrect fills, incorrect
P&L calculations, and in the worst case incorrect risk-gate decisions.

Beyond correctness, the system must handle per-instrument precision requirements: BTC sizes are
expressed to 8 decimal places, some ERC-20 tokens to 18, equities to 2. Truncating or rounding
to a uniform precision would silently destroy real monetary precision at the boundaries of the
system (ingest, storage, display).

The compiler is the right enforcement mechanism: if a float cannot compile its way into a price
field, it cannot cause a production incident. A code review is not a reliable substitute.

## Decision

All prices and sizes throughout the platform use **`Price(Decimal)` and `Size(Decimal)` newtypes
wrapping `rust_decimal::Decimal`**. No `From<f64>` implementation is provided or derived for
either newtype. This makes it a **compile-time error** to assign a float to a price or size.

Storage mappings:
- **PostgreSQL:** `NUMERIC` columns for all price and size fields.
- **ClickHouse:** `Decimal128(scale)` columns, with per-instrument scale taken from instrument
  metadata.

Per-instrument precision (base_precision, quote_precision, tick_size, lot_size) is stored in the
`Instrument` metadata table. Normalization quantizes incoming values to the instrument's declared
precision — precision is never silently truncated.

## Rationale

The `rust_decimal` crate provides base-10 fixed-point arithmetic correct to 28 significant digits,
which is more than sufficient for all financial instruments including 18-decimal-precision tokens.
It is not f64 wrapped in a newtype — it is a genuine decimal type.

Newtypes (`Price` and `Size`) rather than bare `Decimal` provide:
1. Type-level distinction between a price and a size at compile time — passing a size where a price
   is expected is a compiler error, not a runtime mistake.
2. A clean boundary for adding methods (display formatting, validation, arithmetic with overflow
   checks) without polluting the `Decimal` type.
3. The `From<f64>` omission is only meaningful on the newtype: the compiler enforces it at every
   call site.

Using `NUMERIC` in Postgres and `Decimal128` in ClickHouse means values round-trip exactly between
wire, memory, and storage with no precision loss. Storing money as `FLOAT` or `DOUBLE` in the
database is a separate and equally serious error that this decision forecloses.

## Consequences

**Positive:**
- Float-in-price bugs are caught at compile time, not in production.
- Price and size are distinguishable types; price-as-size mixups are compile errors.
- P&L calculations, risk checks, and fill matching are all exact.
- Database storage is exact; historical data can be replayed without precision drift.
- Per-instrument precision is respected through the full pipeline.

**Negative:**
- `rust_decimal::Decimal` arithmetic is slower than f64 arithmetic. For a trading platform
  processing thousands of events per second (not millions of matrix operations), this is
  acceptable.
- Code that imports from external APIs or deserializes JSON must explicitly convert string/integer
  wire representations to `Decimal`; there is no implicit float conversion to hide this step.
- Dependencies on libraries that return f64 (e.g., some charting or statistical libraries) require
  an explicit conversion boundary.

**Neutral:**
- Developers new to the codebase will encounter compile errors when they try to write
  `Price(1.5_f64)`. This is the desired behavior and is a teachable moment, not a usability problem.
- `serde` deserialization from JSON numeric values must use `rust_decimal`'s serde support (or
  string-based deserialization) rather than f64 deserialization. This must be configured per field.

## Alternatives Considered

### Option A: f64 with careful discipline
Use `f64` everywhere and rely on developer discipline to avoid precision errors. Not chosen because
discipline is not a compiler guarantee. Float-in-price bugs are invisible at compile time and are
the kind of bug that only surfaces in production with real money. Every serious financial system
that has used floats for money has eventually paid the cost.

### Option B: i64 fixed-point (cents / satoshis)
Store prices and sizes as integer fixed-point with a known exponent. Avoids floats, but requires
every call site to track the scale factor, introduces integer overflow risk with 18-decimal tokens,
and makes multi-asset cross-precision arithmetic error-prone. `rust_decimal` is strictly better
for this domain.

### Option C: Bare `Decimal` without newtypes
Use `rust_decimal::Decimal` directly without newtypes. Prevents float errors but loses the
type-level Price/Size distinction and does not prevent `From<f64>` (which `rust_decimal` may
provide). Not chosen because the newtype pattern is the correct level of abstraction.

## References

  `Price`/`Size` newtype definitions, per-instrument precision, storage mappings
  Decimals from wire to warehouse, compiler enforcement
  crate selection
