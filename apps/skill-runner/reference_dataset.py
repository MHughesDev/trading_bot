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
* **A timeframe pair** — `fast.parquet` (1m), `slow.parquet` (its exact 1h
  aggregate) and `slow_leaky.parquet` (the same hours written before their last
  minute landed). The leak is planted the way the platform's real one occurs, so
  a check that passes on it has actually caught the bug rather than noise.
* **A family of strategy returns** — `family.parquet`, long format, twelve
  series sharing one common factor with uneven loadings, of which two are
  deliberately short. See `build_family`.
* **A family an optimiser wants to concentrate** — `family_concentrated.parquet`,
  where one strategy has a hundredth of the variance of the rest. Without it
  `allocate_book`'s weight cap is a branch no test ever takes.

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

# The family fixture, for templates that take `kind: trial_returns`.
#
# Twelve strategies over 500 periods is q = N/T = 0.024, which is small enough
# that the real factor clears the Marchenko-Pastur noise edge and the rest do
# not — so a tool that reports every eigenvalue as a factor fails here, and so
# does one that reports none.
FAMILY_STRATEGIES = 12
FAMILY_PERIODS = 500
# One common factor, loaded on unevenly. Without it the covariance is diagonal
# noise and shrinkage has nothing to find; with every strategy loading equally
# it is rank one and shrinkage has nothing to do.
FAMILY_FACTOR_LOADINGS = 0.7
# The timeframe-pair fixture, for `audit.bar_boundary_check`. Thirty days of
# minutes is enough for 720 hourly bars — plenty to find a planted defect and
# small enough that the parquet stays a few megabytes.
PAIR_DAYS = 30
PAIR_MINUTES = PAIR_DAYS * 24 * 60
# How many hourly bars in the leaky variant were written before their last
# minute landed, and how many minutes early. Both are checked by the test, so a
# change here that made the defect undetectable fails rather than passes.
LEAKY_FRACTION = 0.25
LEAKY_MINUTES_EARLY = 20

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
    # The bar's opening point is part of the bar. Prepending a zero increment
    # puts it in the path, and without it `efficient_high` and `efficient_low`
    # are the extremes of everything *after* the open — so they need not bracket
    # the open at all, and on a wide spread the observed open printed at the ask
    # comes out above the observed high.
    #
    # That produced 2,526 bars of 6,000 whose open lay outside their own range,
    # which is not a bar. It was invisible until the Garman-Klass reconciliation
    # (S-5.14): `0.5·ln(h/l)² − (2ln2−1)·ln(c/o)²` is non-negative for any
    # well-formed bar, because `ln(h/l) ≥ |ln(c/o)|` when the range contains both
    # ends — so the engine's per-bar floor at zero can never bind, and it was
    # binding on fifteen bars.
    path = bar_open_log[:, None] + np.concatenate(
        [np.zeros((n, 1)), within], axis=1
    )

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


def build_timeframe_pair() -> tuple[pd.DataFrame, pd.DataFrame, pd.DataFrame]:
    """A 1m series, its exact 1h aggregate, and a deliberately leaking one.

    The leaking variant is what the platform's continuous aggregation produces
    when a row is read at its open timestamp: the hourly bar exists from the
    moment its first minute lands, and its `close` is whatever had arrived when
    the row was last written. So the defect is planted the way the real bug
    occurs — the close of minute 40 stored as the close of the hour — rather than
    as random noise, which any check would catch and which would prove nothing.
    """
    rng = np.random.default_rng(SEED + 1)
    minutes = pd.date_range("2024-01-01T00:00:00Z", periods=PAIR_MINUTES, freq="min")
    r = _standardised_t(rng, PAIR_MINUTES, NU) * 0.0004
    bars = _bars_from_ticks(rng, r, 42_000.0, SPREAD_A)
    fast = pd.DataFrame({"ts": minutes, **bars})

    hourly = fast.set_index("ts").resample("1h")
    slow = pd.DataFrame(
        {
            "open": hourly["open"].first(),
            "high": hourly["high"].max(),
            "low": hourly["low"].min(),
            "close": hourly["close"].last(),
            "volume": hourly["volume"].sum(),
        }
    ).dropna()

    # The leak: for a quarter of the hours, the stored close is the close of a
    # minute that is not the last one, and the high and low only cover the
    # minutes that had landed by then.
    leaky = slow.copy()
    partial = fast.set_index("ts").iloc[:, :].copy()
    chosen = rng.random(len(slow)) < LEAKY_FRACTION
    cut = 60 - LEAKY_MINUTES_EARLY
    for i, (stamp, _) in enumerate(slow.iterrows()):
        if not chosen[i]:
            continue
        window = partial.loc[stamp : stamp + pd.Timedelta(minutes=59)].iloc[:cut]
        if len(window) < cut:
            continue
        leaky.loc[stamp, "close"] = float(window["close"].iloc[-1])
        leaky.loc[stamp, "high"] = float(window["high"].max())
        leaky.loc[stamp, "low"] = float(window["low"].min())

    return (
        fast,
        slow.reset_index().rename(columns={"index": "ts"}),
        leaky.reset_index().rename(columns={"index": "ts"}),
    )

