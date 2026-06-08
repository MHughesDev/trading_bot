#!/usr/bin/env python3
"""Validate a release candidate JSON file against APEX promotion gates (FB-CAN-011).

Example::

    python3 scripts/validate_release_gates.py --candidate path/to/candidate.json --target live
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from orchestration.release_gating import (
    ReleaseCandidate,
    evaluate_promotion_gates,
)


def main() -> int:
    p = argparse.ArgumentParser(description="Evaluate APEX release promotion gates")
    p.add_argument(
        "--candidate",
        type=Path,
        required=True,
        help="Path to JSON describing a ReleaseCandidate",
    )
    p.add_argument(
        "--target",
        choices=["research", "simulation", "shadow", "live"],
        default="live",
        help="Target environment to gate toward",
    )
    p.add_argument(
        "--experiment-registry",
        type=Path,
        default=None,
        help="Optional path to experiment_registry.json when validating linked_experiment_ids (FB-CAN-054)",
    )
    args = p.parse_args()
    raw = json.loads(args.candidate.read_text(encoding="utf-8"))
    cand = ReleaseCandidate.model_validate(raw)
    kwargs: dict = {"target_environment": args.target}
    if args.experiment_registry is not None:
        kwargs["experiment_registry_path"] = args.experiment_registry
    result = evaluate_promotion_gates(cand, **kwargs)
    print(json.dumps(result.model_dump(mode="json"), indent=2))
    return 0 if result.allowed else 2


if __name__ == "__main__":
    raise SystemExit(main())
