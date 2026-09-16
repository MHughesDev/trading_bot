"""The fixture every template is tested against (Set S task S-3.1).

    python apps/skill-runner/reference_dataset.py <out_dir>

Writes `primary.parquet` and `secondary.parquet`: two OHLCV series with the
properties real crypto minute bars have and synthetic noise does not, so that a
template which only works on well-behaved data fails here rather than in front of
an agent.

Deliberately present in the data
--------------------------------
* **Volatility clustering**, from a GARCH(1,1) whose persistence is checked to be
  below one *after* accounting for the fat-tailed innovation's variance. Getting
  this wrong produces an explosive series whose prices overflow to zero, and a
  fixture that silently became a handful of usable bars would make every
  diagnostic downstream look fine while measuring almost nothing.
* **Fat tails**, from a standardised Student-t.
* **A correlated second series**, so spanning, cointegration and correlation
  tools have something with real common structure rather than two independent
  walks.
* **An hour-of-day effect** in the second series only, small enough that it
  should not survive a multiple-testing correction on a short window — which is
  the point: a seasonality tool that reports it as a survivor is over-reporting.

Deliberately absent
-------------------
Gaps and duplicate bars. Those belong in the fixtures of the tools that detect
them, where the defect can be placed at a known timestamp and asserted on, not
scattered through the fixture everything else shares.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import pandas as pd

SEED = 20260916
BARS = 6000
TIMEFRAME = "1h"

# GARCH(1,1). With a standardised t(nu) innovation the variance persistence is
# alpha + beta, because the innovation already has unit variance — the trap is
# using a raw t, whose variance is nu/(nu-2), which turns this into 1.03 and
# makes the process explosive.
OMEGA, ALPHA, BETA, NU = 2.0e-7, 0.09, 0.89, 5.0
assert ALPHA + BETA < 1.0, "the fixture's volatility process must be stationary"


def _standardised_t(rng: np.random.Generator, n: int, nu: float) -> np.ndarray:
    return rng.standard_t(nu, size=n) / np.sqrt(nu / (nu - 2.0))


def build() -> tuple[pd.DataFrame, pd.DataFrame]:
    rng = np.random.default_rng(SEED)
    z1 = _standardised_t(rng, BARS, NU)
    z2 = _standardised_t(rng, BARS, NU)
    # A common factor, so the two series share structure instead of only a
    # correlation coefficient.
    rho = 0.6
    z2 = rho * z1 + np.sqrt(1.0 - rho**2) * z2

    def garch(z: np.ndarray) -> np.ndarray:
        h = np.empty(BARS)
        e = np.zeros(BARS)
        h[0] = OMEGA / (1.0 - ALPHA - BETA)
        e[0] = np.sqrt(h[0]) * z[0]
        for t in range(1, BARS):
            h[t] = OMEGA + ALPHA * e[t - 1] ** 2 + BETA * h[t - 1]
            e[t] = np.sqrt(h[t]) * z[t]
        return e

    r1 = garch(z1)
    r2 = garch(z2)

    ts = pd.date_range("2024-01-01T00:00:00Z", periods=BARS, freq="h")
    # Small enough to be inside the noise on a short window.
    hour_effect = 0.00012 * np.sin(2.0 * np.pi * ts.hour / 24.0)
    r2 = r2 + hour_effect

    frames = []
    for returns, first in ((r1, 42_000.0), (r2, 2_400.0)):
        close = first * np.exp(np.cumsum(returns))
        assert np.isfinite(close).all() and (close > 0).all(), "the fixture overflowed"
        open_ = np.concatenate([[first], close[:-1]])
        wick = np.abs(returns) * 0.5
        frames.append(
            pd.DataFrame(
                {
                    "ts": ts,
                    "open": open_,
                    "high": np.maximum(open_, close) * (1.0 + wick),
                    "low": np.minimum(open_, close) * (1.0 - wick),
                    "close": close,
                    "volume": np.abs(returns) * 1.0e6 + 1.0,
                }
            )
        )
    return frames[0], frames[1]


def main(out_dir: str) -> int:
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    primary, secondary = build()
    primary.to_parquet(out / "primary.parquet", index=False)
    secondary.to_parquet(out / "secondary.parquet", index=False)
    print(f"{len(primary)} bars -> {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1] if len(sys.argv) > 1 else "."))
