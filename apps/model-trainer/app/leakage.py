"""The Python half of the `dataset_build` leakage harness (Set N, N-1.5; FEAT-006 section 3).

Two checks, and a manifest a dataset registration embeds:

  (a) **feature truncation self-test** (`feature_truncation_check`): compute the
      features on the full series and again on a truncated one, and require the
      shared prefix to be identical. A feature whose value at t changes when bars
      after t are removed uses the future.
  (b) **label/fold overlap** (`label_fold_overlap_check`): no training row's label
      span `[label_start, label_end]` may intersect a test fold's span (from the
      earliest label_start to the latest label_end among its test rows), and no
      training row may sit inside the embargo window after a test fold.

A failing report raises `LeakageDetected` (terminal reason `leakage_detected`);
`ensure_clean` is the only way a caller turns a report into a go/no-go, so a
dataset that fails is never registered by a caller that forgot to look.

`check_request` is the trainer's hook (`worker.run_training`). The trainer holds a
materialized frame, not the feature expressions, so it can run (b) and must report
(a) as `skipped` with the reason; the `dataset_build` job, which has the compute
function, calls `feature_truncation_check` itself. A frame with no `label_end`
column is recorded as `{"checked": false, "reason": ...}`, never as a pass.
"""

from __future__ import annotations

from typing import Callable

import numpy as np
import pandas as pd

from . import failures

HARNESS_VERSION = "leakage-harness-1"


class LeakageDetected(failures.TrainerFailure):
    """A dataset failed the leakage harness. It must not be registered or trained on."""

    terminal = failures.LEAKAGE_DETECTED

    def __init__(self, message: str, report: dict | None = None):
        super().__init__(message)
        self.report = report


# --------------------------------------------------------------------------- #
# (a) feature truncation
# --------------------------------------------------------------------------- #
def feature_truncation_check(
    compute: Callable[[pd.DataFrame], pd.DataFrame],
    series: pd.DataFrame,
    cuts: tuple[float, ...] = (0.5, 0.8),
    atol: float = 1e-9,
) -> dict:
    """Recompute features on truncated copies of `series`; the shared prefix must match.

    `compute(df)` returns a frame with one row per input row. NaN equals NaN (a
    warm-up gap is the same gap either way). Returns a check record; `status` is
    "fail" and `bad_features` names every column that moved.
    """
    full = compute(series)
    n = len(series)
    bad: dict[str, int] = {}
    for frac in cuts:
        m = max(1, min(n - 1, int(n * frac)))
        part = compute(series.iloc[:m])
        if len(part) != m or list(part.columns) != list(full.columns):
            return {"name": "feature_truncation", "status": "fail", "cuts": list(cuts),
                    "bad_features": ["<shape or column set changed under truncation>"]}
        a = full.iloc[:m].to_numpy(dtype=float)
        b = part.to_numpy(dtype=float)
        same = np.isclose(a, b, atol=atol, rtol=1e-9, equal_nan=True)
        for j, col in enumerate(full.columns):
            if not same[:, j].all():
                bad[col] = max(bad.get(col, 0), int((~same[:, j]).sum()))
    return {"name": "feature_truncation", "status": "fail" if bad else "pass",
            "cuts": list(cuts), "bad_features": sorted(bad), "rows_changed": bad}


# --------------------------------------------------------------------------- #
# (b) label / fold overlap
# --------------------------------------------------------------------------- #
def _fold_dict(f) -> dict:
    return f.model_dump() if hasattr(f, "model_dump") else dict(f)


def label_fold_overlap_check(
    df: pd.DataFrame,
    folds,
    embargo_bars: int = 0,
    max_examples: int = 5,
) -> dict:
    """Training labels must not overlap a test fold's span or its embargo.

    Row positions come from the fold's half-open index ranges; label spans from
    `label_start` / `label_end` (any comparable numbers: ns timestamps or bar
    indices). Violations are counted per fold with a few examples.
    """
    ls = df["label_start"].to_numpy()
    le = df["label_end"].to_numpy()
    n = len(df)
    malformed = int(np.sum(le < ls))
    violations: list[dict] = []
    for raw in folds:
        f = _fold_dict(raw)
        tr = np.arange(max(0, f["train_start"]), min(n, f["train_end"]))
        te = np.arange(max(0, f["test_start"]), min(n, f["test_end"]))
        if len(te) == 0 or len(tr) == 0:
            continue
        shared = np.intersect1d(tr, te)
        t_lo, t_hi = ls[te].min(), le[te].max()
        overlap = tr[(ls[tr] <= t_hi) & (le[tr] >= t_lo)]
        emb = tr[(tr >= te.max() + 1) & (tr < te.max() + 1 + max(0, int(embargo_bars)))]
        for kind, rows in (("row_in_train_and_test", shared), ("label_overlaps_test_span", overlap),
                           ("inside_embargo", emb)):
            if len(rows):
                violations.append({"fold": int(f["index"]), "kind": kind, "n_rows": int(len(rows)),
                                   "rows": [int(r) for r in rows[:max_examples]]})
    status = "fail" if (violations or malformed) else "pass"
    out = {"name": "label_fold_overlap", "status": status, "n_folds": len(list(folds)),
           "embargo_bars": int(embargo_bars), "violations": violations}
    if malformed:
        out["malformed_label_spans"] = malformed
    return out


# --------------------------------------------------------------------------- #
# The report and the go/no-go
# --------------------------------------------------------------------------- #
def build_report(checks: list[dict], *, engine_version: str = "unknown", data_snapshot: str = "unknown") -> dict:
    """The manifest a dataset registration embeds."""
    failed = [c["name"] for c in checks if c.get("status") == "fail"]
    return {
        "checked": True,
        "passed": not failed,
        "failed_checks": failed,
        "harness_version": HARNESS_VERSION,
        "engine_version": engine_version,
        "data_snapshot": data_snapshot,
        "checks": checks,
    }


def ensure_clean(report: dict) -> dict:
    """Return the report when it passed; raise `LeakageDetected` when it did not."""
    if report.get("checked") and not report.get("passed"):
        raise LeakageDetected(
            "the dataset failed the leakage harness: " + ", ".join(report["failed_checks"]), report
        )
    return report


def check_request(
    definition: dict,
    df: pd.DataFrame,
    folds,
    *,
    data_snapshot: str = "unknown",
) -> dict:
    """The trainer-side harness run. Returns the `leakage` metrics block.

    Raises `LeakageDetected` on a failing dataset. When it cannot check, says so.
    """
    if not folds:
        return {"checked": False, "reason": "no walk-forward folds on the request"}
    missing = [c for c in ("label_start", "label_end") if c not in df.columns]
    if missing:
        return {"checked": False, "reason": f"the dataset has no {', '.join(missing)} column(s)"}
    decl = (definition or {}).get("declaration") or {}
    derived = decl.get("derived") or {}
    embargo = int(derived.get("embargo_bars") or definition.get("_embargo_bars") or 0)
    checks = [
        label_fold_overlap_check(df, folds, embargo_bars=embargo),
        {"name": "feature_truncation", "status": "skipped",
         "reason": "the trainer holds computed features, not their expressions; the dataset_build job runs this check"},
    ]
    report = build_report(
        checks, engine_version=str(decl.get("engine_version") or "unknown"), data_snapshot=data_snapshot
    )
    return ensure_clean(report)
