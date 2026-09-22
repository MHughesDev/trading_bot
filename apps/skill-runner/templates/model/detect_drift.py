"""Have the inputs moved since the model was fitted, and which ones.

```json
{
  "name": "detect_drift",
  "version": 1,
  "namespace": "model",
  "description": "Population Stability Index and a two-sample KS test per market feature, plus one joint MMD permutation test over the whole feature vector, between an earlier reference window and the recent part of the same window. Use it to gate live inference when the inputs have moved; it measures input drift only and reports no performance impact, because this platform stores no realised prediction outcome to score against.",
  "input_schema": {
    "type": "object",
    "required": ["instrument_id", "timeframe", "start", "end", "seed"],
    "additionalProperties": false,
    "properties": {
      "instrument_id": {"type": "string"},
      "timeframe": {"type": "string"},
      "start": {"type": "string"},
      "end": {"type": "string"},
      "seed": {"type": "integer"},
      "method": {"type": "string", "enum": ["psi", "ks", "mmd"], "default": "psi"},
      "features": {"type": "array", "items": {"type": "string"},
                   "description": "subset of the six bar features; omit for all"},
      "reference_fraction": {"type": "number", "minimum": 0.1, "maximum": 0.9,
                             "default": 0.5,
                             "description": "share of the window that is the reference"}
    }
  },
  "summary_template": "{method}: {alert}. Worst feature {worst_feature} (PSI {worst_psi}, KS p {worst_ks_p_adjusted}); joint MMD p {mmd_p_value} over {n_features} features; performance impact {performance_impact}",
  "detail_fields": [
    ["method", ""],
    ["alert", ""],
    ["worst_feature", ""],
    ["worst_psi", ""],
    ["worst_ks_p_adjusted", ""],
    ["mmd_p_value", ""],
    ["n_features", "features"],
    ["performance_impact", ""]
  ],
  "wall_seconds": 120,
  "series": [{"role": "primary"}]
}
```

What this tool does not do, and why it says so in its own output
----------------------------------------------------------------
The specification for this tool asks for an estimated performance impact beside
the drift statistics. It is not computed and the field is null, because the
label it would need does not exist: `trading.model_predictions` holds a
prediction's direction, magnitude, confidence and horizon and **no realised
outcome**, and at the time of writing it holds no rows at all. A calibration or
performance number computed against absent labels is the most dangerous thing
this tier could ship - it has a citation behind it, and nothing downstream can
tell that it was not measured. So `est_perf_impact` is null, `performance_impact`
reads "not measurable", and the reason travels with the result rather than
living in a comment here.

For the same reason this is drift in the *inputs*, not drift in a model's
predictions. It answers "have the features this instrument presents moved away
from the ones a model was fitted on", which is a question the bars can answer on
their own. It cannot answer "has the model got worse", and no rearrangement of
the data this platform currently stores would let it.

One window, split, rather than two windows
-------------------------------------------
The reference and the live sample are both cut from the *same* materialised
window. That is not a convenience: it is what keeps the comparison point-in-time
on both sides. The harness clips one window by the project's `end` and `as_of`
before the file is written, so a split of that file inherits the clip. Two
separately fetched windows would be two separate chances to fetch a reference
that had been restated after the live half's knowledge time, and the resulting
drift statistic would be measuring the restatement.

The split is by position within the window, and the reference is always the
earlier part. There is no argument that can make it the later part.

Three statistics, because the first two are blind to the same thing
--------------------------------------------------------------------
* **PSI** bins the live sample against the reference's own deciles and sums the
  weighted log ratio. It is the industry's stability number and its 0.10/0.25
  thresholds are a convention from credit scorecards (Siddiqi, *Credit Risk
  Scorecards*, 2006), not a distribution with a p-value. It is reported because
  people read it, and its thresholds are named in `detail` so they can be
  argued with.
* **KS** is a test rather than a convention, and on tens of thousands of bars it
  rejects equality for shifts far too small to matter. Its p-values are
  Benjamini-Hochberg adjusted across the features, because six tests reported
  raw is six chances to find a drift that is not there.
* **MMD** is computed **once, over the whole standardised feature vector**, not
  per feature. That is the reason it is here at all. PSI and KS each see one
  margin: a model can break because the *relationship* between volatility and
  range changed while every individual margin stayed exactly where it was, and
  both of them would report nothing. A kernel two-sample test on the joint
  vector sees it (Gretton et al., "A Kernel Two-Sample Test", JMLR 13, 2012).

  That is not a hoped-for property. Shuffling one feature's values inside the
  live half - which leaves its marginal distribution bit-identical, so PSI and
  KS cannot see it by construction - takes the joint p from 0.69 to 0.01 on the
  reference fixture while every PSI stays in the "stable" band.

  `mmd_squared` is the **unbiased** estimator, and an unbiased estimator of a
  quantity that is zero under the null takes small negative values about half
  the time. A negative `mmd_squared` is the null, not a bug; read the p-value.

An absent measurement is not a zero
------------------------------------
A PSI of 0 computed over an empty reference bin is not stability, it is an
arithmetic accident. Bins whose reference or live share is zero are floored at
half an observation, the number of floored bins is reported, and a feature whose
reference is degenerate is dropped by name rather than scored. If every feature
is dropped, or either side is too small to bin, the tool refuses.
"""

