"""The covariance of a family of strategies, shrunk — with the raw one beside it.

```json
{
  "name": "shrunk_covariance",
  "version": 1,
  "namespace": "portfolio",
  "description": "Ledoit-Wolf shrunk covariance across an experiment's recorded strategy returns, reported alongside the raw sample covariance and the Marchenko-Pastur noise band. Use it before sizing a book of several strategies.",
  "input_schema": {
    "type": "object",
    "required": ["experiment_id"],
    "additionalProperties": false,
    "properties": {
      "experiment_id": {"type": "string"},
      "min_overlap": {"type": "integer", "minimum": 20, "maximum": 5000, "default": 60,
                      "description": "periods two strategies must share before their covariance is estimated"}
    }
  },
  "summary_template": "{n_strategies} strategies over {n_periods} common periods (q={aspect_ratio}); shrinkage intensity {shrinkage}; {n_significant} of {n_eigenvalues} eigenvalues clear the Marchenko-Pastur noise band; {verdict}",
  "detail_fields": [
    ["n_strategies", ""],
    ["n_periods", "periods"],
    ["aspect_ratio", ""],
    ["shrinkage", ""],
    ["n_significant", ""],
    ["n_eigenvalues", ""],
    ["verdict", ""]
  ],
  "wall_seconds": 30,
  "series": [{"role": "family", "kind": "trial_returns"}]
}
```

Raw is always reported beside shrunk
-------------------------------------
Shrinkage pulls the sample covariance toward a structured target, and the
structure it pulls toward is *historical average correlation*. That is exactly
wrong before a correlation breakdown: the moment diversification stops working is
the moment the shrunk estimate is most confidently claiming it still does. So the
raw estimate is reported too, and so is the shrinkage intensity — a `shrinkage`
near 1 means the answer is almost entirely the target and almost not at all this
data.

The intensity is not a knob. It is Ledoit-Wolf's analytically optimal value for
minimising expected squared error against the true covariance, and exposing it as
a parameter would let a caller shrink until the book looked diversified.

Marchenko-Pastur
----------------
With `N` strategies and `T` periods, the eigenvalues of a covariance estimated
from *pure noise* still spread out, and the spread depends only on `q = N/T`. The
upper edge is `(1 + sqrt(q))²` times the average variance. An eigenvalue below
that edge is not a weak factor — it is indistinguishable from noise, and a
portfolio weighted on it is weighted on nothing.

`n_significant` is how many clear the edge. When it is zero, the correlation
structure in this family is not measurable at this sample size, and equal weights
are a better answer than anything an optimiser will produce from this matrix.

Ragged families
---------------
Runs cover different windows. This uses the periods **common to all** strategies
and says how many were dropped, rather than padding (which invents returns) or
pairwise-deleting (which produces a matrix that need not be positive
semi-definite, and then an optimiser that silently returns nonsense). A strategy
sharing fewer than `min_overlap` periods with the rest is excluded and named.
"""


