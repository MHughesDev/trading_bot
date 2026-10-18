"""How much of this candidate's return survives shrinkage toward its own family.

```json
{
  "name": "posterior_alpha",
  "version": 1,
  "namespace": "attribution",
  "description": "Empirical-Bayes shrinkage of a candidate's mean return toward the experiment's family, with a 95% credible interval and whether it excludes zero. Run it on the member that looks best before believing it.",
  "input_schema": {
    "type": "object",
    "required": ["experiment_id"],
    "additionalProperties": false,
    "properties": {
      "experiment_id": {"type": "string"},
      "candidate": {
        "type": "string",
        "description": "the run to report; omitted, the member with the highest raw mean is reported and the result says it was chosen that way"
      },
      "prior_mean": {"type": "number", "description": "an archetype prior from research memory, per period; omitted, the family is the prior"},
      "prior_sd": {"type": "number", "minimum": 0, "description": "the prior's spread, per period; needs prior_mean"},
      "min_periods": {"type": "integer", "minimum": 20, "maximum": 5000, "default": 60}
    }
  },
  "summary_template": "{n_candidates} candidates, theme mean {prior_mean} per period ({prior_source} prior); {candidate}: raw {raw_mean}, shrunk {post_mean} [{ci_lo}, {ci_hi}], {verdict}",
  "detail_fields": [
    ["n_candidates", ""],
    ["prior_source", ""],
    ["prior_mean", ""],
    ["between_sd", ""],
    ["candidate", ""],
    ["selected_by", ""],
    ["raw_mean", ""],
    ["shrinkage", ""],
    ["post_mean", ""],
    ["ci_lo", ""],
    ["ci_hi", ""],
    ["excludes_zero", ""],
    ["verdict", ""]
  ],
  "wall_seconds": 60,
  "series": [{"role": "family", "kind": "trial_returns"}]
}
```

What this answers, and the mistake it exists to prevent
--------------------------------------------------------
An experiment tries many configurations and one of them has the best mean. That
mean is the maximum of a noisy sample, so it is biased upward by exactly the
amount the selection rewarded: the winner's curse. A deflated Sharpe corrects the
*significance* of the winner; this corrects the *estimate*, by asking how far the
winner's mean should be pulled back toward what the rest of its theme earns.

The model is the usual normal-normal one. Each candidate's true mean is drawn from
a theme distribution with mean `mu0` and between-candidate standard deviation
`tau`; its sample mean has standard error `se_i`. Then

    posterior mean = B * mu0 + (1 - B) * raw_i,    B = se_i^2 / (se_i^2 + tau^2)

so a candidate whose mean is noisy relative to how much candidates really differ
is pulled hard, and one measured precisely in a family that genuinely varies is
barely moved. `tau^2` is estimated by the method of moments (the spread of raw
means minus the average sampling variance, floored at zero), and `se_i` is the
Newey-West HAC error rather than `s / sqrt(n)`, because an autocorrelated series
makes the plain error flatter the candidate.

The posterior variance carries the uncertainty in `mu0` too:
`(1 - B) * se_i^2 + B^2 * var(mu0_hat)`. Leaving that out would make a family with
few members look as if it knew its own mean exactly.

When `tau` is zero
------------------
If the raw means differ no more than sampling noise explains, `tau^2` floors at
zero and every candidate collapses onto the theme mean. That is the correct
answer for such a family, not a defect: it says the experiment found no evidence
that any member differs from the others, and the tool reports it as the verdict
rather than as a very tight interval.

The archetype prior
-------------------
`prior_mean` and `prior_sd` replace the family's own estimate when supplied. The
intended source is research memory (a prior for this archetype from past
experiments), and the result states `prior_source: supplied` so a number is never
read as the family's when it was not.

What this is not
----------------
It is not a significance test and it spends no trial: it reads returns the platform
already recorded. It does not know *why* the candidates were tried, so a family
assembled by cherry-picking is shrunk toward a cherry-picked mean.
"""

from statistics import NormalDist

CREDIBLE = 0.95