def build_family(rng: np.random.Generator) -> pd.DataFrame:
    """A family of strategy return series, long format: `(run_id, period, ret)`.

    Long rather than wide for the same reason the materialiser is: real families
    are ragged, and the last two strategies here are deliberately short so a tool
    that pads or pairwise-deletes produces a different answer from one that uses
    the common periods and says how many it dropped.
    """
    factor = rng.standard_normal(FAMILY_PERIODS) * 0.01
    stamps = pd.date_range("2024-01-01T00:00:00Z", periods=FAMILY_PERIODS, freq="h")
    rows = []
    for i in range(FAMILY_STRATEGIES):
        loading = FAMILY_FACTOR_LOADINGS * (0.4 + 0.6 * (i / max(FAMILY_STRATEGIES - 1, 1)))
        idiosyncratic = rng.standard_normal(FAMILY_PERIODS) * 0.01
        series = loading * factor + math.sqrt(max(1.0 - loading**2, 0.0)) * idiosyncratic
        # The last two are short. A ragged family is the normal case.
        length = FAMILY_PERIODS if i < FAMILY_STRATEGIES - 2 else FAMILY_PERIODS // 2
        for period in range(length):
            rows.append((f"run_{i:02d}", period, stamps[period], float(series[period])))
    return pd.DataFrame(rows, columns=["run_id", "period", "ts", "ret"])


def build_concentrated_family(rng: np.random.Generator) -> pd.DataFrame:
    """A family built so an optimiser wants to put everything in one strategy.

    `run_00` has a hundredth of the variance of the rest, so minimum variance
    wants roughly all of the book in it. That is the case `allocate_book`'s cap
    exists for, and without a fixture that provokes it the cap is a branch no
    test ever takes — the well-behaved family reaches a maximum weight of 0.15
    against a cap of 0.25 and never exercises the clip.

    Nothing here is unrealistic. A strategy that barely trades has exactly this
    profile, and it is the one a variance-minimising optimiser loves.
    """
    stamps = pd.date_range("2024-01-01T00:00:00Z", periods=FAMILY_PERIODS, freq="h")
    rows = []
    for i in range(FAMILY_STRATEGIES):
        scale = 0.0001 if i == 0 else 0.01
        series = rng.standard_normal(FAMILY_PERIODS) * scale
        for period in range(FAMILY_PERIODS):
            rows.append((f"run_{i:02d}", period, stamps[period], float(series[period])))
    return pd.DataFrame(rows, columns=["run_id", "period", "ts", "ret"])


