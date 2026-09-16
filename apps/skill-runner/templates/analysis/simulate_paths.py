"""Synthetic price paths, each labelled with how much it can be believed.

```json
{
  "name": "simulate_paths",
  "version": 1,
  "namespace": "analysis",
  "description": "Generates synthetic return paths from a named generator and reports the generator's trust tier, its stylised-fact fidelity, and every path the implausibility filter rejected. Use it to stress a strategy against histories that did not happen.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end", "seed", "generator"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "generator": {
        "type": "string",
        "enum": ["stationary_block_bootstrap", "iid_bootstrap", "garch_student_t", "gaussian"],
        "description": "how paths are produced; see the trust tiers"
      },
      "n_paths": {"type": "integer", "minimum": 10, "maximum": 5000, "default": 500},
      "horizon": {"type": "integer", "minimum": 10, "maximum": 10000, "default": 720},
      "max_annualised_return": {"type": "number", "minimum": 0.1, "maximum": 100, "default": 10.0,
                                "description": "implausibility bound; paths above it are rejected"}
    }
  },
  "summary_template": "{n_kept} of {n_requested} paths from `{generator}` (trust tier: {generator_trust_tier}); {n_rejected} rejected as implausible; stylised-fact fidelity {fidelity}; terminal return p05 {p05} / median {p50} / p95 {p95}",
  "detail_fields": [
    ["n_kept", "paths"],
    ["n_requested", "paths"],
    ["generator", ""],
    ["generator_trust_tier", ""],
    ["n_rejected", "paths"],
    ["fidelity", ""],
    ["p05", ""],
    ["p50", ""],
    ["p95", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

The trust tier is the output
-----------------------------
Every generator here produces paths that look like prices. They are not
interchangeable, and the difference is not a matter of degree:

* **`stationary_block_bootstrap` — `resampled_history`.** Every return in every
  path was an observed return, in blocks that preserve volatility clustering.
  It cannot produce a move that never happened, which is both its limitation and
  the reason its output is the only kind admissible as evidence.
* **`iid_bootstrap` — `resampled_history_iid`.** Same returns, order destroyed.
  Useful for one specific question — how much of a result is path-dependence —
  and wrong for every other, because it removes the volatility clustering that
  makes drawdowns.
* **`garch_student_t` — `parametric`.** A fitted model's output. It can produce
  moves that never happened, which is the point, and it inherits every one of
  the model's assumptions.
* **`gaussian` — `parametric_naive`.** A random walk with the sample's mean and
  variance. Present as a *reference*: a strategy that survives here and nowhere
  else is a strategy fitted to Gaussian noise.

There is deliberately no GAN or diffusion generator. The measured failure mode is
that they produce backtests returning 2000% annualised while passing stylised-fact
checks — which means a stylised-fact check is not a sufficient gate, and until
there is a gate that does work, a `generated_synthetic` tier would be a label on
something that should not be produced. AGENT-006 §5.10 names it as a tier; this
tool implements no generator that would claim it.

The implausibility filter
-------------------------
Paths whose annualised return exceeds `max_annualised_return` are **rejected and
counted**, not silently kept and not silently dropped. A generator that produces
many of them is a generator that is not calibrated, and the rejection count is the
only signal of that — which is exactly what the GAN failure looked like from
outside.

Fidelity
--------
Four stylised facts checked against the source series: return autocorrelation
(matched to the source, not to zero — a bid-ask bounce puts real negative
autocorrelation into observed bars), absolute-return autocorrelation, excess
kurtosis and tail weight. `fidelity` is how many of the four the generated paths reproduce. It is
reported *because it is not sufficient* — see above — and a caller reading `4/4`
as validation is reading it wrong; the trust tier is what bounds the claim.
"""

_TRUST_TIER = {
    "stationary_block_bootstrap": "resampled_history",
    "iid_bootstrap": "resampled_history_iid",
    "garch_student_t": "parametric",
    "gaussian": "parametric_naive",
}

_EVIDENTIARY = {"resampled_history"}


