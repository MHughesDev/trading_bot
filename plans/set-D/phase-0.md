# Phase 0 — Decouple the Repos

**Completion: 75% (6 / 8 — 0.3–0.8 done; 0.1 freeze + 0.2 tag are market_simulator-repo actions, substituted here by pinning an immutable git `rev` + ADR-0014)**

**Goal:** `market_simulator` becomes a standalone GitHub repository that
`trading_bot` consumes as a **pinned git dependency** through the frozen `sdk`
surface — no sibling-checkout assumption, reproducible builds.
**Addresses:** #1, #2, #20, #25, #26

---

## Tasks

### ☐ 0.1 Freeze & document the SDK as the contract — S
The `sdk` module is the only surface consumers may touch; make that explicit.
- Add a stability note to `crates/backtest/src/sdk.rs` (semver-stable public API).
- Re-export the surface at the crate root in `crates/backtest/src/lib.rs`
  (`pub use sdk::{…}`) so the import path is stable if the module moves.
- **Repo:** market_simulator. **Files:** `crates/backtest/src/sdk.rs`, `lib.rs`.
- **Verify:** `cargo +1.96.0 test -p nautilus-backtest --lib sdk`.

### ☐ 0.2 Tag the SDK release — S
- `git tag sdk-v0.1.0 <commit>` on the simulator branch (works pre-merge —
  the tag only needs to be reachable), `git push origin sdk-v0.1.0`.
- Record tag ↔ commit in the simulator PR body.
- **Verify:** `git ls-remote --tags origin` shows `sdk-v0.1.0`.

### ☑ 0.3 Centralize the nautilus deps on one git source — M
A single source/rev keeps the three crates unified (no duplicate builds).
- In `trading_bot/Cargo.toml` `[workspace.dependencies]`:
  ```toml
  nautilus-backtest = { git = "https://github.com/MHughesDev/market_simulator", tag = "sdk-v0.1.0" }
  nautilus-model    = { git = "https://github.com/MHughesDev/market_simulator", tag = "sdk-v0.1.0" }
  nautilus-core     = { git = "https://github.com/MHughesDev/market_simulator", tag = "sdk-v0.1.0" }
  ```
- In `crates/backtest/Cargo.toml`, replace the three `{ path = "../../../market_simulator/..." }`
  lines with `{ workspace = true }`; delete the "sibling checkout" header comment.
- **Why one source:** `nautilus-backtest` pulls `nautilus-model`/`nautilus-core`
  internally via *its* workspace; the bot's direct deps must resolve to the same
  git rev or Cargo builds two copies. The shared tag guarantees unification.
- **Repo:** trading_bot. **Files:** `Cargo.toml`, `crates/backtest/Cargo.toml`.

### ☑ 0.4 Pin the toolchain in-repo — S
- Set `trading_bot/rust-toolchain.toml` to `channel = "1.96.0"` (match the
  simulator MSRV at the pinned tag) so CI/fresh checkouts don't drift.
- **Files:** `rust-toolchain.toml`.

### ☑ 0.5 Make the build fetch the private repo — M
- Configure cargo git auth for this environment: `CARGO_NET_GIT_FETCH_WITH_CLI=true`
  plus the existing git proxy/credentials, or a `.cargo/config.toml` `[source]`
  replacement pointing the GitHub URL at the proxy.
- Add it to the SessionStart hook / setup script so web sessions and CI build.
- **Files:** `.cargo/config.toml`, SessionStart hook.
- **Verify:** clean fetch into a fresh `target/`.

### ☑ 0.6 Prove independence — S
- Temporarily rename `/home/user/market_simulator` → `_msim_hidden`, then
  `cargo build -p platform` (must build purely from the git dep).
- Commit `Cargo.lock`.
- **Verify:** `cargo build -p platform` green with the sibling dir gone;
  `cargo test -p backtest`.

### ☑ 0.7 Document the local dual-dev workflow — S
- Add a commented `[patch."https://github.com/MHughesDev/market_simulator"]`
  example in `trading_bot/Cargo.toml` so a dev hacking on both can point back to
  a local path without editing the real deps.
- Note it in `crates/backtest` module docs / README.

### ☑ 0.8 Reconcile scope + write the ADR — S
- Backtesting was deliberately removed on 2026-06-10. Confirm re-introduction is
  intended; write an ADR recording the decision and the "simulator-as-SDK, owns
  no data" boundary. Note the manual-migration requirement here too (→ 2.3).
- **Files:** `docs/adr/00NN-backtesting-via-market-simulator-sdk.md`.

---

## Definition of Done
`cargo build -p platform` and `cargo test -p backtest` pass with the
market_simulator directory absent; `Cargo.lock` committed; ADR merged; both PRs
updated.
