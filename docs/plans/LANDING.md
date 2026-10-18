# LANDING — how the joint commit is staged and checked (2026-10-05)

**Purpose.** About a thousand uncommitted paths from roughly thirty tracks sit in one working
tree, beside a second Claude session ("Agent handoff continuation") that implements Set U and
commits its own phases. This file says how to turn that tree into commits without staging the
wrong thing, and how to prove the *staged* tree builds on its own, because the working tree
always builds (an untracked file you forgot to stage compiles fine here and breaks everywhere
else). Nothing in this file has been staged or committed. The final verification pass
(track VERIFY2) wrote it from `git status` at the time and ran no `git add`.

## 1. Ground rules

1. **No `git add -A`, no `git add .`, no `git commit -a`.** The tree holds files nobody may stage
   (section 3.1) and files the other session stages (section 3.5).
2. **No `git stash`, no worktree, no checkout of anything dirty.** The isolated check below exports
   the *index* to a plain directory. It never touches the working tree.
3. **Stage by pathspec, with the exclusions in section 4.** Then read `git diff --cached --stat`
   before committing. A whole-file rewrite in that stat is a line-ending accident: this repo has
   `core.autocrlf=true`, so working copies are CRLF where HEAD is LF, and git hides that. The
   2026-10-05 pass normalised every flipped file to its HEAD ending; if the stat shows a file with
   insertions close to its line count, run `git diff --cached --ignore-cr-at-eol --stat -- <file>`.
4. **Migrations are LF-only** (`.gitattributes`). `grep -lU $'\r' migrations/*.sql` must print nothing.
   It printed nothing on 2026-10-05, over 163 files, including the new 0246. Numbers are unique.
5. **Commit in the order of section 5**, so every commit builds on the one before. The check in
   section 6 runs against the index after the *last* `git add` of a group, not after each file.

## 2. Owners

| owner | what | how it is staged |
|---|---|---|
| **the tracks of this push** (S, P, T, M, N, Q, R-a, R-b, E, H, L, FB, plus the VERIFY and VERIFY2 passes) | everything not listed under the next rows | by pathspec, section 4 |
| **the Set U session** | the territory in `TRACK-RULES.md` rule 4: `crates/strategy-engine/**`, `crates/strategy-lang/**`, `crates/backtest/src/{models,runner,portfolio,shadow,replay,selection_run,external}.rs` and `env/**`, `crates/api/src/routes/{deployments,strategies,strategies_v3,selections}.rs`, `apps/platform/src/{live_runner,live_deployments,parity_jobs,allocator}.rs`, `crates/mcp-server/src/tools/{strategy_v3,selections,deployment_proposals}.rs`, `frontend/src/components/strategy/**`, `frontend/src/pages/StrategyGraph*`, `docs/plans/plan-sets/set-U/**`, `docs/agent/slv3-guide.md` | **it stages these.** Do not stage them for it, and do not stage a file in this row because it also carries a hunk of mine (section 3.2 lists those; either split with `git add -p`, or let its commit carry the hunk and say so in the message) |
| **the owner** | `docs/specs/README.MD` (a standing no-touch file), `crates/collectors/src/crypto/kraken.rs` (dirty before the push began), `.cargo/config.toml` (a decision, section 7) | never staged by a track |
| **the orchestrator** | the shared files of `TRACK-RULES.md` rule 5 | whole, in the last commit of this push, because both sides have added lines to them (section 3.4) |

The two sessions edit one tree, so **ownership is by hunk, not by file** in the shared and the
mixed lists. The markers used to find mixed hunks (Set U / `U-x.y` / `FEAT-007` against the ids of
Sets M, N, Q, R, T, P, S) are a heuristic over added lines. A hunk with no marker can still be the
other session's. `git add -p` is the only way to be sure.

## 3. The file lists, as of the snapshot (a heuristic over `git status`; section 7 says how to redo it)

1059 dirty paths: 1008 mine, 10 shared, 38 in the Set U session's territory, 3 never staged.

### 3.1 Never staged

