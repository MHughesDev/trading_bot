"""Request-scoped operator user id for multi-tenant data paths (FB-UX-007)."""

from __future__ import annotations

from contextvars import ContextVar, Token
from typing import Callable, TypeVar

_current_user_id: ContextVar[int | None] = ContextVar("tenant_user_id", default=None)

_R = TypeVar("_R")


def get_current_user_id() -> int | None:
    return _current_user_id.get()


def set_current_user_id(user_id: int | None) -> None:
    _current_user_id.set(user_id)


def run_with_user_id(user_id: int | None, fn: Callable[[], _R]) -> _R:
    """Run ``fn`` with ``user_id`` bound (e.g. asset init background thread)."""
    token = _current_user_id.set(user_id)
    try:
        return fn()
    finally:
        _current_user_id.reset(token)


def set_current_user_id_token(user_id: int | None) -> Token[int | None]:
    """Return reset token for FastAPI dependency ``finally`` block."""
    return _current_user_id.set(user_id)


def reset_current_user_id_token(token: Token[int | None]) -> None:
    _current_user_id.reset(token)
