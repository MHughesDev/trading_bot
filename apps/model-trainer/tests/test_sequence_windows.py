"""N-2.5 real windows and N-2.6 the sequence zoo."""
from __future__ import annotations

import pathlib
import sys

import numpy as np
import pandas as pd
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app import engine, failures, windows  # noqa: E402
from app.adapters import torch_adapter  # noqa: E402
from app.families import sequence as seqfam  # noqa: E402


def lag_frame(n=1400, lag=3, seed=0):
    """label[t] = x[t-lag]: invisible to a model that sees only the row at t."""
    rng = np.random.default_rng(seed)
    x = rng.normal(size=n)
    y = np.concatenate([np.zeros(lag), x[:-lag]])
    return pd.DataFrame({"x": x, "noise": rng.normal(size=n), "label": y})


def definition(window=None, arch="lstm", **extra):
    d = {"hyperparameters": {"objective": "regression", "arch": arch, "epochs": 25, "hidden_dim": 16,
                             "learning_rate": 0.01, "batch_size": 64, "seed": 7},
         "max_gpu_hours": 1.0, **extra}
    if window:
        d["declaration"] = {"sequence": {"window": window, "stride": 1}}
    return d


def quiet(*a, **k):
    return None


# ---- windowing -------------------------------------------------------------
def test_windows_are_3d_and_end_on_their_label_row():
    X = np.arange(20, dtype=float).reshape(10, 2)
    y = np.arange(10)
    Xw, yw, _ = windows.make_windows(X, 4, 1, y)
    assert Xw.shape == (7, 4, 2)
    assert list(yw) == [3, 4, 5, 6, 7, 8, 9]
    assert np.array_equal(Xw[0, :, 0], X[0:4, 0]) and np.array_equal(Xw[-1, -1], X[9])


def test_stride_thins_training_windows_only():
    p = engine.prepare(definition(5), lag_frame(300))
    p2, spec = windows.apply_sequence({"declaration": {"sequence": {"window": 5, "stride": 3}}}, p)
    assert spec == {"window": 5, "stride": 3}
    assert len(p2.X_tr) == len(range(4, len(p.X_tr), 3))
    assert len(p2.X_val) == len(p.X_val) - 4 and len(p2.X_te) == len(p.X_te) - 4


def test_no_window_crosses_a_split_boundary():
    W = 8
    df = lag_frame(400)
    df["x"] = np.arange(400, dtype=float)  # a row's value is its own index
    p0 = engine.prepare(definition(), df)
    mean, std = np.array(p0.scaler["mean"]), np.array(p0.scaler["std"])
    p, _ = windows.apply_sequence(definition(W), p0)

    def idx(a):  # un-scale feature 0 back to row indices
        return a * std[0] + mean[0]

    for arr in (p.X_tr, p.X_val, p.X_te):
        raw = idx(arr[:, :, 0])
        for win in raw:
            assert np.allclose(np.diff(win), 1.0, atol=1e-2), "a window is W contiguous rows"
    # each split's windows lie wholly before the next split's first row
    assert idx(p.X_tr[-1, -1, 0]) < idx(p.X_val[0, 0, 0])
    assert idx(p.X_val[-1, -1, 0]) < idx(p.X_te[0, 0, 0])
    # and match the 2-D split rows exactly (nothing from outside the split)
    assert np.allclose(p.X_te[0], p0.X_te[:W]) and np.allclose(p.X_val[-1], p0.X_val[-W:])


def test_serving_window_is_exactly_the_last_w_rows():
    rows = np.arange(30, dtype=float).reshape(15, 2)
    w = windows.serving_window(rows, 6)
    assert w.shape == (1, 6, 2) and np.array_equal(w[0], rows[-6:])


def test_malformed_sequence_declaration_is_a_data_error():
    for bad in ({"window": 0}, {"window": "x"}, {"stride": 2}, "64"):
        with pytest.raises(failures.DataError):
            windows.sequence_spec({"declaration": {"sequence": bad}})
    assert windows.sequence_spec({}) is None
    assert windows.sequence_spec({"declaration": {"sequence": None}}) is None


# ---- N-2.5 acceptance ------------------------------------------------------
def test_bundle_records_sequence_window_64_and_serving_abstains_below_it():
    df = lag_frame(700)
    d = definition(64)
    d["hyperparameters"]["epochs"] = 2
    bundle, metrics = torch_adapter.train(d, df, quiet)
    header, _ = windows.read_bundle(bundle)
    assert header["sequence"] == {"window": 64, "stride": 1}
    assert metrics["sequence"]["window"] == 64
    assert header["feature_order"] == ["x", "noise"]

    res = windows.predict_bundle(bundle, df.iloc[:63])
    assert res == {"abstain": True, "reason": "insufficient_history", "have": 63, "need": 64}
    ok = windows.predict_bundle(bundle, df.iloc[:64])
    assert ok["abstain"] is False and np.isfinite(ok["pred"])
    with pytest.raises(windows.InsufficientHistory):
        windows.serving_window(np.zeros((10, 2)), 64)


def test_serving_uses_only_the_last_window_of_bars():
    df = lag_frame(700)
    d = definition(16)
    d["hyperparameters"]["epochs"] = 2
    bundle, _ = torch_adapter.train(d, df, quiet)
    base = df.iloc[:300].copy()
    a = windows.predict_bundle(bundle, base)["pred"]
    older = base.copy()
    older.loc[older.index[:200], "x"] += 99.0
    assert windows.predict_bundle(bundle, older)["pred"] == pytest.approx(a)
    inside = base.copy()
    inside.loc[inside.index[-1], "x"] += 5.0
    assert windows.predict_bundle(bundle, inside)["pred"] != pytest.approx(a)


