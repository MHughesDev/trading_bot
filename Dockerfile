# FB-CONT-001 — application OCI image (control plane + optional Streamlit / live loop)
# Build: docker build -t trading-bot:local .
# Smoke:  docker run --rm trading-bot:local python -c "import app; import control_plane.api"

FROM python:3.12-slim-bookworm

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . .

RUN chmod +x infra/docker/entrypoint.sh \
    && pip install --no-cache-dir -e ".[alpaca,dashboard]"

EXPOSE 8000 8501

ENTRYPOINT ["/app/infra/docker/entrypoint.sh"]
CMD ["api"]
