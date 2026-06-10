# Multi-stage build for the platform binary and collectors.
# Stage 1: Build
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY apps/ apps/
COPY xtask/ xtask/

# Build release binaries (all apps including Phase 7 satellites)
RUN cargo build --release \
    -p platform \
    -p collector-crypto \
    -p collector-equity \
    -p collector-web \
    -p embedder \
    -p mcp-server

# Stage 2: Runtime
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binaries from builder
COPY --from=builder /build/target/release/platform /usr/local/bin/platform
COPY --from=builder /build/target/release/collector-crypto /usr/local/bin/collector-crypto
COPY --from=builder /build/target/release/collector-equity /usr/local/bin/collector-equity
COPY --from=builder /build/target/release/collector-web /usr/local/bin/collector-web
COPY --from=builder /build/target/release/embedder /usr/local/bin/embedder
COPY --from=builder /build/target/release/mcp-server /usr/local/bin/mcp-server

# Copy config
COPY config/ /app/config/

EXPOSE 8080 8081

CMD ["platform"]
