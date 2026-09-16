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
* **A bid-ask bounce**, at 60 bps on the first series and 120 bps on the
  second — wide for hourly crypto, and `SPREAD_A` explains why.
* **A simulated intrabar path**, so the high and low are real extremes rather
  than a proportional wick around the bar return. `_bars_from_ticks` explains
  what that artefact did to two of the four spread estimators.

Deliberately absent
-------------------
Gaps and duplicate bars. Those belong in the fixtures of the tools that detect
them, where the defect can be placed at a known timestamp and asserted on, not
scattered through the fixture everything else shares.
"""

from __future__ import annotations

import math
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

# Relative effective spreads: 60 bps and 120 bps. Two different values, so a tool
# that cross-checks the pair cannot pass by returning one number twice.
#
# These are WIDE for hourly crypto, deliberately. At a realistic 8 bps the
# bounce contributes -(s/2)^2 = -1.6e-7 to the first return autocovariance,
# while the sampling error of that covariance on 6,000 fat-tailed GARCH bars is
# around 4e-7 — so Roll, EDGE and Abdi-Ranaldo are below their own noise floor
# and correctly decline. That is a true and useful fact about bar-based spread
# estimators at this frequency, and it makes for a fixture that exercises the
# refusal path and nothing else. The spread here is sized so the estimators have
# something recoverable to recover; the honest caveat belongs in the tool, which
# reports the noise floor beside the estimate.
SPREAD_A, SPREAD_B = 60.0e-4, 120.0e-4

# Intrabar steps used to build the high and low. Ten is enough for the range to
# be a real path extreme rather than a function of the bar return.
TICKS_PER_BAR = 10
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
    hour_effect = 0.00012 * np.sin(2.0 * np.pi * np.asarray(ts.hour) / 24.0)
    r2 = np.asarray(r2, dtype=float) + hour_effect

    frames = []
    for returns, first, spread in ((r1, 42_000.0, SPREAD_A), (r2, 2_400.0, SPREAD_B)):
        bars = _bars_from_ticks(rng, returns, first, spread)
        frames.append(pd.DataFrame({"ts": ts, **bars}))
    return frames[0], frames[1]


def _bars_from_ticks(
    rng: np.random.Generator, returns: np.ndarray, first: float, spread: float
) -> dict:
    """Build OHLC from a simulated intrabar path rather than from the bar return.

    This is more work than putting a proportional wick either side of
    `max(open, close)`, and it is the difference between a fixture that tests
    spread estimators and one that tests an artefact.

    Abdi-Ranaldo and EDGE both read the spread out of the relationship between
    the close, the open, and the **mid of the high and low** — specifically, they
    rely on that mid being an unbiased estimate of the *efficient* price. A
    synthetic wick drawn symmetrically around `max(open, close)` does not have
    that property: its mid sits somewhere between the open and the close by
    construction, not near the efficient price, and both estimators come out
    biased low by a factor of two to three against a spread that is known exactly.
    Measured, on the first version of this file.

    So: `TICKS_PER_BAR` steps of the efficient price inside each bar, the high
    and low taken from that path, and the bounce applied to every observed price
    the way a book applies it — trades print at the bid or the ask, so the
    observed high is an ask and the observed low is a bid.
    """
    returns = np.asarray(returns, dtype=float)
    n = returns.size
    steps = TICKS_PER_BAR
    # Each bar's return split into `steps` increments of equal variance, scaled
    # so the sum is exactly the bar's return: the intrabar path is a Brownian
    # bridge, so the bar-level series is untouched by this and every other
    # template sees the same closes it saw before.
    increments = rng.normal(0.0, 1.0, size=(n, steps))
    increments -= increments.mean(axis=1, keepdims=True)
    scale = np.abs(returns)[:, None] / max(steps, 1)
    path_steps = increments * scale + returns[:, None] / steps

    start = math.log(first)
    bar_open_log = start + np.concatenate([[0.0], np.cumsum(returns)[:-1]])
    within = np.cumsum(path_steps, axis=1)
    path = bar_open_log[:, None] + within

    efficient_high = np.exp(path.max(axis=1))
    efficient_low = np.exp(path.min(axis=1))
    efficient_close = np.exp(path[:, -1])
    efficient_open = np.exp(bar_open_log)

    # A trade prints at the bid or the ask. The high is therefore an ask print
    # and the low a bid print — that asymmetry is what Corwin-Schultz measures.
    half = spread / 2.0
    side_close = rng.choice([-1.0, 1.0], size=n)
    side_open = rng.choice([-1.0, 1.0], size=n)
    return {
        "open": efficient_open * (1.0 + side_open * half),
        "high": efficient_high * (1.0 + half),
        "low": efficient_low * (1.0 - half),
        "close": efficient_close * (1.0 + side_close * half),
        "volume": np.abs(returns) * 1.0e6 + 1.0,
    }


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