# The six features, in the order they are reported. Derived from open, high,
# low, close and volume only - the intersection of what the harness materialises
# (which also carries trade_count and quality_flags) and what the reference
# fixture contains. A feature reading a column only one of them has would be a
# tool that works in the test suite and not in production, or the reverse.
_FEATURES = ("log_return", "abs_return", "realised_vol", "range_pct", "gap_pct", "log_volume")

# Trailing bars for the realised-volatility feature. Bars, not hours: at a 1m
# timeframe this is twenty-four minutes, and the window is reported in `detail`
# so the number is never read as a duration it is not.
_VOL_BARS = 24

# PSI's conventional bands (Siddiqi 2006). Stated here rather than buried in an
# `if` so a reader can disagree with them. They are a convention from credit
# scorecards and carry no significance level; the KS and MMD p-values are the
# tests.
_PSI_MODERATE = 0.10
_PSI_SIGNIFICANT = 0.25

_ALPHA = 0.05

# Deciles. More bins on a fat-tailed return series puts single observations in
# the extreme bins, where the log ratio is dominated by the flooring rather than
# by the data.
_BINS = 10

# Fewest observations per side. Below this a decile holds under twenty-five
# observations and PSI is mostly sampling noise.
_MIN_PER_SIDE = 250

# The joint MMD is O(n^2) in the pooled sample and is permuted, so both sides
# are subsampled to this. Four hundred a side is a 800x800 Gram matrix, which is
# six megabytes and well inside the runner's half gigabyte.
_MMD_MAX = 400
_MMD_PERMUTATIONS = 199


def _features(frame, names):
    """The feature frame, aligned to the bars that have a previous bar.

    Every feature is causal: it reads the current bar and, for the two that need
    it, the one before. Nothing here reads forward, which matters because the
    reference half of the split would otherwise be told about the live half
    through a centred window.
    """
    close = pd.to_numeric(frame["close"], errors="coerce").to_numpy(dtype=float)
    high = pd.to_numeric(frame["high"], errors="coerce").to_numpy(dtype=float)
    low = pd.to_numeric(frame["low"], errors="coerce").to_numpy(dtype=float)
    open_ = pd.to_numeric(frame["open"], errors="coerce").to_numpy(dtype=float)
    volume = pd.to_numeric(frame["volume"], errors="coerce").to_numpy(dtype=float)

    # A non-positive close is a data defect rather than a price, and its log is
    # -inf. Marked non-finite here so the row is dropped once, below, with every
    # other non-finite row, instead of poisoning one feature silently.
    with np.errstate(divide="ignore", invalid="ignore"):
        log_close = np.where(close > 0, np.log(np.where(close > 0, close, 1.0)), np.nan)

    log_return = np.diff(log_close)
    columns = {
        "log_return": log_return,
        "abs_return": np.abs(log_return),
        "realised_vol": pd.Series(log_return).rolling(_VOL_BARS).std(ddof=1).to_numpy(),
        "range_pct": np.where(close[1:] > 0, (high[1:] - low[1:]) / close[1:], np.nan),
        "gap_pct": np.where(close[:-1] > 0, (open_[1:] - close[:-1]) / close[:-1], np.nan),
        "log_volume": np.log1p(np.where(volume[1:] >= 0, volume[1:], np.nan)),
    }
    picked = {n: columns[n] for n in names}
    out = pd.DataFrame(picked, index=frame.index[1:])
    return out[np.isfinite(out.to_numpy(dtype=float)).all(axis=1)]


