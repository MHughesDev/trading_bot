# ADR-0041: Decisions Are Trade Intents, Planned Into Orders by One Shared Planner

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect, on the owner's direction that a strategy says "when to buy, what to
buy, and how to buy it … everything from the trading terminal"
**Supersedes:** `ActionKind::PlaceOrder` as the only strategy action; the v1.3 exit policy as a
separate mechanism (it becomes `bracket`/`protect` intents)
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §6; QA-1, QA-5

## Context

A strategy can only place a market order, buy or sell, sized four ways. It cannot hold a target
position, scale in or out, close part of a position, rest a limit or stop order, attach a bracket,
trail a stop, or choose a time in force — although `domain::order` and the paper engine already
support limit and stop-limit orders and the terminal exposes them. Exits live in a separate
1,250-line policy type with its own tracker.

Separately, the backtester keeps its own guess of the position because its venue reports nothing
back, and the paper path re-enters on a restart because a signal that is still true looks new.

## Decision

A trading strategy's output is a set of **trade intents**: `hold`, `target {size}`, `order {side,
size, type, price, trigger, trail, tif, reduce_only, post_only, tag}`, `bracket {entry, stop,
take_profit, trailing}`, `protect {…}`, `close {fraction}`, `add`, `reduce`, `cancel {tag}`. Every
numeric field is a graph value. Order types: market, limit, stop, stop-limit, trailing stop.

One **order planner**, inside the engine and therefore identical in every environment, turns
intents plus the environment's portfolio view (position, working orders, equity, instrument
metadata) into concrete order actions: submit, cancel, replace. It rounds to lot and tick, enforces
minimum size, diffs targets against position *plus working orders*, keeps OCO bookkeeping (emulated
where a venue has none), and refuses an intent a venue cannot express, by name.

## Considered options

- **Raw orders only.** Maximum control, but every strategy re-implements diffing, and a restart or a
  re-evaluation double-enters.
- **Target positions only** (FEAT-004 Layer 1). Clean and idempotent, but cannot express a resting
  limit at a model's quantile, a bracket, or a trailing stop.
- **Intents of both kinds, one planner.** Chosen.

## Consequences

- Good: `target` is restart-safe and idempotent; re-evaluation after a crash emits nothing new.
- Good: the journal records *what was wanted* separately from *what was sent*, so a rejection or a
  rounding is attributable.
- Good: exits stop being a parallel mechanism — stops and targets are orders like any other.
- Bad: the planner is a real component with real edge cases (partial fills against a target,
  OCO across venues). It is tested as the one place those rules live.