- `.cargo/config.toml` (M)
- `crates/collectors/src/crypto/kraken.rs` (M)
- `docs/specs/README.MD` (M)

### 3.2 Hunks of mine inside the Set U session's files (it stages the file; split with `git add -p`, or leave to it)

These files are in its territory but carry added lines that name my tracks' tasks (Set M/N/Q/R/T/P/S ids). The Set U session commits its own hunks, and mine need to ride along with the commit that stages the file, or be split out by hunk.

- `apps/platform/src/live_deployments.rs` (my markers 4, its markers 2)
- `apps/platform/src/live_runner.rs` (my markers 4, its markers 2)
- `crates/api/src/routes/deployments.rs` (my markers 2, its markers 0)
- `crates/backtest/src/replay.rs` (my markers 1, its markers 0)
- `crates/backtest/src/runner.rs` (my markers 2, its markers 0)
- `crates/strategy-engine/src/checkpoint.rs` (my markers 1, its markers 0)
- `crates/strategy-engine/src/compile.rs` (my markers 1, its markers 0)
- `crates/strategy-engine/src/engine.rs` (my markers 1, its markers 0)
- `crates/strategy-engine/src/history.rs` (my markers 1, its markers 0)
- `crates/strategy-engine/src/instance.rs` (my markers 1, its markers 0)
- `crates/strategy-engine/src/plan.rs` (my markers 1, its markers 0)
- `crates/strategy-engine/src/registry.rs` (my markers 1, its markers 0)
- `crates/strategy-lang/Cargo.toml` (my markers 3, its markers 1)
- `crates/strategy-lang/src/form.rs` (my markers 1, its markers 0)
- `crates/strategy-lang/src/lib.rs` (my markers 1, its markers 0)
- `docs/plans/plan-sets/set-U/phase-4-paper-and-live.md` (my markers 3, its markers 1)

### 3.3 Hunks of the Set U session's inside files I own

Added lines naming Set U / U-x.y in files outside its territory. Check these with `git add -p` before staging the file as mine. Documentation files are left out of this list on purpose: plan and queue documents mention Set U in prose all the time, so the marker proves nothing there (`docs/plans/plan-sets/set-U/**` and `docs/agent/slv3-guide.md` are its own and are excluded in section 4).

- `apps/model-trainer/app/schemas.py` (its markers 1, my markers 2)
- `apps/model-trainer/app/worker.py` (its markers 1, my markers 6)
- `crates/api/src/risk_context.rs` (its markers 1, my markers 0)
- `crates/api/src/routes/mod.rs` (its markers 1, my markers 12)
- `crates/execution/src/paper/engine.rs` (its markers 5, my markers 0)
- `crates/mcp-server/src/lib.rs` (its markers 1, my markers 13)
- `crates/mcp-server/src/spec.rs` (its markers 1, my markers 5)
- `crates/model-registry/src/manager.rs` (its markers 1, my markers 11)
- `crates/model-registry/src/sidecar.rs` (its markers 1, my markers 5)
- `crates/skills/src/template.rs` (its markers 1, my markers 0)

### 3.4 Shared files (orchestrator-owned; stage whole, they carry both sides' lines)

- `Cargo.lock` (M)
- `apps/platform/src/main.rs` (M)
- `crates/api/src/lib.rs` (M)
- `crates/api/src/routes/mod.rs` (M)
- `crates/mcp-server/src/lib.rs` (M)
- `crates/mcp-server/src/spec.rs` (M)
- `crates/mcp-server/src/tools/mod.rs` (M)
- `docs/plans/PLAN.md` (M)
- `frontend/src/App.tsx` (M)
- `docs/plans/OPEN-TASKS.md` (??)

### 3.5 The Set U session's files (it stages these)

