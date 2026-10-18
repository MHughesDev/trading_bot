"""Real sequence windows for sequence models (Set N, N-2.5; FEAT-006 section 3).

`LSTMNet.forward` used to do ``x.unsqueeze(1)`` on 2-D rows, so every "sequence"
model saw a sequence of length one: an LSTM that was an MLP with extra steps.
When the declaration carries ``sequence: {window: W, stride: S}`` the rows of
each split become windows instead, and the inputs are 3-D
``(batch, window, features)``.

Rules, each enforced by a test and not by a comment:

  * A window ends at row ``i`` and holds rows ``i-W+1 .. i``. It never holds a
    row after ``i``, and the label at ``i`` is the forward label of the bar the
    window ends on, so nothing past the label time is an input.
  * Windows are cut *inside* a split. A train window never reaches into the
    validation or test rows and a test window never reaches back into train, so
    no window crosses a fold's train/test boundary. The price is the first
    ``W-1`` rows of each later split; that is a cost, not a bug.
  * `stride` thins the *training* windows only (every S-th end row). Validation
    and test are scored on every window so a score is never a function of S.
  * The bundle records ``sequence = {"window": W, "stride": S}`` at the top level
    of its header, and the serving side builds the identical window from the
    most recent W bars. With fewer than W bars it abstains with a reason;
    it never pads, repeats or zero-fills.

A definition without ``declaration.sequence`` is untouched (legacy models).
"""

from __future__ import annotations

import dataclasses
import json
import struct

import numpy as np

from . import engine, failures

ABSTAIN_INSUFFICIENT_HISTORY = "insufficient_history"
ABSTAIN_FEATURE_MISSING = "feature_missing"


class InsufficientHistory(failures.DataError):
    """Fewer than `window` bars were supplied: serving abstains, it does not pad."""

    reason = ABSTAIN_INSUFFICIENT_HISTORY

    def __init__(self, have: int, need: int):
        super().__init__(f"{ABSTAIN_INSUFFICIENT_HISTORY}: have {have} bars, need {need}")
        self.have = int(have)
        self.need = int(need)


# --------------------------------------------------------------------------- #
# The declaration
# --------------------------------------------------------------------------- #
def sequence_spec(definition: dict) -> dict | None:
    """``{"window": W, "stride": S}`` from ``declaration.sequence``, or None.

    Absent / null means a legacy (row) model. A present but malformed block is a
    data error: a typo must not silently turn a sequence model into a row model.
    """
    decl = (definition or {}).get("declaration") or {}
    seq = decl.get("sequence")
    if seq is None:
        return None
    if not isinstance(seq, dict):
        raise failures.DataError(f"declaration.sequence must be an object, got {type(seq).__name__}")
    try:
        window = int(seq["window"])
        stride = int(seq.get("stride", 1))
    except (KeyError, TypeError, ValueError) as e:
        raise failures.DataError(f"declaration.sequence needs integer window/stride: {e}") from e
    if window < 1 or stride < 1:
        raise failures.DataError(f"declaration.sequence window and stride must be >= 1, got {seq}")
    return {"window": window, "stride": stride}


# --------------------------------------------------------------------------- #
# Windowing
# --------------------------------------------------------------------------- #
def window_ends(n: int, window: int, stride: int = 1) -> np.ndarray:
    """Row indices (within a segment of n rows) that end a full window."""
    if n < window:
        return np.zeros(0, dtype=np.int64)
    return np.arange(window - 1, n, stride, dtype=np.int64)


def make_windows(
    X: np.ndarray,
    window: int,
    stride: int = 1,
    y: np.ndarray | None = None,
    w: np.ndarray | None = None,
):
    """Cut one segment's rows into windows.

    Returns ``(Xw, yw, ww)``: ``Xw`` is ``(m, window, features)`` float32, and
    ``yw`` / ``ww`` are the label / weight of each window's *last* row (None when
    not supplied). A segment shorter than `window` yields ``m == 0``.
    """
    X = np.asarray(X, dtype=np.float32)
    if X.ndim != 2:
        raise ValueError(f"make_windows wants 2-D rows, got shape {X.shape}")
    ends = window_ends(len(X), window, stride)
    if len(ends) == 0:
        Xw = np.zeros((0, window, X.shape[1]), dtype=np.float32)
    else:
        idx = ends[:, None] + np.arange(-window + 1, 1)[None, :]
        Xw = X[idx]
    yw = None if y is None else np.asarray(y)[ends]
    ww = None if w is None else np.asarray(w)[ends]
    return Xw, yw, ww


