"""Optional HTTP probes for scaffold microservice processes (operator visibility)."""

from __future__ import annotations

import os
from typing import Any

# Default ports match infra/docker-compose.microservices.yml when using host networking from the host.
_DEFAULT_PROBES: tuple[tuple[str, int], ...] = (
    ("live_runtime", 8208),
    ("market_data_service", 8206),
    ("feature_service", 8205),
    ("decision_service", 8203),
    ("risk_service", 8204),
    ("execution_gateway_service", 8202),
    ("observability_writer_service", 8207),
)


def probe_microservices_health(
    *,
    host: str | None = None,
    timeout_s: float = 1.5,
) -> dict[str, Any]:
    """GET /healthz on each default scaffold port; unreachable services report ok=false."""
    import httpx

    h = (host or os.getenv("NM_MICROSERVICES_HEALTH_HOST", "127.0.0.1")).strip()
    override = os.getenv("NM_MICROSERVICES_HEALTH_PORTS", "").strip()
    if override:
        pairs: list[tuple[str, int]] = []
        for part in override.split(","):
            part = part.strip()
            if ":" in part:
                name, port_s = part.rsplit(":", 1)
                pairs.append((name.strip(), int(port_s)))
            else:
                pairs.append((f"service_{len(pairs)}", int(part)))
    else:
        pairs = list(_DEFAULT_PROBES)

    services: dict[str, Any] = {}
    for name, port in pairs:
        url = f"http://{h}:{port}/healthz"
        try:
            r = httpx.get(url, timeout=timeout_s)
            services[name] = {
                "ok": r.status_code == 200,
                "status_code": r.status_code,
                "url": url,
            }
        except Exception as exc:  # noqa: BLE001
            services[name] = {"ok": False, "error": str(exc)[:300], "url": url}

    return {
        "host": h,
        "services": services,
        "note": "Scaffold processes only; set NM_MICROSERVICES_HEALTH_PORTS=name:port,... to override.",
    }