- `apps/platform/src/live_deployments.rs` (M)
- `apps/platform/src/live_runner.rs` (M)
- `apps/platform/src/parity_jobs.rs` (M)
- `crates/api/src/routes/deployments.rs` (M)
- `crates/backtest/src/replay.rs` (M)
- `crates/backtest/src/runner.rs` (M)
- `crates/strategy-engine/src/checkpoint.rs` (M)
- `crates/strategy-engine/src/compile.rs` (M)
- `crates/strategy-engine/src/engine.rs` (M)
- `crates/strategy-engine/src/history.rs` (M)
- `crates/strategy-engine/src/instance.rs` (M)
- `crates/strategy-engine/src/lib.rs` (M)
- `crates/strategy-engine/src/plan.rs` (M)
- `crates/strategy-engine/src/registry.rs` (M)
- `crates/strategy-lang/Cargo.toml` (M)
- `crates/strategy-lang/src/block.rs` (M)
- `crates/strategy-lang/src/form.rs` (M)
- `crates/strategy-lang/src/lib.rs` (M)
- `crates/strategy-lang/src/skeleton.rs` (M)
- `docs/agent/slv3-guide.md` (M)
- `docs/plans/plan-sets/set-U/MASTER.md` (M)
- `docs/plans/plan-sets/set-U/phase-4-paper-and-live.md` (M)
- `frontend/src/pages/StrategyGraphPage.test.tsx` (M)
- `crates/backtest/src/external.rs` (??)
- `crates/mcp-server/src/tools/deployment_proposals.rs` (??)
- `crates/strategy-engine/src/external.rs` (??)
- `crates/strategy-engine/tests/external_inputs.rs` (??)
- `crates/strategy-lang/docs/slv2-guide.md` (??)
- `crates/strategy-lang/src/guide.rs` (??)
- `crates/strategy-lang/src/intent.rs` (??)
- `crates/strategy-lang/tests/skeletons_fire_on_hourly_bars.rs` (??)
- `crates/strategy-lang/tests/skeletons_on_btc_usd.rs` (??)
- `crates/strategy-lang/tests/slv2_guide.rs` (??)
- `crates/strategy-lang/tests/small_tier_eval.rs` (??)
- `crates/strategy-lang/tests/support/mod.rs` (??)
- `crates/strategy-lang/tests/the_form_schema_stands_alone.rs` (??)
- `crates/strategy-lang/tests/the_preview_names_the_blocker.rs` (??)
- `crates/strategy-lang/tests/the_six_teachings_have_producers.rs` (??)

### 3.6 Mine, by directory

| directory | modified | untracked |
|---|---:|---:|
| `.github` | 1 | 1 |
| `apps/model-inference` | 7 | 1 |
| `apps/model-trainer` | 12 | 39 |
| `apps/platform` | 6 | 2 |
| `apps/skill-runner` | 5 | 15 |
| `apps/tbot` | 3 | 3 |
| `clickhouse` | 0 | 2 |
| `config` | 2 | 3 |
| `crates/api` | 70 | 171 |
| `crates/backtest` | 23 | 24 |
| `crates/builders` | 2 | 0 |
| `crates/collectors` | 8 | 6 |
| `crates/dataplane` | 8 | 2 |
| `crates/domain` | 2 | 1 |
| `crates/event-bus` | 4 | 1 |
| `crates/execution` | 20 | 8 |
| `crates/features` | 4 | 7 |
| `crates/harness` | 14 | 4 |
| `crates/invariants` | 4 | 3 |
| `crates/jobs` | 8 | 4 |
| `crates/ledger` | 6 | 6 |
| `crates/mcp-server` | 24 | 14 |
| `crates/model-registry` | 11 | 20 |
| `crates/monitor` | 7 | 6 |
| `crates/observability` | 4 | 0 |
| `crates/reconciliation` | 9 | 3 |
| `crates/research` | 3 | 13 |
| `crates/research-session` | 6 | 0 |
| `crates/risk` | 1 | 0 |
| `crates/skills` | 3 | 11 |
| `crates/storage` | 8 | 10 |
| `crates/strategy-validator` | 2 | 1 |
| `docs/FOLDER_MAP.md` | 1 | 0 |
| `docs/QUEUE.MD` | 1 | 0 |
| `docs/QUEUE_STACK.csv` | 1 | 0 |
| `docs/adr` | 1 | 1 |
| `docs/archive` | 1 | 0 |
| `docs/backlog` | 1 | 0 |
| `docs/governance` | 1 | 1 |
| `docs/ml-platform-context-pack` | 4 | 0 |
| `docs/open-questions.md` | 1 | 0 |
| `docs/operations` | 0 | 1 |
| `docs/parity-matrix.md` | 1 | 0 |
| `docs/plans` | 51 | 29 |
| `docs/procedures` | 5 | 0 |
| `docs/research` | 1 | 0 |
| `docs/specs` | 12 | 0 |
| `evals` | 2 | 15 |
| `frontend/src/api` | 2 | 6 |
| `frontend/src/components` | 30 | 58 |
| `frontend/src/hooks` | 2 | 2 |
| `frontend/src/lib` | 2 | 22 |
| `frontend/src/pages` | 5 | 15 |
| `frontend/src/utils` | 1 | 1 |
| `migrations` | 0 | 55 |
| `skills` | 0 | 6 |
| `xtask` | 1 | 1 |