def build_spanning(primary: pd.DataFrame, rng: np.random.Generator) -> pd.DataFrame:
    """Two strategies against the primary series' own returns, for
    `attribution.factor_spanning_test`.

    `run_span` is the market with noise: 0.9 of the factor's return and no alpha,
    so the test must find a beta near 0.9 and an alpha it cannot tell from zero.
    `run_alpha` has no loading and a constant per-period edge several times its
    noise, so the test must find a significant alpha. A tool that reported every
    strategy as spanned, or none, fails one of the two.

    Aligned by construction to the factor's own bar timestamps (period i is the
    return into bar i+1), the key the materialiser uses for both series.
    """
    close = primary["close"].to_numpy(dtype=float)
    factor = np.diff(np.log(close))
    stamps = pd.DatetimeIndex(primary["ts"]).to_numpy()[1:]
    n = min(len(factor), 1500)
    factor, stamps = factor[:n], stamps[:n]
    noise_sd = float(np.std(factor)) * 0.3
    rows = []
    span = 0.9 * factor + rng.standard_normal(n) * noise_sd
    alpha = 0.0003 + rng.standard_normal(n) * float(np.std(factor)) * 0.5
    for name, series in (("run_span", span), ("run_alpha", alpha)):
        for period in range(n):
            rows.append((name, period, pd.Timestamp(stamps[period]), float(series[period])))
    return pd.DataFrame(rows, columns=["run_id", "period", "ts", "ret"])


# The predictions fixture, for `kind: predictions` (signal_half_life, conformal_calibrate).
PRED_ROWS = 2000
PRED_IC0 = 0.30
PRED_HALF_LIFE = 12.0
PRED_HORIZONS = (1, 2, 4, 8, 16, 24, 48, 96)


def build_predictions(rng: np.random.Generator) -> pd.DataFrame:
    """A model's answers beside realised forward returns, with a KNOWN decay.

    `y_hat` is a unit-variance signal. For each horizon `h` the label is built so
    its correlation with `y_hat` is exactly `PRED_IC0 * 2 ** (-h / PRED_HALF_LIFE)`
    (`y = rho * z + sqrt(1 - rho**2) * noise`), so `signal_half_life` has a true
    half-life of 12 bars to recover and a tool that reported the horizon grid's
    own spacing would be wrong. `y_h6` is a separate column for the conformal
    test: same construction, but its noise doubles in the second half, so a
    quantile fitted on the calm half under-covers the storm and an adaptive
    interval has something to adapt to.
    """
    z = rng.standard_normal(PRED_ROWS)
    stamps = pd.date_range("2024-01-01T00:00:00Z", periods=PRED_ROWS, freq="h")
    cols = {"ts": stamps, "y_hat": z}
    for h in PRED_HORIZONS:
        rho = PRED_IC0 * 2.0 ** (-h / PRED_HALF_LIFE)
        cols[f"y_h{h}"] = rho * z + math.sqrt(1.0 - rho**2) * rng.standard_normal(PRED_ROWS)
    rho6 = PRED_IC0 * 2.0 ** (-6 / PRED_HALF_LIFE)
    noise = rng.standard_normal(PRED_ROWS) * np.where(np.arange(PRED_ROWS) < PRED_ROWS // 2, 1.0, 2.0)
    cols["y_h6"] = rho6 * z + math.sqrt(1.0 - rho6**2) * noise
    return pd.DataFrame(cols)


def main(out_dir: str) -> int:
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    primary, secondary = build()
    primary.to_parquet(out / "primary.parquet", index=False)
    secondary.to_parquet(out / "secondary.parquet", index=False)

    fast, slow, leaky = build_timeframe_pair()
    fast.to_parquet(out / "fast.parquet", index=False)
    slow.to_parquet(out / "slow.parquet", index=False)
    leaky.to_parquet(out / "slow_leaky.parquet", index=False)

    build_spanning(primary, np.random.default_rng(SEED + 4)).to_parquet(
        out / "spanning.parquet", index=False
    )

    build_predictions(np.random.default_rng(SEED + 5)).to_parquet(
        out / "predictions.parquet", index=False
    )

    family = build_family(np.random.default_rng(SEED + 2))
    family.to_parquet(out / "family.parquet", index=False)
    concentrated = build_concentrated_family(np.random.default_rng(SEED + 3))
    concentrated.to_parquet(out / "family_concentrated.parquet", index=False)

    print(
        f"{len(primary)} bars, {len(fast)} minutes, {len(slow)} hours, "
        f"{family['run_id'].nunique()} strategies -> {out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1] if len(sys.argv) > 1 else "."))
