"""Event sampling for training frames (Set N, N-1.7; FEAT-006 section 3).

`declaration.sampling` is one of:

    "every_bar" | null                      every row (the default; frame untouched)
    {"kind": "cusum", "h": 0.02}            symmetric CUSUM filter on returns
    {"kind": "cusum", "h": "2sigma"}        h = 2 x the causal (expanding) std of returns
    {"kind": "strategy_events", "entry_timestamps": [ts_ns, ...]}

and the training frame is subsampled to the event rows. Each kept row keeps all
its columns, `label_start` / `label_end` included, so purge and embargo still see
the labels' true spans. Row positions change when a frame is thinned, so fold
index ranges are remapped with `remap_folds`: a boundary moves to the number of
kept rows before it, and a row that was inside a fold's window stays inside it.

This lands trainer-side because `crates/dataplane` is outside the track that
built it (ADR-N-PY2-04); the Rust `DatasetRequest` keeps carrying the
declaration and the trainer applies it.
"""

from __future__ import annotations

import re

import numpy as np
import pandas as pd

from . import failures

MIN_SIGMA_OBS = 20


def returns_from_close(close: np.ndarray) -> np.ndarray:
    close = np.asarray(close, dtype=float)
    r = np.zeros(len(close))
    if len(close) > 1:
        r[1:] = np.diff(np.log(close))
    return r


def _causal_sigma(returns: np.ndarray) -> np.ndarray:
    """Expanding std using only returns strictly before each bar (no lookahead)."""
    n = len(returns)
    out = np.full(n, np.nan)
    csum = np.cumsum(returns)
    csq = np.cumsum(returns**2)
    for t in range(MIN_SIGMA_OBS, n):
        k = t  # observations 0..t-1
        mean = csum[t - 1] / k
        var = max(csq[t - 1] / k - mean * mean, 0.0)
        out[t] = np.sqrt(var)
    return out


def parse_h(h) -> tuple[float | None, float | None]:
    """``(absolute_h, sigma_multiple)``; exactly one is not None."""
    if isinstance(h, (int, float)) and not isinstance(h, bool):
        if h <= 0:
            raise failures.DataError(f"cusum h must be > 0, got {h}")
        return float(h), None
    if isinstance(h, str):
        m = re.fullmatch(r"\s*([0-9]*\.?[0-9]+)\s*sigma\s*", h)
        if m and float(m.group(1)) > 0:
            return None, float(m.group(1))
    raise failures.DataError(f"cusum h must be a positive number or '<k>sigma', got {h!r}")


def cusum_events(returns, h) -> np.ndarray:
    """Symmetric CUSUM filter (Lopez de Prado): positions where the cumulative
    positive or negative drift since the last event exceeds `h`.

    S+ = max(0, S+ + r_t); S- = min(0, S- + r_t); an event fires at t when S- < -h
    or S+ > h, and both sums restart at zero. With ``h="<k>sigma"`` the threshold at
    t is k times the std of returns *before* t, so an event never depends on a
    later bar; until MIN_SIGMA_OBS returns exist no event can fire.
    """
    r = np.asarray(returns, dtype=float)
    h_abs, k = parse_h(h)
    sigma = _causal_sigma(r) if k is not None else None
    s_pos = s_neg = 0.0
    events = []
    for t in range(1, len(r)):  # r[0] is the undefined first return
        thr = h_abs if k is None else (k * sigma[t] if np.isfinite(sigma[t]) else np.inf)
        s_pos = max(0.0, s_pos + r[t])
        s_neg = min(0.0, s_neg + r[t])
        if s_neg < -thr:
            s_neg = 0.0
            events.append(t)
        elif s_pos > thr:
            s_pos = 0.0
            events.append(t)
    return np.asarray(events, dtype=np.int64)


def strategy_event_positions(ts_ns: np.ndarray, entry_timestamps) -> np.ndarray:
    """Row positions for a strategy's entry timestamps.

    An entry maps to the last bar whose timestamp is <= the entry's (the bar the
    decision was made on), never a later one; an entry before the first bar is
    dropped; duplicates collapse. The timestamps must be sorted ascending.
    """
    ts = np.asarray(ts_ns, dtype=np.int64)
    if len(ts) > 1 and np.any(np.diff(ts) < 0):
        raise failures.DataError("ts_ns must be sorted ascending to place strategy events")
    entries = np.asarray(list(entry_timestamps), dtype=np.int64)
    pos = np.searchsorted(ts, entries, side="right") - 1
    pos = pos[pos >= 0]
    return np.unique(pos)


def remap_folds(folds, kept: np.ndarray):
    """Move each fold's index ranges onto the thinned frame."""
    from .schemas import FoldSpec

    if not folds:
        return folds

    def m(b: int) -> int:
        return int(np.searchsorted(kept, b, side="left"))

    out = []
    for f in folds:
        d = f.model_dump() if hasattr(f, "model_dump") else dict(f)
        for k in ("train_start", "train_end", "cal_start", "cal_end", "test_start", "test_end"):
            d[k] = m(d[k])
        out.append(FoldSpec(**d))
    return out


def sampling_spec(definition: dict):
    decl = (definition or {}).get("declaration") or {}
    s = decl.get("sampling")
    if s is None or s == "every_bar":
        return None
    if isinstance(s, dict) and s.get("kind") in (None, "every_bar"):
        return None
    if isinstance(s, dict) and s.get("kind") in ("cusum", "strategy_events"):
        return s
    raise failures.DataError(f"unsupported declaration.sampling {s!r}")


def event_positions(spec: dict, df: pd.DataFrame) -> np.ndarray:
    if spec["kind"] == "cusum":
        if "close" not in df.columns:
            raise failures.DataError("cusum sampling needs a 'close' column")
        return cusum_events(returns_from_close(df["close"].to_numpy()), spec.get("h"))
    if "ts_ns" not in df.columns:
        raise failures.DataError("strategy_events sampling needs a 'ts_ns' column")
    if spec.get("entry_timestamps") is None:
        raise failures.DataError("strategy_events sampling needs entry_timestamps")
    return strategy_event_positions(df["ts_ns"].to_numpy(), spec["entry_timestamps"])


def apply_to_request(definition: dict, df: pd.DataFrame, folds=None):
    """Subsample `df` per `declaration.sampling`. Returns ``(df, folds, info)``.

    `info` is None when sampling is every_bar (nothing changed), else
    ``{"kind", "n_rows_before", "n_events"}``. Zero events is a data error: an
    empty training set is not a model.
    """
    spec = sampling_spec(definition)
    if spec is None:
        return df, folds, None
    kept = event_positions(spec, df)
    if len(kept) == 0:
        raise failures.DataError(f"sampling {spec['kind']} produced no events")
    thinned = df.iloc[kept].reset_index(drop=True)
    info = {"kind": spec["kind"], "n_rows_before": int(len(df)), "n_events": int(len(kept))}
    return thinned, remap_folds(folds, kept), info