## 4. Staging commands

Run from the repo root. `git add` with exclusion pathspecs stages everything dirty *except* the
paths named, which is the safe direction: a new file nobody remembered is staged and then caught
by the check in section 6, rather than silently left behind.

```bash
# 4.1  everything of mine: all dirty paths except the owner's, the Set U session's, and the shared files
git add -- . \
  ':(exclude)docs/specs/README.MD' \
  ':(exclude).cargo/config.toml' \
  ':(exclude)crates/collectors/src/crypto/kraken.rs' \
  ':(exclude)crates/strategy-engine' \
  ':(exclude)crates/strategy-lang' \
  ':(exclude)crates/backtest/src/models.rs'        ':(exclude)crates/backtest/src/runner.rs' \
  ':(exclude)crates/backtest/src/portfolio.rs'     ':(exclude)crates/backtest/src/shadow.rs' \
  ':(exclude)crates/backtest/src/replay.rs'        ':(exclude)crates/backtest/src/selection_run.rs' \
  ':(exclude)crates/backtest/src/external.rs'      ':(exclude)crates/backtest/src/env' \
  ':(exclude)crates/api/src/routes/deployments.rs' ':(exclude)crates/api/src/routes/strategies.rs' \
  ':(exclude)crates/api/src/routes/strategies_v3.rs' ':(exclude)crates/api/src/routes/selections.rs' \
  ':(exclude)apps/platform/src/live_runner.rs'     ':(exclude)apps/platform/src/live_deployments.rs' \
  ':(exclude)apps/platform/src/parity_jobs.rs'     ':(exclude)apps/platform/src/allocator.rs' \
  ':(exclude)crates/mcp-server/src/tools/strategy_v3.rs' ':(exclude)crates/mcp-server/src/tools/selections.rs' \
  ':(exclude)crates/mcp-server/src/tools/deployment_proposals.rs' \
  ':(exclude)frontend/src/components/strategy' ':(exclude)frontend/src/pages/StrategyGraph*' \
  ':(exclude)docs/plans/plan-sets/set-U' ':(exclude)docs/agent/slv3-guide.md' \
  ':(exclude)docs/specs/FEAT-007-unified-strategy-engine.md' \
  ':(exclude)crates/domain/src/strategy_graph' \
  ':(exclude)Cargo.toml' ':(exclude)Cargo.lock' \
  ':(exclude)apps/platform/src/main.rs' ':(exclude)crates/api/src/lib.rs' ':(exclude)crates/api/src/routes/mod.rs' \
  ':(exclude)crates/mcp-server/src/taxonomy.rs' ':(exclude)crates/mcp-server/src/spec.rs' \
  ':(exclude)crates/mcp-server/src/lib.rs' ':(exclude)crates/mcp-server/src/tools/mod.rs' \
  ':(exclude)frontend/src/App.tsx' ':(exclude)docs/plans/PLAN.md' ':(exclude)docs/plans/OPEN-TASKS.md'

# 4.2  files in section 3.3 (mine, but carrying Set U hunks): stage by hunk instead of whole.
#      Add those paths to the exclusion list above, then:
git add -p -- <file>            # y for my hunks, n for the Set U session's

# 4.3  my hunks inside the Set U session's files (section 3.2): only if it has not already
#      committed the file. Otherwise they ride in its commit.
git add -p -- <file>

# 4.4  the shared files, last, whole, after both sides' lines are in
git add -- Cargo.toml Cargo.lock apps/platform/src/main.rs crates/api/src/lib.rs crates/api/src/routes/mod.rs \
  crates/mcp-server/src/taxonomy.rs crates/mcp-server/src/spec.rs crates/mcp-server/src/lib.rs \
  crates/mcp-server/src/tools/mod.rs frontend/src/App.tsx docs/plans/PLAN.md docs/plans/OPEN-TASKS.md
```

