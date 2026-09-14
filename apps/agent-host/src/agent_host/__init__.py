"""The agent's in-container host process (AGENT-001 §8)."""

from .config import SessionConfig, build_options
from .hooks import SessionState, make_hooks

__all__ = ["SessionConfig", "build_options", "SessionState", "make_hooks"]