def _psi(reference, live):
    """PSI of `live` against `reference`'s own deciles.

    Returns `(psi, floored_bins, usable)`. `usable` is False when the reference
    has too few distinct values to bin, which is a refusal for that feature and
    not a PSI of zero.
    """
    edges = np.quantile(reference, np.linspace(0.0, 1.0, _BINS + 1))
    edges = np.unique(edges)
    if edges.size < 3:
        return float("nan"), 0, False
    # Open at both ends: a live observation beyond anything in the reference is
    # the strongest evidence of drift there is, and a histogram with closed ends
    # would drop it.
    edges = np.concatenate([[-np.inf], edges[1:-1], [np.inf]])

    count_reference, _ = np.histogram(reference, bins=edges)
    count_live, _ = np.histogram(live, bins=edges)
    n_reference = float(reference.size)
    n_live = float(live.size)

    # Half an observation, the standard continuity correction. Counted, because
    # a PSI dominated by floored bins is a PSI computed mostly from the
    # correction rather than from the data.
    floor_reference = 0.5 / n_reference
    floor_live = 0.5 / n_live
    share_reference = np.maximum(count_reference / n_reference, floor_reference)
    share_live = np.maximum(count_live / n_live, floor_live)
    floored = int(np.sum((count_reference == 0) | (count_live == 0)))

    psi = float(np.sum((share_live - share_reference) * np.log(share_live / share_reference)))
    return psi, floored, True


def _ks(a, b):
    """Two-sample KS statistic and its asymptotic p-value.

    Kolmogorov's series with the Stephens small-sample correction to the
    argument, as given in Press et al., *Numerical Recipes* 3rd ed. §14.3.3.
    Written out rather than taken from scipy for the reason
    `apps/skill-runner/requirements.txt` gives: a p-value approximation that
    moves between minor releases moves a stored number nobody edited.
    """
    combined = np.sort(np.concatenate([a, b]))
    cdf_a = np.searchsorted(np.sort(a), combined, side="right") / a.size
    cdf_b = np.searchsorted(np.sort(b), combined, side="right") / b.size
    statistic = float(np.max(np.abs(cdf_a - cdf_b)))
    effective = math.sqrt(a.size * b.size / (a.size + b.size))
    lam = (effective + 0.12 + 0.11 / effective) * statistic
    total = 0.0
    for k in range(1, 101):
        total += ((-1.0) ** (k - 1)) * math.exp(-2.0 * (k**2) * (lam**2))
    return statistic, max(0.0, min(1.0, 2.0 * total))


def _stride(n, cap):
    """`cap` indices spread evenly over `n`, deterministically.

    Evenly rather than at random, and spread rather than the first `cap`: taking
    the head of a window samples only its beginning, which for a drift test
    would mean comparing the start of the reference against the start of the
    live half and calling the rest unmeasured.
    """
    if n <= cap:
        return np.arange(n)
    return np.unique(np.round(np.linspace(0, n - 1, cap)).astype(int))


