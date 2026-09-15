"""AT-61 — checkpoint resume is bit-identical, per framework.

The measurement is `app/resume_eval.py`; this is the test that runs it and that
holds `checkpoint.BIT_IDENTICAL_FRAMEWORKS` to what it found.

The whole suite exists to stop one specific thing: a name in that set that
nobody measured. So the assertions run in both directions — a framework the
measurement proved must be in the set, and a framework the measurement failed
must not be — and a framework that is *claimed* but cannot be measured in this
environment is a **failure**, not a skip. A claim you cannot check is the thing
the set was created to prevent.
"""

from __future__ import annotations

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

from app import checkpoint, resume_eval  # noqa: E402


@pytest.fixture(scope="module")
def evidence():
    """The measurement, run once. Every process this spawns is a real one."""
    return resume_eval.measure_all()


@pytest.fixture(scope="module")
def measured(evidence):
    return {e.framework: e for e in evidence}


# ───────────────────────────────────────────────────────────────────────────────
# The acceptance test itself
# ───────────────────────────────────────────────────────────────────────────────


def test_at61_resume_reproduces_the_uninterrupted_run(measured):
    """Train `n`, kill at `n-2`, resume, compare — per framework.

    The comparison is on the parameter bytes, the prediction bytes and the packed
    metric bytes. Nothing here is rounded and nothing is `allclose`.
    """
    claimed = set(checkpoint.BIT_IDENTICAL_FRAMEWORKS)
    proved = {
        name
        for name, e in measured.items()
        if e.outcome == resume_eval.BIT_IDENTICAL
    }
    unmeasurable = {
        name
        for name, e in measured.items()
        if e.outcome == resume_eval.NOT_INSTALLED
    }

    assert not (claimed & unmeasurable), (
        f"{sorted(claimed & unmeasurable)} claim bit-identical resume and could not be "
        f"measured here — install the framework or drop the claim; an unverifiable "
        f"claim is exactly what this set exists to prevent"
    )

    diverged = {
        name: e.cause
        for name, e in measured.items()
        if e.outcome == resume_eval.DIVERGED
    }
    offending = claimed & set(diverged)
    assert not offending, (
        "these frameworks claim bit-identical resume and did not reproduce: "
        + "; ".join(f"{n}: {diverged[n]}" for n in sorted(offending))
    )

    assert proved == claimed, (
        f"the measurement and the claim disagree: measured bit-identical "
        f"{sorted(proved)}, BIT_IDENTICAL_FRAMEWORKS says {sorted(claimed)}"
    )


def test_every_framework_got_an_answer(measured):
    """No framework the platform can train is left without a verdict."""
    assert set(measured) == {"torch", "xgboost", "lightgbm", "sklearn", "garch"}
    for name, e in measured.items():
        assert e.outcome in (
            resume_eval.BIT_IDENTICAL,
            resume_eval.DIVERGED,
            resume_eval.NO_CHECKPOINT_BOUNDARY,
            resume_eval.NOT_INSTALLED,
        ), name
        if e.outcome != resume_eval.BIT_IDENTICAL:
            assert e.cause, f"{name} is not bit-identical and names no cause"


def test_the_checkpointing_process_was_actually_killed(measured):
    """A resume proved against a process that shut down cleanly proves nothing.

    `Popen.kill()` is what ends it, so a zero exit status would mean the child
    finished on its own and the checkpoint was never the only surviving state.
    """
    checked = 0
    for e in measured.values():
        for config in e.configs:
            assert config.killed_exit_code is not None, config.config
            assert config.killed_exit_code != 0, (
                f"{e.framework}/{config.config}: the checkpointing process exited 0, "
                f"so it was not killed and this measurement is worthless"
            )
            checked += 1
    assert checked, "no configuration was measured at all"


# ───────────────────────────────────────────────────────────────────────────────
# The comparator is not lenient
# ───────────────────────────────────────────────────────────────────────────────


def test_the_comparator_detects_a_one_bit_change_in_real_weights(measured):
    """Flip one bit of one parameter of a real trained model; the comparison must
    fail. A comparator that cannot fail is not evidence of anything."""
    checked = 0
    for e in measured.values():
        for config in e.configs:
            outputs = config.uninterrupted
            assert outputs is not None
            assert resume_eval.compare(outputs, outputs).identical
            mutated = resume_eval._mutation_probe(outputs)
            result = resume_eval.compare(outputs, mutated)
            assert not result.identical, f"{e.framework}/{config.config}"
            assert not result.weights_equal
            checked += 1
    assert checked


