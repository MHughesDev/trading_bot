"""Session configuration for the agent host (AGENT-001 §8.2).

Every identifier here was checked against ``claude-agent-sdk==0.2.152`` during the
Phase 0 spike (AGENT-001 §23). Two of them are *absences* worth stating, because
the spec originally assumed otherwise:

* There is **no** ``PostCompact`` hook. NOTEBOOK re-injection uses ``PreCompact``
  plus ``UserPromptSubmit`` instead (see :mod:`agent_host.hooks`).
* There is **no** cache-TTL option. The CLI picks its own TTL, so a longer one is
  arranged by the platform proxy rewriting ``cache_control`` (RT-22).
"""

from __future__ import annotations

import os
from dataclasses import dataclass

from claude_agent_sdk import ClaudeAgentOptions

#: Scopes the container's token is expected to carry. Listed here only so a
#: misconfigured session fails loudly at start rather than at the first tool call.
EXPECTED_SCOPES = frozenset(
    {
        "research:data.read",
        "research:features",
        "research:jobs",
        "research:artifacts",
        "research:memory",
        "research:report",
        "llm:proxy",
    }
)


@dataclass(frozen=True)
class SessionConfig:
    """Everything the orchestrator hands the container."""

    api_url: str
    token: str
    project_id: str
    session_id: str
    model: str
    effort: str
    workspace: str = "/workspace"

    @classmethod
    def from_env(cls) -> "SessionConfig":
        missing = [
            name
            for name in ("TBOT_API_URL", "TBOT_TOKEN", "TBOT_PROJECT_ID", "TBOT_SESSION_ID")
            if not os.environ.get(name)
        ]
        if missing:
            raise RuntimeError(
                "the orchestrator must supply " + ", ".join(missing) +
                "; refusing to start a session that cannot reach the platform"
            )
        return cls(
            api_url=os.environ["TBOT_API_URL"],
            token=os.environ["TBOT_TOKEN"],
            project_id=os.environ["TBOT_PROJECT_ID"],
            session_id=os.environ["TBOT_SESSION_ID"],
            # Pinned per project version and never changed inside a session (R3).
            model=os.environ.get("TBOT_MODEL", "claude-opus-5"),
            effort=os.environ.get("TBOT_EFFORT", "high"),
        )


#: The identity and behaviour contract. Stable across a session so it stays cached:
#: anything that changes per turn belongs in the first user message instead, because
#: a system prompt that varies invalidates the cache prefix on every call (CX).
SYSTEM_CORE = """\
You are a quantitative research agent.

Your authority is research only. You cannot place orders, arm automations, promote \
model aliases, or read holdout data — not as a matter of policy you are asked to \
respect, but because the platform does not issue your token those capabilities. \
Attempting them wastes a turn.

What you can do:
- read point-in-time data through `tbot data`, clipped to this project's research cutoff;
- submit durable work through `tbot jobs` and wait on it;
- read and write artifacts by handle;
- record findings in NOTEBOOK.md and submit a validated final_report.

Three things about how this platform counts:
1. Submitting a backtest, sweep, study, gate advance, train or hpo job SPENDS A TRIAL. \
   The count is kept by the platform and cannot be lowered. More trials means a higher \
   significance bar, so search deliberately rather than exhaustively.
2. Looking at data does NOT spend a trial. It is logged to an exploration ledger and \
   reported with your verdict, so a reader can see how much searching preceded a result.
3. An identical job submitted twice is one job and one trial. A `rerun_nonce` makes it \
   a new job and a new trial; use it only when you mean to.

Work in files and code. Read data to `/workspace/data/`, analyse it in scripts, and \
keep `NOTEBOOK.md` current — its first section is your state if the session is \
compacted or resumed.
"""


def build_options(config: SessionConfig, workspace: str, resume: str | None = None) -> ClaudeAgentOptions:
    """Assemble the SDK options for one session."""
    return ClaudeAgentOptions(
        # `exclude_dynamic_sections` keeps the preset's per-run text (dates, cwd
        # listings) out of the cached prefix. Verified snake_case in 0.2.152.
        system_prompt={
            "type": "preset",
            "preset": "claude_code",
            "append": SYSTEM_CORE,
            "exclude_dynamic_sections": True,
        },
        cwd=workspace,
        model=config.model,
        effort=config.effort,  # type: ignore[arg-type]
        # Only project settings: `user` and `local` would pull in whatever is on the
        # host running the orchestrator, which is not part of this project.
        setting_sources=["project"],
        resume=resume,
        # Sub-agents are declared, not discovered, so their tool surface is known.
        # `task_budget` caps the total across a session.
        task_budget={"total": 20},
        # Belt and braces; the proxy is authoritative on spend (AGENT-001 §9).
        max_budget_usd=float(os.environ.get("TBOT_MAX_BUDGET_USD", "25")),
        # Second containment layer inside the container (§8.2). The container's
        # `internal: true` network is the boundary that actually enforces this; the
        # sandbox stops an accidental egress attempt from even being made.
        sandbox={
            "enabled": True,
            "autoAllowBashIfSandboxed": True,
            "allowUnsandboxedCommands": False,
            "network": {
                "allowedDomains": [_host_of(config.api_url)],
                "allowLocalBinding": True,
            },
        },
        env={
            "TBOT_API_URL": config.api_url,
            "TBOT_TOKEN": config.token,
            "TBOT_PROJECT_ID": config.project_id,
            "TBOT_SESSION_ID": config.session_id,
            # The model call goes to the platform proxy, which swaps in the real
            # provider credential. The container never holds one.
            "ANTHROPIC_BASE_URL": f"{config.api_url.rstrip('/')}/llm",
            "ANTHROPIC_AUTH_TOKEN": config.token,
            "ANTHROPIC_CUSTOM_HEADERS": f"X-Tbot-Role: main\nX-Tbot-Session: {config.session_id}",
        },
        include_partial_messages=True,
        include_hook_events=True,
        forward_subagent_text=True,
    )


def _host_of(url: str) -> str:
    from urllib.parse import urlparse

    parsed = urlparse(url)
    return parsed.hostname or "localhost"
