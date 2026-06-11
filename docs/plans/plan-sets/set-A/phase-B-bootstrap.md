---
Type: Formal
Status: Pending
Derived From: SYS-001, ADR-0001, ADR-0003
Note: Canonical executable plans live in docs/plans/. This copy is the traceable documentation record. On any conflict, [deleted - see Phase 7]/ wins.
---

# Phase B — Bootstrap (workspace, infra, CI, Python quarantine)

> **Self-contained execution doc.** You need only: this file, [`../architecture.md`](../architecture.md),
> and the specs in [`../specs/`](../specs/). This phase produces no business logic — it produces the
> empty, correct skeleton that every later phase fills in, plus the local infra and CI that keep the
> work honest.

## Phase goal

After this phase, the repository root is a compiling Cargo **workspace** matching the top-level
layout in [`../architecture.md`](../architecture.md): every crate and app exists as an empty-but-
compiling stub, local infrastructure (NATS JetStream, Postgres, ClickHouse, Redis) starts with one
command, CI runs fmt+clippy+test on every push, and the entire old Python tree is moved to
`legacy_python/` so it remains a behavior reference without polluting the new structure.

## Prerequisites

- **Phase A complete:** the `docs/` documentation workspace exists at the repo root (artifact, ADRs,
  specs, architecture, plans, procedures, skills). This phase scaffolds the *code* skeleton alongside
  it. If `docs/` does not exist yet, stop and run Phase A first.
- Decision gate: none. (Q1/Q2/Q3 are not needed yet.)

## Invariants this phase must respect

- Do **not** delete any Python yet — only move it to `legacy_python/`. It is the parity reference
  for all later phases and is removed only in Phase 7.
- The workspace must **compile from commit one**. Every stub crate has a valid `lib.rs`/`main.rs`.
- Pin external dependency versions **once** in `[workspace.dependencies]` (see
  `[deleted - see Phase 7]/spec/09-tech-stack.md`); member crates use `.workspace = true`.

---

## Tasks

### P B-T01 — Quarantine the Python tree
- **Goal:** Move the entire current Python system into `legacy_python/` without breaking its ability
  to be read/run for reference.
- **Files/dirs:** move `app/ backtesting/ carry_sleeve/ charts/ control_plane/ data_plane/ execution/
  infra/ legacy/ mcp_server/ models/ observability/ orchestration/ packaging/ research/ risk_engine/
  services/ shared/ strategies/ tests/ training_pipeline/ scripts/ operator_packaging/
  TradingBotScript.py run_api.py pyproject.toml requirements.txt setup.* run.* doctor.*` →
  `legacy_python/`. Keep `frontend/`, `[deleted - see Phase 7]/`, `.git/`, `.github/` at root.
- **Context:** Use `git mv` so history is preserved. The `infra/docker-compose*.yml` files are a
  reference for the new root `docker-compose.yml` in P B-T05 — read them, don't reuse blindly.
- **Acceptance:** `git status` shows the moves as renames; `legacy_python/` contains the old tree;
  repo root no longer has the old Python dirs except under `legacy_python/`.
- **Depends on:** none.

### P B-T02 — Toolchain + workspace manifest
- **Goal:** Create the workspace root files so `cargo build` works on an empty skeleton.
- **Files:** `Cargo.toml` (workspace), `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`,
  `deny.toml`, `.cargo/config.toml`, `.gitignore` (add `/target`, `frontend/node_modules`, `.env`).
- **Context:** `Cargo.toml` declares `members = ["crates/*", "apps/*", "xtask"]`, `resolver = "2"`,
  a `[workspace.dependencies]` block with every crate from
  `[deleted - see Phase 7]/spec/09-tech-stack.md` pinned, `[workspace.lints]`, and tuned
  `[profile.release]`. `rust-toolchain.toml` pins a recent stable.
- **Acceptance:** `cargo build` succeeds (no members yet is fine); `cargo fmt --check` and
  `cargo clippy` run clean.
- **Depends on:** P B-T01.

### P B-T03 — Stub every library crate
- **Goal:** Create each crate under `crates/` as an empty compiling library with the module files
  enumerated in [`../architecture.md`](../architecture.md) present as empty stubs
  (`//! TODO(<phase>)` doc-comment + minimal valid Rust).
- **Files:** one `Cargo.toml` + `src/lib.rs` (+ the listed submodule files declared with `mod`/`pub
  mod` but empty) for each of: `domain config observability event-bus storage builders features
  collectors risk execution reconciliation strategy-runtime strategy-validator demand-manager
  venue-router ui-gateway api backtest mcp-server`.
