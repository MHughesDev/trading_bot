#!/usr/bin/env python3
"""FB-CAN-057: semantic guardrails fire on meaningful canonical config changes."""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

import yaml

from app.config.settings import AppSettings
from app.config.canonical_config import resolve_canonical_config
from orchestration.config_diff_audit import build_canonical_config_diff_report


def _write_yaml(path: Path, doc: dict) -> None:
    path.write_text(yaml.dump(doc, sort_keys=False), encoding="utf-8")


def main() -> int:
    base_doc = {
        "apex_canonical": {
            "metadata": {
                "config_version": "ci-base",
                "config_name": "ci",
            },
            "domains": {
                "risk_sizing": {"max_total_exposure_usd": 10000.0},
                "replay": {"backtesting_slippage_bps": 5.0},
            },
        }
    }
    cur_doc = {
        "apex_canonical": {
            "metadata": {
                "config_version": "ci-current",
                "config_name": "ci",
            },
            "domains": {
                "risk_sizing": {"max_total_exposure_usd": 12000.0},
                "replay": {"backtesting_slippage_bps": 5.0},
            },
        }
    }
    with tempfile.TemporaryDirectory() as td:
        p1 = Path(td) / "base.yaml"
        p2 = Path(td) / "cur.yaml"
        _write_yaml(p1, base_doc)
        _write_yaml(p2, cur_doc)
        settings = AppSettings()
        raw_b = yaml.safe_load(p1.read_text(encoding="utf-8"))
        raw_c = yaml.safe_load(p2.read_text(encoding="utf-8"))
        b = resolve_canonical_config(settings, raw_b)
        c = resolve_canonical_config(settings, raw_c)

    rep = build_canonical_config_diff_report(b, c)
    sem = rep.get("semantic_analysis") or {}
    if not sem.get("requires_operator_review"):
        print(
            "ci_config_diff_semantics: expected requires_operator_review for risk_sizing change",
            file=sys.stderr,
        )
        print(json.dumps(rep, indent=2, default=str)[:4000], file=sys.stderr)
        return 1
    if "risk_sizing" not in (sem.get("semantic_categories") or {}):
        print("ci_config_diff_semantics: expected risk_sizing category", file=sys.stderr)
        return 1
    if not rep.get("markdown_render"):
        print("ci_config_diff_semantics: missing markdown_render", file=sys.stderr)
        return 1
    print("ci_config_diff_semantics: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
