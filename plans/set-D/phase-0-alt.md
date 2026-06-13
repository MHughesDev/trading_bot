# Phase 0-ALT — Out-of-Process Boundary (Alternative to Phase 0)

**Completion: 0% (0 / 4 tasks complete)**

**Goal:** Full separation — `market_simulator` runs as its own process/service
with its own toolchain and deploy, and `trading_bot` calls it over a stable JSON
boundary instead of linking it in-process.
**Addresses:** #1, #2 (most completely — removes the shared toolchain too)

> **Mutually exclusive with Phase 0.** Choose this only if "completely separate"
> must also mean separate runtime/deploy. Trade-off: data must be serialized
> across the boundary and the Python runtime is in play; IPC latency is added.

---

## Tasks

### ☐ 0A.1 Define the JSON contract — M
- Adopt the simulator's existing JSON surface as the contract:
  `nautilus_trader.backtest.api.run_backtest_request` and the `python -m
  nautilus_trader.backtest --raw <json>` CLI.
- Document the request shape (run_config + strategies) and the
  `BacktestResult` response shape as a versioned schema.
- **Repo:** market_simulator. **Files:** schema doc under `docs/`.

### ☐ 0A.2 Replace SDK calls with an IPC client — L
- In `trading_bot/crates/backtest/src/sim.rs`, swap the in-process `sdk::*`
  calls for a subprocess (CLI) or HTTP client that serializes bars
  (JSON or Arrow IPC) and parses the result document.
- Remove `nautilus-backtest`, `nautilus-model`, `nautilus-core` from
  `crates/backtest/Cargo.toml` entirely — the bot no longer links nautilus.
- **Repo:** trading_bot.

### ☐ 0A.3 Package the simulator as a runnable — M
- Ship market_simulator as a pinned container/CLI with a version handshake
  (`--version` / health endpoint) the bot checks on startup.
- **Repo:** market_simulator. **Files:** `Dockerfile`/entrypoint, version pin.

### ☐ 0A.4 Stream progress across the boundary — M
- Emit progress events over stdout (NDJSON) or SSE so the manager can update
  job progress/cancellation without the in-process atomics.
- Map cancellation to a kill/abort signal on the child process or request.
- **Repo:** both.

---

## Definition of Done
trading_bot builds with **zero** nautilus Cargo deps; a backtest runs end-to-end
by invoking the pinned simulator process; progress and cancellation work across
the boundary.
