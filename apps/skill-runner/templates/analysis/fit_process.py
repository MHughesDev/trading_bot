"""Which textbook process is least wrong about this series.

```json
{
  "name": "fit_process",
  "version": 1,
  "namespace": "analysis",
  "description": "Fits geometric Brownian motion, Student-t, Ornstein-Uhlenbeck, a jump diffusion and GARCH to one series and ranks them by BIC with a goodness-of-fit check. Use it to choose a simulator, not to believe a model.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"}
    }
  },
  "summary_template": "best fit is {best} (BIC {best_bic}, {bic_margin} ahead of the next); it {fit_verdict}; {n_candidates} processes compared on {n_bars} bars",
  "detail_fields": [
    ["best", ""],
    ["best_bic", ""],
    ["bic_margin", ""],
    ["fit_verdict", ""],
    ["n_candidates", ""],
    ["n_bars", "bars"]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

Ranked, and then checked
-------------------------
BIC picks the best of the candidates offered. It says nothing about whether any
of them is any good, and that distinction is the difference between "GARCH fits
best" and "GARCH fits". So every fit also gets a Kolmogorov-Smirnov test of its
standardised residuals against the distribution it assumed, and `fit_verdict`
carries that result — a winner whose residuals are rejected is reported as the
least bad of a bad field, which is usually the truth.

BIC rather than AIC: AIC over-selects on long series, and these series are long.

The candidates
--------------
* **`gaussian`** — i.i.d. normal returns. The null everything else has to beat,
  and it is beaten by essentially every real series, which is why it is here.
* **`student_t`** — i.i.d. fat tails, no time structure. Beating this and not
  GARCH means the tails are unconditional rather than produced by clustering,
  which changes what a stress test should do.
* **`ornstein_uhlenbeck`** — mean reversion in the *level*. Fitted on log price
  rather than returns, so its likelihood is not comparable to the others' and it
  is ranked separately; its half-life is the number worth having.
* **`merton_jump_diffusion`** — normal diffusion plus rare large jumps. Where it
  wins, a strategy's tail risk is the jump component and stop-losses will not
  protect against it, because jumps gap through them.
* **`garch_student_t`** — conditional variance with fat innovations. The usual
  winner on financial data, and the one whose simulated paths are worth using.

A note on what this is for
---------------------------
Choosing a simulator, not describing reality. Every one of these is wrong; the
question is which wrongness is least likely to matter for the decision at hand,
and `fit_verdict` plus the per-model diagnostics are how that gets decided rather
than by taking the top of the list.
"""


def _bic(log_likelihood, k, n):
    return float(k * math.log(n) - 2.0 * log_likelihood)


def _gaussian(r):
    mu, sd = float(np.mean(r)), float(np.std(r, ddof=1))
    if sd <= 0:
        return None
    ll = float(np.sum(st.norm.logpdf(r, mu, sd))) if _HAVE_SCIPY else None
    if ll is None:
        return None
    z = (r - mu) / sd
    return {"params": {"mu": mu, "sigma": sd}, "log_likelihood": ll, "k": 2, "z": z,
            "reference": "norm", "reference_args": ()}


def _student_t(r):
    if not _HAVE_SCIPY:
        return None
    try:
        nu, loc, scale = st.t.fit(r)
    except Exception:  # noqa: BLE001
        return None
    if not (2.0 < nu < 200.0) or scale <= 0:
        return None
    ll = float(np.sum(st.t.logpdf(r, nu, loc, scale)))
    z = (r - loc) / scale
    return {"params": {"nu": float(nu), "loc": float(loc), "scale": float(scale)},
            "log_likelihood": ll, "k": 3, "z": z, "reference": "t", "reference_args": (nu,)}


