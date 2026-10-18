"""N-2.10: every HPO trial is counted and reported under the key the platform reads."""
from __future__ import annotations

import pathlib
import re

import pandas as pd
import pytest

pytest.importorskip("optuna")

from app import hpo  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[3]


def _definition(n: int) -> dict:
    return {"framework": "lightgbm_q", "seed": 7, "hpo": {"enabled": True, "max_trials": n}}


def _ok(definition, df, emit):
    hp = definition.get("hyperparameters") or {}
    return b"", {"test_crps": 0.1 + float(hp.get("learning_rate", 0.1))}


def test_forty_trials_report_exactly_forty():
    _, n = hpo.run_hpo(_definition(40), pd.DataFrame({"x": [1.0]}), None, _ok, lambda *a: None)
    assert n == 40


def test_a_trial_that_fails_still_counts():
    calls = {"n": 0}

    def flaky(definition, df, emit):
        calls["n"] += 1
        if calls["n"] % 2 == 0:
            raise RuntimeError("diverged")
        return _ok(definition, df, emit)

    _, n = hpo.run_hpo(_definition(10), pd.DataFrame({"x": [1.0]}), None, flaky, lambda *a: None)
    assert n == 10


def test_when_every_trial_fails_the_trials_are_counted_and_nothing_is_tuned():
    def boom(definition, df, emit):
        raise RuntimeError("no")

    d = _definition(6)
    best, n = hpo.run_hpo(d, pd.DataFrame({"x": [1.0]}), None, boom, lambda *a: None)
    assert n == 6
    assert best["hyperparameters"] == {}


def test_a_disabled_search_counts_nothing():
    _, n = hpo.run_hpo({"framework": "lightgbm_q"}, pd.DataFrame(), None, _ok, lambda *a: None)
    assert n == 0


def test_the_trainer_reports_the_key_the_platform_reads():
    """Cross-language contract: the adapters write `hpo_trials`; `manager.rs` reads the same key.
    Before this fix the platform read `trial_count`, so the counter never moved."""
    rust = (ROOT / "crates" / "model-registry" / "src" / "studio.rs").read_text(encoding="utf-8")
    body = rust[rust.index("pub fn hpo_trials_reported"):]
    m = re.search(r'metrics\.get\("([a-z_]+)"\)', body)
    assert m, "hpo_trials_reported in studio.rs moved; update this contract test"
    key = m.group(1)
    for name in ("lightgbm_q_adapter", "xgboost_q_adapter", "sklearn_q_adapter"):
        src = (ROOT / "apps" / "model-trainer" / "app" / "adapters" / f"{name}.py").read_text(encoding="utf-8")
        assert f'"{key}": n_trials' in src, f"{name} does not report {key}"
