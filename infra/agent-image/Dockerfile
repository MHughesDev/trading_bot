# The quant research agent's sandbox (AGENT-001 §7, ADR-0024).
#
# One container per research project. It holds no long-lived secret: model calls go
# to the platform's LLM proxy with the session token, and platform calls go to
# /api/* with the same token. Both are scoped to one project and revocable in one row.
#
# Network reachability is enforced by the container network, not by this file — the
# image is attached to an `internal: true` Docker network on which only the platform
# and a package mirror resolve (§7).

FROM python:3.11-slim

# Node is deliberately absent: claude-agent-sdk ships a self-contained CLI binary in
# claude_agent_sdk/_bundled/ (verified against 0.2.152, AGENT-001 §23). Installing a
# Node runtime would add ~150 MB for nothing.
# libgomp1 is the OpenMP runtime. It is not in python:3.11-slim, and without it both
# lightgbm and xgboost install cleanly and then fail at *import* with
# "libgomp.so.1: cannot open shared object file" — a failure that would first appear
# inside a running research session rather than at build time.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git ca-certificates curl libgomp1 \
 && rm -rf /var/lib/apt/lists/*

# Non-root from here on. The root filesystem is mounted read-only at run time; the
# only writable paths are /workspace, /tmp and the cache dir (§7).
RUN useradd --uid 10001 --create-home --shell /bin/bash agent

WORKDIR /opt/agent

# Dependencies first so that a change to agent-host code does not re-resolve the
# research stack, which is by far the slowest layer.
COPY requirements.txt /opt/agent/requirements.txt
RUN pip install --no-cache-dir -r /opt/agent/requirements.txt

# The harness, pinned. AGENT-001 §23 records what was verified against this exact
# version; moving it means re-running that check.
RUN pip install --no-cache-dir "claude-agent-sdk==0.2.152"

COPY --chown=agent:agent apps/agent-host /opt/agent/agent-host
COPY --chown=agent:agent apps/tbot /opt/agent/tbot
RUN pip install --no-cache-dir -e /opt/agent/tbot \
 && pip install --no-cache-dir -e /opt/agent/agent-host

USER agent
WORKDIR /workspace

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    HOME=/home/agent

# The orchestrator supplies TBOT_API_URL, TBOT_TOKEN, TBOT_PROJECT_ID,
# TBOT_SESSION_ID, ANTHROPIC_BASE_URL and ANTHROPIC_AUTH_TOKEN at run time. None of
# them are baked in, and none of them are a provider key.
ENTRYPOINT ["python", "-m", "agent_host"]
