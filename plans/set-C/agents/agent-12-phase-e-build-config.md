# Agent Query — Apply Full Release Profile + mimalloc + Native CPU Target
## Covers Issues: #10
## Phase: E (but do this first, before measuring any other phase's benchmarks)
## Estimated Effort: 30 minutes
## Prerequisites: None

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The release build uses near-default Cargo settings: thin LTO (or none), unwinding panics, generic x86-64 codegen (`x86-64` baseline), 16 codegen units, and the system allocator. These defaults leave 10–30% throughput on the table for free. Fat LTO enables cross-crate inlining and dead-code elimination across the entire workspace. `panic = "abort"` shrinks binary size and eliminates the unwinding machinery. `codegen-units = 1` enables the compiler to make global optimization decisions across the crate. `target-cpu=native` enables AVX2/AVX-512 and other CPU-specific instructions. `mimalloc` is significantly faster than the system allocator for the allocation patterns in this codebase (many small short-lived allocations, some large long-lived ones). This issue should be done **before** benchmarking any other fix so that all measurements reflect the final production compiler settings.

## Codebase Context

- `Cargo.toml` — workspace manifest; `[profile.release]` section controls LTO, panic, codegen-units.
- `.cargo/config.toml` — Cargo configuration; `rustflags` controls per-target compilation flags.
- `apps/platform/src/main.rs` — main platform binary; needs `#[global_allocator]`.
- `apps/collector-crypto/src/main.rs`, `apps/collector-equity/src/main.rs`, etc. — all collector binaries; each needs `#[global_allocator]`.
- `Cargo.toml` workspace lints section — may have `unsafe_code = "forbid"` which must be relaxed to `"deny"` to allow mimalloc and rkyv (from agent-02).

Current `[profile.release]` (missing optimizations):
```toml
[profile.release]
# Using defaults: lto = "thin" or off, panic = "unwind", codegen-units = 16
```

Current `.cargo/config.toml`:
```toml
# No rustflags set — compiling for generic x86-64 baseline
```

## Task

### Fix #10 — Full perf profile: fat LTO, abort panic, mimalloc, native CPU

**Problem:** The release binary is compiled without fat LTO, with unwinding panics, with 16 codegen units (preventing global optimization), for generic x86-64 (no AVX2), and using the system allocator. Each of these is a 5–15% performance gap that compounds.

**Solution:** Apply all five optimizations in a single change: fat LTO, abort panic, 1 codegen unit, native CPU target, mimalloc global allocator.

**Implementation steps:**

1. Edit `Cargo.toml` — update or add the `[profile.release]` section:
   ```toml
   [profile.release]
   opt-level = 3
   lto = "fat"           # enables cross-crate inlining; was "thin" or absent
   panic = "abort"       # eliminates unwinding machinery; was "unwind"
   codegen-units = 1     # global optimization across crate; was 16
   strip = false         # keep symbols for perf profiling and flamegraphs
   debug = 1             # minimal debug info (line numbers) for flamegraphs
   ```

   Note: `debug = 1` with `strip = false` allows `cargo flamegraph` and `perf` to work against the release binary. When shipping to production without profiling, `debug = 0` and `strip = true` can be used.

2. Edit `.cargo/config.toml` — add `rustflags` for native CPU:
   ```toml
   [build]
   rustflags = ["-C", "target-cpu=native"]
   ```
   Add the following comment immediately below:
   ```toml
   # WARNING: Binaries compiled with target-cpu=native are NOT portable across
   # CPU generations. A binary built on an AVX-512 machine will SIGILL on a
   # machine without AVX-512. Use target-cpu=x86-64-v3 for a portable baseline
   # that still enables AVX2. Only use "native" for single-machine deployments.
   ```

3. Add `mimalloc` to workspace `Cargo.toml` dependencies:
   ```toml
   [workspace.dependencies]
   mimalloc = { version = "0.1", default-features = false }
   ```
   The `default-features = false` disables secure mode (zeroing freed memory) for maximum performance in a trusted environment.

4. Change the workspace lint in `Cargo.toml` from `forbid` to `deny` for unsafe code:
   ```toml
   [workspace.lints.rust]
   unsafe_code = "deny"   # was "forbid" — relaxed to allow mimalloc and rkyv
   ```
   This change is required because `mimalloc` requires `unsafe` blocks in the `#[global_allocator]` declaration, and `rkyv` (from agent-02) requires unsafe for zero-copy access.

