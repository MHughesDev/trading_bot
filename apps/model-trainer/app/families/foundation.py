"""Foundation-model baselines: Chronos, TimesFM, TTM, zero-shot (N-2.8, MD-06 MAY).

These are BASELINES ONLY. The registry marks them `role=foundation_baseline`,
`selectable_as_candidate=false`, and `registry.resolve(..., as_candidate=True)` refuses them.

* The package is imported lazily. When it is absent the result is a typed `unavailable` row
  (`status: "unavailable"`, `scores: null`, a reason) - never a made-up number.
* Every row carries `contamination: {"model_cutoff", "contaminated", "window_start",
  "window_end", "note"}`. A model whose training cutoff is after the evaluation window start has
  seen the window and is flagged `contaminated: true`; an unknown window start is flagged too
  (conservative), and the row text names the window.
* Rows have the baseline wire shape documented in app/families/baseline.py, so they sit in
  `metrics["family_baselines"]` beside the others.
"""

from __future__ import annotations

import importlib
from typing import Callable

import numpy as np
import pandas as pd

from .. import failures
from . import registry

VARIANTS = ("chronos", "timesfm", "ttm")
_PACKAGES = {"chronos": "chronos", "timesfm": "timesfm", "ttm": "tsfm_public"}

# Test/ops hook: variant -> fn(context: np.ndarray, horizon: int) -> float point forecast.
_BACKENDS: dict[str, Callable] = {}


class FoundationUnavailable(failures.TrainerFailure):
    """The foundation package is not installed. Carries the typed result."""

    terminal = failures.DEPENDENCY_FAILURE

    def __init__(self, variant: str, reason: str):
        super().__init__(f"foundation baseline {variant!r} unavailable: {reason}")
        self.variant, self.reason = variant, reason
        self.result = {"status": "unavailable", "variant": variant, "reason": reason}


def register_backend(variant: str, fn: Callable | None) -> None:
    if fn is None:
        _BACKENDS.pop(variant, None)
    else:
        _BACKENDS[variant] = fn


def _entry(variant: str) -> dict:
    return registry.entry("foundation", variant)


def window_bounds(definition: dict, df: pd.DataFrame) -> tuple[str | None, str | None]:
    decl = definition.get("declaration") or {}
    for src in (decl.get("window"), (definition.get("dataset") or {}).get("window"), definition.get("window")):
        if isinstance(src, dict) and src.get("start"):
            return str(src["start"])[:10], (str(src["end"])[:10] if src.get("end") else None)
    if "ts_ns" in df.columns and len(df):
        t = df["ts_ns"].to_numpy()
        f = lambda v: str(pd.to_datetime(int(v), unit="ns").date())  # noqa: E731
        return f(t.min()), f(t.max())
    return None, None


def contamination(variant: str, definition: dict, df: pd.DataFrame) -> dict:
    cutoff = _entry(variant)["contamination"]["model_cutoff"]
    start, end = window_bounds(definition, df)
    contaminated = True if start is None else cutoff > start
    if start is None:
        note = f"{variant} cutoff {cutoff}; evaluation window unknown, flagged contaminated"
    elif contaminated:
        note = f"{variant} cutoff {cutoff} is after window start {start}: the model has seen part of {start}..{end}"
    else:
        note = f"{variant} cutoff {cutoff} precedes window start {start}"
    return {"model_cutoff": cutoff, "contaminated": contaminated, "window_start": start,
            "window_end": end, "note": note}


def _backend(variant: str) -> Callable:
    if variant in _BACKENDS:
        return _BACKENDS[variant]
    pkg = _PACKAGES[variant]
    try:
        mod = importlib.import_module(pkg)
    except ImportError as e:
        raise FoundationUnavailable(variant, f"package {pkg!r} is not installed ({e})") from e
    if variant == "chronos":
        try:
            import torch

            pipe = mod.ChronosPipeline.from_pretrained("amazon/chronos-t5-small", device_map="cpu",
                                                       torch_dtype=torch.float32)
        except Exception as e:  # noqa: BLE001
            raise FoundationUnavailable(variant, f"could not load chronos weights: {e}") from e

        def fn(context: np.ndarray, horizon: int) -> float:
            import torch as _t

            s = pipe.predict(_t.tensor(context[-512:], dtype=_t.float32), horizon)
            return float(np.median(s[0].numpy()[:, -1]))

        return fn
    raise FoundationUnavailable(variant, f"adapter for {variant!r} is not wired in this build")


def baseline_row(variant: str, definition: dict, df: pd.DataFrame, folds=None) -> dict:
    """A scorecard row for one foundation baseline on the same folds as every other baseline."""
    from . import baseline as bl

    cont = contamination(variant, definition, df)
    row = {"name": variant, "family": "foundation", "variant": variant, "status": "ok", "reason": None,
           "folds": [], "scores": None, "per_fold": {}, "series": None, "contamination": cont}
    try:
        fn = _backend(variant)
    except FoundationUnavailable as e:
        row.update(status="unavailable", reason=e.reason)
        return row
    kind = bl.target_kind(definition)
    label_col = "label" if "label" in df.columns else df.columns[-1]
    y = df[label_col].to_numpy(float)
    fs = bl.fold_slices(folds, len(df), definition)
    horizon = int(((definition.get("declaration") or {}).get("derived") or {}).get("horizon_bars", 1) or 1)
    ys, ps, means = [], [], []
    full = np.full(len(df), np.nan)
    for idx, tr, te in fs:
        ctx_end = tr.stop
        hist = y[:ctx_end]
        hist = hist[np.isfinite(hist)]
        pred = np.array([fn(hist, horizon)] * len(y[te]), float)  # zero-shot from the train context
        full[te] = pred
        ys.append(y[te])
        ps.append(pred)
        means.append(float(hist.mean()))
        row["per_fold"][str(idx)] = bl.score_point(kind, y[te], pred, means[-1])
    row["folds"] = [i for i, _, _ in fs]
    row["scores"] = bl._pool(kind, ys, ps, means)
    pts = bl.series_list(full)
    row["series"] = {"point": pts, "sigma": pts if kind == "realized_vol" else None}
    return row


def rows(definition: dict, df: pd.DataFrame, folds=None, variants=None) -> list[dict]:
    return [baseline_row(v, definition, df, folds) for v in (variants or VARIANTS)]


def _train(variant: str, definition, df, emit):
    """Registry entry point. A foundation model is not trained and never produces a candidate
    artifact: either it is unavailable (typed) or it is refused as a candidate."""
    try:
        _backend(variant)
    except FoundationUnavailable:
        raise
    raise registry.NotSelectableAsCandidate(
        f"foundation/{variant} is a baseline only; it is evaluated beside a candidate, never trained as one"
    )


def train_chronos(definition, df, emit):
    return _train("chronos", definition, df, emit)


def train_timesfm(definition, df, emit):
    return _train("timesfm", definition, df, emit)


def train_ttm(definition, df, emit):
    return _train("ttm", definition, df, emit)
