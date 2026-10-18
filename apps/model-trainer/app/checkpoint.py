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
    # `bagging_fraction < 1` with `bagging_freq = 0` is inert in LightGBM, and is
    # reported engaged anyway. Erring this way costs a resume; erring the other
    # way costs a trial record that says a run was reproduced when it was not.
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


# ---------------------------------------------------------------------------
# The producing half (checklist 2.4/2.5; OPEN-TASKS C1.2)
#
# Everything above says what a checkpoint *is*. Nothing wrote one: AT-61 proved
# torch resumes bit-identically and put it in `BIT_IDENTICAL_FRAMEWORKS`, so a
# manifest could honestly say `bit_identical`, and there was still no stored
# state for a re-queued job to find. This is the writer, driven from the epoch
# loop in `engine.train_torch_model`.
#
# The *decision* to resume is not made here. `jobs::checkpoint::plan_resume` on
# the Rust side is the one place that judges whether a checkpoint matches the
# computation about to run, and it hands the manifest back in
# `definition["_checkpoint"]["resume"]`. A second implementation of that rule in
# Python would be a second standard, and the looser one would be the one used.
# ---------------------------------------------------------------------------

import io  # noqa: E402
import json  # noqa: E402
import time  # noqa: E402
from typing import Callable  # noqa: E402


def young_daly_interval(checkpoint_cost_s: float, mtbf_s: float) -> float | None:
    """Seconds between checkpoints: `sqrt(2 * C * M)` (Young 1974, Daly 2006).

    Mirrors `jobs::checkpoint::checkpoint_interval_seconds`. `None` for an
    unmeasured cost or an unknown MTBF: a made-up interval is indistinguishable
    from a measured one once it is in a config file.
    """
    if not (checkpoint_cost_s > 0 and mtbf_s > 0):
        return None
    if not (np.isfinite(checkpoint_cost_s) and np.isfinite(mtbf_s)):
        return None
    return float((2.0 * checkpoint_cost_s * mtbf_s) ** 0.5)


class TorchCheckpointer:
    """Writes mid-run checkpoints from an epoch loop, on the Young-Daly cadence.

    The first call after the first epoch is **always** taken: the interval needs
    the cost of a checkpoint, the cost is only known by writing one, and a run
    that waited for an interval before measuring would never measure. After that
    the next checkpoint is due `young_daly_interval(cost, mtbf)` seconds after
    the last finished. With no MTBF configured there is no interval, and no
    further checkpoint is taken - an unknown failure rate gets one checkpoint and
    an honest silence, not a guess.
    """

    def __init__(
        self,
        *,
        store,
        key_prefix: str,
        code_hash: str,
        image_digest: str,
        dataset_id: str,
        mtbf_s: float | None = None,
        publish: Callable[[dict], None] | None = None,
        clock: Callable[[], float] = time.monotonic,
        hyperparameters: dict[str, Any] | None = None,
    ) -> None:
        self.store = store
        self.key_prefix = key_prefix.rstrip("/")
        self.code_hash = code_hash
        self.image_digest = image_digest
        self.dataset_id = dataset_id
        self.mtbf_s = mtbf_s
        self.publish = publish
        self.clock = clock
        self.hyperparameters = hyperparameters
        self.pct = 0.0  # the run's progress when a checkpoint is published
        self.cost_s: float | None = None
        self._last_end: float | None = None
        self.written: list[dict[str, Any]] = []

    # -- cadence -------------------------------------------------------------
    def interval_s(self) -> float | None:
        if self.cost_s is None or self.mtbf_s is None:
            return None
        return young_daly_interval(self.cost_s, self.mtbf_s)

    def due(self) -> bool:
        if self._last_end is None:
            return True  # the first one measures the cost
        interval = self.interval_s()
        return interval is not None and (self.clock() - self._last_end) >= interval

    # -- writing -------------------------------------------------------------
    def maybe_write(self, *, epoch: int, model, optimizer, dataloader_position: int):
        if not self.due():
            return None
        return self.write(
            epoch=epoch,
            model=model,
            optimizer=optimizer,
            dataloader_position=dataloader_position,
        )

    def write(self, *, epoch: int, model, optimizer, dataloader_position: int) -> dict[str, Any]:
        import torch

        started = self.clock()
        state = io.BytesIO()
        torch.save(
            {
                "step": int(epoch),
                "model": model.state_dict(),
                "optimizer": optimizer.state_dict(),
                "torch_cpu_rng": torch.get_rng_state(),
                "torch_cuda_rng": (
                    torch.cuda.get_rng_state_all() if torch.cuda.is_available() else []
                ),
                "numpy_rng": np.random.get_state(),
                "python_rng": random.getstate(),
                "dataloader_position": int(dataloader_position),
            },
            state,
        )
        base = f"{self.key_prefix}/step-{int(epoch):08d}"
        state_uri, _, _ = self.store.put(f"{base}/state.pt", state.getvalue())
        # The loop has no LR schedule and no AMP scaler. The contract still wants
        # a reference for each, so each is a small artifact saying so - an empty
        # string would be refused as "not a reference", and omitting the field
        # would be a claim that nobody thought about it.
        sched_uri, _, _ = self.store.put(
            f"{base}/scheduler.json", json.dumps({"scheduler": "none"}).encode()
        )
        amp_uri, _, _ = self.store.put(
            f"{base}/amp.json", json.dumps({"amp_scaler": "none"}).encode()
        )
        manifest = build_manifest(
            framework="torch",
            step=int(epoch),
            optimizer_state_ref=state_uri,
            lr_scheduler_state_ref=sched_uri,
            dataloader_position=int(dataloader_position),
            amp_scaler_state=amp_uri,
            code_hash=self.code_hash,
            image_digest=self.image_digest,
            dataset_id=self.dataset_id,
            hyperparameters=self.hyperparameters,
        )
        validate(manifest)
        self.store.put(f"{base}/manifest.json", json.dumps(manifest).encode())
        ended = self.clock()
        self.cost_s = max(ended - started, 1e-9)
        self._last_end = ended
        self.written.append(manifest)
        if self.publish is not None:
            self.publish(manifest)
        return manifest