- **Context:** Submodule files can be empty but must be declared so the path is reserved and the
  later phase only fills bodies. Each crate's `Cargo.toml` depends on `domain` where the dependency
  graph in `architecture.md` allows — **except `builders`/`features`, which must NOT depend on
  `storage`/`event-bus`** (purity rule).
- **Acceptance:** `cargo build` compiles all crates; `cargo tree` shows `builders`/`features` with no
  `storage`/`event-bus`/`sqlx`/`clickhouse`/`redis` edges.
- **Depends on:** P B-T02.

### P B-T04 — Stub every binary app
- **Goal:** Create each app under `apps/` as a `main.rs` that compiles and prints a startup line.
- **Files:** `Cargo.toml` + `src/main.rs` for `platform collector-crypto collector-equity mcp-server
  backtest-runner`; plus `xtask/Cargo.toml` + `src/main.rs`.
- **Context:** Each `main.rs` is wiring-only per `architecture.md`; for now just
  `fn main() { println!("<name> stub"); }`.
- **Acceptance:** `cargo run -p platform` prints the stub line; all apps build.
- **Depends on:** P B-T03.

### P B-T05 — Local infrastructure
- **Goal:** One-command local infra for NATS JetStream, Postgres, ClickHouse, Redis/Valkey.
- **Files:** root `docker-compose.yml`, `.env.example`, `config/default.toml`, `config/local.toml`,
  `config/lanes.toml`, empty `migrations/` + `clickhouse/` dirs with a `.gitkeep` or README.
- **Context:** Model service choices on `[deleted - see Phase 7]/spec/07-storage-and-replay.md`
  and `[deleted - see Phase 7]/spec/09-tech-stack.md`. `legacy_python/infra/docker-compose*.yml`
  is a reference for ports/volumes. JetStream must be enabled (durable streams).
- **Acceptance:** `docker compose up -d` brings up all four services healthy; `.env.example`
  documents every URL/credential the Rust config will read.
- **Depends on:** P B-T02.

### P B-T06 — Task runner + dev ergonomics
- **Goal:** A `justfile` with the common workflows.
- **Files:** `justfile`.
- **Context:** Targets: `dev` (compose up + run platform), `test` (workspace + integration),
  `migrate` (apply Postgres + ClickHouse DDL — no-op until Phase 1), `fmt`, `lint`, `check-money`
  (calls `cargo xtask check-money-f64`), `frontend` (vite dev).
- **Acceptance:** `just fmt`, `just lint`, `just test` all run (test may be near-empty).
- **Depends on:** P B-T04, P B-T05.

### P B-T07 — CI pipelines
- **Goal:** CI enforces fmt + clippy + test + cargo-deny on Rust, and lint/typecheck/build on the
  frontend.
- **Files:** `.github/workflows/ci.yml`, `.github/workflows/frontend.yml`,
  `.github/workflows/release.yml`.
- **Context:** `ci.yml` runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo deny check`. Cache the cargo registry/target. `frontend.yml` runs from `frontend/`.
- **Acceptance:** CI is green on the bootstrap commit.
- **Depends on:** P B-T03, P B-T06.

### P B-T08 — Root README + docs skeleton
- **Goal:** A root `README.md` explaining the new structure + quickstart. The `docs/` workspace
  already exists from Phase A — do not recreate it or add loose files inside it.
- **Files:** root `README.md` (update the stub created in Phase A if present), `tests/README.md`.
- **Context:** README points at `docs/` (the Phase A workspace) for architecture, design decisions,
  and plans. Also note that `[deleted - see Phase 7]/` remains at root as a read-only reference
  anchor and `legacy_python/` is reference-only behavior parity material.
- **Acceptance:** A new contributor can `docker compose up`, `cargo build`, `just test` from the
  README alone; README correctly points to `docs/` not to `[deleted - see Phase 7]/`.
- **Depends on:** P B-T06.

---

## Phase exit criteria

- [ ] Old Python tree lives entirely under `legacy_python/`; nothing else at root references it.
- [ ] Every crate in `crates/` and app in `apps/` from `architecture.md` exists and compiles (including `venue-router`).
- [ ] `builders`/`features` have no I/O-crate dependencies (verified via `cargo tree`).
- [ ] `docker compose up -d` yields healthy NATS, Postgres, ClickHouse, Redis.
- [ ] `just fmt`, `just lint`, `just test` succeed; CI is green.
- [ ] Root `README.md` exists and points at the `docs/` workspace (created in Phase A); `tests/README.md` exists.
- [ ] `docs/` (from Phase A) and `[deleted - see Phase 7]/` are both present at root, untouched by this phase.
