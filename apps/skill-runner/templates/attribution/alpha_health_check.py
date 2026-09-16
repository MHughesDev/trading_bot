"""Is this alpha new, or a re-skin of one the family already has.

```json
{
  "name": "alpha_health_check",
  "version": 1,
  "namespace": "attribution",
  "description": "Checks each strategy in an experiment against its siblings for redundancy, sign instability and dependence on a handful of periods. Run it before promoting anything out of a sweep.",
  "input_schema": {
    "type": "object",
    "required": ["experiment_id"],
    "additionalProperties": false,
    "properties": {
      "experiment_id": {"type": "string"},
      "candidate": {
        "type": "string",
        "description": "the run to focus on; omitted, every member is reported"
      },
      "min_overlap": {"type": "integer", "minimum": 20, "maximum": 5000, "default": 60}
    }
  },
  "summary_template": "{n_strategies} strategies; {n_redundant} are {redundancy_threshold}+ correlated with a sibling; {n_sign_unstable} change sign between halves; {n_concentrated} earn most of their return in under {concentration_pct} of periods; {verdict}",
  "detail_fields": [
    ["n_strategies", ""],
    ["n_redundant", ""],
    ["redundancy_threshold", ""],
    ["n_sign_unstable", ""],
    ["n_concentrated", ""],
    ["concentration_pct", "pct"],
    ["verdict", ""]
  ],
  "wall_seconds": 300,
  "series": [{"role": "family", "kind": "trial_returns"}]
}
```

Three ways an alpha is unhealthy, and none of them is a bad Sharpe
------------------------------------------------------------------
A strategy with a good Sharpe can still be worth nothing to a book:

* **Redundant.** It is 0.9 correlated with a sibling, so it adds a second copy
  of a bet already held. Its Sharpe is real and its contribution is nearly zero,
  and a portfolio that counts it as a separate position is levered into one idea
  it thinks is two.
* **Sign-unstable.** It made money in the first half and lost it in the second,
  or the reverse. A full-sample Sharpe averages those into something positive and
  says nothing about which half the future resembles.
* **Concentrated.** Ninety percent of its return came from three of five hundred
  periods. That is not an edge, it is an event — and the backtest cannot tell you
  whether the event repeats, only that it happened.

None of these is visible in a Sharpe ratio, which is why they are checked
separately rather than folded into a score. A single number here would let a
strategy that fails one badly pass by doing well on the other two.

Scope: siblings, not the whole book
------------------------------------
AGENT-006 §5.3 asks for self-correlation against "the ledger's own alpha pool" —
every alpha the platform holds. This checks against the **experiment's own
members** only, which is a narrower question: "is this variant distinct from the
other variants of this idea" rather than "is this idea distinct from everything
we have". The narrow version is the one the materialiser can answer today, it is
the one that catches the common case (a sweep promoting three points off one
plateau as three alphas), and the difference is stated here rather than implied
away. The pool-wide version needs a tenant-scoped materialisation that does not
exist yet.
"""

#: Above this, two strategies are the same bet with different parameters.
#:
#: 0.8 rather than 0.95: by the time two return series correlate at 0.95 they are
#: visibly identical, and the damage — a book that thinks it holds two positions
#: — is already done at 0.8. Fixed rather than an argument, because a caller who
#: can raise it can make a redundant family look diversified.
REDUNDANCY = 0.8

#: The share of periods that may carry most of a strategy's return before it is
#: an event rather than an edge.
CONCENTRATION_PERIODS = 0.05
CONCENTRATION_SHARE = 0.5


def _half_sharpe(series):
    sd = float(np.std(series, ddof=1)) if series.size > 1 else 0.0
    return float(np.mean(series) / sd) if sd > 0 else 0.0


