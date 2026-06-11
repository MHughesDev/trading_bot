# Issue #010 — Build/runtime config leaves free speed unused

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | E |
| Pattern | Configuration |
| Quick Win | Yes |
| Latency Impact | ~10–30% throughput left on table |
| Location | `Cargo.toml:118-121`, `.cargo/config.toml` |

## Problem
The release profile uses thin LTO, unwinding panics, generic x86-64 codegen, and the default system allocator. These are the Rust release defaults — safe and portable, but leaving significant performance on the table. Fat LTO alone can reduce binary size and improve inlining by 10–30%.

## Root Cause
Default Rust release profile settings are conservative for portability. The trading bot targets a single known machine and should be tuned accordingly. The default allocator (system malloc/jemalloc variant) is not tuned for the allocation patterns in this codebase; mimalloc or jemalloc typically perform better for Rust async workloads.

## Implementation Plan
### Step 1 — Update [profile.release] in Cargo.toml
```toml
[profile.release]
lto = "fat"
panic = "abort"
codegen-units = 1
opt-level = 3
```

### Step 2 — Set target-cpu=native in .cargo/config.toml
```toml
[build]
rustflags = ["-C", "target-cpu=native"]
```
This enables AVX2/AVX-512 vectorization, SIMD float ops, and other native instructions available on the host CPU.

### Step 3 — Add mimalloc as global allocator in apps/platform/src/main.rs
```rust
use mimalloc::MiMalloc;
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
```
Add `mimalloc = "2"` to `apps/platform/Cargo.toml`.

### Step 4 — Relax workspace unsafe_code lint
Change workspace-level `#![forbid(unsafe_code)]` to per-crate `#![deny(unsafe_code)]`. This is required for rkyv (#2) and mimalloc. Document the change and which crates permit unsafe.

### Step 5 — Run benchmark suite to capture baseline delta
Before any other phase lands, run `cargo bench` with and without the new flags to capture the raw speedup from compiler/allocator changes alone.

## Acceptance Criteria
- [ ] `cargo build --release` uses fat LTO, panic=abort, codegen-units=1
- [ ] `target-cpu=native` in .cargo/config.toml
- [ ] mimalloc registered as global allocator in apps/platform
- [ ] Benchmark suite shows measurable throughput delta vs baseline
- [ ] CI passes with new profile (no link errors from fat LTO)

## Files to Change
- `Cargo.toml` — update [profile.release] settings
- `.cargo/config.toml` — add rustflags target-cpu=native
- `apps/platform/src/main.rs` — add mimalloc global allocator
