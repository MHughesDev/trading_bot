#!/usr/bin/env python3
"""Environment preflight for package-index access used by setup/audit scripts."""

from __future__ import annotations

import os
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass

DEFAULT_INDEX = "https://pypi.org/simple/pip/"
TIMEOUT_SECONDS = 10


@dataclass
class CheckResult:
    ok: bool
    status: int | None
    reason: str


def _index_url() -> str:
    base = os.environ.get("PIP_INDEX_URL")
    if base:
        return base.rstrip("/") + "/pip/"
    return DEFAULT_INDEX


def _probe(url: str) -> CheckResult:
    req = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_SECONDS) as resp:
            return CheckResult(ok=True, status=resp.status, reason="reachable")
    except urllib.error.HTTPError as exc:
        return CheckResult(ok=False, status=exc.code, reason=f"HTTP {exc.code}")
    except urllib.error.URLError as exc:
        return CheckResult(ok=False, status=None, reason=str(exc.reason))


def main() -> int:
    url = _index_url()
    result = _probe(url)
    if result.ok:
        print(f"env_preflight: package index reachable ({url})")
        return 0

    proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY")
    print("env_preflight: package index connectivity check failed.", file=sys.stderr)
    print(f"  target: {url}", file=sys.stderr)
    print(f"  reason: {result.reason}", file=sys.stderr)
    if proxy:
        print(f"  proxy: {proxy}", file=sys.stderr)
    print(
        "  fix: set PIP_INDEX_URL to your reachable internal mirror (or repair proxy allowlist/credentials).",
        file=sys.stderr,
    )
    print(
        "  note: in this container, dependency install/audit cannot proceed until index connectivity works.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
