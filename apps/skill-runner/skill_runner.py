"""Run one agent-authored Python skill, once, under limits.

Reads a job as JSON on stdin, writes a result envelope as JSON on stdout, exits
0 whatever happened. A non-zero exit means *the runner* failed, which the caller
records as `runner_failed` and does not hold against the skill.

    {"source": "...", "payload": {...}, "limits": {"cpu_seconds": 10,
     "memory_mb": 512, "output_bytes": 1048576}}

    {"outcome": "ok"|"raised"|"killed"|"bad_output", "result": {...},
     "stdout": "...", "detail": "...", "duration_ms": 12, "peak_memory_bytes": 1}

Wall-clock is deliberately *not* enforced here. The parent kills the process
group on its own timeout, because a child asked to police its own deadline can
be prevented from doing so by the very code it is policing: a tight C loop in a
numpy call never yields to a Python signal handler.

What this does and does not defend
----------------------------------
Three layers, and only two of them are real:

1. **Resource limits** (`RLIMIT_AS`, `RLIMIT_CPU`, `RLIMIT_NPROC`, `RLIMIT_FSIZE`).
   Enforced by the kernel. POSIX only — on Windows the `resource` module does not
   exist, and this says so in the envelope rather than pretending. A Windows
   developer box therefore runs skills with a wall-clock limit and nothing else,
   which is fine for authoring and is *not* the deployment posture.

2. **A blocked socket layer.** Real enough to stop an accident and a casual
   attempt, and not a boundary: `os.system("curl ...")`, a C extension that opens
   its own socket, or `__import__` games all go around it. The actual boundary is
   the container's network namespace, and this exists so that a skill written
   with `import requests` fails with a sentence explaining the rule instead of a
   connection timeout.

3. **A trimmed environment.** The parent decides what is in `os.environ`; this
   just does not add anything back. Credentials are absent because the parent
   did not pass them, not because anything here removed them.

The honest summary: a skill is *sandboxed against mistakes* by this file and
*sandboxed against intent* by the container it runs in. Anyone reading this
should know which is which.
"""

from __future__ import annotations

import builtins
import io
import json
import sys
import time
import traceback

MAX_STDOUT = 64 * 1024


def _apply_resource_limits(limits: dict) -> str | None:
    """Kernel-enforced caps. Returns a note when they could not be applied."""
    try:
        import resource
    except ImportError:
        return (
            "resource limits are unavailable on this platform (no `resource` module), so this "
            "run was bounded by wall-clock only. That is acceptable for authoring and is not "
            "the deployment posture."
        )

    mem_bytes = int(limits.get("memory_mb", 512)) * 1024 * 1024
    cpu_seconds = int(limits.get("cpu_seconds", 10))
    file_bytes = int(limits.get("output_bytes", 1024 * 1024))

    for name, soft in (
        ("RLIMIT_AS", mem_bytes),
        ("RLIMIT_CPU", cpu_seconds),
        ("RLIMIT_FSIZE", file_bytes),
        # No subprocesses. A skill is one transformation.
        ("RLIMIT_NPROC", 0),
    ):
        limit = getattr(resource, name, None)
        if limit is None:
            continue
        try:
            hard = resource.getrlimit(limit)[1]
            resource.setrlimit(limit, (soft, soft if hard == resource.RLIM_INFINITY else min(soft, hard)))
        except (ValueError, OSError):
            # A limit already lower than we asked for is the container doing its
            # job; refusing to run because we could not *raise* it would be
            # backwards.
            continue
    return None


def _block_network() -> None:
    """Make network calls fail with a sentence rather than a timeout.

    Not a boundary. See the module docstring.
    """

    message = (
        "skills run with no network. Data a skill needs must arrive in its payload, and the web "
        "tools (web_search, fetch_url) are the only way out of the platform — they go through "
        "the egress policy, which is the one place outbound destinations are decided."
    )

    class _Blocked(OSError):
        pass

    def refuse(*_args, **_kwargs):
        raise _Blocked(message)

    try:
        import socket

        socket.socket = refuse
        socket.create_connection = refuse
        socket.create_server = refuse
        socket.getaddrinfo = refuse
    except ImportError:
        pass

    # The common wrappers, so the error names the rule rather than an import
    # error three frames down.
    for module_name in ("urllib.request", "http.client", "ftplib", "smtplib", "telnetlib"):
        sys.modules.pop(module_name, None)

    real_import = builtins.__import__

    def guarded_import(name, *args, **kwargs):
        root = name.split(".")[0]
        if root in {"socket", "requests", "httpx", "aiohttp", "urllib3"}:
            raise ImportError(f"{name}: {message}")
        return real_import(name, *args, **kwargs)

    builtins.__import__ = guarded_import


def _peak_memory() -> int | None:
    try:
        import resource
    except ImportError:
        return None
    usage = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    # Linux reports kilobytes, macOS bytes. Nobody agrees, so normalise on the
    # assumption that a number under 2^32 is kilobytes.
    return usage * 1024 if usage < 2**32 else usage


def _envelope(**fields) -> str:
    return json.dumps(fields, default=str)


def main() -> int:
    try:
        job = json.load(sys.stdin)
        source = job["source"]
        payload = job.get("payload", {})
        limits = job.get("limits", {})
    except Exception as exc:  # noqa: BLE001 - the runner must never crash out
        print(_envelope(outcome="runner_failed", detail=f"unreadable job: {exc}"))
        return 0

    note = _apply_resource_limits(limits)
    _block_network()

    # The skill's `print` goes here, not to our stdout, which is carrying the
    # envelope. A runner that lets a skill write to the result channel is a
    # runner whose output a skill can forge.
    captured = io.StringIO()
    real_stdout = sys.stdout
    sys.stdout = captured

    started = time.monotonic()
    outcome, result, detail = "ok", None, note or ""

    try:
        namespace: dict = {"__name__": "__skill__"}
        exec(compile(source, "<skill>", "exec"), namespace)  # noqa: S102 - the whole point
        run = namespace.get("run")
        if not callable(run):
            outcome, detail = "bad_output", "the skill does not define a callable `run`"
        else:
            result = run(payload)
            if not isinstance(result, dict):
                outcome = "bad_output"
                detail = f"`run` returned {type(result).__name__}, and a skill must return a dict"
                result = None
            else:
                try:
                    json.dumps(result)
                except (TypeError, ValueError) as exc:
                    outcome = "bad_output"
                    detail = f"`run` returned something that is not JSON: {exc}"
                    result = None
    except MemoryError:
        outcome, detail = "killed", "the skill exhausted its memory limit"
    except SystemExit as exc:
        # A skill that calls sys.exit() would otherwise take the envelope with
        # it, and the caller would see a runner failure for a skill bug.
        outcome, detail = "raised", f"the skill called sys.exit({exc.code})"
    except BaseException as exc:  # noqa: BLE001 - including KeyboardInterrupt
        outcome = "raised"
        detail = "".join(traceback.format_exception_only(type(exc), exc)).strip()
    finally:
        sys.stdout = real_stdout

    duration_ms = int((time.monotonic() - started) * 1000)
    stdout_text = captured.getvalue()
    if len(stdout_text) > MAX_STDOUT:
        stdout_text = stdout_text[:MAX_STDOUT] + "\n[... truncated]"

    print(
        _envelope(
            outcome=outcome,
            result=result,
            stdout=stdout_text,
            detail=detail,
            duration_ms=duration_ms,
            peak_memory_bytes=_peak_memory(),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