def test_a_model_on_windows_beats_the_same_model_at_seq_len_1_on_a_lag_series():
    df = lag_frame(1400, lag=3)
    _, m_win = torch_adapter.train(definition(8), df, quiet)
    _, m_row = torch_adapter.train(definition(None), df, quiet)
    assert "sequence" not in m_row
    assert m_win["test_mse"] < 0.5 * m_row["test_mse"], (m_win["test_mse"], m_row["test_mse"])
    assert m_row["test_mse"] > 0.8  # seq-len-1 sees nothing: ~ var(label) = 1


def test_legacy_definition_without_sequence_has_no_sequence_in_bundle():
    bundle, _ = torch_adapter.train(definition(None), lag_frame(300), quiet)
    header, _ = windows.read_bundle(bundle)
    assert "sequence" not in header


def test_sequence_declared_on_a_row_arch_is_refused():
    with pytest.raises(ValueError, match="sequence arch"):
        torch_adapter.train(definition(8, arch="mlp"), lag_frame(300), quiet)


# ---- N-2.6 the zoo ---------------------------------------------------------
@pytest.mark.parametrize("variant", sorted(seqfam.ARCHS))
def test_every_variant_trains_on_windows_and_records_vs_gbm(variant):
    d = definition(8, arch=variant)
    d["hyperparameters"]["epochs"] = 4
    df = lag_frame(500)
    bundle, m = getattr(seqfam, f"train_{variant}")(d, df, quiet)
    header, _ = windows.read_bundle(bundle)
    assert header["sequence"]["window"] == 8 and header["arch"] == variant
    assert m["vs_gbm"]["recorded"] is True and m["vs_gbm"]["metric"] == "mse"
    assert m["promotable"] == m["vs_gbm"]["beats_gbm"]
    res = windows.predict_bundle(bundle, df.iloc[:40])
    assert res["abstain"] is False and np.isfinite(res["pred"])
    assert windows.predict_bundle(bundle, df.iloc[:7])["abstain"] is True


@pytest.mark.parametrize("variant", sorted(seqfam.ARCHS))
def test_models_map_3d_to_one_column(variant):
    import torch
    m = seqfam.build_model(variant, 3, 8, 12)
    assert m(torch.randn(5, 12, 3)).shape == (5, 1)


def test_tcn_is_causal_in_its_window():
    import torch
    torch.manual_seed(0)
    m = seqfam.build_model("tcn", 2, 8, 12).eval()
    x = torch.randn(1, 12, 2)
    x2 = x.clone()
    x2[0, 0] += 5.0
    assert not torch.allclose(m(x), m(x2))  # uses the whole window
    # the conv stack is left-padded only: output at step t ignores steps after t
    h = m.inp(x.transpose(1, 2))
    h2 = m.inp(x.clone().transpose(1, 2))
    import torch.nn.functional as F
    xf = x.clone()
    xf[0, -1] += 5.0
    def stack(inp):
        h = m.inp(inp.transpose(1, 2))
        for conv, d in zip(m.convs, m.dil):
            h = h + F.gelu(conv(F.pad(h, ((m.kernel - 1) * d, 0))))
        return h
    assert torch.allclose(stack(x)[:, :, :-1], stack(xf)[:, :, :-1], atol=1e-6)


def test_sequence_family_refuses_without_declaration_or_budget():
    with pytest.raises(failures.DataError, match="requires declaration.sequence"):
        seqfam.train_lstm(definition(None), lag_frame(300), quiet)
    d = definition(8)
    del d["max_gpu_hours"]
    with pytest.raises(failures.DataError, match="max_gpu_hours"):
        seqfam.train_lstm(d, lag_frame(300), quiet)
    d["max_gpu_hours"] = 0
    with pytest.raises(failures.DataError, match="max_gpu_hours"):
        seqfam.train_lstm(d, lag_frame(300), quiet)


def test_wall_clock_budget_stops_training_at_the_epoch_boundary():
    d = definition(8, arch="lstm")
    d["hyperparameters"]["epochs"] = 50
    d["max_gpu_hours"] = 1e-12
    _, m = seqfam.train_lstm(d, lag_frame(400), quiet)
    assert m["budget_exhausted"] is True and m["epochs_run"] == 1
    assert m["wall_seconds"] < 30


def test_promotable_needs_a_recorded_comparison_that_the_model_wins():
    gbm = {"recorded": True, "metric": "mse", "score": 1.0}
    assert seqfam.compare_to_gbm("regression", 0.5, gbm)["beats_gbm"] is True
    assert seqfam.compare_to_gbm("regression", 1.0, gbm)["beats_gbm"] is False  # a tie is not a win
    assert seqfam.compare_to_gbm("regression", 2.0, gbm)["beats_gbm"] is False
    assert seqfam.compare_to_gbm("regression", 0.5, {"recorded": False, "reason": "x"})["beats_gbm"] is False
    assert seqfam.compare_to_gbm("regression", float("nan"), gbm)["beats_gbm"] is False


def test_unpromotable_when_the_gbm_comparison_cannot_be_made(monkeypatch):
    def boom(*a, **k):
        raise RuntimeError("lightgbm exploded")
    monkeypatch.setattr(seqfam, "gbm_on_lagged_windows", boom)
    d = definition(8)
    d["hyperparameters"]["epochs"] = 2
    _, m = seqfam.train_lstm(d, lag_frame(300), quiet)
    assert m["promotable"] is False and m["vs_gbm"]["recorded"] is False
