"""N-4.2 (GARCH serves the conditional sigma or abstains) and N-2.5 (windowed torch serving)."""
from __future__ import annotations

import io
import json
import pathlib
import pickle
import struct
import sys

import numpy as np
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import v2  # noqa: E402
from app.predictors import base, garch_predictor, run_predict, torch_predictor  # noqa: E402

MAGIC = b"TBNDL001"


def wrap(inner: bytes, **header) -> bytes:
    h = {"schema_version": "tb-bundle-1", "feature_order": [], "scaler": None, "extra": {}, **header}
    hb = json.dumps(h).encode()
    return MAGIC + struct.pack("<I", len(hb)) + hb + inner


STATE = {
    "kind": "garch", "mu": 0.0, "scale": 100.0, "nu": 8.0, "init_var": 1.0, "min_bars": 30,
    "params": {"omega": 0.05, "alpha": 0.1, "beta": 0.85, "nu": 8.0},
    "sigma_uncond": 0.01,
}


def garch_bundle(state=STATE) -> bytes:
    return wrap(pickle.dumps({"state": state, "levels": [0.05, 0.5, 0.95]}),
                framework="garch", objective="quantile", output_kind="distribution")


def inst(returns):
    return {"instrument_id": "x", "features": {"recent_returns": list(returns)}}


CALM = [0.001, -0.001] * 20
SPIKED = CALM[:-3] + [0.001, -0.06, 0.05]


def sigma_of(art, rets):
    [f] = run_predict(art, [inst(rets)], "forecaster", "1h")
    return f.sigma


def test_serving_after_a_volatility_spike_is_above_the_calm_sigma_and_matches_the_recursion():
    art = garch_bundle()
    calm, spiked = sigma_of(art, CALM), sigma_of(art, SPIKED)
    assert spiked > 3 * calm
    # independent recomputation of the recursion in percent^2 units
    s2 = 1.0
    for x in np.asarray(SPIKED) * 100:
        s2 = 0.05 + 0.1 * x * x + 0.85 * s2
    assert spiked == pytest.approx(np.sqrt(s2) / 100, rel=1e-9)


def test_the_unconditional_sigma_is_never_served():
    art = garch_bundle()
    assert sigma_of(art, CALM) != pytest.approx(STATE["sigma_uncond"])
    src = pathlib.Path(garch_predictor.__file__).read_text(encoding="utf-8")
    code = "\n".join(l for l in src.splitlines() if not l.lstrip().startswith(("#", '"""')))
    assert 'state.get("sigma_uncond")' not in code and 'state["sigma_uncond"]' not in code


@pytest.mark.parametrize("bars,reason", [(None, "no_recent_bars"), ([0.001] * 5, "insufficient_recent_bars"),
                                         ([0.001] * 29 + [float("nan")], "non_finite_recent_bars")])
def test_missing_short_or_nonfinite_bars_abstain_with_a_reason(bars, reason):
    art = garch_bundle()
    instance = {"instrument_id": "x", "features": {} if bars is None else {"recent_returns": bars}}
    with pytest.raises(base.Abstain) as e:
        run_predict(art, [instance], "forecaster", "1h")
    assert reason in e.value.reason
    [a] = v2.predict_v2(art, [{"key": "k", "features": instance["features"]}], "forecaster")
    assert a["status"] == "abstained" and reason in a["detail"] and a["outputs"] == {}


def test_a_legacy_bundle_with_only_an_unconditional_sigma_abstains():
    art = wrap(pickle.dumps({"sigma_uncond": 0.01, "nu": 8.0}), framework="garch", objective="quantile")
    with pytest.raises(base.Abstain, match="legacy_garch_bundle"):
        run_predict(art, [inst(CALM)], "forecaster", "1h")


def test_v2_answers_with_sigma_and_quantiles():
    [a] = v2.predict_v2(garch_bundle(), [{"key": "k", "features": {"recent_returns": SPIKED}}], "forecaster")
    assert a["status"] == "answered"
    vals = a["outputs"]["quantiles"]["values"]
    assert vals == sorted(vals) and a["outputs"]["sigma"] > 0


def test_gjr_and_egarch_and_realized_kinds_serve_too():
    for kind, params in {
        "gjr": {"omega": 0.05, "alpha": 0.05, "gamma": 0.1, "beta": 0.85, "nu": 8.0},
        "egarch": {"omega": 0.0, "alpha": 0.1, "gamma": -0.05, "beta": 0.95, "nu": 8.0},
    }.items():
        st = {**STATE, "kind": kind, "params": params}
        assert 0 < sigma_of(garch_bundle(st), SPIKED) < 1
    st = {**STATE, "kind": "realized_garch", "params": {"omega": -0.1, "beta": 0.8, "gamma": 0.2}}
    art = garch_bundle(st)
    with pytest.raises(base.Abstain, match="no_realized_measure"):
        run_predict(art, [inst(CALM)], "forecaster", "1h")
    f = {"recent_returns": CALM, "recent_realized": [1e-6] * len(CALM)}
    [out] = run_predict(art, [{"instrument_id": "x", "features": f}], "forecaster", "1h")
    assert out.sigma > 0


# ---------------------------------------------------------------- sequence serving
torch = pytest.importorskip("torch")


def lstm_bundle(window: int | None, n_feat: int = 2) -> bytes:
    net = torch_predictor.LSTMNet(n_feat, 8)
    buf = io.BytesIO()
    torch.save({"state_dict": net.state_dict(), "arch": "lstm", "input_dim": n_feat, "hidden_dim": 8,
                "objective": "regression"}, buf)
    extra = {"feature_order": ["a", "b"], "objective": "regression", "framework": "torch", "arch": "lstm"}
    if window:
        extra["sequence"] = {"window": window, "stride": 1}
    return wrap(buf.getvalue(), **extra)


def bars(n):
    rng = np.random.default_rng(0)
    return [{"instrument_id": "x", "features": {"a": float(v), "b": float(w)}}
            for v, w in rng.normal(size=(n, 2))]


def test_a_windowed_bundle_abstains_below_the_window_and_answers_at_it():
    art = lstm_bundle(window=16)
    with pytest.raises(base.Abstain, match="insufficient_history: have 15, need 16"):
        run_predict(art, bars(15), "forecaster", "1h")
    out = run_predict(art, bars(16), "forecaster", "1h")
    assert len(out) == 1  # one forecast, for the latest bar


def test_the_input_is_one_window_of_the_latest_bars(monkeypatch):
    seen = []
    orig = torch_predictor.LSTMNet.forward

    def spy(self, x):
        seen.append(tuple(x.shape))
        return orig(self, x)

    monkeypatch.setattr(torch_predictor.LSTMNet, "forward", spy)
    run_predict(lstm_bundle(window=16), bars(40), "forecaster", "1h")
    assert seen == [(1, 16, 2)]


def test_without_a_sequence_block_serving_is_unchanged_per_row():
    out = run_predict(lstm_bundle(window=None), bars(5), "forecaster", "1h")
    assert len(out) == 5


def test_v2_marks_context_rows_and_abstains_on_short_history():
    art = lstm_bundle(window=4)
    rows = lambda n: [{"key": f"r{i}", "features": b["features"]} for i, b in enumerate(bars(n))]  # noqa: E731
    short = v2.predict_v2(art, rows(3), "forecaster")
    assert {a["status"] for a in short} == {"abstained"} and "insufficient_history" in short[0]["detail"]
    full = v2.predict_v2(art, rows(6), "forecaster")
    assert [a["status"] for a in full] == ["abstained"] * 5 + ["answered"]
