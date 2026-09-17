"""The sandbox's executor (AGENT-005 §8, AGENT-006 §5.9, AGENT-010 §10; ADR-0035;
Set S task S-8.8).

# What this container stopped being

It was the Claude Agent SDK's host: the *agent* ran in here, holding a session
token, calling the platform, deciding what to do next. ADR-0035 reversed that —
the loop is `harness::drive` and it runs in the platform. What is left for a
container to do is the one thing the platform genuinely cannot do safely in
process: run a program somebody's model wrote.

So the SDK is gone and this is what runs instead. It accepts one request:

    {"script": "...", "payload": {...}, "data_path": "/workspace/..."}

and answers with stdout, stderr, the exit status and whatever the script wrote
to `result.json`.

# What it does not have

**No provider credential.** Not `ANTHROPIC_API_KEY`, not a proxy token, not a
platform bearer token. A script that could call a model could exfiltrate the
workspace by describing it, and a script that could call `/api/*` would be the
agent again. The environment this process starts with is emptied of every
name that could carry one, so a variable leaked into the container by a
deployment mistake still does not reach the script.

**No network of its own.** Reachability is the container network's job
(`internal: true`), not this file's — but nothing here opens a socket either, so
the failure mode if that network is misconfigured is a script that *can* reach
out rather than one this file quietly allowed.

**No second workspace.** `WORKSPACE` is a single directory and every path in the
request is resolved inside it. A sandbox serving two conversations from one
process is a sandbox in name; see the `refuse` on startup.
"""

from __future__ import annotations

import io
import json
import os
import resource
import subprocess
import sys
import tempfile
from pathlib import Path

# Every environment variable that could carry a credential. Removed from the
# environment the *script* sees, and checked at startup so a deployment that
# supplies one is a loud failure rather than a quiet capability.
FORBIDDEN_ENV = (
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "OPENAI_API_KEY",
    "TBOT_TOKEN",
    "TBOT_API_URL",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_ACCESS_KEY_ID",
    "GOOGLE_API_KEY",
    "HF_TOKEN",
)

WORKSPACE = Path(os.environ.get("SANDBOX_WORKSPACE", "/workspace")).resolve()

# Wall-clock and address-space ceilings. A script that loops forever holds the
# sandbox; one that allocates without bound takes the host down with it.
TIMEOUT_S = int(os.environ.get("SANDBOX_TIMEOUT_S", "300"))
MAX_ADDRESS_SPACE = int(os.environ.get("SANDBOX_MAX_RSS_BYTES", str(4 * 1024**3)))
MAX_OUTPUT_BYTES = 256 * 1024


class Refused(Exception):
    """The request will not be run, and the reason is the caller's to fix."""


def child_env() -> dict[str, str]:
    """The environment the script runs in.

    An allow-list rather than a deny-list. A deny-list is one rename away from
    leaking, and the set of variables a numeric script legitimately needs is
    small enough to write down.
    """
    keep = ("PATH", "HOME", "LANG", "LC_ALL", "PYTHONHASHSEED", "TZ")
    env = {k: v for k, v in os.environ.items() if k in keep}
    env["PYTHONUNBUFFERED"] = "1"
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    # Deterministic by default. A script that wants a different seed sets one;
    # a script that forgot gets the same answer twice, which is the behaviour a
    # trial ledger needs.
    env.setdefault("PYTHONHASHSEED", "0")
    return env


def inside_workspace(candidate: str) -> Path:
    """Resolve `candidate` and refuse anything outside the workspace.

    Resolved before the check, not after: `/workspace/../etc/passwd` is inside
    the workspace as a string and outside it as a path.
    """
    path = (WORKSPACE / candidate).resolve() if not candidate.startswith("/") else Path(candidate).resolve()
    if path != WORKSPACE and WORKSPACE not in path.parents:
        raise Refused(f"`{candidate}` is outside the workspace")
    return path