def apply_sequence(definition: dict, p: engine.Prepared) -> tuple[engine.Prepared, dict | None]:
    """Turn a prepared (2-D) split into windows when the declaration asks.

    Returns ``(prepared, spec)``; `spec` is None and `prepared` is returned
    unchanged for a legacy definition. `engine.prepare` already sliced and scaled
    each split, so cutting windows inside each of `X_tr`, `X_val`, `X_te` keeps
    every window within one split by construction.
    """
    spec = sequence_spec(definition)
    if spec is None:
        return p, None
    W, S = spec["window"], spec["stride"]
    Xtr, ytr, wtr = make_windows(p.X_tr, W, S, p.y_tr, p.w_tr)
    Xva, yva, _ = make_windows(p.X_val, W, 1, p.y_val)
    Xte, yte, _ = make_windows(p.X_te, W, 1, p.y_te)
    if len(Xtr) == 0:
        raise failures.DataError(
            f"the training split has {len(p.X_tr)} rows, fewer than the declared window {W}"
        )
    return (
        dataclasses.replace(
            p, X_tr=Xtr, y_tr=ytr, X_val=Xva, y_val=yva, X_te=Xte, y_te=yte, w_tr=wtr
        ),
        spec,
    )


# --------------------------------------------------------------------------- #
# Serving
# --------------------------------------------------------------------------- #
def serving_window(rows: np.ndarray, window: int) -> np.ndarray:
    """The one window serving scores: the most recent `window` rows, shape (1, W, F).

    Raises `InsufficientHistory` rather than padding. Rows must be oldest first.
    """
    rows = np.asarray(rows, dtype=np.float32)
    if rows.ndim != 2:
        raise ValueError(f"serving_window wants 2-D rows, got shape {rows.shape}")
    if len(rows) < window:
        raise InsufficientHistory(len(rows), window)
    return rows[-window:][None, :, :]


def read_bundle(data: bytes) -> tuple[dict, bytes]:
    """``(header, inner)`` of a `tb-bundle-1` envelope (mirror of `engine.wrap_bundle`)."""
    if bytes(data[:8]) != engine.MAGIC:
        raise failures.DataError("not a tb-bundle-1 artifact (bad magic)")
    (hlen,) = struct.unpack("<I", bytes(data[8:12]))
    header = json.loads(bytes(data[12 : 12 + hlen]).decode("utf-8"))
    return header, bytes(data[12 + hlen :])


def prepare_serving_rows(header: dict, feature_rows) -> np.ndarray:
    """Order columns by the bundle's feature_order and apply its scaler.

    `feature_rows` is a DataFrame (columns by name) or a list of dicts, oldest
    first. A missing feature is an error: never zero-filled (U-5.2).
    """
    import pandas as pd

    df = feature_rows if isinstance(feature_rows, pd.DataFrame) else pd.DataFrame(list(feature_rows))
    order = list(header.get("feature_order") or [])
    missing = [c for c in order if c not in df.columns]
    if missing:
        raise failures.DataError(f"{ABSTAIN_FEATURE_MISSING}: " + ", ".join(missing))
    X = df[order].to_numpy(dtype=np.float64)
    sc = header.get("scaler")
    if sc:
        X = engine.apply_scaler(X, np.asarray(sc["mean"], dtype=float), np.asarray(sc["std"], dtype=float))
    return X.astype(np.float32)


def predict_bundle(bundle: bytes, feature_rows) -> dict:
    """Score the latest bar of a windowed torch bundle from recent bars.

    Returns ``{"abstain": False, "pred": float, "window": W}`` or, with fewer
    than W bars, ``{"abstain": True, "reason": "insufficient_history", ...}``.
    Builds the same window `apply_sequence` trained on.
    """
    import io

    import torch

    header, inner = read_bundle(bundle)
    seq = header.get("sequence")
    if not seq:
        raise failures.DataError("bundle has no sequence block: not a windowed model")
    W = int(seq["window"])
    X = prepare_serving_rows(header, feature_rows)
    try:
        xw = serving_window(X, W)
    except InsufficientHistory as e:
        return {"abstain": True, "reason": e.reason, "have": e.have, "need": e.need}
    payload = torch.load(io.BytesIO(inner), map_location="cpu", weights_only=False)
    from .families import sequence as seqfam  # local: sequence imports this module

    model = seqfam.build_model(payload["arch"], int(payload["input_dim"]), int(payload["hidden_dim"]), W)
    model.load_state_dict(payload["state_dict"])
    model.eval()
    with torch.no_grad():
        raw = float(model(torch.tensor(xw)).reshape(-1)[0])
    if header.get("objective") == "classification":
        raw = 1.0 / (1.0 + float(np.exp(-raw)))
    return {"abstain": False, "pred": raw, "window": W}
