# Task runner for the trading platform.
# Install just: cargo install just
# Usage: just <recipe>

# Default: show available recipes
default:
    @just --list

# ── Development ────────────────────────────────────────────────────────────────

# Start infra and run the platform
dev:
    docker compose up -d
    cargo run -p platform

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
    for f in clickhouse/*.sql; do
        echo "Applying $f..."
        curl -s -X POST "http://trading:trading@localhost:8123/" --data-binary @"$f"
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
