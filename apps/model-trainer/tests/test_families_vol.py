"""N-2.3 vol family and N-4.2 conditional-sigma serving."""
import pathlib
import pickle

import numpy as np
import pandas as pd
import pytest

from _fam_common import noop_emit, sim_garch
from app import engine
from app.families import vol

R = sim_garch(2500, g=0.06, seed=11)  # clustered, leverage effect


@pytest.mark.parametrize("kind", ["garch", "gjr", "egarch"])
def test_fits_and_one_step_sigma_differs_from_unconditional(kind):
    st = vol.fit(kind, R)
    assert st["converged"] or st["loglik"] < 0
    sp = vol.sigma_path(st, R)
    ok = sp[np.isfinite(sp)]
    assert ok.std() / ok.mean() > 0.15  # genuinely time-varying
    # the last conditional sigma differs from the unconditional one
    assert abs(sp[-1] - st["sigma_uncond"]) / st["sigma_uncond"] > 1e-3
    p = st["params"]
    if kind in ("garch", "gjr"):
        assert 0 < p["alpha"] + p["beta"] < 1
    if kind == "gjr":
        assert p["gamma"] > 0.0  # leverage was planted
    assert 2.1 < p["nu"] < 100


def test_gjr_beats_garch_on_leverage_data_by_likelihood():
    assert vol.fit("gjr", R)["loglik"] > vol.fit("garch", R)["loglik"] + 2.0


def test_realized_garch_fits_with_realised_measure():
    rng = np.random.default_rng(2)
    st0 = vol.fit("garch", R)
    sp = np.nan_to_num(vol.sigma_path(st0, R), nan=R.std())
    xr = (sp**2) * np.exp(0.2 * rng.standard_normal(len(R)))
    st = vol.fit("realized_garch", R, xr)
    assert 0 < st["params"]["beta"] < 1 and np.isfinite(st["loglik"])
    s = vol.conditional_sigma(st, R[-60:], xr[-60:])
    assert s > 0
    with pytest.raises(vol.Abstain):
        vol.conditional_sigma(st, R[-60:], None)  # needs its realised measure


def test_serving_sigma_rises_after_a_spike_and_exceeds_unconditional():
    st = vol.fit("garch", R)
    calm = np.full(60, 0.0001) * np.where(np.arange(60) % 2, 1, -1)
    spike = np.concatenate([calm, [0.06, -0.05, 0.07]])
    s_calm = vol.conditional_sigma(st, calm)
    s_spike = vol.conditional_sigma(st, spike)
    assert s_spike > st["sigma_uncond"] > s_calm
    assert s_spike > 2 * s_calm


def test_no_bars_abstains_with_a_reason_never_a_value():
    st = vol.fit("garch", R)
    for bad in (None, [], np.array([0.01] * 5)):
        with pytest.raises(vol.Abstain) as ei:
            vol.conditional_sigma(st, bad)
        assert ei.value.reason
        out = vol.serve_from_state(st, bad)
        assert out["abstain"] is True and "sigma" not in out and out["reason"]
    with pytest.raises(vol.Abstain):
        vol.conditional_sigma(st, np.array([0.01] * 40 + [np.nan]))
    assert vol.serve_from_state(st, R[-60:])["abstain"] is False


def test_no_code_path_returns_the_unconditional_value():
    """Grep-style guard. No serving function reads sigma_uncond; the adapter and the trainer's
    eval path do not mention it at all; in vol.py it appears only as the diagnostic written at fit."""
    import inspect

    for fn in (vol.conditional_sigma, vol.serve_from_state, vol.sigma_path, vol.quantiles,
               vol.variance_path):
        assert "uncond" not in inspect.getsource(fn).lower(), fn.__name__
    root = pathlib.Path(vol.__file__).resolve().parents[1]
    for rel in ("adapters/garch_adapter.py", "worker.py"):
        assert "sigma_uncond" not in (root / rel).read_text(encoding="utf-8"), rel
    src = (root / "families/vol.py").read_text(encoding="utf-8")
    body = src[src.index("from __future__"):]
    uses = [ln.strip() for ln in body.splitlines() if "sigma_uncond" in ln]
    assert all(ln.startswith('"sigma_uncond":') or 'state["sigma_uncond"]' in ln for ln in uses), uses
    assert [n for n in dir(vol) if "uncond" in n.lower()] == []


def test_legacy_unconditional_bundle_cannot_be_served():
    from app import failures

    legacy = pickle.dumps({"params": {}, "sigma_uncond": 0.01, "nu": 8.0})
    with pytest.raises(failures.DataError):
        vol.state_from_bundle_inner(legacy)


def test_train_entrypoint_bundle_and_adapter_alias():
    from app.adapters import garch_adapter

    df = pd.DataFrame({"ret": R[:1500], "label": np.append(R[1:1500], 0.0)})
    d = {"output": {"quantile_levels": [0.05, 0.5, 0.95]}}
    art, m = garch_adapter.train(d, df, noop_emit)
    assert art[:8] == engine.MAGIC and m["arch"] == "garch" and np.isfinite(m["crps"])
    assert m["serving"] == "conditional_sigma_or_abstain"
    hl = int.from_bytes(art[8:12], "little")
    inner = art[12 + hl:]
    st = vol.state_from_bundle_inner(inner)
    assert vol.serve_from_state(st, R[:80])["abstain"] is False
    art2, _ = vol.train_egarch(d, df, noop_emit)
    assert art2[:8] == engine.MAGIC
