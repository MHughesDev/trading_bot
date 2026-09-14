"""Session hooks (AGENT-001 §12).

Hooks are the harness half of enforcement. They are not the guarantee — the platform
services are — but they give fast, cheap feedback in the loop, and they are where
context waste is trimmed.

The NOTEBOOK re-injection here is the mechanism that replaced the spec's original
plan. SDK 0.2.152 has **no** ``PostCompact`` hook, so instead:

* ``PreCompact`` tells the summariser to preserve NOTEBOOK section 1, and records
  that a compaction happened;
* the next ``UserPromptSubmit`` prepends NOTEBOOK section 1 verbatim.

That pairing is better than a post-compaction hook would have been: it does not
depend on what the summariser chose to keep.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
from typing import Any

#: Tool output larger than this is written to a file and replaced by a pointer.
#: A model that reads 40k tokens of parquet dump learns almost nothing it could not
#: have learned from a summary, and pays for all of it.
MAX_TOOL_OUTPUT_CHARS = 32_000

#: How much of NOTEBOOK.md's first section to re-inject (roughly 2k tokens).
MAX_NOTEBOOK_CHARS = 8_000


class SessionState:
    """Mutable state shared by the hooks within one session."""

    def __init__(self, workspace: str = "/workspace") -> None:
        self.workspace = Path(workspace)
        self.compaction_pending = False
        self.reads: dict[str, str] = {}
        self.overflow_dir = Path("/tmp/out")

    # ── NOTEBOOK ────────────────────────────────────────────────────────────

    def notebook_head(self) -> str | None:
        """The 'Current state' section of NOTEBOOK.md, if there is one."""
        notebook = self.workspace / "NOTEBOOK.md"
        try:
            text = notebook.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            return None

        # Section 1 runs to the next top-level heading.
        lines = text.splitlines()
        collected: list[str] = []
        seen_heading = False
        for line in lines:
            if line.startswith("## "):
                if seen_heading:
                    break
                seen_heading = True
            collected.append(line)
            if sum(len(l) for l in collected) > MAX_NOTEBOOK_CHARS:
                break
        head = "\n".join(collected).strip()
        return head or None


def make_hooks(state: SessionState) -> dict[str, list[Any]]:
    """Build the hook table passed to ``ClaudeAgentOptions.hooks``."""

    async def pre_compact(payload: dict[str, Any], _tool_use_id: Any, _ctx: Any) -> dict[str, Any]:
        """Preserve research state across a compaction."""
        state.compaction_pending = True
        instructions = (
            "Preserve verbatim: the 'Current state' section of NOTEBOOK.md, every open "
            "hypothesis with its status, the trial count, and any job ids still running. "
            "Summarise everything else freely."
        )
        existing = payload.get("custom_instructions") or ""
        return {
            "hookSpecificOutput": {
                "hookEventName": "PreCompact",
                "customInstructions": (existing + "\n" + instructions).strip(),
            }
        }

    async def user_prompt_submit(_payload: dict[str, Any], _tool_use_id: Any, _ctx: Any) -> dict[str, Any]:
        """Re-inject NOTEBOOK state after a compaction.

        Deterministic by design: the file on disk is the source of truth, so what
        comes back does not depend on what the summariser decided to keep.
        """
        if not state.compaction_pending:
            return {}
        state.compaction_pending = False
        head = state.notebook_head()
        if not head:
            return {}
        return {
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": (
                    "Research state restored from NOTEBOOK.md after compaction:\n\n" + head
                ),
            }
        }

    async def post_tool_use(payload: dict[str, Any], _tool_use_id: Any, _ctx: Any) -> dict[str, Any]:
        """Spill oversized tool output to a file and hand back a pointer."""
        response = payload.get("tool_response")
        text = response if isinstance(response, str) else None
        if text is None or len(text) <= MAX_TOOL_OUTPUT_CHARS:
            return {}

        state.overflow_dir.mkdir(parents=True, exist_ok=True)
        digest = hashlib.sha256(text.encode("utf-8", "replace")).hexdigest()[:16]
        path = state.overflow_dir / f"{digest}.txt"
        try:
            path.write_text(text, encoding="utf-8")
        except OSError:
            return {}

        head = text[:1_000]
        return {
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": (
                    f"[output truncated: {len(text)} chars written to {path}]\n"
                    f"first 1000 chars:\n{head}\n"
                    f"Read slices of {path} with offset/limit, or process it in a script."
                ),
            }
        }

    async def pre_tool_use_bash(payload: dict[str, Any], _tool_use_id: Any, _ctx: Any) -> dict[str, Any]:
        """Deny commands that would reach around the platform (AGENT-001 §12).

        Fast feedback only — the platform re-checks everything. The value is that the
        agent learns immediately rather than after a round trip, and the denial says
        what to use instead.
        """
        command = (payload.get("tool_input") or {}).get("command", "")
        api_host = os.environ.get("TBOT_API_URL", "")
        forbidden = [
            ("docker", "the container cannot manage containers"),
            ("sudo", "there is no privileged path inside the sandbox"),
            ("/proc/self/environ", "read configuration from the documented env vars"),
        ]
        for needle, reason in forbidden:
            if needle in command:
                return {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                }

        # Raw HTTP to anywhere but the platform: use `tbot`, which carries the token
        # and records the exploration ledger entry that a bare curl would skip.
        for tool in ("curl ", "wget "):
            if tool in command and api_host and api_host not in command:
                return {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": (
                            "external network access is not available; use `tbot` for "
                            "platform data, which also records the read in the "
                            "exploration ledger"
                        ),
                    }
                }
        return {}

    from claude_agent_sdk import HookMatcher

    return {
        "PreCompact": [HookMatcher(hooks=[pre_compact])],
        "UserPromptSubmit": [HookMatcher(hooks=[user_prompt_submit])],
        "PostToolUse": [HookMatcher(hooks=[post_tool_use])],
        "PreToolUse": [HookMatcher(matcher="Bash", hooks=[pre_tool_use_bash])],
    }