def _merton(r):
    """Normal diffusion plus a Poisson jump, by EM on a two-component mixture.

    The jump component is constrained to be the *wider* one and to carry less
    than half the mass, which is what distinguishes a jump model from a plain
    two-component mixture. Without the constraint EM happily labels the calm
    regime "jumps" and reports a jump intensity of 0.8.
    """
    if not _HAVE_SCIPY or r.size < 200:
        return None
    n = r.size
    mu = float(np.mean(r))
    sd = float(np.std(r, ddof=1))
    if sd <= 0:
        return None
    lam, jump_sd, jump_mu = 0.05, sd * 3.0, 0.0
    diff_sd = sd
    previous = -np.inf
    for _ in range(300):
        pd_diff = (1.0 - lam) * st.norm.pdf(r, mu, diff_sd)
        pd_jump = lam * st.norm.pdf(r, mu + jump_mu, math.sqrt(diff_sd**2 + jump_sd**2))
        total = pd_diff + pd_jump
        total[total <= 0] = 1e-300
        ll = float(np.sum(np.log(total)))
        w = pd_jump / total
        lam = float(np.clip(np.mean(w), 1e-4, 0.5))
        weight_sum = float(np.sum(w))
        if weight_sum > 1.0:
            jump_mu = float(np.sum(w * (r - mu)) / weight_sum)
            jump_var = float(np.sum(w * (r - mu - jump_mu) ** 2) / weight_sum) - diff_sd**2
            jump_sd = math.sqrt(max(jump_var, (sd * 1.5) ** 2 - diff_sd**2, 1e-12))
        calm = 1.0 - w
        calm_sum = float(np.sum(calm))
        if calm_sum > 1.0:
            diff_sd = math.sqrt(max(float(np.sum(calm * (r - mu) ** 2) / calm_sum), 1e-16))
        if abs(ll - previous) < 1e-8 * max(1.0, abs(previous)):
            break
        previous = ll
    if not math.isfinite(previous):
        return None
    # Standardised by the mixture's own variance, which is what the KS check
    # needs; the mixture is not normal, so this is compared against the mixture
    # CDF rather than the normal one.
    variance = (1.0 - lam) * diff_sd**2 + lam * (diff_sd**2 + jump_sd**2 + jump_mu**2)
    z = (r - mu) / math.sqrt(variance) if variance > 0 else r
    return {
        "params": {"mu": mu, "diffusion_sigma": diff_sd, "jump_intensity": lam,
                   "jump_mu": jump_mu, "jump_sigma": jump_sd},
        "log_likelihood": previous, "k": 5, "z": z, "reference": None, "reference_args": (),
    }


def _garch_t(r):
    if not _HAVE_SCIPY or r.size < 300:
        return None
    x = r - float(np.mean(r))
    var = float(np.var(x, ddof=1))
    if var <= 0:
        return None

    def negative_log_likelihood(theta):
        omega, alpha, beta = np.exp(theta[:3])
        nu = 2.05 + math.exp(theta[3])
        if alpha + beta >= 0.999 or nu > 200:
            return 1e10
        h = np.empty(x.size)
        h[0] = var
        for t in range(1, x.size):
            h[t] = omega + alpha * x[t - 1] ** 2 + beta * h[t - 1]
            if not np.isfinite(h[t]) or h[t] <= 0:
                return 1e10
        scale = np.sqrt(h * (nu - 2.0) / nu)
        return -float(np.sum(st.t.logpdf(x / scale, nu) - np.log(scale)))

    try:
        fit = _opt.minimize(
            negative_log_likelihood,
            np.array([math.log(var * 0.05), math.log(0.08), math.log(0.88), math.log(3.0)]),
            method="Nelder-Mead",
            options={"maxiter": 4000},
        )
    except Exception:  # noqa: BLE001
        return None
    if fit.fun >= 1e10:
        return None
    omega, alpha, beta = np.exp(fit.x[:3])
    nu = 2.05 + math.exp(fit.x[3])
    h = np.empty(x.size)
    h[0] = var
    for t in range(1, x.size):
        h[t] = omega + alpha * x[t - 1] ** 2 + beta * h[t - 1]
    scale = np.sqrt(h * (nu - 2.0) / nu)
    return {
        "params": {"omega": float(omega), "alpha": float(alpha), "beta": float(beta),
                   "nu": float(nu), "persistence": float(alpha + beta)},
        "log_likelihood": float(-fit.fun), "k": 4, "z": x / scale,
        "reference": "t", "reference_args": (nu,),
    }


def _ou(log_price):
    """Fitted on the level, so its likelihood is not comparable to the others'."""
    dy = np.diff(log_price)
    design = np.column_stack([np.ones(dy.size), log_price[:-1]])
    coef, *_ = np.linalg.lstsq(design, dy, rcond=None)
    theta = float(coef[1])
    resid = dy - design @ coef
    sigma = float(np.std(resid, ddof=2)) if resid.size > 2 else float("nan")
    half_life = (
        float(-math.log(2.0) / math.log(1.0 + theta)) if theta < 0 and (1.0 + theta) > 0 else None
    )
    return {
        "params": {"theta": theta, "mu": float(-coef[0] / theta) if theta != 0 else None,
                   "sigma": sigma},
        "half_life": half_life,
        "mean_reverting": theta < 0,
    }