def _fit_garch(x):
    """Same quasi-likelihood GARCH(1,1) as `estimate_volatility`, plus the
    innovation's tail index, so the simulator draws from a t rather than a
    normal and reproduces the tails the source actually has."""
    if not _HAVE_SCIPY or x.size < 200:
        return None
    var = float(np.var(x, ddof=1))
    if var <= 0:
        return None

    def negative_log_likelihood(theta):
        omega, alpha, beta = np.exp(theta)
        if alpha + beta >= 0.999:
            return 1e10
        h = np.empty(x.size)
        h[0] = var
        for t in range(1, x.size):
            h[t] = omega + alpha * x[t - 1] ** 2 + beta * h[t - 1]
            if not np.isfinite(h[t]) or h[t] <= 0:
                return 1e10
        return 0.5 * float(np.sum(np.log(h) + x**2 / h))

    try:
        fit = _opt.minimize(
            negative_log_likelihood,
            np.log(np.array([var * 0.05, 0.08, 0.88])),
            method="Nelder-Mead",
            options={"maxiter": 2000},
        )
    except Exception:  # noqa: BLE001
        return None
    omega, alpha, beta = np.exp(fit.x)
    h = np.empty(x.size)
    h[0] = var
    for t in range(1, x.size):
        h[t] = omega + alpha * x[t - 1] ** 2 + beta * h[t - 1]
    standardised = x / np.sqrt(h)
    kurtosis = float(np.mean(standardised**4) / (np.mean(standardised**2) ** 2))
    # Method of moments for the degrees of freedom of a standardised t.
    nu = 4.0 + 6.0 / max(kurtosis - 3.0, 1e-6) if kurtosis > 3.0 else 30.0
    return {
        "omega": float(omega),
        "alpha": float(alpha),
        "beta": float(beta),
        "nu": float(min(max(nu, 2.5), 30.0)),
        "last_h": float(h[-1]),
        "last_x": float(x[-1]),
        "converged": bool(fit.success),
    }


def _generate(name, r, n_paths, horizon, generator, block):
    mean = float(np.mean(r))
    x = r - mean

    if name == "iid_bootstrap":
        index = generator.integers(0, r.size, size=(n_paths, horizon))
        return r[index], {"method": "iid resample"}

    if name == "stationary_block_bootstrap":
        # Politis-Romano: geometric block lengths, so the resampled series is
        # stationary rather than only blockwise stationary — which matters when
        # the paths are longer than the source.
        paths = np.empty((n_paths, horizon))
        p = 1.0 / max(block, 1)
        for i in range(n_paths):
            out = np.empty(horizon)
            pos = int(generator.integers(0, r.size))
            for t in range(horizon):
                out[t] = r[pos % r.size]
                if generator.random() < p:
                    pos = int(generator.integers(0, r.size))
                else:
                    pos += 1
            paths[i] = out
        return paths, {"method": "stationary block bootstrap", "mean_block": block}

    if name == "gaussian":
        sd = float(np.std(r, ddof=1))
        return generator.normal(mean, sd, size=(n_paths, horizon)), {"method": "gaussian"}

    if name == "garch_student_t":
        fit = _fit_garch(x)
        if fit is None:
            return None, {"error": "the GARCH fit did not converge"}
        nu = fit["nu"]
        z = generator.standard_t(nu, size=(n_paths, horizon)) / math.sqrt(nu / (nu - 2.0))
        paths = np.empty((n_paths, horizon))
        h = np.full(n_paths, fit["last_h"])
        previous = np.full(n_paths, fit["last_x"])
        for t in range(horizon):
            h = fit["omega"] + fit["alpha"] * previous**2 + fit["beta"] * h
            previous = np.sqrt(h) * z[:, t]
            paths[:, t] = previous + mean
        return paths, {"method": "GARCH(1,1) with standardised t innovations", **fit}

    return None, {"error": f"unknown generator {name!r}"}


