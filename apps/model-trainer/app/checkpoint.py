"""Checkpoint manifests from the trainer's side (SPEC 9, checklist 2.4/2.5,
ADR-P2-07, AT-61).

The Rust registry refuses a checkpoint whose manifest is missing any contract
field. This is the half that produces one, and the reason it lives here is that
only the training process can see the state: the RNG generators, the optimizer's
moments, where the dataloader had got to.

`capture_rng_state` is the one worth reading twice. Every generator it captures
is consumed by a real training loop, and the one left out is the one that makes
a resumed run silently different — a different dropout mask, a different
shuffle, a different augmentation, and a loss curve that looks completely
normal. There is no symptom. That is why the contract is checked rather than
trusted.

## `resume_support`

A framework declares whether its resume is bit-identical. `unsupported` is a
real and useful answer: the run restarts from step 0 under the same trial rather
than resuming into an approximation. What is not allowed is claiming
`bit_identical` without AT-61 having demonstrated it.

The demonstration is `app/resume_eval.py`, run by
`tests/test_resume_eval.py::test_at61_resume_reproduces_the_uninterrupted_run`.
It trains `n` steps, trains `n-2` and **kills the process**, resumes from what
reached the disk, and compares the parameter bytes, the prediction bytes and the
packed metric bytes. What it found, 2026-09-15:

* **torch** — bit-identical, across `mlp`, `lstm` and an `mlp` with dropout.
  Unconditional, so it is in `BIT_IDENTICAL_FRAMEWORKS`.
* **xgboost** and **lightgbm** — bit-identical *only when row and column
  sampling are off*. Their sampling RNG is not part of the saved booster, so a
  resumed run draws a different subsample for the remaining rounds. Two
  uninterrupted runs of the sampling configuration agree with each other, so
  this is the resume and not nondeterminism. They are therefore **not**
  unconditionally bit-identical; `SAMPLING_SENSITIVE_FRAMEWORKS` records the
  condition and `resume_support` requires the hyperparameters before it will say
  `bit_identical` for one of them.
* **sklearn** and **garch** — no checkpoint boundary at all. Both adapters fit
  in a single call, so there is no step `n-2` to resume from. Not a failure;
  a different answer.
"""

from __future__ import annotations

import base64
import hashlib
import random
from typing import Any

import numpy as np

# Frameworks whose resume AT-61 demonstrated bit-identical **under every
# configuration the platform can produce**.
#
# A record of measurements, not a statement of intent. A name enters this set
# only when `app/resume_eval.py` has trained it, killed it, resumed it and found
# the parameter bytes identical — and `test_resume_eval.py` re-runs that
# measurement on every PR and fails if this set and the measurement disagree in
# either direction. There is no way to add a name here that the test will not
# check.
BIT_IDENTICAL_FRAMEWORKS: frozenset[str] = frozenset({"torch"})

# Frameworks whose resume is bit-identical **only when sampling is off**, with
# the hyperparameters that turn sampling on.
#
# XGBoost and LightGBM both carry their boosting rounds in the saved model and
# neither carries the state of the generator that draws the row/column sample.
# Resume the booster and the remaining rounds are fitted on a different sample:
# the trees differ, and nothing says so. Measured, not inferred — AT-61 runs each
# knob below and finds the resumed model different, and finds two uninterrupted
# runs of the same configuration identical, which is what rules out plain
# nondeterminism.
#
# This is why `resume_support` will not answer for one of these frameworks
# without being shown the hyperparameters. An unstated configuration is not a
# measured one, and the honest answer to "I do not know what was configured" is
# `unsupported`.
SAMPLING_SENSITIVE_FRAMEWORKS: dict[str, tuple[str, ...]] = {
    "xgboost": (
        "subsample",
        "colsample_bytree",
        "colsample_bylevel",
        "colsample_bynode",
        "sampling_method",
    ),
    # LightGBM accepts `subsample`/`colsample_bytree` as aliases of
    # `bagging_fraction`/`feature_fraction`, so both spellings are listed; a knob
    # that is only caught under one of its names is not caught.
    "lightgbm": (
        "bagging_fraction",
        "subsample",
        "feature_fraction",
        "colsample_bytree",
        "feature_fraction_bynode",
        "extra_trees",
        "data_sample_strategy",
    ),
}

# LightGBM builds histograms in thread-scheduling order unless both of these are
# set, and then differs run to run on one machine. XGBoost needs a fixed seed and
# a single thread for the reproducibility test specifically.
DETERMINISM_FLAGS: dict[str, dict[str, str]] = {
    "lightgbm": {"deterministic": "true", "force_row_wise": "true"},
    "xgboost": {"seed": "fixed", "nthread": "1"},
}


def _encode(b: bytes) -> str:
    return base64.b64encode(b).decode("ascii")


def capture_rng_state() -> dict[str, Any]:
    """Every generator a training loop draws from.

    `torch_cuda` is a list, one entry per visible device, and an **empty list is
    a valid answer** for a CPU run. It is never omitted: absent and empty are
    different claims, and only one of them can be checked.
    """
    py_state = repr(random.getstate()).encode("utf-8")
    np_state = repr(np.random.get_state()).encode("utf-8")

    torch_cpu = ""
    torch_cuda: list[str] = []
    try:
        import torch

        torch_cpu = _encode(torch.get_rng_state().numpy().tobytes())
        if torch.cuda.is_available():
            torch_cuda = [
                _encode(s.numpy().tobytes()) for s in torch.cuda.get_rng_state_all()
            ]
    except Exception:  # noqa: BLE001 - torch is optional for GBDT-only installs
        # A hash of "there is no torch state" is still a statement about the
        # state, and it is a true one. An empty string would fail the contract,
        # which is correct for a torch run and wrong for a LightGBM one; this
        # distinguishes them.
        torch_cpu = "no_torch"

    return {
        "python": _encode(hashlib.sha256(py_state).digest()),
        "numpy": _encode(hashlib.sha256(np_state).digest()),
        "torch_cpu": torch_cpu,
        "torch_cuda": torch_cuda,
    }