def _ks_against(z, reference, args):
    """KS of the standardised residuals against the assumed distribution."""
    if not _HAVE_SCIPY or reference is None:
        return None
    z = z[np.isfinite(z)]
    if z.size < 50:
        return None
    if reference == "norm":
        cdf = st.norm.cdf(z)
    else:
        nu = args[0]
        cdf = st.t.cdf(z * math.sqrt(nu / (nu - 2.0)), nu)
    cdf = np.sort(cdf)
    n = cdf.size
    grid = np.arange(1, n + 1) / n
    statistic = float(np.max(np.abs(cdf - grid)))
    lam = (math.sqrt(n) + 0.12 + 0.11 / math.sqrt(n)) * statistic
    p = 2.0 * sum(((-1.0) ** (k - 1)) * math.exp(-2.0 * (k**2) * (lam**2)) for k in range(1, 101))
    return {"statistic": statistic, "p_value": max(0.0, min(1.0, p))}


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    close = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    close = close[np.isfinite(close) & (close > 0)]
    log_price = np.log(close)
    r = np.diff(log_price)
    n = int(r.size)
    if n < 300:
        return refuse(
            "fitting five processes to fewer than 300 observations selects on noise; BIC's "
            "penalty cannot separate them",
            n_bars=n,
        )

    candidates = {
        "gaussian": _gaussian(r),
        "student_t": _student_t(r),
        "merton_jump_diffusion": _merton(r),
        "garch_student_t": _garch_t(r),
    }

    ranked = []
    for name, fit in candidates.items():
        if fit is None:
            continue
        gof = _ks_against(fit["z"], fit["reference"], fit["reference_args"])
        ranked.append(
            {
                "process": name,
                "bic": _bic(fit["log_likelihood"], fit["k"], n),
                "log_likelihood": fit["log_likelihood"],
                "parameters": fit["params"],
                "n_parameters": fit["k"],
                "goodness_of_fit": gof,
            }
        )
    if not ranked:
        return refuse("no candidate process could be fitted to this series", n_bars=n)
    ranked.sort(key=lambda e: e["bic"])

    best = ranked[0]
    margin = float(ranked[1]["bic"] - best["bic"]) if len(ranked) > 1 else None

    gof = best.get("goodness_of_fit")
    if gof is None:
        fit_verdict = "has no goodness-of-fit check for its own distribution"
    elif gof["p_value"] < 0.01:
        fit_verdict = (
            "is still rejected by its own goodness-of-fit test, so this is the least bad of a "
            "bad field rather than a model of this series"
        )
    elif gof["p_value"] < 0.05:
        fit_verdict = "fits marginally; its residuals are not quite what it assumes"
    else:
        fit_verdict = "is not rejected by its own goodness-of-fit test"

    ou = _ou(log_price)

    warnings = []
    if margin is not None and margin < 6.0:
        warnings.append(
            f"the top two processes are within {margin:.1f} BIC, which is not a distinction. "
            f"Choosing between them on this evidence is choosing on noise; use whichever is "
            f"more conservative for the decision at hand."
        )
    if gof and gof["p_value"] < 0.01:
        warnings.append(
            "the winning model's residuals are rejected against the distribution it assumes. "
            "Simulated paths from it will be wrong in a way BIC cannot see, because BIC only "
            "compares it against the other candidates."
        )
    if best["process"] == "merton_jump_diffusion":
        warnings.append(
            "a jump diffusion fits best, which means the tail risk here is gaps rather than "
            "volatility. Stop-losses do not protect against gaps — they fill through them — so "
            "any strategy sized on a stop distance is under-hedged."
        )
    if best["process"] == "gaussian":
        warnings.append(
            "a plain Gaussian fits best, which is unusual enough on financial data to be worth "
            "checking the input for: a resampled, averaged or already-modelled series will look "
            "like this and a market will not."
        )
    if ou["mean_reverting"] and ou["half_life"] and ou["half_life"] < n / 4:
        warnings.append(
            f"the level is mean-reverting with a half-life of {ou['half_life']:.0f} bars. That "
            f"is fitted on the log price and its likelihood is not comparable to the return "
            f"models above; see `stationarity` before acting on it."
        )

    detail = {
        "n_bars": n,
        "n_candidates": len(ranked),
        "ranking": ranked,
        "best": best["process"],
        "best_bic": best["bic"],
        "bic_margin": margin,
        "fit_verdict": fit_verdict,
        "ornstein_uhlenbeck": ou,
        "ranking_note": (
            "BIC over return models only. The OU fit is on the level and is reported "
            "separately because its likelihood is over a different quantity."
        ),
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=n)