def _stylised_facts(paths, source):
    """Four facts, checked on the generated paths against the source."""
    def facts(sample):
        return {
            "return_autocorr": autocorr(sample, 1),
            "abs_return_autocorr": autocorr(np.abs(sample), 1),
            "excess_kurtosis": float(
                np.mean((sample - sample.mean()) ** 4) / (np.var(sample) ** 2) - 3.0
            )
            if np.var(sample) > 0
            else float("nan"),
            "tail_fraction": float(np.mean(np.abs(sample - sample.mean()) > 3.0 * np.std(sample))),
        }

    reference = facts(source)
    flat = paths.reshape(-1)
    generated = facts(flat)

    checks = {
        # Match the SOURCE's return autocorrelation, not zero.
        #
        # "returns are serially uncorrelated" is a fact about efficient prices
        # and not about observed bars: a bid-ask bounce of s puts roughly
        # -(s/2)^2/var of negative autocorrelation into every close-to-close
        # series, and on a wide market that is large. Checking against zero
        # would mark a generator as unfaithful precisely when it faithfully
        # reproduced the microstructure — measured on the reference series,
        # whose 60 bps spread gives it a lag-1 autocorrelation of -0.33.
        "return_autocorrelation_matches": abs(
            generated["return_autocorr"] - reference["return_autocorr"]
        )
        < 0.1,
        # Clustering: absolute returns correlated, if the source has it.
        "volatility_clusters": (
            generated["abs_return_autocorr"] > 0.05
            if reference["abs_return_autocorr"] > 0.05
            else True
        ),
        # Fat tails within a factor of two of the source's.
        "excess_kurtosis": (
            math.isfinite(generated["excess_kurtosis"])
            and generated["excess_kurtosis"] > reference["excess_kurtosis"] / 2.0
        ),
        "tail_weight": generated["tail_fraction"] > reference["tail_fraction"] / 2.0,
    }
    return {"source": reference, "generated": generated, "checks": checks,
            "passed": int(sum(checks.values())), "of": len(checks)}


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    r = close_returns(frame)
    n = int(r.size)
    if n < 300:
        return refuse(
            "a generator fitted or resampled from fewer than 300 bars produces paths that are "
            "mostly a description of those 300 bars",
            n_bars=n,
        )

    name = payload.get("generator") or "stationary_block_bootstrap"
    tier = _TRUST_TIER.get(name)
    if tier is None:
        return refuse(f"unknown generator {name!r}", known=sorted(_TRUST_TIER))

    n_paths = int(payload.get("n_paths") or 500)
    horizon = int(payload.get("horizon") or 720)
    bound = float(payload.get("max_annualised_return") or 10.0)
    generator = rng(payload)
    block = optimal_block_length(r)

    paths, info = _generate(name, r, n_paths, horizon, generator, block)
    if paths is None:
        return refuse(info.get("error", "the generator failed"), generator=name)

    per_year = bars_per_year(payload.get("timeframe") or "1h")
    terminal = np.exp(paths.sum(axis=1)) - 1.0
    years = horizon / per_year if math.isfinite(per_year) and per_year > 0 else None

    # The implausibility filter. Rejected paths are counted and their extremes
    # reported: a generator producing many of them is a generator that is not
    # calibrated, and this count is the only thing that says so.
    if years and years > 0:
        annualised = np.sign(1.0 + terminal) * (np.abs(1.0 + terminal) ** (1.0 / years)) - 1.0
        implausible = np.abs(annualised) > bound
    else:
        annualised = np.full(terminal.shape, np.nan)
        implausible = np.zeros(terminal.shape, dtype=bool)

    kept = paths[~implausible]
    n_rejected = int(implausible.sum())
    if kept.shape[0] == 0:
        return refuse(
            f"every one of {n_paths} paths exceeded the implausibility bound of "
            f"{bound:.0%} annualised. The generator is not calibrated to this series and its "
            f"output must not be used",
            generator=name,
            generator_trust_tier=tier,
            n_rejected=n_rejected,
        )

    kept_terminal = terminal[~implausible]
    facts = _stylised_facts(kept, r)

    warnings = []
    if tier not in _EVIDENTIARY:
        warnings.append(
            f"`{name}` is trust tier `{tier}`. Results computed on these paths are a property "
            f"of the generator as much as of the market, and they must not be cited as evidence "
            f"the way a result on resampled history can be."
        )
    if n_rejected > n_paths * 0.01:
        warnings.append(
            f"{n_rejected} of {n_paths} paths ({n_rejected / n_paths:.1%}) exceeded the "
            f"implausibility bound. Above about 1% the generator is mis-calibrated, and the "
            f"surviving paths are a filtered sample rather than the generator's distribution."
        )
    if facts["passed"] < facts["of"]:
        failed = [k for k, v in facts["checks"].items() if not v]
        warnings.append(
            f"the generated paths do not reproduce {', '.join(failed)}. A strategy stressed "
            f"against them is being stressed against the wrong kind of market."
        )
    if name == "iid_bootstrap":
        warnings.append(
            "the i.i.d. bootstrap destroys volatility clustering, so drawdowns in these paths "
            "are systematically shallower than anything the market produces. It answers "
            "\"how much of this result is path dependence\" and no other question."
        )
    if facts["passed"] == facts["of"] and tier not in _EVIDENTIARY:
        warnings.append(
            "all four stylised facts pass and the trust tier is still not evidentiary. That is "
            "the intended reading: stylised-fact fidelity is a necessary check and is not a "
            "sufficient one — the measured failure is generators that pass it and produce "
            "2000% annualised backtests."
        )

    detail = {
        "n_source_bars": n,
        "generator": name,
        "generator_trust_tier": tier,
        "generator_info": {k: v for k, v in info.items() if k != "error"},
        "admissible_as_evidence": tier in _EVIDENTIARY,
        "n_requested": n_paths,
        "n_kept": int(kept.shape[0]),
        "n_rejected": n_rejected,
        "implausibility_bound_annualised": bound,
        "horizon": horizon,
        "horizon_years": years,
        "p05": float(np.percentile(kept_terminal, 5)),
        "p50": float(np.percentile(kept_terminal, 50)),
        "p95": float(np.percentile(kept_terminal, 95)),
        "terminal_mean": float(np.mean(kept_terminal)),
        "max_drawdown_p50": float(
            np.percentile([max_drawdown(np.exp(np.cumsum(p))) for p in kept[:200]], 50)
        ),
        "stylised_facts": facts,
        "fidelity": f"{facts['passed']}/{facts['of']}",
        "warnings": warnings,
        "window": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=int(kept.shape[0]))
