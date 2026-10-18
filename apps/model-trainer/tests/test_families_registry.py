"""N-2.1: the family registry."""
import json
import pathlib

import pytest

import _fam_common  # noqa: F401
from app.families import registry


def test_registry_json_is_valid_and_complete():
    entries = registry.load()
    keys = {(e["family"], e["variant"]) for e in entries}
    want = (
        [("baseline", v) for v in ("mean", "ewma", "ridge", "har", "log_har", "harq")]
        + [("vol", v) for v in ("garch", "gjr", "egarch", "realized_garch")]
        + [("regime", v) for v in ("hmm", "jm", "cjm", "sjm")]
        + [("probabilistic", v) for v in ("ngboost", "qrf", "conformal")]
        + [("foundation", v) for v in ("chronos", "timesfm", "ttm")]
        + [("sequence", v) for v in ("lstm", "tcn", "dlinear", "nhits", "patchtst", "tft", "tsmixer")]
        + [("byo", "model_interface")]
    )
    for fv in want:
        assert fv in keys, fv
    for e in entries:
        assert set(registry.REQUIRED_KEYS) <= set(e)
    raw = json.loads(pathlib.Path(registry.REGISTRY_PATH).read_text(encoding="utf-8"))
    assert len(raw) == len(entries)


def test_unknown_pair_is_refused_with_known_list():
    with pytest.raises(registry.UnknownFamily) as ei:
        registry.resolve("gbm", "nope")
    assert "gbm/lightgbm" in str(ei.value) and "vol/garch" in str(ei.value)


def test_sequence_family_refuses_dataset_without_sequence():
    with pytest.raises(registry.SequenceRequired):
        registry.resolve("sequence", "tcn", {"declaration": {"sequence": None}})
    try:
        registry.resolve("sequence", "tcn", {"declaration": {"sequence": {"window": 64, "stride": 1}}})
    except registry.RegistryError:
        pytest.fail("guard refused a dataset that has a sequence")
    except (ImportError, AttributeError):
        pass  # the sequence module belongs to another track


@pytest.mark.parametrize("fw,kind,quant,expect", [
    ("xgboost", "classifier", False, ("gbm", "xgboost")),
    ("lightgbm", "classifier", False, ("gbm", "lightgbm")),
    ("sklearn", "classifier", False, ("linear", "sklearn")),
    ("torch", "classifier", False, ("nn", "torch")),
    ("torch", "forecaster", False, ("forecaster", "lstm")),
    ("forecaster", "forecaster", False, ("forecaster", "lstm")),
    ("garch", "forecaster", False, ("vol", "garch")),
    ("lightgbm", "forecaster", True, ("gbm", "lightgbm_q")),
    ("xgboost", "forecaster", True, ("gbm", "xgboost_q")),
    ("sklearn", "forecaster", True, ("linear", "sklearn_q")),
])
def test_every_legacy_framework_is_reachable(fw, kind, quant, expect):
    d = {"output": {"quantile_levels": [0.1, 0.9]}} if quant else {}
    assert registry.alias_for(fw, kind, d) == expect
    assert callable(registry.route(fw, kind, d))  # imports the adapter lazily


def test_route_prefers_explicit_family_and_garch_adapter_is_vol():
    from app.adapters import garch_adapter
    from app.families import vol

    fn = registry.route("", "", {"family": {"family": "vol", "variant": "gjr"}})
    assert fn is vol.train_gjr
    assert callable(garch_adapter.train)


def test_unsupported_framework_refused():
    with pytest.raises(ValueError):
        registry.route("tensorflow", "classifier", {})


def test_foundation_never_selectable_as_candidate():
    for v in ("chronos", "timesfm", "ttm"):
        with pytest.raises(registry.NotSelectableAsCandidate):
            registry.resolve("foundation", v, as_candidate=True)
        with pytest.raises(registry.NotSelectableAsCandidate):
            registry.route("", "", {"family": {"family": "foundation", "variant": v}})
        e = registry.entry("foundation", v)
        assert e["selectable_as_candidate"] is False and e["contamination"]["model_cutoff"]
