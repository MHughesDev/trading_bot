# Track rules — for every agent working the open-task backlog (2026-10-03)

Several agents and one other Claude session ("Agent handoff continuation") are editing this one
working tree at the same time. Read this whole file before you touch anything.

## Absolute rules

1. **No commits.** Don't commit, stash, reset, checkout or revert. Leave all your work in the tree.
2. **No worktrees.** Everything happens in this tree.
3. **Stay inside your territory.** The orchestrator's brief lists the files you own. If you need a
   line in a file outside it, stop at that point and put the exact line in your report. Never edit
   it yourself.
4. **Never touch the following:**
   - `docs/specs/README.MD`. It is the user's file.
   - Any file that was already dirty when the session started and isn't in your territory:
     `crates/collectors/src/crypto/kraken.rs`, `crates/domain/src/strategy_graph/**`,
     `crates/strategy-engine/src/compile.rs`, `crates/strategy-lang/**`,
     `crates/api/src/routes/strategies*.rs`, `crates/mcp-server/src/{lib.rs,spec.rs,tools/mod.rs,tools/strategy_v3.rs}`,
     `crates/mcp-server/docs/authoring_guide.md`, `crates/backtest/src/models.rs`,
     `docs/specs/FEAT-007-unified-strategy-engine.md`.
   - **All of Set U** (every U-x.y) and the other session's whole territory:
     `crates/strategy-engine/**`, `crates/backtest/src/{models.rs,runner.rs,portfolio.rs,selection_run.rs,env/**}`,
     `crates/api/src/routes/{strategies.rs,strategies_v3.rs,selections.rs,deployments.rs}`,
     `crates/mcp-server/src/tools/{strategy_v3.rs,selections.rs}`,
     `apps/platform/src/{allocator.rs,live_runner.rs,live_deployments.rs}`, `docs/agent/slv3-guide.md`,
     `frontend/src/components/strategy/**`, `docs/plans/plan-sets/set-U/**`.
     That session commits its own phases and stages only its own hunks. So when you edit a
     shared file, add a separate block. Never interleave with or reformat its lines.
5. **Shared files** belong to the orchestrator: `Cargo.toml`, `Cargo.lock`,
   `apps/platform/src/main.rs`, `crates/api/src/lib.rs`, `crates/api/src/routes/mod.rs`,
   `crates/mcp-server/src/taxonomy.rs`, `crates/mcp-server/src/spec.rs`, `frontend/src/App.tsx`,
   `docs/plans/PLAN.md`, `docs/plans/OPEN-TASKS.md`. Report the exact edit you need (file, anchor
   line, text to insert) and the orchestrator applies it. **Exception:** a crate's own
   `Cargo.toml` is yours if the crate is in your territory.
6. **Migrations:** use only the numbers your brief allocates. Write them LF-only (see
   `.gitattributes`). Every new schema or table must GRANT to `app_role` in the same migration.
   Owner-connected tests can't see a missing grant, so add a `SET LOCAL ROLE app_role` check
   (pattern: `crates/backtest/tests/pg_universe.rs`).

## Building and testing

- Run `cargo ... -p <your crate>` and nothing broader. **Never run `cargo test --workspace`.**
  **Update 2026-10-04:** `.cargo/config.toml` now sets jobs = 6 and links with rust-lld (it was
  jobs = 2 because MSVC link.exe exhausted the page file). Don't pass `-j` higher than 6, and keep
  using the shared target dir. The first build after the change recompiles everything once.
- Capture cargo's real exit code by redirecting to a file and then running `echo EXIT=$?`. Never
  pipe cargo into `tail`.
- Check free disk space (`Get-PSDrive C`) before a big build. If `target/debug/incremental` grows
  huge, tell the orchestrator. Don't delete it yourself.
- Postgres for tests: a native PG 17 runs at `127.0.0.1:5433` as `postgres`/`postgres`. Validate a
  migration in a scratch database (create it, apply its dependencies, apply yours, drop it).
  **5432 is NOT the platform DB**, so never trust it. pgvector isn't installed on 5433.
- Frontend: `npx tsc -b --noEmit`, `npx vitest run <paths>`, `npm run lint` (must stay at 0).
  Never add a client call to a route the server doesn't serve: `every_client_path_is_served`
  fails the build.
- Don't start the platform binary. It starts live collectors.

## How the work is judged

- Close each task against its **acceptance clauses** in the phase file. Code merely existing
  doesn't close a task.
- The dominant defect in this repo is **built but unreached**: complete, tested code with no
  caller. A task isn't done until production code reaches it. Before you build something, check
  whether it already exists and is simply unwired. Wiring it is often the whole job.
- Authority order: `INVARIANTS.md` > `spec/SPEC.md` > `reference/*` > your own judgement. If a spec
  is ambiguous or wrong, make the call, then write it down as an ADR row (with the alternative you
  rejected) in the relevant ADR index or phase file. Keep going rather than stopping to ask.
- REQUIRED / never / must must be enforced by a type, a schema constraint, a GRANT or a test.
  A comment doesn't count.
- **Never mark a partial invariant as enforced.** An honest `PARTIAL` with a note saying what is
  missing is better than a false `DONE`.
- When a task closes, update **its own block in its own phase file**: the status marker (`DONE` /
  `PARTIAL` / `NOT STARTED` / `BLOCKED:HUMAN` / `BLOCKED:UPSTREAM`) and a short dated note
  (2026-10-03) with the evidence: test names and counts. Edit only the blocks for your tasks.
- Mutation-check your key tests: break the thing the test guards and confirm the test fails.

## Your report (final message)

For each task, give: ID, new status, what changed (files), evidence (tests and counts),
anything still missing, and any shared-file edits you need from the orchestrator, written out
exactly.
