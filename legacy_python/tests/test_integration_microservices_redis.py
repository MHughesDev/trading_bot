"""
Optional cross-process microservice handoff: Redis Streams + execution gateway (subprocess).

Requires Redis (e.g. ``docker compose -f infra/docker-compose.yml up -d redis``).

Run with: ``NM_INTEGRATION_SERVICES=1 python -m pytest tests/test_integration_microservices_redis.py -q``
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
from typing import Any

import pytest

pytest.importorskip("httpx")

import httpx

from services.runtime_bridge import RuntimeHandoffBridge
from shared.messaging.factory import create_message_bus


def _integration_enabled() -> bool:
    return os.getenv("NM_INTEGRATION_SERVICES", "").lower() in ("1", "true", "yes")


pytestmark = pytest.mark.skipif(
    not _integration_enabled(),
    reason="Set NM_INTEGRATION_SERVICES=1 and start Redis (see infra/docker-compose.yml)",
)


def _wait_http(url: str, *, timeout_s: float = 15.0) -> None:
    deadline = time.monotonic() + timeout_s
    last_exc: Exception | None = None
    while time.monotonic() < deadline:
        try:
            r = httpx.get(url, timeout=1.0)
            if r.status_code == 200:
                return
        except Exception as exc:  # noqa: BLE001
            last_exc = exc
        time.sleep(0.15)
    raise AssertionError(f"HTTP {url} did not become ready: {last_exc!r}")


def test_redis_runtime_bridge_external_gateway_subprocess() -> None:
    """Producer publishes handoff to Redis; separate uvicorn gateway consumes and submits (stub)."""
    redis_url = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
    try:
        import redis as redis_mod

        redis_mod.Redis.from_url(redis_url, socket_connect_timeout=2).ping()
    except Exception as exc:  # noqa: BLE001
        pytest.skip(f"Redis not reachable at {redis_url!r}: {exc}")

    secret = "integration-test-risk-secret-32chars!!"

    gw_env = os.environ.copy()
    gw_env.update(
        {
            "NM_MESSAGING_BACKEND": "redis_streams",
            "NM_REDIS_URL": redis_url,
            "NM_RISK_SIGNING_SECRET": secret,
            "NM_EXECUTION_GATEWAY_SUBMIT": "true",
            "NM_EXECUTION_ADAPTER": "stub",
            "NM_ALLOW_UNSIGNED_EXECUTION": "false",
        }
    )

    proc = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "uvicorn",
            "services.execution_gateway_service.main:app",
            "--host",
            "127.0.0.1",
            "--port",
            "18765",
        ],
        env=gw_env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        _wait_http("http://127.0.0.1:18765/healthz")

        prod_env = os.environ.copy()
        prod_env.update(
            {
                "NM_MESSAGING_BACKEND": "redis_streams",
                "NM_REDIS_URL": redis_url,
                "NM_RISK_SIGNING_SECRET": secret,
            }
        )
        _keys = list(prod_env.keys())
        _saved = {k: os.environ.get(k) for k in _keys}
        try:
            os.environ.update(prod_env)
            bridge = RuntimeHandoffBridge(
                create_message_bus(),
                execution_gateway_mode="external",
            )
            bridge.process_feature_row(
                {
                    "symbol": "BTC/USD",
                    "direction": 1,
                    "size_fraction": 0.1,
                    "route_id": "SCALPING",
                    "mid_price": 50_000.0,
                    "spread_bps": 5.0,
                }
            )
        finally:
            for k in _keys:
                v = _saved.get(k)
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

        deadline = time.monotonic() + 20.0
        recent: dict[str, Any] = {}
        while time.monotonic() < deadline:
            r = httpx.get("http://127.0.0.1:18765/events/recent", timeout=2.0)
            r.raise_for_status()
            recent = r.json()
            if recent.get("submitted_orders"):
                break
            time.sleep(0.2)

        assert recent.get("submitted_orders"), f"gateway saw no orders: {recent!r}"
        first = recent["submitted_orders"][0]
        if isinstance(first, dict) and "order_intent" in first:
            assert first["order_intent"]["symbol"] == "BTC/USD"
        else:
            assert first.get("symbol") == "BTC/USD"
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        err = proc.stderr.read().decode() if proc.stderr else ""
        if proc.returncode not in (0, -15) and err:
            pytest.fail(f"uvicorn stderr: {err[:2000]}")


def test_redis_external_gateway_mock_alpaca_paper_path() -> None:
    """Paper execution path without Alpaca SDK: mock_alpaca_paper adapter + OrderIntent signing."""
    redis_url = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
    try:
        import redis as redis_mod

        redis_mod.Redis.from_url(redis_url, socket_connect_timeout=2).ping()
    except Exception as exc:  # noqa: BLE001
        pytest.skip(f"Redis not reachable at {redis_url!r}: {exc}")

    secret = "integration-mock-alpaca-secret-32b!!"

    gw_env = os.environ.copy()
    gw_env.update(
        {
            "NM_MESSAGING_BACKEND": "redis_streams",
            "NM_REDIS_URL": redis_url,
            "NM_RISK_SIGNING_SECRET": secret,
            "NM_EXECUTION_GATEWAY_SUBMIT": "true",
            "NM_EXECUTION_ADAPTER": "mock_alpaca_paper",
            "NM_EXECUTION_MODE": "paper",
            "NM_ALLOW_UNSIGNED_EXECUTION": "false",
        }
    )

    proc = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "uvicorn",
            "services.execution_gateway_service.main:app",
            "--host",
            "127.0.0.1",
            "--port",
            "18766",
        ],
        env=gw_env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        _wait_http("http://127.0.0.1:18766/healthz")

        prod_env = os.environ.copy()
        prod_env.update(
            {
                "NM_MESSAGING_BACKEND": "redis_streams",
                "NM_REDIS_URL": redis_url,
                "NM_RISK_SIGNING_SECRET": secret,
            }
        )
        _keys = list(prod_env.keys())
        _saved = {k: os.environ.get(k) for k in _keys}
        try:
            os.environ.update(prod_env)
            bridge = RuntimeHandoffBridge(
                create_message_bus(),
                execution_gateway_mode="external",
            )
            bridge.process_feature_row(
                {
                    "symbol": "BTC-USD",
                    "direction": 1,
                    "size_fraction": 0.1,
                    "route_id": "SCALPING",
                    "mid_price": 50_000.0,
                    "spread_bps": 5.0,
                }
            )
        finally:
            for k in _keys:
                v = _saved.get(k)
                if v is None:
                    os.environ.pop(k, None)
                else:
                    os.environ[k] = v

        deadline = time.monotonic() + 20.0
        recent: dict[str, Any] = {}
        while time.monotonic() < deadline:
            r = httpx.get("http://127.0.0.1:18766/events/recent", timeout=2.0)
            r.raise_for_status()
            recent = r.json()
            if recent.get("submitted_orders"):
                break
            time.sleep(0.2)

        assert recent.get("submitted_orders")
        first = recent["submitted_orders"][0]
        assert isinstance(first, dict)
        assert "ack" in first
        assert first["ack"]["adapter"] == "mock_alpaca_paper"
        assert first["ack"]["order_id"].startswith("mock-")
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        err = proc.stderr.read().decode() if proc.stderr else ""
        if proc.returncode not in (0, -15) and err:
            pytest.fail(f"uvicorn stderr: {err[:2000]}")


def test_redis_three_process_decision_risk_gateway_chain() -> None:
    """Decision → Risk → Execution gateway as separate uvicorn processes (Redis only)."""
    redis_url = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
    try:
        import redis as redis_mod

        redis_mod.Redis.from_url(redis_url, socket_connect_timeout=2).ping()
    except Exception as exc:  # noqa: BLE001
        pytest.skip(f"Redis not reachable at {redis_url!r}: {exc}")

    secret = "integration-3proc-chain-secret-32chars!!"
    base = {
        "NM_MESSAGING_BACKEND": "redis_streams",
        "NM_REDIS_URL": redis_url,
        "NM_RISK_SIGNING_SECRET": secret,
        "NM_EXECUTION_GATEWAY_SUBMIT": "true",
        "NM_EXECUTION_ADAPTER": "stub",
        "NM_ALLOW_UNSIGNED_EXECUTION": "false",
    }

    decision_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.decision_service.main:app", "--host", "127.0.0.1", "--port", "18771"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    risk_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.risk_service.main:app", "--host", "127.0.0.1", "--port", "18772"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    gw_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.execution_gateway_service.main:app", "--host", "127.0.0.1", "--port", "18773"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        _wait_http("http://127.0.0.1:18771/healthz")
        _wait_http("http://127.0.0.1:18772/healthz")
        _wait_http("http://127.0.0.1:18773/healthz")

        for url, name in (
            ("http://127.0.0.1:18771/messaging", "decision"),
            ("http://127.0.0.1:18772/messaging", "risk"),
            ("http://127.0.0.1:18773/messaging", "gateway"),
        ):
            r = httpx.get(url, timeout=2.0)
            r.raise_for_status()
            assert r.json().get("messaging_backend") == "redis_streams", name

        r = httpx.post(
            "http://127.0.0.1:18771/ingest/features-row",
            json={
                "symbol": "BTC-USD",
                "direction": 1,
                "size_fraction": 0.1,
                "route_id": "SCALPING",
                "mid_price": 50_000.0,
                "spread_bps": 5.0,
            },
            timeout=5.0,
        )
        r.raise_for_status()

        deadline = time.monotonic() + 25.0
        recent: dict[str, Any] = {}
        while time.monotonic() < deadline:
            recent = httpx.get("http://127.0.0.1:18773/events/recent", timeout=2.0).json()
            if recent.get("submitted_orders"):
                break
            time.sleep(0.2)

        assert recent.get("submitted_orders"), f"gateway empty: {recent!r}"
        first = recent["submitted_orders"][0]
        if isinstance(first, dict) and "order_intent" in first:
            assert first["order_intent"]["symbol"] == "BTC-USD"
    finally:
        for p in (gw_proc, risk_proc, decision_proc):
            p.terminate()
        for p in (gw_proc, risk_proc, decision_proc):
            try:
                p.wait(timeout=8)
            except subprocess.TimeoutExpired:
                p.kill()
        for p in (gw_proc, risk_proc, decision_proc):
            err = p.stderr.read().decode() if p.stderr else ""
            if p.returncode not in (0, -15) and err:
                pytest.fail(f"uvicorn stderr: {err[:1500]}")


def test_redis_four_process_feature_decision_risk_gateway_chain() -> None:
    """Feature → Decision → Risk → Gateway (market tick ingest on feature service)."""
    redis_url = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
    try:
        import redis as redis_mod

        redis_mod.Redis.from_url(redis_url, socket_connect_timeout=2).ping()
    except Exception as exc:  # noqa: BLE001
        pytest.skip(f"Redis not reachable at {redis_url!r}: {exc}")

    secret = "integration-4proc-chain-secret-32chars!!"
    base = {
        "NM_MESSAGING_BACKEND": "redis_streams",
        "NM_REDIS_URL": redis_url,
        "NM_RISK_SIGNING_SECRET": secret,
        "NM_EXECUTION_GATEWAY_SUBMIT": "true",
        "NM_EXECUTION_ADAPTER": "stub",
        "NM_ALLOW_UNSIGNED_EXECUTION": "false",
    }

    feature_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.feature_service.main:app", "--host", "127.0.0.1", "--port", "18781"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    decision_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.decision_service.main:app", "--host", "127.0.0.1", "--port", "18782"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    risk_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.risk_service.main:app", "--host", "127.0.0.1", "--port", "18783"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    gw_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.execution_gateway_service.main:app", "--host", "127.0.0.1", "--port", "18784"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        for port in (18781, 18782, 18783, 18784):
            _wait_http(f"http://127.0.0.1:{port}/healthz")

        for url, name in (
            ("http://127.0.0.1:18781/messaging", "feature"),
            ("http://127.0.0.1:18782/messaging", "decision"),
            ("http://127.0.0.1:18783/messaging", "risk"),
            ("http://127.0.0.1:18784/messaging", "gateway"),
        ):
            r = httpx.get(url, timeout=2.0)
            r.raise_for_status()
            assert r.json().get("messaging_backend") == "redis_streams", name

        r = httpx.post(
            "http://127.0.0.1:18781/ingest/market-tick",
            json={
                "symbol": "BTC-USD",
                "direction": 1,
                "size_fraction": 0.1,
                "route_id": "SCALPING",
                "mid_price": 50_000.0,
                "spread_bps": 5.0,
            },
            timeout=5.0,
        )
        r.raise_for_status()

        deadline = time.monotonic() + 30.0
        recent: dict[str, Any] = {}
        while time.monotonic() < deadline:
            recent = httpx.get("http://127.0.0.1:18784/events/recent", timeout=2.0).json()
            if recent.get("submitted_orders"):
                break
            time.sleep(0.2)

        assert recent.get("submitted_orders"), f"gateway empty: {recent!r}"
        first = recent["submitted_orders"][0]
        if isinstance(first, dict) and "order_intent" in first:
            assert first["order_intent"]["symbol"] == "BTC-USD"
    finally:
        for p in (gw_proc, risk_proc, decision_proc, feature_proc):
            p.terminate()
        for p in (gw_proc, risk_proc, decision_proc, feature_proc):
            try:
                p.wait(timeout=8)
            except subprocess.TimeoutExpired:
                p.kill()
        for p in (gw_proc, risk_proc, decision_proc, feature_proc):
            err = p.stderr.read().decode() if p.stderr else ""
            if p.returncode not in (0, -15) and err:
                pytest.fail(f"uvicorn stderr: {err[:1500]}")


def test_redis_five_process_market_data_through_gateway_chain() -> None:
    """Market data → Feature → Decision → Risk → Gateway (raw tick on market_data service)."""
    redis_url = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
    try:
        import redis as redis_mod

        redis_mod.Redis.from_url(redis_url, socket_connect_timeout=2).ping()
    except Exception as exc:  # noqa: BLE001
        pytest.skip(f"Redis not reachable at {redis_url!r}: {exc}")

    secret = "integration-5proc-chain-secret-32chars!!"
    base = {
        "NM_MESSAGING_BACKEND": "redis_streams",
        "NM_REDIS_URL": redis_url,
        "NM_RISK_SIGNING_SECRET": secret,
        "NM_EXECUTION_GATEWAY_SUBMIT": "true",
        "NM_EXECUTION_ADAPTER": "stub",
        "NM_ALLOW_UNSIGNED_EXECUTION": "false",
    }

    md_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.market_data_service.main:app", "--host", "127.0.0.1", "--port", "18791"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    feature_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.feature_service.main:app", "--host", "127.0.0.1", "--port", "18792"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    decision_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.decision_service.main:app", "--host", "127.0.0.1", "--port", "18793"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    risk_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.risk_service.main:app", "--host", "127.0.0.1", "--port", "18794"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    gw_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.execution_gateway_service.main:app", "--host", "127.0.0.1", "--port", "18795"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        for port in (18791, 18792, 18793, 18794, 18795):
            _wait_http(f"http://127.0.0.1:{port}/healthz")

        for url, name in (
            ("http://127.0.0.1:18791/messaging", "market_data"),
            ("http://127.0.0.1:18792/messaging", "feature"),
            ("http://127.0.0.1:18793/messaging", "decision"),
            ("http://127.0.0.1:18794/messaging", "risk"),
            ("http://127.0.0.1:18795/messaging", "gateway"),
        ):
            r = httpx.get(url, timeout=2.0)
            r.raise_for_status()
            assert r.json().get("messaging_backend") == "redis_streams", name

        r = httpx.post(
            "http://127.0.0.1:18791/ingest/raw-tick",
            json={
                "symbol": "BTC-USD",
                "direction": 1,
                "size_fraction": 0.1,
                "route_id": "SCALPING",
                "mid_price": 50_000.0,
                "spread_bps": 5.0,
            },
            timeout=5.0,
        )
        r.raise_for_status()

        deadline = time.monotonic() + 35.0
        recent: dict[str, Any] = {}
        while time.monotonic() < deadline:
            recent = httpx.get("http://127.0.0.1:18795/events/recent", timeout=2.0).json()
            if recent.get("submitted_orders"):
                break
            time.sleep(0.2)

        assert recent.get("submitted_orders"), f"gateway empty: {recent!r}"
        first = recent["submitted_orders"][0]
        if isinstance(first, dict) and "order_intent" in first:
            assert first["order_intent"]["symbol"] == "BTC-USD"
    finally:
        for p in (gw_proc, risk_proc, decision_proc, feature_proc, md_proc):
            p.terminate()
        for p in (gw_proc, risk_proc, decision_proc, feature_proc, md_proc):
            try:
                p.wait(timeout=8)
            except subprocess.TimeoutExpired:
                p.kill()
        for p in (gw_proc, risk_proc, decision_proc, feature_proc, md_proc):
            err = p.stderr.read().decode() if p.stderr else ""
            if p.returncode not in (0, -15) and err:
                pytest.fail(f"uvicorn stderr: {err[:1500]}")


def test_redis_six_process_chain_includes_observability_writer() -> None:
    """Same as five-process chain plus observability_writer consuming execution events from Redis."""
    redis_url = os.getenv("NM_REDIS_URL", "redis://127.0.0.1:6379/0")
    try:
        import redis as redis_mod

        redis_mod.Redis.from_url(redis_url, socket_connect_timeout=2).ping()
    except Exception as exc:  # noqa: BLE001
        pytest.skip(f"Redis not reachable at {redis_url!r}: {exc}")

    secret = "integration-6proc-obs-secret-32chars!!"
    base = {
        "NM_MESSAGING_BACKEND": "redis_streams",
        "NM_REDIS_URL": redis_url,
        "NM_RISK_SIGNING_SECRET": secret,
        "NM_EXECUTION_GATEWAY_SUBMIT": "true",
        "NM_EXECUTION_ADAPTER": "stub",
        "NM_ALLOW_UNSIGNED_EXECUTION": "false",
    }

    md_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.market_data_service.main:app", "--host", "127.0.0.1", "--port", "18801"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    feature_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.feature_service.main:app", "--host", "127.0.0.1", "--port", "18802"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    decision_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.decision_service.main:app", "--host", "127.0.0.1", "--port", "18803"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    risk_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.risk_service.main:app", "--host", "127.0.0.1", "--port", "18804"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    gw_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.execution_gateway_service.main:app", "--host", "127.0.0.1", "--port", "18805"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    obs_proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "services.observability_writer_service.main:app", "--host", "127.0.0.1", "--port", "18806"],
        env={**os.environ, **base},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    try:
        for port in (18801, 18802, 18803, 18804, 18805, 18806):
            _wait_http(f"http://127.0.0.1:{port}/healthz")

        for url, name in (
            ("http://127.0.0.1:18801/messaging", "market_data"),
            ("http://127.0.0.1:18802/messaging", "feature"),
            ("http://127.0.0.1:18803/messaging", "decision"),
            ("http://127.0.0.1:18804/messaging", "risk"),
            ("http://127.0.0.1:18805/messaging", "gateway"),
            ("http://127.0.0.1:18806/messaging", "observability_writer"),
        ):
            r = httpx.get(url, timeout=2.0)
            r.raise_for_status()
            assert r.json().get("messaging_backend") == "redis_streams", name

        r = httpx.post(
            "http://127.0.0.1:18801/ingest/raw-tick",
            json={
                "symbol": "BTC-USD",
                "direction": 1,
                "size_fraction": 0.1,
                "route_id": "SCALPING",
                "mid_price": 50_000.0,
                "spread_bps": 5.0,
            },
            timeout=5.0,
        )
        r.raise_for_status()

        deadline = time.monotonic() + 40.0
        recent_gw: dict[str, Any] = {}
        recent_obs: dict[str, Any] = {}
        while time.monotonic() < deadline:
            recent_gw = httpx.get("http://127.0.0.1:18805/events/recent", timeout=2.0).json()
            recent_obs = httpx.get("http://127.0.0.1:18806/events/recent", timeout=2.0).json()
            if recent_gw.get("submitted_orders") and (
                recent_obs.get("execution_acks") or recent_obs.get("execution_fills")
            ):
                break
            time.sleep(0.2)

        assert recent_gw.get("submitted_orders"), f"gateway empty: {recent_gw!r}"
        assert recent_obs.get("execution_acks") or recent_obs.get("execution_fills"), (
            f"observability writer saw no execution events: {recent_obs!r}"
        )
    finally:
        for p in (obs_proc, gw_proc, risk_proc, decision_proc, feature_proc, md_proc):
            p.terminate()
        for p in (obs_proc, gw_proc, risk_proc, decision_proc, feature_proc, md_proc):
            try:
                p.wait(timeout=8)
            except subprocess.TimeoutExpired:
                p.kill()
        for p in (obs_proc, gw_proc, risk_proc, decision_proc, feature_proc, md_proc):
            err = p.stderr.read().decode() if p.stderr else ""
            if p.returncode not in (0, -15) and err:
                pytest.fail(f"uvicorn stderr: {err[:1500]}")