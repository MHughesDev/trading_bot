# ADR: Backtesting via the market_simulator SDK (Set D)

**Status:** Accepted (2026-06-13)
**Scope:** The `backtest` crate (`crates/backtest`), the `/api/backtests`
surface, the `backtest_runs` table, and the `market_simulator` git dependency.
**Supersedes the scope note:** backtesting was deliberately removed on
2026-06-10; this ADR records its re-introduction and the boundary that keeps it
contained.

---

## Context

The platform needs to replay v1.0 strategy definitions over historical bars and
report fills/PnL. A full second execution engine is not worth building: the
`market_simulator` repository (a NautilusTrader fork) already implements venue
simulation, order matching, fees, and result statistics.

Two forces are in tension:

1. **We do not want to vendor or co-develop a second large engine** inside this
   repo, nor assume a sibling checkout sitting next to `trading_bot`.
2. **The platform must own all data and all strategy semantics** — bars
   (ClickHouse), strategy interpretation (`strategy-runtime`), and indicator
   computation (`features`) are live/replay-shared and must not fork into the
   simulator.

Backtesting was previously pulled from scope (2026-06-10) precisely because an
unbounded simulator coupling is a liability. Re-introducing it is only
acceptable with an explicit, frozen boundary.

---

## Decision

**1. The simulator is consumed as an embedded SDK through one frozen surface.**
`market_simulator` exposes a `sdk` module (`nautilus-backtest::sdk`). That
module is the *only* surface the `backtest` crate may touch. The SDK takes bars
and a per-bar callback in memory and returns a result document. It is treated as
a semver-stable contract.

**2. The simulator owns no data.** Bars are loaded from this platform's
ClickHouse `market_bars`, handed to the engine in memory, and dropped when the
run ends. Nothing is persisted on the simulator side. Strategy interpretation
and indicator math run on *this* platform's pure crates (`strategy-runtime`,
`features`), so a backtest sees the same evaluation a live instance would.

**3. The simulator is a pinned git dependency, not a path dependency.**
`trading_bot` consumes `nautilus-backtest`, `nautilus-model`, and
`nautilus-core` from `https://github.com/MHughesDev/market_simulator`, pinned to
a single git revision in the root `[workspace.dependencies]`. A fresh
`trading_bot` clone builds with no sibling checkout. All three crates share one
revision so Cargo resolves a single unified nautilus tree (pinning different
revisions would build two incompatible copies). The host toolchain is pinned to
the simulator's MSRV (`rust-toolchain.toml` → 1.96.0) so the in-process build is
reproducible.

> When `market_simulator` publishes an `sdk-vX.Y.Z` tag, swap the single
> `rev = "…"` for `tag = "sdk-vX.Y.Z"` in the root manifest. A commented
> `[patch."…/market_simulator"]` block documents the local dual-development
> workflow (point the deps at a local checkout without editing the pinned rev).

**4. The data the engine receives is owned and migrated here.** The
`backtest_runs` table is created by `migrations/0011_backtest_runs.sql` (+
`0012_backtest_user_scope.sql`) and applied automatically on platform startup
(`storage::postgres::run_migrations`) — no manual migration step.

**5. Job lifecycle is best-effort and interrupted-by-design across restarts.**
See the [backtesting spec](../specs/FEAT-002-backtesting.md). Runs do not resume
after a platform restart; in-flight rows are surfaced as failed
("interrupted by platform restart"). Persisting enough to resume `Queued` jobs
is deliberately deferred until there is demand — the cost (durable phase state,
idempotent resumption) is not yet justified.

---

## Consequences

- A fresh checkout builds and backtests with **only** `trading_bot` cloned.
- The nautilus toolchain (1.96.0, edition 2024) is in play in-process; if full
  runtime separation is ever required, Phase 0-ALT (out-of-process JSON
  boundary) is the documented alternative.
- The SDK surface is a contract: changes on the simulator side that break it
  require a new pinned rev/tag and a coordinated bump here.
- Because the platform owns interpretation and data, **live/replay parity** is
  structural, not aspirational.

---

## Alternatives considered

- **Path dependency on a sibling checkout** (the pre-Set-D state): rejected —
  it forces both repos to live side by side and is not reproducible in CI or a
  fresh web session.
- **Out-of-process service (Phase 0-ALT):** a JSON/IPC boundary that also
  separates the runtime/toolchain. Heavier (serialization + Python runtime +
  IPC latency); kept as the documented escape hatch, not the default.
- **Building our own execution simulator:** rejected — large, duplicative, and
  exactly the work the SDK avoids.
