# ADR-0009: Append-Only Raw Event Archive as Ground Truth

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

Time-series financial data arrives late, out of order, and sometimes incorrectly. A market data feed may publish a trade with the wrong price due to a technical issue at the exchange, then publish a correction. A bar for 10:30 may be published with incomplete trade volume if trades arrive out of order, then a corrected bar is issued when the late trades arrive. A collector reconnect may replay a window of events that overlap with already-received events.

The naive response to corrections is to update the stored record in place: overwrite the incorrect bar with the correct one, update the trade record, patch the price. This approach seems clean but is catastrophically wrong for a trading platform with a backtesting requirement:

- **Reproducibility is destroyed.** If a backtest runs today against mutated history, it will produce different results than a backtest that ran last week against the original history. There is no way to reproduce what a live strategy actually saw.
- **What the strategy saw live is lost.** A strategy might have made decisions based on the original (incorrect) bar before the correction arrived. If history is rewritten, there is no way to audit whether the strategy's decision was appropriate given the information available at the time.
- **Derived stores are invalidated silently.** ClickHouse bars, Redis snapshots, and Postgres aggregates are all derived from raw events. If the raw events are mutated, the derived stores are stale in ways that cannot be detected without replaying everything from scratch.

A related question is deduplication: when a collector reconnects and replays a window of already-received events, the system must recognize duplicates without mutating history.

## Decision

**History is never mutated.** The raw normalized event archive (Parquet on object storage) is immutable and append-only. No record is updated, deleted, or overwritten.

When late data arrives that affects a previously published event:
- The original event remains in the archive, unchanged, with its original `available_time` and `revision: 0`.
- A **revision event** is emitted: a new immutable fact with `revision: 1` (or higher for subsequent revisions), the corrected payload, a new `event_id`, and a new `available_time` reflecting when the revision became available. The revision event references the original via the dedup key.
- Downstream consumers (ClickHouse `ReplacingMergeTree`, Redis cache) apply the revision by superseding the prior entry using the dedup key. The original record in the raw archive is untouched.

Deduplication of replayed/redelivered events is handled by **deterministic dedup keys** derived from the source (not random UUIDs at ingest). A redelivered event with the same dedup key as an already-stored event is recognized and discarded at the writer; no new record is written.

The raw normalized event archive is written **before any derivation** (bar building, feature computation, ClickHouse ingestion). It is the insurance policy: if any derived store is found to be wrong, it can be rebuilt from scratch by replaying the raw archive through the same builder code.

## Rationale

The append-only, immutable archive is what makes "same strategy, same result" a guarantee rather than a hope. The backtest replay engine (market_simulator) reads the raw archive and replays both the original events and their revisions at their true `available_time`s. This reproduces exactly what the live strategy saw, including whether a correction arrived before or after the strategy made a decision.

Separating the concepts of "what the data source originally said" (the original event, `revision: 0`) from "what we now know was correct" (the revision event, `revision: 1`) preserves the historical record of the system's actual knowledge at each point in time. An audit query can answer: "What did the system believe about BTC-USDT at 14:31:00?" with a precise answer.

Append-only architecture composed with deterministic dedup keys and idempotent writers means the four core data engineering properties hold: **append-only, deterministic, idempotent, validated-at-write**. These properties compose — a system built on them does not get more fragile as more asset classes are added, because every new source flows through the same hardened path.

## Consequences

**Positive:**
- Backtests are fully reproducible: running the same backtest at any future date produces the same result because the underlying raw archive is immutable.
- Live strategy audit is complete: every decision a strategy made can be explained by the events available to it at the time, without gaps from rewritten history.
- Derived stores can be rebuilt from raw if found to be wrong: ClickHouse, Redis, and Postgres aggregates are all reconstructable by replaying the archive through the same builder code.
- Late corrections are explicitly modeled as new facts (revision events) rather than silent mutations, making them visible to downstream consumers that want to know when and how data changed.

**Negative:**
- Storage grows monotonically; the raw archive is never compacted by overwriting old records. Retention policy (how long raw events are kept in object storage) must be defined (Q-8 in open questions).
- Consumers must be aware that multiple revisions of the same bar or event may exist in the archive and must apply the correct supersession logic. `ReplacingMergeTree` in ClickHouse handles this eventually; queries at the application layer may need to filter to the latest revision explicitly.
- The revision event model requires normalizers to emit revision events when late data arrives, which is a more complex normalizer design than a simple overwrite.

**Neutral:**
- The dedup key schema (`lane + instrument_id + venue_id + sequence + source` for sequenced streams, `venue_id + exchange_trade_id` for trades) must be defined and enforced consistently across all collectors. Variation in dedup key construction between collectors undermines the idempotency guarantee.
- Parquet files accumulate small files on high-volume days; the nightly compaction job rolls them into large files without altering the events' content or ordering.

## Alternatives Considered

### Option A: In-Place Updates for Corrections
When a correction arrives, update the stored record in place. The "current" record is always the most recently corrected version.

Not chosen because: in-place updates destroy reproducibility and audit capability. A backtest run against updated history will not reproduce what a live strategy saw. The correction history is lost. This is the standard failure mode of financial data stores that eventually need to be abandoned because they cannot answer "what did the system believe at time T?"

### Option B: Soft Deletes with a `deleted` Flag
Mark the original event as `deleted` and insert a replacement. Preserve "old" records for audit but exclude them from normal queries.

Not chosen because: soft deletes are operationally equivalent to mutation from the perspective of reproducibility. The "active" record has changed, and queries that do not explicitly filter for soft-deleted records will miss the original. This approach also adds complexity without adding the clarity of explicit revision events with their own `available_time`.

### Option C: Separate Correction Log (External Audit Trail)
Keep the main event store mutable but maintain a separate correction log recording what changed and when.

Not chosen because: maintaining consistency between two parallel stores (main store + correction log) is error-prone. Any time the main store is updated without a corresponding correction log entry, the audit trail is incomplete. The revision event model keeps the original and the correction in the same append-only stream, making inconsistency impossible.

## References

