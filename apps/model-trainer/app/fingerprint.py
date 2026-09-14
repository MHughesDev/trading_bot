"""Tier-1 asset fingerprints (SPEC 5.2, checklist 3.1, ADR-P3-01).

A fingerprint is a statistical description of how an instrument *behaves* —
volatility at several horizons, how much of its variance arrives in jumps, how
far its autocorrelation reaches, how expensive it is to trade. It is what makes
"find me assets like this one" answerable, and it is computed from bars alone.

Three things about it that are decisions rather than details.

**It is not a feature.** INV-14 says a strategy is served one windowed
implementation per feature, evaluated through one function, on the bar clock.
A fingerprint is computed over a 21-day trailing window by a different job on a
different schedule, and letting one into a feature vector by name would break
that quietly. AT-68 scans the feature runtime for exactly this. If a dimension
is ever genuinely wanted as a feature, it is registered as a windowed feature
like any other.

**Seasonality is deflated first.** Crypto volatility has a strong
hour-of-week shape; measuring realized volatility without removing it produces a
fingerprint that mostly encodes what time the window happened to start
(checklist 3.2).

**Absent is not zero.** A 24/7 venue has no overnight gap, and an instrument
with no signed volume has no Kyle lambda. Both come back `None` with a flag
rather than `0.0`. A zero here is a measurement, and a measurement nobody made
is the kind of thing that ends up in a kNN distance.

EDGE is Ardia, Guidotti & Kroencke's efficient estimator of the effective
bid-ask spread from OHLC alone. It matters here because this platform has no
tick data and no venue fee schedule for most instruments, and a cost estimate
from free data is the difference between a capacity gate that runs and one that
does not.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any

import numpy as np

# The horizons realized volatility is measured at, in bars. Five, spanning two
# orders of magnitude, because an instrument that is calm minute to minute and
# violent week to week is a different instrument from one that is the reverse —
# and a single horizon cannot tell them apart.
RV_HORIZONS = (1, 5, 21, 63, 252)

# Lags the autocorrelation of returns is measured at.
ACF_LAGS = (1, 5, 21)

# Variance-ratio horizons. A ratio near 1 is a random walk; above 1 is trending,
# below is mean-reverting.
VR_HORIZONS = (1, 5, 21)

# Hours in a week, for the crypto seasonal profile.
HOURS_PER_WEEK = 168


@dataclass
class Bars:
    """OHLCV on one clock. Every array is the same length."""

    ts_ns: np.ndarray
    open: np.ndarray
    high: np.ndarray
    low: np.ndarray
    close: np.ndarray
    volume: np.ndarray
    # Signed volume when the venue publishes trade sides; `None` otherwise, and
    # `None` is a different claim from zero.
    signed_volume: np.ndarray | None = None
    # A venue that never closes has no overnight gap to measure.
    continuous_venue: bool = True

    def __len__(self) -> int:
        return len(self.close)


@dataclass
class Fingerprint:
    """The tier-1 vector, plus what could not be measured and why."""

    values: dict[str, float] = field(default_factory=dict)
    # Names that could not be measured, each with its reason. Never silently
    # filled with zero: a zero is a measurement.
    absent: dict[str, str] = field(default_factory=dict)

    def vector(self, order: tuple[str, ...]) -> list[float | None]:
        """The fingerprint in a declared dimension order, `None` where absent."""
        return [self.values.get(name) for name in order]

    def dimension_names(self) -> tuple[str, ...]:
        return tuple(sorted(self.values) + sorted(self.absent))


def log_returns(close: np.ndarray) -> np.ndarray:
    """Log returns, with non-positive prices breaking the chain rather than
    producing a NaN that propagates through every statistic downstream."""
    c = np.asarray(close, dtype=np.float64)
    if len(c) < 2:
        return np.empty(0, dtype=np.float64)
    prev, nxt = c[:-1], c[1:]
    ok = (prev > 0) & (nxt > 0)
    return np.log(nxt[ok] / prev[ok])


def seasonal_profile(ts_ns: np.ndarray, returns: np.ndarray) -> np.ndarray:
    """Mean absolute return per hour-of-week, normalised to mean 1.

    Crypto's hour-of-week volatility shape is strong enough that a realized
    volatility measured without removing it encodes mostly what time the window
    started (checklist 3.2).
    """
    if len(returns) == 0:
        return np.ones(HOURS_PER_WEEK)
    hours = ((np.asarray(ts_ns[1:], dtype=np.int64) // 3_600_000_000_000) % HOURS_PER_WEEK)
    hours = hours[: len(returns)]
    profile = np.ones(HOURS_PER_WEEK, dtype=np.float64)
    magnitude = np.abs(returns)
    for h in range(HOURS_PER_WEEK):
        mask = hours == h
        if mask.sum() >= 2:
            profile[h] = float(magnitude[mask].mean())
    mean = profile.mean()
    return profile / mean if mean > 0 else np.ones(HOURS_PER_WEEK)


def deflate_seasonality(ts_ns: np.ndarray, returns: np.ndarray) -> np.ndarray:
    """Divide each return by its hour-of-week factor (checklist 3.2)."""
    if len(returns) == 0:
        return returns
    profile = seasonal_profile(ts_ns, returns)
    hours = ((np.asarray(ts_ns[1:], dtype=np.int64) // 3_600_000_000_000) % HOURS_PER_WEEK)
    hours = hours[: len(returns)]
    factors = np.clip(profile[hours], 1e-6, None)
    return returns / factors


def edge_spread(bars: Bars) -> float | None:
    """The effective bid-ask spread from OHLC (Ardia–Guidotti–Kroencke).

    The estimator rests on one observation: under a bid-ask bounce, the
    covariance between consecutive *midpoint-to-close* deviations is negative,
    and its magnitude is the squared half-spread. Using the high and the low as
    a midpoint proxy is what makes it work without tick data.

    `None` when the window is too short or degenerate. A spread of zero from
    four bars is a number, not an estimate.
    """
    if len(bars) < 21:
        return None
    o = np.asarray(bars.open, dtype=np.float64)
    h = np.asarray(bars.high, dtype=np.float64)
    low = np.asarray(bars.low, dtype=np.float64)
    c = np.asarray(bars.close, dtype=np.float64)
    if np.any(o <= 0) or np.any(h <= 0) or np.any(low <= 0) or np.any(c <= 0):
        return None

    log_o, log_h, log_l, log_c = np.log(o), np.log(h), np.log(low), np.log(c)
    mid = (log_h + log_l) / 2.0

    # Deviations of close and open from the bar's own midpoint.
    x1 = log_c[:-1] - mid[:-1]
    y1 = log_o[1:] - mid[:-1]
    if len(x1) < 2:
        return None

    cov = float(np.cov(x1, y1, ddof=1)[0, 1]) if len(x1) > 1 else 0.0
    # A positive covariance means no detectable bounce: the honest answer is a
    # spread of zero, not the square root of a negative number.
    squared = max(-4.0 * cov, 0.0)
    spread = math.sqrt(squared)
    return spread if math.isfinite(spread) else None


def amihud_illiquidity(returns: np.ndarray, volume: np.ndarray) -> float | None:
    """Mean |return| per unit of dollar volume. Higher is less liquid."""
    v = np.asarray(volume[1:], dtype=np.float64)[: len(returns)]
    ok = v > 0
    if ok.sum() < 5:
        return None
    return float(np.mean(np.abs(returns[ok]) / v[ok]))


def kyle_lambda(returns: np.ndarray, signed_volume: np.ndarray | None) -> float | None:
    """Price impact per unit of signed order flow.

    `None` without signed volume — and that is most venues on free data. An
    unsigned-volume regression would estimate something else entirely and wear
    the same name.
    """
    if signed_volume is None:
        return None
    sv = np.asarray(signed_volume[1:], dtype=np.float64)[: len(returns)]
    if len(sv) < 20 or np.std(sv) <= 0:
        return None
    slope, _ = np.polyfit(sv, returns[: len(sv)], 1)
    return float(slope) if math.isfinite(slope) else None


def bipower_jump_share(returns: np.ndarray) -> float | None:
    """The fraction of quadratic variation that arrives in jumps.

    Realized variance counts everything; bipower variation is robust to jumps.
    Their difference is the jump component, and the share of it is what
    separates an instrument that drifts from one that gaps.
    """
    if len(returns) < 20:
        return None
    rv = float(np.sum(returns**2))
    if rv <= 0:
        return None
    mu1 = math.sqrt(2.0 / math.pi)
    bv = float(np.sum(np.abs(returns[:-1]) * np.abs(returns[1:])) / (mu1**2))
    return float(np.clip((rv - bv) / rv, 0.0, 1.0))


def variance_ratio(returns: np.ndarray, q: int) -> float | None:
    """`Var(q-period return) / (q · Var(1-period return))`.

    1 is a random walk, above is trending, below is mean-reverting. The
    deviation from 1 is the whole signal.
    """
    if q < 1 or len(returns) < q * 10:
        return None
    var1 = float(np.var(returns, ddof=1))
    if var1 <= 0:
        return None
    usable = (len(returns) // q) * q
    aggregated = returns[:usable].reshape(-1, q).sum(axis=1)
    if len(aggregated) < 2:
        return None
    return float(np.var(aggregated, ddof=1) / (q * var1))


def hurst_exponent(returns: np.ndarray) -> float | None:
    """Rescaled-range Hurst exponent. 0.5 is a random walk."""
    n = len(returns)
    if n < 64:
        return None
    scales = [s for s in (8, 16, 32, 64, 128, 256) if s <= n // 2]
    if len(scales) < 3:
        return None
    rs = []
    for s in scales:
        chunks = returns[: (n // s) * s].reshape(-1, s)
        deviations = np.cumsum(chunks - chunks.mean(axis=1, keepdims=True), axis=1)
        ranges = deviations.max(axis=1) - deviations.min(axis=1)
        sds = chunks.std(axis=1, ddof=1)
        ok = sds > 0
        if ok.sum() == 0:
            return None
        rs.append(float(np.mean(ranges[ok] / sds[ok])))
    slope, _ = np.polyfit(np.log(scales), np.log(rs), 1)
    return float(slope) if math.isfinite(slope) else None


def overnight_share(bars: Bars) -> tuple[float | None, str | None]:
    """The fraction of absolute return that arrives between bars.

    Returns `(value, absence_reason)`. A continuous venue gets
    `(None, "continuous_venue")`: it has no overnight, and reporting 0 would say
    it has one and it is quiet.
    """
    if bars.continuous_venue:
        return None, "continuous_venue"
    if len(bars) < 3:
        return None, "too_few_bars"
    o = np.asarray(bars.open, dtype=np.float64)
    c = np.asarray(bars.close, dtype=np.float64)
    ok = (o[1:] > 0) & (c[:-1] > 0)
    if ok.sum() < 2:
        return None, "non_positive_prices"
    overnight = np.abs(np.log(o[1:][ok] / c[:-1][ok]))
    intraday = np.abs(np.log(np.clip(c[1:][ok], 1e-12, None) / o[1:][ok]))
    total = overnight.sum() + intraday.sum()
    if total <= 0:
        return None, "no_movement"
    return float(overnight.sum() / total), None


def compute(bars: Bars, *, deflate: bool = True) -> Fingerprint:
    """The tier-1 fingerprint.

    `deflate` removes the hour-of-week seasonal shape before any volatility
    statistic (checklist 3.2). Off only for instruments with no such shape, and
    the default is on because the failure it prevents is silent.
    """
    fp = Fingerprint()
    raw = log_returns(bars.close)
    if len(raw) < 2:
        fp.absent["all"] = "fewer than two usable bars"
        return fp

    returns = deflate_seasonality(bars.ts_ns, raw) if deflate else raw

    # Realized volatility at each horizon, annualisation deliberately omitted:
    # these are compared to each other, and a per-horizon scaling factor would
    # be an assumption about the bar period that this function does not know.
    for h in RV_HORIZONS:
        if len(returns) >= h * 2:
            windowed = returns[: (len(returns) // h) * h].reshape(-1, h).sum(axis=1)
            if len(windowed) >= 2:
                fp.values[f"rv_{h}"] = float(np.std(windowed, ddof=1))
            else:
                fp.absent[f"rv_{h}"] = "too few windows"
        else:
            fp.absent[f"rv_{h}"] = "window longer than the series"

    # Vol-of-vol: how unstable the volatility itself is.
    if len(returns) >= 42:
        rolling = np.array(
            [float(np.std(returns[i : i + 21], ddof=1)) for i in range(len(returns) - 21)]
        )
        fp.values["vol_of_vol"] = float(np.std(rolling, ddof=1)) if len(rolling) > 1 else 0.0
    else:
        fp.absent["vol_of_vol"] = "fewer than 42 returns"

    sd = float(np.std(returns, ddof=1))
    if sd > 0:
        centred = returns - returns.mean()
        fp.values["skew"] = float(np.mean(centred**3) / sd**3)
        fp.values["kurtosis"] = float(np.mean(centred**4) / sd**4)
        fp.values["downside_vol"] = float(np.std(returns[returns < 0], ddof=1)) if (returns < 0).sum() > 1 else 0.0
        fp.values["upside_vol"] = float(np.std(returns[returns > 0], ddof=1)) if (returns > 0).sum() > 1 else 0.0
    else:
        for name in ("skew", "kurtosis", "downside_vol", "upside_vol"):
            fp.absent[name] = "the series does not move"

    for lag in ACF_LAGS:
        if len(returns) > lag * 4:
            a, b = returns[:-lag], returns[lag:]
            if np.std(a) > 0 and np.std(b) > 0:
                fp.values[f"acf_{lag}"] = float(np.corrcoef(a, b)[0, 1])
            else:
                fp.absent[f"acf_{lag}"] = "no variation at this lag"
        else:
            fp.absent[f"acf_{lag}"] = "series shorter than four lags"

    for q in VR_HORIZONS:
        vr = variance_ratio(returns, q)
        if vr is None:
            fp.absent[f"vr_{q}"] = "series too short for this horizon"
        else:
            fp.values[f"vr_{q}"] = vr

    for name, value, reason in (
        ("hurst", hurst_exponent(returns), "fewer than 64 returns"),
        ("jump_share", bipower_jump_share(returns), "fewer than 20 returns"),
        ("amihud", amihud_illiquidity(returns, bars.volume), "no positive volume"),
        ("kyle_lambda", kyle_lambda(returns, bars.signed_volume), "no signed volume published"),
        ("edge_spread", edge_spread(bars), "fewer than 21 bars or degenerate prices"),
    ):
        if value is None:
            fp.absent[name] = reason
        else:
            fp.values[name] = value

    on_share, on_reason = overnight_share(bars)
    if on_share is None:
        fp.absent["overnight_share"] = on_reason or "unmeasurable"
    else:
        fp.values["overnight_share"] = on_share

    vol = np.asarray(bars.volume, dtype=np.float64)
    if (vol > 0).sum() >= 5:
        fp.values["turnover"] = float(np.mean(vol[vol > 0]))
    else:
        fp.absent["turnover"] = "no positive volume"

    return fp


def as_manifest(fp: Fingerprint, *, instrument_id: str, venue_id: int, window_spec: str) -> dict[str, Any]:
    """What the knowledge plane stores alongside the vector.

    `absent` travels with it. A consumer that treats a missing dimension as zero
    is making a measurement up, and the only way to stop that is to hand it the
    list of what was never measured.
    """
    return {
        "instrument_id": instrument_id,
        "venue_id": venue_id,
        "window_spec": window_spec,
        "info_class": "market_public",
        "dimensions": sorted(fp.values),
        "absent": fp.absent,
        "values": fp.values,
    }
