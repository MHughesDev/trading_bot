"""Which regime is this, by three methods that can disagree.

```json
{
  "name": "detect_regime",
  "version": 1,
  "namespace": "analysis",
  "description": "Labels each bar's volatility regime three ways - trailing terciles, a two-state Gaussian mixture, and a trend/mean-reversion score from Hurst, variance ratio and ADX. Use it before assuming a strategy's edge is regime-independent.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "window": {"type": "integer", "minimum": 10, "maximum": 1000, "default": 72,
                 "description": "trailing bars used for the volatility label"}
    }
  },
  "summary_template": "currently {current_regime} volatility ({current_share} of the sample is in this state); Hurst {hurst}, variance ratio {variance_ratio}, ADX {adx} - the series {trend_verdict}; the three methods {agreement}",
  "detail_fields": [
    ["current_regime", ""],
    ["current_share", "pct"],
    ["hurst", ""],
    ["variance_ratio", ""],
    ["adx", ""],
    ["trend_verdict", ""],
    ["agreement", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

Three methods, on purpose
-------------------------
A regime label is not a measurement, it is a model, and the three here fail
differently:

* **Trailing terciles** are non-parametric and honest about being relative: the
  "high" tercile of a calm year is lower than the "low" tercile of a violent one.
  They are also *causal* — each bar's label uses only bars before it, which is
  the whole reason this is not `pandas.qcut` over the full sample. Labelling with
  a full-sample quantile is a lookahead, and it is the single most common one in
  published regime work.
* **A two-state Gaussian mixture** on log absolute returns gives a probabilistic
  label and a transition matrix, so "how long does a regime last" has an answer.
  It assumes exactly two states, which is usually wrong and is at least wrong in
  a stated way.
* **Hurst, the variance ratio and ADX** answer a different question — trending or
  mean-reverting, not calm or violent. They are reported together because two of
  them agreeing and the third not is the usual case, and a single number here
  invites more confidence than any of them support.

The `agreement` field is the one to read first. When the three disagree, the
honest conclusion is that the series has no clean regime structure over this
window, and a strategy conditioned on a regime label is conditioned on noise.
"""


def _causal_terciles(volatility, window):
    """Label each bar against the distribution of the bars *before* it.

    The lookahead this avoids: a full-sample `qcut` tells bar 10 that it is in
    the calm tercile using information from bar 5,000. Every backtest built on
    that label is testing a strategy that knew the future distribution of
    volatility, and it will look excellent.
    """
    n = volatility.size
    labels = np.full(n, -1, dtype=int)
    # The first `window` bars have no history to be judged against and stay
    # unlabelled rather than being judged against a partial one.
    for i in range(window, n):
        history = volatility[:i]
        lo, hi = np.nanpercentile(history, [33.333, 66.667])
        v = volatility[i]
        labels[i] = 0 if v <= lo else (1 if v <= hi else 2)
    return labels


def _two_state_mixture(x, generator, iterations=200):
    """EM for a two-component Gaussian mixture, plus the implied transitions.

    Written out rather than imported (see requirements.txt). Initialised from the
    sample's own quantiles rather than randomly, so the fit is reproducible from
    the seed alone and the two states come out in a stable order — low first.
    """
    x = x[np.isfinite(x)]
    n = x.size
    if n < 100:
        return None

    lo, hi = np.percentile(x, [25.0, 75.0])
    mu = np.array([lo, hi], dtype=float)
    sigma = np.array([np.std(x) / 2.0, np.std(x) / 2.0], dtype=float)
    weight = np.array([0.5, 0.5])
    if not np.all(np.isfinite(sigma)) or np.any(sigma <= 0):
        return None

    responsibility = np.zeros((n, 2))
    previous = -np.inf
    for _ in range(iterations):
        for k in range(2):
            responsibility[:, k] = weight[k] * np.exp(
                -0.5 * ((x - mu[k]) / sigma[k]) ** 2
            ) / (sigma[k] * math.sqrt(2.0 * math.pi))
        total = responsibility.sum(axis=1, keepdims=True)
        total[total <= 0] = 1e-300
        log_likelihood = float(np.sum(np.log(total)))
        responsibility /= total

        counts = responsibility.sum(axis=0)
        if np.any(counts < 1e-6):
            return None
        weight = counts / n
        mu = (responsibility * x[:, None]).sum(axis=0) / counts
        sigma = np.sqrt(
            ((responsibility * (x[:, None] - mu) ** 2).sum(axis=0) / counts)
        )
        sigma = np.maximum(sigma, 1e-12)
        if abs(log_likelihood - previous) < 1e-8 * max(1.0, abs(previous)):
            break
        previous = log_likelihood

    order = np.argsort(mu)
    mu, sigma, weight = mu[order], sigma[order], weight[order]
    responsibility = responsibility[:, order]
    state = responsibility.argmax(axis=1)

    # The empirical transition matrix of the hard labels. Not the HMM's own — a
    # mixture has no transition parameter, and presenting one as though it did
    # would be claiming a model that was not fitted.
    transitions = np.zeros((2, 2))
    for a, b in zip(state[:-1], state[1:]):
        transitions[a, b] += 1
    rows = transitions.sum(axis=1, keepdims=True)
    rows[rows == 0] = 1.0
    transitions = transitions / rows

    return {
        "means": mu.tolist(),
        "sigmas": sigma.tolist(),
        "weights": weight.tolist(),
        "transition_matrix": transitions.tolist(),
        "expected_duration_bars": [
            float(1.0 / (1.0 - transitions[k, k])) if transitions[k, k] < 1 else None
            for k in range(2)
        ],
        "current_state": int(state[-1]),
        "current_probability": float(responsibility[-1, state[-1]]),
        "log_likelihood": previous,
    }