def test_a_metric_that_differs_in_its_last_bit_is_a_difference():
    """The classic way to fake this test is to compare metrics as rounded
    decimals. Metrics are carried as packed doubles precisely so that cannot
    happen, and this is the proof."""
    a = resume_eval.Outputs(b"w", b"p", {"loss": resume_eval.pack(0.1)})
    b = resume_eval.Outputs(
        b"w", b"p", {"loss": resume_eval.pack(0.1 + 5e-17)}
    )
    assert resume_eval.pack(0.1) != resume_eval.pack(0.1 + 5e-17)
    assert not resume_eval.compare(a, b).identical
    assert not resume_eval.compare(a, b).metrics_equal


def test_a_prediction_that_differs_anywhere_is_a_difference():
    a = resume_eval.Outputs(b"w", b"\x00" * 16, {})
    b = resume_eval.Outputs(b"w", b"\x00" * 15 + b"\x01", {})
    assert not resume_eval.compare(a, b).predictions_equal


def test_the_lightgbm_exclusion_hides_nothing():
    """`lgb_model_weights` drops exactly one line — the one that echoes the last
    `lgb.train` call's `num_boost_round`. That is the only leniency in the
    module, so it gets its own adversarial test: change a leaf value and the
    comparison must still fail."""
    lgb = pytest.importorskip("lightgbm")

    x, y = resume_eval.dataset()
    booster = lgb.train(
        resume_eval._lgb_params("deterministic"),
        lgb.Dataset(x, label=y, free_raw_data=False),
        num_boost_round=3,
    )
    model_string = booster.model_to_string()
    baseline = resume_eval.lgb_model_weights(model_string)

    # Only the call-argument line is removed.
    assert len(baseline.splitlines()) == len(model_string.splitlines()) - 1
    assert resume_eval.LGB_CALL_ARGUMENT_LINE not in baseline.decode("utf-8")

    # A changed leaf value still shows up.
    lines = model_string.splitlines()
    leaf_line = next(i for i, ln in enumerate(lines) if ln.startswith("leaf_value="))
    head, _, values = lines[leaf_line].partition("=")
    first, *rest = values.split(" ")
    lines[leaf_line] = f"{head}=" + " ".join([str(float(first) + 1.0), *rest])
    assert resume_eval.lgb_model_weights("\n".join(lines)) != baseline

    # And if LightGBM ever stops writing that line, this must break loudly
    # rather than start excluding something else.
    with pytest.raises(ValueError, match="num_iterations"):
        resume_eval.lgb_model_weights("no such line here")


# ───────────────────────────────────────────────────────────────────────────────
# What the measurement licenses
# ───────────────────────────────────────────────────────────────────────────────


def test_a_framework_without_a_step_boundary_says_so(measured):
    """sklearn and garch are reported `no_checkpoint_boundary`. The claim is
    checked against the adapters' own source rather than believed."""
    root = pathlib.Path(__file__).resolve().parents[1] / "app" / "adapters"
    for module in ("sklearn_adapter.py", "garch_adapter.py"):
        src = (root / module).read_text(encoding="utf-8")
        assert "warm_start" not in src, f"{module} now has a resume boundary"
        assert "partial_fit" not in src, f"{module} now has a resume boundary"
        assert "init_model" not in src, f"{module} now has a resume boundary"
        assert "xgb_model" not in src, f"{module} now has a resume boundary"

    from sklearn.ensemble import RandomForestRegressor
    from sklearn.linear_model import Ridge

    for cls in (RandomForestRegressor, Ridge):
        assert not hasattr(cls, "partial_fit"), cls.__name__

    for name in ("sklearn", "garch"):
        assert measured[name].outcome == resume_eval.NO_CHECKPOINT_BOUNDARY
        assert name not in checkpoint.BIT_IDENTICAL_FRAMEWORKS
        assert name not in checkpoint.SAMPLING_SENSITIVE_FRAMEWORKS
        assert checkpoint.resume_support(name) == "unsupported"