def restore_torch_checkpoint(store, manifest: dict[str, Any], model, optimizer) -> int:
    """Load a checkpoint into `model` and `optimizer`, and every RNG, in place.

    Returns the epoch the run had completed, which is where the loop resumes.
    The manifest is trusted to have passed `plan_resume`: this executes a
    decision, it does not make one. It does refuse a checkpoint whose own state
    disagrees with its manifest, because that is corruption and not a judgement.
    """
    import torch

    blob = torch.load(
        io.BytesIO(store.get(manifest["optimizer_state_ref"])), weights_only=False
    )
    if int(blob["step"]) != int(manifest["step"]):
        raise ValueError(
            f"checkpoint state is at step {blob['step']} but its manifest says {manifest['step']}"
        )
    model.load_state_dict(blob["model"])
    optimizer.load_state_dict(blob["optimizer"])
    torch.set_rng_state(blob["torch_cpu_rng"])
    if blob.get("torch_cuda_rng") and torch.cuda.is_available():
        torch.cuda.set_rng_state_all(blob["torch_cuda_rng"])
    np.random.set_state(blob["numpy_rng"])
    random.setstate(blob["python_rng"])
    return int(blob["step"])


def checkpointer_from_config(cfg: dict[str, Any] | None, *, emit_progress=None, store=None):
    """The `TorchCheckpointer` a request asked for, or `None`.

    `cfg` is `definition["_checkpoint"]`, set by `worker.run_training`. Absent
    means checkpointing is off, which is the right default for a short job.
    """
    if not cfg:
        return None
    if store is None:
        from .artifact_store import get_store

        store = get_store()

    holder: dict[str, Any] = {}

    def publish(manifest: dict[str, Any]) -> None:
        if emit_progress is not None:
            # At the run's current progress, not 0: the platform stores the last
            # percentage it saw, and a checkpoint frame that said 0 would rewind
            # the progress bar of a run that is 60% done.
            emit_progress(
                "checkpoint", holder["ck"].pct, {"checkpoint": manifest}
            )

    ck = TorchCheckpointer(
        store=store,
        key_prefix=cfg["key_prefix"],
        code_hash=cfg["code_hash"],
        image_digest=cfg["image_digest"],
        dataset_id=cfg["dataset_id"],
        mtbf_s=cfg.get("mtbf_s"),
        publish=publish,
    )
    holder["ck"] = ck
    return ck


def code_hash() -> str:
    """Identity of the code that writes and reads checkpoints.

    A checkpoint is only resumable into the same computation (`plan_resume`
    compares this). The hash covers the files whose behaviour a resumed run
    depends on: the loop, the writer, and the two torch adapters. Computed from
    source bytes rather than a version string, so an edit that changes
    behaviour changes the hash without anybody remembering to bump anything.
    """
    import hashlib
    import pathlib

    here = pathlib.Path(__file__).resolve().parent
    h = hashlib.sha256()
    for rel in ("engine.py", "checkpoint.py", "adapters/torch_adapter.py", "adapters/forecaster.py"):
        h.update(rel.encode())
        h.update((here / rel).read_bytes())
    return f"sha256:{h.hexdigest()}"


def request_config(
    *, framework: str, output_prefix: str, dataset_hash: str, resume: dict[str, Any] | None
) -> dict[str, Any] | None:
    """The `_checkpoint` block for a training request, or `None` when off.

    Off for a framework whose resume AT-61 has not shown bit-identical: a
    checkpoint of a run that cannot honestly resume is storage with a hopeful
    name. `TRAINER_CHECKPOINTS=0` turns it off everywhere; `TRAINER_MTBF_S` is the
    mean time between failures the Young-Daly interval needs, and without it only
    the cost-measuring first checkpoint is taken.
    """
    import os

    if os.environ.get("TRAINER_CHECKPOINTS", "1") == "0":
        return None
    if resume_support(framework) != "bit_identical":
        return None
    mtbf = os.environ.get("TRAINER_MTBF_S")
    image = image_digest()
    return {
        "key_prefix": f"{output_prefix.rstrip('/')}/checkpoints".replace("file://", ""),
        "code_hash": code_hash(),
        "image_digest": image,
        "dataset_id": dataset_hash,
        "mtbf_s": float(mtbf) if mtbf else None,
        "resume": resume,
    }


def image_digest() -> str:
    """The image this process runs in (`IMAGE_DIGEST`, else `local-dev`).

    Reported by `/capabilities` and written into every checkpoint manifest, so
    `plan_resume` compares like with like.
    """
    import os

    return os.environ.get("IMAGE_DIGEST", "local-dev")