def _hurst(x):
    """Hurst exponent by rescaled range over dyadic block sizes.

    R/S rather than DFA because it needs no detrending choice, and a detrending
    choice is a knob an agent could turn until it liked the answer.
    """
    x = np.asarray(x, dtype=float)
    n = x.size
    if n < 128:
        return None
    sizes, values = [], []
    size = 16
    while size <= n // 4:
        blocks = n // size
        rescaled = []
        for b in range(blocks):
            chunk = x[b * size : (b + 1) * size]
            deviation = np.cumsum(chunk - chunk.mean())
            spread = float(deviation.max() - deviation.min())
            s = float(np.std(chunk, ddof=1))
            if s > 0 and spread > 0:
                rescaled.append(spread / s)
        if rescaled:
            sizes.append(size)
            values.append(float(np.mean(rescaled)))
        size *= 2
    if len(sizes) < 3:
        return None
    slope, _ = np.polyfit(np.log(sizes), np.log(values), 1)
    return float(slope)


def _variance_ratio(r, q=5):
    """Lo-MacKinlay variance ratio with the heteroskedasticity-robust statistic.

    The homoskedastic version rejects the random walk on essentially every
    financial series through volatility clustering alone, so it would report
    mean reversion that is not there.
    """
    r = np.asarray(r, dtype=float)
    n = r.size
    if n < q * 20:
        return None
    mu = r.mean()
    var1 = float(np.sum((r - mu) ** 2) / (n - 1))
    if var1 <= 0:
        return None
    aggregated = np.convolve(r, np.ones(q), mode="valid")
    varq = float(np.sum((aggregated - q * mu) ** 2) / ((n - q + 1) * q))
    vr = varq / var1

    # Robust standard error: a weighted sum of the heteroskedasticity-consistent
    # autocovariance terms.
    e2 = (r - mu) ** 2
    theta = 0.0
    for j in range(1, q):
        delta_num = float(np.sum(e2[j:] * e2[:-j]))
        delta = delta_num / (var1 * (n - 1)) ** 2 * (n - 1)
        theta += (2.0 * (q - j) / q) ** 2 * delta
    se = math.sqrt(theta) if theta > 0 else float("nan")
    z = (vr - 1.0) / se if se and math.isfinite(se) and se > 0 else float("nan")
    p = float(2.0 * st.norm.sf(abs(z))) if _HAVE_SCIPY and math.isfinite(z) else None
    return {"q": q, "ratio": float(vr), "z": float(z), "p_value": p}