def _gram(x):
    """Gaussian RBF Gram matrix with the median heuristic bandwidth.

    The bandwidth is the median pairwise squared distance over the *pooled*
    sample, which is the standard choice and is what makes the statistic scale
    free. Distances are formed by broadcasting rather than by a Gram product:
    a BLAS reduction sums in a thread-dependent order, and the preamble's
    single-thread setting is a guard rather than a guarantee.
    """
    diff = x[:, None, :] - x[None, :, :]
    sq = np.sum(diff * diff, axis=-1)
    upper = sq[np.triu_indices_from(sq, k=1)]
    median = float(np.median(upper))
    # A degenerate pooled sample - every row identical - has a zero median and
    # no scale. Falling back to 1.0 keeps the kernel defined; the MMD is then
    # zero because the samples really are identical.
    bandwidth = median if median > 0 else 1.0
    return np.exp(-sq / bandwidth), bandwidth


def _mmd_from_gram(gram, idx_a, idx_b):
    """Unbiased MMD^2 (Gretton et al. 2012, eq. 3) from a precomputed Gram."""
    n = idx_a.size
    m = idx_b.size
    kaa = gram[np.ix_(idx_a, idx_a)]
    kbb = gram[np.ix_(idx_b, idx_b)]
    kab = gram[np.ix_(idx_a, idx_b)]
    term_a = (float(kaa.sum()) - float(np.trace(kaa))) / (n * (n - 1))
    term_b = (float(kbb.sum()) - float(np.trace(kbb))) / (m * (m - 1))
    return term_a + term_b - 2.0 * float(kab.mean())


