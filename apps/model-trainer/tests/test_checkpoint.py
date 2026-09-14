"""The checkpoint manifest contract from the trainer's side (AT-61)."""

from __future__ import annotations

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

from app import checkpoint  # noqa: E402


def complete(framework: str = "torch", **over):
    base = dict(
        framework=framework,
        step=1200,
        optimizer_state_ref="art_opt",
        lr_scheduler_state_ref="art_sched",
        dataloader_position=4096,
        amp_scaler_state="art_scaler",
        code_hash="sha256:code",
        image_digest="sha256:image",
        dataset_id="ds:abc",
    )
    base.update(over)
    return checkpoint.build_manifest(**base)


def test_a_built_manifest_satisfies_the_contract():
    m = complete()
    checkpoint.validate(m)
    for field in checkpoint.REQUIRED_FIELDS:
        assert m.get(field) is not None, field
    for source in checkpoint.REQUIRED_RNG_SOURCES:
        assert m["rng_state"].get(source) is not None, source


def test_every_rng_source_is_captured_and_cuda_may_be_empty():
    rng = checkpoint.capture_rng_state()
    assert set(rng) == set(checkpoint.REQUIRED_RNG_SOURCES)
    # A CPU-only box has no CUDA generators. Empty is a claim; absent is not.
    assert isinstance(rng["torch_cuda"], list)
    assert rng["python"] and rng["numpy"] and rng["torch_cpu"]


def test_the_rng_capture_actually_tracks_the_generator():
    """A capture that returned a constant would pass every structural check and
    be worthless. This is the test that it is reading something."""
    import random

    random.seed(1)
    a = checkpoint.capture_rng_state()["python"]
    random.seed(2)
    b = checkpoint.capture_rng_state()["python"]
    assert a != b


def test_validate_names_the_missing_field():
    for field in checkpoint.REQUIRED_FIELDS:
        m = complete()
        m.pop(field)
        with pytest.raises(ValueError, match=field):
            checkpoint.validate(m)


def test_an_empty_reference_is_refused():
    for field in ("optimizer_state_ref", "code_hash", "dataset_id"):
        m = complete()
        m[field] = "   "
        with pytest.raises(ValueError, match=field):
            checkpoint.validate(m)


def test_a_boosting_framework_must_supply_its_round_and_seed():
    with pytest.raises(ValueError, match="boosting_round"):
        complete(framework="lightgbm")

    m = complete(framework="lightgbm", boosting_round=400, seed=7)
    checkpoint.validate(m)
    # LightGBM without force_row_wise differs run to run on one machine; the
    # manifest records which flags were actually on.
    assert m["boosting"]["determinism_flags"]["force_row_wise"] == "true"
    assert m["boosting"]["determinism_flags"]["deterministic"] == "true"


def test_resume_support_is_a_measurement_not_an_intention():
    """No framework may claim bit-identical resume until AT-61 has shown it."""
    assert checkpoint.BIT_IDENTICAL_FRAMEWORKS == frozenset()
    for fw in ("torch", "lightgbm", "xgboost", "sklearn"):
        assert checkpoint.resume_support(fw) == "unsupported"
        assert complete(framework=fw, boosting_round=1, seed=1)["resume_support"] == (
            "unsupported"
        )
