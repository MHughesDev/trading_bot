"""Shared Streamlit helpers: API base URL and simple HTTP helpers."""

from __future__ import annotations

import os
from typing import Any

import httpx


def get_api_base() -> str:
    return os.getenv("NM_CONTROL_PLANE_URL", "http://127.0.0.1:8000").rstrip("/")


def get_control_plane_key() -> str:
    return os.getenv("NM_CONTROL_PLANE_API_KEY", "")


def get_grafana_url() -> str:
    return os.getenv("NM_GRAFANA_URL", "http://127.0.0.1:3000").rstrip("/")


def get_loki_url() -> str:
    return os.getenv("NM_LOKI_URL", "http://127.0.0.1:3100").rstrip("/")


def get_questdb_console_url() -> str:
    return os.getenv("NM_QUESTDB_CONSOLE_URL", "http://127.0.0.1:9000").rstrip("/")


def api_get_json(path: str, *, timeout: float = 10.0) -> dict[str, Any]:
    r = httpx.get(f"{get_api_base()}{path}", timeout=timeout)
    r.raise_for_status()
    return r.json()


def _mutate_headers() -> dict[str, str]:
    headers: dict[str, str] = {}
    key = get_control_plane_key()
    if key:
        headers["X-API-Key"] = key
    return headers


def api_post_json(
    path: str,
    body: dict[str, Any] | None = None,
    *,
    timeout: float = 15.0,
    require_key: bool = True,
) -> dict[str, Any]:
    headers = _mutate_headers() if require_key else {}
    r = httpx.post(
        f"{get_api_base()}{path}",
        json=body or {},
        timeout=timeout,
        headers=headers,
    )
    r.raise_for_status()
    return r.json()


def api_put_json(
    path: str,
    body: dict[str, Any] | None = None,
    *,
    timeout: float = 15.0,
    require_key: bool = True,
) -> dict[str, Any]:
    headers = _mutate_headers() if require_key else {}
    r = httpx.put(
        f"{get_api_base()}{path}",
        json=body or {},
        timeout=timeout,
        headers=headers,
    )
    r.raise_for_status()
    return r.json()


def api_delete_json(path: str, *, timeout: float = 15.0, require_key: bool = True) -> dict[str, Any]:
    headers = _mutate_headers() if require_key else {}
    r = httpx.delete(f"{get_api_base()}{path}", timeout=timeout, headers=headers)
    r.raise_for_status()
    return r.json()