def test_the_sampling_sensitive_frameworks_are_the_ones_that_diverged(measured):
    """The conditional list is a record of the same measurement.

    A framework whose *only* failures are its sampling configurations is
    conditional; one whose deterministic configuration also failed would not be,
    and must not be listed.
    """
    for name in checkpoint.SAMPLING_SENSITIVE_FRAMEWORKS:
        e = measured[name]
        if e.outcome == resume_eval.NOT_INSTALLED:
            pytest.fail(
                f"{name} is listed as conditionally bit-identical and was not measured here"
            )
        assert e.deterministic_config_identical, (
            f"{name} is listed as bit-identical without sampling, and its "
            f"deterministic configuration did not reproduce: {e.cause}"
        )
        assert e.stochastic_configs_that_diverged, (
            f"{name} is listed as sampling-sensitive but no sampling configuration "
            f"diverged — either the list is stale or the measurement stopped testing it"
        )


def test_a_divergence_is_the_resume_and_not_nondeterminism(measured):
    """Every configuration reported as diverged is shown to be deterministic
    first. Otherwise "the resume broke it" would be a guess."""
    checked = 0
    for e in measured.values():
        for config in e.configs:
            if config.identical:
                continue
            assert config.repeatability is not None, config.config
            assert config.repeatability.identical, (
                f"{e.framework}/{config.config} is not deterministic run to run "
                f"({config.repeatability.detail}), so its resume result says nothing"
            )
            checked += 1
    assert checked, "no divergence was measured, so nothing needed ruling out"


def test_resume_support_will_not_answer_without_the_configuration():
    """A sampling-sensitive framework with no hyperparameters supplied is
    `unsupported`. Failing closed is the whole design: the cost is a wasted
    resume, and the alternative cost is a false claim."""
    for name in checkpoint.SAMPLING_SENSITIVE_FRAMEWORKS:
        assert checkpoint.resume_support(name) == "unsupported"
        assert checkpoint.resume_support(name, hyperparameters=None) == "unsupported"
        assert (
            checkpoint.resume_support(name, hyperparameters={}) == "bit_identical"
        ), "an explicitly empty configuration engages no sampling knob"


def test_every_sampling_knob_the_measurement_used_is_caught():
    """The knob list and the measured configurations are the same list.

    If `resume_eval` grows a configuration whose knob `checkpoint` does not know
    about, a manifest would declare `bit_identical` for a run that AT-61 has just
    shown is not.
    """
    measured_knobs = {
        "xgboost": resume_eval.XGB_CONFIGS,
        "lightgbm": resume_eval.LGB_CONFIGS,
    }
    for framework, configs in measured_knobs.items():
        for config, params in configs.items():
            engaged = checkpoint.sampling_is_stochastic(framework, params)
            if config == "deterministic":
                assert not engaged, f"{framework}/{config} should engage no knob"
            else:
                assert engaged, (
                    f"{framework}/{config} sets {params} and "
                    f"checkpoint.sampling_is_stochastic does not notice"
                )


def test_an_unknown_framework_is_never_bit_identical():
    for name in ("jax", "catboost", "", "TORCH-ish"):
        assert checkpoint.resume_support(name) == "unsupported"
    assert checkpoint.sampling_is_stochastic("catboost", {}) is True


def test_the_manifest_carries_what_the_measurement_licensed():
    """End to end: the manifest a torch run writes says `bit_identical`, the one
    a subsampling xgboost run writes does not, and both pass the contract."""
    common = dict(
        step=4,
        optimizer_state_ref="art_opt",
        lr_scheduler_state_ref="art_sched",
        dataloader_position=256,
        amp_scaler_state="art_scaler",
        code_hash="sha256:code",
        image_digest="sha256:image",
        dataset_id="ds:abc",
    )
    torch_manifest = checkpoint.build_manifest(framework="torch", **common)
    checkpoint.validate(torch_manifest)
    assert torch_manifest["resume_support"] == "bit_identical"

    clean = checkpoint.build_manifest(
        framework="xgboost",
        boosting_round=4,
        seed=7,
        hyperparameters={"subsample": 1.0, "colsample_bytree": 1.0},
        **common,
    )
    checkpoint.validate(clean)
    assert clean["resume_support"] == "bit_identical"

    sampled = checkpoint.build_manifest(
        framework="xgboost",
        boosting_round=4,
        seed=7,
        hyperparameters={"subsample": 0.7},
        **common,
    )
    checkpoint.validate(sampled)
    assert sampled["resume_support"] == "unsupported"
