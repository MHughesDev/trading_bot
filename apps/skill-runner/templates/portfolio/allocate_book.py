"""Weights across a family of strategies, with the cap outside the objective.

```json
{
  "name": "allocate_book",
  "version": 1,
  "namespace": "portfolio",
  "description": "Risk-parity or minimum-variance weights over an experiment's recorded strategy returns, with a hard per-strategy cap that no input can exceed. Use it to size a book; it proposes weights and deploys nothing.",
  "input_schema": {
    "type": "object",
    "required": ["experiment_id"],
    "additionalProperties": false,
    "properties": {
      "experiment_id": {"type": "string"},
      "objective": {
        "type": "string",
        "enum": ["risk_parity", "minimum_variance", "equal_weight"],
        "default": "risk_parity"
      },
      "min_overlap": {"type": "integer", "minimum": 20, "maximum": 5000, "default": 60}
    }
  },
  "summary_template": "{n_strategies} strategies, {objective}; largest weight {max_weight} against a cap of {cap}; {n_capped} hit the cap; effective breadth {effective_n} of {n_strategies}; {verdict}",
  "detail_fields": [
    ["n_strategies", ""],
    ["objective", ""],
    ["max_weight", ""],
    ["cap", ""],
    ["n_capped", ""],
    ["effective_n", ""],
    ["verdict", ""]
  ],
  "wall_seconds": 30,
  "series": [{"role": "family", "kind": "trial_returns"}]
}
```

The cap is not in the objective, and that is the whole design
--------------------------------------------------------------
A weight cap expressed as a *constraint inside* an optimisation is a constraint
the optimiser negotiates with: give it a return target it cannot reach under the
cap and it will find the corner of the feasible set that comes closest, which is
the cap, on the strategy with the best in-sample numbers. The cap holds and the
book is concentrated in the most overfitted candidate.

So `MAX_WEIGHT` is applied **after** the objective produces weights, by clipping
and redistributing, and then asserted. There is no input — no objective, no
covariance, no return target — that produces a weight above it, because nothing
that produces weights is consulted about it. A run whose weights fail the final
assertion refuses rather than returning them.

`MAX_WEIGHT` is a constant here and not an argument, for the same reason.

Why risk parity is the default
-------------------------------
Minimum variance is the objective that most rewards a covariance estimate being
wrong: it puts weight exactly where the estimated variance is lowest, and the
lowest estimated variance is usually the worst estimate rather than the calmest
strategy. Risk parity uses only the diagonal, which is the part of a covariance
matrix that is actually estimable at the sample sizes a strategy family has. Run
`shrunk_covariance` first and read `n_significant`: when no eigenvalue clears the
noise band, minimum variance is optimising over noise and equal weight is the
honest answer.

What it does not do
-------------------
It proposes. It does not deploy, size against capital, or write anything —
`Risk::Read` is enforced for every template, and the numbers here are a
suggestion for a human or a later gated tool to act on.
"""

#: The most any single strategy may receive, whatever the objective says.
#:
#: A constant and not an argument: a caller who can raise the cap can concentrate
#: the book, and the cap exists precisely to stop that from being reachable by
#: optimising.
MAX_WEIGHT = 0.25

#: The floor below which a weight is set to zero and redistributed.
#:
#: A book with a 0.3% allocation is a book paying a round trip of costs for
#: nothing. Zeroing it is more honest than reporting it.
MIN_WEIGHT = 0.01


def _cap_and_renormalise(weights, cap):
    """Clip to `cap` and push the excess onto whoever is under it.

    Iterative because one pass is not enough: redistributing to the uncapped
    strategies can push one of *them* over the cap. It terminates because each
    pass either caps another strategy or converges, and there are finitely many
    strategies.

    Returns `(weights, capped_mask)`, and the weights sum to one whenever the cap
    admits it — `n * cap >= 1` is checked by the caller.
    """
    w = np.array(weights, dtype=float)
    w = np.maximum(w, 0.0)
    total = w.sum()
    if total <= 0:
        w = np.full(w.size, 1.0 / w.size)
    else:
        w = w / total
    capped = np.zeros(w.size, dtype=bool)
    for _ in range(w.size + 1):
        over = w > cap + 1e-12
        if not over.any():
            break
        capped |= over
        excess = float((w[over] - cap).sum())
        w[over] = cap
        free = ~capped
        if not free.any():
            # Everyone is capped: the cap cannot admit a full allocation, which
            # the caller checks for, so this is a floor rather than a state.
            break
        room = w[free].sum()
        if room > 0:
            w[free] = w[free] + excess * (w[free] / room)
        else:
            w[free] = w[free] + excess / float(free.sum())
    return w, capped