`crates/mcp-server/src/{lib.rs,spec.rs,tools/mod.rs}` were dirty before the push began and appear
in both lists. If the Set U session has not yet committed its `propose_deployment` tool
(`deployment_proposals.rs`), staging `tools/mod.rs` whole **registers a module whose file is not in
the index**, and the isolated check fails with `file not found for module`. That failure is the
check doing its job: stage `deployment_proposals.rs` with it (and tell the other session), or hold
`tools/mod.rs` back.

After staging, always:

```bash
git diff --cached --stat | tail -3
git diff --cached --name-only | grep -E '^(docs/specs/README\.MD|\.cargo/config\.toml|crates/collectors/src/crypto/kraken\.rs)$' && echo "STOP: a never-stage file is in the index"
git diff --cached --check               # trailing whitespace / conflict markers
grep -lU $'\r' migrations/*.sql         # must print nothing
git ls-files -o --exclude-standard      # untracked and not ignored: each one must be intentional
```

## 5. Commit order

1. **Migrations and schema grants** (`migrations/0145` to `0246`, `crates/storage`, `clickhouse/*.sql`).
   They are checked by applying them (section 6.3).
2. **Foundation crates** (`domain`, `execution`, `risk`, `reconciliation`, `monitor`, `ledger`,
   `jobs`, `dataplane`, `collectors`, `features`, `model-registry`, `skills`, `harness`, `llm`,
   `research*`, `invariants`), because the api and the platform depend on them.
3. **`backtest`** (the files outside the Set U territory), then the Set U session's commits for its
   own backtest files. `backtest` does not compile without `models.rs`, `runner.rs`, `external.rs`,
   so these two groups **must be consecutive** and the first one alone does not build. Say so in
   the first commit's message.
4. **`mcp-server`, `api`, `apps/platform`**, then `apps/model-trainer`, `apps/model-inference`,
   `apps/skill-runner`, `apps/tbot`, `skills/`, `evals/`.
5. **`frontend/`.**
6. **Docs**, including `docs/plans/PLAN.md` and `OPEN-TASKS.md`. They are derived from the plan
   files, so they go in the same commit as the last plan-file edit or after it, never before.
7. **The shared files** (4.4), last, because they name modules and routes from everything above.

Commits 3 and 7 are where a mid-push `HEAD` would not build. If a bisectable history matters less
than a short one, collapse 2 to 7 into one commit.

## 6. The isolated check of the staged tree

The question: *does the index, by itself, build?* No stash and no worktree: `git write-tree`
turns the index into a tree object and `git archive` writes that tree to a directory, without
reading or changing a single working file.

### 6.1 Prepare (disk first)

```bash
# needs about 15 GB for the check target. Under 40 GB free on C:, delete target/debug/incremental first.
powershell -c "Get-PSDrive C | % { [math]::Round(\$_.Free/1GB,1) }"
rm -rf /c/tmp/landing_check /c/tmp/landing_target && mkdir -p /c/tmp/landing_check
```

### 6.2 Export the index and check it

```bash
cd /d/coding_projects/trading_bot
git write-tree                                   # prints the tree id; fails if the index has unmerged paths
git archive --format=tar "$(git write-tree)" | tar -x -C /c/tmp/landing_check

cd /c/tmp/landing_check
export CARGO_TARGET_DIR=/c/tmp/landing_target    # NOT the shared target: this must start from nothing
cargo check --locked -p api -p platform -p research > /c/tmp/landing_check.log 2>&1; echo EXIT=$?
```