def run(payload: dict) -> dict:
    frame = load_family(payload, "family")
    table = frame
    if "run_id" not in table.columns:
        return refuse("the materialised family has no `run_id` column")

    index = "ts" if "ts" in table.columns else "period"
    wide = table.pivot_table(index=index, columns="run_id", values="ret", aggfunc="first")
    min_overlap = int(payload.get("min_overlap") or 60)
    counts = wide.notna().sum(axis=0)
    excluded = [str(c) for c in counts.index[counts < min_overlap]]
    wide = wide.drop(columns=excluded)
    if wide.shape[1] < 2:
        return refuse(
            f"fewer than two strategies have {min_overlap} periods; redundancy is a question "
            f"about a family and this one has no siblings to compare against",
            excluded=excluded,
        )

    names = [str(c) for c in wide.columns]
    reports = []
    n_redundant = 0
    n_sign_unstable = 0
    n_concentrated = 0

    for name in names:
        own = wide[name]
        own_clean = own.dropna()
        if own_clean.size < min_overlap:
            continue
        series = own_clean.to_numpy(dtype=float)

        # Redundancy: the highest correlation with any sibling, computed on the
        # periods the pair actually share rather than on a padded union.
        best_name, best_corr = None, 0.0
        for other in names:
            if other == name:
                continue
            pair = wide[[name, other]].dropna()
            if len(pair) < min_overlap:
                continue
            a = pair[name].to_numpy(dtype=float)
            b = pair[other].to_numpy(dtype=float)
            if np.std(a) <= 0 or np.std(b) <= 0:
                continue
            c = float(np.corrcoef(a, b)[0, 1])
            if abs(c) > abs(best_corr):
                best_name, best_corr = other, c

        # Sign stability across halves. Halves, not a rolling window: a rolling
        # window has a length nobody chose and the choice moves the answer.
        midpoint = series.size // 2
        first, second = series[:midpoint], series[midpoint:]
        sharpe_first = _half_sharpe(first)
        sharpe_second = _half_sharpe(second)
        sign_flip = bool(sharpe_first * sharpe_second < 0)

        # Concentration: the share of the total carried by the top few periods.
        total = float(np.sum(series))
        top_k = max(1, int(math.ceil(CONCENTRATION_PERIODS * series.size)))
        if total > 0:
            top = float(np.sum(np.sort(series)[::-1][:top_k]))
            share = top / total
        else:
            share = None
        concentrated = bool(share is not None and share >= CONCENTRATION_SHARE)

        redundant = bool(best_corr is not None and abs(best_corr) >= REDUNDANCY)
        n_redundant += int(redundant)
        n_sign_unstable += int(sign_flip)
        n_concentrated += int(concentrated)

        problems = []
        if redundant:
            problems.append(f"redundant with {best_name} ({best_corr:.2f})")
        if sign_flip:
            problems.append(
                f"sign flips between halves ({sharpe_first:.2f} then {sharpe_second:.2f})"
            )
        if concentrated:
            problems.append(
                f"{share:.0%} of its return comes from its best {top_k} period(s)"
            )

        reports.append(
            {
                "run_id": name,
                "n_periods": int(series.size),
                "sharpe_per_period": _half_sharpe(series),
                "most_correlated_sibling": best_name,
                "max_correlation": best_corr,
                "redundant": redundant,
                "sharpe_first_half": sharpe_first,
                "sharpe_second_half": sharpe_second,
                "sign_flip": sign_flip,
                "top_period_share": share,
                "top_periods": top_k,
                "concentrated": concentrated,
                "healthy": not (redundant or sign_flip or concentrated),
                "problems": problems,
            }
        )

    if not reports:
        return refuse("no strategy had enough periods to check")

    candidate = payload.get("candidate")
    focus = next((r for r in reports if r["run_id"] == candidate), None) if candidate else None
    if candidate and focus is None:
        return refuse(
            f"no run named {candidate!r} is in this experiment's recorded family",
            members=[r["run_id"] for r in reports][:50],
        )

    healthy = [r["run_id"] for r in reports if r["healthy"]]
    if focus is not None:
        verdict = (
            f"{candidate} is healthy on all three checks"
            if focus["healthy"]
            else f"{candidate}: " + "; ".join(focus["problems"])
        )
    elif not healthy:
        verdict = (
            "no member passes all three checks — this family is one idea with parameters, not "
            "a set of alphas"
        )
    else:
        verdict = f"{len(healthy)} of {len(reports)} members pass all three checks"

    warnings = []
    if n_redundant >= len(reports) // 2 and len(reports) > 2:
        warnings.append(
            f"{n_redundant} of {len(reports)} members are {REDUNDANCY}+ correlated with a "
            f"sibling. Promoting several of these as separate alphas would lever one bet while "
            f"the book believes it is diversified."
        )
    if n_concentrated:
        warnings.append(
            f"{n_concentrated} member(s) earn most of their return in a handful of periods. "
            f"That is an event, not an edge, and a backtest cannot say whether it repeats."
        )
    if n_sign_unstable:
        warnings.append(
            f"{n_sign_unstable} member(s) change sign between halves. A full-sample Sharpe "
            f"averages the two and says nothing about which half the future resembles."
        )
    warnings.append(
        "redundancy here is against this experiment's own members only, not the platform's "
        "whole alpha pool (§5.3). A strategy can pass this and still duplicate something in "
        "another experiment."
    )

    detail = {
        "experiment_id": payload.get("experiment_id"),
        "n_strategies": len(reports),
        "excluded_strategies": excluded,
        "redundancy_threshold": REDUNDANCY,
        "concentration_pct": CONCENTRATION_PERIODS * 100.0,
        "concentration_share": CONCENTRATION_SHARE,
        "n_redundant": n_redundant,
        "n_sign_unstable": n_sign_unstable,
        "n_concentrated": n_concentrated,
        "n_healthy": len(healthy),
        "healthy": healthy,
        "candidate": candidate,
        "candidate_report": focus,
        "members": reports,
        "verdict": verdict,
        "warnings": warnings,
    }
    return result(detail, n=len(reports))
