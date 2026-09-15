# Task runner for the trading platform.
# Install just: cargo install just
# Usage: just <recipe>

# Default: show available recipes
default:
    @just --list

# ── Development ────────────────────────────────────────────────────────────────

# Start infra and run the platform
#
# The platform's order gate fails closed on the monitor's lease (COMP-007 §4),
# so `just monitor` has to be running in another terminal or every order is
# refused with `monitor_unavailable`. That is the design, not a bug: if the
# monitor dies, the platform stops.
dev:
    docker compose up -d
    cargo run -p platform

# Run the live monitor (COMP-007). A separate process, on purpose — a monitor
# inside the platform dies with it and has therefore checked nothing.
monitor:
    DATABASE_URL=postgres://trading:trading@localhost:5432/trading cargo run -p monitor --bin tbot-monitor

# Rehearse the kill switch end to end (COMP-007 §4.2, §8.1-§8.3).
#
# Nobody will trip a global halt on a live book to see whether it works, so
# without this the most important path in the platform is the one path never
# exercised. A drill whose propagation exceeds one cadence fails the deploy.
drill:
    MONITOR_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
      cargo test -j 2 -p monitor --test fail_closed_drill -- --test-threads=1

# Run the MCP server (requires PLATFORM_API_TOKEN — see docs/mcp/README.md)
run-mcp:
    cargo run -p mcp-server

# Start infra only
infra:
    docker compose up -d

# Stop infra
infra-stop:
    docker compose down

# ── Build & Check ──────────────────────────────────────────────────────────────

# Check all crates compile
check:
    cargo check --workspace --all-targets

# Format all code
fmt:
    cargo fmt --all

# Check formatting (for CI)
fmt-check:
    cargo fmt --all -- --check

# Run clippy lints
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Check for f64 usage on price/size (money safety)
check-money:
    cargo xtask check-money-f64

# ── Testing ────────────────────────────────────────────────────────────────────

# Run all tests (requires infra)
test:
    cargo test --workspace

# Run unit tests only (no infra needed)
test-unit:
    cargo test --workspace --lib

# Run integration tests (requires infra)
test-integration:
    docker compose up -d
    cargo test --workspace --test '*'

# ── Database ───────────────────────────────────────────────────────────────────

# Apply Postgres migrations
migrate:
    sqlx migrate run --database-url postgres://trading:trading@localhost:5432/trading

# Apply ClickHouse DDL
migrate-ch:
    #!/usr/bin/env bash
    # The HTTP interface rejects multi-statement bodies (05_backtest_run_series.sql
    # has two), and without ?database= the tables land in the user's default DB —
    # split on trailing-semicolon boundaries and post one statement at a time.
    for f in clickhouse/*.sql; do
        echo "Applying $f..."
        awk 'BEGIN{RS=";[[:space:]]*\n"} /[^[:space:]]/ {printf "%s;\0", $0}' "$f" \
        | while IFS= read -r -d '' stmt; do
            curl -s -X POST "http://trading:trading@localhost:8123/?database=trading" --data-binary "$stmt"
        done
    done

# ── Frontend ───────────────────────────────────────────────────────────────────

# Run frontend dev server
frontend:
    cd frontend && npm run dev

# Build frontend
frontend-build:
    cd frontend && npm run build

# ── Release ────────────────────────────────────────────────────────────────────

# Build release binary
build-release:
    cargo build --release -p platform -p collector-crypto -p collector-equity -p mcp-server

# ── Hot-path integrity ─────────────────────────────────────────────────────────

# Verify no Publisher::publish calls exist on the strategy evaluation path.
# Only tee.rs is allowed to reference Publisher.
check-hot-path:
    @echo "Checking: Publisher::publish must not appear in apps/platform/src/ except tee.rs..."
    @result=$$(grep -r "Publisher::publish" apps/platform/src/ | grep -v "tee\.rs"); \
    if [ -n "$$result" ]; then \
        echo "FAIL: Publisher::publish found on hot path:"; \
        echo "$$result"; \
        exit 1; \
    else \
        echo "OK: Publisher::publish is confined to tee.rs"; \
    fi
