#!/usr/bin/env python3
"""FB-CAN-056: assert canonical monitoring domain metric coverage."""

from __future__ import annotations

import json
import sys

from observability.monitoring_domain_checklist import validate_monitoring_domain_coverage


def main() -> int:
    ok, reasons = validate_monitoring_domain_coverage()
    if not ok:
        print("ci_monitoring_domain_checklist: FAILED", file=sys.stderr)
        print(json.dumps({"ok": False, "reasons": reasons}, indent=2), file=sys.stderr)
        return 1
    print("ci_monitoring_domain_checklist: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