`--locked` is the point of staging `Cargo.lock` with the manifests: it fails if a staged manifest
needs a lock the index does not carry. Read the real exit code (`EXIT=`); never pipe cargo.
Interpreting a failure:

| message | meaning | fix |
|---|---|---|
| `file not found for module X` or `couldn't read ...` | a file that exists in the working tree is untracked or unstaged | stage it, or hold back the module declaration |
| `unresolved import` or a missing function across a crate boundary | a hunk of a shared file was staged without the file it calls | stage the other half (sections 3.2 and 3.3) |
| `the lock file needs to be updated` | `Cargo.lock` is not staged with its manifests | stage it |
| `failed to read ... Cargo.toml` | a `members` path or a path dependency is untracked | stage the crate |

Then widen it, because `-p api -p platform -p research` does not compile the other crates' tests:

```bash
cargo check --locked --workspace --all-targets > /c/tmp/landing_check_all.log 2>&1; echo EXIT=$?
cargo test  --locked -p invariants --no-fail-fast > /c/tmp/landing_inv.log 2>&1; echo EXIT=$?   # reads the exported tree's sources
```

The invariants run matters most: `static_invariants`, `the_plan_counts_what_the_plans_say` and
`every_client_path_is_served` read files, so on the exported tree they judge what will actually be
committed, and a plan file or a route that exists only in the working tree fails them there.

For the frontend, link the existing dependencies instead of installing:

```bash
cd /c/tmp/landing_check/frontend && cmd //c mklink /J node_modules "D:\coding_projects\trading_bot\frontend\node_modules"
npx tsc -b --noEmit && npx vitest run && npm run lint
```

### 6.3 Migrations against an empty database

```bash
export PGPASSWORD=postgres; P="/c/Program Files/PostgreSQL/17/bin/psql.exe"
"$P" -h 127.0.0.1 -p 5433 -U postgres -c "create database landing_scratch"
"$P" -h 127.0.0.1 -p 5433 -U postgres -d landing_scratch -c "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMPTZ NOT NULL DEFAULT now(), success BOOLEAN NOT NULL, checksum BYTEA NOT NULL, execution_time BIGINT NOT NULL)"
for f in $(ls /c/tmp/landing_check/migrations/*.sql | sort); do
  # pgvector is not installed on 5433: float8[] stands in for vector(n), and the extension line is dropped
  sed -E 's/CREATE EXTENSION IF NOT EXISTS vector;//; s/vector\(([0-9]+)\)/float8[]/g' "$f" \
    | "$P" -h 127.0.0.1 -p 5433 -U postgres -d landing_scratch -v ON_ERROR_STOP=1 -q -1 -f - || { echo "FAIL $f"; break; }
done
"$P" -h 127.0.0.1 -p 5433 -U postgres -c "drop database landing_scratch"
```

### 6.4 Clean up

```bash
rm -rf /c/tmp/landing_check /c/tmp/landing_target /c/tmp/landing_*.log
```

Delete `C:\tmp\landing_*` the same hour: the check target is the one thing in this procedure that
can fill the disk.

## 7. Regenerating section 3, and the one owner decision

The lists above come from `git status --short -uall` and a path table copied from
`TRACK-RULES.md` rule 4. After the other session commits a phase, the files it committed drop out
of `git status` and out of its list. Re-run `git status --short -uall` and re-read sections 3.2 and
3.3 before staging anything, because the snapshot ages as fast as that session works.

**`.cargo/config.toml` is a decision for the owner, not a staging question.** The diff raises
`jobs` from 2 to 6 and adds `rust-lld` as the Windows MSVC linker. The linker table is
Windows-only, but `jobs = 6` applies on every machine, including CI. On 2026-10-05 a build with
`-j 6` hit `os error 1455` (the paging file is too small) while compiling the `api` crate's ~130
test binaries, and `-j 1` was needed for it; the old value of 2 was set for the same reason.
Options: commit it as is, commit `jobs = 2` with the linker lines, or leave it local. This file
recommends the second.