def _knob_is_engaged(name: str, value: Any) -> bool:
    """Whether one sampling hyperparameter is actually turned on.

    Fails closed: a value that cannot be read as a fraction is treated as
    engaged, because a knob whose setting is unreadable is a knob nobody has
    cleared.
    """
    if name == "sampling_method":
        return str(value).lower() != "uniform"
    if name == "data_sample_strategy":
        return str(value).lower() != "bagging"
    if name == "extra_trees":
        return bool(value)
    try:
        return float(value) < 1.0
    except (TypeError, ValueError):
        return True


def sampling_is_stochastic(framework: str, hyperparameters: dict[str, Any]) -> bool:
    """Whether this run draws a random row/column sample per boosting round.

    Returns `True` for any framework AT-61 has not measured. The default answer
    is the conservative one everywhere in this module.
    """
    knobs = SAMPLING_SENSITIVE_FRAMEWORKS.get(framework.lower())
    if knobs is None:
        return True
    return any(
        _knob_is_engaged(name, hyperparameters[name])
        for name in knobs
        if name in hyperparameters
    )


def resume_support(
    framework: str, *, hyperparameters: dict[str, Any] | None = None
) -> str:
    """Whether this framework's resume has been *measured* bit-identical.

    `hyperparameters` are the ones the run is actually using. They matter only
    for the frameworks in `SAMPLING_SENSITIVE_FRAMEWORKS`, and for those they are
    **required**: omitting them answers `unsupported`, because the measurement
    was conditional and a condition nobody stated has not been met. The default
    is safe in the only direction that matters — it can waste a resume, never
    claim one that is not bit-identical.
    """
    fw = framework.lower()
    if fw in BIT_IDENTICAL_FRAMEWORKS:
        return "bit_identical"
    if fw in SAMPLING_SENSITIVE_FRAMEWORKS:
        if hyperparameters is None:
            return "unsupported"
        return (
            "unsupported" if sampling_is_stochastic(fw, hyperparameters) else "bit_identical"
        )
    return "unsupported"


def build_manifest(
    *,
    framework: str,
    step: int,
    optimizer_state_ref: str,
    lr_scheduler_state_ref: str,
    dataloader_position: int,
    amp_scaler_state: str,
    code_hash: str,
    image_digest: str,
    dataset_id: str,
    boosting_round: int | None = None,
    seed: int | None = None,
    hyperparameters: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Assemble a manifest the Rust registry will accept.

    Every argument is keyword-only and required. There are no defaults, because
    a default here is a field the caller did not think about, and the field the
    caller did not think about is the one that makes the resume wrong.

    `hyperparameters` is the exception, and only because its default is the
    *conservative* answer rather than a guess: leave it out and a
    sampling-sensitive framework declares `unsupported`, which costs a resume and
    claims nothing.
    """
    manifest: dict[str, Any] = {
        "step": int(step),
        "rng_state": capture_rng_state(),
        "optimizer_state_ref": optimizer_state_ref,
        "lr_scheduler_state_ref": lr_scheduler_state_ref,
        "dataloader_position": int(dataloader_position),
        "amp_scaler_state": amp_scaler_state,
        "code_hash": code_hash,
        "image_digest": image_digest,
        "dataset_id": dataset_id,
        "resume_support": resume_support(framework, hyperparameters=hyperparameters),
    }

    flags = DETERMINISM_FLAGS.get(framework.lower())
    if flags is not None:
        if boosting_round is None or seed is None:
            raise ValueError(
                f"{framework} is a boosting framework: a checkpoint needs its "
                f"boosting_round and seed, or it cannot be resumed from"
            )
        manifest["boosting"] = {
            "boosting_round": int(boosting_round),
            "seed": int(seed),
            # The flags actually in effect. Recorded rather than assumed: a
            # checkpoint from a run that had them off is not reproducible, and
            # the manifest must not imply that it is.
            "determinism_flags": dict(flags),
        }
    return manifest


# The fields the Rust side checks, mirrored here so a missing one is caught in
# the process that could still fix it rather than after the upload.
REQUIRED_FIELDS = (
    "step",
    "rng_state",
    "optimizer_state_ref",
    "lr_scheduler_state_ref",
    "dataloader_position",
    "amp_scaler_state",
    "code_hash",
    "image_digest",
    "dataset_id",
    "resume_support",
)

REQUIRED_RNG_SOURCES = ("python", "numpy", "torch_cpu", "torch_cuda")


def validate(manifest: dict[str, Any]) -> None:
    """Raise if the manifest would be refused.

    # Raises
    `ValueError` naming the first missing or empty field.
    """
    for field in REQUIRED_FIELDS:
        if manifest.get(field) is None:
            raise ValueError(f"the checkpoint manifest has no `{field}`")
    rng = manifest["rng_state"]
    for source in REQUIRED_RNG_SOURCES:
        if rng.get(source) is None:
            raise ValueError(f"the checkpoint manifest has no `rng_state.{source}`")
    for field in (
        "optimizer_state_ref",
        "lr_scheduler_state_ref",
        "amp_scaler_state",
        "code_hash",
        "image_digest",
        "dataset_id",
    ):
        if not str(manifest[field]).strip():
            raise ValueError(f"`{field}` is empty; an empty reference is not a reference")
    boosting = manifest.get("boosting")
    if boosting is not None and not boosting.get("determinism_flags"):
        raise ValueError(
            "a boosting checkpoint must declare the determinism flags in effect"
        )