def _adx(frame, period=14):
    """Wilder's ADX. Trend strength without direction."""
    h = pd.to_numeric(frame["high"], errors="coerce").to_numpy(dtype=float)
    l = pd.to_numeric(frame["low"], errors="coerce").to_numpy(dtype=float)
    c = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    n = c.size
    if n < period * 4:
        return None

    up = h[1:] - h[:-1]
    down = l[:-1] - l[1:]
    plus_dm = np.where((up > down) & (up > 0), up, 0.0)
    minus_dm = np.where((down > up) & (down > 0), down, 0.0)
    tr = np.maximum.reduce(
        [h[1:] - l[1:], np.abs(h[1:] - c[:-1]), np.abs(l[1:] - c[:-1])]
    )

    def wilder(x):
        out = np.empty(x.size)
        out[: period - 1] = np.nan
        out[period - 1] = x[:period].sum()
        for i in range(period, x.size):
            out[i] = out[i - 1] - out[i - 1] / period + x[i]
        return out

    atr = wilder(tr)
    with np.errstate(divide="ignore", invalid="ignore"):
        plus_di = 100.0 * wilder(plus_dm) / atr
        minus_di = 100.0 * wilder(minus_dm) / atr
        dx = 100.0 * np.abs(plus_di - minus_di) / (plus_di + minus_di)
    dx = dx[np.isfinite(dx)]
    if dx.size < period:
        return None
    adx = np.convolve(dx, np.ones(period) / period, mode="valid")
    return {"adx": float(adx[-1]), "adx_mean": float(np.mean(adx))}


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    r = close_returns(frame)
    n = int(r.size)
    if n < 200:
        return refuse(
            "a regime label on fewer than 200 bars is a label on one regime",
            n_bars=n,
        )

    window = int(payload.get("window") or 72)
    window = max(10, min(window, n // 4))

    # Trailing realised volatility, causal by construction.
    squared = r**2
    kernel = np.ones(window) / window
    trailing = np.sqrt(np.convolve(squared, kernel, mode="valid"))
    padded = np.concatenate([np.full(window - 1, np.nan), trailing])

    labels = _causal_terciles(padded, window)
    labelled = labels[labels >= 0]
    names = {0: "low", 1: "medium", 2: "high"}
    current = int(labels[-1]) if labels[-1] >= 0 else None
    shares = {
        names[k]: float(100.0 * np.mean(labelled == k)) if labelled.size else None
        for k in (0, 1, 2)
    }

    # The mixture runs on log absolute returns: the scale of a return is what
    # separates the states, and logging it makes the two components closer to
    # Gaussian than the raw returns ever are.
    magnitude = np.abs(r)
    magnitude = magnitude[magnitude > 0]
    mixture = _two_state_mixture(np.log(magnitude), rng(payload))

    # `r`, not `cumsum(r)`. R/S takes the *increments*: the cumulative deviation
    # is formed inside `_hurst`. Handing it the price path integrates twice and
    # returns H ~ 1.0 for a random walk, which reads as "strongly trending" for
    # every series ever measured.
    hurst = _hurst(r)
    vr = _variance_ratio(r)
    adx = _adx(frame)

    # Each method's vote on trending versus mean-reverting, and then whether they
    # agree. Votes rather than an average: averaging three numbers on three
    # different scales produces a number with no units and a lot of authority.
    votes = {}
    if hurst is not None:
        votes["hurst"] = "trending" if hurst > 0.55 else ("mean-reverting" if hurst < 0.45 else "neither")
    if vr and vr.get("p_value") is not None:
        if vr["p_value"] < 0.05:
            votes["variance_ratio"] = "trending" if vr["ratio"] > 1 else "mean-reverting"
        else:
            votes["variance_ratio"] = "neither"
    if adx:
        votes["adx"] = "trending" if adx["adx"] > 25 else "neither"

    distinct = set(votes.values())
    if not votes:
        trend_verdict, agreement = "undetermined", "could not be compared"
    elif len(distinct) == 1:
        trend_verdict = distinct.pop()
        agreement = "agree"
    else:
        trend_verdict = "is not cleanly one or the other"
        agreement = "disagree"

    warnings = []
    if agreement == "disagree":
        warnings.append(
            "the trend diagnostics disagree, so this window has no clean regime structure. A "
            "strategy conditioned on a regime label here is conditioned on the label's noise, "
            "and its out-of-sample behaviour will not resemble its backtest."
        )
    if mixture and mixture["current_probability"] < 0.7:
        warnings.append(
            f"the mixture assigns the latest bar to its state with probability "
            f"{mixture['current_probability']:.2f}, which is close to a coin flip: the current "
            f"label should not be used as a gate."
        )

    detail = {
        "n_bars": n,
        "window_bars": window,
        "current_regime": names.get(current) if current is not None else None,
        "current_share": shares.get(names.get(current)) if current is not None else None,
        "regime_shares_pct": shares,
        "mixture": mixture,
        "hurst": hurst,
        "variance_ratio": vr["ratio"] if vr else None,
        "variance_ratio_test": vr,
        "adx": adx["adx"] if adx else None,
        "adx_detail": adx,
        "votes": votes,
        "trend_verdict": trend_verdict,
        "agreement": agreement,
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n)
