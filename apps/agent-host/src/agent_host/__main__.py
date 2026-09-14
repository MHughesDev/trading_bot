"""Entry point for a research session (AGENT-001 §8).

Runs the Claude Agent SDK against a task, streaming every message back to the
orchestrator so the UI and the session record see the same events. The container
holds no provider credential: model calls go to the platform proxy with the session
token.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
from typing import Any

from claude_agent_sdk import ClaudeSDKClient

from .config import SessionConfig, build_options
from .hooks import SessionState, make_hooks


def _emit(kind: str, payload: dict[str, Any]) -> None:
    """One JSON line per event on stdout.

    The orchestrator reads this stream. Line-delimited JSON rather than a socket so
    that a crashed host still leaves a readable record of how far it got.
    """
    print(json.dumps({"kind": kind, **payload}, default=str), flush=True)


async def run_session(task: str) -> int:
    config = SessionConfig.from_env()
    state = SessionState(workspace=config.workspace)
    options = build_options(config, config.workspace, resume=os.environ.get("TBOT_RESUME"))
    options.hooks = make_hooks(state)

    _emit("session_start", {"project_id": config.project_id, "session_id": config.session_id,
                            "model": config.model, "effort": config.effort})

    last_error: str | None = None
    try:
        async with ClaudeSDKClient(options=options) as client:
            await client.query(task)
            async for message in client.receive_response():
                _emit("message", {"message": _describe(message)})
    except Exception as error:  # noqa: BLE001 - the host must report, not crash silently
        last_error = f"{type(error).__name__}: {error}"
        _emit("session_error", {"error": last_error})
        return 1

    _emit("session_end", {"session_id": config.session_id})
    return 0


def _describe(message: Any) -> dict[str, Any]:
    """Flatten an SDK message into something the orchestrator can store.

    Only the shape is kept, not the full content: the transcript already lives in
    the SDK session, and duplicating it into Postgres would double the storage for
    no new information.
    """
    kind = type(message).__name__
    described: dict[str, Any] = {"type": kind}

    for attribute in ("session_id", "subtype", "total_cost_usd", "duration_ms", "is_error"):
        value = getattr(message, attribute, None)
        if value is not None:
            described[attribute] = value

    content = getattr(message, "content", None)
    if isinstance(content, list):
        described["blocks"] = [type(block).__name__ for block in content]
        texts = [getattr(block, "text", None) for block in content]
        texts = [t for t in texts if isinstance(t, str)]
        if texts:
            joined = "\n".join(texts)
            described["text"] = joined[:2_000]
    elif isinstance(content, str):
        described["text"] = content[:2_000]

    usage = getattr(message, "usage", None)
    if isinstance(usage, dict):
        described["usage"] = usage

    return described


def main() -> int:
    task = " ".join(sys.argv[1:]).strip()
    if not task:
        task = os.environ.get("TBOT_TASK", "").strip()
    if not task:
        print("usage: python -m agent_host <task>  (or set TBOT_TASK)", file=sys.stderr)
        return 2
    return asyncio.run(run_session(task))


if __name__ == "__main__":
    raise SystemExit(main())