def run(payload: dict) -> dict:
    frame = load(payload, "primary")
    for column in ("open", "high", "low", "close", "volume"):
        if column not in frame.columns:
            return refuse(
                f"the materialised bars have no `{column}` column, and every feature this tool "
                f"compares is built from open, high, low, close and volume",
                columns=sorted(str(c) for c in frame.columns),
            )

    requested = payload.get("features")
    if requested:
        names = [n for n in _FEATURES if n in set(requested)]
        unknown = sorted(set(requested) - set(_FEATURES))
        if unknown:
            return refuse(
                f"this tool compares a fixed set of bar features and does not know {unknown}. "
                f"It reads bars, not a model's feature store, so it cannot compare a feature "
                f"the bars do not contain.",
                known_features=list(_FEATURES),
            )
    else:
        names = list(_FEATURES)
        unknown = []
    if not names:
        return refuse("no features were selected", known_features=list(_FEATURES))

    table = _features(frame, names)
    total = int(table.shape[0])
    if total < 2 * _MIN_PER_SIDE:
        return refuse(
            f"a drift comparison needs at least {_MIN_PER_SIDE} usable observations on each "
            f"side and this window yields {total} in total. A statistic computed over a window "
            f"too short to bin is not a small drift, it is no measurement.",
            n_usable=total,
            minimum_per_side=_MIN_PER_SIDE,
            vol_window_bars=_VOL_BARS,
        )

    fraction = payload.get("reference_fraction")
    fraction = 0.5 if fraction is None else float(fraction)
    if not 0.1 <= fraction <= 0.9:
        return refuse(
            f"reference_fraction must be between 0.1 and 0.9; got {fraction}",
            reference_fraction=fraction,
        )
    cut = int(total * fraction)
    cut = max(_MIN_PER_SIDE, min(cut, total - _MIN_PER_SIDE))

    method = str(payload.get("method") or "psi")
    if method not in ("psi", "ks", "mmd"):
        return refuse(f"{method!r} is not one of psi, ks, mmd", method=method)

    reference_rows = table.iloc[:cut]
    live_rows = table.iloc[cut:]
    n_reference = int(reference_rows.shape[0])
    n_live = int(live_rows.shape[0])

    warnings = []
    by_feature = []
    dropped = []
    raw_p = []
    scored = []
    for name in names:
        reference = reference_rows[name].to_numpy(dtype=float)
        live = live_rows[name].to_numpy(dtype=float)
        psi, floored, usable = _psi(reference, live)
        if not usable:
            dropped.append(name)
            continue
        statistic, p = _ks(reference, live)
        scored.append(name)
        raw_p.append(p)
        by_feature.append(
            {
                "feature": name,
                "psi": psi,
                "psi_band": (
                    "stable"
                    if psi < _PSI_MODERATE
                    else ("moderate" if psi < _PSI_SIGNIFICANT else "significant")
                ),
                "floored_bins": floored,
                "ks_statistic": statistic,
                "ks_p_value": p,
                "reference_mean": float(np.mean(reference)),
                "live_mean": float(np.mean(live)),
                "reference_sd": float(np.std(reference, ddof=1)),
                "live_sd": float(np.std(live, ddof=1)),
            }
        )
        if floored:
            warnings.append(
                f"`{name}` has {floored} bin(s) empty on one side, floored at half an "
                f"observation. Its PSI is partly the correction rather than the data."
            )

    if dropped:
        warnings.append(
            f"dropped {dropped}: the reference half has too few distinct values to bin, which "
            f"is a refusal for those features and not a PSI of zero."
        )
    if not by_feature:
        return refuse(
            "every selected feature has a degenerate reference distribution, so there is "
            "nothing to compare against. This is an absent measurement, not an absence of drift.",
            dropped_features=dropped,
            n_reference=n_reference,
            n_live=n_live,
        )

    adjusted, rejected = benjamini_hochberg(np.asarray(raw_p, dtype=float), _ALPHA)
    for row, adj, rej in zip(by_feature, adjusted.tolist(), rejected.tolist()):
        row["ks_p_adjusted"] = float(adj)
        row["ks_rejects_equality"] = bool(rej)

    # ── The joint test ───────────────────────────────────────────────────────
    # Standardised by the *reference*'s own mean and scale, deliberately. Pooled
    # standardisation would subtract part of the very shift being tested for.
    joint_names = [row["feature"] for row in by_feature]
    reference_joint = reference_rows[joint_names].to_numpy(dtype=float)
    live_joint = live_rows[joint_names].to_numpy(dtype=float)
    centre = reference_joint.mean(axis=0)
    scale = reference_joint.std(axis=0, ddof=1)
    scale = np.where(scale > 0, scale, 1.0)
    reference_joint = (reference_joint - centre) / scale
    live_joint = (live_joint - centre) / scale

    take_reference = _stride(reference_joint.shape[0], _MMD_MAX)
    take_live = _stride(live_joint.shape[0], _MMD_MAX)
    sub_reference = reference_joint[take_reference]
    sub_live = live_joint[take_live]
    if take_reference.size < reference_joint.shape[0] or take_live.size < live_joint.shape[0]:
        warnings.append(
            f"the joint MMD is computed on {take_reference.size} and {take_live.size} rows, "
            f"evenly spread over each half, because the test is quadratic in the sample. The "
            f"per-feature PSI and KS use every observation."
        )

    pooled = np.vstack([sub_reference, sub_live])
    gram, bandwidth = _gram(pooled)
    index_reference = np.arange(sub_reference.shape[0])
    index_live = np.arange(sub_reference.shape[0], pooled.shape[0])
    mmd = _mmd_from_gram(gram, index_reference, index_live)

    generator = rng(payload)
    size = pooled.shape[0]
    split = index_reference.size
    exceed = 0
    for _ in range(_MMD_PERMUTATIONS):
        order = generator.permutation(size)
        exceed += int(_mmd_from_gram(gram, order[:split], order[split:]) >= mmd)
    # The +1 on both sides is the standard finite-permutation p-value: a p of
    # exactly zero would claim more evidence than 199 permutations can carry.
    mmd_p = float((exceed + 1) / (_MMD_PERMUTATIONS + 1))

    # ── The verdict ──────────────────────────────────────────────────────────
    worst = max(by_feature, key=lambda r: r["psi"])
    worst_ks = min(by_feature, key=lambda r: r["ks_p_adjusted"])
    any_ks = any(row["ks_rejects_equality"] for row in by_feature)

    if method == "psi":
        if worst["psi"] >= _PSI_SIGNIFICANT:
            alert = f"significant input drift in `{worst['feature']}`"
        elif worst["psi"] >= _PSI_MODERATE:
            alert = f"moderate input drift in `{worst['feature']}`"
        else:
            alert = "the inputs are stable by PSI"
    elif method == "ks":
        alert = (
            f"KS rejects equality for {sum(r['ks_rejects_equality'] for r in by_feature)} of "
            f"{len(by_feature)} features after correction"
            if any_ks
            else "KS rejects no feature after correction"
        )
    else:
        alert = (
            f"the joint distribution has moved (MMD p={mmd_p:.3g})"
            if mmd_p < _ALPHA
            else f"the joint distribution has not detectably moved (MMD p={mmd_p:.3g})"
        )

    if mmd_p < _ALPHA and worst["psi"] < _PSI_MODERATE:
        warnings.append(
            "every feature is stable on its own and the joint test rejects, which means the "
            "relationship between them has changed while the margins have not. A model reading "
            "more than one of these features is affected and no per-feature number shows it."
        )
    if any_ks and worst["psi"] < _PSI_MODERATE:
        warnings.append(
            f"KS rejects on {n_reference} against {n_live} observations while every PSI is below "
            f"{_PSI_MODERATE}. At this sample size KS rejects shifts far too small to retrade on; "
            f"read the PSI and the mean/sd pairs as the effect size."
        )

    # Hoisted rather than written inline in the dict: `and` binds tighter than
    # `or`, so the inline form is correct and reads as though it might not be,
    # and this is the field a gate branches on.
    alerting = {
        "psi": worst["psi"] >= _PSI_MODERATE,
        "ks": any_ks,
        "mmd": mmd_p < _ALPHA,
    }[method]

    detail = {
        "method": method,
        "alert": alert,
        "alerting": alerting,
        "n_features": len(by_feature),
        "features_scored": scored,
        "features_dropped": dropped,
        "features_unknown": unknown,
        "worst_feature": worst["feature"],
        "worst_psi": worst["psi"],
        "worst_ks_feature": worst_ks["feature"],
        "worst_ks_p_adjusted": worst_ks["ks_p_adjusted"],
        "by_feature": by_feature,
        "mmd_squared": mmd,
        "mmd_p_value": mmd_p,
        "mmd_permutations": _MMD_PERMUTATIONS,
        "mmd_bandwidth": bandwidth,
        "mmd_rows": {"reference": int(take_reference.size), "live": int(take_live.size)},
        "split": {
            "reference_start": str(reference_rows.index[0]),
            "reference_end": str(reference_rows.index[-1]),
            "live_start": str(live_rows.index[0]),
            "live_end": str(live_rows.index[-1]),
            "reference_fraction": fraction,
            "n_reference": n_reference,
            "n_live": n_live,
        },
        # The refusal, in the result, where a caller reading `detail` cannot
        # miss it. `est_perf_impact` is the field the specification asks for and
        # it is null on purpose: see the module docstring.
        "est_perf_impact": None,
        "performance_impact": "not measurable",
        "performance_impact_reason": (
            "no realised prediction outcome is stored: trading.model_predictions carries "
            "direction, magnitude, confidence and horizon and no label, so there is nothing to "
            "score a prediction against. A performance impact computed here would be a number "
            "this tool did not measure."
        ),
        "thresholds": {
            "psi_moderate": _PSI_MODERATE,
            "psi_significant": _PSI_SIGNIFICANT,
            "psi_source": "Siddiqi, Credit Risk Scorecards (2006) - a convention, not a test",
            "alpha": _ALPHA,
            "ks_correction": "benjamini_hochberg",
        },
        "point_in_time": (
            "both halves are cut from one materialised window, so the project's end and as_of "
            "clip applies to the reference and the live sample identically; the reference is "
            "always the earlier half"
        ),
        "vol_window_bars": _VOL_BARS,
        "bins": _BINS,
        "warnings": warnings,
        "instrument": {
            "instrument_id": payload.get("instrument_id"),
            "timeframe": payload.get("timeframe"),
            "start": payload.get("start"),
            "end": payload.get("end"),
        },
    }
    return result(detail, n=min(n_reference, n_live))
