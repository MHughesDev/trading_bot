"""Shared synthetic data for the family tests."""
from __future__ import annotations

import pathlib
import sys

import numpy as np
import pandas as pd

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))


def noop_emit(*_a, **_k):
    return None


def sim_garch(n, om=2e-6, a=0.08, g=0.0, b=0.88, nu=8, seed=0):
    rng = np.random.default_rng(seed)
    r = np.zeros(n)
    s2 = om / (1 - a - b - g / 2)
    for t in range(n):
        z = rng.standard_t(nu) * np.sqrt((nu - 2) / nu)
        r[t] = np.sqrt(s2) * z
        s2 = om + (a + g * (r[t] < 0)) * r[t] ** 2 + b * s2
    return r


def vol_frame(n=900, seed=3, with_rq=True):
    """A realised-variance frame: label = next-bar realised variance."""
    rng = np.random.default_rng(seed)
    lv = np.zeros(n)
    for t in range(1, n):
        lv[t] = 0.9 * lv[t - 1] + 0.3 * rng.standard_normal()
    rv = np.exp(lv - 8.0)
    d = {"realized_variance": rv, "label": np.append(rv[1:], rv[-1])}
    if with_rq:
        d["realized_quarticity"] = rv**2 * (1 + 0.1 * rng.random(n))
    return pd.DataFrame(d)


def ret_frame(n=800, seed=5, signal=0.0):
    rng = np.random.default_rng(seed)
    X = rng.standard_normal((n, 3))
    y = signal * X[:, 0] + 0.01 * rng.standard_normal(n)
    return pd.DataFrame({"f0": X[:, 0], "f1": X[:, 1], "f2": X[:, 2], "label": y})


def folds(n, k=3, test=100, train=300):
    out = []
    for i in range(k):
        te0 = train + i * test
        out.append({"index": i, "train_start": 0, "train_end": te0, "cal_start": te0, "cal_end": te0,
                    "test_start": te0, "test_end": min(te0 + test, n)})
    return out
