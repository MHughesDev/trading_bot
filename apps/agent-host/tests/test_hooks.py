"""Hook behaviour (AGENT-001 §12).

The NOTEBOOK tests matter most. SDK 0.2.152 has no ``PostCompact`` hook, so research
state survives a compaction only because ``PreCompact`` records that one happened and
the next ``UserPromptSubmit`` re-reads the file. If that pairing breaks, a long
session quietly forgets its own hypotheses and keeps going as though it had not.
"""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from agent_host.hooks import MAX_TOOL_OUTPUT_CHARS, SessionState, make_hooks


def run(coro):
    # `asyncio.run` rather than `get_event_loop`, which is removed on 3.12+ when no
    # loop is running and raises rather than creating one.
    return asyncio.run(coro)


def hook(state: SessionState, event: str, matcher_index: int = 0):
    matchers = make_hooks(state)[event]
    return matchers[matcher_index].hooks[0]


def test_notebook_head_stops_at_the_second_heading(tmp_path: Path):
    (tmp_path / "NOTEBOOK.md").write_text(
        "## Current state\nhypothesis A open\ntrials: 3\n\n## Older notes\nlots of detail\n",
        encoding="utf-8",
    )
    state = SessionState(str(tmp_path))
    head = state.notebook_head()
    assert "hypothesis A open" in head
    assert "Older notes" not in head, "only the first section is the live state"


def test_notebook_head_is_none_when_absent(tmp_path: Path):
    state = SessionState(str(tmp_path))
    assert state.notebook_head() is None


def test_compaction_state_round_trips(tmp_path: Path):
    (tmp_path / "NOTEBOOK.md").write_text(
        "## Current state\ntrials: 7\nopen: momentum decay\n", encoding="utf-8"
    )
    state = SessionState(str(tmp_path))

    # Nothing to restore before a compaction happens.
    submit = hook(state, "UserPromptSubmit")
    assert run(submit({}, None, None)) == {}

    pre = hook(state, "PreCompact")
    out = run(pre({"trigger": "auto", "custom_instructions": None}, None, None))
    assert "NOTEBOOK.md" in out["hookSpecificOutput"]["customInstructions"]
    assert state.compaction_pending is True

    restored = run(submit({}, None, None))
    context = restored["hookSpecificOutput"]["additionalContext"]
    assert "trials: 7" in context
    assert "momentum decay" in context

    # Only once: the state is already back in context after the first turn.
    assert run(submit({}, None, None)) == {}


def test_precompact_keeps_instructions_the_caller_already_set(tmp_path: Path):
    state = SessionState(str(tmp_path))
    pre = hook(state, "PreCompact")
    out = run(pre({"custom_instructions": "keep the plan"}, None, None))
    instructions = out["hookSpecificOutput"]["customInstructions"]
    assert "keep the plan" in instructions, "a caller's instructions must not be dropped"
    assert "NOTEBOOK.md" in instructions


def test_large_tool_output_is_spilled_to_a_file(tmp_path: Path):
    state = SessionState(str(tmp_path))
    state.overflow_dir = tmp_path / "out"
    post = hook(state, "PostToolUse")

    payload = {"tool_response": "x" * (MAX_TOOL_OUTPUT_CHARS + 1_000)}
    out = run(post(payload, None, None))
    context = out["hookSpecificOutput"]["additionalContext"]

    assert "output truncated" in context
    written = list((tmp_path / "out").glob("*.txt"))
    assert len(written) == 1
    assert len(written[0].read_text(encoding="utf-8")) == MAX_TOOL_OUTPUT_CHARS + 1_000
    assert len(context) < 2_000, "the pointer must be far smaller than the output"


def test_small_tool_output_is_passed_through_untouched(tmp_path: Path):
    state = SessionState(str(tmp_path))
    state.overflow_dir = tmp_path / "out"
    post = hook(state, "PostToolUse")
    assert run(post({"tool_response": "short"}, None, None)) == {}
    assert not (tmp_path / "out").exists(), "no file for output that fits"


@pytest.mark.parametrize("command", ["docker ps", "sudo rm -rf /", "cat /proc/self/environ"])
def test_bash_denies_reaching_around_the_platform(tmp_path: Path, command: str):
    state = SessionState(str(tmp_path))
    pre = hook(state, "PreToolUse")
    out = run(pre({"tool_input": {"command": command}}, None, None))
    assert out["hookSpecificOutput"]["permissionDecision"] == "deny"
    assert out["hookSpecificOutput"]["permissionDecisionReason"]


def test_bash_denies_external_http_and_says_what_to_use(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("TBOT_API_URL", "http://platform:7080")
    state = SessionState(str(tmp_path))
    pre = hook(state, "PreToolUse")
    out = run(pre({"tool_input": {"command": "curl https://example.com/data.csv"}}, None, None))
    reason = out["hookSpecificOutput"]["permissionDecisionReason"]
    assert "tbot" in reason, "a denial without an alternative just becomes a retry"
    assert "exploration ledger" in reason


def test_bash_allows_calls_to_the_platform(tmp_path: Path, monkeypatch):
    monkeypatch.setenv("TBOT_API_URL", "http://platform:7080")
    state = SessionState(str(tmp_path))
    pre = hook(state, "PreToolUse")
    out = run(pre({"tool_input": {"command": "curl http://platform:7080/api/jobs"}}, None, None))
    assert out == {}, "the platform itself is reachable"


def test_bash_allows_ordinary_analysis(tmp_path: Path):
    state = SessionState(str(tmp_path))
    pre = hook(state, "PreToolUse")
    assert run(pre({"tool_input": {"command": "python analyse.py"}}, None, None)) == {}
