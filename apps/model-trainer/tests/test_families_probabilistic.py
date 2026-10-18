"""N-2.7: probabilistic family and the stated coverage tolerance."""
import numpy as np
import pandas as pd

from _fam_common import noop_emit
from app.families import probabilistic as P


def hetero(n=3000, seed=7):
    rng = np.random.default_rng(seed)
    X = rng.uniform(-2, 2, (n, 3))
    sd = 0.2 + 0.8 * np.abs(X[:, 0])
    y = 0.5 * X[:, 1] + sd * rng.standard_normal(n)
    return X, y


X, Y = hetero()
TR, VA, TE = slice(0, 1800), slice(1800, 2400), slice(2400, 3000)


def test_ngboost_coverage_within_tolerance_and_adapts_sigma():
    m = P.GaussianNGB().fit(X[TR], Y[TR], X[VA], Y[VA])
    q = m.predict_quantiles(X[TE], [0.05, 0.95])
    assert abs(P.coverage(Y[TE], q[:, 0], q[:, 1]) - P.COVERAGE_TARGET) <= P.COVERAGE_TOLERANCE
    _, s = m.predict_dist(X[TE])
    hi, lo = s[np.abs(X[TE][:, 0]) > 1.5].mean(), s[np.abs(X[TE][:, 0]) < 0.3].mean()
    assert hi > 1.8 * lo  # the planted heteroscedasticity is learned


def test_qrf_coverage_within_tolerance():
    m = P.QRF().fit(X[TR], Y[TR])
    q = m.predict_quantiles(X[TE], [0.05, 0.95])
    assert abs(P.coverage(Y[TE], q[:, 0], q[:, 1]) - P.COVERAGE_TARGET) <= P.COVERAGE_TOLERANCE


def test_split_conformal_coverage_within_tolerance_and_finite_sample_correction():
    sc = P.SplitConformal(P._GBRBase().fit(X[TR], Y[TR])).fit_calibration(X[VA], Y[VA])
    lo, hi = sc.interval(X[TE], 0.10)
    assert abs(P.coverage(Y[TE], lo, hi) - P.COVERAGE_TARGET) <= P.COVERAGE_TOLERANCE
    assert sc.half_width(0.05) > sc.half_width(0.10) > sc.half_width(0.5)


def test_conformal_refuses_tiny_calibration_set():
    import pytest
    from app import failures

    with pytest.raises(failures.DataError):
        P.SplitConformal(P._GBRBase().fit(X[TR], Y[TR])).fit_calibration(X[:5], Y[:5])


def test_entrypoints_report_coverage_on_heldout_rows():
    df = pd.DataFrame(X, columns=["a", "b", "c"])
    df["label"] = Y
    d = {"output": {"quantile_levels": [0.05, 0.25, 0.5, 0.75, 0.95]}, "hyperparameters": {"n_estimators": 80}}
    for fn in (P.train_ngboost, P.train_qrf, P.train_conformal):
        art, m = fn(d, df, noop_emit)
        assert art[:8] == b"TBNDL001"
        assert abs(m["coverage_90"] - P.COVERAGE_TARGET) <= P.COVERAGE_TOLERANCE + 0.03, (fn.__name__, m["coverage_90"])
