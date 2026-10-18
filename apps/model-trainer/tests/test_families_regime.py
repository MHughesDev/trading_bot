"""N-2.4: filtered-only regime labels."""
import numpy as np
import pandas as pd
import pytest

from _fam_common import noop_emit
from app import engine
from app.families import regime


def _data(T=1500, seed=1):
    rng = np.random.default_rng(seed)
    s = np.zeros(T, int)
    for t in range(1, T):
        s[t] = s[t - 1] if rng.random() > 0.02 else 1 - s[t - 1]
    x = np.where(s == 0, 0.005, 0.02) * rng.standard_normal(T)
    return np.column_stack([np.abs(x), x, rng.standard_normal(T)]), s


X, S = _data()


@pytest.mark.parametrize("kind", regime.KINDS)
def test_label_at_t_unchanged_when_later_bars_are_appended(kind):
    st = regime.fit(kind, X[:1000])
    full, pfull = regime.filter_labels(st, X)
    for cut in range(60, 1500, 11):
        part, ppart = regime.filter_labels(st, X[:cut])
        assert np.array_equal(full[:cut], part), (kind, cut)
        assert np.allclose(pfull[:cut], ppart)


@pytest.mark.parametrize("kind", regime.KINDS)
def test_regimes_are_recovered_out_of_sample(kind):
    st = regime.fit(kind, X[:1000])
    lab, _ = regime.filter_labels(st, X)
    acc = max(np.mean(lab[1000:] == S[1000:]), np.mean(lab[1000:] != S[1000:]))
    assert acc > 0.75


def test_no_smoothed_path_exists():
    assert regime.SMOOTHED_PATH_EXISTS is False
    public = [n for n in dir(regime) if not n.startswith("_") and n != "SMOOTHED_PATH_EXISTS"]
    assert not [n for n in public if any(w in n.lower() for w in ("smooth", "viterbi", "backward"))]
    st = regime.fit("hmm", X[:800])
    assert not hasattr(regime, "smooth_labels")
    # filtered probabilities are a proper distribution
    _, p = regime.filter_labels(st, X[:200])
    assert np.allclose(p.sum(1), 1.0)


def test_sparse_jm_zeroes_the_noise_feature():
    st = regime.fit("sjm", X[:1000], sparse_budget=1.2)
    w = st["w"]
    assert w[2] < 0.5 * max(w[0], w[1])  # the pure-noise third column is down-weighted


def test_train_entrypoint_emits_regime_labels_artifact():
    df = pd.DataFrame({"f_abs": X[:, 0], "f_ret": X[:, 1], "label": np.zeros(len(X))})
    for fn in (regime.train_hmm, regime.train_jm, regime.train_cjm, regime.train_sjm):
        art, m = fn({}, df, noop_emit)
        assert art[:8] == engine.MAGIC
        assert m["artifact_type"] == "regime_labels" and m["filtered_only"] is True
        hl = int.from_bytes(art[8:12], "little")
        import json
        header = json.loads(art[12:12 + hl])
        assert header["output_kind"] == "regime_labels"
        assert header["extra"]["artifact_type"] == "regime_labels"