def run(payload: dict) -> dict:
    frame = load_family(payload, "family")
    if "run_id" not in frame.columns:
        return refuse("the materialised family has no `run_id` column")

    index = "ts" if "ts" in frame.columns else "period"
    wide = frame.pivot_table(index=index, columns="run_id", values="ret", aggfunc="first")
    min_periods = int(payload.get("min_periods") or 60)
    counts = wide.notna().sum(axis=0)
    excluded = [str(c) for c in counts.index[counts < min_periods]]
    wide = wide.drop(columns=excluded)
    if wide.shape[1] < 3:
        return refuse(
            f"fewer than three strategies have {min_periods} periods; shrinkage toward a family "
            f"needs a family, and with two members the between-candidate spread is not estimable",
            excluded=excluded,
        )

    names = [str(c) for c in wide.columns]
    means, ses, ns = [], [], []
    for c in wide.columns:
        x = wide[c].dropna().to_numpy(dtype=float)
        means.append(float(np.mean(x)))
        ses.append(newey_west_se(x))
        ns.append(int(x.size))
    means = np.array(means)
    ses = np.array(ses)
    if not np.isfinite(ses).all() or (ses <= 0).any():
        return refuse("a member's standard error is not positive and finite, so it cannot be shrunk")

    supplied = payload.get("prior_mean") is not None
    if supplied:
        mu0 = float(payload["prior_mean"])
        prior_sd = payload.get("prior_sd")
        if prior_sd is None:
            return refuse("`prior_mean` needs `prior_sd`: a prior with no spread is a point mass")
        tau2 = float(prior_sd) ** 2
        var_mu0 = 0.0
        prior_source = "supplied"
    else:
        # Method-of-moments between-candidate variance, then precision weights.
        tau2 = max(float(np.var(means, ddof=1) - np.mean(ses**2)), 0.0)
        w = 1.0 / (ses**2 + tau2)
        mu0 = float(np.sum(w * means) / np.sum(w))
        var_mu0 = float(1.0 / np.sum(w))
        prior_source = "family"

    B = ses**2 / (ses**2 + tau2)
    post_mean = B * mu0 + (1.0 - B) * means
    post_var = (1.0 - B) * ses**2 + B**2 * var_mu0
    z = NormalDist().inv_cdf(0.5 + CREDIBLE / 2.0)
    half = z * np.sqrt(post_var)

    candidate = payload.get("candidate")
    if candidate is not None:
        candidate = str(candidate)
        if candidate not in names:
            return refuse(f"`{candidate}` is not a member with enough periods", members=names)
        selected_by = "requested"
        i = names.index(candidate)
    else:
        i = int(np.argmax(means))
        candidate = names[i]
        selected_by = "highest raw mean (the winner's curse applies: this is the member selection favoured)"

    lo, hi = float(post_mean[i] - half[i]), float(post_mean[i] + half[i])
    excludes = bool(lo > 0 or hi < 0)
    if tau2 == 0.0 and not supplied:
        verdict = (
            "no between-candidate spread beyond noise: this family gives no evidence that any "
            "member differs, so the candidate is shrunk onto the theme mean"
        )
    elif excludes and lo > 0:
        verdict = "the credible interval excludes zero on the positive side"
    elif excludes:
        verdict = "the credible interval excludes zero on the negative side"
    else:
        verdict = "the credible interval includes zero"

    members = [
        {
            "run_id": names[j],
            "n": ns[j],
            "raw_mean": float(means[j]),
            "raw_se": float(ses[j]),
            "shrinkage": float(B[j]),
            "post_mean": float(post_mean[j]),
            "ci_lo": float(post_mean[j] - half[j]),
            "ci_hi": float(post_mean[j] + half[j]),
        }
        for j in range(len(names))
    ]
    detail = {
        "experiment_id": payload.get("experiment_id"),
        "n_candidates": len(names),
        "excluded_strategies": excluded,
        "prior_source": prior_source,
        "prior_mean": mu0,
        "between_sd": float(np.sqrt(tau2)),
        "credible_level": CREDIBLE,
        "candidate": candidate,
        "selected_by": selected_by,
        "raw_mean": float(means[i]),
        "shrinkage": float(B[i]),
        "post_mean": float(post_mean[i]),
        "ci_lo": lo,
        "ci_hi": hi,
        "excludes_zero": excludes,
        "verdict": verdict,
        "members": members,
        "warnings": [
            "returns are per period; the interval is for the candidate's true mean per period",
            "this reads recorded returns and spends no trial; it cannot know why the family was assembled",
        ],
    }
    return result(detail, n=len(names))