5. In `apps/platform/src/main.rs`, add the global allocator declaration at the top of the file (after any `#![allow(...)]` attributes):
   ```rust
   #[cfg(not(test))]   // don't override allocator in tests to avoid conflicts
   #[global_allocator]
   static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
   ```
   Add `mimalloc = { workspace = true }` to `apps/platform/Cargo.toml`.

6. Apply the same `#[global_allocator]` declaration to every collector binary:
   - `apps/collector-crypto/src/main.rs`
   - `apps/collector-dex/src/main.rs`
   - `apps/collector-equity/src/main.rs`
   - `apps/collector-futures/src/main.rs`
   - `apps/collector-fx/src/main.rs`
   - `apps/collector-kalshi/src/main.rs`
   - `apps/collector-options/src/main.rs`
   - `apps/collector-reddit/src/main.rs`
   - `apps/collector-web/src/main.rs`
   - `apps/embedder/src/main.rs`
   Add `mimalloc = { workspace = true }` to each app's `Cargo.toml`.

7. Run `cargo build --release` and verify it compiles successfully. Check binary size — fat LTO should reduce it by 10–20% compared to thin/no LTO.

8. Run `cargo test` and verify all tests pass. The `#[cfg(not(test))]` guard on `#[global_allocator]` prevents test failures due to allocator conflicts.

9. Run the existing criterion benchmarks (if any) before and after the changes. Document the throughput delta as a comment in `Cargo.toml`:
   ```toml
   # Performance delta from switching to fat-LTO + mimalloc + native:
   # tick_to_intent: was X µs, now Y µs (Z% improvement)
   # flush throughput: was A events/s, now B events/s
   ```

**Important notes:**
- `lto = "fat"` increases compile time significantly (2–5× longer). This is expected and acceptable for release builds. Debug builds are unaffected.
- If CI has a time limit, consider adding a `[profile.ci]` profile with `lto = "thin"` for CI builds while keeping `fat` for production release.
- `panic = "abort"` means `std::panic::catch_unwind` will no longer work. Audit any code that uses it. If found, replace with `Result`-based error handling or accept that the process will terminate on panic (which is the desired behavior for a trading system — fail fast, restart clean).

**Acceptance test:**
- `cargo build --release` completes without errors.
- `cargo test` passes (all tests green).
- The compiled platform binary uses mimalloc (verify by checking for `mi_` symbols: `nm target/release/platform | grep mi_` should have results).
- `objdump -d target/release/platform | grep -c "ymm\|zmm"` shows AVX2/AVX-512 instructions are present (native CPU enabled).

## Overall Acceptance Criteria
- [ ] `[profile.release]` has `lto = "fat"`, `panic = "abort"`, `codegen-units = 1`, `opt-level = 3`
- [ ] `.cargo/config.toml` has `target-cpu=native` rustflag with portability warning comment
- [ ] `mimalloc` is the `#[global_allocator]` in the platform binary
- [ ] `mimalloc` is the `#[global_allocator]` in all collector binaries (10 total)
- [ ] Workspace lint `unsafe_code` changed from `"forbid"` to `"deny"`
- [ ] `cargo build --release` succeeds
- [ ] `cargo test` passes

## Files to Touch
- `Cargo.toml` — add `[profile.release]` settings; add `mimalloc` to workspace dependencies; change unsafe_code lint
- `.cargo/config.toml` — add `rustflags = ["-C", "target-cpu=native"]`
- `apps/platform/src/main.rs` — add `#[global_allocator]`; add `#![allow(unsafe_code)]`
- `apps/platform/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-crypto/src/main.rs` — add `#[global_allocator]`
- `apps/collector-crypto/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-dex/src/main.rs` — add `#[global_allocator]`
- `apps/collector-dex/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-equity/src/main.rs` — add `#[global_allocator]`
- `apps/collector-equity/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-futures/src/main.rs` — add `#[global_allocator]`
- `apps/collector-futures/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-fx/src/main.rs` — add `#[global_allocator]`
- `apps/collector-fx/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-kalshi/src/main.rs` — add `#[global_allocator]`
- `apps/collector-kalshi/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-options/src/main.rs` — add `#[global_allocator]`
- `apps/collector-options/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-reddit/src/main.rs` — add `#[global_allocator]`
- `apps/collector-reddit/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/collector-web/src/main.rs` — add `#[global_allocator]`
- `apps/collector-web/Cargo.toml` — add `mimalloc = { workspace = true }`
- `apps/embedder/src/main.rs` — add `#[global_allocator]`
- `apps/embedder/Cargo.toml` — add `mimalloc = { workspace = true }`
