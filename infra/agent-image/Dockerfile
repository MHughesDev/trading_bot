# The sandbox `run_python` runs in (AGENT-005 §8, AGENT-006 §5.9, AGENT-010 §10;
# ADR-0024 as repurposed by ADR-0035; Set S task S-8.8).
#
# One container per conversation, mounting that conversation's workspace, torn down
# with it.
#
# ## What changed, and why the image kept its name
#
# It was the Claude Agent SDK's host: the agent ran in here and held a session token.
# ADR-0035 reversed that — the loop is `harness::drive` and it runs in the platform.
# What is left for a container to do is the one thing the platform cannot do safely
# in process: run a program a model wrote.
#
# So the SDK is gone, `agent-host` is gone, and the entrypoint is `executor.py`,
# which reads one request on stdin and answers on stdout. The research stack, the
# non-root user and the read-only root are unchanged, because those were never about
# the agent — they were about running somebody else's code.
#
# ## No credential, by construction
#
# This container is given no provider key, no proxy token and no platform bearer
# token, and `executor.py` refuses to start if it finds one. A script that could call
# a model could exfiltrate the workspace by describing it, and a script that could
# call /api/* would be the agent again.
#
# Network reachability is enforced by the container network, not by this file — the
# image is attached to an `internal: true` Docker network (§7).

FROM python:3.11-slim

# libgomp1 is the OpenMP runtime. It is not in python:3.11-slim, and without it both
# lightgbm and xgboost install cleanly and then fail at *import* with
# "libgomp.so.1: cannot open shared object file" — a failure that would first appear
# inside a running research session rather than at build time.
#
# Node, git and curl are gone with the SDK: this image runs numeric Python and
# nothing else, and every binary it does not carry is one a script cannot shell out
# to.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates libgomp1 \
 && rm -rf /var/lib/apt/lists/*

# Non-root from here on. The root filesystem is mounted read-only at run time; the
# only writable paths are /workspace and /tmp (§7).
RUN useradd --uid 10001 --create-home --shell /bin/bash agent

WORKDIR /opt/agent

# Dependencies first so that a change to the executor does not re-resolve the
# research stack, which is by far the slowest layer.
COPY infra/agent-image/requirements.txt /opt/agent/requirements.txt
RUN pip install --no-cache-dir -r /opt/agent/requirements.txt

# The executor, and nothing else. No SDK, no platform client, no agent host: the
# things that made this container an *agent* are exactly the things that made it
# dangerous to hand a model-written script.
COPY --chown=agent:agent infra/agent-image/executor.py /opt/agent/executor.py

USER agent
WORKDIR /workspace

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PYTHONHASHSEED=0 \
    HOME=/home/agent \
    SANDBOX_WORKSPACE=/workspace

# The orchestrator supplies **nothing** but the workspace mount. That is the change:
# the six environment variables this image used to expect were five credentials and a
# session id, and `executor.py` refuses to start if any of them is present.
ENTRYPOINT ["python", "/opt/agent/executor.py"]