def run(payload: dict) -> dict:
    frame = load_family(payload, "family")
    table = frame
    if "run_id" not in table.columns:
        return refuse("the materialised family has no `run_id` column")

    wide = table.pivot_table(index="period", columns="run_id", values="ret", aggfunc="first")
    min_overlap = int(payload.get("min_overlap") or 60)
    counts = wide.notna().sum(axis=0)
    excluded = [str(c) for c in counts.index[counts < min_overlap]]
    wide = wide.drop(columns=excluded).dropna(axis=0, how="any")

    names = [str(c) for c in wide.columns]
    x = wide.to_numpy(dtype=float)
    t, n = x.shape
    if n < 2:
        return refuse(
            f"fewer than two strategies have {min_overlap} periods of returns; there is no "
            f"book to allocate",
            excluded=excluded,
        )
    if n * MAX_WEIGHT < 1.0:
        # With a 0.25 cap that means fewer than four strategies. Refusing is the
        # honest answer: no allocation summing to one respects the cap, and
        # silently relaxing it would defeat the cap in exactly the case where
        # concentration is most likely.
        return refuse(
            f"{n} strategies cannot be allocated under a per-strategy cap of {MAX_WEIGHT:.0%}: "
            f"no set of weights both sums to one and respects it. A book this narrow is a "
            f"concentration decision, not an allocation",
            n_strategies=n,
            cap=MAX_WEIGHT,
        )

    objective = payload.get("objective") or "risk_parity"
    centred = x - x.mean(axis=0)
    covariance = (centred.T @ centred) / t
    variances = np.diag(covariance).copy()

    if objective == "equal_weight":
        raw = np.full(n, 1.0 / n)
    elif objective == "risk_parity":
        # Inverse volatility: the diagonal only. The off-diagonal of a strategy
        # family's covariance is mostly estimation noise at these sample sizes,
        # and an allocation that uses it is an allocation to the noise.
        sd = np.sqrt(np.maximum(variances, 1e-300))
        inverse = np.where(sd > 0, 1.0 / sd, 0.0)
        raw = inverse / inverse.sum() if inverse.sum() > 0 else np.full(n, 1.0 / n)
    elif objective == "minimum_variance":
        try:
            inverse_cov = np.linalg.pinv(covariance)
        except np.linalg.LinAlgError:
            return refuse("the covariance matrix could not be inverted")
        ones = np.ones(n)
        denominator = float(ones @ inverse_cov @ ones)
        if abs(denominator) < 1e-300:
            return refuse("the minimum-variance solution is degenerate on this family")
        raw = (inverse_cov @ ones) / denominator
    else:
        return refuse(f"unknown objective {objective!r}")

    weights, capped = _cap_and_renormalise(raw, MAX_WEIGHT)

    # The floor, applied after the cap and re-capped, because zeroing small
    # weights concentrates the rest.
    small = (weights > 0) & (weights < MIN_WEIGHT)
    if small.any() and (~small).sum() >= 1:
        weights[small] = 0.0
        weights, capped_again = _cap_and_renormalise(weights, MAX_WEIGHT)
        capped |= capped_again

    # The assertion the whole design exists for. Nothing that produced these
    # weights was consulted about the cap, so this cannot be satisfied by an
    # optimiser finding the corner of a feasible set.
    largest = float(weights.max())
    if largest > MAX_WEIGHT + 1e-9:
        return refuse(
            f"the allocation produced a weight of {largest:.4f} above the cap of {MAX_WEIGHT}, "
            f"which should be unreachable. Refusing rather than returning it",
            objective=objective,
            weights=weights.tolist(),
        )

    # Effective breadth: the inverse Herfindahl. Twelve strategies at 1/12 each
    # has breadth 12; twelve with one at the cap and the rest tiny has breadth
    # near 1, and the count alone would not say so.
    herfindahl = float(np.sum(weights**2))
    effective_n = 1.0 / herfindahl if herfindahl > 0 else 0.0

    portfolio = x @ weights
    portfolio_sd = float(np.std(portfolio, ddof=1)) if t > 1 else float("nan")
    average_sd = float(np.mean(np.sqrt(np.maximum(variances, 0.0))))

    n_capped = int(capped.sum())
    if n_capped:
        verdict = (
            f"{n_capped} strategy(ies) were cut to the cap; the objective wanted more in them "
            f"than a book should hold in any one thing"
        )
    elif effective_n < n / 2:
        verdict = (
            f"effective breadth {effective_n:.1f} against {n} strategies — this is more "
            f"concentrated than the count suggests"
        )
    else:
        verdict = "no strategy reached the cap and the book is broadly spread"

    warnings = []
    if n_capped:
        warnings.append(
            f"{n_capped} weight(s) were clipped to {MAX_WEIGHT:.0%} and the excess redistributed. "
            f"The cap is applied after the objective and cannot be reached by optimising, which "
            f"is why a return target this book cannot meet produces a diversified book rather "
            f"than a concentrated one."
        )
    if objective == "minimum_variance":
        warnings.append(
            "minimum variance is the objective most rewarded by a wrong covariance: it puts "
            "weight where estimated variance is lowest, and the lowest estimate is usually the "
            "worst one. Check `shrunk_covariance`'s `n_significant` before trusting this — if "
            "no eigenvalue clears the noise band, this is optimising over noise."
        )
    if portfolio_sd and average_sd and portfolio_sd > average_sd:
        warnings.append(
            "the book's volatility is above the average of its members', so these strategies "
            "are positively correlated enough that combining them is not diversifying."
        )
    if excluded:
        warnings.append(
            f"{len(excluded)} strategy(ies) were excluded for fewer than {min_overlap} common "
            f"periods and hold no weight here: {', '.join(excluded[:10])}."
        )

    detail = {
        "experiment_id": payload.get("experiment_id"),
        "objective": objective,
        "cap": MAX_WEIGHT,
        "floor": MIN_WEIGHT,
        "n_strategies": n,
        "n_periods": t,
        "excluded_strategies": excluded,
        "weights": {name: float(w) for name, w in zip(names, weights)},
        "weights_before_cap": {name: float(w) for name, w in zip(names, np.asarray(raw, float))},
        "capped": [name for name, c in zip(names, capped) if c],
        "n_capped": n_capped,
        "max_weight": largest,
        "effective_n": effective_n,
        "herfindahl": herfindahl,
        "portfolio_sd_per_period": portfolio_sd,
        "mean_member_sd_per_period": average_sd,
        "diversification_ratio": (
            average_sd / portfolio_sd if portfolio_sd and portfolio_sd > 0 else None
        ),
        "verdict": verdict,
        "warnings": warnings,
        "note": "these are proposed weights. Nothing here deploys, sizes against capital, or "
                "writes anything.",
    }
    return result(detail, n=t)