def run(payload: dict) -> dict:
    frame = load(payload, "family")
    for column in ("run_id", "period", "ret"):
        if column not in frame.columns and frame.index.name != column:
            return refuse(
                f"the materialised family has no `{column}` column; this template reads the "
                f"long-format trial-returns table"
            )

    table = frame.reset_index() if frame.index.name == "period" else frame.copy()
    wide = table.pivot_table(index="period", columns="run_id", values="ret", aggfunc="first")

    min_overlap = int(payload.get("min_overlap") or 60)
    # Drop strategies too short to be estimated against the rest, and say which.
    counts = wide.notna().sum(axis=0)
    excluded = [str(c) for c in counts.index[counts < min_overlap]]
    wide = wide.drop(columns=excluded)
    if wide.shape[1] < 2:
        return refuse(
            f"fewer than two strategies have {min_overlap} periods of returns; a covariance "
            f"needs at least two",
            n_excluded=len(excluded),
            excluded=excluded,
        )

    before = int(wide.shape[0])
    wide = wide.dropna(axis=0, how="any")
    dropped = before - int(wide.shape[0])
    x = wide.to_numpy(dtype=float)
    t, n = x.shape
    if t < max(min_overlap, n + 2):
        return refuse(
            f"only {t} periods are common to all {n} strategies. A covariance over fewer "
            f"periods than strategies is singular, and one barely above is noise",
            n_periods=t,
            n_strategies=n,
        )

    centred = x - x.mean(axis=0)
    sample = (centred.T @ centred) / t

    # Ledoit-Wolf against the constant-correlation target: each variance kept,
    # every correlation replaced by the family's average. The identity target is
    # the more common textbook choice and is worse here — it assumes strategies
    # are uncorrelated, which for variants of one idea is the assumption most
    # likely to be false.
    variances = np.diag(sample).copy()
    sd = np.sqrt(np.maximum(variances, 1e-300))
    correlation = sample / np.outer(sd, sd)
    off = ~np.eye(n, dtype=bool)
    mean_correlation = float(correlation[off].mean()) if n > 1 else 0.0
    target = np.outer(sd, sd) * mean_correlation
    np.fill_diagonal(target, variances)

    # The optimal intensity: expected estimation error of the sample covariance
    # over its squared distance from the target.
    squared = centred**2
    pi_matrix = (squared.T @ squared) / t - sample**2
    pi_hat = float(pi_matrix.sum())
    rho_hat = float(np.trace(pi_matrix))
    gamma_hat = float(((target - sample) ** 2).sum())
    shrinkage = 0.0 if gamma_hat <= 0 else (pi_hat - rho_hat) / gamma_hat / t
    shrinkage = float(min(max(shrinkage, 0.0), 1.0))
    shrunk = shrinkage * target + (1.0 - shrinkage) * sample

    # Marchenko-Pastur: the noise edge depends only on q = N/T.
    q = n / t
    average_variance = float(np.mean(variances))
    upper_edge = average_variance * (1.0 + math.sqrt(q)) ** 2
    eigenvalues = np.sort(np.linalg.eigvalsh(sample))[::-1]
    n_significant = int(np.sum(eigenvalues > upper_edge))

    condition_sample = (
        float(eigenvalues[0] / eigenvalues[-1]) if eigenvalues[-1] > 1e-300 else None
    )
    shrunk_eigenvalues = np.sort(np.linalg.eigvalsh(shrunk))[::-1]
    condition_shrunk = (
        float(shrunk_eigenvalues[0] / shrunk_eigenvalues[-1])
        if shrunk_eigenvalues[-1] > 1e-300
        else None
    )

    if n_significant == 0:
        verdict = (
            "no eigenvalue clears the noise band, so the correlation structure here is not "
            "measurable at this sample size — equal weights beat anything an optimiser "
            "produces from this matrix"
        )
    elif shrinkage > 0.7:
        verdict = (
            f"shrinkage is {shrinkage:.2f}, so the answer is mostly the target and barely this "
            f"data; treat the correlations as assumed rather than measured"
        )
    else:
        verdict = f"{n_significant} factor(s) are distinguishable from noise"

    warnings = []
    if excluded:
        warnings.append(
            f"{len(excluded)} strategy(ies) were excluded for fewer than {min_overlap} periods: "
            f"{', '.join(excluded[:10])}. They are not in this matrix and must not be sized "
            f"from it."
        )
    if dropped:
        warnings.append(
            f"{dropped} period(s) were dropped because at least one strategy had no return "
            f"there. The matrix is over the {t} periods common to all — padding would have "
            f"invented returns, and pairwise deletion would produce a matrix an optimiser can "
            f"silently fail on."
        )
    if q > 0.5:
        warnings.append(
            f"q = N/T is {q:.2f}: with {n} strategies over {t} periods, most of this sample "
            f"covariance is estimation noise whatever the shrinkage does with it."
        )
    warnings.append(
        "the shrinkage target is the family's average correlation, so the shrunk estimate is "
        "biased toward historical structure. That is exactly wrong just before a correlation "
        "breakdown, which is when a book most needs to be right — the raw estimate is reported "
        "beside it for that reason and is not decoration."
    )

    detail = {
        "n_strategies": n,
        "n_periods": t,
        "n_periods_dropped": dropped,
        "excluded_strategies": excluded,
        "strategies": [str(c) for c in wide.columns],
        "aspect_ratio": q,
        "shrinkage": shrinkage,
        "mean_correlation": mean_correlation,
        "covariance_shrunk": shrunk.tolist(),
        "covariance_raw": sample.tolist(),
        "variances": variances.tolist(),
        "eigenvalues_raw": eigenvalues.tolist(),
        "eigenvalues_shrunk": shrunk_eigenvalues.tolist(),
        "marchenko_pastur_upper_edge": upper_edge,
        "n_eigenvalues": int(eigenvalues.size),
        "n_significant": n_significant,
        "condition_number_raw": condition_sample,
        "condition_number_shrunk": condition_shrunk,
        "verdict": verdict,
        "warnings": warnings,
        "experiment_id": payload.get("experiment_id"),
    }
    return result(detail, n=t)