def limits() -> None:
    """Applied in the child, after fork and before exec."""
    resource.setrlimit(resource.RLIMIT_AS, (MAX_ADDRESS_SPACE, MAX_ADDRESS_SPACE))
    # No core dumps: a dump of a script that held a payload is a copy of the
    # payload in a file nobody is tracking.
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def run(request: dict) -> dict:
    script = request.get("script")
    if not isinstance(script, str) or not script.strip():
        raise Refused("a request carries a script")

    payload = request.get("payload") or {}
    if not isinstance(payload, dict):
        raise Refused("`payload` is an object")

    data_path = request.get("data_path")
    resolved_data = str(inside_workspace(data_path)) if data_path else None
    if resolved_data and not Path(resolved_data).exists():
        raise Refused(f"`{data_path}` does not exist in this workspace")

    with tempfile.TemporaryDirectory(dir=WORKSPACE) as scratch:
        scratch_dir = Path(scratch)
        (scratch_dir / "script.py").write_text(script, encoding="utf-8")
        (scratch_dir / "payload.json").write_text(json.dumps(payload), encoding="utf-8")

        env = child_env()
        env["PAYLOAD_PATH"] = str(scratch_dir / "payload.json")
        env["RESULT_PATH"] = str(scratch_dir / "result.json")
        if resolved_data:
            env["DATA_PATH"] = resolved_data

        try:
            done = subprocess.run(  # noqa: S603 — the whole point of this file
                [sys.executable, str(scratch_dir / "script.py")],
                cwd=str(scratch_dir),
                env=env,
                capture_output=True,
                text=True,
                timeout=TIMEOUT_S,
                preexec_fn=limits,  # noqa: PLW1509 — single-threaded parent
                check=False,
            )
        except subprocess.TimeoutExpired:
            return {
                "ok": False,
                "timed_out": True,
                "exit_code": None,
                "stdout": "",
                "stderr": f"the script did not finish within {TIMEOUT_S}s",
                "result": None,
            }

        result = None
        result_file = scratch_dir / "result.json"
        if result_file.exists():
            try:
                result = json.loads(result_file.read_text(encoding="utf-8"))
            except json.JSONDecodeError as e:
                # Reported rather than swallowed. A script that wrote malformed
                # JSON believes it returned something.
                result = {"error": "result.json is not valid JSON", "detail": str(e)}

        return {
            "ok": done.returncode == 0,
            "timed_out": False,
            "exit_code": done.returncode,
            "stdout": done.stdout[:MAX_OUTPUT_BYTES],
            "stderr": done.stderr[:MAX_OUTPUT_BYTES],
            "truncated": len(done.stdout) > MAX_OUTPUT_BYTES or len(done.stderr) > MAX_OUTPUT_BYTES,
            "result": result,
        }


def startup_checks() -> None:
    present = [k for k in FORBIDDEN_ENV if os.environ.get(k)]
    if present:
        # Not sanitised and continued. A container started with a provider
        # credential is a container somebody configured wrongly, and starting
        # anyway would make the mistake invisible until the day the allow-list
        # gains a name.
        raise SystemExit(
            "refusing to start: this sandbox was given "
            + ", ".join(present)
            + ". It runs model-written code and holds no credential of any kind."
        )
    if not WORKSPACE.is_dir():
        raise SystemExit(f"refusing to start: {WORKSPACE} is not a directory")
    if os.geteuid() == 0:
        raise SystemExit("refusing to start: this sandbox does not run as root")


def main() -> int:
    """One request on stdin, one response on stdout.

    A line protocol rather than an HTTP server, deliberately: a server is a
    listening socket in a container whose whole value is that it cannot be
    reached, and a per-conversation container is started, used and torn down.
    """
    startup_checks()
    raw = io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8").read()
    try:
        request = json.loads(raw)
    except json.JSONDecodeError as e:
        print(json.dumps({"ok": False, "error": "bad_request", "detail": str(e)}))
        return 2
    try:
        print(json.dumps(run(request)))
    except Refused as e:
        print(json.dumps({"ok": False, "error": "refused", "detail": str(e)}))
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
