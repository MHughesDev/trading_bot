# FB-CONT-001 — application OCI image (control plane + React UI + optional live loop)
# Build: docker build -t trading-bot:local .
# Smoke:  docker run --rm trading-bot:local python -c "import app; import control_plane.api"

FROM python:3.12-slim-bookworm AS python-base

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# ── React frontend build stage ─────────────────────────────────────────────────
FROM node:24-slim AS frontend-build

WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ── Final image ────────────────────────────────────────────────────────────────
FROM python-base

COPY . .
# Overlay the pre-built React dist so FastAPI can serve it
COPY --from=frontend-build /app/frontend/dist /app/frontend/dist

RUN chmod +x infra/docker/entrypoint.sh \
    && pip install --no-cache-dir -e ".[alpaca,dashboard]"

EXPOSE 8001

ENTRYPOINT ["/app/infra/docker/entrypoint.sh"]
CMD ["api"]
