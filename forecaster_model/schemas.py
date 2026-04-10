"""
Canonical tensor / array contracts (human spec §5).

All arrays are NumPy `float64` unless noted. Batch dimension `B` may be 1 for inference.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class ForecasterTensors:
    """Holds named arrays matching spec shapes for one batch."""

    x_obs: np.ndarray  # [B, L, F_obs]
    x_known: np.ndarray  # [B, H, F_known]
    x_static: np.ndarray | None  # [B, F_static] or None
    r_cur: np.ndarray  # [B, F_regime]
    y: np.ndarray | None  # [B, H] targets (training)


def validate_shapes(
    x_obs: np.ndarray,
    x_known: np.ndarray,
    r_cur: np.ndarray,
    *,
    x_static: np.ndarray | None = None,
    y: np.ndarray | None = None,
) -> None:
    if x_obs.ndim != 3:
        raise ValueError("x_obs must be [B, L, F_obs]")
    if x_known.ndim != 3:
        raise ValueError("x_known must be [B, H, F_known]")
    if r_cur.ndim != 2:
        raise ValueError("r_cur must be [B, F_regime]")
    B = x_obs.shape[0]
    if x_known.shape[0] != B or r_cur.shape[0] != B:
        raise ValueError("batch dimension mismatch")
    if x_static is not None and x_static.shape[0] != B:
        raise ValueError("x_static batch mismatch")
    if y is not None:
        if y.ndim != 2 or y.shape[0] != B:
            raise ValueError("y must be [B, H]")
        if y.shape[1] != x_known.shape[1]:
            raise ValueError("y horizon must match x_known")
