#!/usr/bin/env python3
"""Emit a JSON release evidence bundle (FB-CAN-026).

Example::

  python3 scripts/build_release_evidence_bundle.py --out /tmp/evidence.json
  python3 scripts/build_release_evidence_bundle.py --baseline app/config/default.yaml --out evidence.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from orchestration.release_evidence import build_release_evidence_bundle
from orchestration.release_gating import RollbackTarget


def main() -> int:
    p = argparse.ArgumentParser(description="Build APEX release evidence bundle JSON")
    p.add_argument(
        "--out",
        type=Path,
        help="Write bundle to this path (default: stdout)",
    )
    p.add_argument(
        "--baseline",
        type=Path,
        help="YAML file with apex_canonical to diff against (same merge as runtime)",
    )
    p.add_argument(
        "--replay-run-id",
        action="append",
        default=[],
        dest="replay_run_ids",
        help="Replay run id (repeatable)",
    )
    p.add_argument(
        "--shadow-run-id",
        action="append",
        default=[],
        dest="shadow_run_ids",
        help="Shadow run id (repeatable)",
    )
    p.add_argument(
        "--fault-stress-run-id",
        action="append",
        default=[],
        dest="fault_stress_run_ids",
        help="Replay run id under canonical fault profiles (FB-CAN-037, repeatable)",
    )
    p.add_argument(
        "--fault-profile-satisfied",
        action="append",
        default=[],
        dest="fault_profile_ids_satisfied",
        help="Canonical fault profile id exercised (repeatable; use all seven for promotion evidence)",
    )
    p.add_argument("--rollback-config-version", type=str, default=None)
    p.add_argument("--rollback-logic-version", type=str, default=None)
    p.add_argument("--rollback-instructions", type=str, default="")
    args = p.parse_args()

    rb = RollbackTarget(
        target_config_version=args.rollback_config_version,
        target_logic_version=args.rollback_logic_version,
        instructions=args.rollback_instructions or "",
    )
    bundle = build_release_evidence_bundle(
        baseline_yaml_path=args.baseline,
        replay_run_ids=args.replay_run_ids,
        shadow_run_ids=args.shadow_run_ids,
        fault_stress_run_ids=args.fault_stress_run_ids,
        fault_profile_ids_satisfied=args.fault_profile_ids_satisfied,
        rollback=rb,
    )
    text = json.dumps(bundle.model_dump(mode="json"), indent=2)
    if args.out:
        args.out.write_text(text, encoding="utf-8")
        print(f"wrote {args.out}", file=sys.stderr)
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
