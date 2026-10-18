"""N-4.1 ACI calibration with CUSUM resets."""
from __future__ import annotations

import pathlib
import sys

import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
try:  # app.main imports the worker, which imports nats at load
    import nats  # noqa: F401
except ModuleNotFoundError:
    import types

    _stub = types.ModuleType("nats")

    async def _no_connect(*a, **k):
        raise ConnectionError("nats is not installed in this environment")

    _stub.connect = _no_connect
    sys.modules["nats"] = _stub

from app import calibration as cal  # noqa: E402
from app import failures  # noqa: E402

SHIFT = 600
N = 1400
MAX_RESET_LAG = 30  # bars after the shift by which the reset must fire


def shift_series(seed=0, sigma=(1.0, 3.0), shift=SHIFT, n=N):
    rng = np.random.default_rng(seed)
    s = np.where(np.arange(n) < shift, sigma[0], sigma[1])
    y = rng.normal(0, 1, n) * s
    return np.zeros(n), y


def test_defaults_match_the_platform_monitor():
    assert (cal.DEFAULT_K, cal.DEFAULT_H) == (0.05, 4.0)


def test_rolling_coverage_stays_closer_to_nominal_than_static_split_after_a_regime_shift():
    point, y = shift_series()
    kw = dict(n_cal=100, window=200, rolling=100)
    split = cal.calibrate(point, y, method="split", **kw)
    aci = cal.calibrate(point, y, method="aci", **kw)
    acc = cal.calibrate(point, y, method="aci_cusum", **kw)
    g_split = cal.mean_abs_coverage_gap(split, start=SHIFT)
    g_aci = cal.mean_abs_coverage_gap(aci, start=SHIFT)
    g_acc = cal.mean_abs_coverage_gap(acc, start=SHIFT)
    assert g_acc < 0.5 * g_split, (g_acc, g_split)
    assert g_aci < g_split
    assert g_acc <= g_aci + 1e-9, (g_acc, g_aci)
    # static split is badly under-covering just after the shift; the wrapper has recovered by the end
    assert split["rolling_coverage"][SHIFT + 99] < 0.7
    assert abs(acc["rolling_coverage"][-1] - 0.9) < 0.05


def test_a_reset_fires_within_the_stated_lag_of_the_shift_and_not_before():
    point, y = shift_series()
    r = cal.calibrate(point, y, method="aci_cusum")
    first_after = [i for i in r["reset_indices"] if i >= SHIFT]
    assert first_after, "no reset after the shift"
    assert first_after[0] - SHIFT <= MAX_RESET_LAG, first_after[0] - SHIFT
    assert not [i for i in r["reset_indices"] if i < SHIFT], "a reset fired in the calm regime"


def test_a_downward_shift_resets_too_and_the_other_methods_never_reset():
    point, y = shift_series(sigma=(3.0, 1.0))
    r = cal.calibrate(point, y, method="aci_cusum")
    assert [i for i in r["reset_indices"] if SHIFT <= i <= SHIFT + 120]  # over-coverage is a weaker signal than misses: slower
    assert cal.calibrate(point, y, method="aci")["reset_indices"] == []
    assert cal.calibrate(point, y, method="split")["reset_indices"] == []


def test_stationary_series_has_nominal_coverage_and_no_resets():
    point, y = shift_series(seed=2, sigma=(1.0, 1.0))
    r = cal.calibrate(point, y, method="aci_cusum")
    assert abs(r["coverage"] - 0.9) < 0.03
    assert len(r["reset_indices"]) <= 1  # at the CUSUM constants a rare false alarm is allowed


def test_a_reset_restores_alpha_t_and_cuts_the_window_to_the_change_point():
    point, y = shift_series()
    r = cal.calibrate(point, y, method="aci_cusum")
    t0 = [i for i in r["reset_indices"] if i >= SHIFT][0]
    assert r["alpha_t"][t0 + 1] == pytest.approx(0.10)
    # the band after the reset is wider than the one just before it: the stale calm scores are gone
    assert r["qhat"][t0 + 1] > 1.5 * r["qhat"][SHIFT - 1]


def test_quantile_band_inputs_use_the_cqr_score():
    rng = np.random.default_rng(1)
    n = 500
    y = rng.normal(0, 1, n)
    lower, upper = np.full(n, -1.0), np.full(n, 1.0)  # a model's own ~68% band
    r = cal.calibrate(np.zeros(n), y, lower=lower, upper=upper, method="aci", n_cal=100)
    assert abs(r["coverage"] - 0.9) < 0.04
    widths = [w for w in r["spread_90"] if w is not None]
    assert min(widths) > 2.0  # widened beyond the model's own band to reach 90%


def test_output_shape_and_the_calibration_window_has_no_band():
    point, y = shift_series(n=300, shift=200)
    r = cal.calibrate(point, y, n_cal=50, method="aci_cusum")
    assert len(r["lower"]) == len(r["upper"]) == len(r["rolling_coverage"]) == len(r["spread_90"]) == 300
    assert r["lower"][49] is None and r["lower"][50] is not None
    assert r["rolling_coverage"][49] is None
    w = r["upper"][100] - r["lower"][100]
    assert r["spread_90"][100] == pytest.approx(w)
    import json
    json.dumps(r)  # JSON-clean: no NaN / inf


def test_bad_inputs_are_data_errors():
    point, y = shift_series(n=300)
    for kw in ({"method": "magic"}, {"alpha": 0.0}, {"alpha": 1.0}, {"n_cal": 1}, {"n_cal": 400}):
        with pytest.raises(failures.DataError):
            cal.calibrate(point, y, **kw)
    with pytest.raises(failures.DataError):
        cal.calibrate(point[:-1], y)
    with pytest.raises(failures.DataError):
        cal.calibrate(point, y, lower=point)


def test_the_http_route_runs_calibrate():
    from app import main  # noqa: F401
    from fastapi.testclient import TestClient

    point, y = shift_series(n=500, shift=300)
    c = TestClient(main.app)
    res = c.post("/calibrate", json={"scores": point.tolist(), "realized": y.tolist(),
                                     "method": "aci_cusum", "gamma": 0.005})
    assert res.status_code == 200, res.text
    body = res.json()
    assert body["method"] == "aci_cusum" and body["reset_indices"] and len(body["rolling_coverage"]) == 500
    bad = c.post("/calibrate", json={"scores": [0.0], "realized": [0.0, 1.0]})
    assert bad.status_code == 422
