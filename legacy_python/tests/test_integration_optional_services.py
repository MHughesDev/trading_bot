"""
Optional integration checks against local services (HG-13 / FB-I2).

Run with: NM_INTEGRATION_SERVICES=1 pytest tests/test_integration_optional_services.py
Or start services: docker compose -f infra/docker-compose.yml up -d
"""

from __future__ import annotations

import os

import pytest

REDIS_URL = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
QUESTDB_HOST = os.getenv("NM_QUESTDB_HOST", "127.0.0.1")
QDRANT_URL = os.getenv("NM_QDRANT_URL", "http://127.0.0.1:6333")


def _integration_enabled() -> bool:
    return os.getenv("NM_INTEGRATION_SERVICES", "").lower() in ("1", "true", "yes")


pytestmark = pytest.mark.skipif(
    not _integration_enabled(),
    reason="Set NM_INTEGRATION_SERVICES=1 and start docker compose services",
)


def test_redis_ping() -> None:
    import redis

    r = redis.Redis.from_url(REDIS_URL, socket_connect_timeout=2)
    assert r.ping() is True


def test_questdb_port_open() -> None:
    import socket

    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(2)
    try:
        s.connect((QUESTDB_HOST, 8812))
    finally:
        s.close()


def test_qdrant_collections_endpoint() -> None:
    import httpx

    r = httpx.get(f"{QDRANT_URL}/collections", timeout=5.0)
    assert r.status_code == 200
